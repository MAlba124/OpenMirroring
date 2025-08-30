pub mod models;
pub mod packet;

use anyhow::Result;
use models::Header;
use packet::Packet;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::tcp::{ReadHalf, WriteHalf},
};

pub const HEADER_BUFFER_SIZE: usize = 5;
pub const MAX_BODY_SIZE: u32 = 32000 - 1;

/// Attempt to read and decode FCast packet from `stream`.
pub async fn read_packet(stream: &mut ReadHalf<'_>) -> Result<Packet> {
    let mut header_buf: [u8; HEADER_BUFFER_SIZE] = [0; HEADER_BUFFER_SIZE];

    stream.read_exact(&mut header_buf).await?;

    let header = Header::decode(header_buf);

    let mut body_string = String::new();

    if header.size > 0 {
        let mut body_buf = vec![0; header.size as usize];
        stream.read_exact(&mut body_buf).await?;
        body_string = String::from_utf8(body_buf)?;
    }

    Packet::decode(header, &body_string)
}

pub async fn write_packet(stream: &mut WriteHalf<'_>, packet: Packet) -> Result<()> {
    let bytes = packet.encode();
    stream.write_all(&bytes).await?;
    Ok(())
}
