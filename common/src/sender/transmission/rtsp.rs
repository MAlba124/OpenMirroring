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
        use std::{str::FromStr, sync::LazyLock};

        use super::*;

        #[derive(Default)]
        pub struct Factory {
            source_config: Mutex<SourceConfigVariant>,
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
                        glib::ParamSpecEnum::builder_with_default(
                            "source-config",
                            SourceConfigVariant::AudioVideo,
                        )
                        .nick("Source config")
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
                        *source_config = new_source_config;
                    }
                    _ => unimplemented!(),
                }
            }
        }

        impl RTSPMediaFactoryImpl for Factory {
            fn create_element(&self, _url: &gst_rtsp::RTSPUrl) -> Option<gst::Element> {
                let bin = gst::Bin::default();

                let src_config = *self.source_config.lock();

                let mut n_payloaders = 0;

                if src_config == SourceConfigVariant::Video
                    || src_config == SourceConfigVariant::AudioVideo
                {
                    debug!("Adding video source");
                    let src = gst::ElementFactory::make("intervideosrc")
                        .property("timeout", 5000000000u64)
                        .build()
                        .unwrap();
                    let convert_queue = gst::ElementFactory::make("queue").build().unwrap();
                    let convert = gst::ElementFactory::make("videoconvert").build().unwrap();
                    let scale = gst::ElementFactory::make("videoscale").build().unwrap();
                    let rate = gst::ElementFactory::make("videorate").build().unwrap();
                    let capsfilter = gst::ElementFactory::make("capsfilter")
                        .property("caps", gst::Caps::from_str("video/x-raw,width=(int)[16,8192,2],height=(int)[16,8192,2],framerate=30/1").ok()?)
                        .build()
                        .unwrap();
                    let enc_queue = gst::ElementFactory::make("queue").build().unwrap();
                    let enc = gst::ElementFactory::make("x264enc")
                        .property_from_str("tune", "zerolatency")
                        .property_from_str("speed-preset", "ultrafast")
                        .property("b-adapt", false)
                        .property("key-int-max", 2250u32)
                        .build()
                        .unwrap();
                    let enc_capsfilter = gst::ElementFactory::make("capsfilter")
                        .property(
                            "caps",
                            gst::Caps::from_str("video/x-h264,profile=baseline").ok()?,
                        )
                        .build()
                        .unwrap();
                    let pay = gst::ElementFactory::make("rtph264pay")
                        .property("config-interval", -1i32)
                        .name(format!("pay{n_payloaders}"))
                        .build()
                        .unwrap();
                    n_payloaders += 1;
                    let elems = [
                        src,
                        convert_queue,
                        convert,
                        scale,
                        rate,
                        capsfilter,
                        enc_queue,
                        enc,
                        enc_capsfilter,
                        pay,
                    ];
                    bin.add_many(&elems).unwrap();
                    gst::Element::link_many(&elems).unwrap();
                }
                if src_config == SourceConfigVariant::Audio
                    || src_config == SourceConfigVariant::AudioVideo
                {
                    debug!("Adding audio source");
                    let src = gst::ElementFactory::make("interaudiosrc").build().unwrap();
                    let convert_queue = gst::ElementFactory::make("queue").build().unwrap();
                    let convert = gst::ElementFactory::make("audioconvert").build().unwrap();
                    let resample = gst::ElementFactory::make("audioresample").build().unwrap();
                    let capsfilter = gst::ElementFactory::make("capsfilter")
                        .property("caps", gst::Caps::from_str("audio/x-raw,rate=48000").ok()?)
                        .build()
                        .unwrap();
                    let enc_queue = gst::ElementFactory::make("queue").build().unwrap();
                    let enc = gst::ElementFactory::make("opusenc").build().unwrap();
                    let pay = gst::ElementFactory::make("rtpopuspay")
                        .name(format!("pay{n_payloaders}"))
                        .build()
                        .unwrap();
                    let elems = [
                        src,
                        convert_queue,
                        convert,
                        resample,
                        capsfilter,
                        enc_queue,
                        enc,
                        pay,
                    ];
                    bin.add_many(&elems).unwrap();
                    gst::Element::link_many(&elems).unwrap();
                }

                Some(bin.upcast())
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
    pub fn new(
        pipeline: &gst::Pipeline,
        source_config: SourceConfig,
        port: u16,
    ) -> anyhow::Result<Self> {
        let server = RTSPServer::new();

        server.set_service(&format!("{port}"));

        let mounts = RTSPMountPoints::new();
        server.set_mount_points(Some(&mounts));

        let factory = media_factory::Factory::default();
        factory.set_shared(true);

        match source_config {
            SourceConfig::AudioVideo { video, audio } => {
                let video_sink = gst::ElementFactory::make("intervideosink").build()?;
                let audio_sink = gst::ElementFactory::make("interaudiosink").build()?;
                pipeline.add_many([&video_sink, &audio_sink])?;
                video.link(&video_sink)?;
                audio.link(&audio_sink)?;
                factory.set_property("source-config", SourceConfigVariant::AudioVideo);
            }
            SourceConfig::Video(video) => {
                let video_sink = gst::ElementFactory::make("intervideosink").build()?;
                pipeline.add(&video_sink)?;
                video.link(&video_sink)?;
                factory.set_property("source-config", SourceConfigVariant::Video);
                // factory.set_launch(
                //     "( intervideosrc timeout=5000000000 ! queue ! videoconvert ! videoscale ! videorate \
                //        ! video/x-raw,width=(int)[16,4096,2],height=(int)[16,4096,2],framerate=30/1 \
                //        ! queue ! vah264enc target-usage=6 ! h264parse \
                //        ! rtph264pay config-interval=-1 name=pay0 )"
                // );
            }
            SourceConfig::Audio(audio) => {
                let audio_sink = gst::ElementFactory::make("interaudiosink").build()?;
                pipeline.add(&audio_sink)?;
                audio.link(&audio_sink)?;
                factory.set_property("source-config", SourceConfigVariant::Audio);
            }
        }

        factory.set_latency(0);

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

        // pipeline.debug_to_dot_file(gst::DebugGraphDetails::all(), "rtsp-pipeline");

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
