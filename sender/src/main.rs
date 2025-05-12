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
use common::sender::session::{self, SessionMessage};
use common::video::opengl::SlintOpenGLSink;
use log::{debug, error, trace};
use sender::discovery::discover;
use simple_mdns::async_discovery::ServiceDiscovery;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::{self, oneshot};

use std::net::SocketAddr;
use std::rc::Rc;

use common::sender::pipeline;
use sender::Event;

slint::include_modules!();

struct Application {
    pipeline: pipeline::Pipeline,
    ui_weak: slint::Weak<MainWindow>,
    event_tx: Sender<Event>,
    session_tx: Sender<SessionMessage>,
    select_source_tx: Sender<usize>,
    selected_source: bool,
    receivers: Vec<(ReceiverItem, SocketAddr)>,
    appsink: gst::Element,
    _discovery: ServiceDiscovery,
}

impl Application {
    pub async fn new(
        ui_weak: slint::Weak<MainWindow>,
        event_tx: Sender<Event>,
        session_tx: Sender<SessionMessage>,
        appsink: gst::Element,
        gotten_gl: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<Self> {
        // We need to wait until the preview sink has gotten it's required GL contexts,
        // if not the pipeline would fail to init
        gotten_gl.await?;

        let (select_source_tx, pipeline) =
            Self::new_pipeline(event_tx.clone(), appsink.clone()).await?;

        let discovery = discover(event_tx.clone())?;

        Ok(Self {
            pipeline,
            ui_weak,
            event_tx,
            session_tx,
            select_source_tx,
            selected_source: false,
            receivers: Vec::new(),
            appsink,
            _discovery: discovery,
        })
    }

    async fn new_pipeline(
        event_tx: Sender<Event>,
        appsink: gst::Element,
    ) -> Result<(Sender<usize>, pipeline::Pipeline)> {
        let (selected_tx, selected_rx) = sync::mpsc::channel::<usize>(1);

        let pipeline = pipeline::Pipeline::new(
            appsink,
            {
                let event_tx = event_tx.clone();
                let pipeline_has_finished = std::sync::Arc::new(AtomicBool::new(false));
                move |event| {
                    let event_tx = event_tx.clone();
                    let pipeline_has_finished = std::sync::Arc::clone(&pipeline_has_finished);
                    async move {
                        match event {
                            pipeline::Event::PipelineIsPlaying => {
                                event_tx
                                    .send(crate::Event::PipelineIsPlaying)
                                    .await
                                    .unwrap();
                            }
                            pipeline::Event::Eos => {
                                if !pipeline_has_finished.load(Ordering::Acquire) {
                                    event_tx.send(crate::Event::PipelineFinished).await.unwrap();
                                    pipeline_has_finished.store(true, Ordering::Release);
                                }
                            }
                            pipeline::Event::Error => {
                                if !pipeline_has_finished.load(Ordering::Acquire) {
                                    event_tx.send(crate::Event::PipelineFinished).await.unwrap();
                                    pipeline_has_finished.store(true, Ordering::Release);
                                }
                            }
                        }
                    }
                }
            },
            {
                let selected_rx = std::sync::Arc::new(std::sync::Mutex::new(selected_rx));
                move |vals| {
                    let sources = vals[1].get::<Vec<String>>().unwrap();
                    event_tx.blocking_send(Event::Sources(sources)).unwrap();
                    let mut selected_rx = selected_rx.lock().unwrap();
                    let res = selected_rx.blocking_recv().unwrap() as u64;
                    use gst::prelude::*;
                    Some(res.to_value())
                }
            },
        )
        .await?;

        Ok((selected_tx, pipeline))
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

    fn set_all_receivers_connectable(&mut self) {
        for r in &mut self.receivers {
            if r.0.state != ReceiverState::Connectable {
                r.0.state = ReceiverState::Connectable;
            }
        }
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

    pub async fn run_event_loop(
        &mut self,
        mut event_rx: Receiver<Event>,
        fin_tx: oneshot::Sender<()>,
    ) -> Result<()> {
        while let Some(event) = event_rx.recv().await {
            match event {
                Event::SessionTerminated => break,
                Event::StartCast => {
                    let Some(play_msg) = self.pipeline.get_play_msg() else {
                        error!("Could not get stream uri");
                        continue;
                    };

                    debug!("Sending play message: {play_msg:?}");

                    self.session_tx.send(SessionMessage::Play(play_msg)).await?;

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
                    for r in &mut self.receivers {
                        if r.0.state != ReceiverState::Connected {
                            continue;
                        }

                        if r.0.name.starts_with("OpenMirroring") {
                            self.pipeline.add_rtp_sink()?;
                        } else {
                            self.pipeline.add_hls_sink()?;
                        }

                        self.update_receivers_in_ui()?;

                        break;
                    }

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.set_has_source(true);
                    })?;
                }
                Event::Packet(packet) => {
                    trace!("Unhandled packet: {packet:?}");
                }
                Event::ReceiverAvailable(receiver) => {
                    if let Some(idx) = self.receivers_contains(&receiver.name) {
                        self.receivers[idx].1 = receiver.addresses[0];
                    } else {
                        self.receivers.push((
                            ReceiverItem {
                                name: receiver.name.into(),
                                state: self.receivers_general_state(),
                            },
                            receiver.addresses[0],
                        ));
                    }

                    self.update_receivers_in_ui()?;
                }
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
                Event::ConnectedToReceiver => {
                    debug!("Succesfully connected to receiver");

                    for r in &mut self.receivers {
                        if r.0.state == ReceiverState::Connecting {
                            r.0.state = ReceiverState::Connected;

                            if r.0.name.starts_with("OpenMirroring") {
                                self.pipeline.add_rtp_sink()?;
                            } else {
                                self.pipeline.add_hls_sink()?;
                            }
                            break;
                        }
                    }

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_receiver_connected();
                    })?;

                    self.update_receivers_in_ui()?;
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
                    self.pipeline.remove_transmission_sink()?;

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_receiver_disconnected();
                    })?;
                }
                Event::ChangeSource | Event::PipelineFinished => {
                    self.shutdown_pipeline_and_create_new_and_update_ui()
                        .await?;
                }
                Event::PipelineIsPlaying => {
                    self.pipeline.playing().await?;
                }
                Event::DisconnectedFromReceiver => {
                    debug!("Disconnected from receiver");

                    self.set_all_receivers_connectable();
                    self.update_receivers_in_ui()?;

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_receiver_disconnected();
                    })?;
                }
            }
        }

        debug!("Quitting");

        if !self.selected_source {
            debug!("Source is not selected, sending fake");
            self.select_source_tx.send(0).await?;
        }

        self.pipeline.shutdown()?;

        fin_tx.send(()).unwrap();

        Ok(())
    }

    async fn shutdown_pipeline_and_create_new_and_update_ui(&mut self) -> Result<()> {
        if !self.selected_source {
            self.select_source_tx.send(0).await?;
        }

        self.pipeline.shutdown()?;

        let (new_select_srouce_tx, new_pipeline) =
            Self::new_pipeline(self.event_tx.clone(), self.appsink.clone()).await?;

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
        .filter_module("common", common::default_log_level())
        .init();

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .require_opengl()
        .select()?;

    gst::init()?;
    scapgst::plugin_register_static()?;
    common::sender::pipeline::init()?;

    let (event_tx, event_rx) = sync::mpsc::channel::<Event>(100);
    let (session_tx, session_rx) = sync::mpsc::channel::<SessionMessage>(100);
    let (fin_tx, fin_rx) = oneshot::channel::<()>();

    // This sink is used in every consecutively created pipelines
    let mut slint_sink = SlintOpenGLSink::new()?;
    let slint_appsink = slint_sink.video_sink();

    let ui = MainWindow::new()?;
    slint::set_xdg_app_id("com.github.malba124.OpenMirroring.sender")?;

    let (gotten_gl_tx, gotten_gl_rx) = tokio::sync::oneshot::channel();

    ui.window().set_rendering_notifier({
        let ui_weak = ui.as_weak();

        let new_frame_cb = |ui: MainWindow, new_frame| {
            ui.set_preview_frame(new_frame);
        };

        let mut gotten_gl_tx = Some(gotten_gl_tx);

        move |state, graphics_api| match state {
            slint::RenderingState::RenderingSetup => {
                let ui_weak = ui_weak.clone();
                slint_sink
                    .connect(graphics_api, move || {
                        ui_weak
                            .upgrade_in_event_loop(move |ui| {
                                ui.window().request_redraw();
                            })
                            .ok();
                    })
                    .unwrap();
                if let Some(tx) = gotten_gl_tx.take() {
                    assert!(tx.send(()).is_ok());
                }
            }
            slint::RenderingState::BeforeRendering => {
                if let Some(next_frame) = slint_sink.fetch_next_frame() {
                    new_frame_cb(ui_weak.unwrap(), next_frame);
                }
            }
            slint::RenderingState::RenderingTeardown => {
                slint_sink.deactivate_and_pause().unwrap();
            }
            _ => (),
        }
    })?;

    common::runtime().spawn(session::session(session_rx, {
        let event_tx = event_tx.clone();
        move |event| {
            let event_tx = event_tx.clone();
            async move {
                match event {
                    session::Event::SessionTerminated => {
                        event_tx.send(Event::SessionTerminated).await.unwrap();
                    }
                    session::Event::FcastPacket(packet) => {
                        event_tx.send(Event::Packet(packet)).await.unwrap();
                    }
                    session::Event::ConnectedToReceiver => {
                        event_tx.send(Event::ConnectedToReceiver).await.unwrap();
                    }
                    session::Event::DisconnectedFromReceiver => {
                        event_tx
                            .send(Event::DisconnectedFromReceiver)
                            .await
                            .unwrap();
                    }
                }
            }
        }
    }));

    common::runtime().spawn({
        let ui_weak = ui.as_weak();
        let event_tx = event_tx.clone();
        let session_tx = session_tx.clone();
        async move {
            let mut app =
                Application::new(ui_weak, event_tx, session_tx, slint_appsink, gotten_gl_rx)
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
        ui.on_start_cast(move || {
            event_tx.blocking_send(Event::StartCast).unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_stop_cast(move || {
            event_tx.blocking_send(Event::StopCast).unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_disconnect_receiver(move || {
            event_tx.blocking_send(Event::DisconnectReceiver).unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_change_source(move || {
            event_tx.blocking_send(Event::ChangeSource).unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_add_receiver_manually(move |name, addr, port| {
            let parsed_addr = match format!("{addr}:{port}").parse::<std::net::SocketAddr>() {
                Ok(a) => a,
                Err(err) => {
                    // TODO: show in UI
                    error!("Failed to parse manually added receiver socket address: {err}");
                    return;
                }
            };
            event_tx
                .blocking_send(Event::ReceiverAvailable(sender::Receiver {
                    name: name.to_string(),
                    addresses: vec![parsed_addr],
                }))
                .unwrap();
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
