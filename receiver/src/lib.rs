// Copyright (C) 2025 Marcus L. Hanestad <marlhan@proton.me>
//
// This file is part of OpenMirroring.
//
// OpenMirroring is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// OpenMirroring is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with OpenMirroring.  If not, see <https://www.gnu.org/licenses/>.

use fcast_lib::models::{
    PlayMessage, PlaybackState, SeekMessage, SetSpeedMessage, SetVolumeMessage,
};
use session::SessionId;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::net::TcpStream;

pub mod dispatcher;
pub mod session;
pub mod video;

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
        stream: TcpStream,
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
