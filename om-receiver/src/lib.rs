use fcast_lib::models::{PlayMessage, PlaybackState, SeekMessage, SetSpeedMessage, SetVolumeMessage};
use session::SessionId;
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;

pub mod dispatcher;
pub mod session;
// pub mod video;

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
    // Playback(PlaybackEvent),
    PlaybackUpdate {
        time: f64,
        duration: f64,
        state: PlaybackState,
        speed: f64,
    },
}
