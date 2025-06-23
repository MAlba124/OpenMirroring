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
use log::{debug, error, trace};
use std::cell::Cell;
use std::ffi::CString;
use std::ffi::{CStr, NulError};
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

use std::net::{IpAddr, SocketAddr};

#[cfg(target_os = "linux")]
use ashpd::desktop::{
    PersistMode,
    screencast::{CursorMode, Screencast, SourceType},
};

#[cfg(target_os = "linux")]
use x11::xlib::{XFreeStringList, XGetTextProperty, XTextProperty, XmbTextPropertyToTextList};
#[cfg(target_os = "linux")]
use xcb::{
    Xid,
    randr::{GetCrtcInfo, GetOutputInfo, GetScreenResources},
    x::{self, GetPropertyReply},
};

use common::sender::pipeline::{self, SourceConfig};

slint::include_modules!();

pub type ProducerId = String;

#[derive(Debug)]
pub enum AudioSource {
    #[cfg(target_os = "linux")]
    Pipewire { name: String, id: u32 },
}

impl AudioSource {
    #[cfg(target_os = "linux")]
    pub fn display_name(&self) -> String {
        match self {
            AudioSource::Pipewire { name, .. } => name.clone(),
        }
    }
}

#[cfg(target_os = "linux")]
pub fn get_audio_devices() -> anyhow::Result<Vec<AudioSource>> {
    use std::cell::RefCell;

    let mainloop =
        pipewire::main_loop::MainLoop::new(None).context("failed to create PipeWire main loop")?;
    let context =
        pipewire::context::Context::new(&mainloop).context("failed to create PipeWire context")?;
    let core = context
        .connect(None)
        .context("failed to connect to PipeWire core")?;
    let registry = core
        .get_registry()
        .context("failed to get PipeWire registry")?;

    let done = Rc::new(Cell::new(false));
    let done_clone = done.clone();
    let loop_clone = mainloop.clone();
    let pending = core.sync(0).expect("sync failed");
    let pw_sources = Rc::new(RefCell::new(Vec::new()));
    let pw_sources_clone = pw_sources.clone();

    let _listener_core = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pipewire::core::PW_ID_CORE && seq == pending {
                done_clone.set(true);
                loop_clone.quit();
            }
        })
        .register();
    let _listener_reg = registry
        .add_listener_local()
        .global(move |global| {
            let props = global.props.as_ref().unwrap();
            let Some(mut name) = props
                .get(&pipewire::keys::NODE_DESCRIPTION)
                .or_else(|| props.get(&pipewire::keys::NODE_NICK))
                .or_else(|| props.get(&pipewire::keys::NODE_NAME))
                .map(|n| n.to_string())
            else {
                return;
            };

            let Some(media_class) = props.get(&pipewire::keys::MEDIA_CLASS) else {
                return;
            };

            if !media_class.contains("Audio") {
                return;
            }

            if media_class == "Audio/Source" {
                name.push_str(" (source)");
            } else if media_class == "Audio/Sink" {
                name.push_str(" (sink)");
            }

            if props
                .get(&pipewire::keys::MEDIA_CLASS)
                .map(|class| class.contains("Audio"))
                != Some(true)
            {
                debug!("PipeWire object `{name}` is not audio class");
                return;
            }

            pw_sources_clone.borrow_mut().push(AudioSource::Pipewire {
                name,
                id: global.id,
            });
        })
        .register();

    while !done.get() {
        mainloop.run();
    }

    Ok(pw_sources.take())
}

#[derive(Debug)]
enum VideoSource {
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
}

impl VideoSource {
    #[cfg(target_os = "linux")]
    pub fn display_name(&self) -> String {
        match self {
            VideoSource::PipeWire { .. } => "PipeWire Video Source".to_owned(),
            VideoSource::XWindow { name, .. } => name.clone(),
            VideoSource::XDisplay { name, .. } => name.clone(),
        }
    }
}

#[cfg(target_os = "linux")]
fn get_atom(conn: &xcb::Connection, atom_name: &str) -> Result<x::Atom, xcb::Error> {
    let cookie = conn.send_request(&x::InternAtom {
        only_if_exists: true,
        name: atom_name.as_bytes(),
    });
    Ok(conn.wait_for_reply(cookie)?.atom())
}

#[cfg(target_os = "linux")]
fn get_property(
    conn: &xcb::Connection,
    win: x::Window,
    prop: x::Atom,
    typ: x::Atom,
    length: u32,
) -> Result<GetPropertyReply, xcb::Error> {
    let cookie = conn.send_request(&x::GetProperty {
        delete: false,
        window: win,
        property: prop,
        r#type: typ,
        long_offset: 0,
        long_length: length,
    });
    conn.wait_for_reply(cookie)
}

