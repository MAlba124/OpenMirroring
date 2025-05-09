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

use fcast_lib::models::{PlayMessage, SeekMessage, SetSpeedMessage, SetVolumeMessage};
use session::SessionId;
use tokio::net::TcpStream;

pub mod dispatcher;
pub mod pipeline;
pub mod session;

#[derive(Debug)]
pub enum Event {
    CreateSessionRequest { stream: TcpStream, id: SessionId },
    Pause,
    Play(PlayMessage),
    Resume,
    Stop,
    SetSpeed(SetSpeedMessage),
    Seek(SeekMessage),
    SetVolume(SetVolumeMessage),
    SendPlaybackUpdate,
    Quit,
    PipelineEos,
    PipelineError,
    SessionFinished,
}
