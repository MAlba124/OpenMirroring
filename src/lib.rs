// use std::sync::{Arc, Mutex};

// use protocol_types::{PlayMessage, SeekMessage, SetSpeedMessage, SetVolumeMessage};
// use session::SessionId;
// use tokio::net::TcpStream;

// pub mod packet;
// pub mod protocol_types;
// pub mod session;

// #[derive(Debug, Clone)]
// pub struct CreateSessionRequest {
//     pub net_stream_mutex: Arc<Mutex<Option<TcpStream>>>,
//     pub id: SessionId,
// }

// #[derive(Debug, Clone)]
// pub enum Message {
//     EndOfStream,
//     Pause,
//     Play(PlayMessage),
//     Resume,
//     SetSpeed(SetSpeedMessage),
//     Seek(SeekMessage),
//     SetVolume(SetVolumeMessage),
//     Stop,
//     CreateSession(CreateSessionRequest),
//     PlaybackError(String),
//     Tick,
//     Nothing,
//     SessionDestroyed(SessionId),
// }

use models::{PlayMessage, PlaybackState, SeekMessage, SetSpeedMessage, SetVolumeMessage};
use session::SessionId;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::{net::TcpStream, runtime::Runtime};

pub mod dispatcher;
pub mod models;
pub mod packet;
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

pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().unwrap())
}
