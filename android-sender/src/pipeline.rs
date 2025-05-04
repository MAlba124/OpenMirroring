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

use common::transmission::{hls::HlsSink, webrtc::WebrtcSink, TransmissionSink};
use gst::{glib, prelude::*};

use anyhow::{bail, Result};
use gst_video::VideoFrameExt;
use log::error;

pub struct Pipeline {
    inner: gst::Pipeline,
    appsrc: gst::Element,
    sink: Option<Box<dyn TransmissionSink>>,
}

fn current_time_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

impl Pipeline {
    pub fn new() -> Result<Self> {
        // let appsrc = gst::ElementFactory::make("videotestsrc")
        // .build()?;

        let appsrc = gst_app::AppSrc::builder()
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .format(gst_video::VideoFormat::Rgba)
                    .build(),
            )
            .is_live(true)
            .do_timestamp(true)
            .format(gst::Format::Time)
            .max_buffers(1)
            .build();

        let mut caps = None::<gst::Caps>;
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

                    let now_caps = gst_video::VideoInfo::builder(
                        frame.format(),
                        frame.width(),
                        frame.height(),
                    )
                    .build()
                    .unwrap()
                    .to_caps()
                    .unwrap();

                    match &caps {
                        Some(old_caps) => {
                            if *old_caps != now_caps {
                                appsrc.set_caps(Some(&now_caps));
                                caps = Some(now_caps);
                            }
                        }
                        None => {
                            appsrc.set_caps(Some(&now_caps));
                            caps = Some(now_caps);
                        }
                    }

                    let _ = appsrc.push_buffer(frame.into_buffer());
                })
                .build(),
        );

        let pipeline = gst::Pipeline::new();

        // pipeline.add_many(&[appsrc.upcast_ref(), &convert])?;
        // gst::Element::link_many(&[appsrc.upcast_ref(), &convert])?;

        pipeline.add_many(&[&appsrc])?;
        // gst::Element::link_many(&[&appsrc])?;

        // let sink = common::transmission::hls::HlsSink::new(
        //     &pipeline,
        //     convert
        //         .static_pad("src")
        //         .ok_or(glib::bool_error!("Convert has not src pad"))?,
        // )?;

        // let tx_queue = gst::ElementFactory::make("queue")
        //     .property("silent", true)
        //     .build()?;

        let bus = pipeline
            .bus()
            .ok_or(glib::bool_error!("Pipeline is missing bus"))?;

        let pipeline_weak = pipeline.downgrade();
        bus.set_sync_handler(move |_, msg| {
            use gst::MessageView;

            match msg.view() {
                MessageView::StateChanged(state_changed) => {
                    let Some(pipeline) = pipeline_weak.upgrade() else {
                        return gst::BusSyncReply::Drop;
                    };

                    if state_changed.src() == Some(pipeline.upcast_ref())
                        && state_changed.old() == gst::State::Paused
                        && state_changed.current() == gst::State::Playing
                    {
                        crate::tx!()
                            .send_blocking(crate::Event::PipelineIsPlaying)
                            .unwrap();
                    }
                }
                _ => (),
            }

            gst::BusSyncReply::Drop
        });

        // pipeline.add(&tx_queue)?;

        Ok(Self {
            inner: pipeline,
            // sink: Box::new(sink),
            sink: None,
            appsrc: appsrc.upcast(),
            // convert,
        })
    }

    pub async fn playing(&mut self) {
        match &mut self.sink {
            Some(sink) => sink.playing().await,
            None => error!("No sink available to send playing signal to"),
        }
    }

    pub fn add_webrtc_sink(&mut self) -> Result<()> {
        // let Some(convert_pad) = self.convert.static_pad("src") else {
        let Some(appsrc_pad) = self.appsrc.static_pad("src") else {
            bail!("appsrc is missing src pad");
        };
        self.sink = Some(Box::new(WebrtcSink::new(&self.inner, appsrc_pad)?));

        // self.inner.set_state(gst::State::Playing)?;

        Ok(())
    }

    pub fn add_hls_sink(&mut self) -> Result<()> {
        let Some(appsrc_pad) = self.appsrc.static_pad("src") else {
            bail!("appsrc is missing src pad");
        };
        self.sink = Some(Box::new(HlsSink::new(&self.inner, appsrc_pad)?));

        self.inner.set_state(gst::State::Playing)?;

        Ok(())
    }

    pub fn get_play_msg(&self) -> Option<crate::SessionMessage> {
        if let Some(sink) = &self.sink {
            let msg = sink.get_play_msg()?;
            Some(crate::SessionMessage::Play {
                mime: msg.mime,
                uri: msg.uri,
            })
        } else {
            None
        }
    }
}
