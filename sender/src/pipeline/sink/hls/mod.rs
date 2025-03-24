use fake_file_writer::FakeFileWriter;
use gst::{glib, prelude::*};
use log::{debug, error, trace};
use m3u8_rs::{MasterPlaylist, VariantStream};
use std::{collections::HashMap, path::PathBuf};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc::{Receiver, Sender},
};
use url_utils::decode_path;

mod fake_file_writer;
mod url_utils;

async fn serve_dir(
    base: PathBuf,
    event_tx: Sender<crate::Event>,
    mut file_rx: Receiver<fake_file_writer::ChannelElement>,
) {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();

    debug!("HTTP server listening on {:?}", listener.local_addr());

    event_tx
        .send(crate::Event::HlsServerAddr {
            port: listener.local_addr().unwrap().port(),
        })
        .await
        .unwrap();

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
                        files.remove(&v.location);
                    }
                    fake_file_writer::Request::Add(vec) => {
                        files.insert(v.location, vec);
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

pub struct Hls {
    pub main_path: PathBuf,
    pub enc: gst::Element,
    pub enc_caps: gst::Element,
    pub sink: gst::Element,
    write_playlist: bool,
    file_tx: Sender<fake_file_writer::ChannelElement>,
}

impl Hls {
    pub fn new(
        pipeline: &gst::Pipeline,
        event_tx: Sender<crate::Event>,
    ) -> Result<Self, gst::glib::BoolError> {
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

        common::runtime().spawn(serve_dir(base_path.clone(), event_tx, file_rx));

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

        pipeline.add_many([&enc, &enc_caps, &sink])?;

        Ok(Self {
            enc,
            enc_caps,
            sink,
            main_path: manifest_path,
            write_playlist: true,
            file_tx,
        })
    }

    pub fn write_manifest_file(&mut self) {
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
            .blocking_send(fake_file_writer::ChannelElement {
                location: self.main_path.to_string_lossy().to_string(),
                request: fake_file_writer::Request::Add(buf),
            })
            .unwrap();

        self.write_playlist = false;
    }
}
