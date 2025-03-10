// TODO: make `views` module and put the views as sub
pub mod views;
// pub mod loading;
// pub mod loading_hls;
// pub mod primary;
// pub mod select_source;
pub mod session;
pub mod sink;

#[derive(Debug)]
pub enum Message {
    Play { mime: String, uri: String },
    Quit,
    Stop,
}

pub type ProducerId = String;

#[derive(Debug)]
pub enum Event {
    Quit,
    ProducerConnected(ProducerId),
    Start,
    Stop,
    EnablePreview,
    DisablePreview,
    Sources(Vec<String>),
    SelectSource(usize, usize),
    Packet(fcast_lib::packet::Packet),
    HlsServerAddr { port: u16 },
    HlsStreamReady,
}
