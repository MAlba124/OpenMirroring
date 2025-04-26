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

use common::video::GstGlContext;
use fcast_lib::models::PlaybackState;
use gst::prelude::*;
use gst_gl::prelude::*;

use anyhow::{bail, Result};

use log::debug;
use tokio::sync::mpsc::Sender;

pub struct Pipeline {
    inner: gst::Pipeline,
    playbin: gst::Element,
}

impl Pipeline {
    pub fn new(
        appsink: gst::Element,
        gst_gl_context: GstGlContext,
        event_tx: Sender<crate::Event>,
    ) -> Result<Self> {
        let pipeline = gst::Pipeline::new();

        let playbin = gst::ElementFactory::make("playbin3").build()?;

        pipeline.add(&playbin)?;

        pipeline.set_state(gst::State::Ready)?;

        let bus = pipeline.bus().expect("Pipeline without bus");

        bus.set_sync_handler(move |_, msg| {
            use gst::MessageView;

            match msg.view() {
                MessageView::Eos(_) => debug!("EOS"),
                MessageView::Error(err) => debug!("{err}"),
                MessageView::NeedContext(ctx) => {
                    let ctx_type = ctx.context_type();

                    if ctx_type == *gst_gl::GL_DISPLAY_CONTEXT_TYPE {
                        if let Some(element) = msg
                            .src()
                            .and_then(|source| source.downcast_ref::<gst::Element>())
                        {
                            let g = gst_gl_context.lock().unwrap();
                            if let Some((_, gst_gl_display)) = g.as_ref() {
                                let gst_context = gst::Context::new(ctx_type, true);
                                gst_context.set_gl_display(gst_gl_display);
                                element.set_context(&gst_context);
                            }
                        }
                    } else if ctx_type == "gst.gl.app_context" {
                        if let Some(element) = msg
                            .src()
                            .and_then(|source| source.downcast_ref::<gst::Element>())
                        {
                            let mut gst_context = gst::Context::new(ctx_type, true);
                            {
                                let g = gst_gl_context.lock().unwrap();
                                if let Some((gst_gl_context, _)) = g.as_ref() {
                                    let gst_context = gst_context.get_mut().unwrap();
                                    let structure = gst_context.structure_mut();
                                    structure.set("context", gst_gl_context);
                                }
                            }
                            element.set_context(&gst_context);
                        }
                    }
                }
                _ => (),
            }

            gst::BusSyncReply::Drop
        });

        playbin.set_property("video-sink", &appsink);

        {
            let pipeline = pipeline.clone();
            common::runtime().spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

                    let position: Option<gst::ClockTime> = pipeline.query_position();
                    let duration: Option<gst::ClockTime> = pipeline.query_duration();
                    let speed = {
                        let mut query = gst::query::Segment::new(gst::Format::Time);
                        if pipeline.query(&mut query) {
                            query
                                .get_mut()
                                .unwrap() // We know the query succeeded
                                .result()
                                .0
                        } else {
                            1.0f64
                        }
                    };
                    let state = match pipeline.state(gst::ClockTime::NONE).1 {
                        gst::State::Paused => PlaybackState::Paused,
                        gst::State::Playing => PlaybackState::Playing,
                        _ => PlaybackState::Idle,
                    };
                    if event_tx
                        .send(crate::Event::PlaybackUpdate {
                            time: position.unwrap_or_default().seconds_f64(),
                            duration: duration.unwrap_or_default().seconds_f64(),
                            state,
                            speed,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                debug!("Playback update watcher finished");
            });
        }

        Ok(Self {
            inner: pipeline,
            playbin,
        })
    }

    pub fn set_playback_uri(&self, uri: &str) -> Result<()> {
        self.inner.set_state(gst::State::Ready)?;
        self.playbin.set_property("uri", uri);

        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        self.inner.set_state(gst::State::Paused)?;

        Ok(())
    }

    pub fn play_or_resume(&self) -> Result<()> {
        self.inner.set_state(gst::State::Playing)?;

        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.inner.set_state(gst::State::Null)?;
        self.playbin.set_property("uri", "");

        Ok(())
    }

    pub fn set_volume(&self, new_volume: f64) {
        self.playbin
            .set_property("volume", new_volume.clamp(0.0, 1.0));
    }

    pub fn seek(&self, seek_to: f64) -> Result<()> {
        self.inner.seek_simple(
            gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
            gst::ClockTime::from_seconds_f64(seek_to),
        )?;

        Ok(())
    }

    pub fn set_speed(&self, new_speed: f64) -> Result<()> {
        let Some(position) = self.inner.query_position::<gst::ClockTime>() else {
            bail!("Failed to query playback position");
        };

        if new_speed > 0.0 {
            self.inner.seek(
                new_speed,
                gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
                gst::SeekType::Set,
                position,
                gst::SeekType::End,
                gst::ClockTime::ZERO,
            )?;
        } else {
            self.inner.seek(
                new_speed,
                gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
                gst::SeekType::Set,
                gst::ClockTime::ZERO,
                gst::SeekType::End,
                position,
            )?;
        }

        Ok(())
    }
}
