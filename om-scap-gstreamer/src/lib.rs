// Copyright (C) 2024-2025 Marcus L. Hanestad <marlhan@proton.me>

use gst::glib;

mod scapsrc;

fn plugin_init(plugin: &gst::Plugin) -> Result<(), glib::BoolError> {
    scapsrc::register(plugin)?;
    Ok(())
}

gst::plugin_define!(
    scapgst,
    env!("CARGO_PKG_DESCRIPTION"),
    plugin_init,
    concat!(env!("CARGO_PKG_VERSION"), "-", env!("COMMIT_ID")),
    "MIT/X11",
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_REPOSITORY"),
    env!("BUILD_REL_DATE")
);
