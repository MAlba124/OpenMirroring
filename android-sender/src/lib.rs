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

use std::net::SocketAddr;
use std::rc::Rc;

use common::sender::pipeline;
use common::sender::session::{self, SessionMessage};
use gst_video::VideoFrameExt;
use jni::objects::JObject;
use jni::JavaVM;

use log::debug;
use log::error;

use anyhow::Result;
use log::trace;

mod discovery;

lazy_static::lazy_static! {
    pub static ref EVENT_CHAN: (async_channel::Sender<Event>, async_channel::Receiver<Event>) =
        async_channel::unbounded();

    pub static ref FRAME_CHAN: (
        crossbeam_channel::Sender<gst_video::VideoFrame<gst_video::video_frame::Writable>>,
        crossbeam_channel::Receiver<gst_video::VideoFrame<gst_video::video_frame::Writable>>
    ) = crossbeam_channel::bounded(2);
}

#[macro_export]
macro_rules! tx {
    () => {
        crate::EVENT_CHAN.0
    };
}

macro_rules! rx {
    () => {
        crate::EVENT_CHAN.1
    };
}

slint::include_modules!();

pub enum Event {
    CaptureStarted,
    CaptureStopped,
    ReceiverAvailable { name: String, addr: SocketAddr },
    Packet(fcast_lib::packet::Packet),
    ConnectedToReceiver,
    SessionTerminated,
    PipelineIsPlaying,
    SelectReceiver(String),
    DisconnectReceiver,
    StartCast,
    StopCast,
}

struct Application {
    ui_weak: slint::Weak<MainWindow>,
    session_tx: tokio::sync::mpsc::Sender<SessionMessage>,
    receivers: Vec<(ReceiverItem, SocketAddr)>,
}

impl Application {
    pub fn new(
        ui_weak: slint::Weak<MainWindow>,
        session_tx: tokio::sync::mpsc::Sender<SessionMessage>,
    ) -> Self {
        Self {
            ui_weak,
            session_tx,
            receivers: Vec::new(),
        }
    }

    fn receivers_contains(&self, x: &str) -> Option<usize> {
        for (idx, y) in self.receivers.iter().enumerate() {
            if y.0.name == x {
                return Some(idx);
            }
        }

        None
    }

    /// Returns the state for all non connecting/connected receivers.
    fn receivers_general_state(&self) -> ReceiverState {
        for r in &self.receivers {
            if r.0.state != ReceiverState::Connectable {
                return ReceiverState::Inactive;
            }
        }

        ReceiverState::Connectable
    }

