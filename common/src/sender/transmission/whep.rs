use gst::prelude::*;

use crate::sender::pipeline::SourceConfig;

use super::TransmissionSink;

pub struct WhepSink {
}

impl WhepSink {
    pub fn new(pipeline: &gst::Pipeline, source_config: SourceConfig) -> anyhow::Result<Self> {
        let sink = gst::ElementFactory::make("whepserversink")
            .build()?;

        pipeline.add(&sink)?;

        match source_config {
            SourceConfig::AudioVideo { video, audio } => {
                video.link(&sink)?;
                audio.link(&sink)?;
            }
            SourceConfig::Video(video) => video.link(&sink)?,
            SourceConfig::Audio(audio) => audio.link(&sink)?,
        }

        Ok(Self {})
    }
}

impl TransmissionSink for WhepSink {
    fn get_play_msg(&self, addr: std::net::IpAddr) -> Option<fcast_lib::models::PlayMessage> {
        Some(fcast_lib::models::PlayMessage {
            container: "application/x-whep".to_owned(),
            // url: Some(format!("http://127.0.0.1:9090/whep/endpoint",)),
            // url: Some(format!("http://192.168.1.9:9090/whep/endpoint",)),
            url: Some(format!("http://[::ffff:192.168.1.9]:9090/whep/endpoint",)),
            content: None,
            time: None,
            speed: None,
            headers: None,
        })
    }

    fn playing(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn shutdown(&mut self) {}
}
