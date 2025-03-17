use gtk4 as gtk;

use gtk::prelude::*;
use std::sync::{Arc, Mutex};

use gst::bus::BusWatchGuard;
use gst::glib;
use gst::prelude::*;
use log::{debug, error};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::Event;

mod sink;

use sink::{HlsSink, WebrtcSink};

#[allow(dead_code)]
enum SinkState {
    Unset,
    WebRTC(WebrtcSink),
    Hls(HlsSink),
}

pub struct Pipeline {
    pub inner: gst::Pipeline,
    sink_state: Arc<Mutex<SinkState>>,
    tee: gst::Element,
}

impl Pipeline {
    pub fn new(
        event_tx: Sender<Event>,
        selected_rx: Receiver<usize>,
    ) -> Result<(Self, gst_gtk4::RenderWidget), gst::glib::BoolError> {
        let tee = gst::ElementFactory::make("tee").build()?;
        let src = gst::ElementFactory::make("scapsrc")
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
        let preview_convert = gst::ElementFactory::make("videoconvert")
            .name("preview_convert")
            .build()?;
        let gtksink = gst::ElementFactory::make("gtk4paintablesink")
            .name("gtksink")
            .build()?;

        // https://gitlab.freedesktop.org/gstreamer/gstreamer/-/issues/3993
        src.static_pad("src").unwrap().add_probe(
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
        src.connect("select-source", false, move |vals| {
            let event_tx = event_tx_clone.clone();
            let selected_rx = Arc::clone(&selected_rx);

            let sources = vals[1].get::<Vec<String>>().unwrap();
            event_tx.blocking_send(Event::Sources(sources)).unwrap();
            let mut selected_rx = selected_rx.lock().unwrap();
            let res = selected_rx.blocking_recv().unwrap() as u64;
            Some(res.to_value())

            // om_common::runtime().block_on(async move {
            //     let sources = vals[1].get::<Vec<String>>().unwrap();
            //     event_tx.send(Event::Sources(sources)).await.unwrap();
            //     let mut selected_rx = selected_rx.lock().unwrap();
            //     let res = selected_rx.recv().await.unwrap() as u64;
            //     Some(res.to_value())
            // })
        });

        let pipeline = gst::Pipeline::new();
        pipeline.add_many([&src, &tee, &preview_queue, &preview_convert, &gtksink])?;

        gst::Element::link_many([&src, &tee])?;
        gst::Element::link_many([&preview_queue, &preview_convert, &gtksink])?;

        let tee_preview_pad = tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let queue_preview_pad = preview_queue
            .static_pad("sink")
            .map_or_else(|| Err(glib::bool_error!("`static_pad()` failed")), Ok)?;
        tee_preview_pad
            .link(&queue_preview_pad)
            .map_err(|err| glib::bool_error!("{err}"))?;

        let widget = gst_gtk4::RenderWidget::new(&gtksink);

        let pipeline_weak = pipeline.downgrade();
        // Start pipeline in background to not freeze UI
        let _ = std::thread::spawn(move || {
            let Some(pipeline) = pipeline_weak.upgrade() else {
                panic!("No pipeline");
            };
            debug!("Starting pipeline");
            pipeline.set_state(gst::State::Playing).unwrap();
        });

        let sink_state = Arc::new(Mutex::new(SinkState::Unset));

        Ok((
            Self {
                inner: pipeline,
                sink_state,
                tee,
            },
            widget,
        ))
    }

    pub fn setup_bus_watch(
        &self,
        app_weak: glib::WeakRef<gtk::Application>,
        event_tx: Sender<Event>,
    ) -> Result<BusWatchGuard, glib::BoolError> {
        let bus = self.inner.bus().unwrap();
        let pipeline_weak = self.inner.downgrade();
        let s = Arc::clone(&self.sink_state);
        let bus_watch = bus.add_watch_local(move |_, msg| {
            use gst::MessageView;

            let Some(app) = app_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            match msg.view() {
                MessageView::StateChanged(state_changed) => {
                    let Some(pipeline) = pipeline_weak.upgrade() else {
                        todo!();
                    };

                    let mut s = s.lock().unwrap();
                    match *s {
                        SinkState::Hls(ref mut hls) => {
                            if state_changed.src() == Some(pipeline.upcast_ref())
                                && state_changed.old() == gst::State::Paused
                                && state_changed.current() == gst::State::Playing
                            {
                                hls.hls.write_manifest_file();
                                event_tx.blocking_send(Event::HlsStreamReady).unwrap();
                                // om_common::runtime().block_on(async {
                                // event_tx.send(Event::HlsStreamReady).await.unwrap();
                                // });
                            }
                        }
                        _ => (),
                    }
                }
                MessageView::Eos(..) => app.quit(),
                MessageView::Error(err) => {
                    error!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    app.quit();
                }
                _ => (),
            };

            glib::ControlFlow::Continue
        })?;

        Ok(bus_watch)
    }

    pub fn add_webrtc_sink(&mut self, event_tx: Sender<Event>) -> Result<(), glib::BoolError> {
        debug!("Adding WebRTC sink");
        let tee_pad = self.tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let webrtc_ = WebrtcSink::new(&self.inner, event_tx)?;
        let queue_pad = webrtc_
            .queue
            .static_pad("sink")
            .map_or_else(|| Err(glib::bool_error!("`static_pad()` failed")), Ok)?;
        tee_pad
            .link(&queue_pad)
            .map_err(|err| glib::bool_error!("{err}"))?;
        let mut s = self
            .sink_state
            .lock()
            .map_err(|err| glib::bool_error!("{err}"))?;
        *s = SinkState::WebRTC(webrtc_);

        Ok(())
    }

    pub fn add_hls_sink(&mut self, event_tx: Sender<Event>) -> Result<(), glib::BoolError> {
        debug!("Adding HLS sink");
        let tee_pad = self.tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let hls_ = HlsSink::new(&self.inner, event_tx)?;
        let queue_pad = hls_
            .queue
            .static_pad("sink")
            .map_or_else(|| Err(glib::bool_error!("`static_pad()` failed")), Ok)?;
        tee_pad
            .link(&queue_pad)
            .map_err(|err| glib::bool_error!("{err}"))?;

        let mut s = self
            .sink_state
            .lock()
            .map_err(|err| glib::bool_error!("{err}"))?;
        *s = SinkState::Hls(hls_);

        Ok(())
    }

    pub fn get_play_msg(&self) -> Option<crate::Message> {
        let s = self.sink_state.lock().unwrap();
        match *s {
            SinkState::Unset => unreachable!(),
            SinkState::WebRTC(ref sink) => sink.get_play_msg(),
            SinkState::Hls(ref sink) => sink.get_play_msg(),
        }
    }

    pub fn shutdown(&self) {
        let s = self.sink_state.lock().unwrap();
        match *s {
            SinkState::Hls(ref sink) => sink.hls.shutdown(),
            _ => (),
        }
    }

    pub fn set_producer_id(&mut self, producer_id: String) {
        let mut s = self.sink_state.lock().unwrap();
        match *s {
            SinkState::WebRTC(ref mut sink) => sink.producer_id = Some(producer_id),
            _ => error!("Attempted to set producer peer id for non WebRTC sink"),
        }
    }

    pub fn set_server_port(&mut self, port: u16) {
        let mut s = self.sink_state.lock().unwrap();
        match *s {
            SinkState::Hls(ref mut sink) => sink.server_port = Some(port),
            _ => error!("Attempted to set server port for non HLS sink"),
        }
    }
}
