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
