//! Cross Platform, Performant and High Quality screen recordings

pub mod capturer;
pub mod frame;
mod pool;
mod targets;
mod utils;

// Helper Methods
pub use targets::get_all_targets;
pub use targets::Target;
pub(crate) use utils::has_permission;
pub(crate) use utils::is_supported;

#[cfg(target_os = "macos")]
pub mod engine {
    pub use crate::capturer::engine::mac;
}
