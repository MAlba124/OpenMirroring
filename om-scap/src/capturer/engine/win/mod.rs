use crate::{
    capturer::{OnFormatChangedCb, OnFrameCb, Options},
    targets::{self, Target},
};
use log::debug;
use windows_capture::capture::Context;
use windows_capture::{
    capture::{CaptureControl, GraphicsCaptureApiHandler},
    frame::Frame as WCFrame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor as WCMonitor,
    settings::{ColorFormat, CursorCaptureSettings, DrawBorderSettings, Settings as WCSettings},
    window::Window as WCWindow,
};

struct Capturer {
    info: crate::frame::FrameInfo,
    on_format_changed: OnFormatChangedCb,
    on_frame: OnFrameCb,
    base_time: u64,
}

#[derive(Clone)]
enum Settings {
    Window(WCSettings<FlagStruct, WCWindow>),
    Display(WCSettings<FlagStruct, WCMonitor>),
}

pub struct WCStream {
    settings: Settings,
    capture_control: Option<CaptureControl<Capturer, Box<dyn std::error::Error + Send + Sync>>>,
}

impl GraphicsCaptureApiHandler for Capturer {
    type Flags = FlagStruct;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let mut on_format_changed = context.flags.on_format_changed.lock().unwrap();
        let mut on_frame = context.flags.on_frame.lock().unwrap();

        Ok(Self {
            info: crate::frame::FrameInfo {
                format: crate::frame::FrameFormat::BGRA,
                width: 0,
                height: 0,
            },
            on_format_changed: on_format_changed.take().unwrap(),
            on_frame: on_frame.take().unwrap(),
            base_time: 0,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut WCFrame,
        _: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let (width, height) = (frame.width(), frame.height());

        let this_info = crate::frame::FrameInfo {
            format: crate::frame::FrameFormat::BGRA,
            width,
            height,
        };

        if self.info != this_info {
            (self.on_format_changed)(this_info.clone());
            self.info = this_info;
        }

        let mut pts = frame.timespan().Duration as u64 * 100;
        if self.base_time == 0 {
            self.base_time = pts;
        }
        pts -= self.base_time;
        let mut frame_buffer = frame.buffer().unwrap();
        let raw_frame_buffer = frame_buffer.as_raw_buffer();
        (self.on_frame)(pts, raw_frame_buffer);

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        debug!("Closed");
        Ok(())
    }
}

impl WCStream {
    pub fn start_capture(&mut self) {
        if self.capture_control.is_some() {
            return;
        }

        let cc = match &self.settings {
            Settings::Display(st) => Capturer::start_free_threaded(st.to_owned()).unwrap(),
            Settings::Window(st) => Capturer::start_free_threaded(st.to_owned()).unwrap(),
        };

        self.capture_control = Some(cc)
    }

    pub fn stop_capture(&mut self) {
        let capture_control = self.capture_control.take().unwrap();
        let _ = capture_control.stop();
    }
}

#[derive(Clone)]
struct FlagStruct {
    pub on_format_changed: std::sync::Arc<std::sync::Mutex<Option<OnFormatChangedCb>>>,
    pub on_frame: std::sync::Arc<std::sync::Mutex<Option<OnFrameCb>>>,
}

pub fn create_capturer(
    options: &Options,
    on_format_changed: OnFormatChangedCb,
    on_frame: OnFrameCb,
) -> WCStream {
    let target = options
        .target
        .clone()
        .unwrap_or_else(|| Target::Display(targets::get_main_display()));

    let color_format = ColorFormat::Bgra8;

    let show_cursor = match options.show_cursor {
        true => CursorCaptureSettings::WithCursor,
        false => CursorCaptureSettings::WithoutCursor,
    };

    let settings = match target {
        Target::Display(display) => Settings::Display(WCSettings::new(
            WCMonitor::from_raw_hmonitor(display.raw_handle.0),
            show_cursor,
            DrawBorderSettings::Default,
            color_format,
            FlagStruct {
                on_format_changed: std::sync::Arc::new(std::sync::Mutex::new(Some(
                    on_format_changed,
                ))),
                on_frame: std::sync::Arc::new(std::sync::Mutex::new(Some(on_frame))),
            },
        )),
        Target::Window(window) => Settings::Window(WCSettings::new(
            WCWindow::from_raw_hwnd(window.raw_handle.0),
            show_cursor,
            DrawBorderSettings::Default,
            color_format,
            FlagStruct {
                on_format_changed: std::sync::Arc::new(std::sync::Mutex::new(Some(
                    on_format_changed,
                ))),
                on_frame: std::sync::Arc::new(std::sync::Mutex::new(Some(on_frame))),
            },
        )),
    };

    WCStream {
        settings,
        capture_control: None,
    }
}
