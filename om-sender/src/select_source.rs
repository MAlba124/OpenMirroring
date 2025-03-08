use crate::Event;
use gtk::glib;
use gtk::prelude::*;
use gtk4 as gtk;

#[allow(dead_code)]
pub struct SelectSourceView {
    vbox: gtk::Box,
    hbox: gtk::Box,
    source_label: gtk::Label,
    pub drop_down: gtk::DropDown,
    button: gtk::Button,
}

impl SelectSourceView {
    pub fn new(event_tx: tokio::sync::mpsc::Sender<Event>) -> Self {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        let hbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        let source_label = gtk::Label::new(Some("Select source"));
        vbox.append(&source_label);
        let sources_drop_down = gtk::DropDown::builder().build();
        let button = gtk::Button::with_label("Ok");
        let sink_drop_down = gtk::DropDown::from_strings(&["WebRTC", "HLS"]);
        button.connect_clicked(glib::clone!(
            #[weak]
            sources_drop_down,
            #[weak]
            sink_drop_down,
            move |_| {
                let selected = sources_drop_down.selected() as usize;
                let sink_type = sink_drop_down.selected() as usize;
                let event_tx = event_tx.clone();
                om_common::runtime().block_on(async move {
                    event_tx
                        .send(Event::SelectSource(selected, sink_type))
                        .await
                        .unwrap();
                });
            }
        ));
        hbox.append(&sources_drop_down);
        hbox.append(&sink_drop_down);
        hbox.append(&button);
        vbox.append(&hbox);

        Self {
            vbox,
            hbox,
            source_label,
            drop_down: sources_drop_down,
            button,
        }
    }

    pub fn main_widget(&self) -> &gtk::Box {
        &self.vbox
    }
}
