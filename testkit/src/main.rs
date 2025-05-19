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

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    rc::Rc,
    thread::{self, JoinHandle, sleep},
    time::Duration,
};

use anyhow::Result;
use crossbeam_channel::Receiver;
use fcast_lib::packet::Packet;
use log::{debug, error};
use mdns_sd::ServiceEvent;
use slint::{Model, Weak};

slint::include_modules!();

enum Event {
    Quit,
    ConnectReceiver(String),
    ReceiverAvailable {
        name: String,
        addresses: Vec<SocketAddr>,
    },
    DisconnectReceiver,
    SendPacket(Packet),
}

struct Application {
    ui_weak: Weak<MainWindow>,
    receivers: Vec<(ReceiverItem, SocketAddr)>,
}

impl Application {
    pub fn new(ui_weak: Weak<MainWindow>) -> Self {
        Self {
            ui_weak,
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

    // fn set_all_receivers_connectable(&mut self) {
    //     for r in &mut self.receivers {
    //         if r.0.state != ReceiverState::Connectable {
    //             r.0.state = ReceiverState::Connectable;
    //         }
    //     }
    // }

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

    fn push_message(&self, direction: MessageDirection, msg: String) -> Result<()> {
        self.ui_weak.upgrade_in_event_loop(move |ui| {
            let messages_model = ui.get_messages();
            let messages_model = messages_model
                .as_any()
                .downcast_ref::<slint::ReverseModel<slint::VecModel<Message>>>()
                .unwrap();
            let messages_model = messages_model.source_model();
            messages_model.push(Message {
                direction,
                text: msg.into(),
                time: format!("{}", chrono::Local::now().format("%H:%M:%S")).into(),
            })
        })?;

        Ok(())
    }

    fn add_receiver(&mut self, name: String, addrs: Vec<SocketAddr>) -> Result<()> {
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

    pub fn run_event_loop(mut self, event_rx: Receiver<Event>) -> Result<()> {
        // Set the message model to be what we want so no errors occur later
        self.ui_weak.upgrade_in_event_loop(|ui| {
            let model = slint::VecModel::from(Vec::<Message>::new());
            let reverse_model = slint::ReverseModel::new(model);
            ui.set_messages(Rc::new(reverse_model).into());
        })?;

        let mdns = mdns_sd::ServiceDaemon::new()?;
        let mdns_receiver = mdns.browse("_fcast._tcp.local.")?;

        // TODO: move session to own thread
        let mut session_stream_jh = None::<JoinHandle<Result<TcpStream, std::io::Error>>>;
        let mut session_stream = None::<TcpStream>;

        loop {
            match event_rx.try_recv() {
                Ok(event) => match event {
                    Event::Quit => break,
                    Event::ConnectReceiver(name) => {
                        if let Some(idx) = self.receivers_contains(&name) {
                            self.receivers[idx].0.state = ReceiverState::Connecting;

                            self.push_message(
                                MessageDirection::Info,
                                format!("Attempting to connect to {}", self.receivers[idx].0.name),
                            )?;

                            let addr = self.receivers[idx].1;
                            session_stream_jh =
                                Some(thread::spawn(move || TcpStream::connect(addr)));

                            self.update_receivers_in_ui()?;
                        } else {
                            error!("No receiver `{name}` available");
                        }
                    }
                    Event::ReceiverAvailable { name, addresses } => {
                        self.add_receiver(name, addresses)?;
                    }
                    Event::DisconnectReceiver => {
                        let _ = session_stream_jh.take();
                        let _ = session_stream.take();

                        for r in &mut self.receivers {
                            if r.0.state == ReceiverState::Connected
                                || r.0.state == ReceiverState::Connecting
                            {
                                r.0.state = ReceiverState::Connectable;
                                self.update_receivers_in_ui()?;
                                break;
                            }
                        }

                        self.push_message(
                            MessageDirection::Info,
                            "Disconnected from receiver".to_owned(),
                        )?;
                    }
                    Event::SendPacket(packet) => {
                        if let Some(stream) = session_stream.as_mut() {
                            self.push_message(MessageDirection::Out, format!("{packet:?}"))?;
                            stream.write_all(&packet.encode())?;
                        }
                    }
                },
                Err(err) => {
                    if err == crossbeam_channel::TryRecvError::Disconnected {
                        return Err(err.into());
                    }
                }
            }

            match mdns_receiver.try_recv() {
                Ok(event) => match event {
                    ServiceEvent::ServiceResolved(service_info) => {
                        let port = service_info.get_port();
                        let addrs = service_info
                            .get_addresses()
                            .iter()
                            .map(|a| SocketAddr::new(*a, port))
                            .collect::<Vec<SocketAddr>>();
                        let mut name = service_info.get_fullname().to_owned();
                        if let Some(stripped) = name.strip_suffix("._fcast._tcp.local.") {
                            name = stripped.to_owned();
                        }
                        debug!("Receiver available: {}", name);
                        self.add_receiver(name, addrs)?;
                    }
                    ServiceEvent::ServiceRemoved(_, mut fullname) => {
                        if let Some(stripped) = fullname.strip_suffix("._fcast._tcp.local.") {
                            fullname = stripped.to_owned();
                        }
                        if let Some(idx) = self.receivers_contains(&fullname) {
                            debug!("Receiver unavailable: {fullname}");
                            self.receivers.remove(idx);
                            self.update_receivers_in_ui()?;
                        }
                    }
                    _ => (),
                },
                Err(err) => {
                    if err == flume::TryRecvError::Disconnected {
                        return Err(err.into());
                    }
                }
            }

            if let Some(jh) = session_stream_jh.take_if(|jh| jh.is_finished()) {
                match jh.join().unwrap() {
                    Ok(stream) => {
                        debug!("Successfully connected to receiver");

                        self.push_message(
                            MessageDirection::Info,
                            "Successfully connected to receiver".to_owned(),
                        )?;

                        for r in &mut self.receivers {
                            if r.0.state == ReceiverState::Connecting {
                                r.0.state = ReceiverState::Connected;
                                break;
                            }
                        }

                        // self.ui_weak.upgrade_in_event_loop(|ui| {
                        //     ui.invoke_receiver_connected();
                        // })?;

                        self.update_receivers_in_ui()?;

                        stream.set_nonblocking(true)?;
                        session_stream = Some(stream);
                    }
                    Err(err) => {
                        error!("Failed to connect to receiver: {err}");
                        self.push_message(
                            MessageDirection::Info,
                            format!("Failed to connect to receiver: {err}"),
                        )?;
                    }
                }
            }

            'out: {
                if let Some(stream) = session_stream.as_mut() {
                    let mut header_buf = [0u8; 5];
                    if let Err(err) = stream.read_exact(&mut header_buf) {
                        if err.kind() != std::io::ErrorKind::WouldBlock {
                            return Err(err.into());
                        }
                        break 'out;
                    }

                    let header = fcast_lib::models::Header::decode(header_buf);

                    let mut body_string = String::new();

                    if header.size > 0 {
                        let mut body_buf = vec![0; header.size as usize];
                        // TODO: this can fail badly, need to read the whole packet
                        if let Err(err) = stream.read_exact(&mut body_buf) {
                            if err.kind() != std::io::ErrorKind::WouldBlock {
                                return Err(err.into());
                            }
                            error!("Failed to read from receiver (cuz nonblocking)");
                            break 'out;
                        }
                        body_string = String::from_utf8(body_buf)?;
                    }

                    let packet = Packet::decode(header, &body_string)?;
                    self.push_message(MessageDirection::In, format!("{packet:?}"))?;
                    if packet == Packet::Ping {
                        let packet = Packet::Pong;
                        self.push_message(MessageDirection::Out, format!("{packet:?}"))?;
                        stream.write_all(&packet.encode())?;
                    }
                }
            }

            sleep(Duration::from_millis(25));
        }

        mdns.shutdown()?;

        Ok(())
    }
}

fn main() {
    env_logger::Builder::from_default_env()
        .filter_module("testkit", log::LevelFilter::Debug)
        .filter_module("mdns_sd", log::LevelFilter::Debug)
        .init();

    let ui = MainWindow::new().unwrap();

    let (event_tx, event_rx) = crossbeam_channel::bounded(10);

    let ui_weak = ui.as_weak();

    {
        let event_tx = event_tx.clone();
        ui.on_connect_receiver(move |name| {
            event_tx
                .send(Event::ConnectReceiver(name.to_string()))
                .unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_disconnect_receiver(move || {
            event_tx.send(Event::DisconnectReceiver).unwrap();
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
                .send(Event::ReceiverAvailable {
                    name: name.to_string(),
                    addresses: vec![parsed_addr],
                })
                .unwrap();
        });
    }

    macro_rules! simple_packet {
        ($ui:expr, $on:ident, $event_tx:expr, $packet:ident) => {{
            let event_tx = $event_tx.clone();
            $ui.$on(move || {
                event_tx.send(Event::SendPacket(Packet::$packet)).unwrap();
            });
        }};
    }

    simple_packet!(ui, on_send_none, event_tx, None);
    simple_packet!(ui, on_send_pause, event_tx, Pause);
    simple_packet!(ui, on_send_resume, event_tx, Resume);
    simple_packet!(ui, on_send_stop, event_tx, Stop);
    simple_packet!(ui, on_send_ping, event_tx, Ping);
    simple_packet!(ui, on_send_pong, event_tx, Pong);

    {
        let event_tx = event_tx.clone();
        ui.on_send_play(move |container, url, content, time, speed| {
            use fcast_lib::models::PlayMessage;
            if container.is_empty() {
                error!("`container` is required but it's empty");
            }

            let message = PlayMessage {
                container: container.to_string(),
                url: if url.is_empty() {
                    None
                } else {
                    Some(url.to_string())
                },
                content: if content.is_empty() {
                    None
                } else {
                    Some(content.to_string())
                },
                time: time.parse::<f64>().ok(),
                speed: speed.parse::<f64>().ok(),
                headers: None,
            };

            event_tx
                .send(Event::SendPacket(Packet::Play(message)))
                .unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_send_seek(move |time| {
            if let Ok(time) = time.parse::<f64>() {
                let message = fcast_lib::models::SeekMessage { time };
                event_tx
                    .send(Event::SendPacket(Packet::Seek(message)))
                    .unwrap();
            } else {
                error!("`{time}` is not a valid timestamp");
            }
        });
    }

    fn current_time_millis() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    {
        let event_tx = event_tx.clone();
        ui.on_send_playback_update(move |time, duration, state, speed| {
            let Ok(time) = time.parse::<f64>() else {
                error!("`{time}` is not a valid timestamp");
                return;
            };
            let Ok(duration) = duration.parse::<f64>() else {
                error!("`{duration}` is not a valid timestamp");
                return;
            };
            let Ok(speed) = speed.parse::<f64>() else {
                error!("`{speed}` is not a valid float");
                return;
            };

            let state = match state.as_str() {
                "Idle" => fcast_lib::models::PlaybackState::Idle,
                "Playing" => fcast_lib::models::PlaybackState::Playing,
                "Paused" => fcast_lib::models::PlaybackState::Paused,
                _ => {
                    error!("Invalid state: {state}");
                    return;
                }
            };

            let message = fcast_lib::models::PlaybackUpdateMessage {
                generation: current_time_millis(),
                time,
                duration,
                state,
                speed,
            };

            event_tx
                .send(Event::SendPacket(Packet::PlaybackUpdate(message)))
                .unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_send_volume_update(move |volume| {
            if let Ok(volume) = volume.parse::<f64>() {
                let message = fcast_lib::models::VolumeUpdateMessage {
                    generation: current_time_millis(),
                    volume,
                };
                event_tx
                    .send(Event::SendPacket(Packet::VolumeUpdate(message)))
                    .unwrap();
            } else {
                error!("`{volume}` is not a valid volume");
            }
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_send_set_volume(move |volume| {
            if let Ok(volume) = volume.parse::<f64>() {
                let message = fcast_lib::models::SetVolumeMessage { volume };
                event_tx
                    .send(Event::SendPacket(Packet::SetVolume(message)))
                    .unwrap();
            } else {
                error!("`{volume}` is not a valid volume");
            }
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_send_playback_error(move |err_message| {
            let message = fcast_lib::models::PlaybackErrorMessage {
                message: err_message.to_string(),
            };
            event_tx
                .send(Event::SendPacket(Packet::PlaybackError(message)))
                .unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_send_set_speed(move |speed| {
            if let Ok(speed) = speed.parse::<f64>() {
                let message = fcast_lib::models::SetSpeedMessage { speed };
                event_tx
                    .send(Event::SendPacket(Packet::SetSpeed(message)))
                    .unwrap();
            } else {
                error!("`{speed}` is not a valid speed");
            }
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_send_version(move |version| {
            if let Ok(version) = version.parse::<u64>() {
                let message = fcast_lib::models::VersionMessage { version };
                event_tx
                    .send(Event::SendPacket(Packet::Version(message)))
                    .unwrap();
            } else {
                error!("`{version}` is not a valid version");
            }
        });
    }

    let app_thread_jh = std::thread::spawn(move || {
        Application::new(ui_weak).run_event_loop(event_rx).unwrap();
    });

    ui.run().unwrap();

    event_tx.send(Event::Quit).unwrap();

    app_thread_jh.join().unwrap();
}
