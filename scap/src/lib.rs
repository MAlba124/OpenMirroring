// TODO: handle transforms
// TODO: on error callback

pub mod capturer;
pub mod frame;
mod targets;
mod utils;

pub use targets::get_all_targets;
pub use targets::Target;
pub(crate) use utils::has_permission;
pub(crate) use utils::is_supported;

#[cfg(target_os = "macos")]
pub mod engine {
    pub use crate::capturer::engine::mac;
}
