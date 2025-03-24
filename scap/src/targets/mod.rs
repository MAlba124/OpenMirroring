use std::fmt::Debug;

#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex};

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "linux")]
pub(crate) mod linux;

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub(crate) enum LinuxWindow {
    #[allow(dead_code)]
    Wayland,
    X11 {
        raw_handle: xcb::x::Window,
    },
}

#[cfg(target_os = "linux")]
#[derive(Clone)]
pub(crate) enum LinuxDisplay {
    Wayland {
        connection: Arc<Mutex<dbus::blocking::Connection>>,
    },
    X11 {
        raw_handle: xcb::x::Window,
        width: u16,
        height: u16,
        x_offset: i16,
        y_offset: i16,
    },
}

#[cfg(target_os = "linux")]
impl Debug for LinuxDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: finishme
        write!(f, "Display")
    }
}

#[derive(Debug, Clone)]
pub struct Window {
    pub id: u32,
    pub title: String,

    #[cfg(target_os = "windows")]
    pub raw_handle: windows::Win32::Foundation::HWND,

    #[cfg(target_os = "macos")]
    pub raw_handle: core_graphics_helmer_fork::window::CGWindowID,

    #[cfg(target_os = "linux")]
    pub(crate) raw: LinuxWindow,
}

#[derive(Debug, Clone)]
pub struct Display {
    pub id: u32,
    pub title: String,

    #[cfg(target_os = "windows")]
    pub raw_handle: windows::Win32::Graphics::Gdi::HMONITOR,

    #[cfg(target_os = "macos")]
    pub raw_handle: core_graphics_helmer_fork::display::CGDisplay,

    #[cfg(target_os = "linux")]
    pub(crate) raw: LinuxDisplay,
}

#[derive(Debug, Clone)]
pub enum Target {
    Window(Window),
    Display(Display),
}

impl Target {
    pub fn title(&self) -> String {
        match self {
            Target::Window(window) => window.title.clone(),
            Target::Display(display) => display.title.clone(),
        }
    }
}

/// Returns a list of targets that can be captured
pub fn get_all_targets() -> Vec<Target> {
    #[cfg(target_os = "macos")]
    return mac::get_all_targets();

    #[cfg(target_os = "windows")]
    return win::get_all_targets();

    #[cfg(target_os = "linux")]
    return linux::get_all_targets();
}

#[allow(dead_code)]
pub fn get_main_display() -> Display {
    #[cfg(target_os = "macos")]
    return mac::get_main_display();

    #[cfg(target_os = "windows")]
    return win::get_main_display();

    #[cfg(target_os = "linux")]
    return linux::get_main_display();
}
