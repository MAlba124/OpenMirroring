use gst::prelude::*;
use log::{debug, error, trace};
use sender::discovery::discover;
use sender::session::session;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::{self, oneshot};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;

use sender::{Event, Message};

slint::include_modules!();

async fn event_loop(
    mut pipeline: sender::pipeline::Pipeline,
    ui: slint::Weak<MainWindow>,
    mut event_rx: Receiver<Event>,
    event_tx: Sender<Event>,
    session_tx: Sender<Message>,
    select_source_tx: Sender<usize>,
    fin_tx: oneshot::Sender<()>,
) {
    let mut selected_source = false;
    let mut receivers: HashMap<String, Vec<SocketAddr>> = HashMap::new();
    let mut receiver_connected_to = String::new();
    let mut receiver_connecting_to = String::new();
    while let Some(event) = event_rx.recv().await {
        match event {
            Event::Quit => break,
            Event::ProducerConnected(id) => {
                debug!("Got producer peer id: {id}");
                pipeline.set_producer_id(id);
            }
            Event::Start => {
                if let Some(play_msg) = pipeline.get_play_msg() {
                    session_tx.send(play_msg).await.unwrap();
                } else {
                    error!("Could not get stream uri");
                }

                ui.upgrade_in_event_loop(|ui| {
                    ui.set_starting_cast(false);
                    ui.set_casting(true);
                })
                .unwrap();
            }
            Event::Stop => {
                session_tx.send(Message::Stop).await.unwrap();
            }
            Event::Sources(sources) => {
                debug!("Available sources: {sources:?}");
                ui.upgrade_in_event_loop(move |ui| {
                    let model = Rc::new(slint::VecModel::<slint::SharedString>::from(
                        sources
                            .iter()
                            .map(|s| s.into())
                            .into_iter()
                            .collect::<Vec<slint::SharedString>>(),
                    ));
                    ui.set_sources_model(model.into());
                })
                .unwrap();
            }
            Event::SelectSource(idx) => {
                select_source_tx.send(idx).await.unwrap();
                selected_source = true;
            }
            Event::Packet(packet) => {
                trace!("Unhandled packet: {packet:?}");
            }
            Event::HlsServerAddr { port } => pipeline.set_server_port(port),
            Event::HlsStreamReady => (),
            Event::ReceiverAvailable(receiver) => {
                if receivers
                    .insert(receiver.name.clone(), receiver.addresses)
                    .is_some()
                {
                    debug!("Receiver dup {}", receiver.name);
                }

                let mut receivers_vec = receivers
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<String>>();
                receivers_vec.sort();
                let receiver_connected_to = receiver_connected_to.clone();
                let receiver_connecting_to = receiver_connecting_to.clone();
                ui.upgrade_in_event_loop(move |ui| {
                    let receiver_connected_to = &receiver_connected_to;
                    let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                        receivers_vec
                            .iter()
                            .map(|name| ReceiverItem {
                                name: name.into(),
                                connected: name == receiver_connected_to,
                                connecting: *name == receiver_connecting_to,
                            })
                            .into_iter(),
                    ));
                    ui.set_receivers_model(model.into());
                })
                .unwrap();
            }
            Event::SelectReceiver(receiver) => {
                let Some(addresses) = receivers.get(&receiver) else {
                    error!("No receiver with id {receiver}");
                    continue;
                };

                if receiver.starts_with("OpenMirroring") {
                    pipeline.add_webrtc_sink(event_tx.clone()).unwrap();
                } else {
                    pipeline.add_hls_sink(event_tx.clone()).unwrap();
                }

                receiver_connecting_to = receiver;

                session_tx
                    .send(Message::Connect(addresses[0]))
                    .await
                    .unwrap();

                let mut receivers_vec = receivers
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<String>>();
                receivers_vec.sort();
                let receiver_connected_to = receiver_connected_to.clone();
                let receiver_connecting_to = receiver_connecting_to.clone();
                ui.upgrade_in_event_loop(move |ui| {
                    let receiver_connected_to = &receiver_connected_to;
                    let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                        receivers_vec
                            .iter()
                            .map(|name| ReceiverItem {
                                name: name.into(),
                                connected: name == receiver_connected_to,
                                connecting: *name == receiver_connecting_to,
                            })
                            .into_iter(),
                    ));
                    ui.set_receiver_is_connecting(true);
                    ui.set_receiver_is_connected(false);
                    ui.set_receivers_model(model.into());
                })
                .unwrap();
            }
            Event::ConnectedToReceiver => {
                debug!("Succesfully connected to receiver");
                receiver_connected_to = receiver_connecting_to;
                receiver_connecting_to = String::new();

                let mut receivers_vec = receivers
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<String>>();
                receivers_vec.sort();
                let receiver_connected_to = receiver_connected_to.clone();
                let receiver_connecting_to = receiver_connecting_to.clone();
                ui.upgrade_in_event_loop(move |ui| {
                    let receiver_connected_to = &receiver_connected_to;
                    let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                        receivers_vec
                            .iter()
                            .map(|name| ReceiverItem {
                                name: name.into(),
                                connected: name == receiver_connected_to,
                                connecting: *name == receiver_connecting_to,
                            })
                            .into_iter(),
                    ));
                    ui.set_receiver_is_connecting(false);
                    ui.set_receiver_is_connected(true);
                    ui.set_receivers_model(model.into());
                })
                .unwrap();
            }
        }
    }

    debug!("Quitting");

    if !selected_source {
        debug!("Source is not selected, sending fake");
        select_source_tx.send(0).await.unwrap();
    }

    pipeline.inner.set_state(gst::State::Null).unwrap();

    fin_tx.send(()).unwrap();
}

fn main() {
    env_logger::Builder::from_default_env()
        .filter_module("sender", common::default_log_level())
        .filter_module("scap", common::default_log_level())
        .init();

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .require_opengl_es()
        .select()
        .unwrap();

    gst::init().unwrap();
    scapgst::plugin_register_static().unwrap();
    gst_webrtc::plugin_register_static().unwrap();

    let (event_tx, event_rx) = sync::mpsc::channel::<Event>(100);
    let (selected_tx, selected_rx) = sync::mpsc::channel::<usize>(1);
    let (session_tx, session_rx) = sync::mpsc::channel::<Message>(100);
    let (fin_tx, fin_rx) = oneshot::channel::<()>();

    let ui = MainWindow::new().unwrap();
    slint::set_xdg_app_id("com.github.malba124.OpenMirroring.sender").unwrap();

    let new_frame_cb = |app: MainWindow, new_frame| {
        app.set_preview_frame(new_frame);
    };

    let pipeline =
        sender::pipeline::Pipeline::new(&ui, event_tx.clone(), selected_rx, new_frame_cb).unwrap();

    let bus_watch = pipeline.setup_bus_watch(event_tx.clone()).unwrap();

    common::runtime().spawn(session(session_rx, event_tx.clone()));
    common::runtime().spawn(discover(event_tx.clone()));
    common::runtime().spawn(event_loop(
        pipeline,
        ui.as_weak(),
        event_rx,
        event_tx.clone(),
        session_tx.clone(),
        selected_tx,
        fin_tx,
    ));

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

    ui.run().unwrap();

    drop(bus_watch);

    common::runtime().block_on(async move {
        if !session_tx.is_closed() {
            session_tx.send(Message::Quit).await.unwrap();
            fin_rx.await.unwrap();
        }
    });
}
