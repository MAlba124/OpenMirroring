use iced::futures::{
    channel::mpsc::{self, Sender},
    SinkExt, Stream,
};

use log::{debug, error, warn};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::{packet::Packet, protocol_types::Header, Message};

const HEADER_BUFFER_SIZE: usize = 5;

pub type SessionMessage = Packet;
pub type SessionId = u64;

pub struct Session {
    net_stream: TcpStream,
    id: SessionId,
}

impl Session {
    pub fn new(net_stream: TcpStream, id: SessionId) -> Self {
        Self { net_stream, id }
    }

    async fn get_next_packet(&mut self) -> Result<Packet, tokio::io::Error> {
        let mut header_buf: [u8; HEADER_BUFFER_SIZE] = [0; HEADER_BUFFER_SIZE];

        self.net_stream.read_exact(&mut header_buf).await?;

        let header = Header::decode(header_buf);

        let mut body_string = String::new();

        if header.size > 0 {
            let mut body_buf = vec![0; header.size as usize];
            self.net_stream.read_exact(&mut body_buf).await?;
            body_string =
                String::from_utf8(body_buf).map_err(|e| tokio::io::Error::other(e.to_string()))?;
        }

        Packet::decode(header, &body_string).map_err(|e| tokio::io::Error::other(e.to_string()))
    }

    async fn send_pong(&mut self) -> Result<(), tokio::io::Error> {
        self.net_stream.write_all(&Packet::pong().encode()).await
    }

    async fn handle_packet(
        &mut self,
        pack: Packet,
        tx: &mut Sender<Message>,
    ) -> Result<(), mpsc::SendError> {
        debug!("Got packet: {pack:?}");
        match pack {
            Packet::None => {}
            Packet::Play(play_message) => tx.send(Message::Play(play_message)).await?,
            Packet::Pause => tx.send(Message::Pause).await?,
            Packet::Resume => tx.send(Message::Resume).await?,
            Packet::Stop => tx.send(Message::Stop).await?,
            Packet::Seek(seek_message) => tx.send(Message::Seek(seek_message)).await?,
            Packet::SetVolume(set_volume_message) => {
                tx.send(Message::SetVolume(set_volume_message)).await?
            }
            Packet::SetSpeed(set_speed_message) => {
                tx.send(Message::SetSpeed(set_speed_message)).await?
            }
            Packet::Ping => self.send_pong().await.unwrap(), // TODO: propper error
            Packet::Pong => debug!("Got pong from sender"),
            _ => warn!("Invalid packet from sender: {pack:?}"),
        }

        Ok(())
    }

    async fn handle_message(&mut self, msg: SessionMessage) -> Result<(), tokio::io::Error> {
        let buffer = msg.encode();
        self.net_stream.write_all(&buffer).await?;

        Ok(())
    }

    pub fn stream(
        mut self,
        mut rx: tokio::sync::mpsc::Receiver<SessionMessage>,
    ) -> impl Stream<Item = Message> {
        iced::stream::channel(1, move |mut tx: Sender<Message>| async move {
            loop {
                tokio::select! {
                    pack_res = self.get_next_packet() => {
                        match pack_res {
                            Ok(pack) => if let Err(e) = self.handle_packet(pack, &mut tx).await {
                                error!("Error occured during packet handling: {e}");
                            },
                            Err(e) => {
                                error!("Error occured reading packet: {e}");
                                break;
                            }
                        }
                    }
                    sess_msg = rx.recv() => {
                        match sess_msg {
                            Some(msg) => if let Err(e) = self.handle_message(msg).await {
                                error!("Error occured handling message from ui: {e}");
                            },
                            None => {
                                warn!("Failed to receive from channel");
                            }
                        }
                    }
                }
            }

            tx.send(Message::SessionDestroyed(self.id)).await.unwrap();
        })
    }
}
