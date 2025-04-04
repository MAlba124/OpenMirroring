pub mod engine;

use std::error::Error;

use crate::{frame::FrameInfo, has_permission, is_supported, targets::Target};

/// Presentation timestamp
pub type Pts = u64;

pub type OnFormatChangedCb = Box<dyn FnMut(FrameInfo) + Send + Sync>;
pub type OnFrameCb = Box<dyn FnMut(Pts, &[u8]) + Send + Sync>;

#[derive(Debug, Clone, Copy, Default)]
pub enum Resolution {
    _480p,
    _720p,
    _1080p,
    _1440p,
    _2160p,
    _4320p,

    #[default]
    Captured,
}

#[allow(dead_code)]
impl Resolution {
    fn value(&self, aspect_ratio: f32) -> [u32; 2] {
        match *self {
            Resolution::_480p => [640, (640_f32 / aspect_ratio).floor() as u32],
            Resolution::_720p => [1280, (1280_f32 / aspect_ratio).floor() as u32],
            Resolution::_1080p => [1920, (1920_f32 / aspect_ratio).floor() as u32],
            Resolution::_1440p => [2560, (2560_f32 / aspect_ratio).floor() as u32],
            Resolution::_2160p => [3840, (3840_f32 / aspect_ratio).floor() as u32],
            Resolution::_4320p => [7680, (7680_f32 / aspect_ratio).floor() as u32],
            Resolution::Captured => {
                panic!(".value should not be called when Resolution type is Captured")
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Default, Clone)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Default, Clone)]
pub struct Area {
    pub origin: Point,
    pub size: Size,
}

/// Options passed to the screen capturer
#[derive(Debug, Default, Clone)]
pub struct Options {
    pub fps: u32,
    pub show_cursor: bool,
    pub show_highlight: bool,
    pub target: Option<Target>,
    pub output_resolution: Resolution,
}

/// Screen capturer class
pub struct Capturer {
    engine: engine::Engine,
}

#[derive(Debug)]
pub enum CapturerBuildError {
    NotSupported,
    PermissionNotGranted,
}

impl std::fmt::Display for CapturerBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapturerBuildError::NotSupported => write!(f, "Screen capturing is not supported"),
            CapturerBuildError::PermissionNotGranted => {
                write!(f, "Permission to capture the screen is not granted")
            }
        }
    }
}

impl Error for CapturerBuildError {}

impl Capturer {
    /// Build a new [Capturer] instance with the provided options
    pub fn build(
        options: Options,
        on_format_changed: OnFormatChangedCb,
        on_frame: OnFrameCb,
    ) -> Result<Capturer, CapturerBuildError> {
        if !is_supported() {
            return Err(CapturerBuildError::NotSupported);
        }

        if !has_permission() {
            return Err(CapturerBuildError::PermissionNotGranted);
        }

        let engine = engine::Engine::new(options, on_format_changed, on_frame);

        Ok(Capturer { engine })
    }

    // TODO
    // Prevent starting capture if already started
    /// Start capturing the frames
    pub fn start_capture(&mut self) {
        self.engine.start();
    }

    /// Stop the capturer
    pub fn stop_capture(&mut self) {
        self.engine.stop();
    }

    pub fn raw(&self) -> RawCapturer {
        RawCapturer { capturer: self }
    }
}

#[allow(dead_code)]
pub struct RawCapturer<'a> {
    capturer: &'a Capturer,
}
