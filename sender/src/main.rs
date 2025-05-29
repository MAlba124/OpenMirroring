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
use common::sender::session::{Session, SessionEvent};
use common::video::opengl::SlintOpenGLSink;
use fcast_lib::packet::Packet;
use log::{debug, error, trace};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, atomic};
use std::thread::sleep;
use std::time::{Duration, Instant};

use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;

use common::sender::pipeline;

slint::include_modules!();

pub type ProducerId = String;

#[derive(Debug)]
pub enum Event {
    StartCast {
        addr_idx: usize,
        port: i32,
    },
    StopCast,
    Sources(Vec<String>),
    SelectSource(usize),
    Packet(fcast_lib::packet::Packet),
    ReceiverAvailable {
        name: String,
        addresses: Vec<SocketAddr>,
    },
    SelectReceiver(String),
    DisconnectReceiver,
    ChangeSource,
    PipelineFinished,
    PipelineIsPlaying,
    DisconnectedFromReceiver,
    Quit,
}

struct Application {
    pipeline: pipeline::Pipeline,
    ui_weak: slint::Weak<MainWindow>,
    event_tx: crossbeam_channel::Sender<Event>,
    select_source_tx: crossbeam_channel::Sender<usize>,
    selected_source: bool,
    receivers: Vec<(ReceiverItem, SocketAddr)>,
    appsink: gst::Element,
    addresses: Vec<IpAddr>,
    mdns: mdns_sd::ServiceDaemon,
    mdns_receiver: flume::Receiver<mdns_sd::ServiceEvent>,
    session: Session,
    should_play: bool,
    selected_addr_idx: usize,
}

impl Application {
    pub fn new(
        ui_weak: slint::Weak<MainWindow>,
        event_tx: crossbeam_channel::Sender<Event>,
        appsink: gst::Element,
    ) -> Result<Self> {
        let (select_source_tx, pipeline) = Self::new_pipeline(event_tx.clone(), appsink.clone())?;

        let mdns = mdns_sd::ServiceDaemon::new()?;
        let mdns_receiver = mdns.browse("_fcast._tcp.local.")?;

        Ok(Self {
            pipeline,
            ui_weak,
            event_tx,
            select_source_tx,
            selected_source: false,
            receivers: Vec::new(),
            appsink,
            addresses: Vec::new(),
            mdns,
            mdns_receiver,
            session: Session::default(),
            should_play: false,
            selected_addr_idx: 0,
        })
    }

    fn new_pipeline(
        event_tx: crossbeam_channel::Sender<Event>,
        appsink: gst::Element,
    ) -> Result<(crossbeam_channel::Sender<usize>, pipeline::Pipeline)> {
        let (selected_tx, selected_rx) = crossbeam_channel::bounded::<usize>(1);

        let pipeline = pipeline::Pipeline::new(
            appsink,
            {
                let event_tx = event_tx.clone();
                let pipeline_has_finished = std::sync::Arc::new(AtomicBool::new(false));
                move |event| {
                    let event_tx = event_tx.clone();
                    let pipeline_has_finished = std::sync::Arc::clone(&pipeline_has_finished);
                    match event {
                        pipeline::Event::PipelineIsPlaying => {
                            event_tx.send(crate::Event::PipelineIsPlaying).unwrap();
                        }
                        pipeline::Event::Eos => {
                            if !pipeline_has_finished.load(Ordering::Acquire) {
                                event_tx.send(crate::Event::PipelineFinished).unwrap();
                                pipeline_has_finished.store(true, Ordering::Release);
                            }
                        }
                        pipeline::Event::Error => {
                            if !pipeline_has_finished.load(Ordering::Acquire) {
                                event_tx.send(crate::Event::PipelineFinished).unwrap();
                                pipeline_has_finished.store(true, Ordering::Release);
                            }
                        }
                    }
                }
            },
            {
                let selected_rx = std::sync::Arc::new(std::sync::Mutex::new(selected_rx));
                move |vals| {
                    let sources = vals[1].get::<Vec<String>>().unwrap();
                    event_tx.send(Event::Sources(sources)).unwrap();
                    let selected_rx = selected_rx.lock().unwrap();
                    let res = selected_rx.recv().unwrap() as u64;
                    use gst::prelude::*;
                    Some(res.to_value())
                }
            },
        )?;

        Ok((selected_tx, pipeline))
    }

