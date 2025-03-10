use std::net::Ipv4Addr;

use gst::{glib, prelude::*};
use tokio::sync::mpsc::Sender;

mod hls;
mod webrtc;

const GST_WEBRTC_MIME_TYPE: &str = "application/x-gst-webrtc";
const HLS_MIME_TYPE: &str = "application/vnd.apple.mpegurl";

fn get_default_addr() -> om_common::net::Addr {
    let addrs = om_common::net::get_all_ip_addresses();
    for addr in addrs {
        match addr {
            om_common::net::Addr::V4(v4) => {
                if v4.is_loopback() {
                    continue;
                }
                return addr;
            }
            om_common::net::Addr::V6(v6) => {
                if v6.is_loopback() {
                    continue;
                }
                return addr;
            }
        }
    }

    om_common::net::Addr::V4(Ipv4Addr::LOCALHOST)
}

#[allow(dead_code)]
pub struct HlsSink {
    pub queue: gst::Element,
    convert: gst::Element,
    pub hls: hls::Hls,
    pub server_port: Option<u16>,
}

impl HlsSink {
    pub fn new(
        pipeline: &gst::Pipeline,
        event_tx: Sender<crate::Event>,
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

        Ok(Self {
            queue,
            convert,
            hls,
            server_port: None,
        })
    }

    pub fn get_play_msg(&self) -> Option<crate::Message> {
        if let Some(server_port) = self.server_port {
            Some(crate::Message::Play {
                mime: HLS_MIME_TYPE.to_owned(),
                uri: format!(
                    "http://{}:{server_port}/manifest.m3u8",
                    get_default_addr().to_string()
                ),
            })
        } else {
            None
        }
    }
}

#[allow(dead_code)]
pub struct WebrtcSink {
    pub queue: gst::Element,
    convert: gst::Element,
    webrtc: webrtc::Webrtc,
    pub producer_id: Option<String>,
}

impl WebrtcSink {
    pub fn new(
        pipeline: &gst::Pipeline,
        event_tx: Sender<crate::Event>,
    ) -> Result<Self, glib::BoolError> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
            .build()?;
        let convert = gst::ElementFactory::make("videoconvert")
            .name("sink_convert")
            .build()?;
        let webrtc = webrtc::Webrtc::new(pipeline, event_tx)?;

        pipeline.add_many([&queue, &convert])?;
        gst::Element::link_many([&queue, &convert, &webrtc.sink])?;

        Ok(Self {
            queue,
            convert,
            webrtc,
            producer_id: None,
        })
    }

    pub fn get_play_msg(&self) -> Option<crate::Message> {
        if let Some(producer_id) = &self.producer_id {
            Some(crate::Message::Play {
                mime: GST_WEBRTC_MIME_TYPE.to_owned(),
                uri: format!(
                    "gstwebrtc://{}:8443?peer-id={producer_id}",
                    get_default_addr().to_string()
                ),
            })
        } else {
            None
        }
    }
}