    fn update_receivers_in_ui(&mut self) -> Result<()> {
        let g_state = self.receivers_general_state();
        for r in &mut self.receivers {
            if r.0.state != ReceiverState::Connecting && r.0.state != ReceiverState::Connected {
                r.0.state = g_state;
            }
        }

        let receivers = self
            .receivers
            .iter()
            .map(|r| r.0.clone())
            .collect::<Vec<ReceiverItem>>();
        self.ui_weak.upgrade_in_event_loop(move |ui| {
            let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                receivers.into_iter(),
            ));
            ui.set_receivers_model(model.into());
        })?;

        Ok(())
    }

    pub async fn run_event_loop(mut self) -> Result<()> {
        let mut pipeline = pipeline::Pipeline::new(FRAME_CHAN.1.clone(), async move |event| {
            match event {
                pipeline::Event::PipelineIsPlaying => {
                    tx!().send(Event::PipelineIsPlaying).await.unwrap();
                }
                pipeline::Event::Eos => (),   // TODO
                pipeline::Event::Error => (), // TODO
            }
        })
        .await?;

        // NOTE: untested because I can't get emu to work
        pipeline.add_rtp_sink()?;

        while let Ok(event) = rx!().recv().await {
            match event {
                Event::CaptureStarted => {
                    self.ui_weak.upgrade_in_event_loop(move |handle| {
                        handle.invoke_screen_capture_started();
                    })?;
                }
                Event::CaptureStopped => {
                    self.ui_weak.upgrade_in_event_loop(move |handle| {
                        handle.invoke_screen_capture_stopped();
                    })?;
                }
                Event::ReceiverAvailable { name, addr } => {
                    if let Some(idx) = self.receivers_contains(&name) {
                        self.receivers[idx].1 = addr;
                    } else {
                        self.receivers.push((
                            ReceiverItem {
                                name: name.into(),
                                state: self.receivers_general_state(),
                            },
                            addr,
                        ));
                    }

                    self.update_receivers_in_ui()?;
                }
                Event::Packet(packet) => trace!("{packet:?}"),
                Event::ConnectedToReceiver => {
                    debug!("Succesfully connected to receiver");

                    for r in &mut self.receivers {
                        if r.0.state == ReceiverState::Connecting {
                            r.0.state = ReceiverState::Connected;
                            break;
                        }
                    }

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_receiver_connected();
                    })?;

                    self.update_receivers_in_ui()?;
                }
                Event::SessionTerminated => break,
                Event::PipelineIsPlaying => pipeline.playing().await?,
                Event::SelectReceiver(receiver) => {
                    if let Some(idx) = self.receivers_contains(&receiver) {
                        self.receivers[idx].0.state = ReceiverState::Connecting;

                        self.session_tx
                            .send(SessionMessage::Connect(self.receivers[idx].1))
                            .await?;

                        self.update_receivers_in_ui()?;
                    } else {
                        error!("No receiver `{receiver}` available");
                    }
                }
                Event::DisconnectReceiver => {
                    for r in &mut self.receivers {
                        if r.0.state == ReceiverState::Connected
                            || r.0.state == ReceiverState::Connecting
                        {
                            r.0.state = ReceiverState::Connectable;
                            self.update_receivers_in_ui()?;
                            break;
                        }
                    }

                    self.session_tx.send(SessionMessage::Disconnect).await?;

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_receiver_disconnected();
                    })?;
                }
                Event::StartCast => {
                    let Some(play_msg) = pipeline.get_play_msg() else {
                        error!("Pipeline could not provide play message");
                        // TODO: tell UI
                        continue;
                    };

                    self.session_tx
                        .send(SessionMessage::Play {
                            mime: play_msg.mime,
                            uri: play_msg.uri,
                        })
                        .await?;

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_cast_started();
                    })?;
                }
                Event::StopCast => {
                    self.session_tx.send(SessionMessage::Stop).await?;

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_cast_stopped();
                    })?;
                }
            }
        }

        debug!("Quitting");

        Ok(())
    }
}

#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(common::default_log_level()),
    );

    debug!("Hello from rust");

    #[cfg(debug_assertions)]
    {
        std::env::set_var("GST_DEBUG_NO_COLOR", "true");
        std::env::set_var("GST_DEBUG", "3");
    }

    gst::init().unwrap();
    common::sender::pipeline::init().unwrap();

    // let payloaders = gst::ElementFactory::factories_with_type(gst::ElementFactoryType::PAYLOADER, gst::Rank::MARGINAL,); for p in payloaders {debug!("{p:?}");}

    debug!("GStreamer version: {:?}", gst::version());

    let app_clone = app.clone();

    slint::android::init(app).unwrap();

    let ui = MainWindow::new().unwrap();

    ui.on_request_start_capture(move || {
        debug!("request_start_capture was called");

        let vm = unsafe {
            let ptr = app_clone.vm_as_ptr() as *mut jni::sys::JavaVM;
            assert!(!ptr.is_null(), "JavaVM ptr is null");
            JavaVM::from_raw(ptr).unwrap()
        };
        let activity = unsafe {
            let ptr = app_clone.activity_as_ptr() as *mut jni::sys::_jobject;
            assert!(!ptr.is_null(), "Activity ptr is null");
            JObject::from_raw(ptr)
        };

        match vm.get_env() {
            Ok(mut env) => match env.call_method(activity, "startScreenCapture", "()V", &[]) {
                Ok(_) => (),
                Err(err) => error!("Failed to call `startScreenCapture`: {err}"),
            },
            Err(err) => error!("Failed to get env from VM: {err}"),
        }
    });

    ui.on_connect_receiver(|receiver| {
        tx!()
            .send_blocking(Event::SelectReceiver(receiver.to_string()))
            .unwrap();
    });

    ui.on_disconnect_receiver(|| {
        tx!().send_blocking(Event::DisconnectReceiver).unwrap();
    });

    ui.on_start_cast(|| {
        tx!().send_blocking(Event::StartCast).unwrap();
    });

    ui.on_stop_cast(|| {
        tx!().send_blocking(Event::StopCast).unwrap();
    });

    let ui_weak = ui.as_weak();

    let (session_tx, session_rx) = tokio::sync::mpsc::channel(10);

    common::runtime().spawn(Application::new(ui_weak, session_tx).run_event_loop());

    common::runtime().spawn(discovery::discover());

    common::runtime().spawn(session::session(session_rx, async move |event| {
        match event {
            session::Event::SessionTerminated => {
                tx!().send(Event::SessionTerminated).await.unwrap();
            }
            session::Event::FcastPacket(packet) => {
                tx!().send(Event::Packet(packet)).await.unwrap();
            }
            session::Event::ConnectedToReceiver => {
                tx!().send(Event::ConnectedToReceiver).await.unwrap();
            }
        }
    }));

    ui.run().unwrap();
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn Java_com_github_malba124_openmirroring_android_sender_ScreenCaptureService_nativeCaptureStarted<
    'local,
