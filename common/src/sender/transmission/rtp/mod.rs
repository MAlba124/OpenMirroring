use std::{net::IpAddr, str::FromStr};

use anyhow::Result;
use gio::glib;
use gst::prelude::{ElementExt, GstBinExtManual, PadExt, PadExtManual};

use super::{PlayMessage, TransmissionSink};

const FCAST_GST_WEBRTC_MIME_TYPE: &str = "application/x-fcast-gst-rtp";

pub struct RtpSink {
    src_pad: gst::Pad,
    queue: gst::Element,
    convert: gst::Element,
    scale: gst::Element,
    capsfilter: gst::Element,
    enc: gst::Element,
    enc_caps: gst::Element,
    pay: gst::Element,
    queue2: gst::Element,
    sink: gst::Element,
    port: u16,
    receiver_addr: IpAddr,
}

impl RtpSink {
    pub fn new(
        pipeline: &gst::Pipeline,
        src_pad: gst::Pad,
        port: u16,
        receiver_addr: IpAddr,
    ) -> Result<Self> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
            .property("max-size-buffers", 1u32)
            .property_from_str("leaky", "downstream")
            .build()?;
        let convert = gst::ElementFactory::make("videoconvert")
            .name("sink_convert")
            .build()?;

        let scale = gst::ElementFactory::make("videoscale")
            .name("sink_scale")
            .build()?;
        let capsfilter = gst::ElementFactory::make("capsfilter")
            .name("sink_capsfilter")
            .property(
                "caps",
                gst::Caps::from_str("video/x-raw,width=(int)[16,8192,2],height=(int)[16,8192,2]")?,
            )
            .build()?;

        // TODO: issues with high startup time and "decreasing pts"
        let enc = gst::ElementFactory::make("x264enc")
            .property_from_str("tune", "zerolatency")
            .property_from_str("speed-preset", "ultrafast")
            .property("key-int-max", 125u32)
            .property("b-adapt", false)
            .build()?;
        let enc_caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-h264")
                    .field("profile", "baseline")
                    .build(),
            )
            .build()?;
        let pay = gst::ElementFactory::make("rtph264pay")
            .property("config-interval", -1i32)
            .property("pt", 96u32)
            .build()?;
        let queue2 = gst::ElementFactory::make("queue")
            .name("sink_queue2")
            .property("silent", true)
            .property_from_str("leaky", "downstream")
            .build()?;

        let sink = gst::ElementFactory::make("udpsink")
            .property("host", receiver_addr.to_string())
            .property("port", port as i32)
            .property("sync", true)
            .build()?;

        let elems = [
            &queue,
            &convert,
            &scale,
            &capsfilter,
            &enc,
            &enc_caps,
            &queue2,
            &pay,
            &sink,
        ];

        pipeline.add_many(elems)?;
        gst::Element::link_many(elems)?;

        let src_pad_block = super::block_downstream(&src_pad)?;
        let queue_sink_pad = queue
            .static_pad("sink")
            .ok_or(anyhow::anyhow!("Failed to get static sink pad from queue"))?;
        src_pad.link(&queue_sink_pad)?;
        src_pad.remove_probe(src_pad_block);

        for elem in elems {
            elem.sync_state_with_parent()?;
        }

        Ok(Self {
            src_pad,
            queue,
            convert,
            enc,
            pay,
            queue2,
            sink,
            scale,
            enc_caps,
            capsfilter,
            port,
            receiver_addr,
        })
    }
}

#[async_trait::async_trait]
impl TransmissionSink for RtpSink {
    fn get_play_msg(&self, _addr: IpAddr) -> Option<PlayMessage> {
        Some(PlayMessage {
            container: FCAST_GST_WEBRTC_MIME_TYPE.to_owned(),
            #[cfg(not(target_os = "android"))]
            url: Some(format!(
                "rtp://{}:{}?\
                    media=video\
                    &clock-rate=90000\
                    &encoding-name=H264\
                    &payload=96\
                    &rtp-profile=1",
                super::addr_to_url_string(self.receiver_addr),
                self.port,
            )),
            #[cfg(target_os = "android")]
            url: Some(format!(
                "rtp://127.0.0.1:{}\
                    ?media=video\
                    &clock-rate=90000\
                    &encoding-name=H264\
                    &payload=96\
                    &rtp-profile=1",
                self.port,
            )),
            content: None,
            time: Some(0.0),
            speed: Some(1.0),
            headers: None,
        })
    }

    async fn playing(&mut self) -> Result<()> {
        Ok(())
    }

    fn shutdown(&mut self) {}

    fn unlink(&mut self, pipeline: &gst::Pipeline) -> Result<(), glib::error::BoolError> {
        let block = super::block_downstream(&self.src_pad)?;
        let queue_sink_pad = self.queue.static_pad("sink").ok_or(glib::bool_error!(
            "Failed to get static sink pad from queue"
        ))?;
        self.src_pad.unlink(&queue_sink_pad)?;
        self.src_pad.remove_probe(block);

        let elems = [
            &self.queue,
            &self.convert,
            &self.scale,
            &self.capsfilter,
            &self.enc,
            &self.enc_caps,
            &self.queue2,
            &self.pay,
            &self.sink,
        ];

        pipeline.remove_many(elems)?;

        for elem in elems {
            elem.set_state(gst::State::Null)
                .map_err(|err| glib::bool_error!("{err}"))?;
        }

        Ok(())
    }
}
