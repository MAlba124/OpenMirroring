use std::sync::OnceLock;
use tokio::runtime::Runtime;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::tcp::{ReadHalf, WriteHalf},
};

pub mod net;

// #[cfg(feature = "sender")]
// pub mod sender;

pub const HEADER_BUFFER_SIZE: usize = 5;
pub const MAX_BODY_SIZE: u32 = 32000 - 1;

pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().unwrap())
}

pub fn default_log_level() -> log::LevelFilter {
    if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    }
}

use anyhow::{Context, bail};

use fcast_protocol::{
    Opcode, PlaybackErrorMessage, SeekMessage, SetSpeedMessage, SetVolumeMessage, VersionMessage,
    VolumeUpdateMessage,
    v2::{PlayMessage, PlaybackUpdateMessage},
};

#[derive(Debug, PartialEq)]
pub struct Header {
    pub size: u32,
    pub opcode: Opcode,
}

impl Header {
    pub fn new(opcode: Opcode, size: u32) -> Self {
        Self {
            size: size + 1,
            opcode,
        }
    }

    pub fn decode(buf: [u8; 5]) -> Self {
        Self {
            size: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) - 1,
            opcode: Opcode::try_from(buf[4]).unwrap(),
        }
    }

    pub fn encode(&self) -> [u8; 5] {
        let size_slice = u32::to_le_bytes(self.size);
        [
            size_slice[0],
            size_slice[1],
            size_slice[2],
            size_slice[3],
            self.opcode as u8,
        ]
    }
}

#[derive(Debug)]
pub enum Packet {
    None,
    Play(PlayMessage),
    Pause,
    Resume,
    Stop,
    Seek(SeekMessage),
    PlaybackUpdate(PlaybackUpdateMessage),
    VolumeUpdate(VolumeUpdateMessage),
    SetVolume(SetVolumeMessage),
    PlaybackError(PlaybackErrorMessage),
    SetSpeed(SetSpeedMessage),
    Version(VersionMessage),
    Ping,
    Pong,
}

impl From<&Packet> for Opcode {
    fn from(value: &Packet) -> Self {
        match value {
            Packet::None => Opcode::None,
            Packet::Play(_) => Opcode::Play,
            Packet::Pause => Opcode::Pause,
            Packet::Resume => Opcode::Resume,
            Packet::Stop => Opcode::Stop,
            Packet::Seek(_) => Opcode::Seek,
            Packet::PlaybackUpdate(_) => Opcode::PlaybackUpdate,
            Packet::VolumeUpdate(_) => Opcode::VolumeUpdate,
            Packet::SetVolume(_) => Opcode::SetVolume,
            Packet::PlaybackError(_) => Opcode::PlaybackError,
            Packet::SetSpeed(_) => Opcode::SetSpeed,
            Packet::Version(_) => Opcode::Version,
            Packet::Ping => Opcode::Ping,
            Packet::Pong => Opcode::Pong,
        }
    }
}

impl From<PlaybackErrorMessage> for Packet {
    fn from(value: PlaybackErrorMessage) -> Packet {
        Packet::PlaybackError(value)
    }
}

impl From<PlaybackUpdateMessage> for Packet {
    fn from(value: PlaybackUpdateMessage) -> Self {
        Self::PlaybackUpdate(value)
    }
}

impl From<VolumeUpdateMessage> for Packet {
    fn from(value: VolumeUpdateMessage) -> Self {
        Packet::VolumeUpdate(value)
    }
}

impl From<PlayMessage> for Packet {
    fn from(value: PlayMessage) -> Self {
        Packet::Play(value)
    }
}

impl Packet {
    pub fn decode(header: Header, body: &str) -> anyhow::Result<Self> {
        Ok(match header.opcode {
            Opcode::None => Self::None,
            Opcode::Play => Self::Play(serde_json::from_str(body).context("Play")?),
            Opcode::Pause => Self::Pause,
            Opcode::Resume => Self::Resume,
            Opcode::Stop => Self::Stop,
            Opcode::Seek => Self::Seek(serde_json::from_str(body)?),
            Opcode::PlaybackUpdate => {
                Self::PlaybackUpdate(serde_json::from_str(body).context("PlaybackUpdate")?)
            }
            Opcode::VolumeUpdate => {
                Self::VolumeUpdate(serde_json::from_str(body).context("VolumeUpdate")?)
            }
            Opcode::SetVolume => Self::SetVolume(serde_json::from_str(body).context("SetVolume")?),
            Opcode::PlaybackError => {
                Self::PlaybackError(serde_json::from_str(body).context("PlaybackError")?)
            }
            Opcode::SetSpeed => Self::SetSpeed(serde_json::from_str(body).context("SetSpeed")?),
            Opcode::Version => Self::Version(serde_json::from_str(body).context("Version")?),
            Opcode::Ping => Self::Ping,
            Opcode::Pong => Self::Pong,
            _ => bail!("Unsupported opcode: {:?}", header.opcode),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let body = match self {
            Packet::Play(play_message) => {
                serde_json::to_string(&play_message).unwrap().into_bytes()
            }
            Packet::Seek(seek_message) => {
                serde_json::to_string(&seek_message).unwrap().into_bytes()
            }
            Packet::PlaybackUpdate(playback_update_message) => {
                serde_json::to_string(&playback_update_message)
                    .unwrap()
                    .into_bytes()
            }
            Packet::VolumeUpdate(volume_update_message) => {
                serde_json::to_string(&volume_update_message)
                    .unwrap()
                    .into_bytes()
            }
            Packet::SetVolume(set_volume_message) => serde_json::to_string(&set_volume_message)
                .unwrap()
                .into_bytes(),
            Packet::PlaybackError(playback_error_message) => {
                serde_json::to_string(&playback_error_message)
                    .unwrap()
                    .into_bytes()
            }
            Packet::SetSpeed(set_speed_message) => serde_json::to_string(&set_speed_message)
                .unwrap()
                .into_bytes(),
            Packet::Version(version_message) => serde_json::to_string(&version_message)
                .unwrap()
                .into_bytes(),
            _ => Vec::new(),
        };

        assert!(body.len() < 32 * 1000);
        let header = Header::new(self.into(), body.len() as u32).encode();
        let mut pack = header.to_vec();
        pack.extend_from_slice(&body);
        pack
    }
}

/// Attempt to read and decode FCast packet from `stream`.
pub async fn read_packet(stream: &mut ReadHalf<'_>) -> anyhow::Result<Packet> {
    let mut header_buf: [u8; HEADER_BUFFER_SIZE] = [0; HEADER_BUFFER_SIZE];

    stream.read_exact(&mut header_buf).await?;

    let header = Header::decode(header_buf);

    let mut body_string = String::new();

    if header.size > 0 {
        let mut body_buf = vec![0; header.size as usize];
        stream.read_exact(&mut body_buf).await?;
        body_string = String::from_utf8(body_buf)?;
    }

    Packet::decode(header, &body_string)
}

pub async fn write_packet(stream: &mut WriteHalf<'_>, packet: Packet) -> anyhow::Result<()> {
    let bytes = packet.encode();
    stream.write_all(&bytes).await?;
    Ok(())
}
