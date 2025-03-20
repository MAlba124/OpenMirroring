use fcast_lib::{models, models::Header, packet::Packet};
use log::debug;
use std::io::{Read, Write};
use std::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{Event, Message};

const HEADER_BUFFER_SIZE: usize = 5;

fn read_packet_from_stream(stream: &mut TcpStream) -> Result<Option<Packet>, String> {
    let mut header_buf: [u8; HEADER_BUFFER_SIZE] = [0; HEADER_BUFFER_SIZE];

    match stream.read_exact(&mut header_buf) {
        Ok(_) => (),
        Err(err)
            if err.kind() == std::io::ErrorKind::WouldBlock
                || err.kind() == std::io::ErrorKind::TimedOut =>
        {
            return Ok(None)
        }
        Err(err) => panic!("{err}"),
    }

    let header = Header::decode(header_buf);

    let mut body_string = String::new();

    if header.size > 0 {
        let mut body_buf = vec![0; header.size as usize];
        stream.read_exact(&mut body_buf).unwrap();
        body_string = String::from_utf8(body_buf).unwrap();
    }

    Ok(Some(Packet::decode(header, &body_string).unwrap()))
}

fn send_packet(stream: &mut TcpStream, packet: Packet) -> Result<(), String> {
    let bytes = packet.encode();
    stream.write_all(&bytes).unwrap();
    Ok(())
}

pub fn session(mut msg_rx: Receiver<Message>, event_tx: Sender<Event>) {
    let mut stream = TcpStream::connect("127.0.0.1:46899").unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(25)))
        .unwrap();
    loop {
        if let Some(packet) = read_packet_from_stream(&mut stream).unwrap() {
            match packet {
                Packet::Ping => {
                    send_packet(&mut stream, Packet::Ping).unwrap();
                }
                _ => {
                    event_tx.blocking_send(Event::Packet(packet)).unwrap();
                }
            }
        }

        use tokio::sync::mpsc::error::TryRecvError;
        match msg_rx.try_recv() {
            Ok(msg) => {
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
                        send_packet(&mut stream, packet).unwrap();
                    }
                    Message::Quit => break,
                    Message::Stop => send_packet(&mut stream, Packet::Stop).unwrap(),
                }
            }
            Err(TryRecvError::Empty) => (),
            Err(err) => panic!("{err}"),
        }
    }

    debug!("Session terminated");

    event_tx.blocking_send(Event::Quit).unwrap();
}
