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

use super::transmission::{self, TransmissionSink, hls::HlsSink, rtp::RtpSink};
use anyhow::Result;
use fcast_lib::models::PlayMessage;
use futures::StreamExt;
use gst::prelude::*;
use log::debug;
use log::error;
use std::{future::Future, net::IpAddr};

pub use transmission::init;

pub enum Event {
    PipelineIsPlaying,
    Eos,
    Error,
}

#[cfg(not(target_os = "android"))]
pub struct Pipeline {
    inner: gst::Pipeline,
    tx_sink: Option<Box<dyn TransmissionSink>>,
    tee: gst::Element,
    preview_queue: gst::Element,
    preview_appsink: gst::Element,
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
    pub async fn new<E, Fut, S>(
        preview_appsink: gst::Element,
        mut on_event: E,
        on_sources: S,
    ) -> Result<Self>
    where
        E: FnMut(Event) -> Fut + Send + Clone + 'static,
        Fut: Future<Output = ()> + Send + 'static,
        S: Fn(&[gst::glib::Value]) -> Option<gst::glib::Value> + Send + Sync + 'static,
    {
        let tee = gst::ElementFactory::make("tee").build()?;
        let scapsrc = gst::ElementFactory::make("scapsrc")
            .property("perform-internal-preroll", true)
            .build()?;
        let preview_queue = gst::ElementFactory::make("queue")
            .name("preview_queue")
            .property("max-size-time", 0u64)
            .property("max-size-buffers", 0u32)
            .property("max-size-bytes", 0u32)
            .property_from_str("leaky", "downstream")
            .property("silent", true) // Don't emit signals, can give better perf.
            .build()?;

        let pipeline = gst::Pipeline::new();

        let tx_sink = None::<Box<dyn TransmissionSink>>;

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
                            debug!(
                                "Failed to handle state change bus message because pipeline is missing"
                            );
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

        // https://gitlab.freedesktop.org/gstreamer/gstreamer/-/issues/3993
        scapsrc.static_pad("src").unwrap().add_probe(
            gst::PadProbeType::QUERY_UPSTREAM.union(gst::PadProbeType::PUSH),
            |_pad, info| match info.query_mut().map(|query| query.view_mut()) {
                Some(gst::QueryViewMut::Latency(latency)) => {
                    let (_live, min, max) = latency.result();
                    latency.set(false, min, max);
                    gst::PadProbeReturn::Handled
                }
                _ => gst::PadProbeReturn::Pass,
            },
        );

        scapsrc.connect("select-source", false, on_sources);

        pipeline.add_many([&scapsrc, &tee, &preview_queue, &preview_appsink])?;
        gst::Element::link_many([&scapsrc, &tee])?;
        gst::Element::link_many([&preview_queue, &preview_appsink])?;

        let tee_preview_pad = tee
            .request_pad_simple("src_%u")
            .map_or_else(|| Err(anyhow::anyhow!("`request_pad_simple()` failed")), Ok)?;
        let queue_preview_pad = preview_queue
            .static_pad("sink")
            .ok_or(anyhow::anyhow!("preview_queue is missing static sink pad"))?;
        tee_preview_pad.link(&queue_preview_pad)?;

        // Start the pipeline in background thread because `scapsrc` initialization will block until
        // the user selects the input source.
        let _ = std::thread::spawn({
            let pipeline = pipeline.clone();
            move || {
                debug!("Starting pipeline");
                if let Err(err) = pipeline.set_state(gst::State::Playing) {
                    error!("Failed to start pipeline: {err}");
                }
            }
        });

        Ok(Self {
            inner: pipeline,
            tx_sink,
            tee,
            preview_queue,
            preview_appsink,
        })
    }

    pub async fn playing(&mut self) -> Result<()> {
        match &mut self.tx_sink {
            Some(sink) => sink.playing(),
            None => Ok(()),
        }
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

        self.preview_queue.unlink(&self.preview_appsink);
        self.inner.remove(&self.preview_appsink)?;

        if let Some(sink) = &mut self.tx_sink {
            sink.shutdown();
        }

        Ok(())
    }

    #[cfg(not(target_os = "android"))]
    pub fn add_hls_sink(&mut self, port: u16) -> Result<()> {
        let tee_pad = self
            .tee
            .request_pad_simple("src_%u")
            .ok_or(anyhow::anyhow!("`request_pad_simple()` failed"))?;
        let hls = HlsSink::new(&self.inner, tee_pad, port)?;
        self.tx_sink = Some(Box::new(hls));

        debug!("Added HLS sink");

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

    #[cfg(not(target_os = "android"))]
    pub fn add_rtp_sink(&mut self, port: u16, receiver_addr: IpAddr) -> Result<()> {
        let tee_pad = self
            .tee
            .request_pad_simple("src_%u")
            .ok_or(anyhow::anyhow!("`request_pad_simple()` failed"))?;
        let rtp = RtpSink::new(&self.inner, tee_pad, port, receiver_addr)?;
        self.tx_sink = Some(Box::new(rtp));

        debug!("Added RTP sink");

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

    pub fn remove_transmission_sink(&mut self) -> Result<()> {
        if let Some(sink) = &mut self.tx_sink {
            sink.shutdown();
            sink.unlink(&self.inner)?;
        }

        self.tx_sink = None;

        Ok(())
    }

    /// Get the message that should be sent to a receiver to consume the stream if a transmission
    /// sink is present
    pub fn get_play_msg(&self, addr: IpAddr) -> Option<PlayMessage> {
        if let Some(sink) = &self.tx_sink {
            sink.get_play_msg(addr)
        } else {
            None
        }
    }
}
