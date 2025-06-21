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

use crate::sender::pipeline::SourceConfig;

use super::TransmissionSink;

use gst::glib;
use gst_rtsp_server::{RTSPMediaFactory, RTSPMountPoints, RTSPServer, prelude::*};

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

        let factory = RTSPMediaFactory::default();
        factory.set_shared(true);

        match source_config {
            SourceConfig::AudioVideo { video, audio } => {
                let video_sink = gst::ElementFactory::make("intervideosink").build()?;
                let audio_sink = gst::ElementFactory::make("interaudiosink").build()?;
                pipeline.add_many([&video_sink, &audio_sink])?;
                video.link(&video_sink)?;
                audio.link(&audio_sink)?;
                factory.set_launch(
                    "( intervideosrc ! queue ! videoconvert ! videoscale \
                       ! video/x-raw,width=(int)[16,8192,2],height=(int)[16,8192,2] \
                       ! queue ! x264enc tune=zerolatency speed-preset=ultrafast b-adapt=false key-int-max=2250 \
                       ! video/x-h264,profile=baseline ! rtph264pay config-interval=-1 name=pay0 \
                       interaudiosrc ! queue ! audioconvert ! audioresample ! audio/x-raw,rate=48000 \
                       ! queue ! opusenc ! rtpopuspay name=pay1 )"
                );
            }
            SourceConfig::Video(video) => {
                let video_sink = gst::ElementFactory::make("intervideosink").build()?;
                pipeline.add(&video_sink)?;
                video.link(&video_sink)?;
                factory.set_launch(
                    "( intervideosrc ! queue ! videoconvert ! videoscale \
                       ! video/x-raw,width=(int)[16,8192,2],height=(int)[16,8192,2] \
                       ! queue ! x264enc tune=zerolatency speed-preset=ultrafast b-adapt=false key-int-max=2250 \
                       ! video/x-h264,profile=baseline ! rtph264pay config-interval=-1 name=pay0 )"
                );
            }
            SourceConfig::Audio(audio) => {
                let audio_sink = gst::ElementFactory::make("interaudiosink").build()?;
                pipeline.add(&audio_sink)?;
                audio.link(&audio_sink)?;
                factory.set_launch(
                    "( interaudiosrc ! queue ! audioconvert ! audioresample ! audio/x-raw,rate=48000 \
                      ! queue ! opusenc ! rtpopuspay name=pay0 )"
                );
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
    fn get_play_msg(&self, addr: std::net::IpAddr) -> Option<fcast_lib::models::PlayMessage> {
        Some(fcast_lib::models::PlayMessage {
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

    fn unlink(&mut self, pipeline: &gst::Pipeline) -> Result<(), gio::glib::error::BoolError> {
        Ok(())
    }
}
