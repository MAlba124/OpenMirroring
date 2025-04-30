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

use gst::{glib, prelude::*};

use anyhow::Result;
use log::error;

pub struct Pipeline {
    inner: gst::Pipeline,
    sink: Box<dyn common::transmission::TransmissionSink>,
}

impl Pipeline {
    pub fn new() -> Result<Self> {
        let appsrc = gst_app::AppSrc::builder()
            .is_live(true)
            .format(gst::Format::Time)
            .build();
        let convert = gst::ElementFactory::make("videoconvert").build()?;

        appsrc.set_callbacks(
            gst_app::AppSrcCallbacks::builder()
                .need_data(move |appsrc, _| {
                    let frame = match crate::FRAME_CHAN.1.recv() {
                        Ok(frame) => frame,
                        Err(err) => {
                            error!("Failed to receive frame: {err}");
                            let _ = appsrc.end_of_stream();
                            return;
                        }
                    };

                    let _ = appsrc.push_buffer(frame);
                })
                .build(),
        );

        let pipeline = gst::Pipeline::new();

        pipeline.add_many(&[appsrc.upcast_ref(), &convert])?;
        gst::Element::link_many(&[appsrc.upcast_ref(), &convert])?;

        let sink = common::transmission::hls::HlsSink::new(
            &pipeline,
            convert
                .static_pad("src")
                .ok_or(glib::bool_error!("Convert has not src pad"))?,
        )?;

        // let tx_queue = gst::ElementFactory::make("queue")
        //     .property("silent", true)
        //     .build()?;

        let bus = pipeline
            .bus()
            .ok_or(glib::bool_error!("Pipeline is missing bus"))?;

        bus.set_sync_handler(move |_, msg| gst::BusSyncReply::Drop);

        // pipeline.add(&tx_queue)?;

        Ok(Self {
            inner: pipeline,
            sink: Box::new(sink),
        })
    }
}
