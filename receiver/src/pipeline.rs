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

use fcast_lib::models::{PlaybackState, PlaybackUpdateMessage};
use futures::StreamExt;
use gst::prelude::*;

use anyhow::{anyhow, bail, Result};

use log::{debug, error};
use tokio::sync::mpsc::Sender;

pub struct Pipeline {
    inner: gst::Pipeline,
    playbin: gst::Element,
}

impl Pipeline {
    pub async fn new(appsink: gst::Element, event_tx: Sender<crate::Event>) -> Result<Self> {
        let pipeline = gst::Pipeline::new();

        let playbin = gst::ElementFactory::make("playbin3").build()?;

        pipeline.add(&playbin)?;

        pipeline.set_state(gst::State::Ready)?;

        tokio::spawn({
            let bus = pipeline.bus().ok_or(anyhow!("Pipeline without bus"))?;
            let event_tx = event_tx.clone();

            async move {
                let mut messages = bus.stream();

                while let Some(msg) = messages.next().await {
                    use gst::MessageView;

                    match msg.view() {
                        MessageView::Eos(..) => {
                            event_tx.send(crate::Event::PipelineEos).await.unwrap()
                        }
                        MessageView::Error(err) => {
                            error!(
                                "Error from {:?}: {} ({:?})",
                                err.src().map(|s| s.path_string()),
                                err.error(),
                                err.debug()
                            );
                            event_tx.send(crate::Event::PipelineError).await.unwrap();
                        }
                        _ => (),
                    }
                }
            }
        });

        playbin.set_property("video-sink", &appsink);

        Ok(Self {
            inner: pipeline,
            playbin,
        })
    }

    pub fn get_playback_state(&self) -> Result<PlaybackUpdateMessage> {
        let position: Option<gst::ClockTime> = self.inner.query_position();
        let duration: Option<gst::ClockTime> = self.inner.query_duration();

        let speed = {
            let mut query = gst::query::Segment::new(gst::Format::Time);
            if self.inner.query(&mut query) {
                query
                    .get_mut()
                    .unwrap() // We know the query succeeded
                    .result()
                    .0
            } else {
                1.0f64
            }
        };

        let state = match self.inner.state(gst::ClockTime::NONE).1 {
            gst::State::Paused => PlaybackState::Paused,
            gst::State::Playing => PlaybackState::Playing,
            _ => PlaybackState::Idle,
        };

        fn current_time_millis() -> u64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
        }

        Ok(PlaybackUpdateMessage {
            time: position.unwrap_or_default().seconds_f64(),
            duration: duration.unwrap_or_default().seconds_f64(),
            state,
            speed,
            generation: current_time_millis(),
        })
    }

    pub fn set_playback_uri(&self, uri: &str) -> Result<()> {
        self.inner.set_state(gst::State::Ready)?;
        self.playbin.set_property("uri", uri);

        debug!("Playback URI set to: {uri}");

        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        self.inner.set_state(gst::State::Paused)?;

        debug!("Playback paused");

        Ok(())
    }

    pub fn play_or_resume(&self) -> Result<()> {
        self.inner.set_state(gst::State::Playing)?;

        debug!("Playback resumed");

        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.inner.set_state(gst::State::Null)?;
        self.playbin.set_property("uri", "");

        debug!("Playback stopped");

        Ok(())
    }

    pub fn set_volume(&self, new_volume: f64) {
        self.playbin
            .set_property("volume", new_volume.clamp(0.0, 1.0));

        debug!("Volume set to {}", new_volume.clamp(0.0, 1.0));
    }

    pub fn seek(&self, seek_to: f64) -> Result<()> {
        self.inner.seek_simple(
            gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
            gst::ClockTime::from_seconds_f64(seek_to),
        )?;

        debug!("Seeked to: {seek_to}");

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

        debug!("Playback speed set to: {new_speed}");

        Ok(())
    }
}
