use std::net::SocketAddr;

use fcast_lib::{models, models::Header, packet::Packet};
use log::{debug, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{Event, Message};

const HEADER_BUFFER_SIZE: usize = 5;

/// Attempt to read and decode FCast packet from `stream`.
async fn read_packet_from_stream(stream: &mut TcpStream) -> Result<Packet, String> {
    let mut header_buf: [u8; HEADER_BUFFER_SIZE] = [0; HEADER_BUFFER_SIZE];

    stream
        .read_exact(&mut header_buf)
        .await
        .map_err(|err| err.to_string())?;

    let header = Header::decode(header_buf);

    let mut body_string = String::new();

    if header.size > 0 {
        let mut body_buf = vec![0; header.size as usize];
        stream
            .read_exact(&mut body_buf)
            .await
            .map_err(|err| err.to_string())?;
        body_string = String::from_utf8(body_buf).map_err(|err| err.to_string())?;
    }

    Ok(Packet::decode(header, &body_string).map_err(|err| err.to_string())?)
}

async fn send_packet(stream: &mut TcpStream, packet: Packet) -> Result<(), String> {
    let bytes = packet.encode();
    stream.write_all(&bytes).await.unwrap();
    Ok(())
}

/// Connect and relay messages to/from receiver.
///
/// Returns `true` if [Message::Quit] was received.
async fn connect_to_receiver(
    addr: SocketAddr,
    msg_rx: &mut Receiver<Message>,
    event_tx: &Sender<Event>,
) -> bool {
    let mut stream = TcpStream::connect(addr).await.unwrap();

    event_tx.send(Event::ConnectedToReceiver).await.unwrap();

    loop {
        tokio::select! {
            // TODO: fix as this is an ugly hack and likely not safe
            packet = read_packet_from_stream(&mut stream) => {
                let packet = packet.unwrap();
                match packet {
                    Packet::Ping => {
                        send_packet(&mut stream, Packet::Pong).await.unwrap();
                    }
                    _ => {
                        event_tx.send(Event::Packet(packet)).await.unwrap();
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
                        send_packet(&mut stream, packet).await.unwrap();
                    }
                    Message::Stop => send_packet(&mut stream, Packet::Stop).await.unwrap(),
                    Message::Quit => return true,
                    Message::Disconnect => return false,
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
                if connect_to_receiver(addr, &mut msg_rx, &event_tx).await {
                    break;
                }
            }
            Message::Quit => break,
            _ => warn!("Received invalid message ({msg:?}) for the current session state"),
        }
    }

    debug!("Session terminated");

    event_tx.send(Event::Quit).await.unwrap();
}
