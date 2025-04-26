use common::video_preview::software::SlintSwSink;
use gst::prelude::*;

use anyhow::{bail, Result};

use log::debug;

pub struct Pipeline {
    inner: gst::Pipeline,
    playbin: gst::Element,
}

impl Pipeline {
    pub fn new(preview: SlintSwSink) -> Result<Self> {
        let pipeline = gst::Pipeline::new();

        let playbin = gst::ElementFactory::make("playbin3").build()?;

        pipeline.add(&playbin)?;

        pipeline.set_state(gst::State::Ready)?;

        let bus = pipeline.bus().expect("Pipeline without bus");

        // TODO: use async handling instead
        bus.set_sync_handler(move |_, msg| {
            use gst::MessageView;

            match msg.view() {
                MessageView::Eos(eos) => debug!("EOS"),
                MessageView::Error(err) => debug!("{err}"),
                _ => (),
            }

            gst::BusSyncReply::Drop
        });

        playbin.set_property("video-sink", &preview.bin);

        Ok(Self {
            inner: pipeline,
            playbin,
        })
    }

    pub fn set_playback_uri(&self, uri: &str) -> Result<()> {
        self.inner.set_state(gst::State::Ready)?;
        self.playbin.set_property("uri", uri);

        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        self.inner.set_state(gst::State::Paused)?;

        Ok(())
    }

    pub fn play_or_resume(&self) -> Result<()> {
        self.inner.set_state(gst::State::Playing)?;

        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.inner.set_state(gst::State::Null)?;
        self.playbin.set_property("uri", "");

        Ok(())
    }

    pub fn set_volume(&self, new_volume: f64) {
        self.playbin
            .set_property("volume", new_volume.clamp(0.0, 1.0));
    }

    pub fn seek(&self, seek_to: f64) -> Result<()> {
        self.inner.seek_simple(
            gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
            gst::ClockTime::from_seconds_f64(seek_to),
        )?;

        Ok(())
    }

    pub fn set_speed(&self, new_speed: f64) -> Result<()> {
        let Some(position) = self.inner.query_position::<gst::ClockTime>() else {
            bail!("Failed to query playback position");
        };

        if new_speed > 0.0 {
            self.inner.seek(
                new_speed,
                gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
                gst::SeekType::Set,
                position,
                gst::SeekType::End,
                gst::ClockTime::ZERO,
            )?;
        } else {
            self.inner.seek(
                new_speed,
                gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
                gst::SeekType::Set,
                gst::ClockTime::ZERO,
                gst::SeekType::End,
                position,
            )?;
        }

        Ok(())
    }
}