    fn disconnect_receiver(&mut self) -> Result<()> {
        for r in self.receivers.iter_mut() {
            if r.0.state == ReceiverState::Connected || r.0.state == ReceiverState::Connecting {
                r.0.state = ReceiverState::Connectable;
                self.update_receivers_in_ui()?;
                break;
            }
        }

        if let Err(err) = self.session.disconnect() {
            error!("Error occured when disconnecting from receiver: {err}");
        }
        self.pipeline.remove_transmission_sink()?;

        self.ui_weak.upgrade_in_event_loop(|ui| {
            ui.invoke_receiver_disconnected();
        })?;

        Ok(())
    }

    fn send_packet_to_receiver(&mut self, packet: Packet) -> Result<()> {
        if let Err(err) = self.session.send_packet(packet) {
            error!("Failed to send packet to receiver: {err}");
            self.disconnect_receiver()?;
        }

        Ok(())
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

    fn update_addresses_and_in_ui(&mut self) -> Result<()> {
        let mut new_addrs = common::net::get_all_ip_addresses();
        new_addrs.sort_by(|a, b| {
            if a.is_loopback() || b.is_loopback() {
                std::cmp::Ordering::Less
            } else if a.is_ipv4() && b.is_ipv6() {
                std::cmp::Ordering::Greater
            } else {
                a.cmp(b)
            }
        });

        if new_addrs != self.addresses {
            self.addresses = new_addrs;
        } else {
            return Ok(());
        }

        let addrs = self
            .addresses
            .iter()
            .map(|a| slint::SharedString::from(a.to_string()))
            .collect::<Vec<slint::SharedString>>();
        self.ui_weak.upgrade_in_event_loop(move |ui| {
            ui.set_addresses_model(
                Rc::new(slint::VecModel::<slint::SharedString>::from_iter(
                    addrs.into_iter(),
                ))
                .into(),
            );
        })?;

        Ok(())
    }

    fn add_receiver(&mut self, mut name: String, addrs: Vec<SocketAddr>) -> Result<()> {
        if let Some(stripped) = name.strip_suffix("._fcast._tcp.local.") {
            name = stripped.to_owned();
        }
        debug!("Receiver available: {}", name);
        if let Some(idx) = self.receivers_contains(&name) {
            self.receivers[idx].1 = addrs[0];
        } else {
            self.receivers.push((
                ReceiverItem {
                    name: name.into(),
                    state: self.receivers_general_state(),
                },
                addrs[0],
            ));
        }

        self.update_receivers_in_ui()?;

        Ok(())
    }

    /// Returns `true` if the event loop should quit
    fn handle_event(&mut self, event: Event) -> Result<bool> {
        match event {
            Event::StartCast { addr_idx, port } => {
                let port = {
                    if port < 1 || port > u16::MAX as i32 {
                        error!("Port ({port}) is not in the valid port range");
                        return Ok(false);
                    }
                    port as u16
                };
                self.selected_addr_idx = addr_idx;

                for r in self.receivers.iter_mut() {
                    if r.0.state != ReceiverState::Connected {
                        continue;
                    }

                    if r.0.name.starts_with("OpenMirroring") {
                        self.pipeline.add_rtsp_sink(port)?;

                        let addr = match self.addresses.get(self.selected_addr_idx) {
                            Some(addr) => addr,
                            None => {
                                error!(
                                    "Address ({}) is out of bounds in the addresses list",
                                    self.selected_addr_idx,
                                );
                                return Ok(false);
                            }
                        };

                        let Some(play_msg) = self.pipeline.get_play_msg(*addr) else {
                            error!("Could not get stream uri");
                            return Ok(false);
                        };

                        debug!("Sending play message: {play_msg:?}");

                        self.send_packet_to_receiver(Packet::Play(play_msg))?;

                        self.ui_weak.upgrade_in_event_loop(|ui| {
                            ui.invoke_cast_started();
                        })?;

                        return Ok(false);
                    } else {
                        self.pipeline.add_hls_sink(port)?;
                    }

                    break;
                }

                self.ui_weak.upgrade_in_event_loop(|ui| {
                    ui.invoke_cast_starting();
                })?;

                self.should_play = true;
            }
            Event::StopCast => {
                self.send_packet_to_receiver(Packet::Stop)?;
                self.ui_weak.upgrade_in_event_loop(|ui| {
                    ui.invoke_cast_stopped();
                })?;
                self.pipeline.remove_transmission_sink()?;
            }
            Event::Sources(sources) => {
                debug!("Available sources: {sources:?}");
                if sources.len() == 1 {
                    debug!("One source available, auto selecting");
                    self.event_tx.send(Event::SelectSource(0))?;
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
                self.select_source_tx.send(idx)?;
                self.selected_source = true;
                self.ui_weak.upgrade_in_event_loop(|ui| {
                    ui.set_has_source(true);
                })?;
            }
            Event::Packet(packet) => {
                trace!("Unhandled packet: {packet:?}");
            }
            Event::ReceiverAvailable {
                mut name,
                addresses,
            } => {
                if let Some(stripped) = name.strip_suffix("._fcast._tcp.local.") {
                    name = stripped.to_owned();
                }
                self.add_receiver(name, addresses)?;
            }
            Event::SelectReceiver(receiver) => {
                if let Some(idx) = self.receivers_contains(&receiver) {
                    self.receivers[idx].0.state = ReceiverState::Connecting;
                    self.session.connect(self.receivers[idx].1);
                    self.update_receivers_in_ui()?;
                } else {
                    error!("No receiver `{receiver}` available");
                }
            }
            Event::DisconnectReceiver => self.disconnect_receiver()?,
            Event::ChangeSource | Event::PipelineFinished => {
                self.shutdown_pipeline_and_create_new_and_update_ui()?;
            }
            Event::PipelineIsPlaying => {
                self.pipeline.playing()?;

                if self.should_play {
                    let addr = match self.addresses.get(self.selected_addr_idx) {
                        Some(addr) => addr,
                        None => {
                            error!(
                                "Address ({}) is out of bounds in the addresses list",
                                self.selected_addr_idx,
                            );
                            return Ok(false);
                        }
                    };

                    let Some(play_msg) = self.pipeline.get_play_msg(*addr) else {
                        error!("Could not get stream uri");
                        return Ok(false);
                    };

                    debug!("Sending play message: {play_msg:?}");

                    self.send_packet_to_receiver(Packet::Play(play_msg))?;

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_cast_started();
                    })?;
                    self.should_play = false;
                }
            }
            Event::DisconnectedFromReceiver => {
                debug!("Disconnected from receiver");

                self.set_all_receivers_connectable();
                self.update_receivers_in_ui()?;

                self.ui_weak.upgrade_in_event_loop(|ui| {
                    ui.invoke_receiver_disconnected();
                })?;
            }
            Event::Quit => return Ok(true),
        }

        Ok(false)
    }

