// Copyright (C) 2025 Marcus L. Hanestad <marlhan@proton.me>
//
// This file is part of OpenMirroring.
//
// OpenMirroring is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// OpenMirroring is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with OpenMirroring.  If not, see <https://www.gnu.org/licenses/>.

use common::net::get_default_ipv4_addr;
use fake_file_writer::FakeFileWriter;
use gst::{glib, prelude::*};
use log::{debug, error, trace};
use m3u8_rs::{MasterPlaylist, VariantStream};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc::{Receiver, Sender},
};
use url_utils::decode_path;

use super::TransmissionSink;

mod fake_file_writer;
mod url_utils;

const HLS_MIME_TYPE: &str = "application/vnd.apple.mpegurl";

async fn serve_dir(
    base: PathBuf,
    server_port: Arc<Mutex<Option<u16>>>,
    mut file_rx: Receiver<fake_file_writer::ChannelElement>,
) {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();

    debug!("HTTP server listening on {:?}", listener.local_addr());

    {
        let mut server_port = server_port.lock().unwrap();
        *server_port = Some(listener.local_addr().unwrap().port());
    }

    let mut files: HashMap<String, Vec<u8>> = HashMap::new();

    let mut request_buf = Vec::new();
    let mut response_buf = Vec::new();

    loop {
        tokio::select! {
            v = listener.accept() => {
                let (mut stream, _) = v.unwrap();

                request_buf.clear();
                response_buf.clear();

                let mut buf = [0; 4096];
                loop {
                    let bytes_read = stream.read(&mut buf).await.unwrap();
                    request_buf.extend_from_slice(&buf);
                    if bytes_read < buf.len() {
                        break;
                    }
                }

                let request = http::Request::parse(&request_buf).unwrap();

                if request.start_line.method != http::RequestMethod::Get {
                    let response = http::Response {
                        start_line: http::ResponseStartLine {
                            version: http::HttpVersion::One,
                            status: http::StatusCode::NotImplemented,
                        },
                        headers: vec![],
                        body: None,
                    };
                    response.serialize_into(&mut response_buf);
                    stream.write_all(&response_buf).await.unwrap();
                    continue;
                }

                let Ok(uri) = decode_path(
                    request
                        .start_line
                        .target
                        .trim_start_matches("/")
                ) else {
                    error!("Failed to decode path {}", request.start_line.target);
                    let response = http::Response {
                        start_line: http::ResponseStartLine {
                            version: http::HttpVersion::One,
                            status: http::StatusCode::InternalServerError,
                        },
                        headers: vec![],
                        body: None,
                    };
                    response.serialize_into(&mut response_buf);
                    stream.write_all(&response_buf).await.unwrap();
                    continue;
                };

                let mut base_path = base.clone();
                base_path.push(&uri);

                let key = base_path.to_string_lossy().to_string();

                match files.get(&key) {
                    Some(file_contents) => {
                        let response = http::Response {
                            start_line: http::ResponseStartLine {
                                version: http::HttpVersion::One,
                                status: http::StatusCode::Ok,
                            },
                            headers: vec![
                                // TODO: function
                                http::Header {
                                    key: "Content-Type".to_owned(),
                                    value: "application/octet-stream".to_owned(),
                                },
                                http::Header {
                                    key: "Content-Length".to_owned(),
                                    value: file_contents.len().to_string(),
                                },
                            ],
                            body: Some(file_contents),
                        };
                        response.serialize_into(&mut response_buf);
                        stream.write_all(&response_buf).await.unwrap();
                    }
                    None => {
                        error!("File not found: {}", base_path.display());
                        let response = http::Response {
                            start_line: http::ResponseStartLine {
                                version: http::HttpVersion::One,
                                status: http::StatusCode::NotFound,
                            },
                            headers: vec![],
                            body: None,
                        };
                        response.serialize_into(&mut response_buf);
                        stream.write_all(&response_buf).await.unwrap();
                        continue;
                    }
                }
            }
            v = file_rx.recv() => {
                let Some(v) = v else {
                    continue;
                };
                match v.request {
                    fake_file_writer::Request::Delete => {
                        files.remove(&v.location.replace('\\', "/"));
                    }
                    fake_file_writer::Request::Add(vec) => {
                        files.insert(v.location.replace('\\', "/"), vec);
                    }
                }
            }
        }
    }
}

