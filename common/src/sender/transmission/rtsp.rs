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

// Custom subclasses are derived from
// https://gitlab.freedesktop.org/gstreamer/gstreamer-rs/-/blob/main/examples/src/bin/rtsp-server-subclass.rs?ref_type=heads

use crate::sender::pipeline::SourceConfig;
use crate::sender::pipeline::SourceConfigBoxed;

use super::TransmissionSink;

use fcast_protocol::v2::PlayMessage;
use gst::glib;
use gst::prelude::*;
use gst_rtsp_server::{RTSPMountPoints, RTSPServer, prelude::*};

#[derive(Default, PartialEq, Eq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "GstSourceConfigVariant")]
enum SourceConfigVariant {
    #[enum_value(name = "Audio and video", nick = "audio-video")]
    #[default]
    AudioVideo,
    #[enum_value(name = "Video", nick = "video")]
    Video,
    #[enum_value(name = "Audio", nick = "audio")]
    Audio,
}

mod media_factory {
    use gst_rtsp_server::subclass::prelude::*;

    use super::*;

    mod imp {
        use log::debug;
        use parking_lot::Mutex;
        use std::sync::LazyLock;

        use crate::sender::pipeline::{AudioSource, SourceConfigBoxed, VideoSource};

        use super::*;

        fn create_video_src(src: &VideoSource) -> anyhow::Result<gst::Element> {
            match src {
                #[cfg(target_os = "linux")]
                VideoSource::PipeWire { node_id, .. } => {
                    Ok(gst::ElementFactory::make("pipewiresrc")
                        .property("path", node_id.to_string())
                        .build()?)
                }
                #[cfg(target_os = "linux")]
                VideoSource::XWindow { id, .. } => Ok(gst::ElementFactory::make("ximagesrc")
                    .property("xid", *id as u64)
                    .property("use-damage", false)
                    .build()?),
                #[cfg(target_os = "linux")]
                VideoSource::XDisplay {
                    id,
                    width,
                    height,
                    x_offset,
                    y_offset,
                    ..
                } => Ok(gst::ElementFactory::make("ximagesrc")
                    .property("xid", *id as u64)
                    .property("startx", *x_offset as u32)
                    .property("starty", *y_offset as u32)
                    .property("endx", (*x_offset as u32) + (*width as u32) - 1)
                    .property("endy", (*y_offset as u32) + (*height as u32) - 1)
                    .property("use-damage", false)
                    .build()?),
                #[cfg(target_os = "macos")]
                VideoSource::DefaultAvf => Ok(gst::ElementFactory::make("avfvideosrc")
                    .property("capture-screen", true)
                    .property("capture-screen-cursor", true)
                    .build()?),
            }
        }

        fn create_audio_src(src: &AudioSource) -> anyhow::Result<gst::Element> {
            #[cfg(target_os = "linux")]
            match src {
                #[cfg(target_os = "linux")]
                AudioSource::Pipewire { id, .. } => Ok(gst::ElementFactory::make("pipewiresrc")
                    .property("path", id.to_string())
                    .build()?),
            }
            #[cfg(target_os = "macos")]
            unreachable!();
        }

