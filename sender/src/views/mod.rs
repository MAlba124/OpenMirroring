use gtk::prelude::Cast;
use gtk4 as gtk;
use tokio::sync::mpsc::Sender;

use crate::Event;

pub mod connecting_to_receiver;
pub mod loading_hls_stream;
pub mod loading_sources;
pub mod primary;
pub mod select_receiver;
pub mod select_source;

pub trait View {
    fn main_widget(&self) -> &gtk::Widget;
}

pub enum StateChange {
    LoadingSourcesToSelectSources(Vec<String>),
    SelectSourceToSelectReceiver,
    SelectReceiverToConnectingToReceiver,
    ConnectingToReceiverToPrimary,
    SelectReceiverToPrimary,
}

pub struct Main {
    pub stack: gtk::Stack,
    pub select_source: select_source::SelectSource,
    pub select_receiver: select_receiver::SelectReceiver,
    pub loading_hls_stream: loading_hls_stream::LoadingHlsStream,
    pub primary: primary::Primary,
    pub connecting_to_receiver: connecting_to_receiver::ConnectingToReceiver,
}

impl Main {
    pub fn new(event_tx: Sender<Event>, gst_widget: gst_gtk4::RenderWidget) -> Self {
        let stack = gtk::Stack::new();
        let loading_sources = loading_sources::LoadingSources::new();
        let select_source = select_source::SelectSource::new(event_tx.clone());
        let select_receiver = select_receiver::SelectReceiver::new(event_tx.clone());
        let loading_hls_stream = loading_hls_stream::LoadingHlsStream::new();
        let primary = primary::Primary::new(event_tx, gst_widget).unwrap();
        let connecting_to_receiver = connecting_to_receiver::ConnectingToReceiver::new();

        stack.add_child(loading_sources.main_widget());
        stack.add_child(select_source.main_widget());
        stack.add_child(select_receiver.main_widget());
        stack.add_child(loading_hls_stream.main_widget());
        stack.add_child(primary.main_widget());
        stack.add_child(connecting_to_receiver.main_widget());

        Self {
            stack,
            select_source,
            select_receiver,
            loading_hls_stream,
            primary,
            connecting_to_receiver,
        }
    }

    pub fn change_state(&self, state_change: StateChange) {
        match state_change {
            StateChange::LoadingSourcesToSelectSources(sources) => {
                self.select_source.add_sources(sources);
                self.stack
                    .set_visible_child(self.select_source.main_widget());
            }
            StateChange::SelectSourceToSelectReceiver => {
                self.stack
                    .set_visible_child(self.select_receiver.main_widget());
            }
            StateChange::SelectReceiverToConnectingToReceiver => {
                self.stack
                    .set_visible_child(self.connecting_to_receiver.main_widget());
            }
            StateChange::SelectReceiverToPrimary => {
                self.stack.set_visible_child(self.primary.main_widget())
            }
            StateChange::ConnectingToReceiverToPrimary => {
                self.stack.set_visible_child(self.primary.main_widget())
            }
        }
    }
}

impl View for Main {
    fn main_widget(&self) -> &gtk4::Widget {
        self.stack.upcast_ref()
    }
}
