use gst::{glib, prelude::*};

mod hls;
mod webrtc;

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

    pub fn get_stream_uri(&self) -> String {
        "http://127.0.0.1:6969/manifest.m3u8".to_owned()
        // format!(
        //     "http://127.0.0.1:6969/",
        //     self.hls
        //         .main_path
        //         .to_str()
        //         .expect("Failed to convert manifest path to str")
        // )
    }
}

#[allow(dead_code)]
pub struct WebrtcSink {
    pub queue: gst::Element,
    convert: gst::Element,
    webrtc: webrtc::Webrtc,
}

impl WebrtcSink {
    pub fn new(pipeline: &gst::Pipeline) -> Result<Self, glib::BoolError> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
            .build()?;
        let convert = gst::ElementFactory::make("videoconvert")
            .name("sink_convert")
            .build()?;
        let webrtc = webrtc::Webrtc::new(pipeline)?;

        pipeline.add_many([&queue, &convert])?;
        gst::Element::link_many([&queue, &convert, &webrtc.sink])?;

        Ok(Self {
            queue,
            convert,
            webrtc,
        })
    }

    pub fn get_stream_uri(&self) -> String {
        "gstwebrtc://127.0.0.1:8443?peer-id=todo".to_owned()
    }
}
