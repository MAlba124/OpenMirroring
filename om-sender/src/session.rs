use fcast_lib::{models, models::Header, packet::Packet};
use log::debug;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{Event, Message};

const HEADER_BUFFER_SIZE: usize = 5;
const GST_WEBRTC_MIME_TYPE: &str = "application/x-gst-webrtc";

async fn read_packet_from_stream(stream: &mut TcpStream) -> Result<Packet, tokio::io::Error> {
    let mut header_buf: [u8; HEADER_BUFFER_SIZE] = [0; HEADER_BUFFER_SIZE];

    stream.read_exact(&mut header_buf).await?;

    let header = Header::decode(header_buf);

    let mut body_string = String::new();

    if header.size > 0 {
        let mut body_buf = vec![0; header.size as usize];
        stream.read_exact(&mut body_buf).await?;
        body_string =
            String::from_utf8(body_buf).map_err(|e| tokio::io::Error::other(e.to_string()))?;
    }

    Packet::decode(header, &body_string).map_err(|e| tokio::io::Error::other(e.to_string()))
}

async fn send_packet(stream: &mut TcpStream, packet: Packet) -> Result<(), tokio::io::Error> {
    let bytes = packet.encode();
    stream.write_all(&bytes).await?;
    Ok(())
}

pub async fn session(mut msg_rx: Receiver<Message>, event_tx: Sender<Event>) {
    let mut stream = TcpStream::connect("127.0.0.1:46899").await.unwrap();
    // let mut stream = TcpStream::connect("192.168.1.23:46899").await.unwrap();
    loop {
        tokio::select! {
            packet = read_packet_from_stream(&mut stream) => {
                match packet {
                    Ok(packet) => match packet {
                        Packet::Ping => {
                            send_packet(&mut stream, Packet::Pong).await.unwrap();
                        }
                        _ => {
                            event_tx.send(Event::Packet(packet)).await.unwrap();
                        }
                    },
                    Err(err) => panic!("{err}"),
                }
            }
            msg = msg_rx.recv() => match msg {
                Some(msg) => {
                    debug!("{msg:?}");
                    match msg {
                        Message::Play(url) => {
                            let packet = Packet::from(
                                models::PlayMessage {
                                    // container: GST_WEBRTC_MIME_TYPE.to_owned(),
                                    container: "application/vnd.apple.mpegurl".to_owned(),
                                    url: Some(url),
                                    content: None,
                                    time: None,
                                    speed: None,
                                    headers: None
                                }
                            );
                            send_packet(&mut stream, packet).await.unwrap();
                        }
                        Message::Quit => break,
                        Message::Stop => send_packet(&mut stream, Packet::Stop).await.unwrap(),
                    }
                }
                None => panic!("rx closed"), // TODO
            }
        }
    }

    debug!("Session terminated");
}
