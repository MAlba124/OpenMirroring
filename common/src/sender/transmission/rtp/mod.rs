use anyhow::Result;
use gio::glib;

use super::{PlayMessage, TransmissionSink};

pub struct RtpSink {
    src_pad: gst::Pad,
}

impl RtpSink {
    pub fn new(pipeline: &gst::Pipeline, src_pad: gst::Pad) -> Result<Self> {
        Ok(Self { src_pad })
    }
}

#[async_trait::async_trait]
impl TransmissionSink for RtpSink {
    fn get_play_msg(&self) -> Option<PlayMessage> {
        None
    }

    async fn playing(&mut self) -> Result<()> {
        Ok(())
    }

    fn shutdown(&mut self) {}

    fn unlink(&mut self, _pipeline: &gst::Pipeline) -> Result<(), glib::error::BoolError> {
        Ok(())
    }
}
