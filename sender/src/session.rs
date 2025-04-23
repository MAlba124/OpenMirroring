use anyhow::Result;
use std::net::SocketAddr;

use fcast_lib::{models, models::Header, packet::Packet};
use log::{debug, error, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{Event, Message};

const HEADER_BUFFER_SIZE: usize = 5;

/// Attempt to read and decode FCast packet from `stream`.
async fn read_packet_from_stream(stream: &mut TcpStream) -> Result<Packet> {
    let mut header_buf: [u8; HEADER_BUFFER_SIZE] = [0; HEADER_BUFFER_SIZE];

    stream.read_exact(&mut header_buf).await?;

    let header = Header::decode(header_buf);

    let mut body_string = String::new();

    if header.size > 0 {
        let mut body_buf = vec![0; header.size as usize];
        stream.read_exact(&mut body_buf).await?;
        body_string = String::from_utf8(body_buf)?;
    }

    Ok(Packet::decode(header, &body_string)?)
}

async fn send_packet(stream: &mut TcpStream, packet: Packet) -> Result<()> {
    let bytes = packet.encode();
    stream.write_all(&bytes).await?;
    Ok(())
}

/// Connect and relay messages to/from receiver.
///
/// Returns `true` if [Message::Quit] was received.
async fn connect_to_receiver(
    addr: SocketAddr,
    msg_rx: &mut Receiver<Message>,
    event_tx: &Sender<Event>,
) -> Result<bool> {
    let mut stream = TcpStream::connect(addr).await?;

    event_tx.send(Event::ConnectedToReceiver).await?;

    loop {
        tokio::select! {
            // TODO: fix as this is an ugly hack and likely not safe
            packet = read_packet_from_stream(&mut stream) => {
                let packet = packet?;
                match packet {
                    Packet::Ping => {
                        send_packet(&mut stream, Packet::Pong).await?;
                    }
                    _ => {
                        event_tx.send(Event::Packet(packet)).await?;
                    }
                }
            }
            msg = msg_rx.recv() => {
                let msg = msg.unwrap();
                debug!("Got message: {msg:?}");
                match msg {
                    Message::Play { mime, uri } => {
                        let packet = Packet::from(models::PlayMessage {
                            container: mime,
                            url: Some(uri),
                            content: None,
                            time: None,
                            speed: None,
                            headers: None,
                        });
                        send_packet(&mut stream, packet).await?;
                    }
                    Message::Stop => send_packet(&mut stream, Packet::Stop).await?,
                    Message::Quit => return Ok(true),
                    Message::Disconnect => return Ok(false),
                    _ => warn!("Received invalid message ({msg:?}) for the current session state"),
                }
            }
        }
    }
}

/// Dispatch receiver connection requests.
pub async fn session(mut msg_rx: Receiver<Message>, event_tx: Sender<Event>) {
    while let Some(msg) = msg_rx.recv().await {
        match msg {
            Message::Connect(addr) => {
                match connect_to_receiver(addr, &mut msg_rx, &event_tx).await {
                    Ok(f) => {
                        if f {
                            break;
                        }
                    }
                    Err(err) => {
                        error!("Error occured while connecting to receiver: {err}");
                    }
                }
            }
            Message::Quit => break,
            _ => warn!("Received invalid message ({msg:?}) for the current session state"),
        }
    }

    debug!("Session terminated");

    event_tx.send(Event::Quit).await.unwrap();
}
