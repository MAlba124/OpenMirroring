use gst::{glib, prelude::*};

mod hls;
mod webrtc;

const GST_WEBRTC_MIME_TYPE: &str = "application/x-gst-webrtc";
const HLS_MIME_TYPE: &str = "application/vnd.apple.mpegurl";

#[allow(dead_code)]
pub struct HlsSink {
    pub queue: gst::Element,
    convert: gst::Element,
    pub hls: hls::Hls,
}

impl HlsSink {
    pub fn new(pipeline: &gst::Pipeline) -> Result<Self, glib::BoolError> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
            .build()?;
        let convert = gst::ElementFactory::make("videoconvert")
            .name("sink_convert")
            .build()?;
        let hls = hls::Hls::new(pipeline)?;

        pipeline.add_many([&queue, &convert])?;
        gst::Element::link_many([&queue, &convert, &hls.enc, &hls.enc_caps, &hls.sink])?;

        Ok(Self {
            queue,
            convert,
            hls,
        })
    }

    pub fn get_play_msg(&self) -> Option<crate::Message> {
        Some(crate::Message::Play {
            mime: HLS_MIME_TYPE.to_owned(),
            uri: "http://127.0.0.1:6969/manifest.m3u8".to_owned(),
        })
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
        event_tx: tokio::sync::mpsc::Sender<crate::Event>,
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
            Some(
                crate::Message::Play {
                    mime: GST_WEBRTC_MIME_TYPE.to_owned(),
                    uri: format!("gstwebrtc://127.0.0.1:8443?peer-id={producer_id}"),
                }
            )
        } else {
            None
        }
    }
}
