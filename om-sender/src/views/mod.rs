use gtk::prelude::Cast;
use gtk4 as gtk;
use tokio::sync::mpsc::Sender;

use crate::Event;

pub mod loading_hls_stream;
pub mod loading_sources;
pub mod primary;
pub mod select_source;

pub trait View {
    fn main_widget(&self) -> &gtk::Widget;
}

pub enum StateChange {
    LoadingSourcesToSelectSources(Vec<String>),
    SelectSourceToPrimary,
    SelectSourceToLoadingHlsStream,
    LoadingHlsStreamToPrimary,
}

pub struct Main {
    pub stack: gtk::Stack,
    #[allow(dead_code)]
    loading_sources: loading_sources::LoadingSources,
    pub select_source: select_source::SelectSource,
    pub loading_hls_stream: loading_hls_stream::LoadingHlsStream,
    pub primary: primary::Primary,
}

impl Main {
    // pub fn new(event_tx: Sender<Event>, selected_rx: Receiver<usize>) -> Self {
    pub fn new(event_tx: Sender<Event>, gst_widget: gst_gtk4::RenderWidget) -> Self {
        let stack = gtk::Stack::new();
        let loading_sources = loading_sources::LoadingSources::new();
        let select_source = select_source::SelectSource::new(event_tx.clone());
        let loading_hls_stream = loading_hls_stream::LoadingHlsStream::new();
        // let primary = primary::Primary::new(event_tx, selected_rx).unwrap();
        let primary = primary::Primary::new(event_tx, gst_widget).unwrap();

        stack.add_child(loading_sources.main_widget());
        stack.add_child(select_source.main_widget());
        stack.add_child(loading_hls_stream.main_widget());
        stack.add_child(primary.main_widget());

        Self {
            stack,
            loading_sources,
            select_source,
            loading_hls_stream,
            primary,
        }
    }

    pub fn change_state(&self, state_change: StateChange) {
        match state_change {
            StateChange::LoadingSourcesToSelectSources(sources) => {
                let l = gtk::StringList::new(
                    &sources.iter().map(|s| s.as_str()).collect::<Vec<&str>>(),
                );
                self.select_source.drop_down.set_model(Some(&l));
                self.stack
                    .set_visible_child(self.select_source.main_widget());
            }
            StateChange::SelectSourceToPrimary => {
                self.stack.set_visible_child(self.primary.main_widget())
            }
            StateChange::SelectSourceToLoadingHlsStream => self
                .stack
                .set_visible_child(self.loading_hls_stream.main_widget()),
            StateChange::LoadingHlsStreamToPrimary => {
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
