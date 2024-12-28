use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum Opcode {
    None,
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

#[derive(Debug)]
pub struct Header {
    pub size: u32,
    pub opcode: Opcode,
}

#[allow(dead_code)]
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
    pub generation: f64,
    pub time: f64,
    pub duration: f64,
    pub state: f64,
    pub speed: f64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct VolumeUpdateMessage {
    pub generation: f64,
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
    pub version: f64,
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
