use gst::{glib, prelude::*};
use log::{debug, error, trace};
use m3u8_rs::{MasterPlaylist, VariantStream};
use rand::Rng;
use std::path::PathBuf;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc::Sender,
};
use url_utils::decode_path;

mod url_utils;

async fn serve_dir(base: PathBuf, event_tx: Sender<crate::Event>) {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();

    debug!("HTTP server listening on {:?}", listener.local_addr());

    event_tx
        .send(crate::Event::HlsServerAddr {
            port: listener.local_addr().unwrap().port(),
        })
        .await
        .unwrap();

    let mut request_buf = Vec::new();
    let mut response_buf = Vec::new();
    let mut file_buf = Vec::new();

    loop {
        request_buf.clear();
        file_buf.clear();
        response_buf.clear();

        let (mut stream, _) = listener.accept().await.unwrap();

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
            &request
                .start_line
                .target
                .trim_start_matches("/")
                .replace("..", ""),
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

        let Ok(mut file) = tokio::fs::File::open(&base_path).await else {
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
        };

        file.read_to_end(&mut file_buf).await.unwrap();

        let response = http::Response {
            start_line: http::ResponseStartLine {
                version: http::HttpVersion::One,
                status: http::StatusCode::Ok,
            },
            headers: vec![
                http::Header {
                    key: "Content-Type".to_owned(),
                    value: "application/octet-stream".to_owned(),
                },
                http::Header {
                    key: "Content-Length".to_owned(),
                    value: file_buf.len().to_string(),
                },
            ],
            body: Some(&file_buf),
        };
        response.serialize_into(&mut response_buf);
        stream.write_all(&response_buf).await.unwrap();
    }
}

// TODO: Make portable
fn generate_rand_tmp_dir_path() -> PathBuf {
    let mut rng = rand::rng();
    // Spin until we generate a sub directory in /tmp that does not exist
    loop {
        let path = PathBuf::from(format!("/tmp/om-{}", rng.random::<u32>()));
        if !path.exists() {
            return path;
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
    base_path: PathBuf,
    pub main_path: PathBuf,
    pub enc: gst::Element,
    pub enc_caps: gst::Element,
    pub sink: gst::Element,
    write_playlist: bool,
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

        let base_path = generate_rand_tmp_dir_path();
        std::fs::create_dir_all(&base_path).unwrap();

        om_common::runtime().spawn(serve_dir(base_path.clone(), event_tx));

        let mut manifest_path = base_path.clone();
        manifest_path.push("manifest.m3u8");

        let mut path = base_path.clone();
        path.push("video");
        std::fs::create_dir_all(&path).unwrap();

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
            // Give upstream 150ms to encode and stuff
            // TODO: find out what the minimum is
            .property("latency", 150000000u64)
            .build()?;

        sink.connect_closure(
            "get-init-stream",
            false,
            glib::closure!(move |sink: &gst::Element, location: &str| {
                trace!("{}, writing init segment to {location}", sink.name());
                let file = std::fs::File::create(location).unwrap();
                gio::WriteOutputStream::new(file).upcast::<gio::OutputStream>()
            }),
        );

        sink.connect_closure(
            "get-fragment-stream",
            false,
            glib::closure!(move |sink: &gst::Element, location: &str| {
                trace!("{}, writing segment to {location}", sink.name());
                let file = std::fs::File::create(location).unwrap();
                gio::WriteOutputStream::new(file).upcast::<gio::OutputStream>()
            }),
        );

        sink.connect_closure(
            "delete-fragment",
            false,
            glib::closure!(move |sink: &gst::Element, location: &str| {
                trace!("{}, removing segment {location}", sink.name());
                std::fs::remove_file(location).unwrap();
                true
            }),
        );

        pipeline.add_many([&enc, &enc_caps, &sink])?;

        Ok(Self {
            base_path,
            enc,
            enc_caps,
            sink,
            main_path: manifest_path,
            write_playlist: true,
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

        let mut file = std::fs::File::create(&self.main_path).unwrap();
        playlist.write_to(&mut file).unwrap();

        self.write_playlist = false;
    }

    pub fn shutdown(&self) {
        if let Err(err) = std::fs::remove_dir_all(&self.base_path) {
            error!("Failed to remove {}: {err}", self.base_path.display());
        }
        debug!("Removed stream directory at {}", self.base_path.display());
    }
}
