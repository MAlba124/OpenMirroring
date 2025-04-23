use std::sync::Arc;

use fcast_lib::models::PlaybackState;
use gst::glib;
use gst::prelude::*;
use log::{debug, error};
use tokio::sync::mpsc::Sender;

use crate::AtomicF64;
use crate::Event;
use crate::GuiEvent;

pub struct VideoView {
    pub pipeline: gst::Pipeline,
    pub playbin: gst::Element,
    // widget: gst_gtk4::RenderWidget,
}

impl VideoView {
    pub fn new() -> Result<Self, glib::BoolError> {
        let pipeline = gst::Pipeline::new();
        let gtksink = gst::ElementFactory::make("gtk4paintablesink").build()?;

        let playbin = gst::ElementFactory::make("playbin3").build()?;

        let sink = gst::Bin::default();
        let convert = gst::ElementFactory::make("videoconvert").build()?;

        sink.add(&convert)?;
        sink.add(&gtksink)?;
        convert.link(&gtksink)?;

        sink.add_pad(&gst::GhostPad::with_target(
            &convert.static_pad("sink").map_or_else(
                || {
                    Err(glib::bool_error!(
                        "Failed to get sink pad from `videoconvert`"
                    ))
                },
                Ok,
            )?,
        )?)?;

        playbin.set_property("video-sink", &sink);

        pipeline.add(&playbin)?;

        // let widget = gst_gtk4::RenderWidget::new(&gtksink);

        pipeline
            .set_state(gst::State::Ready)
            .map_err(|err| glib::bool_error!("{err}"))?;

        Ok(Self {
            pipeline,
            playbin,
            // widget,
        })
    }

    pub fn setup_bus_watch(
        &self,
        gui_event_tx: Sender<GuiEvent>,
    ) -> Result<gst::bus::BusWatchGuard, glib::BoolError> {
        let bus = self.pipeline.bus().unwrap();
        let pipeline_weak = self.pipeline.downgrade();
        let bus_watch = bus.add_watch_local(move |_, msg| {
            use gst::MessageView;

            let Some(pipeline) = pipeline_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            match msg.view() {
                MessageView::Eos(..) => {
                    debug!("Reached EOS");
                    pipeline
                        .set_state(gst::State::Ready)
                        .map_err(|_| glib::ControlFlow::Break)
                        .unwrap(); // TODO
                    let get_clone = gui_event_tx.clone();
                    common::runtime().spawn(async move {
                        get_clone.send(GuiEvent::Eos).await.unwrap();
                    });
                }
                MessageView::Error(err) => {
                    error!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    pipeline
                        .set_state(gst::State::Null)
                        .map_err(|_| glib::ControlFlow::Break)
                        .unwrap(); // TODO: notify GUI
                }
                _ => (),
            };
            glib::ControlFlow::Continue
        });

        bus_watch
    }

    pub fn setup_timeout(
        &self,
        event_tx: tokio::sync::mpsc::Sender<Event>,
        playback_speed: Arc<AtomicF64>,
    ) -> glib::SourceId {
        let pipeline_weak = self.pipeline.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(1000), move || {
            let event_tx = event_tx.clone();
            common::runtime().block_on(async {
                let Some(pipeline) = pipeline_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };

                let position: Option<gst::ClockTime> = pipeline.query_position();
                let duration: Option<gst::ClockTime> = pipeline.query_duration();
                let speed = playback_speed.load(std::sync::atomic::Ordering::SeqCst);
                let state = match pipeline.state(gst::ClockTime::NONE).1 {
                    gst::State::Paused => PlaybackState::Paused,
                    gst::State::Playing => PlaybackState::Playing,
                    _ => PlaybackState::Idle,
                };
                event_tx
                    .send(Event::PlaybackUpdate {
                        time: position.unwrap_or_default().seconds_f64(),
                        duration: duration.unwrap_or_default().seconds_f64(),
                        state,
                        speed,
                    })
                    .await
                    .unwrap();
                glib::ControlFlow::Continue
            })
        })
    }

    pub fn main_widget(&self) -> &gst_gtk4::RenderWidget {
        &self.widget
    }
}
