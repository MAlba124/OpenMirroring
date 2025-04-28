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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use common::video::GstGlContext;
use gst::glib;
use gst::prelude::*;
use log::{debug, error};
use tokio::sync::mpsc::{Receiver, Sender};
use transmission_sink::TransmissionSink;
use transmission_sink::{hls::HlsSink, webrtc::WebrtcSink};

use gst_gl::prelude::*;

use crate::Event;

mod transmission_sink;

pub struct Pipeline {
    inner: gst::Pipeline,
    tx_sink: Arc<tokio::sync::Mutex<Option<Box<dyn TransmissionSink>>>>,
    tee: gst::Element,
    preview_queue: gst::Element,
    preview_appsink: gst::Element,
}

impl Pipeline {
    pub fn new(
        event_tx: Sender<Event>,
        selected_rx: Receiver<usize>,
        preview_appsink: gst::Element,
        gst_gl_context: GstGlContext,
    ) -> Result<Self> {
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

        let tx_sink = Arc::new(tokio::sync::Mutex::new(None::<Box<dyn TransmissionSink>>));

        let bus = pipeline
            .bus()
            .ok_or(glib::bool_error!("Pipeline is missing bus"))?;

        let pipeline_weak = pipeline.downgrade();
        let tx_sink_clone = Arc::clone(&tx_sink);
        let event_tx_clone = event_tx.clone();
        let pipeline_has_finished = AtomicBool::new(false);
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
                        let tx_sink = tx_sink_clone.clone();
                        // The HLS sink needs to know of the state change message
                        common::runtime().spawn(async move {
                            let mut s = tx_sink.lock().await;
                            if let Some(ref mut sink) = *s {
                                sink.playing().await;
                            }
                        });
                    }
                }
                MessageView::Eos(..) => {
                    if !pipeline_has_finished.load(Ordering::Acquire) {
                        event_tx_clone
                            .blocking_send(crate::Event::PipelineFinished)
                            .unwrap();
                        pipeline_has_finished.store(true, Ordering::Release);
                    }
                }
                MessageView::Error(err) => {
                    error!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    if !pipeline_has_finished.load(Ordering::Acquire) {
                        event_tx_clone
                            .blocking_send(crate::Event::PipelineFinished)
                            .unwrap();
                        pipeline_has_finished.store(true, Ordering::Release);
                    }
                }
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
            };

            gst::BusSyncReply::Drop
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

        let selected_rx = Arc::new(Mutex::new(selected_rx));
        let event_tx_clone = event_tx.clone();
        scapsrc.connect("select-source", false, move |vals| {
            let event_tx = event_tx_clone.clone();
            let selected_rx = Arc::clone(&selected_rx);

            let sources = vals[1].get::<Vec<String>>().unwrap();
            event_tx.blocking_send(Event::Sources(sources)).unwrap();
            let mut selected_rx = selected_rx.lock().unwrap();
            let res = selected_rx.blocking_recv().unwrap() as u64;
            Some(res.to_value())
        });

        pipeline.add_many([&scapsrc, &tee, &preview_queue, &preview_appsink])?;
        gst::Element::link_many([&scapsrc, &tee])?;
        gst::Element::link_many([&preview_queue, &preview_appsink])?;

        let tee_preview_pad = tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let queue_preview_pad = preview_queue.static_pad("sink").ok_or(glib::bool_error!(
            "preview_queue is missing static sink pad"
        ))?;
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

    pub async fn shutdown(&self) -> Result<()> {
        self.inner.set_state(gst::State::Null)?;

        self.preview_queue.unlink(&self.preview_appsink);
        self.inner.remove(&self.preview_appsink)?;

        let mut tx_sink = self.tx_sink.lock().await;
        if let Some(sink) = &mut (*tx_sink) {
            sink.shutdown();
        }

        Ok(())
    }

    pub async fn add_webrtc_sink(&mut self) -> Result<()> {
        let tee_pad = self.tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let webrtc = WebrtcSink::new(&self.inner, tee_pad)?;
        let mut s = self.tx_sink.lock().await;
        *s = Some(Box::new(webrtc));

        debug!("Added WebRTC sink");

        Ok(())
    }

    pub async fn add_hls_sink(&mut self) -> Result<()> {
        let tee_pad = self.tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let hls = HlsSink::new(&self.inner, tee_pad)?;
        let mut s = self.tx_sink.lock().await;
        *s = Some(Box::new(hls));

        debug!("Added HLS sink");

        Ok(())
    }

    pub async fn remove_transmission_sink(&self) -> Result<()> {
        let mut tx_sink = self.tx_sink.lock().await;
        if let Some(sink) = &mut (*tx_sink) {
            sink.shutdown();
            sink.unlink(&self.inner)?;
        }

        *tx_sink = None;

        Ok(())
    }

    /// Get the message that should be sent to a receiver to consume the stream if a transmission
    /// sink is present
    pub async fn get_play_msg(&self) -> Option<crate::SessionMessage> {
        let tx_sink = self.tx_sink.lock().await;
        if let Some(sink) = &(*tx_sink) {
            sink.get_play_msg()
        } else {
            None
        }
    }
}
