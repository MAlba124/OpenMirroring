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

use std::future::Future;
use std::net::SocketAddr;
use std::thread::JoinHandle;

use anyhow::{Result, bail};
use fcast_lib::models::PlayMessage;
use fcast_lib::{packet::Packet, read_packet, write_packet};
use log::{debug, error, warn};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc::Receiver;
use tokio_stream::StreamExt;

pub enum Event {
    SessionTerminated,
    FcastPacket(Packet),
    ConnectedToReceiver,
    DisconnectedFromReceiver,
}

#[derive(Debug)]
pub enum SessionMessage {
    Play(PlayMessage),
    Quit,
    Stop,
    Connect(SocketAddr),
    Disconnect,
}

/// Relay messages to/from receiver.
///
/// Returns `true` if [Message::Quit] was received.
async fn handle_receiver<F, Fut>(
    tcp_stream: &mut TcpStream,
    instruction_rx: &mut Receiver<SessionMessage>,
    on_event: &mut F,
) -> Result<bool>
where
    F: FnMut(Event) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (tcp_stream_rx, mut tcp_stream_tx) = tcp_stream.split();

    on_event(Event::ConnectedToReceiver).await;

    let packets_stream = futures::stream::unfold(tcp_stream_rx, |mut tcp_stream| async move {
        match read_packet(&mut tcp_stream).await {
            Ok(p) => Some((p, tcp_stream)),
            Err(err) => {
                error!("Failed to receive packet: {err}");
                None
            }
        }
    });

    let instruction_stream = futures::stream::unfold(
        instruction_rx,
        |instruction_rx: &mut Receiver<SessionMessage>| async move {
            instruction_rx
                .recv()
                .await
                .map(|inst| (inst, instruction_rx))
        },
    );

    tokio::pin!(packets_stream);
    tokio::pin!(instruction_stream);

    loop {
        tokio::select! {
            r = packets_stream.next() => {
                let Some(packet) = r else {
                    break;
                };

                match packet {
                    Packet::Ping => write_packet(&mut tcp_stream_tx, Packet::Pong).await?,
                    _ => on_event(Event::FcastPacket(packet)).await,
                }
            }
            r = instruction_stream.next() => {
                let Some(inst) = r else {
                    break;
                };

                debug!("Got instruction: {inst:?}");
                match inst {
                    SessionMessage::Play(play_msg) => {
                        let packet = Packet::from(play_msg);
                        write_packet(&mut tcp_stream_tx, packet).await?;
                    }
                    SessionMessage::Stop => write_packet(&mut tcp_stream_tx, Packet::Stop).await?,
                    SessionMessage::Quit => return Ok(true),
                    SessionMessage::Disconnect => return Ok(false),
                    _ => warn!("Received invalid message ({inst:?}) for the current session state"),
                }
            }
        }
    }

    Ok(false)
}

/// Dispatch receiver connection requests.
pub async fn session<F, Fut>(mut msg_rx: Receiver<SessionMessage>, mut on_event: F)
where
    F: FnMut(Event) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    while let Some(msg) = msg_rx.recv().await {
        match msg {
            SessionMessage::Connect(addr) => {
                let mut tcp_stream = match TcpStream::connect(addr).await {
                    Ok(s) => s,
                    Err(err) => {
                        error!("Failed to connect to receiver ({addr:?}): {err}");
                        continue;
                    }
                };
                match handle_receiver(&mut tcp_stream, &mut msg_rx, &mut on_event).await {
                    Ok(f) => {
                        if let Err(err) = tcp_stream.shutdown().await {
                            error!("Failed to shutdown receiver TCP stream: {err}");
                        }

                        if f {
                            break;
                        }
                    }
                    Err(err) => {
                        error!("Error occured while connecting to receiver: {err}");
                    }
                }
                on_event(Event::DisconnectedFromReceiver).await;
            }
            SessionMessage::Quit => break,
            _ => warn!("Received invalid message ({msg:?}) for the current session state"),
        }
    }

    debug!("Session terminated");

    on_event(Event::SessionTerminated).await;
}

pub enum SessionEvent {
    Packet(fcast_lib::packet::Packet),
    Connected,
}

#[derive(Default)]
pub struct Session {
    connect_jh: Option<JoinHandle<std::io::Result<std::net::TcpStream>>>,
    stream: Option<std::net::TcpStream>,
}

impl Session {
    pub fn connect(&mut self, addr: SocketAddr) {
        self.connect_jh = Some(std::thread::spawn(move || {
            std::net::TcpStream::connect(addr)
        }));
    }

    pub fn disconnect(&mut self) -> Result<()> {
        if let Some(jh) = self.connect_jh.take() {
            if jh.is_finished() {
                let _ = jh.join();
            }
        }

        if let Some(stream) = self.stream.take() {
            stream.shutdown(std::net::Shutdown::Both)?;
        }

        Ok(())
    }

    pub fn poll_event(&mut self) -> Result<Option<SessionEvent>> {
        if let Some(jh) = self.connect_jh.as_mut() {
            if jh.is_finished() {
                let jh = self.connect_jh.take().unwrap();
                let stream = jh.join().unwrap()?;
                stream.set_nonblocking(true)?;
                self.stream = Some(stream);
                return Ok(Some(SessionEvent::Connected));
            }
        }

        if let Some(stream) = self.stream.as_mut() {
            use std::io::Read;
            let mut header_buf = [0u8; fcast_lib::HEADER_BUFFER_SIZE];
            if let Err(err) = stream.read_exact(&mut header_buf) {
                if err.kind() != std::io::ErrorKind::WouldBlock {
                    return Err(err.into());
                }
                return Ok(None);
            }

            let header = fcast_lib::models::Header::decode(header_buf);

            let mut body_string = String::new();

            if header.size > 0 {
                if header.size > fcast_lib::MAX_BODY_SIZE {
                    bail!(
                        "Body size ({}) exceeds MAX_BODY_SIZE ({})",
                        header.size,
                        fcast_lib::MAX_BODY_SIZE
                    );
                }
                let mut body_buf = vec![0; header.size as usize];
                stream.set_nonblocking(false)?;
                if let Err(err) = stream.read_exact(&mut body_buf) {
                    stream.set_nonblocking(true)?;
                    return Err(err.into());
                }
                stream.set_nonblocking(true)?;
                body_string = String::from_utf8(body_buf)?;
            }

            let packet = fcast_lib::packet::Packet::decode(header, &body_string)?;
            return Ok(Some(SessionEvent::Packet(packet)));
        }

        Ok(None)
    }

    pub fn send_packet(&mut self, packet: fcast_lib::packet::Packet) -> Result<()> {
        if let Some(stream) = self.stream.as_mut() {
            use std::io::Write;
            stream.write_all(&packet.encode())?;
        }

        Ok(())
    }
}