        fn create_video_encode_elements(n_payloaders: usize) -> anyhow::Result<Vec<gst::Element>> {
            // if let Some(enc) = gst::ElementFactory::find("vavp8enc") {
            //     debug!("Using vavp8enc");
            //     let enc = enc.create()
            //         .property("target-usage", 6u32)
            //         .build()?;
            //     let pay = gst::ElementFactory::make("rtpvp8pay")
            //         .name(format!("pay{n_payloaders}"))
            //         .build()?;
            //     Ok(vec![enc, pay])
            // } else if let Some(enc) = gst::ElementFactory::find("vp8enc") {
            //     // TODO: These settings are not so good (taken from https://github.com/GStreamer/gst-plugins-rs/blob/main/net/webrtc/src/webrtcsink/imp.rs)
            //     let enc = enc.create()
            //         .property("deadline", 1i64)
            //         .property("target-bitrate", 12 * 1000 * 1000)
            //         .property_from_str("end-usage", "cbr")
            //         .property("cpu-used", -16i32)
            //         .property("keyframe-max-dist", 1000i32)
            //         .property_from_str("keyframe-mode", "disabled")
            //         .property("buffer-initial-size", 100i32)
            //         .property("buffer-optimal-size", 120i32)
            //         .property("buffer-size", 150i32)
            //         .property("max-intra-bitrate", 250i32)
            //         .property_from_str("error-resilient", "default")
            //         .property("lag-in-frames", 0i32)
            //         .build()?;
            //     let pay = gst::ElementFactory::make("rtpvp8pay")
            //         .name(format!("pay{n_payloaders}"))
            //         .build()?;
            //     Ok(vec![enc, pay])
            // } else if let Some(enc) = gst::ElementFactory::find("vah264enc") {
            //     enc.set_property("bitrate", start_bitrate / 1000);
            //     enc.set_property("keyframe-period", 2560u32);
            //     enc.set_property_from_str("rate-control", "cbr");
            //     Ok(elems)
            // } else if let Some(enc) = gst::ElementFactory::find("x264enc") {
            if let Some(enc) = gst::ElementFactory::find("x264enc") {
                let enc = enc
                    .create()
                    .property_from_str("tune", "zerolatency")
                    .property_from_str("speed-preset", "ultrafast")
                    .property("b-adapt", false)
                    .property("key-int-max", 2560u32)
                    .property("vbv-buf-capacity", 120u32)
                    .build()?;
                let enc_capsfilter = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-h264")
                            .field("profile", "constrained-baseline")
                            .build(),
                    )
                    .build()?;
                let pay = gst::ElementFactory::make("rtph264pay")
                    .property("config-interval", -1i32)
                    .name(format!("pay{n_payloaders}"))
                    .build()?;
                Ok(vec![enc, enc_capsfilter, pay])
            } else {
                anyhow::bail!("No encoder available")
            }
        }

        #[derive(Default)]
        pub struct Factory {
            source_config: Mutex<Option<SourceConfigBoxed>>,
        }

        impl Factory {
            fn try_create_element(&self) -> anyhow::Result<gst::Element> {
                let bin = gst::Bin::default();

                let Some(ref src_config) = *self.source_config.lock() else {
                    anyhow::bail!("Missing source config");
                };

                let (video_src, audio_src) = match &src_config.0 {
                    SourceConfig::AudioVideo { video, audio } => (
                        Some(create_video_src(video)?),
                        Some(create_audio_src(audio)?),
                    ),
                    SourceConfig::Video(video) => (Some(create_video_src(video)?), None),
                    SourceConfig::Audio(audio) => (None, Some(create_audio_src(audio)?)),
                };

                assert!(video_src.is_some() || audio_src.is_some());

                let mut n_payloaders = 0;

                if let Some(video_src) = video_src {
                    debug!("Adding video source");
                    let convert_queue = gst::ElementFactory::make("queue").build()?;
                    let convert = gst::ElementFactory::make("videoconvert").build()?;
                    let scale = gst::ElementFactory::make("videoscale").build()?;
                    let flip = gst::ElementFactory::make("videoflip")
                        .property_from_str("video-direction", "auto")
                        .build()?;
                    let rate = gst::ElementFactory::make("videorate").build()?;
                    let capsfilter = gst::ElementFactory::make("capsfilter")
                        .property(
                            "caps",
                            gst::Caps::builder("video/x-raw")
                                .field("width", gst::IntRange::with_step(32i32, 4096, 2))
                                .field("height", gst::IntRange::with_step(32i32, 4096, 2))
                                .field("framerate", gst::Fraction::new(30, 1))
                                .build(),
                        )
                        .build()?;
                    let enc_queue = gst::ElementFactory::make("queue").build()?;
                    let mut elems = vec![
                        video_src,
                        convert_queue,
                        convert,
                        scale,
                        flip,
                        rate,
                        capsfilter,
                        enc_queue,
                    ];
                    let enc_elems = create_video_encode_elements(n_payloaders)?;
                    n_payloaders += 1;
                    elems.extend_from_slice(&enc_elems);
                    bin.add_many(&elems)?;
                    gst::Element::link_many(&elems)?;
                }

                if let Some(audio_src) = audio_src {
                    debug!("Adding audio source");
                    let convert_queue = gst::ElementFactory::make("queue").build()?;
                    let convert = gst::ElementFactory::make("audioconvert").build()?;
                    let resample = gst::ElementFactory::make("audioresample").build()?;
                    let capsfilter = gst::ElementFactory::make("capsfilter")
                        .property(
                            "caps",
                            gst::Caps::builder("audio/x-raw")
                                .field("rate", 48000i32)
                                .field("channels", 2)
                                .build(),
                        )
                        .build()?;
                    let enc_queue = gst::ElementFactory::make("queue").build()?;
                    let enc = gst::ElementFactory::make("opusenc")
                        .property("bitrate", 128_000i32)
                        .build()?;
                    let pay = gst::ElementFactory::make("rtpopuspay")
                        .name(format!("pay{n_payloaders}"))
                        .build()?;
                    let elems = [
                        audio_src,
                        convert_queue,
                        convert,
                        resample,
                        capsfilter,
                        enc_queue,
                        enc,
                        pay,
                    ];
                    bin.add_many(&elems)?;
                    gst::Element::link_many(&elems)?;
                }

                Ok(bin.upcast())
            }
        }

