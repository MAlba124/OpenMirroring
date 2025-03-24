use std::env;

use log::debug;
use wayland::WaylandCapturer;
use x11::X11Capturer;

use crate::capturer::{OnFrameCb, Options};

use super::OnFormatChangedCb;

pub(crate) mod error;

mod wayland;
mod x11;

pub trait LinuxCapturerImpl: Send + Sync {
    fn start_capture(&mut self);
    fn stop_capture(&mut self);
}

pub struct LinuxCapturer {
    pub imp: Box<dyn LinuxCapturerImpl>,
}

impl LinuxCapturer {
    pub fn new(
        options: Options,
        on_format_changed: OnFormatChangedCb,
        on_frame: OnFrameCb,
    ) -> Self {
        if env::var("WAYLAND_DISPLAY").is_ok() {
            debug!("On wayland");
            Self {
                imp: Box::new(WaylandCapturer::new(options, on_format_changed, on_frame)),
            }
        } else if env::var("DISPLAY").is_ok() {
            debug!("On X11");
            return Self {
                imp: Box::new(X11Capturer::new(options, on_format_changed, on_frame).unwrap()),
            };
        } else {
            panic!("Unsupported platform. Could not detect Wayland or X11 displays")
        }
    }
}

pub fn create_capturer(
    options: Options,
    on_format_changed: OnFormatChangedCb,
    on_frame: OnFrameCb,
) -> LinuxCapturer {
    LinuxCapturer::new(options, on_format_changed, on_frame)
}