#[cfg(target_os = "linux")]
fn decode_compound_text(
    conn: &xcb::Connection,
    value: &[u8],
    client: &xcb::x::Window,
    ttype: xcb::x::Atom,
) -> Result<String, NulError> {
    let display = conn.get_raw_dpy();
    assert!(!display.is_null());

    let c_string = CString::new(value.to_vec())?;
    let mut text_prop = XTextProperty {
        value: std::ptr::null_mut(),
        encoding: 0,
        format: 0,
        nitems: 0,
    };
    let res = unsafe {
        XGetTextProperty(
            display,
            client.resource_id() as u64,
            &mut text_prop,
            x::ATOM_WM_NAME.resource_id() as u64,
        )
    };
    if res == 0 || text_prop.nitems == 0 {
        return Ok(String::from("n/a"));
    }

    let xname = XTextProperty {
        value: c_string.as_ptr() as *mut u8,
        encoding: ttype.resource_id() as u64,
        format: 8,
        nitems: text_prop.nitems,
    };
    let mut list: *mut *mut i8 = std::ptr::null_mut();
    let mut count: i32 = 0;
    let result = unsafe { XmbTextPropertyToTextList(display, &xname, &mut list, &mut count) };
    if result < 1 || list.is_null() || count < 1 {
        Ok(String::from("n/a"))
    } else {
        let title = unsafe { CStr::from_ptr(*list).to_string_lossy().into_owned() };
        unsafe { XFreeStringList(list) };
        Ok(title)
    }
}

#[cfg(target_os = "linux")]
fn get_x11_targets(conn: &xcb::Connection) -> Result<Vec<VideoSource>> {
    let setup = conn.get_setup();
    let screens = setup.roots();

    let wm_client_list = get_atom(conn, "_NET_CLIENT_LIST")?;
    assert!(wm_client_list != x::ATOM_NONE, "EWMH not supported");

    let atom_net_wm_name = get_atom(conn, "_NET_WM_NAME")?;
    let atom_text = get_atom(conn, "TEXT")?;
    let atom_utf8_string = get_atom(conn, "UTF8_STRING")?;
    let atom_compound_text = get_atom(conn, "COMPOUND_TEXT")?;

    let mut targets = Vec::new();
    for screen in screens {
        let window_list = get_property(conn, screen.root(), wm_client_list, x::ATOM_NONE, 100)?;

        for client in window_list.value::<x::Window>() {
            let cr = get_property(conn, *client, atom_net_wm_name, x::ATOM_STRING, 4096)?;
            if !cr.value::<x::Atom>().is_empty() {
                targets.push(VideoSource::XWindow {
                    id: client.resource_id(),
                    name: String::from_utf8(cr.value().to_vec())
                        .map_err(|_| xcb::Error::Connection(xcb::ConnError::ClosedParseErr))?,
                });
                continue;
            }

            let reply = get_property(conn, *client, x::ATOM_WM_NAME, x::ATOM_ANY, 4096)?;
            let value: &[u8] = reply.value();
            if !value.is_empty() {
                let ttype = reply.r#type();
                let title =
                    if ttype == x::ATOM_STRING || ttype == atom_utf8_string || ttype == atom_text {
                        String::from_utf8(reply.value().to_vec()).unwrap_or(String::from("n/a"))
                    } else if ttype == atom_compound_text {
                        decode_compound_text(conn, value, client, ttype)
                            .map_err(|_| xcb::Error::Connection(xcb::ConnError::ClosedParseErr))?
                    } else {
                        String::from_utf8(reply.value().to_vec()).unwrap_or(String::from("n/a"))
                    };

                targets.push(VideoSource::XWindow {
                    id: client.resource_id(),
                    name: title,
                });
                continue;
            }
            targets.push(VideoSource::XWindow {
                id: client.resource_id(),
                name: "n/a".to_owned(),
            });
        }

        let resources = conn.send_request(&GetScreenResources {
            window: screen.root(),
        });
        let resources = conn.wait_for_reply(resources)?;
        for output in resources.outputs() {
            let info = conn.send_request(&GetOutputInfo {
                output: *output,
                config_timestamp: 0,
            });
            let info = conn.wait_for_reply(info)?;
            if info.connection() == xcb::randr::Connection::Connected {
                let crtc = info.crtc();
                let crtc_info = conn.send_request(&GetCrtcInfo {
                    crtc,
                    config_timestamp: 0,
                });
                let crtc_info = conn.wait_for_reply(crtc_info)?;
                let title = String::from_utf8(info.name().to_vec()).unwrap_or(String::from("n/a"));
                targets.push(VideoSource::XDisplay {
                    name: title,
                    id: screen.root().resource_id(),
                    width: crtc_info.width(),
                    height: crtc_info.height(),
                    x_offset: crtc_info.x(),
                    y_offset: crtc_info.y(),
                });
            }
        }
    }

    Ok(targets)
}

