use gst::prelude::*;
use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow};
use gtk4 as gtk;
use log::{debug, error, trace};
use sender::discovery::discover;
use sender::session::session;
use sender::views::{StateChange, View};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::oneshot;

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;

use sender::{Event, Message};

async fn event_loop(
    mut pipeline: sender::pipeline::Pipeline,
    mut main_view: sender::views::Main,
    mut event_rx: Receiver<Event>,
    event_tx: Sender<Event>,
    session_tx: Sender<Message>,
    select_source_tx: Sender<usize>,
    fin_tx: oneshot::Sender<()>,
) {
    let mut selected_source = false;
    let mut receivers: HashMap<String, Vec<SocketAddr>> = HashMap::new();
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
            }
            Event::Stop => {
                session_tx.send(Message::Stop).await.unwrap();
            }
            Event::EnablePreview => {
                main_view
                    .primary
                    .preview_stack
                    .set_visible_child(&main_view.primary.gst_widget);
            }
            Event::DisablePreview => {
                main_view
                    .primary
                    .preview_stack
                    .set_visible_child(&main_view.primary.preview_disabled_label);
            }
            Event::Sources(sources) => {
                debug!("Available sources: {sources:?}");
                main_view.change_state(StateChange::LoadingSourcesToSelectSources(sources));
            }
            Event::SelectSource(idx) => {
                select_source_tx.send(idx).await.unwrap();
                selected_source = true;
                main_view.change_state(StateChange::SelectSourceToSelectReceiver);
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
                main_view.select_receiver.add_receiver(receiver.name);
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

                main_view.change_state(StateChange::SelectReceiverToConnectingToReceiver);
                session_tx
                    .send(Message::Connect(addresses[0]))
                    .await
                    .unwrap();
            }
            Event::ConnectedToReceiver => {
                debug!("Succesfully connected to receiver");
                main_view.change_state(StateChange::ConnectingToReceiverToPrimary);
            }
        }
    }

    debug!("Quitting");

    if !selected_source {
        debug!("Source is not selected, sending fake");
        select_source_tx.send(0).await.unwrap();
    }

    fin_tx.send(()).unwrap();
}

fn build_ui(app: &Application) {
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<Event>(100);
    let (selected_tx, selected_rx) = tokio::sync::mpsc::channel::<usize>(1);

    let (pipeline, gst_widget) =
        sender::pipeline::Pipeline::new(event_tx.clone(), selected_rx).unwrap();

    let main_view = sender::views::Main::new(event_tx.clone(), gst_widget);

    let (session_tx, session_rx) = tokio::sync::mpsc::channel::<Message>(100);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("OMSender")
        .child(main_view.main_widget())
        .build();

    window.present();

    let bus_watch = pipeline
        .setup_bus_watch(app.downgrade(), event_tx.clone())
        .expect("Failed to add bus watch");

    let tx_clone = session_tx.clone();
    let pipeline_weak = RefCell::new(Some(pipeline.inner.downgrade()));
    let event_tx_clone = event_tx.clone();
    let (fin_tx, fin_rx) = oneshot::channel::<()>();
    glib::spawn_future_local(async move {
        event_loop(
            pipeline,
            main_view,
            event_rx,
            event_tx_clone,
            tx_clone,
            selected_tx,
            fin_tx,
        )
        .await;
    });

    let bus_watch = RefCell::new(Some(bus_watch));
    let fin_rx = std::sync::Arc::new(RefCell::new(Some(fin_rx)));
    app.connect_shutdown(move |_| {
        debug!("Shutting down");

        window.close();

        drop(bus_watch.borrow_mut().take());

        let pipeline = pipeline_weak
            .borrow_mut()
            .take()
            .unwrap()
            .upgrade()
            .unwrap();

        // Since the event-loop runs on the main thread and glib does not have a `runtime::block_on`
        // alternative (i think), we need to create a MainLoop that we use to wait for the future
        // spawned below to finish.
        let main_loop = glib::MainLoop::new(None, false);

        glib::spawn_future_local(glib::clone!(
            #[strong]
            fin_rx,
            #[strong]
            session_tx,
            #[strong]
            main_loop,
            async move {
                if !session_tx.is_closed() {
                    session_tx.send(Message::Quit).await.unwrap();
                    if let Some(fin_rx) = fin_rx.borrow_mut().take() {
                        fin_rx.await.unwrap();
                    }
                }
                main_loop.quit();
            }
        ));

        main_loop.run();

        pipeline
            .set_state(gst::State::Null)
            .expect("Unable to set the pipeline to the `Null` state");
    });

    common::runtime().spawn(session(session_rx, event_tx.clone()));
    common::runtime().spawn(discover(event_tx));
}

fn main() -> glib::ExitCode {
    env_logger::Builder::from_default_env()
        .filter_module("sender", log::LevelFilter::Debug)
        .filter_module("scap", log::LevelFilter::Debug)
        .init();

    gst::init().unwrap();
    scapgst::plugin_register_static().unwrap();
    gst_gtk4::plugin_register_static().unwrap();
    gst_webrtc::plugin_register_static().unwrap();

    let app = Application::builder()
        .application_id("com.github.malba124.OpenMirroring.sender")
        .build();

    app.connect_activate(build_ui);

    app.run()
}
