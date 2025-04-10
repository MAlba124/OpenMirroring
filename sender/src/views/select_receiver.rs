use gtk::glib;
use gtk::prelude::*;
use gtk4 as gtk;

use crate::Event;

pub struct SelectReceiver {
    vbox: gtk::Box,
    drop_down: gtk::DropDown,
    receivers: Vec<String>,
}

impl SelectReceiver {
    pub fn new(event_tx: tokio::sync::mpsc::Sender<Event>) -> Self {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();

        let label = gtk::Label::new(Some("Cast to"));
        let drop_down = gtk::DropDown::builder().build();
        let button = gtk::Button::with_label("Ok");

        // TODO: input IPA and port for non broadcasted receivers

        button.connect_clicked(glib::clone!(
            #[weak]
            drop_down,
            move |_| {
                let Some(model) = drop_down.model() else {
                    log::warn!("Attempted to select receiver when none available");
                    return;
                };
                let Ok(receivers_list) = model.downcast::<gtk::StringList>() else {
                    log::error!("The drop_down model is not a `gtk::StringList`");
                    return;
                };

                let Some(receiver) = receivers_list.string(drop_down.selected()) else {
                    log::error!("Failed to get receiver string");
                    return;
                };

                event_tx
                    .blocking_send(Event::SelectReceiver(receiver.to_string()))
                    .unwrap();
            }
        ));

        vbox.append(&label);
        vbox.append(&drop_down);
        vbox.append(&button);

        Self {
            vbox,
            drop_down,
            receivers: Vec::new(),
        }
    }

    pub fn add_receiver(&mut self, receiver: String) {
        self.receivers.push(receiver);
        let receivers_list = gtk::StringList::new(
            &self
                .receivers
                .iter()
                .map(|r| r.as_str())
                .collect::<Vec<&str>>(),
        );
        self.drop_down.set_model(Some(&receivers_list));
    }
}

impl super::View for SelectReceiver {
    fn main_widget(&self) -> &gtk4::Widget {
        self.vbox.upcast_ref()
    }
}