fn get_codec_name(sink: &gst::Element) -> String {
    let pad = sink.static_pad("sink").unwrap();
    let caps = pad.sticky_event::<gst::event::Caps>(0).unwrap();
    gst_pbutils::codec_utils_caps_get_mime_codec(caps.caps())
        .unwrap()
        .to_string()
}

pub struct HlsSink {
    src_pad: gst::Pad,
    queue_pad: gst::Pad,
    queue: gst::Element,
    convert: gst::Element,
    pub server_port: Arc<Mutex<Option<u16>>>,
    main_path: PathBuf,
    enc: gst::Element,
    enc_caps: gst::Element,
    sink: gst::Element,
    write_playlist: bool,
    file_tx: Sender<fake_file_writer::ChannelElement>,
}

impl HlsSink {
    pub fn new(pipeline: &gst::Pipeline, src_pad: gst::Pad) -> Result<Self, glib::BoolError> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
            .build()?;
        let convert = gst::ElementFactory::make("videoconvert")
            .name("sink_convert")
            .build()?;

        let enc = gst::ElementFactory::make("x264enc")
            .property("bframes", 0u32)
            // TODO: find a good bitrate
            .property("bitrate", 2_048_000 / 1000u32)
            .property("key-int-max", i32::MAX as u32)
            .property_from_str("tune", "zerolatency")
            .property_from_str("speed-preset", "superfast")
            .build()?;
        let enc_caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-h264")
                    .field("profile", "main")
                    .build(),
            )
            .build()?;

        let base_path = PathBuf::from("/");

        let (file_tx, file_rx) = tokio::sync::mpsc::channel::<fake_file_writer::ChannelElement>(10);

        let server_port = Arc::new(Mutex::new(None));
        common::runtime().spawn(serve_dir(
            base_path.clone(),
            Arc::clone(&server_port),
            file_rx,
        ));

        let mut manifest_path = base_path.clone();
        manifest_path.push("manifest.m3u8");

        let mut path = base_path.clone();
        path.push("video");

        let mut playlist_location = path.clone();
        playlist_location.push("manifest.m3u8");

        let mut init_location = path.clone();
        init_location.push("init_%30d.mp4");

        let mut location = path.clone();
        location.push("segment_%05d.m4s");

        let sink = gst::ElementFactory::make("hlscmafsink")
            .name("hls_sink")
            .property("target-duration", 1u32)
            .property("playlist-location", playlist_location.to_str().unwrap())
            .property("init-location", init_location.to_str().unwrap())
            .property("location", location.to_str().unwrap())
            .property("enable-program-date-time", true)
            .property("sync", true)
            .property("latency", 0u64)
            .build()?;

        let file_tx_clone = file_tx.clone();
        sink.connect_closure(
            "get-init-stream",
            false,
            glib::closure!(move |sink: &gst::Element, location: &str| {
                trace!("{}, writing init segment to {location}", sink.name());
                FakeFileWriter::new(location.to_string(), file_tx_clone.clone())
                    .upcast::<gio::OutputStream>()
            }),
        );

        let file_tx_clone = file_tx.clone();
        sink.connect_closure(
            "get-fragment-stream",
            false,
            glib::closure!(move |sink: &gst::Element, location: &str| {
                trace!("{}, writing segment to {location}", sink.name());
                FakeFileWriter::new(location.to_string(), file_tx_clone.clone())
                    .upcast::<gio::OutputStream>()
            }),
        );

        let file_tx_clone = file_tx.clone();
        sink.connect_closure(
            "delete-fragment",
            false,
            glib::closure!(move |sink: &gst::Element, location: &str| {
                trace!("{}, removing segment {location}", sink.name());
                file_tx_clone
                    .blocking_send(fake_file_writer::ChannelElement {
                        location: location.to_string(),
                        request: fake_file_writer::Request::Delete,
                    })
                    .is_ok()
            }),
        );

        let file_tx_clone = file_tx.clone();
        sink.connect_closure(
            "get-playlist-stream",
            false,
            glib::closure!(move |sink: &gst::Element, location: &str| {
                trace!("{}, writing playlist to {location}", sink.name());
                FakeFileWriter::new(location.to_string(), file_tx_clone.clone())
                    .upcast::<gio::OutputStream>()
            }),
        );

        pipeline.add_many([&queue, &convert, &enc, &enc_caps, &sink])?;
        gst::Element::link_many([&queue, &convert, &enc, &enc_caps, &sink])?;

        queue.sync_state_with_parent().unwrap();
        convert.sync_state_with_parent().unwrap();
        enc.sync_state_with_parent().unwrap();
        enc_caps.sync_state_with_parent().unwrap();
        sink.sync_state_with_parent().unwrap();

        let queue_pad = queue
            .static_pad("sink")
            .map_or_else(|| Err(glib::bool_error!("`static_pad()` failed")), Ok)?;
        src_pad
            .link(&queue_pad)
            .map_err(|err| glib::bool_error!("{err}"))?;

        Ok(Self {
            src_pad,
            queue_pad,
            queue,
            convert,
            server_port,
            main_path: manifest_path,
            enc,
            enc_caps,
            sink,
            write_playlist: true,
            file_tx,
        })
    }

    pub async fn write_manifest_file(&mut self) {
        if !self.write_playlist {
            return;
        }

        let video_codec = get_codec_name(&self.sink);

        let variants = vec![VariantStream {
            uri: "video/manifest.m3u8".to_string(),
            codecs: Some(video_codec),
            ..Default::default()
        }];

        let playlist = MasterPlaylist {
            version: Some(6),
            variants,
            ..Default::default()
        };

        debug!("Writing master manifest to {}", self.main_path.display());

        let mut buf = Vec::new();
        playlist.write_to(&mut buf).unwrap();

        self.file_tx
            .send(fake_file_writer::ChannelElement {
                location: self.main_path.to_string_lossy().to_string(),
                request: fake_file_writer::Request::Add(buf),
            })
            .await
            .unwrap();

        self.write_playlist = false;
    }
}

#[async_trait::async_trait]
impl TransmissionSink for HlsSink {
    fn get_play_msg(&self) -> Option<crate::Message> {
        let server_port = self.server_port.lock().unwrap();

        (*server_port)
            .as_ref()
            .map(|server_port| crate::Message::Play {
                mime: HLS_MIME_TYPE.to_owned(),
                uri: format!(
                    "http://{}:{server_port}/manifest.m3u8",
                    get_default_ipv4_addr(),
                ),
            })
    }

    async fn playing(&mut self) {
        self.write_manifest_file().await;
    }

    fn shutdown(&mut self) {}

    fn unlink(&mut self, pipeline: &gst::Pipeline) -> Result<(), glib::error::BoolError> {
        let block = self
            .src_pad
            .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                gst::PadProbeReturn::Ok
            })
            .unwrap();
        self.src_pad.unlink(&self.queue_pad)?;
        self.src_pad.remove_probe(block);

        let elems = [
            &self.queue,
            &self.convert,
            &self.enc,
            &self.enc_caps,
            &self.sink,
        ];

        pipeline.remove_many(elems)?;

        for elem in elems {
            elem.set_state(gst::State::Null)
                .map_err(|err| glib::bool_error!("{err}"))?;
        }

        Ok(())
    }
}