#[derive(Debug)]
enum Event {
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
                let mut _proxy = None;
                let mut _session = None;
                let mut conn = None;
                enum WindowingSystem {
                    Wayland,
                    X11,
                }

                let winsys = if std::env::var("WAYLAND_DISPLAY").is_ok() {
                    WindowingSystem::Wayland
                } else if std::env::var("DISPLAY").is_ok() {
                    conn = Some(xcb::Connection::connect(None).unwrap().0);
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
                            let new_proxy = match Screencast::new().await {
                                Ok(proxy) => proxy,
                                Err(err) => {
                                    error!("Failed to create Screencast proxy");
                                    continue;
                                }
                            };
                            let new_session = match new_proxy.create_session().await {
                                Ok(session) => session,
                                Err(err) => {
                                    error!("Failed to create screencast session");
                                    continue;
                                }
                            };
                            if let Err(err) = new_proxy
                                .select_sources(
                                    &new_session,
                                    CursorMode::Embedded,
                                    SourceType::Monitor | SourceType::Window,
                                    false,
                                    None,
                                    PersistMode::DoNot,
                                )
                                .await
                            {
                                error!("Failed to select source: {err}");
                                continue;
                            }

                            let response = match new_proxy
                                .start(&new_session, None)
                                .await
                            {
                                Ok(resp) => resp,
                                Err(err) => {
                                    error!("Failed to start screencast session: {err}");
                                    continue;
                                }
                            };
                            let response = match response.response() {
                                Ok(resp) => resp,
                                Err(err) => {
                                    error!("Failed to get response: {err}");
                                    continue;
                                }
                            };

                            let Some(stream) = response.streams().first() else {
                                error!("No screencast streams available");
                                continue;
                            };

                            // let fd = new_proxy
                            //     .open_pipe_wire_remote(&new_session)
                            //     .await
                            //     .unwrap()
                            //     .as_raw_fd();

                            event_tx
                                .send(Event::VideosAvailable(vec![VideoSource::PipeWire {
                                    node_id: stream.pipe_wire_node_id(),
                                    // fd,
                                    fd: 0,
                                }]))
                                .unwrap();

                            _proxy = Some(new_proxy);
                            _session = Some(new_session);
                        }
                        (FetchEvent::Fetch, WindowingSystem::X11) => {
                            let Some(xconn) = conn.as_ref() else {
                                error!("No xcb connection available");
                                continue;
                            };

                            let sources = get_x11_targets(xconn).unwrap();
                            event_tx.send(Event::VideosAvailable(sources)).unwrap();
                        }
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
                                Ok(devs) => event_tx.send(Event::AudiosAvailable(devs)).unwrap(),
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
                // TODO
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
                            #[cfg(target_os = "linux")]
                            VideoSource::PipeWire { node_id, .. } => Some(
                                gst::ElementFactory::make("pipewiresrc")
                                    .property("path", node_id.to_string())
                                    .build()?,
                            ),
                            #[cfg(target_os = "linux")]
                            VideoSource::XWindow { id, .. } => Some(
                                gst::ElementFactory::make("ximagesrc")
                                    .property("xid", *id as u64)
                                    .property("use-damage", false)
                                    .build()?,
                            ),
                            #[cfg(target_os = "linux")]
                            VideoSource::XDisplay {
                                id,
                                width,
                                height,
                                x_offset,
                                y_offset,
                                ..
                            } => Some(
                                gst::ElementFactory::make("ximagesrc")
                                    .property("xid", *id as u64)
                                    .property("startx", *x_offset as u32)
                                    .property("starty", *y_offset as u32)
                                    .property("endx", (*x_offset as u32) + (*width as u32) - 1)
                                    .property("endy", (*y_offset as u32) + (*height as u32) - 1)
                                    .property("use-damage", false)
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
                            #[cfg(target_os = "linux")]
                            AudioSource::Pipewire { id, .. } => Some(
                                gst::ElementFactory::make("pipewiresrc")
                                    .property("path", id.to_string())
                                    .build()?,
                            ),
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
                                        if !pipeline_has_finished.load(Ordering::Relaxed) {
                                            event_tx.send(crate::Event::PipelineFinished).unwrap();
                                            pipeline_has_finished.store(true, Ordering::Relaxed);
                                        }
                                    }
                                    pipeline::Event::Error => {
                                        if !pipeline_has_finished.load(Ordering::Relaxed) {
                                            event_tx.send(crate::Event::PipelineFinished).unwrap();
                                            pipeline_has_finished.store(true, Ordering::Relaxed);
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

                    let Some(play_msg) = self.pipeline.as_ref().unwrap().get_play_msg(*addr) else {
                        error!("Could not get stream uri");
                        return Ok(false);
                    };

                    debug!("Sending play message: {play_msg:?}");

                    self.send_packet_to_receiver(Packet::Play(play_msg))?;

                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_cast_started();
                    })?;

                    return Ok(false);
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
