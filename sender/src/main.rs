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

use anyhow::{Context, Result, bail};
use common::sender::session::{Session, SessionEvent};
use fcast_lib::packet::Packet;
use gst::glib::object::ObjectExt;
use gst::prelude::{DeviceExt, GstObjectExt};
use log::{debug, error, trace};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};
use tokio::runtime::{self, Runtime};

use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;

use ashpd::desktop::{
    PersistMode,
    screencast::{CursorMode, Screencast, SourceType},
};

use common::sender::pipeline::{self, SourceConfig};

slint::include_modules!();

pub type ProducerId = String;

#[derive(Debug)]
pub enum AudioSource {
    Pulse(gst::Device),
}

impl AudioSource {
    pub fn display_name(&self) -> gst::glib::GString {
        match self {
            AudioSource::Pulse(device) => device.display_name(),
        }
    }
}

#[cfg(target_os = "linux")]
pub fn get_audio_devices() -> anyhow::Result<Vec<AudioSource>> {
    use anyhow::bail;
    use gst::prelude::*;

    let provider = gst::DeviceProviderFactory::by_name("pulsedeviceprovider").ok_or(
        anyhow::anyhow!("Could not find pulse device provider factory"),
    )?;

    provider.start()?;
    let devices = provider.devices();
    provider.stop();

    for device in devices {
        if !device.has_classes("Audio/Sink") {
            continue;
        }
        let Some(props) = device.properties() else {
            continue;
        };
        if props.get::<bool>("is-default") == Ok(true) {
            return Ok(vec![AudioSource::Pulse(device)]);
        }
    }

    bail!("No device found")
}

#[derive(Debug)]
enum VideoSource {
    PipeWire { node_id: u32, fd: i32 },
}

impl VideoSource {
    pub fn display_name(&self) -> String {
        match self {
            VideoSource::PipeWire { .. } => "PipeWire Video Source".to_owned(),
        }
    }
}

#[derive(Debug)]
pub enum Event {
    StartCast {
        addr_idx: usize,
        port: i32,
        video_idx: Option<usize>,
        audio_idx: Option<usize>,
    },
    StopCast,
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
    VideosAvailable(Vec<VideoSource>),
    AudiosAvailable(Vec<AudioSource>),
    ReloadVideoSources,
    ReloadAudioSources,
    Quit,
}

enum FetchEvent {
    Fetch,
    Quit,
}

struct Application {
    pipeline: Option<pipeline::Pipeline>,
    ui_weak: slint::Weak<MainWindow>,
    event_tx: crossbeam_channel::Sender<Event>,
    receivers: Vec<(ReceiverItem, SocketAddr)>,
    addresses: Vec<IpAddr>,
    mdns: mdns_sd::ServiceDaemon,
    mdns_receiver: flume::Receiver<mdns_sd::ServiceEvent>,
    session: Session,
    should_play: bool,
    selected_addr_idx: usize,
    video_sources: Vec<VideoSource>,
    audio_sources: Vec<AudioSource>,
    video_source_fetcher_tx: tokio::sync::mpsc::Sender<FetchEvent>,
    audio_source_fetcher_tx: tokio::sync::mpsc::Sender<FetchEvent>,
}

