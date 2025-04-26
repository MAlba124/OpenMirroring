use anyhow::Result;

use super::{OnFormatChangedCb, OnFrameCb, Options};

#[cfg(target_os = "macos")]
pub mod mac;

#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "linux")]
pub(crate) mod linux;

#[cfg(target_os = "macos")]
pub type ChannelItem = (
    screencapturekit::cm_sample_buffer::CMSampleBuffer,
    screencapturekit::sc_output_handler::SCStreamOutputType,
);

pub struct Engine {
    #[cfg(target_os = "macos")]
    mac: screencapturekit::sc_stream::SCStream,
    #[cfg(target_os = "macos")]
    error_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,

    #[cfg(target_os = "windows")]
    win: win::WCStream,

    #[cfg(target_os = "linux")]
    linux: linux::LinuxCapturer,
}

impl Engine {
    pub fn new(
        options: Options,
        on_format_changed: OnFormatChangedCb,
        on_frame: OnFrameCb,
    ) -> Result<Engine> {
        #[cfg(target_os = "macos")]
        {
            let error_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let mac = mac::create_capturer(options, tx, error_flag.clone());

            Ok(Engine { mac, error_flag })
        }

        #[cfg(target_os = "windows")]
        {
            let win = win::create_capturer(&options, on_format_changed, on_frame);
            Ok(Engine { win })
        }

        #[cfg(target_os = "linux")]
        {
            let linux = linux::create_capturer(options, on_format_changed, on_frame)?;
            Ok(Engine { linux })
        }
    }

    pub fn start(&mut self) {
        #[cfg(target_os = "macos")]
        {
            // self.mac.add_output(Capturer::new(tx));
            self.mac.start_capture().expect("Failed to start capture");
        }

        #[cfg(target_os = "windows")]
        {
            self.win.start_capture();
        }

        #[cfg(target_os = "linux")]
        {
            self.linux.imp.start();
        }
    }

    pub fn stop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.mac.stop_capture().expect("Failed to stop capture");
        }

        #[cfg(target_os = "windows")]
        {
            self.win.stop_capture();
        }

        #[cfg(target_os = "linux")]
        {
            self.linux.imp.stop();
        }
    }

    // pub fn process_channel_item(&self, data: ChannelItem) -> Option<Frame> {
    //     #[cfg(target_os = "macos")]
    //     {
    //         mac::process_sample_buffer(data.0, data.1, self.options.output_type)
    //     }
    //     #[cfg(not(target_os = "macos"))]
    //     Some(data)
    // }
}
