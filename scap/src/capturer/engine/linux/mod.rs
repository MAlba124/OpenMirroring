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

use std::env;

use log::debug;
use wayland::WaylandCapturer;
use x11::X11Capturer;

use anyhow::{bail, Result};

use crate::capturer::{OnFrameCb, Options};

use super::OnFormatChangedCb;

mod wayland;
mod x11;

pub trait LinuxCapturerImpl: Send + Sync {
    fn start(&mut self);
    fn stop(&mut self);
}

pub struct LinuxCapturer {
    pub imp: Box<dyn LinuxCapturerImpl>,
}

impl LinuxCapturer {
    pub fn new(
        options: Options,
        on_format_changed: OnFormatChangedCb,
        on_frame: OnFrameCb,
    ) -> Result<Self> {
        if env::var("WAYLAND_DISPLAY").is_ok() {
            debug!("On wayland");
            Ok(Self {
                imp: Box::new(WaylandCapturer::new(options, on_format_changed, on_frame)?),
            })
        } else if env::var("DISPLAY").is_ok() {
            debug!("On X11");
            Ok(Self {
                imp: Box::new(X11Capturer::new(options, on_format_changed, on_frame)?),
            })
        } else {
            bail!("Unsupported platform. Could not detect Wayland or X11 displays")
        }
    }
}

pub fn create_capturer(
    options: Options,
    on_format_changed: OnFormatChangedCb,
    on_frame: OnFrameCb,
) -> Result<LinuxCapturer> {
    LinuxCapturer::new(options, on_format_changed, on_frame)
}
