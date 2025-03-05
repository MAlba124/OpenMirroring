use std::sync::{Arc, Mutex};

use gst::{glib, prelude::*};
use gtk::prelude::*;
use gtk4 as gtk;
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
        let tee = gst::ElementFactory::make("tee").build().unwrap();
        let src = gst::ElementFactory::make("scapsrc")
            .property("perform-internal-preroll", true)
            .build()
            .unwrap();

        let preview_queue = gst::ElementFactory::make("queue")
            .name("preview_queue")
            .build()
            .unwrap();
        let preview_convert = gst::ElementFactory::make("videoconvert")
            .name("preview_convert")
            .build()
            .unwrap();
        let gtksink = gst::ElementFactory::make("gtk4paintablesink")
            .name("gtksink")
            .build()
            .unwrap();

        let webrtcsink_queue = gst::ElementFactory::make("queue")
            .name("webrtcsink_queue")
            .build()
            .unwrap();
        let webrtcsink_convert = gst::ElementFactory::make("videoconvert")
            .name("webrtcsink_convert")
            .build()
            .unwrap();
        let webrtcsink = gst::ElementFactory::make("webrtcsink")
            .property("signalling-server-host", "127.0.0.1")
            .property("signalling-server-port", 8443u32)
            .build()
            .unwrap();

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
        pipeline
            .add_many([
                &src,
                &tee,
                &preview_queue,
                &preview_convert,
                &gtksink,
                &webrtcsink_queue,
                &webrtcsink_convert,
                &webrtcsink,
            ])
            .unwrap();

        gst::Element::link_many([&src, &tee]).unwrap();
        gst::Element::link_many([&preview_queue, &preview_convert, &gtksink]).unwrap();
        gst::Element::link_many([&webrtcsink_queue, &webrtcsink_convert, &webrtcsink]).unwrap();

        let tee_preview_pad = tee.request_pad_simple("src_%u").unwrap();
        let queue_preview_pad = preview_queue.static_pad("sink").unwrap();
        tee_preview_pad.link(&queue_preview_pad).unwrap();

        let tee_webrtcsink_pad = tee.request_pad_simple("src_%u").unwrap();
        let queue_webrtcsink_pad = webrtcsink_queue.static_pad("sink").unwrap();
        tee_webrtcsink_pad.link(&queue_webrtcsink_pad).unwrap();

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

        Ok(Self {
            pipeline,
            vbox,
            preview_stack,
            gst_widget,
            preview_disabled_label,
        })
    }

    pub fn main_widget(&self) -> &gtk::Box {
        &self.vbox
    }
}
