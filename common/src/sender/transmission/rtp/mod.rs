use crate::net::get_default_ipv4_addr;
use std::str::FromStr;

use anyhow::Result;
use gio::glib;
use gst::prelude::{ElementExt, ElementExtManual, GstBinExtManual, PadExt, PadExtManual};

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
    rtpbin: gst::Element,
    sink: gst::Element,
    host: String,
}

impl RtpSink {
    pub fn new(pipeline: &gst::Pipeline, src_pad: gst::Pad) -> Result<Self> {
        let queue = gst::ElementFactory::make("queue")
            .name("sink_queue")
            .property("silent", true)
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

        // TODO: dynamically select the fastest/best(?) codec, HW or SW
        // TODO: these settings give awful quality, fix settings
        let enc = gst::ElementFactory::make("x264enc")
            .property("bframes", 0u32)
            .property("bitrate", 1024 * 6u32)
            .property("key-int-max", 1u32)
            .property_from_str("tune", "zerolatency")
            .property_from_str("speed-preset", "superfast")
            .build()?;
        let enc_caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-h264")
                    .field("profile", "main")
                    .build(),
            )
            .build()?;
        let pay = gst::ElementFactory::make("rtph264pay")
            // TODO: do these even do anything?
            .property("config-interval", -1i32)
            .property_from_str("aggregate-mode", "zero-latency")
            .build()?;

        let queue2 = gst::ElementFactory::make("queue")
            .name("sink_queue2")
            .property("silent", true)
            .build()?;
        let rtpbin = gst::ElementFactory::make("rtpbin").build()?;
        let host = format!("{}", get_default_ipv4_addr());
        let sink = gst::ElementFactory::make("udpsink")
            .property("host", &host)
            .property("port", 5004i32)
            .property("sync", true)
            .build()?;

        pipeline.add_many([
            &queue,
            &convert,
            &scale,
            &capsfilter,
            &enc,
            &enc_caps,
            &queue2,
            &pay,
            &rtpbin,
            &sink,
        ])?;
        gst::Element::link_many([
            &queue,
            &convert,
            &scale,
            &capsfilter,
            &enc,
            &enc_caps,
            &pay,
            &queue2,
        ])?;

        queue.sync_state_with_parent()?;
        convert.sync_state_with_parent()?;
        scale.sync_state_with_parent()?;
        capsfilter.sync_state_with_parent()?;
        enc.sync_state_with_parent()?;
        enc_caps.sync_state_with_parent()?;
        pay.sync_state_with_parent()?;
        queue2.sync_state_with_parent()?;

        {
            // TODO: reusable blocks
            let src_pad_block = src_pad
                .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                    gst::PadProbeReturn::Ok
                })
                .ok_or(anyhow::anyhow!("Failed to add pad probe to src_pad"))?;

            let queue_sink_pad = queue
                .static_pad("sink")
                .ok_or(anyhow::anyhow!("Failed to get static sink pad from queue"))?;
            src_pad.link(&queue_sink_pad)?;

            // TODO: how long does it have to be blocked?
            src_pad.remove_probe(src_pad_block);
        }

        let queue2_src_pad = queue2
            .static_pad("src")
            .ok_or(anyhow::anyhow!("Failed to get static src pad from queue2"))?;
        let sink_pad = rtpbin
            .request_pad_simple("send_rtp_sink_0")
            .ok_or(anyhow::anyhow!(
                "Failed to get send_rtp_sink_0 pad from rtpbin"
            ))?;
        queue2_src_pad.link(&sink_pad)?;

        let rtp_src_pad = rtpbin.static_pad("send_rtp_src_0").ok_or(anyhow::anyhow!(
            "Failed to get static send_rtp_src_0 pad from rtpbin"
        ))?;
        let sink_pad = sink
            .static_pad("sink")
            .ok_or(anyhow::anyhow!("Failed to get static sink pad from sink"))?;
        rtp_src_pad.link(&sink_pad)?;

        rtpbin.sync_state_with_parent()?;
        sink.sync_state_with_parent()?;

        Ok(Self {
            src_pad,
            queue,
            convert,
            enc,
            pay,
            queue2,
            rtpbin,
            sink,
            scale,
            enc_caps,
            capsfilter,
            host,
        })
    }
}

#[async_trait::async_trait]
impl TransmissionSink for RtpSink {
    fn get_play_msg(&self) -> Option<PlayMessage> {
        Some(PlayMessage {
            container: FCAST_GST_WEBRTC_MIME_TYPE.to_owned(),
            #[cfg(not(target_os = "android"))]
            url: Some(format!(
                "rtp://{}:5004?\
                    media=video\
                    &clock-rate=90000\
                    &encoding-name=H264\
                    &payload=96\
                    &rtp-profile=1",
                self.host
            )),
            #[cfg(target_os = "android")]
            url: Some(
                "rtp://127.0.0.1:5004\
                    ?media=video\
                    &clock-rate=90000\
                    &encoding-name=H264\
                    &payload=96\
                    &rtp-profile=1"
                    .to_owned(),
            ),
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
        let block = self
            .src_pad
            .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                gst::PadProbeReturn::Ok
            })
            .unwrap();
        // .ok_or(anyhow::anyhow!("Failed to add pad probe to src_pad"))?;

        let queue_sink_pad = self.queue.static_pad("sink").unwrap();
        // .ok_or(anyhow::anyhow!("Failed to get static sink pad from queue"))?;
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
            &self.rtpbin,
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
