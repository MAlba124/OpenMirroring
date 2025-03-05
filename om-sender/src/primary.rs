use std::sync::{Arc, Mutex};

use gst::{bus::BusWatchGuard, glib, prelude::*};
use gtk::prelude::*;
use gtk4 as gtk;
use log::error;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::Event;

pub struct PrimaryView {
    pub pipeline: gst::Pipeline,
    vbox: gtk::Box,
    pub preview_stack: gtk::Stack,
    pub gst_widget: gst_gtk4::RenderWidget,
    pub preview_disabled_label: gtk::Label,
}

impl PrimaryView {
    pub fn new(
        event_tx: Sender<Event>,
        selected_rx: Receiver<usize>,
    ) -> Result<Self, glib::BoolError> {
        let tee = gst::ElementFactory::make("tee").build()?;
        let src = gst::ElementFactory::make("scapsrc")
            .property("perform-internal-preroll", true)
            .build()?;

        let preview_queue = gst::ElementFactory::make("queue")
            .name("preview_queue")
            .build()?;
        let preview_convert = gst::ElementFactory::make("videoconvert")
            .name("preview_convert")
            .build()?;
        let gtksink = gst::ElementFactory::make("gtk4paintablesink")
            .name("gtksink")
            .build()?;

        let webrtcsink_queue = gst::ElementFactory::make("queue")
            .name("webrtcsink_queue")
            .build()?;
        let webrtcsink_convert = gst::ElementFactory::make("videoconvert")
            .name("webrtcsink_convert")
            .build()?;
        let webrtcsink = gst::ElementFactory::make("webrtcsink")
            .property("signalling-server-host", "127.0.0.1")
            .property("signalling-server-port", 8443u32)
            .build()?;

        let selected_rx = Arc::new(Mutex::new(selected_rx));
        let event_tx_clone = event_tx.clone();
        src.connect("select-source", false, move |vals| {
            // let sources_tx = sources_tx.clone();
            let event_tx = event_tx_clone.clone();
            let selected_rx = Arc::clone(&selected_rx);
            om_common::runtime().block_on(async move {
                let sources = vals[1].get::<Vec<String>>().unwrap();
                event_tx.send(Event::Sources(sources)).await.unwrap();
                let mut selected_rx = selected_rx.lock().unwrap();
                let res = selected_rx.recv().await.unwrap() as u64;
                Some(res.to_value())
            })
        });

        let pipeline = gst::Pipeline::new();
        pipeline.add_many([
            &src,
            &tee,
            &preview_queue,
            &preview_convert,
            &gtksink,
            &webrtcsink_queue,
            &webrtcsink_convert,
            &webrtcsink,
        ])?;

        gst::Element::link_many([&src, &tee])?;
        gst::Element::link_many([&preview_queue, &preview_convert, &gtksink])?;
        gst::Element::link_many([&webrtcsink_queue, &webrtcsink_convert, &webrtcsink])?;

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

        let tee_webrtcsink_pad = tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let queue_webrtcsink_pad = webrtcsink_queue
            .static_pad("sink")
            .map_or_else(|| Err(glib::bool_error!("`static_pad()` failed")), Ok)?;
        tee_webrtcsink_pad
            .link(&queue_webrtcsink_pad)
            .map_err(|err| glib::bool_error!("{err}"))?;

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let preview_stack = gtk::Stack::new();
        let preview_disabled_label = gtk::Label::new(Some("Preview disabled"));
        let gst_widget = gst_gtk4::RenderWidget::new(&gtksink);

        preview_stack.add_child(&gst_widget);
        preview_stack.add_child(&preview_disabled_label);

        vbox.append(&preview_stack);

        let start_button = gtk::Button::builder().label("Start casting").build();
        let event_tx_clone = event_tx.clone();
        start_button.connect_clicked(move |_| {
            glib::spawn_future_local(glib::clone!(
                #[strong]
                event_tx_clone,
                async move {
                    event_tx_clone.send(Event::Start).await.unwrap();
                }
            ));
        });
        vbox.append(&start_button);

        let stop_button = gtk::Button::builder().label("Stop casting").build();
        let event_tx_clone = event_tx.clone();
        stop_button.connect_clicked(move |_| {
            glib::spawn_future_local(glib::clone!(
                #[strong]
                event_tx_clone,
                async move {
                    event_tx_clone.send(Event::Stop).await.unwrap();
                }
            ));
        });
        vbox.append(&stop_button);

        let enable_preview = gtk::CheckButton::builder()
            .active(true)
            .label("Enable preview")
            .build();

        let event_tx_clone = event_tx.clone();
        enable_preview.connect_toggled(move |btn| {
            let new = btn.property::<bool>("active");
            glib::spawn_future_local(glib::clone!(
                #[strong]
                event_tx_clone,
                async move {
                    match new {
                        true => event_tx_clone.send(Event::EnablePreview).await.unwrap(),
                        false => event_tx_clone.send(Event::DisablePreview).await.unwrap(),
                    }
                }
            ));
        });

        vbox.append(&enable_preview);

        let pipeline_weak = pipeline.downgrade();
        // Start pipeline in background to not freez UI
        let _ = std::thread::spawn(move || {
            let Some(pipeline) = pipeline_weak.upgrade() else {
                panic!("No pipeline");
            };
            pipeline.set_state(gst::State::Playing).unwrap();
        });

        Ok(Self {
            pipeline,
            vbox,
            preview_stack,
            gst_widget,
            preview_disabled_label,
        })
    }

    pub fn setup_bus_watch(
        &self,
        app_weak: glib::WeakRef<gtk::Application>,
    ) -> Result<BusWatchGuard, glib::BoolError> {
        let bus = self.pipeline.bus().unwrap();
        let bus_watch = bus.add_watch_local(move |_, msg| {
            use gst::MessageView;

            let Some(app) = app_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            match msg.view() {
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

    pub fn main_widget(&self) -> &gtk::Box {
        &self.vbox
    }
}
