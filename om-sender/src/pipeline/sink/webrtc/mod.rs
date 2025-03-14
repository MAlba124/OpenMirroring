use gst::prelude::GstBinExtManual;
use tokio::sync::mpsc::Sender;

mod signaller;

pub struct Webrtc {
    pub sink: gst::Element,
}

impl Webrtc {
    pub fn new(
        pipeline: &gst::Pipeline,
        event_tx: Sender<crate::Event>,
    ) -> Result<Self, gst::glib::BoolError> {
        om_common::runtime().spawn(signaller::run_server(event_tx));

        let sink = gst::ElementFactory::make("webrtcsink")
            .name("webrtc_sink")
            .property("signalling-server-host", "127.0.0.1")
            .property("signalling-server-port", 8443u32)
            .build()?;

        pipeline.add_many([&sink])?;

        Ok(Self { sink })
    }
}
