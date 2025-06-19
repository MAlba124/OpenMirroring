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

#[cfg(target_os = "android")]
use super::transmission::rtp::RtpSink;
use super::transmission::{self, TransmissionSink, hls::HlsSink};
use anyhow::Result;
use fcast_lib::models::PlayMessage;
#[cfg(target_os = "android")]
use futures::StreamExt;
use gst::prelude::*;
use log::debug;
use log::error;
#[cfg(target_os = "android")]
use std::future::Future;
use std::net::IpAddr;

pub use transmission::init;

pub enum Event {
    PipelineIsPlaying,
    Eos,
    Error,
}

pub enum SourceConfig {
    AudioVideo {
        video: gst::Element,
        audio: gst::Element,
    },
    Video(gst::Element),
    Audio(gst::Element),
}

#[cfg(not(target_os = "android"))]
pub struct Pipeline {
    inner: gst::Pipeline,
    tx_sink: Box<dyn TransmissionSink>,
}

#[cfg(target_os = "android")]
pub struct Pipeline {
    inner: gst::Pipeline,
    appsrc: gst::Element,
    tx_sink: Option<Box<dyn TransmissionSink>>,
}

impl Pipeline {
    #[cfg(target_os = "android")]
    pub async fn new<E, Fut>(
        frame_rx: crossbeam_channel::Receiver<
            gst_video::VideoFrame<gst_video::video_frame::Writable>,
        >,
        mut on_event: E,
    ) -> Result<Self>
    where
        E: FnMut(Event) -> Fut + Send + Clone + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
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
                    // let frame = match crate::FRAME_CHAN.1.recv() {
                    let frame = match frame_rx.recv() {
                        Ok(frame) => frame,
                        Err(err) => {
                            error!("Failed to receive frame: {err}");
                            let _ = appsrc.end_of_stream();
                            return;
                        }
                    };

                    use gst_video::prelude::*;

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

        pipeline.add_many(&[&appsrc])?;

        let bus = pipeline
            .bus()
            .ok_or(anyhow::anyhow!("Pipeline is missing bus"))?;

        let pipeline_weak = pipeline.downgrade();
        tokio::spawn(async move {
            let mut messages = bus.stream();

            while let Some(msg) = messages.next().await {
                use gst::MessageView;

                match msg.view() {
                    MessageView::Eos(..) => (on_event)(Event::Eos).await,
                    MessageView::Error(err) => {
                        error!(
                            "Error from {:?}: {} ({:?})",
                            err.src().map(|s| s.path_string()),
                            err.error(),
                            err.debug()
                        );
                        (on_event)(Event::Error).await;
                    }
                    MessageView::StateChanged(state_changed) => {
                        let Some(pipeline) = pipeline_weak.upgrade() else {
                            return;
                        };

                        if state_changed.src() == Some(pipeline.upcast_ref())
                            && state_changed.old() == gst::State::Paused
                            && state_changed.current() == gst::State::Playing
                        {
                            (on_event)(Event::PipelineIsPlaying).await;
                        }
                    }
                    _ => (),
                }
            }
        });

