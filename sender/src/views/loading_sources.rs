use gtk::prelude::*;
use gtk4 as gtk;

pub struct LoadingSources(gtk::Box);

impl LoadingSources {
    pub fn new() -> Self {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();

        let spinner = gtk::Spinner::builder().spinning(true).build();
        let label = gtk::Label::new(Some("Loading sources"));

        vbox.append(&spinner);
        vbox.append(&label);

        Self(vbox)
    }
}

impl super::View for LoadingSources {
    fn main_widget(&self) -> &gtk4::Widget {
        self.0.upcast_ref()
    }
}