>(
    _env: jni::JNIEnv<'local>,
    _class: jni::objects::JClass<'local>,
) {
    debug!("Screen capture was started");
    tx!().send_blocking(Event::CaptureStarted).unwrap();
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn Java_com_github_malba124_openmirroring_android_sender_ScreenCaptureService_nativeCaptureStopped<
    'local,
>(
    _env: jni::JNIEnv<'local>,
    _class: jni::objects::JClass<'local>,
) {
    debug!("Screen capture was stopped");
    tx!().send_blocking(Event::CaptureStopped).unwrap();
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn Java_com_github_malba124_openmirroring_android_sender_ScreenCaptureService_nativeProcessFrame<
    'local,
>(
    env: jni::JNIEnv<'local>,
    _class: jni::objects::JClass<'local>,
    buffer: jni::objects::JByteBuffer<'local>,
    width: jni::sys::jint,
    height: jni::sys::jint,
    pixel_stride: jni::sys::jint,
    _row_stride: jni::sys::jint,
) {
    if FRAME_CHAN.0.is_full() {
        return;
    }

    let width = width as usize;
    let height = height as usize;
    let pixel_stride = pixel_stride as usize;
    let frame_size = width * height * pixel_stride;

    let buffer_cap = match env.get_direct_buffer_capacity(&buffer) {
        Ok(cap) => cap,
        Err(err) => {
            error!("Failed to get capacity of the byte buffer: {err}");
            return;
        }
    };

    if buffer_cap < frame_size {
        error!("buffer_cap < frame_size: {buffer_cap} < {frame_size}");
        return;
    }

    let buffer_ptr = match env.get_direct_buffer_address(&buffer) {
        Ok(ptr) => {
            assert!(!ptr.is_null());
            ptr
        }
        Err(err) => {
            error!("Failed to get buffer address: {err}");
            return;
        }
    };

    let buffer_slice: &[u8] = unsafe { std::slice::from_raw_parts(buffer_ptr, buffer_cap) };

    let buffer = gst::Buffer::with_size(frame_size).unwrap();

    let info = match gst_video::VideoInfo::builder(
        gst_video::VideoFormat::Rgba,
        width as u32,
        height as u32,
    )
    .build()
    {
        Ok(info) => info,
        Err(err) => {
            error!("Failed to crate video info: {err}");
            return;
        }
    };

    let Ok(mut vframe) = gst_video::VideoFrame::from_buffer_writable(buffer, &info) else {
        error!("Failed to crate VideoFrame from buffer");
        return;
    };

    let dest_stride = vframe.plane_stride()[0] as usize;
    let dest = vframe.plane_data_mut(0).unwrap();

    for (dest, src) in dest
        .chunks_exact_mut(dest_stride)
        .zip(buffer_slice.chunks_exact(dest_stride))
    {
        dest[..dest_stride].copy_from_slice(&src[..dest_stride]);
    }

    if let Err(err) = FRAME_CHAN.0.send(vframe) {
        error!("Failed to send frame: {err}");
    }
}