    pub fn run_event_loop(mut self, event_rx: crossbeam_channel::Receiver<Event>) -> Result<()> {
        self.update_addresses_and_in_ui()?;
        const ADDR_UPDATE_INTERVAL: Duration = Duration::from_secs(5);
        let mut prev_addr_update = Instant::now();

        loop {
            match event_rx.try_recv() {
                Ok(event) => {
                    if self.handle_event(event)? {
                        break;
                    }
                }
                Err(err) if err.is_disconnected() => {
                    error!("Failed to receive event: {err}");
                    break;
                }
                _ => (),
            }

            match self.mdns_receiver.try_recv() {
                Ok(event) => match event {
                    mdns_sd::ServiceEvent::ServiceResolved(service_info) => {
                        let port = service_info.get_port();
                        let addrs = service_info
                            .get_addresses()
                            .iter()
                            .map(|a| SocketAddr::new(*a, port))
                            .collect::<Vec<SocketAddr>>();

                        self.add_receiver(service_info.get_fullname().to_owned(), addrs)?;
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, mut fullname) => {
                        if let Some(stripped) = fullname.strip_suffix("._fcast._tcp.local.") {
                            fullname = stripped.to_owned();
                        }
                        if let Some(idx) = self.receivers_contains(&fullname) {
                            debug!("Receiver removed: {fullname}");
                            self.receivers.remove(idx);
                            self.update_receivers_in_ui()?;
                        }
                    }
                    _ => (),
                },
                Err(err) if err == flume::TryRecvError::Disconnected => {
                    error!("Failed to receive mDNS event: {err}");
                    break;
                }
                _ => (),
            }

            match self.session.poll_event() {
                Ok(Some(event)) => match event {
                    SessionEvent::Packet(packet) => {
                        if packet == Packet::Ping {
                            self.send_packet_to_receiver(Packet::Pong)?;
                        }
                    }
                    SessionEvent::Connected => {
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
                },
                Err(err) => {
                    error!("Failed to poll session event: {err}");
                    self.disconnect_receiver()?;
                }
                _ => (),
            }

            if prev_addr_update.elapsed() >= ADDR_UPDATE_INTERVAL {
                prev_addr_update = Instant::now();
                self.update_addresses_and_in_ui()?;
            }

            sleep(Duration::from_millis(25));
        }

        debug!("Quitting");

        if !self.selected_source {
            debug!("Source is not selected, sending fake");
            self.select_source_tx.send(0)?;
        }

        self.pipeline.shutdown()?;

        self.mdns.shutdown()?;

        Ok(())
    }

    fn shutdown_pipeline_and_create_new_and_update_ui(&mut self) -> Result<()> {
        if !self.selected_source {
            self.select_source_tx.send(0)?;
        }

        self.pipeline.shutdown()?;

        let (new_select_srouce_tx, new_pipeline) =
            Self::new_pipeline(self.event_tx.clone(), self.appsink.clone())?;

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
        .filter_module("mdns_sd", common::default_log_level())
        .init();

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .require_opengl()
        .select()?;

    gst::init()?;
    scapgst::plugin_register_static()?;
    common::sender::pipeline::init()?;

    let (event_tx, event_rx) = crossbeam_channel::bounded::<Event>(100);

    // This sink is used in every consecutively created pipelines
    let mut slint_sink = SlintOpenGLSink::new()?;
    let slint_appsink = slint_sink.video_sink();

    let ui = MainWindow::new()?;
    slint::set_xdg_app_id("com.github.malba124.OpenMirroring.sender")?;

    let gotten_gl = Arc::new(AtomicBool::new(false));

    ui.window().set_rendering_notifier({
        let ui_weak = ui.as_weak();
        let gotten_gl = Arc::clone(&gotten_gl);

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
                gotten_gl.store(true, atomic::Ordering::Release);
            }
            slint::RenderingState::BeforeRendering => {
                if let Some(next_frame) = slint_sink.fetch_next_frame() {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_preview_frame(next_frame);
                    } else {
                        error!("Failed to upgrade ui_weak");
                    }
                }
            }
            slint::RenderingState::RenderingTeardown => {
                slint_sink.deactivate_and_pause().unwrap();
            }
            _ => (),
        }
    })?;

    let event_loop_jh = std::thread::spawn({
        let ui_weak = ui.as_weak();
        let event_tx = event_tx.clone();
        move || {
            // We need to wait until the preview sink has gotten it's required GL contexts,
            // if not, creating a pipeline would fail
            while !gotten_gl.load(atomic::Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(25));
            }

            Application::new(ui_weak, event_tx, slint_appsink)
                .unwrap()
                .run_event_loop(event_rx)
                .unwrap();
        }
    });

    macro_rules! event_handler {
        ($event_tx:expr, $closure:expr) => {{
            let event_tx = $event_tx.clone();
            $closure(event_tx);
        }};
    }

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
        ui.on_select_source(move |idx| {
            assert!(idx >= 0, "Invalid select source index");
            event_tx.send(Event::SelectSource(idx as usize)).unwrap();
        });
    });

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
        ui.on_connect_receiver(move |receiver| {
            event_tx
                .send(Event::SelectReceiver(receiver.to_string()))
                .unwrap();
        });
    });

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
        ui.on_start_cast(move |(addr_idx, port)| {
            event_tx
                .send(Event::StartCast {
                    addr_idx: addr_idx as usize,
                    port,
                })
                .unwrap();
        });
    });

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
        ui.on_stop_cast(move || {
            event_tx.send(Event::StopCast).unwrap();
        });
    });

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
        ui.on_disconnect_receiver(move || {
            event_tx.send(Event::DisconnectReceiver).unwrap();
        });
    });

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
        ui.on_change_source(move || {
            event_tx.send(Event::ChangeSource).unwrap();
        });
    });

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
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
                .send(Event::ReceiverAvailable {
                    name: name.to_string(),
                    addresses: vec![parsed_addr],
                })
                .unwrap();
        });
    });

    ui.run()?;

    event_tx.send(Event::Quit).unwrap();

    event_loop_jh.join().unwrap();

    Ok(())
}
