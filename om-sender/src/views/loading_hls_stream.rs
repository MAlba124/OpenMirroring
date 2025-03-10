use gtk4::{self as gtk, prelude::BoxExt, prelude::Cast};

pub struct LoadingHlsStream {
    vbox: gtk::Box,
}

impl LoadingHlsStream {
    pub fn new() -> Self {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        let spinner = gtk::Spinner::builder().spinning(true).build();
        let label = gtk::Label::new(Some("Setting up stream"));

        vbox.append(&spinner);
        vbox.append(&label);

        Self { vbox }
    }
}

impl super::View for LoadingHlsStream {
    fn main_widget(&self) -> &gtk4::Widget {
        self.vbox.upcast_ref()
    }
}
