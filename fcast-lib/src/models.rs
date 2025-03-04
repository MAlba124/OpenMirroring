use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};

#[derive(Debug, PartialEq, Eq)]
pub enum Opcode {
    None = 0,
    Play,
    Pause,
    Resume,
    Stop,
    Seek,
    PlaybackUpdate,
    VolumeUpdate,
    SetVolume,
    PlaybackError,
    SetSpeed,
    Version,
    Ping,
    Pong,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Header {
    pub size: u32,
    pub opcode: Opcode,
}

#[derive(Serialize_repr, Deserialize_repr, PartialEq, Debug, Clone)]
#[repr(u8)]
pub enum PlaybackState {
    Idle = 0,
    Playing = 1,
    Paused = 2,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PlayMessage {
    pub container: String,
    pub url: Option<String>,
    pub content: Option<String>,
    pub time: Option<f64>,
    pub speed: Option<f64>,
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SeekMessage {
    pub time: f64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PlaybackUpdateMessage {
    #[serde(rename = "generationTime")]
    pub generation: u64,
    pub time: f64,
    pub duration: f64,
    pub state: PlaybackState,
    pub speed: f64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct VolumeUpdateMessage {
    #[serde(rename = "generationTime")]
    pub generation: u64,
    pub volume: f64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SetVolumeMessage {
    pub volume: f64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SetSpeedMessage {
    pub speed: f64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PlaybackErrorMessage {
    pub message: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct VersionMessage {
    pub version: u64,
}

impl From<bool> for PlaybackState {
    fn from(value: bool) -> Self {
        match value {
            true => Self::Paused,
            false => Self::Playing,
        }
    }
}

impl From<String> for PlaybackErrorMessage {
    fn from(value: String) -> Self {
        Self { message: value }
    }
}

impl From<u8> for Opcode {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Play,
            2 => Self::Pause,
            3 => Self::Resume,
            4 => Self::Stop,
            5 => Self::Seek,
            6 => Self::PlaybackUpdate,
            7 => Self::VolumeUpdate,
            8 => Self::SetVolume,
            9 => Self::PlaybackError,
            10 => Self::SetSpeed,
            11 => Self::Version,
            12 => Self::Ping,
            13 => Self::Pong,
            _ => Self::None,
        }
    }
}

impl From<&Opcode> for u8 {
    fn from(value: &Opcode) -> Self {
        match value {
            Opcode::Play => 1,
            Opcode::Pause => 2,
            Opcode::Resume => 3,
            Opcode::Stop => 4,
            Opcode::Seek => 5,
            Opcode::PlaybackUpdate => 6,
            Opcode::VolumeUpdate => 7,
            Opcode::SetVolume => 8,
            Opcode::PlaybackError => 9,
            Opcode::SetSpeed => 10,
            Opcode::Version => 11,
            Opcode::Ping => 12,
            Opcode::Pong => 13,
            _ => 0,
        }
    }
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
            opcode: Opcode::from(buf[4]),
        }
    }

    pub fn encode(&self) -> [u8; 5] {
        let size_slice = u32::to_le_bytes(self.size);
        [
            size_slice[0],
            size_slice[1],
            size_slice[2],
            size_slice[3],
            (&self.opcode).into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use crate::models::{Header, Opcode};

    #[test]
    fn test_header_encode() {
        assert_eq!(
            Header::new(Opcode::Ping, 0).encode(),
            [1, 0, 0, 0, 12],
        );
        assert_eq!(
            Header::new(Opcode::Play, 200).encode(),
            [201, 0, 0, 0, 1],
        );
        assert_eq!(
            Header::new(Opcode::None, 0).encode(),
            [1, 0, 0, 0, 0],
        );
    }
}
