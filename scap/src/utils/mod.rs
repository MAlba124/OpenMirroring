#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "windows")]
mod win;

/// Checks if process has permission to capture the screen
pub fn has_permission() -> bool {
    #[cfg(target_os = "macos")]
    return mac::has_permission();

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    return true;
}

/// Prompts user to grant screen capturing permission to current process
#[allow(dead_code)]
pub fn request_permission() -> bool {
    #[cfg(target_os = "macos")]
    return mac::request_permission();

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    return true;
}

/// Checks if scap is supported on the current system
pub fn is_supported() -> bool {
    #[cfg(target_os = "macos")]
    return mac::is_supported();

    #[cfg(target_os = "windows")]
    return win::is_supported();

    #[cfg(target_os = "linux")]
    return true;
}
