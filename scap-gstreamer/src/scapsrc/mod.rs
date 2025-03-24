// Copyright (C) 2024-2025 Marcus L. Hanestad <marlhan@proton.me>

use gst::glib;
use gst::prelude::*;

mod imp;

glib::wrapper! {
    pub struct ScapSrc(ObjectSubclass<imp::ScapSrc>) @extends gst_base::PushSrc, gst_base::BaseSrc, gst::Element, gst::Object;
}

pub fn register(plugin: &gst::Plugin) -> Result<(), glib::BoolError> {
    gst::Element::register(
        Some(plugin),
        "scapsrc",
        gst::Rank::NONE,
        ScapSrc::static_type(),
    )
}
