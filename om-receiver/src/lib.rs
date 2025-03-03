use fcast_lib::models::{
    PlayMessage, PlaybackState, SeekMessage, SetSpeedMessage, SetVolumeMessage,
};
use session::SessionId;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tokio::net::TcpStream;

pub mod dispatcher;
pub mod session;

#[derive(Debug)]
pub enum GuiEvent {
    Play(PlayMessage),
    Eos,
    Pause,
    Resume,
    Stop,
    SetSpeed(f64),
    Seek(f64),
    SetVolume(f64),
}

#[derive(Debug)]
pub enum Event {
    CreateSessionRequest {
        net_stream_mutex: Arc<Mutex<Option<TcpStream>>>,
        id: SessionId,
    },
    Pause,
    Play(PlayMessage),
    Resume,
    Stop,
    SetSpeed(SetSpeedMessage),
    Seek(SeekMessage),
    SetVolume(SetVolumeMessage),
    PlaybackUpdate {
        time: f64,
        duration: f64,
        state: PlaybackState,
        speed: f64,
    },
}

pub struct AtomicF64 {
    inner: AtomicU64,
}

impl AtomicF64 {
    pub fn new(v: f64) -> Self {
        Self {
            inner: AtomicU64::new(v.to_bits()),
        }
    }

    pub fn load(&self, order: Ordering) -> f64 {
        f64::from_bits(self.inner.load(order))
    }

    pub fn store(&self, v: f64, order: Ordering) {
        self.inner.store(v.to_bits(), order);
    }
}
