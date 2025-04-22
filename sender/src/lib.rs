use std::net::SocketAddr;

pub mod discovery;
pub mod pipeline;
pub mod session;

#[derive(Debug)]
pub enum Message {
    Play { mime: String, uri: String },
    Quit,
    Stop,
    Connect(SocketAddr),
    Disconnect,
}

pub type ProducerId = String;

#[derive(Debug)]
pub struct Receiver {
    pub name: String,
    pub addresses: Vec<SocketAddr>,
}

#[derive(Debug)]
pub enum Event {
    Quit,
    Start,
    Stop,
    Sources(Vec<String>),
    SelectSource(usize),
    Packet(fcast_lib::packet::Packet),
    ReceiverAvailable(Receiver),
    SelectReceiver(String),
    ConnectedToReceiver,
    DisconnectReceiver,
    ChangeSource,
}
