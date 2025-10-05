// Copyright (C) 2025 Marcus L. Hanestad <marlhan@proton.me>
//
// This file is part of OpenMirroring.
//
// OpenMirroring is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// OpenMirroring is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with OpenMirroring.  If not, see <https://www.gnu.org/licenses/>.

use super::transmission::TransmissionSink;
use anyhow::Result;
use fcast_protocol::v2::PlayMessage;
#[cfg(target_os = "android")]
use futures::StreamExt;
use gst::glib;
#[cfg(target_os = "android")]
use std::future::Future;
use std::net::IpAddr;

pub enum Event {
    PipelineIsPlaying,
    Eos,
    Error,
}

#[derive(Clone, Debug)]
pub enum AudioSource {
    #[cfg(target_os = "linux")]
    Pipewire { name: String, id: u32 },
}

impl AudioSource {
    pub fn display_name(&self) -> String {
        #[cfg(target_os = "linux")]
        match self {
            AudioSource::Pipewire { name, .. } => name.clone(),
        }
        #[cfg(target_os = "macos")]
        "n/a".to_string()
    }
}

#[derive(Clone, Debug)]
pub enum VideoSource {
    #[cfg(target_os = "linux")]
    PipeWire {
        node_id: u32,
        #[allow(dead_code)]
        fd: i32, // TODO: does not work, why not?
    },
    #[cfg(target_os = "linux")]
    XWindow { id: u32, name: String },
    #[cfg(target_os = "linux")]
    XDisplay {
        id: u32,
        width: u16,
        height: u16,
        x_offset: i16,
        y_offset: i16,
        name: String,
    },
    #[cfg(target_os = "macos")]
    DefaultAvf,
}

impl VideoSource {
    pub fn display_name(&self) -> String {
        match self {
            #[cfg(target_os = "linux")]
            VideoSource::PipeWire { .. } => "PipeWire Video Source".to_owned(),
            #[cfg(target_os = "linux")]
            VideoSource::XWindow { name, .. } => name.clone(),
            #[cfg(target_os = "linux")]
            VideoSource::XDisplay { name, .. } => name.clone(),
            #[cfg(target_os = "macos")]
            VideoSource::DefaultAvf => "Default".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum SourceConfig {
    AudioVideo {
        video: VideoSource,
        audio: AudioSource,
    },
    Video(VideoSource),
    Audio(AudioSource),
}

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "SourceConfigBoxed")]
pub struct SourceConfigBoxed(pub SourceConfig);

pub struct Pipeline {
    tx_sink: Box<dyn TransmissionSink>,
}

impl Pipeline {
    // #[cfg(target_os = "android")]
    // pub async fn new<E, Fut>(
    //     frame_rx: crossbeam_channel::Receiver<
    //         gst_video::VideoFrame<gst_video::video_frame::Writable>,
    //     >,
    //     mut on_event: E,
    // ) -> Result<Self>
    // where
    //     E: FnMut(Event) -> Fut + Send + Clone + 'static,
    //     Fut: Future<Output = ()> + Send + 'static,
    // {
    //     let appsrc = gst_app::AppSrc::builder()
    //         .caps(
    //             &gst_video::VideoCapsBuilder::new()
    //                 .format(gst_video::VideoFormat::Rgba)
    //                 .build(),
    //         )
    //         .is_live(true)
    //         .do_timestamp(true)
    //         .format(gst::Format::Time)
    //         .max_buffers(1)
    //         .build();

    //     let mut caps = None::<gst::Caps>;
    //     appsrc.set_callbacks(
    //         gst_app::AppSrcCallbacks::builder()
    //             .need_data(move |appsrc, _| {
    //                 // let frame = match crate::FRAME_CHAN.1.recv() {
    //                 let frame = match frame_rx.recv() {
    //                     Ok(frame) => frame,
    //                     Err(err) => {
    //                         error!("Failed to receive frame: {err}");
    //                         let _ = appsrc.end_of_stream();
    //                         return;
    //                     }
    //                 };

    //                 use gst_video::prelude::*;

    //                 let now_caps = gst_video::VideoInfo::builder(
    //                     frame.format(),
    //                     frame.width(),
    //                     frame.height(),
    //                 )
    //                 .build()
    //                 .unwrap()
    //                 .to_caps()
    //                 .unwrap();

    //                 match &caps {
    //                     Some(old_caps) => {
    //                         if *old_caps != now_caps {
    //                             appsrc.set_caps(Some(&now_caps));
    //                             caps = Some(now_caps);
    //                         }
    //                     }
    //                     None => {
    //                         appsrc.set_caps(Some(&now_caps));
    //                         caps = Some(now_caps);
    //                     }
    //                 }

    //                 let _ = appsrc.push_buffer(frame.into_buffer());
    //             })
    //             .build(),
    //     );

    //     let pipeline = gst::Pipeline::new();

    //     pipeline.add_many(&[&appsrc])?;

    //     let bus = pipeline
    //         .bus()
    //         .ok_or(anyhow::anyhow!("Pipeline is missing bus"))?;

    //     let pipeline_weak = pipeline.downgrade();
    //     tokio::spawn(async move {
    //         let mut messages = bus.stream();

    //         while let Some(msg) = messages.next().await {
    //             use gst::MessageView;

    //             match msg.view() {
    //                 MessageView::Eos(..) => (on_event)(Event::Eos).await,
    //                 MessageView::Error(err) => {
    //                     error!(
    //                         "Error from {:?}: {} ({:?})",
    //                         err.src().map(|s| s.path_string()),
    //                         err.error(),
    //                         err.debug()
    //                     );
    //                     (on_event)(Event::Error).await;
    //                 }
    //                 MessageView::StateChanged(state_changed) => {
    //                     let Some(pipeline) = pipeline_weak.upgrade() else {
    //                         return;
    //                     };

    //                     if state_changed.src() == Some(pipeline.upcast_ref())
    //                         && state_changed.old() == gst::State::Paused
    //                         && state_changed.current() == gst::State::Playing
    //                     {
    //                         (on_event)(Event::PipelineIsPlaying).await;
    //                     }
    //                 }
    //                 _ => (),
    //             }
    //         }
    //     });

    //     Ok(Self {
    //         inner: pipeline,
    //         tx_sink: None,
    //         appsrc: appsrc.upcast(),
    //     })
    // }

    pub fn new_rtsp<E>(mut on_event: E, source: SourceConfig) -> Result<Self>
    where
        E: FnMut(Event) + Send + Clone + 'static,
    {
        let sink = crate::sender::transmission::rtsp::RtspSink::new(source, 5554)?;
        let p = Self {
            tx_sink: Box::new(sink),
        };

        Ok(p)
    }

    pub fn playing(&mut self) -> Result<()> {
        self.tx_sink.playing()
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.tx_sink.shutdown();

        Ok(())
    }

    /// Get the message that should be sent to a receiver to consume the stream if a transmission
    /// sink is present
    pub fn get_play_msg(&self, addr: IpAddr) -> Option<PlayMessage> {
        self.tx_sink.get_play_msg(addr)
    }
}
