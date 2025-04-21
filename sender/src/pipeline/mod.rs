use std::sync::{Arc, Mutex};

use gst::bus::BusWatchGuard;
use gst::glib;
use gst::prelude::*;
use gst_video::VideoFrameExt;
use log::{debug, error};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::Event;

mod sink;

use sink::{HlsSink, WebrtcSink};

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
    pub fn new<App>(
        app: &App,
        event_tx: Sender<Event>,
        selected_rx: Receiver<usize>,
        new_frame_cb: fn(App, slint::Image),
    ) -> Result<Self, gst::glib::BoolError>
    where
        App: slint::ComponentHandle + 'static,
    {
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
        // The software frame renderer is extremely slow, we need downscaling until we get faster frame renderer
        let preview_scale = gst::ElementFactory::make("videoscale")
            .name("preview_scale")
            .build()?;
        let appsink = gst_app::AppSink::builder()
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .format(gst_video::VideoFormat::Rgb)
                    .width(256)
                    .height(256 / 2)
                    .build(),
            )
            .build();

        let app_weak = app.as_weak();
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer_owned().unwrap();
                    let caps = sample.caps().unwrap();
                    let video_info = gst_video::VideoInfo::from_caps(caps).unwrap();
                    let video_frame =
                        gst_video::VideoFrame::from_buffer_readable(buffer, &video_info).unwrap();
                    let slint_frame = match video_frame.format() {
                        gst_video::VideoFormat::Rgb => {
                            let mut slint_pixel_buffer =
                                slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(
                                    video_frame.width(),
                                    video_frame.height(),
                                );
                            video_frame
                                .buffer()
                                .copy_to_slice(0, slint_pixel_buffer.make_mut_bytes())
                                .unwrap();
                            slint_pixel_buffer
                        }
                        _ => unreachable!(),
                    };

                    app_weak
                        .upgrade_in_event_loop(move |app| {
                            new_frame_cb(app, slint::Image::from_rgb8(slint_frame));
                        })
                        .unwrap();

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

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
        });

        let pipeline = gst::Pipeline::new();
        // pipeline.add_many([&src, &tee, &preview_queue, &preview_convert, &gtksink])?;
        pipeline.add_many([&src, &tee, &preview_queue, &preview_convert, &preview_scale])?;
        pipeline.add(&appsink)?;

        gst::Element::link_many([&src, &tee])?;
        // gst::Element::link_many([&preview_queue, &preview_convert, &gtksink])?;
        gst::Element::link_many([&preview_queue, &preview_convert, &preview_scale])?;
        preview_scale.link(&appsink)?;

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

        Ok(Self {
            inner: pipeline,
            sink_state,
            tee,
        })
    }

    pub fn setup_bus_watch(
        &self,
        event_tx: Sender<Event>,
    ) -> Result<BusWatchGuard, glib::BoolError> {
        let bus = self.inner.bus().unwrap();
        let pipeline_weak = self.inner.downgrade();
        let s = Arc::clone(&self.sink_state);
        let bus_watch = bus.add_watch_local(move |_, msg| {
            use gst::MessageView;

            match msg.view() {
                MessageView::StateChanged(state_changed) => {
                    let Some(pipeline) = pipeline_weak.upgrade() else {
                        todo!();
                    };

                    let mut s = s.lock().unwrap();
                    if let SinkState::Hls(ref mut hls) = *s {
                        if state_changed.src() == Some(pipeline.upcast_ref())
                            && state_changed.old() == gst::State::Paused
                            && state_changed.current() == gst::State::Playing
                        {
                            hls.hls.write_manifest_file();
                            event_tx.blocking_send(Event::HlsStreamReady).unwrap();
                        }
                    }
                }
                MessageView::Eos(..) => (), // app.quit()), TODO
                MessageView::Error(err) => {
                    error!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    // app.quit(); TODO
                }
                _ => (),
            };

            glib::ControlFlow::Continue
        })?;

        Ok(bus_watch)
    }

    pub fn add_webrtc_sink(&mut self, event_tx: Sender<Event>) -> Result<(), glib::BoolError> {
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

        debug!("Added WebRTC sink");

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
