use gst::prelude::*;
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
        common::runtime().spawn(signaller::run_server(event_tx));

        let sink = gst::ElementFactory::make("webrtcsink")
            .name("webrtc_sink")
            .build()?;

        let signaller = sink.property::<gst_webrtc::signaller::Signaller>("signaller");
        signaller.set_property("uri", "ws://127.0.0.1:8443");

        pipeline.add_many([&sink])?;

        Ok(Self { sink })
    }
}
