use crate::models::{
    Header, Opcode, PlayMessage, PlaybackErrorMessage, PlaybackUpdateMessage, SeekMessage,
    SetSpeedMessage, SetVolumeMessage, VersionMessage, VolumeUpdateMessage,
};

#[derive(Debug, Clone)]
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

impl Packet {
    pub const fn pong() -> Self {
        Self::Pong
    }

    pub fn decode(header: Header, body: &str) -> Result<Self, serde_json::Error> {
        Ok(match header.opcode {
            Opcode::None => Self::None,
            Opcode::Play => Self::Play(serde_json::from_str(body)?),
            Opcode::Pause => Self::Pause,
            Opcode::Resume => Self::Resume,
            Opcode::Stop => Self::Stop,
            Opcode::Seek => Self::Seek(serde_json::from_str(body)?),
            Opcode::PlaybackUpdate => Self::PlaybackUpdate(serde_json::from_str(body)?),
            Opcode::VolumeUpdate => Self::VolumeUpdate(serde_json::from_str(body)?),
            Opcode::SetVolume => Self::SetVolume(serde_json::from_str(body)?),
            Opcode::PlaybackError => Self::PlaybackError(serde_json::from_str(body)?),
            Opcode::SetSpeed => Self::SetSpeed(serde_json::from_str(body)?),
            Opcode::Version => Self::Version(serde_json::from_str(body)?),
            Opcode::Ping => Self::Ping,
            Opcode::Pong => Self::Pong,
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
