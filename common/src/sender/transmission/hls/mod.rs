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

use anyhow::{Result, anyhow};
use fake_file_writer::FakeFileWriter;
use fcast_lib::models::PlayMessage;
use gst::{glib, prelude::*};
use log::{debug, error, trace};
use m3u8_rs::{MasterPlaylist, VariantStream};
use std::net::IpAddr;
use std::str::FromStr;
use std::{collections::HashMap, path::PathBuf};
use tokio::sync::oneshot;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc::{Receiver, Sender},
};
use url_utils::decode_path;

use super::TransmissionSink;

mod fake_file_writer;
mod url_utils;

const HLS_MIME_TYPE: &str = "application/vnd.apple.mpegurl";

// TODO: handler errors
async fn serve_dir(
    base: PathBuf,
    server_port: u16,
    mut file_rx: Receiver<fake_file_writer::ChannelElement>,
    mut fin: oneshot::Receiver<()>,
) {
    let listener = tokio::net::TcpListener::bind(format!(":::{server_port}"))
        .await
        .unwrap();

    debug!("HTTP server listening on {:?}", listener.local_addr());

    let mut files: HashMap<String, Vec<u8>> = HashMap::new();

    let mut request_buf = Vec::new();
    let mut response_buf = Vec::new();

    loop {
        tokio::select! {
            _ = &mut fin => {
                break;
            }
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
                                http::Header::new("Content-Type", "application/octet-stream"),
                                http::Header::new("Content-Length", file_contents.len().to_string()),
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

    debug!("Quitting http server");
}

fn get_codec_name(sink: &gst::Element) -> Result<String> {
    let pad = sink
        .static_pad("sink")
        .ok_or(anyhow!("Failed to get static sink pad"))?;
    let caps = pad
        .sticky_event::<gst::event::Caps>(0)
        .ok_or(anyhow!("Failed to get caps from sink pad"))?;
    Ok(gst_pbutils::codec_utils_caps_get_mime_codec(caps.caps())?.to_string())
}

pub struct HlsSink {
    src_pad: gst::Pad,
    queue_pad: gst::Pad,
    queue: gst::Element,
    convert: gst::Element,
    scale: gst::Element,
    capsfilter: gst::Element,
    pub server_port: u16,
    main_path: PathBuf,
    enc: gst::Element,
    enc_caps: gst::Element,
    sink: gst::Element,
    write_playlist: bool,
    file_tx: Sender<fake_file_writer::ChannelElement>,
    server_fin_tx: Option<oneshot::Sender<()>>,
}

impl HlsSink {
    pub fn new(
        pipeline: &gst::Pipeline,
        src_pad: gst::Pad,
        port: u16,
    ) -> Result<Self, glib::BoolError> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
            .build()?;

        let convert = gst::ElementFactory::make("videoconvert")
            .name("sink_convert")
            .build()?;
        let scale = gst::ElementFactory::make("videoscale")
            .name("sink_scale")
            .build()?;
        let capsfilter = gst::ElementFactory::make("capsfilter")
            .name("sink_capsfilter")
            .property(
                "caps",
                gst::Caps::from_str("video/x-raw,width=(int)[16,8192,2],height=(int)[16,8192,2]")?,
            )
            .build()?;

        let enc = gst::ElementFactory::make("x264enc")
            .property("bframes", 0u32)
            .property("bitrate", 1024 * 4u32)
            .property("key-int-max", i32::MAX as u32)
            .property_from_str("tune", "zerolatency")
            .property_from_str("speed-preset", "superfast")
            .build()?;
        let enc_caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-h264")
                    .field("profile", "main")
                    .field("framerate", gst::Fraction::new(0, 1))
                    .build(),
            )
            .build()?;

        let base_path = PathBuf::from("/");

        let (file_tx, file_rx) = tokio::sync::mpsc::channel::<fake_file_writer::ChannelElement>(10);

        let (server_fin_tx, server_fin_rx) = oneshot::channel::<()>();
        let server_port = port;
        crate::runtime().spawn(serve_dir(
            base_path.clone(),
            server_port,
            file_rx,
            server_fin_rx,
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

        pipeline.add_many([
            &queue,
            &convert,
            &scale,
            &capsfilter,
            &enc,
            &enc_caps,
            &sink,
        ])?;
        gst::Element::link_many([
            &queue,
            &convert,
            &scale,
            &capsfilter,
            &enc,
            &enc_caps,
            &sink,
        ])?;

        queue.sync_state_with_parent()?;
        convert.sync_state_with_parent()?;
        scale.sync_state_with_parent()?;
        capsfilter.sync_state_with_parent()?;
        enc.sync_state_with_parent()?;
        enc_caps.sync_state_with_parent()?;
        sink.sync_state_with_parent()?;

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
            scale,
            capsfilter,
            server_port,
            main_path: manifest_path,
            enc,
            enc_caps,
            sink,
            write_playlist: true,
            file_tx,
            server_fin_tx: Some(server_fin_tx),
        })
    }

    pub async fn write_manifest_file(&mut self) -> Result<()> {
        if !self.write_playlist {
            return Ok(());
        }

        let video_codec = get_codec_name(&self.sink)?;

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
        playlist.write_to(&mut buf)?;

        self.file_tx
            .send(fake_file_writer::ChannelElement {
                location: self.main_path.to_string_lossy().to_string(),
                request: fake_file_writer::Request::Add(buf),
            })
            .await?;

        self.write_playlist = false;

        Ok(())
    }
}

#[async_trait::async_trait]
impl TransmissionSink for HlsSink {
    fn get_play_msg(&self, addr: IpAddr) -> Option<PlayMessage> {
        Some(PlayMessage {
            container: HLS_MIME_TYPE.to_owned(),
            url: Some(format!(
                "http://{}:{}/manifest.m3u8", // TODO: correct addr string formatting?
                super::addr_to_url_string(addr), self.server_port,
            )),
            content: None,
            time: Some(0.0),
            speed: Some(1.0),
            headers: None,
        })
    }

    async fn playing(&mut self) -> Result<()> {
        self.write_manifest_file().await
    }

    fn shutdown(&mut self) {
        if let Some(fin_tx) = self.server_fin_tx.take() {
            let _ = fin_tx.send(());
        }
    }

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
            &self.scale,
            &self.capsfilter,
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