        #[glib::object_subclass]
        impl ObjectSubclass for Factory {
            const NAME: &'static str = "OmCustomRTSPMediaFactory";
            type Type = super::Factory;
            type ParentType = gst_rtsp_server::RTSPMediaFactory;
        }

        impl ObjectImpl for Factory {
            fn properties() -> &'static [glib::ParamSpec] {
                static PROPERTIES: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
                    vec![
                        glib::ParamSpecBoxed::builder::<SourceConfigBoxed>("source-config")
                            .mutable_ready()
                            .build(),
                    ]
                });

                &PROPERTIES
            }

            fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
                match pspec.name() {
                    "source-config" => {
                        let mut source_config = self.source_config.lock();
                        let new_source_config = value.get().expect("type checked upstream");
                        *source_config = Some(new_source_config);
                    }
                    _ => unimplemented!(),
                }
            }
        }

        impl RTSPMediaFactoryImpl for Factory {
            fn create_element(&self, _url: &gst_rtsp::RTSPUrl) -> Option<gst::Element> {
                match self.try_create_element() {
                    Ok(elem) => Some(elem),
                    Err(err) => {
                        log::error!("Failed to create source element: {err}");
                        return None;
                    }
                }
            }
        }
    }

    glib::wrapper! {
        pub struct Factory(ObjectSubclass<imp::Factory>) @extends gst_rtsp_server::RTSPMediaFactory;
    }

    impl Default for Factory {
        fn default() -> Factory {
            glib::Object::new()
        }
    }
}

pub struct RtspSink {
    server: RTSPServer,
    id: Option<glib::SourceId>,
    main_loop: glib::MainLoop,
}

impl RtspSink {
    pub fn new(source_config: SourceConfig, port: u16) -> anyhow::Result<Self> {
        let server = RTSPServer::new();

        server.set_service(&format!("{port}"));

        let mounts = RTSPMountPoints::new();
        server.set_mount_points(Some(&mounts));

        let factory = media_factory::Factory::default();
        factory.set_shared(true);
        factory.set_latency(0);
        factory.set_property("source-config", SourceConfigBoxed(source_config));

        mounts.add_factory("/", factory);

        let id = server.attach(None)?;

        let main_loop = glib::MainLoop::new(None, false);
        {
            let main_loop = main_loop.clone();
            std::thread::spawn(move || {
                main_loop.run();
                log::debug!("Main loop runner thread finished");
            });
        }

        Ok(Self {
            server,
            id: Some(id),
            main_loop,
        })
    }
}

impl TransmissionSink for RtspSink {
    fn get_play_msg(&self, addr: std::net::IpAddr) -> Option<PlayMessage> {
        Some(PlayMessage {
            container: "application/x-rtsp".to_owned(),
            url: Some(format!(
                "rtsp://{}:{}/",
                super::addr_to_url_string(addr),
                self.server.bound_port()
            )),
            content: None,
            time: None,
            speed: None,
            headers: None,
        })
    }

    fn playing(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn shutdown(&mut self) {
        self.main_loop.quit();
        if let Some(id) = self.id.take() {
            id.remove();
        }
    }
}
