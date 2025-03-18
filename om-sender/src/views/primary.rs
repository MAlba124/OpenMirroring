use gst::{glib, prelude::*};
use gtk::prelude::*;
use gtk4 as gtk;
use tokio::sync::mpsc::Sender;

use crate::Event;

pub struct Primary {
    vbox: gtk::Box,
    pub preview_stack: gtk::Stack,
    pub gst_widget: gst_gtk4::RenderWidget,
    pub preview_disabled_label: gtk::Label,
}

impl Primary {
    pub fn new(
        event_tx: Sender<Event>,
        gst_widget: gst_gtk4::RenderWidget,
    ) -> Result<Self, glib::BoolError> {
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let preview_stack = gtk::Stack::new();
        let preview_disabled_label = gtk::Label::new(Some("Preview disabled"));

        preview_stack.add_child(&gst_widget);
        preview_stack.add_child(&preview_disabled_label);

        vbox.append(&preview_stack);

        let cast_button = gtk::Button::builder().label("Start casting").build();

        let event_tx_clone = event_tx.clone();
        cast_button.connect_clicked(move |btn| {
            if btn.label().unwrap() == "Start casting" {
                event_tx_clone.blocking_send(Event::Start).unwrap();
                btn.set_label("Stop casting");
            } else {
                event_tx_clone.blocking_send(Event::Stop).unwrap();
                btn.set_label("Start casting");
            }
        });
        vbox.append(&cast_button);

        let enable_preview = gtk::CheckButton::builder()
            .active(true)
            .label("Enable preview")
            .build();

        let event_tx_clone = event_tx.clone();
        enable_preview.connect_toggled(move |btn| {
            match btn.property::<bool>("active") {
                true => event_tx_clone.blocking_send(Event::EnablePreview).unwrap(),
                false => event_tx_clone.blocking_send(Event::DisablePreview).unwrap(),
            }
        });

        vbox.append(&enable_preview);

        Ok(Self {
            vbox,
            preview_stack,
            gst_widget,
            preview_disabled_label,
        })
    }
}

impl super::View for Primary {
    fn main_widget(&self) -> &gtk4::Widget {
        self.vbox.upcast_ref()
    }
}
