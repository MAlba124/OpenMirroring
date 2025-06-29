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

use std::net::IpAddr;

use fcast_lib::models::PlayMessage;

#[cfg(target_os = "android")]
pub mod rtp;
pub mod rtsp;
pub mod whep;

fn addr_to_url_string(addr: IpAddr) -> String {
    match addr {
        IpAddr::V4(ipv4_addr) => ipv4_addr.to_string(),
        IpAddr::V6(ipv6_addr) => format!("[{ipv6_addr}]"),
    }
}

pub fn init() -> anyhow::Result<()> {
    gst_webrtc::plugin_register_static()?;
    Ok(())
}

pub trait TransmissionSink: Send {
    /// Get the message that should be sent to a receiver to consume the stream
    fn get_play_msg(&self, addr: IpAddr) -> Option<PlayMessage>;

    /// Called when the pipeline enters the playing state
    fn playing(&mut self) -> anyhow::Result<()>;

    /// Perform any necessary shutdown procedures
    fn shutdown(&mut self);
}
