use fcast_lib::{models, models::Header, packet::Packet};
use log::debug;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{Event, Message};

const HEADER_BUFFER_SIZE: usize = 5;

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

pub async fn session(mut msg_rx: Receiver<Message>, event_tx: Sender<Event>) {
    let mut stream = TcpStream::connect("127.0.0.1:46899").await.unwrap();
    loop {
        tokio::select! {
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
                    Message::Quit => break,
                    Message::Stop => send_packet(&mut stream, Packet::Stop).await.unwrap(),
                }
            }
        }
    }

    debug!("Session terminated");

    event_tx.send(Event::Quit).await.unwrap();
}
