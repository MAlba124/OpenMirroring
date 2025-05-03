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

use anyhow::Result;
use common::video::opengl::SlintOpenGLSink;
use common::video::GstGlContext;
use log::{debug, error, trace};
use sender::discovery::discover;
use sender::session::session;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::{self, oneshot};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use sender::{pipeline, Event, SessionMessage};

slint::include_modules!();

struct Application {
    pipeline: pipeline::Pipeline,
    ui_weak: slint::Weak<MainWindow>,
    event_tx: Sender<Event>,
    session_tx: Sender<SessionMessage>,
    select_source_tx: Sender<usize>,
    selected_source: bool,
    receivers: HashMap<String, Vec<SocketAddr>>,
    appsink: gst::Element,
    gst_gl_context: GstGlContext,
}

impl Application {
    pub async fn new(
        ui_weak: slint::Weak<MainWindow>,
        event_tx: Sender<Event>,
        session_tx: Sender<SessionMessage>,
        appsink: gst::Element,
        gst_gl_context: GstGlContext,
    ) -> Result<Self> {
        let (select_source_tx, pipeline) = Self::new_pipeline(
            &ui_weak,
            event_tx.clone(),
            appsink.clone(),
            Arc::clone(&gst_gl_context),
        )
        .await?;

        Ok(Self {
            pipeline,
            ui_weak,
            event_tx,
            session_tx,
            select_source_tx,
            selected_source: false,
            receivers: HashMap::new(),
            appsink,
            gst_gl_context,
        })
    }

    async fn new_pipeline(
        ui_weak: &slint::Weak<MainWindow>,
        event_tx: Sender<Event>,
        appsink: gst::Element,
        gst_gl_context: GstGlContext,
    ) -> Result<(Sender<usize>, pipeline::Pipeline)> {
        let (selected_tx, selected_rx) = sync::mpsc::channel::<usize>(1);

        let (pipeline_tx, pipeline_rx) = oneshot::channel();
        ui_weak.upgrade_in_event_loop({
            move |_ui| {
                let pipeline = {
                    pipeline::Pipeline::new(event_tx, selected_rx, appsink, gst_gl_context).unwrap()
                };
                if pipeline_tx.send(pipeline).is_err() {
                    panic!("Failed to send pipeline");
                }
            }
        })?;

        Ok((selected_tx, pipeline_rx.await?))
    }

