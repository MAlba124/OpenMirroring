use super::{Display, Target};
use windows::Win32::{Foundation::HWND, Graphics::Gdi::HMONITOR};
use windows_capture::{monitor::Monitor, window::Window};

use anyhow::{Context, Result};

pub fn get_all_targets() -> Result<Vec<Target>> {
    let mut targets: Vec<Target> = Vec::new();

    // Add displays to targets
    let displays = Monitor::enumerate().context("Failed to enumerate monitors")?;
    for display in displays {
        let id = display.as_raw_hmonitor() as u32;
        let title = display
            .device_name()
            .context("Failed to get monitor name")?;

        let target = Target::Display(super::Display {
            id,
            title,
            raw_handle: HMONITOR(display.as_raw_hmonitor()),
        });
        targets.push(target);
    }

    // Add windows to targets
    let windows = Window::enumerate().context("Failed to enumerate windows")?;
    for window in windows {
        let id = window.as_raw_hwnd() as u32;
        let title = window.title().unwrap().to_string();

        let target = Target::Window(super::Window {
            id,
            title,
            raw_handle: HWND(window.as_raw_hwnd()),
        });
        targets.push(target);
    }

    Ok(targets)
}

pub fn get_main_display() -> Result<Display> {
    let display = Monitor::primary().context("Failed to get primary monitor")?;
    let id = display.as_raw_hmonitor() as u32;

    Ok(Display {
        id,
        title: display
            .device_name()
            .context("Failed to get monitor name")?,
        raw_handle: HMONITOR(display.as_raw_hmonitor()),
    })
}
