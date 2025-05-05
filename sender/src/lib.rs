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

use std::net::SocketAddr;

pub mod discovery;

pub type ProducerId = String;

#[derive(Debug)]
pub struct Receiver {
    pub name: String,
    pub addresses: Vec<SocketAddr>,
}

#[derive(Debug)]
pub enum Event {
    StartCast,
    StopCast,
    Sources(Vec<String>),
    SelectSource(usize),
    Packet(fcast_lib::packet::Packet),
    ReceiverAvailable(Receiver),
    SelectReceiver(String),
    ConnectedToReceiver,
    DisconnectReceiver,
    ChangeSource,
    PipelineFinished,
    PipelineIsPlaying,
    SessionTerminated,
}
