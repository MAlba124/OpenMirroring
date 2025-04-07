use gtk::prelude::*;
use gtk4 as gtk;

pub struct ConnectingToReceiver {
    vbox: gtk::Box,
}

impl ConnectingToReceiver {
    pub fn new() -> Self {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();

        let spinner = gtk::Spinner::builder().spinning(true).build();
        let label = gtk::Label::new(Some("Connecting"));

        vbox.append(&spinner);
        vbox.append(&label);

        Self { vbox }
    }
}

impl super::View for ConnectingToReceiver {
    fn main_widget(&self) -> &gtk4::Widget {
        self.vbox.upcast_ref()
    }
}
