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

use super::TransmissionSink;

use gst::glib;
use gst_rtsp_server::{RTSPMediaFactory, RTSPMountPoints, RTSPServer, prelude::*};

pub struct RtspSink {
    server: RTSPServer,
    id: Option<glib::SourceId>,
    main_loop: glib::MainLoop,
    src_pad: gst::Pad,
    videointersink: gst::Element,
}

impl RtspSink {
    pub fn new(src_pad: gst::Pad, pipeline: &gst::Pipeline, port: u16) -> anyhow::Result<Self> {
        let videointersink = gst::ElementFactory::make("intervideosink").build()?;

        pipeline.add(&videointersink)?;

        let src_pad_block = super::block_downstream(&src_pad)?;
        let intersink_pad = videointersink.static_pad("sink").ok_or(anyhow::anyhow!(
            "Failed to get static sink pad from intersink"
        ))?;
        src_pad.link(&intersink_pad)?;
        src_pad.remove_probe(src_pad_block);

        videointersink.sync_state_with_parent()?;

        let server = RTSPServer::new();

        server.set_service(&format!("{port}"));

        let mounts = RTSPMountPoints::new();
        server.set_mount_points(Some(&mounts));

        let factory = RTSPMediaFactory::default();
        factory.set_shared(true);
        // TODO: vp8 maybe?
        // NOTE: "superfast" speed-preset seems to be fine. "ultrafast" yields no noticeable difference in latency
        factory.set_launch("( intervideosrc ! videoconvert ! videoscale \
                            ! video/x-raw,width=(int)[16,8192,2],height=(int)[16,8192,2] \
                            ! queue ! x264enc tune=zerolatency speed-preset=superfast b-adapt=false key-int-max=2250 \
                            ! video/x-h264,profile=baseline ! rtph264pay config-interval=-1 name=pay0 )");
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

        Ok(Self {
            server,
            id: Some(id),
            main_loop,
            src_pad,
            videointersink,
        })
    }
}

impl TransmissionSink for RtspSink {
    fn get_play_msg(&self, addr: std::net::IpAddr) -> Option<fcast_lib::models::PlayMessage> {
        Some(fcast_lib::models::PlayMessage {
            container: "video/x-rtsp".to_owned(),
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
        let block = super::block_downstream(&self.src_pad)?;
        let intersink_pad = self
            .videointersink
            .static_pad("sink")
            .ok_or(glib::bool_error!(
                "Failed to get static sink pad from intersink"
            ))?;
        self.src_pad.unlink(&intersink_pad)?;
        self.src_pad.remove_probe(block);

        pipeline.remove(&self.videointersink)?;

        self.videointersink
            .set_state(gst::State::Null)
            .map_err(|err| glib::bool_error!("{err}"))?;

        Ok(())
    }
}
