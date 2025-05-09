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

use fcast_lib::models::PlayMessage;
use gst::glib;

pub mod hls;
pub mod rtp;

pub fn init() -> Result<(), gst::glib::BoolError> {
    Ok(())
}

#[async_trait::async_trait]
pub trait TransmissionSink: Send {
    /// Get the message that should be sent to a receiver to consume the stream
    fn get_play_msg(&self) -> Option<PlayMessage>;

    /// Called when the pipeline enters the playing state
    async fn playing(&mut self) -> anyhow::Result<()>;

    /// Perform any necessary shutdown procedures
    fn shutdown(&mut self);

    /// Remove the sink's elements from the pipeline and unlink them from the source
    fn unlink(&mut self, pipeline: &gst::Pipeline) -> Result<(), glib::error::BoolError>;
}