impl Application {
    /// Must be called from a tokio runtime.
    pub fn new(
        ui_weak: slint::Weak<MainWindow>,
        event_tx: crossbeam_channel::Sender<Event>,
    ) -> Result<Self> {
        let mdns = mdns_sd::ServiceDaemon::new()?;
        let mdns_receiver = mdns.browse("_fcast._tcp.local.")?;

        let (video_source_fetcher_tx, mut video_source_fetcher_rx) = tokio::sync::mpsc::channel(10);
        let (audio_source_fetcher_tx, mut audio_source_fetcher_rx) = tokio::sync::mpsc::channel(10);

        #[cfg(target_os = "linux")]
        {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let mut proxy = None;
                let mut session = None;
                enum WindowingSystem {
                    Wayland,
                    X11,
                }

                let winsys = if std::env::var("WAYLAND_DISPLAY").is_ok() {
                    WindowingSystem::Wayland
                } else if std::env::var("DISPLAY").is_ok() {
                    WindowingSystem::X11
                } else {
                    panic!("Unsupported windowing system!");
                    // TODO: tell user or something
                };

                loop {
                    let Some(event) = video_source_fetcher_rx.recv().await else {
                        error!("Failed to receive new video source fetcher event");
                        break;
                    };

                    match (event, &winsys) {
                        (FetchEvent::Fetch, WindowingSystem::Wayland) => {
                            let new_proxy = Screencast::new().await.unwrap();
                            let new_session = new_proxy.create_session().await.unwrap();
                            new_proxy
                                .select_sources(
                                    &new_session,
                                    CursorMode::Embedded,
                                    SourceType::Monitor | SourceType::Window,
                                    false,
                                    None,
                                    PersistMode::DoNot,
                                )
                                .await
                                .unwrap();

                            let response = new_proxy
                                .start(&new_session, None)
                                .await
                                .unwrap()
                                .response()
                                .unwrap();
                            let stream = response.streams().get(0).unwrap();
                            let fd = new_proxy
                                .open_pipe_wire_remote(&new_session)
                                .await
                                .unwrap()
                                .as_raw_fd();
                            event_tx
                                .send(Event::VideosAvailable(vec![VideoSource::PipeWire {
                                    node_id: stream.pipe_wire_node_id(),
                                    fd,
                                }]))
                                .unwrap();

                            proxy = Some(new_proxy);
                            session = Some(new_session);
                        }
                        (FetchEvent::Fetch, WindowingSystem::X11) => debug!("TODO: X11"),
                        (FetchEvent::Quit, _) => break,
                    }
                }

                debug!("Video source fetch loop quit");
            });
        }

        #[cfg(target_os = "linux")]
        {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                loop {
                    let Some(event) = audio_source_fetcher_rx.recv().await else {
                        error!("Failed to receive new video source fetcher event");
                        break;
                    };

                    match event {
                        FetchEvent::Fetch => {
                            match get_audio_devices() {
                                Ok(devs) => {
                                    for dev in devs {
                                        event_tx.send(Event::AudiosAvailable(vec![dev])).unwrap();
                                    }
                                }
                                Err(err) => error!("Failed to get default pulse device: {err}"),
                            };
                        }
                        FetchEvent::Quit => break,
                    }
                }

                debug!("Audio source fetch loop quit");
            });
        }

        Ok(Self {
            pipeline: None,
            ui_weak,
            event_tx,
            receivers: Vec::new(),
            addresses: Vec::new(),
            mdns,
            mdns_receiver,
            session: Session::default(),
            should_play: false,
            selected_addr_idx: 0,
            video_sources: Vec::new(),
            audio_sources: Vec::new(),
            video_source_fetcher_tx,
            audio_source_fetcher_tx,
        })
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

        if let Some(mut pipeline) = self.pipeline.take() {
            if let Err(err) = pipeline.shutdown() {
                error!("Failed to shutdown pipeline: {err}");
            }
        }

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
    async fn handle_event(&mut self, event: Event) -> Result<bool> {
        debug!("Handling event: {event:?}");

        match event {
            Event::StartCast {
                addr_idx,
                port,
                video_idx,
                audio_idx,
            } => {
                let port = {
                    if port < 1 || port > u16::MAX as i32 {
                        error!("Port ({port}) is not in the valid port range");
                        return Ok(false);
                    }
                    port as u16
                };
                self.selected_addr_idx = addr_idx;

                let video_src = match video_idx {
                    Some(video_idx) => {
                        let Some(video_src) = self.video_sources.get(video_idx) else {
                            error!(
                                "Failed to get video source, video_idx ({video_idx}) > video_sources.len ({})",
                                self.video_sources.len()
                            );
                            return Ok(false);
                        };
                        match video_src {
                            VideoSource::PipeWire { node_id, .. } => Some(
                                gst::ElementFactory::make("pipewiresrc")
                                    .property("path", node_id.to_string())
                                    // .property("fd", fd) // ??
                                    .build()?,
                            ),
                        }
                    }
                    None => None,
                };

                let audio_src = match audio_idx {
                    Some(audio_idx) => {
                        let Some(audio_src) = self.audio_sources.get(audio_idx) else {
                            error!(
                                "Failed to get audio source, audio_idx ({audio_idx}) > audio_sources.len ({})",
                                self.audio_sources.len()
                            );
                            return Ok(false);
                        };
                        match audio_src {
                            AudioSource::Pulse(device) => {
                                let pulsesink = device.create_element(None)?;
                                let device_name = pulsesink
                                    .property::<Option<String>>("device")
                                    .context("No device name")?;

                                let monitor_name = format!("{}.monitor", device_name);
                                Some(
                                    gst::ElementFactory::make("pulsesrc")
                                        .property("provide-clock", false)
                                        .property("device", format!("{}.monitor", device_name))
                                        .build()?,
                                )
                            }
                        }
                    }
                    None => None,
                };

                let source_config = match (video_src, audio_src) {
                    (Some(video), Some(audio)) => SourceConfig::AudioVideo { video, audio },
                    (Some(video), None) => SourceConfig::Video(video),
                    (None, Some(audio)) => SourceConfig::Audio(audio),
                    _ => bail!("must have at least one source"),
                };

                for r in self.receivers.iter_mut() {
                    if r.0.state != ReceiverState::Connected {
                        continue;
                    }

                    debug!("Adding RTSP pipeline");
                    self.pipeline = Some(pipeline::Pipeline::new_rtsp(
                        {
                            let event_tx = self.event_tx.clone();
                            let pipeline_has_finished = Arc::new(AtomicBool::new(false));
                            move |event| {
                                let event_tx = event_tx.clone();
                                let pipeline_has_finished = Arc::clone(&pipeline_has_finished);
                                match event {
                                    pipeline::Event::PipelineIsPlaying => {
                                        event_tx.send(crate::Event::PipelineIsPlaying).unwrap();
                                    }
                                    pipeline::Event::Eos => {
                                        if !pipeline_has_finished.load(Ordering::Acquire) {
                                            event_tx
                                                .send(crate::Event::PipelineFinished)
                                                .unwrap();
                                            pipeline_has_finished
                                                .store(true, Ordering::Release);
                                        }
                                    }
                                    pipeline::Event::Error => {
                                        if !pipeline_has_finished.load(Ordering::Acquire) {
                                            event_tx
                                                .send(crate::Event::PipelineFinished)
                                                .unwrap();
                                            pipeline_has_finished
                                                .store(true, Ordering::Release);
                                        }
                                    }
                                }
                            }
                        },
                        source_config,
                    )?);

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

                    let Some(play_msg) = self.pipeline.as_ref().unwrap().get_play_msg(*addr)
                    else {
                        error!("Could not get stream uri");
                        return Ok(false);
                    };

                    debug!("Sending play message: {play_msg:?}");

                    self.send_packet_to_receiver(Packet::Play(play_msg))?;

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_cast_started();
                    })?;

                    return Ok(false);

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
                if let Some(mut pipeline) = self.pipeline.take() {
                    if let Err(err) = pipeline.shutdown() {
                        error!("Failed to shutdown pipeline: {err}");
                    }
                }
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
            Event::ChangeSource | Event::PipelineFinished => error!("TODO"),
            Event::PipelineIsPlaying => {
                if let Some(pipeline) = self.pipeline.as_mut() {
                    if let Err(err) = pipeline.playing() {
                        error!("Failed to run playing on pipeline: {err}");
                        // TODO: set pipeline to None?
                    }
                }

                if self.should_play {
                    self.should_play = false;
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

                    let Some(pipeline) = self.pipeline.as_ref() else {
                        error!("Should play but missing pipeline");
                        return Ok(false);
                    };
                    let Some(play_msg) = pipeline.get_play_msg(*addr) else {
                        error!("Could not get stream uri");
                        return Ok(false);
                    };

                    debug!("Sending play message: {play_msg:?}");

                    self.send_packet_to_receiver(Packet::Play(play_msg))?;

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_cast_started();
                    })?;
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
            Event::VideosAvailable(sources) => {
                self.video_sources = sources;
                self.update_video_sources_in_ui()?;
            }
            Event::AudiosAvailable(sources) => {
                self.audio_sources = sources;
                self.update_audio_sources_in_ui()?;
            }
            Event::ReloadVideoSources => {
                self.video_source_fetcher_tx.send(FetchEvent::Fetch).await?
            }
            Event::ReloadAudioSources => {
                self.audio_source_fetcher_tx.send(FetchEvent::Fetch).await?
            }
        }

        Ok(false)
    }

    fn update_audio_sources_in_ui(&self) -> Result<()> {
        let audio_dev_names = self
            .audio_sources
            .iter()
            .map(|dev| slint::SharedString::from(dev.display_name().as_str()))
            .collect::<Vec<slint::SharedString>>();
        self.ui_weak.upgrade_in_event_loop(move |ui| {
            ui.set_audio_sources_model(
                Rc::new(slint::VecModel::<slint::SharedString>::from_slice(
                    &audio_dev_names,
                ))
                .into(),
            );
        })?;

        Ok(())
    }

    fn update_video_sources_in_ui(&self) -> Result<()> {
        let video_dev_names = self
            .video_sources
            .iter()
            .map(|dev| slint::SharedString::from(dev.display_name().as_str()))
            .collect::<Vec<slint::SharedString>>();
        self.ui_weak.upgrade_in_event_loop(move |ui| {
            ui.set_video_sources_model(
                Rc::new(slint::VecModel::<slint::SharedString>::from_slice(
                    &video_dev_names,
                ))
                .into(),
            );
        })?;

        Ok(())
    }

    pub async fn run_event_loop(
        mut self,
        event_rx: crossbeam_channel::Receiver<Event>,
    ) -> Result<()> {
        self.update_addresses_and_in_ui()?;
        const ADDR_UPDATE_INTERVAL: Duration = Duration::from_secs(5);
        let mut prev_addr_update = Instant::now();

        self.video_source_fetcher_tx.send(FetchEvent::Fetch).await?;
        self.audio_source_fetcher_tx.send(FetchEvent::Fetch).await?;

        // TODO: do all async
        loop {
            match event_rx.try_recv() {
                Ok(event) => {
                    if self.handle_event(event).await? {
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

            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        debug!("Quitting event loop");

        if let Some(mut pipeline) = self.pipeline.take() {
            if let Err(err) = pipeline.shutdown() {
                error!("Failed to shutdown pipeline: {err}");
            }
        }

        self.mdns.shutdown()?;

        self.video_source_fetcher_tx.send(FetchEvent::Quit).await?;
        self.audio_source_fetcher_tx.send(FetchEvent::Quit).await?;

        Ok(())
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_module("sender", common::default_log_level())
        .filter_module("common", common::default_log_level())
        .filter_module("mdns_sd", common::default_log_level())
        .init();

    gst::init()?;
    common::sender::pipeline::init()?;

    let runtime = Runtime::new()?;

    let (event_tx, event_rx) = crossbeam_channel::bounded::<Event>(100);

    let ui = MainWindow::new()?;
    slint::set_xdg_app_id("com.github.malba124.OpenMirroring.sender")?;

    let event_loop_jh = runtime.spawn({
        let ui_weak = ui.as_weak();
        let event_tx = event_tx.clone();
        async move {
            Application::new(ui_weak, event_tx)
                .unwrap()
                .run_event_loop(event_rx)
                .await
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
        ui.on_connect_receiver(move |receiver| {
            event_tx
                .send(Event::SelectReceiver(receiver.to_string()))
                .unwrap();
        });
    });

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
        ui.on_start_cast(move |(addr_idx, port), video_idx, audio_idx| {
            event_tx
                .send(Event::StartCast {
                    addr_idx: addr_idx as usize,
                    port,
                    video_idx: if video_idx >= 0 {
                        Some(video_idx as usize)
                    } else {
                        None
                    },
                    audio_idx: if audio_idx >= 0 {
                        Some(audio_idx as usize)
                    } else {
                        None
                    },
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

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
        ui.on_reload_video_sources(move || {
            event_tx.send(Event::ReloadVideoSources).unwrap();
        });
    });

    event_handler!(event_tx, |event_tx: crossbeam_channel::Sender<Event>| {
        ui.on_reload_audio_sources(move || {
            event_tx.send(Event::ReloadAudioSources).unwrap();
        });
    });

    ui.run()?;

    runtime.block_on(async move {
        event_tx.send(Event::Quit).unwrap();
        event_loop_jh.await.unwrap();
    });

    Ok(())
}
