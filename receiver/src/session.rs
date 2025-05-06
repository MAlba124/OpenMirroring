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

use std::pin::pin;

use anyhow::Result;
use futures::stream::unfold;
use log::{debug, error, trace, warn};
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::broadcast::Receiver};
use tokio_stream::StreamExt;

use crate::Event;
use fcast_lib::packet::Packet;
use fcast_lib::{read_packet, write_packet};

pub type SessionId = u64;

pub struct Session {
    stream: TcpStream,
    event_tx: tokio::sync::mpsc::Sender<Event>,
    id: SessionId,
}

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

    pub async fn run(mut self, updates_rx: Receiver<Vec<u8>>) -> Result<()> {
        debug!("id={} Session was started", self.id);

        let (tcp_stream_rx, mut tcp_stream_tx) = self.stream.split();

        enum Message {
            Packet(Packet),
            Update(Vec<u8>),
        }

        let packets_stream = unfold(tcp_stream_rx, |mut tcp_stream| async move {
            match read_packet(&mut tcp_stream).await {
                Ok(p) => Some((Message::Packet(p), tcp_stream)),
                Err(err) => {
                    error!("Failed to receive packet: {err}");
                    None
                }
            }
        });

        let updates_stream = unfold(updates_rx, |mut updates_rx: Receiver<Vec<u8>>| async move {
            updates_rx
                .recv()
                .await
                .ok()
                .map(|update| (Message::Update(update), updates_rx))
        });

        let mut msg_stream = pin!(packets_stream.merge(updates_stream));
        while let Some(msg) = msg_stream.next().await {
            match msg {
                Message::Packet(packet) => {
                    trace!("id={} Got packet: {packet:?}", self.id);
                    match packet {
                        Packet::None => (),
                        Packet::Play(play_message) => {
                            self.event_tx.send(Event::Play(play_message)).await?
                        }
                        Packet::Pause => self.event_tx.send(Event::Pause).await?,
                        Packet::Resume => self.event_tx.send(Event::Resume).await?,
                        Packet::Stop => self.event_tx.send(Event::Stop).await?,
                        Packet::Seek(seek_message) => {
                            self.event_tx.send(Event::Seek(seek_message)).await?
                        }
                        Packet::SetVolume(set_volume_message) => {
                            self.event_tx
                                .send(Event::SetVolume(set_volume_message))
                                .await?;
                        }
                        Packet::SetSpeed(set_speed_message) => {
                            self.event_tx
                                .send(Event::SetSpeed(set_speed_message))
                                .await?;
                        }
                        Packet::Ping => write_packet(&mut tcp_stream_tx, Packet::Pong).await?,
                        Packet::Pong => trace!("id={} Got pong from sender", self.id),
                        _ => warn!(
                            "id={} Invalid packet from sender packet={packet:?}",
                            self.id
                        ),
                    }
                }
                Message::Update(update) => {
                    tcp_stream_tx.write_all(&update).await?;
                    trace!("id={} Sent update", self.id);
                }
            }
        }

        Ok(())
    }
}