        Ok(Self {
            inner: pipeline,
            tx_sink: None,
            appsrc: appsrc.upcast(),
        })
    }

    #[cfg(not(target_os = "android"))]
    pub fn new_rtsp<E>(mut on_event: E, source: SourceConfig) -> Result<Self>
    where
        E: FnMut(Event) + Send + Clone + 'static,
    {
        use std::str::FromStr;

        use crate::sender::transmission::rtsp::RtspSink;

        fn setup_video_source(pipeline: &gst::Pipeline, src: gst::Element) -> Result<gst::Element> {
            let videorate = gst::ElementFactory::make("videorate")
                .property("skip-to-first", true)
                .build()?;
            let capsfilter = gst::ElementFactory::make("capsfilter")
                .name("video_capsfilter")
                .property("caps", gst::Caps::from_str("video/x-raw,framerate=25/1")?)
                .build()?;

            pipeline.add_many([&src, &videorate, &capsfilter])?;
            gst::Element::link_many([&src, &videorate, &capsfilter])?;

            Ok(capsfilter)
        }

        fn setup_audio_source(pipeline: &gst::Pipeline, src: gst::Element) -> Result<gst::Element> {
            let capsfilter = gst::ElementFactory::make("capsfilter")
                .name("audio_capsfilter")
                .property(
                    "caps",
                    gst::Caps::from_str("audio/x-raw,channels=2,rate=48000")?,
                )
                .build()?;

            pipeline.add_many([&src, &capsfilter])?;
            gst::Element::link_many([&src, &capsfilter])?;

            Ok(capsfilter)
        }

        let pipeline = gst::Pipeline::new();

        let source = match source {
            SourceConfig::AudioVideo { video, audio } => SourceConfig::AudioVideo {
                video: setup_video_source(&pipeline, video)?,
                audio: setup_audio_source(&pipeline, audio)?,
            },
            SourceConfig::Video(video) => {
                SourceConfig::Video(setup_video_source(&pipeline, video)?)
            }
            SourceConfig::Audio(audio) => {
                SourceConfig::Audio(setup_audio_source(&pipeline, audio)?)
            }
        };

        let rtsp = RtspSink::new(&pipeline, source, 3000)?;
        let p = Self {
            inner: pipeline.clone(),
            tx_sink: Box::new(rtsp),
        };

        // Start the pipeline in background thread because `scapsrc` initialization will block until
        // the user selects the input source.
        let _ = std::thread::spawn({
            let bus = pipeline
                .bus()
                .ok_or(anyhow::anyhow!("Pipeline without bus"))?;
            // We keep weak pipeline ref because the thread does not receive a finish signal,
            // therefore when we can't upgrade the ref, we know to quit
            let pipeline_weak = pipeline.downgrade();
            move || {
                {
                    let Some(pipeline) = pipeline_weak.upgrade() else {
                        debug!("Failed to upgrade pipeline before starting");
                        return;
                    };
                    debug!("Starting pipeline...");
                    if let Err(err) = pipeline.set_state(gst::State::Playing) {
                        error!("Failed to start pipeline: {err}");
                    } else {
                        debug!("Pipeline started");
                    }
                }

                for msg in bus.iter_timed(gst::ClockTime::NONE) {
                    use gst::MessageView;
                    match msg.view() {
                        MessageView::Eos(..) => (on_event)(Event::Eos),
                        MessageView::Error(err) => {
                            error!(
                                "Error from {:?}: {} ({:?})",
                                err.src().map(|s| s.path_string()),
                                err.error(),
                                err.debug()
                            );
                            (on_event)(Event::Error);
                        }
                        MessageView::StateChanged(state_changed) => {
                            let Some(pipeline) = pipeline_weak.upgrade() else {
                                debug!(
                                    "Failed to handle state change bus message because pipeline is missing"
                                );
                                return;
                            };

                            if state_changed.src() == Some(pipeline.upcast_ref())
                                && state_changed.old() == gst::State::Paused
                                && state_changed.current() == gst::State::Playing
                            {
                                (on_event)(Event::PipelineIsPlaying);
                            }
                        }
                        _ => (),
                    }
                }

                debug!("Bus watcher quit");
            }
        });

        Ok(p)
    }

    pub fn playing(&mut self) -> Result<()> {
        self.tx_sink.playing()
    }

    #[cfg(target_os = "android")]
    pub fn start(&self) -> Result<()> {
        use anyhow::bail;

        if let Err(err) = self.inner.set_state(gst::State::Playing) {
            bail!("{err}")
        } else {
            Ok(())
        }
    }

    #[cfg(not(target_os = "android"))]
    pub fn shutdown(&mut self) -> Result<()> {
        self.inner.set_state(gst::State::Null)?;
        self.tx_sink.shutdown();

        Ok(())
    }

    #[cfg(target_os = "android")]
    pub fn add_hls_sink(&mut self) -> Result<()> {
        let appsrc_pad = self
            .appsrc
            .static_pad("src")
            .ok_or(anyhow::anyhow!("appsrc is missing src pad"))?;
        self.tx_sink = Some(Box::new(HlsSink::new(&self.inner, appsrc_pad, 5004)?));

        self.inner.set_state(gst::State::Playing)?; // NOTE: beware this is here

        log::debug!("Added HLS sink");

        Ok(())
    }

    #[cfg(target_os = "android")]
    pub fn add_rtp_sink(&mut self, port: u16, receiver_addr: IpAddr) -> Result<()> {
        let appsrc_pad = self
            .appsrc
            .static_pad("src")
            .ok_or(anyhow::anyhow!("appsrc is missing src pad"))?;
        let rtp = RtpSink::new(&self.inner, appsrc_pad, port, receiver_addr)?;
        self.tx_sink = Some(Box::new(rtp));

        debug!("Added RTP sink");

        Ok(())
    }

    /// Get the message that should be sent to a receiver to consume the stream if a transmission
    /// sink is present
    pub fn get_play_msg(&self, addr: IpAddr) -> Option<PlayMessage> {
        self.tx_sink.get_play_msg(addr)
    }
}
