use std::sync::Arc;
use std::sync::Mutex;

use common::net::get_default_ipv4_addr;
use gst::glib;
use gst::prelude::*;
use tokio::sync::oneshot;

use super::TransmissionSink;

mod signaller;

const GST_WEBRTC_MIME_TYPE: &str = "application/x-gst-webrtc";

pub struct WebrtcSink {
    src_pad: gst::Pad,
    queue_pad: gst::Pad,
    queue: gst::Element,
    convert: gst::Element,
    signaller_quit_tx: Option<oneshot::Sender<()>>,
    sink: gst::Element,
    peer_id: Arc<Mutex<Option<String>>>,
}

impl WebrtcSink {
    pub fn new(pipeline: &gst::Pipeline, src_pad: gst::Pad) -> Result<Self, glib::BoolError> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
            .build()?;
        let convert = gst::ElementFactory::make("videoconvert")
            .name("sink_convert")
            .build()?;
        let (signaller_quit_tx, signaller_quit_rx) = oneshot::channel();

        let peer_id = Arc::new(Mutex::new(None));
        common::runtime().spawn(signaller::run_server(
            Arc::clone(&peer_id),
            signaller_quit_rx,
        ));

        let sink = gst::ElementFactory::make("webrtcsink")
            .name("webrtc_sink")
            .build()?;

        let signaller = sink.property::<gst_webrtc::signaller::Signaller>("signaller");
        signaller.set_property("uri", "ws://127.0.0.1:8443");

        pipeline.add_many([&sink])?;

        pipeline.add_many([&queue, &convert])?;
        gst::Element::link_many([&queue, &convert, &sink])?;

        queue.sync_state_with_parent().unwrap();
        convert.sync_state_with_parent().unwrap();
        sink.sync_state_with_parent().unwrap();

        let queue_pad = queue
            .static_pad("sink")
            .map_or_else(|| Err(glib::bool_error!("`static_pad()` failed")), Ok)?;

        let src_pad_block = src_pad
            .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                gst::PadProbeReturn::Ok
            })
            .unwrap();

        src_pad
            .link(&queue_pad)
            .map_err(|err| glib::bool_error!("{err}"))?;

        src_pad.remove_probe(src_pad_block);

        Ok(Self {
            src_pad,
            queue_pad,
            queue,
            convert,
            signaller_quit_tx: Some(signaller_quit_tx),
            sink,
            peer_id,
        })
    }
}

#[async_trait::async_trait]
impl TransmissionSink for WebrtcSink {
    fn get_play_msg(&self) -> Option<crate::Message> {
        let peer_id = self.peer_id.lock().unwrap();
        (*peer_id).as_ref().map(|producer_id| crate::Message::Play {
            mime: GST_WEBRTC_MIME_TYPE.to_owned(),
            uri: format!(
                "gstwebrtc://{}:8443?peer-id={producer_id}",
                get_default_ipv4_addr(),
            ),
        })
    }

    async fn playing(&mut self) {}

    fn shutdown(&mut self) {
        if let Some(signaller_quit_tx) = self.signaller_quit_tx.take() {
            signaller_quit_tx.send(()).unwrap();
        }
    }

    fn unlink(&mut self, pipeline: &gst::Pipeline) -> Result<(), glib::error::BoolError> {
        let block = self
            .src_pad
            .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                gst::PadProbeReturn::Ok
            })
            .unwrap();
        self.src_pad.unlink(&self.queue_pad)?;
        self.src_pad.remove_probe(block);

        let elems = [&self.queue, &self.convert, &self.sink];

        pipeline.remove_many(elems)?;

        for elem in elems {
            elem.set_state(gst::State::Null)
                .map_err(|err| glib::bool_error!("{err}"))?;
        }

        Ok(())
    }
}
