use crate::Event;
use gtk::glib;
use gtk::prelude::*;
use gtk4 as gtk;

pub struct SelectSource {
    hbox: gtk::Box,
    pub drop_down: gtk::DropDown,
}

impl SelectSource {
    pub fn new(event_tx: tokio::sync::mpsc::Sender<Event>) -> Self {
        let source_vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_end(5)
            .build();
        let hbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        let source_label = gtk::Label::new(Some("Select source"));
        let sources_drop_down = gtk::DropDown::builder().build();
        let sink_drop_down = gtk::DropDown::from_strings(&["WebRTC", "HLS"]);
        let sink_vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        let sink_label = gtk::Label::new(Some("Select protocol"));
        let button = gtk::Button::builder().label("Ok").margin_start(5).build();
        button.connect_clicked(glib::clone!(
            #[weak]
            sources_drop_down,
            #[weak]
            sink_drop_down,
            move |_| {
                let source = sources_drop_down.selected() as usize;
                let sink_type = sink_drop_down.selected() as usize;
                let event_tx = event_tx.clone();
                event_tx
                    .blocking_send(Event::SelectSource(source, sink_type))
                    .unwrap();
            }
        ));

        source_vbox.append(&source_label);
        source_vbox.append(&sources_drop_down);

        sink_vbox.append(&sink_label);
        sink_vbox.append(&sink_drop_down);

        hbox.append(&source_vbox);
        hbox.append(&sink_vbox);
        hbox.append(&button);

        Self {
            hbox,
            drop_down: sources_drop_down,
        }
    }
}

impl super::View for SelectSource {
    fn main_widget(&self) -> &gtk4::Widget {
        self.hbox.upcast_ref()
    }
}
