use std::sync::{Arc, Mutex};

use protocol_types::{PlayMessage, SeekMessage, SetSpeedMessage, SetVolumeMessage};
use session::SessionId;
use tokio::net::TcpStream;

pub mod packet;
pub mod protocol_types;
pub mod session;

#[derive(Debug, Clone)]
pub struct CreateSessionRequest {
    pub net_stream_mutex: Arc<Mutex<Option<TcpStream>>>,
    pub id: SessionId,
}

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
    CreateSession(CreateSessionRequest),
    PlaybackError(String),
    Tick,
    Nothing,
    SessionDestroyed(SessionId),
}
