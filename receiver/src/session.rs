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

use log::{debug, error, trace, warn};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::Event;
use fcast_lib::models::Header;
use fcast_lib::packet::Packet;

pub type SessionId = u64;

pub struct Session {
    stream: TcpStream,
    event_tx: tokio::sync::mpsc::Sender<Event>,
    id: SessionId,
}

const HEADER_BUFFER_SIZE: usize = 5;

impl Session {
    pub fn new(
        stream: TcpStream,
        event_tx: tokio::sync::mpsc::Sender<Event>,
        id: SessionId,
    ) -> Self {
        Self {
            stream,
            event_tx,
            id,
        }
    }

    async fn get_next_packet(&mut self) -> Result<Packet, tokio::io::Error> {
        let mut header_buf: [u8; HEADER_BUFFER_SIZE] = [0; HEADER_BUFFER_SIZE];

        self.stream.read_exact(&mut header_buf).await?;

        let header = Header::decode(header_buf);

        let mut body_string = String::new();

        if header.size > 0 {
            let mut body_buf = vec![0; header.size as usize];
            self.stream.read_exact(&mut body_buf).await?;
            body_string =
                String::from_utf8(body_buf).map_err(|e| tokio::io::Error::other(e.to_string()))?;
        }

        Packet::decode(header, &body_string).map_err(|e| tokio::io::Error::other(e.to_string()))
    }

    pub async fn run(mut self, mut updates_rx: tokio::sync::broadcast::Receiver<Vec<u8>>) {
        debug!("id={} Session was started", self.id);
        // TODO: stream
        loop {
            tokio::select! {
                maybe_packet = self.get_next_packet() => {
                    match maybe_packet {
                        Ok(packet) => {
                        trace!("id={} Got packet: {packet:?}", self.id);
                        match packet {
                            Packet::None => {}
                            Packet::Play(play_message) => self.event_tx.send(Event::Play(play_message)).await.unwrap(),
                            Packet::Pause => self.event_tx.send(Event::Pause).await.unwrap(),
                            Packet::Resume => self.event_tx.send(Event::Resume).await.unwrap(),
                            Packet::Stop => self.event_tx.send(Event::Stop).await.unwrap(),
                            Packet::Seek(seek_message) => self.event_tx.send(Event::Seek(seek_message)).await.unwrap(),
                            Packet::SetVolume(set_volume_message) => {
                                self.event_tx.send(Event::SetVolume(set_volume_message)).await.unwrap()
                            }
                            Packet::SetSpeed(set_speed_message) => {
                                self.event_tx.send(Event::SetSpeed(set_speed_message)).await.unwrap()
                            }
                            Packet::Ping => todo!(), // TODO
                            Packet::Pong => trace!("id={} Got pong from sender", self.id),
                            _ => warn!("id={} Invalid packet from sender packet={packet:?}", self.id),
                        }
                        }
                        Err(err) => {
                            error!("id={} Got error: `{err}`, treating it as disconnect", self.id);
                            return;
                        }
                    }
                }
                maybe_update = updates_rx.recv() => match maybe_update {
                    Ok(update) => {
                        self.stream.write_all(&update).await.unwrap();
                        trace!("id={} Sent update", self.id);
                    }
                    Err(err) => panic!("{err}"),
                }
            }
        }
    }
}
