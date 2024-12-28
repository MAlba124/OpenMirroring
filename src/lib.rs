use std::sync::{Arc, Mutex};

use protocol_types::{PlayMessage, SeekMessage, SetSpeedMessage, SetVolumeMessage};
use tokio::net::TcpStream;

pub mod packet;
pub mod protocol_types;
pub mod session;

#[derive(Debug, Clone)]
pub enum Message {
    EndOfStream,
    Pause,
    Play(PlayMessage),
    Resume,
    SetSpeed(SetSpeedMessage),
    Seek(SeekMessage),
    SetVolume(SetVolumeMessage),
    Stop,
    CreateSession(Arc<Mutex<Option<TcpStream>>>),
    PlaybackError(String),
    // PlaybackNewFrame,
    Tick,
    Nothing,
    SessionCreated,
    SessionDestroyed,
}
