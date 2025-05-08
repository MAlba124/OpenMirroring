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
