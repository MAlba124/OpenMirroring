use gtk4::{self as gtk, prelude::BoxExt};

pub struct LoadingHlsView {
    vbox: gtk::Box,
}

impl LoadingHlsView {
    pub fn new() -> Self {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        let spinner = gtk::Spinner::builder()
            .spinning(true)
            .build();
        let label = gtk::Label::new(Some("Setting up stream"));

        vbox.append(&spinner);
        vbox.append(&label);

        Self {
            vbox,
        }
    }

    pub fn main_widget(&self) -> &gtk::Box {
        &self.vbox
    }
}
