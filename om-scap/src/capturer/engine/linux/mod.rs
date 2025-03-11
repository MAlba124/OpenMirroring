use std::{env, sync::{Arc, Mutex}};

use log::debug;
use wayland::WaylandCapturer;
use x11::X11Capturer;

use crate::{capturer::Options, frame::Frame, pool::FramePool};

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
    pub fn new(options: &Options, tx: crossbeam_channel::Sender<Frame>, pool: Arc<Mutex<FramePool>>) -> Self {
        if env::var("WAYLAND_DISPLAY").is_ok() {
            debug!("On wayland");
            Self {
                imp: Box::new(WaylandCapturer::new(options, tx, pool)),
            }
        } else if env::var("DISPLAY").is_ok() {
            debug!("On X11");
            return Self {
                imp: Box::new(X11Capturer::new(options, tx).unwrap()),
            };
        } else {
            panic!("Unsupported platform. Could not detect Wayland or X11 displays")
        }
    }
}

pub fn create_capturer(options: &Options, tx: crossbeam_channel::Sender<Frame>, pool: Arc<Mutex<FramePool>>) -> LinuxCapturer {
    LinuxCapturer::new(options, tx, pool)
}
