use std::net::{IpAddr, Ipv4Addr};

use gst::{glib, prelude::*};
use tokio::sync::{mpsc::Sender, oneshot};

mod hls;
mod webrtc;

const GST_WEBRTC_MIME_TYPE: &str = "application/x-gst-webrtc";
const HLS_MIME_TYPE: &str = "application/vnd.apple.mpegurl";

fn get_default_ipv4_addr() -> Ipv4Addr {
    let addrs = common::net::get_all_ip_addresses();
    for addr in addrs {
        if let IpAddr::V4(v4) = addr {
            if v4.is_loopback() {
                continue;
            }
            return v4;
        }
    }

    Ipv4Addr::LOCALHOST
}

// TODO: use a Sink trait

#[derive(Debug)]
pub struct HlsSink {
    src_pad: gst::Pad,
    queue_pad: gst::Pad,
    queue: gst::Element,
    convert: gst::Element,
    pub hls: hls::Hls,
    pub server_port: Option<u16>,
}

impl HlsSink {
    pub fn new(
        pipeline: &gst::Pipeline,
        event_tx: Sender<crate::Event>,
        src_pad: gst::Pad,
    ) -> Result<Self, glib::BoolError> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
            .build()?;
        let convert = gst::ElementFactory::make("videoconvert")
            .name("sink_convert")
            .build()?;
        let hls = hls::Hls::new(pipeline, event_tx)?;

        pipeline.add_many([&queue, &convert])?;
        gst::Element::link_many([&queue, &convert, &hls.enc, &hls.enc_caps, &hls.sink])?;

        queue.sync_state_with_parent().unwrap();
        convert.sync_state_with_parent().unwrap();
        hls.enc.sync_state_with_parent().unwrap();
        hls.enc_caps.sync_state_with_parent().unwrap();
        hls.sink.sync_state_with_parent().unwrap();

        let queue_pad = queue
            .static_pad("sink")
            .map_or_else(|| Err(glib::bool_error!("`static_pad()` failed")), Ok)?;
        src_pad
            .link(&queue_pad)
            .map_err(|err| glib::bool_error!("{err}"))?;

        Ok(Self {
            src_pad,
            queue_pad,
            queue,
            convert,
            hls,
            server_port: None,
        })
    }

    pub fn get_play_msg(&self) -> Option<crate::Message> {
        self.server_port.map(|server_port| crate::Message::Play {
            mime: HLS_MIME_TYPE.to_owned(),
            uri: format!(
                "http://{}:{server_port}/manifest.m3u8",
                get_default_ipv4_addr(),
            ),
        })
    }

    pub fn shutdown_and_unlink(
        &mut self,
        pipeline: &gst::Pipeline,
    ) -> Result<(), glib::error::BoolError> {
        let block = self
            .src_pad
            .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                gst::PadProbeReturn::Ok
            })
            .unwrap();
        self.src_pad.unlink(&self.queue_pad)?;
        self.src_pad.remove_probe(block);

        let elems = [
            &self.queue,
            &self.convert,
            &self.hls.enc,
            &self.hls.enc_caps,
            &self.hls.sink,
        ];

        pipeline.remove_many(&elems)?;

        for elem in elems {
            elem.set_state(gst::State::Null)
                .map_err(|err| glib::bool_error!("{err}"))?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct WebrtcSink {
    src_pad: gst::Pad,
    queue_pad: gst::Pad,
    queue: gst::Element,
    convert: gst::Element,
    pub producer_id: Option<String>,
    signaller_quit_tx: Option<oneshot::Sender<()>>,
    webrtc: webrtc::Webrtc,
}

impl WebrtcSink {
    pub fn new(
        pipeline: &gst::Pipeline,
        event_tx: Sender<crate::Event>,
        src_pad: gst::Pad,
    ) -> Result<Self, glib::BoolError> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
            .build()?;
        let convert = gst::ElementFactory::make("videoconvert")
            .name("sink_convert")
            .build()?;
        let (signaller_quit_tx, signaller_quit_rx) = oneshot::channel();
        let webrtc = webrtc::Webrtc::new(pipeline, event_tx, signaller_quit_rx)?;

        pipeline.add_many([&queue, &convert])?;
        gst::Element::link_many([&queue, &convert, &webrtc.sink])?;

        queue.sync_state_with_parent().unwrap();
        convert.sync_state_with_parent().unwrap();
        webrtc.sink.sync_state_with_parent().unwrap();

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
            producer_id: None,
            signaller_quit_tx: Some(signaller_quit_tx),
            webrtc,
        })
    }

    pub fn get_play_msg(&self) -> Option<crate::Message> {
        self.producer_id
            .as_ref()
            .map(|producer_id| crate::Message::Play {
                mime: GST_WEBRTC_MIME_TYPE.to_owned(),
                uri: format!(
                    "gstwebrtc://{}:8443?peer-id={producer_id}",
                    get_default_ipv4_addr(),
                ),
            })
    }

    pub fn shutdown(&mut self) {
        if let Some(signaller_quit_tx) = self.signaller_quit_tx.take() {
            signaller_quit_tx.send(()).unwrap();
        }
    }

    pub fn shutdown_and_unlink(
        &mut self,
        pipeline: &gst::Pipeline,
    ) -> Result<(), glib::error::BoolError> {
        self.shutdown();
        let block = self
            .src_pad
            .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                gst::PadProbeReturn::Ok
            })
            .unwrap();
        self.src_pad.unlink(&self.queue_pad)?;
        self.src_pad.remove_probe(block);

        let elems = [&self.queue, &self.convert, &self.webrtc.sink];

        pipeline.remove_many(&elems)?;

        for elem in elems {
            elem.set_state(gst::State::Null)
                .map_err(|err| glib::bool_error!("{err}"))?;
        }

        Ok(())
    }
}
