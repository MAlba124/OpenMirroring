use std::sync::OnceLock;
use tokio::runtime::Runtime;

pub mod net;

#[cfg(feature = "video")]
pub mod video;

// #[cfg(feature = "sender")]
// pub mod sender;

pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().unwrap())
}

pub fn default_log_level() -> log::LevelFilter {
    if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    }
}