    pub async fn run_event_loop(
        &mut self,
        mut event_rx: Receiver<Event>,
        fin_tx: oneshot::Sender<()>,
    ) -> Result<()> {
        let mut receiver_connected_to = String::new();
        let mut receiver_connecting_to = String::new();
        while let Some(event) = event_rx.recv().await {
            match event {
                Event::Quit => break,
                Event::Start => {
                    let Some(play_msg) = self.pipeline.get_play_msg().await else {
                        error!("Could not get stream uri");
                        continue;
                    };

                    self.session_tx.send(play_msg).await?;
                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.set_starting_cast(false);
                        ui.set_casting(true);
                    })?;
                }
                Event::Stop => {
                    self.session_tx.send(SessionMessage::Stop).await?;
                }
                Event::Sources(sources) => {
                    debug!("Available sources: {sources:?}");
                    if sources.len() == 1 {
                        debug!("One source available, auto selecting");
                        self.event_tx.send(Event::SelectSource(0)).await?;
                    } else {
                        self.ui_weak.upgrade_in_event_loop(move |ui| {
                            let model = Rc::new(slint::VecModel::<slint::SharedString>::from(
                                sources
                                    .iter()
                                    .map(|s| s.into())
                                    .collect::<Vec<slint::SharedString>>(),
                            ));
                            ui.set_sources_model(model.into());
                        })?;
                    }
                }
                Event::SelectSource(idx) => {
                    self.select_source_tx.send(idx).await?;
                    self.selected_source = true;

                    // If we're connected to a receiver, add the sink for it
                    if receiver_connected_to.starts_with("OpenMirroring") {
                        self.pipeline.add_webrtc_sink().await?;
                    } else if !receiver_connected_to.is_empty() {
                        self.pipeline.add_hls_sink().await?;
                    }

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.set_has_source(true);
                    })?;
                }
                Event::Packet(packet) => {
                    trace!("Unhandled packet: {packet:?}");
                }
                Event::ReceiverAvailable(receiver) => {
                    if self
                        .receivers
                        .insert(receiver.name.clone(), receiver.addresses)
                        .is_some()
                    {
                        debug!("Receiver dup {}", receiver.name);
                    }

                    // TODO: Do something about this receivers situation
                    let mut receivers_vec = self.receivers.keys().cloned().collect::<Vec<String>>();
                    receivers_vec.sort();
                    let receiver_connected_to = receiver_connected_to.clone();
                    let receiver_connecting_to = receiver_connecting_to.clone();
                    self.ui_weak.upgrade_in_event_loop(move |ui| {
                        let receiver_connected_to = &receiver_connected_to;
                        let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                            receivers_vec.iter().map(|name| ReceiverItem {
                                name: name.into(),
                                connected: name == receiver_connected_to,
                                connecting: *name == receiver_connecting_to,
                            }),
                        ));
                        ui.set_receivers_model(model.into());
                    })?;
                }
                Event::SelectReceiver(receiver) => {
                    let Some(addresses) = self.receivers.get(&receiver) else {
                        error!("No receiver with id {receiver}");
                        continue;
                    };

                    receiver_connecting_to = receiver;

                    self.session_tx
                        .send(SessionMessage::Connect(addresses[0]))
                        .await?;

                    let mut receivers_vec = self.receivers.keys().cloned().collect::<Vec<String>>();
                    receivers_vec.sort();
                    let receiver_connected_to = receiver_connected_to.clone();
                    let receiver_connecting_to = receiver_connecting_to.clone();
                    self.ui_weak.upgrade_in_event_loop(move |ui| {
                        let receiver_connected_to = &receiver_connected_to;
                        let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                            receivers_vec.iter().map(|name| ReceiverItem {
                                name: name.into(),
                                connected: name == receiver_connected_to,
                                connecting: *name == receiver_connecting_to,
                            }),
                        ));
                        ui.set_receiver_is_connecting(true);
                        ui.set_receiver_is_connected(false);
                        ui.set_receivers_model(model.into());
                    })?;
                }
                Event::ConnectedToReceiver => {
                    debug!("Succesfully connected to receiver");
                    receiver_connected_to = receiver_connecting_to;
                    receiver_connecting_to = String::new();

                    if receiver_connected_to.starts_with("OpenMirroring") {
                        self.pipeline.add_webrtc_sink().await?;
                    } else {
                        self.pipeline.add_hls_sink().await?;
                    }

                    let mut receivers_vec = self.receivers.keys().cloned().collect::<Vec<String>>();
                    receivers_vec.sort();
                    let receiver_connected_to = receiver_connected_to.clone();
                    let receiver_connecting_to = receiver_connecting_to.clone();
                    self.ui_weak.upgrade_in_event_loop(move |ui| {
                        let receiver_connected_to = &receiver_connected_to;
                        let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                            receivers_vec.iter().map(|name| ReceiverItem {
                                name: name.into(),
                                connected: name == receiver_connected_to,
                                connecting: *name == receiver_connecting_to,
                            }),
                        ));
                        ui.set_receiver_is_connecting(false);
                        ui.set_receiver_is_connected(true);
                        ui.set_receivers_model(model.into());
                    })?;
                }
                Event::DisconnectReceiver => {
                    self.session_tx.send(SessionMessage::Disconnect).await?;
                    self.pipeline.remove_transmission_sink().await?;
                    receiver_connected_to.clear();
                    let mut receivers_vec = self.receivers.keys().cloned().collect::<Vec<String>>();
                    receivers_vec.sort();
                    self.ui_weak.upgrade_in_event_loop(move |ui| {
                        let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                            receivers_vec.iter().map(|name| ReceiverItem {
                                name: name.into(),
                                connected: false,
                                connecting: false,
                            }),
                        ));
                        ui.set_receiver_is_connecting(false);
                        ui.set_receiver_is_connected(false);
                        ui.set_receivers_model(model.into());
                    })?;
                }
                Event::ChangeSource | Event::PipelineFinished => {
                    self.shutdown_pipeline_and_create_new_and_update_ui()
                        .await?;
                }
            }
        }

        debug!("Quitting");

        if !self.selected_source {
            debug!("Source is not selected, sending fake");
            self.select_source_tx.send(0).await?;
        }

        self.pipeline.shutdown().await?;

        fin_tx.send(()).unwrap();

        Ok(())
    }

    async fn shutdown_pipeline_and_create_new_and_update_ui(&mut self) -> Result<()> {
        if !self.selected_source {
            self.select_source_tx.send(0).await?;
        }

        self.pipeline.shutdown().await?;

        let (new_select_srouce_tx, new_pipeline) = Self::new_pipeline(
            &self.ui_weak,
            self.event_tx.clone(),
            self.appsink.clone(),
            Arc::clone(&self.gst_gl_context),
        )
        .await?;

        self.pipeline = new_pipeline;
        self.select_source_tx = new_select_srouce_tx;
        self.selected_source = false;

        self.ui_weak.upgrade_in_event_loop(|ui| {
            ui.set_has_source(false);
            ui.set_sources_model(Rc::new(slint::VecModel::<slint::SharedString>::default()).into());
        })?;

        Ok(())
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_module("sender", common::default_log_level())
        .filter_module("scap", common::default_log_level())
        .init();

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .require_opengl()
        .select()?;

    gst::init()?;
    scapgst::plugin_register_static()?;
    gst_webrtc::plugin_register_static()?;

    let (event_tx, event_rx) = sync::mpsc::channel::<Event>(100);
    let (session_tx, session_rx) = sync::mpsc::channel::<SessionMessage>(100);
    let (fin_tx, fin_rx) = oneshot::channel::<()>();

    // This sink is used in every consecutively created pipelines
    let mut slint_sink = SlintOpenGLSink::new()?;
    let slint_appsink = slint_sink.video_sink();
    let gst_gl_context = Arc::new(Mutex::new(None::<(gst_gl::GLContext, gst_gl::GLDisplay)>));

    let ui = MainWindow::new()?;
    slint::set_xdg_app_id("com.github.malba124.OpenMirroring.sender")?;

    ui.window().set_rendering_notifier({
        let gst_gl_context = Arc::clone(&gst_gl_context);
        let ui_weak = ui.as_weak();

        let new_frame_cb = |ui: MainWindow, new_frame| {
            ui.set_preview_frame(new_frame);
        };

        move |state, graphics_api| match state {
            slint::RenderingState::RenderingSetup => {
                let ui_weak = ui_weak.clone();
                let mut gst_gl_context = gst_gl_context.lock().unwrap();
                *gst_gl_context = Some(
                    slint_sink
                        .connect(
                            graphics_api,
                            Box::new(move || {
                                ui_weak
                                    .upgrade_in_event_loop(move |ui| {
                                        ui.window().request_redraw();
                                    })
                                    .ok();
                            }),
                        )
                        .unwrap(),
                );
            }
            slint::RenderingState::BeforeRendering => {
                if let Some(next_frame) = slint_sink.fetch_next_frame() {
                    new_frame_cb(ui_weak.unwrap(), next_frame);
                }
            }
            slint::RenderingState::RenderingTeardown => {
                slint_sink.deactivate_and_pause();
            }
            _ => (),
        }
    })?;

    common::runtime().spawn(session(session_rx, event_tx.clone()));
    common::runtime().spawn(discover(event_tx.clone()));
    common::runtime().spawn({
        let ui_weak = ui.as_weak();
        let event_tx = event_tx.clone();
        let session_tx = session_tx.clone();
        async move {
            let mut app =
                Application::new(ui_weak, event_tx, session_tx, slint_appsink, gst_gl_context)
                    .await
                    .unwrap();
            app.run_event_loop(event_rx, fin_tx).await.unwrap();
        }
    });

    {
        let event_tx = event_tx.clone();
        ui.on_select_source(move |idx| {
            assert!(idx >= 0, "Invalid select source index");
            event_tx
                .blocking_send(Event::SelectSource(idx as usize))
                .unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_connect_receiver(move |receiver| {
            event_tx
                .blocking_send(Event::SelectReceiver(receiver.to_string()))
                .unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        let ui_weak = ui.as_weak();
        ui.on_start_cast(move || {
            event_tx.blocking_send(Event::Start).unwrap();
            ui_weak
                .upgrade_in_event_loop(|ui| {
                    ui.set_starting_cast(true);
                })
                .unwrap();
        });
    }

    {
        let ui_weak = ui.as_weak();
        let event_tx = event_tx.clone();
        ui.on_stop_cast(move || {
            event_tx.blocking_send(Event::Stop).unwrap();
            ui_weak
                .upgrade_in_event_loop(|ui| {
                    ui.set_starting_cast(false);
                    ui.set_casting(false);
                })
                .unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_disconnect_receiver(move || {
            event_tx.blocking_send(Event::DisconnectReceiver).unwrap();
        });
    }

    {
        ui.on_change_source(move || {
            event_tx.blocking_send(Event::ChangeSource).unwrap();
        });
    }

    ui.run()?;

    common::runtime().block_on(async move {
        if !session_tx.is_closed() {
            session_tx.send(SessionMessage::Quit).await.unwrap();
            fin_rx.await.unwrap();
        }
    });

    Ok(())
}
