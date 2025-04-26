use anyhow::Result;
use gst::prelude::*;

pub struct SlintOpenGlSink {
    pub bin: gst::Bin,
}

impl SlintOpenGlSink {
    pub fn new() -> Result<Self> {
        let bin = gst::Bin::new();
        // let convert = gst::Elem
        let appsink = gst_app::AppSink::builder()
            .enable_last_sample(false)
            .max_buffers(1u32)
            .build();

        bin.add(&appsink)?;

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(|appsink| {
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let Some(appsink_pad) = appsink.static_pad("sink") else {
            anyhow::bail!("Unable to get static sink pad from appsink");
        };

        let bin_sink_ghost_pad = gst::GhostPad::with_target(&appsink_pad)?;
        bin_sink_ghost_pad.set_active(true)?;
        bin.add_pad(&bin_sink_ghost_pad)?;

        Ok(Self { bin })
    }
}
