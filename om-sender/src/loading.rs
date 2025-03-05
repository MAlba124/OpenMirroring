use gtk::prelude::*;
use gtk4 as gtk;

#[allow(dead_code)]
pub struct LoadingSourcesView {
    vbox: gtk::Box,
    spinner: gtk::Spinner,
    label: gtk::Label,
}

impl LoadingSourcesView {
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

        Self {
            vbox,
            spinner,
            label,
        }
    }

    pub fn main_widget(&self) -> &gtk::Box {
        &self.vbox
    }
}
