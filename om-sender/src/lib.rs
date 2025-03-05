pub mod loading;
pub mod primary;
pub mod select_source;
pub mod session;
pub mod signaller;

#[derive(Debug)]
pub enum Message {
    Play(String),
    Quit,
    Stop,
}

pub type ProducerId = String;

#[derive(Debug)]
pub enum Event {
    ProducerConnected(ProducerId),
    Start,
    Stop,
    EnablePreview,
    DisablePreview,
    Sources(Vec<String>),
    SelectSource(usize),
    Packet(fcast_lib::packet::Packet),
}
