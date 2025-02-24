mod imp;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::glib;
use glib::translate::*;

glib::wrapper! {
    pub struct Video(ObjectSubclass<imp::Video>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Video {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Video {
    pub fn new() -> Self {
        glib::Object::builder()
            // .property("event_tx", 0)
            // .property("gui_event_tx", 0)
            .build()
    }

    pub fn shutdown(&mut self) {
        self.imp().shutdown();
    }
}
