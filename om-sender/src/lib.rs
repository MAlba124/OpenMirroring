pub mod views;
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
