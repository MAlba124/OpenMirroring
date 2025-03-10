use gtk::prelude::*;
use gtk4 as gtk;

#[allow(dead_code)]
pub struct LoadingSources {
    vbox: gtk::Box,
    spinner: gtk::Spinner,
    label: gtk::Label,
}

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

        spinner.start();

        Self {
            vbox,
            spinner,
            label,
        }
    }
}

impl super::View for LoadingSources {
    fn main_widget(&self) -> &gtk4::Widget {
        self.vbox.upcast_ref()
    }
}
