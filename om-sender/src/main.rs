use fcast_lib::models::{self, Header};
use fcast_lib::packet::Packet;
use gst::prelude::*;
use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow};
use gtk4 as gtk;
use log::{debug, error, info, warn};
use om_sender::primary::PrimaryView;
use om_sender::select_source::SelectSourceView;
use om_sender::signaller::run_server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use std::cell::RefCell;

use om_sender::Event;

const GST_WEBRTC_MIME_TYPE: &str = "application/x-gst-webrtc";

#[derive(Debug)]
enum Message {
    Play(String),
    Quit,
    Stop,
}

const HEADER_BUFFER_SIZE: usize = 5;

async fn read_packet_from_stream(stream: &mut TcpStream) -> Result<Packet, tokio::io::Error> {
    let mut header_buf: [u8; HEADER_BUFFER_SIZE] = [0; HEADER_BUFFER_SIZE];

    stream.read_exact(&mut header_buf).await?;

    let header = Header::decode(header_buf);

    let mut body_string = String::new();

    if header.size > 0 {
        let mut body_buf = vec![0; header.size as usize];
        stream.read_exact(&mut body_buf).await?;
        body_string =
            String::from_utf8(body_buf).map_err(|e| tokio::io::Error::other(e.to_string()))?;
    }

    Packet::decode(header, &body_string).map_err(|e| tokio::io::Error::other(e.to_string()))
}

async fn send_packet(stream: &mut TcpStream, packet: Packet) -> Result<(), tokio::io::Error> {
    let bytes = packet.encode();
    stream.write_all(&bytes).await?;
    Ok(())
}

async fn session(mut rx: tokio::sync::mpsc::Receiver<Message>) {
    let mut stream = TcpStream::connect("127.0.0.1:46899").await.unwrap();
    // let mut stream = TcpStream::connect("192.168.1.23:46899").await.unwrap();
    loop {
        tokio::select! {
            packet = read_packet_from_stream(&mut stream) => {
                match packet {
                    Ok(packet) => match packet {
                        Packet::Ping => {
                            send_packet(&mut stream, Packet::Pong).await.unwrap();
                        }
                        _ => warn!("Unhandled packet: {packet:?}"),
                    },
                    Err(err) => panic!("{err}"),
                }
            }
            msg = rx.recv() => match msg {
                Some(msg) => {
                    debug!("{msg:?}");
                    match msg {
                        Message::Play(url) => {
                            let packet = Packet::from(
                                models::PlayMessage {
                                    container: GST_WEBRTC_MIME_TYPE.to_owned(),
                                    url: Some(url),
                                    content: None,
                                    time: None,
                                    speed: None,
                                    headers: None
                                }
                            );
                            send_packet(&mut stream, packet).await.unwrap();
                        }
                        Message::Quit => break,
                        Message::Stop => send_packet(&mut stream, Packet::Stop).await.unwrap(),
                    }
                }
                None => panic!("rx closed"), // TODO
            }
        }
    }

    debug!("Session terminated");
}

async fn event_loop(
    primary_view: PrimaryView,
    mut event_rx: tokio::sync::mpsc::Receiver<Event>,
    tx: tokio::sync::mpsc::Sender<Message>,
    select_source_tx: tokio::sync::mpsc::Sender<usize>,
    main_view_stack: gtk::Stack,
    select_source_view: SelectSourceView,
) {
    let mut producer_id = None;
    while let Some(event) = event_rx.recv().await {
        match event {
            Event::ProducerConnected(id) => producer_id = Some(id),
            Event::Start => {
                let Some(ref producer_id) = producer_id else {
                    error!("No producer available for casting");
                    continue;
                };
                tx.send(Message::Play(format!(
                    // "gstwebrtc://192.168.1.133:8443?peer-id={producer_id}"
                    "gstwebrtc://127.0.0.1:8443?peer-id={producer_id}"
                )))
                .await
                .unwrap();
            }
            Event::Stop => {
                tx.send(Message::Stop).await.unwrap();
            }
            Event::EnablePreview => {
                primary_view
                    .preview_stack
                    .set_visible_child(&primary_view.gst_widget);
            }
            Event::DisablePreview => {
                primary_view
                    .preview_stack
                    .set_visible_child(&primary_view.preview_disabled_label);
            }
            Event::Sources(sources) => {
                debug!("Available sources: {sources:?}");
                let l = gtk::StringList::new(
                    &sources.iter().map(|s| s.as_str()).collect::<Vec<&str>>(),
                );
                select_source_view.drop_down.set_model(Some(&l));
                main_view_stack.set_visible_child(select_source_view.main_widget());
            }
            Event::SelectSource(idx) => {
                select_source_tx.send(idx).await.unwrap();
                main_view_stack.set_visible_child(primary_view.main_widget());
            }
        }
    }
}

fn build_ui(app: &Application) {
    info!("Starting signalling server");
    let (prod_peer_tx, prod_peer_rx) = tokio::sync::oneshot::channel();
    om_common::runtime().spawn(run_server(prod_peer_tx));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<Event>(100);
    let (selected_tx, selected_rx) = tokio::sync::mpsc::channel::<usize>(1);
    let primary_view = om_sender::primary::PrimaryView::new(event_tx.clone(), selected_rx).unwrap();

    // TODO: Rename
    let (tx, rx) = tokio::sync::mpsc::channel::<Message>(100);

    let main_view_stack = gtk::Stack::new();
    let loading_sources_view = om_sender::loading::LoadingSourcesView::new();
    main_view_stack.add_child(loading_sources_view.main_widget());

    let select_source_view = om_sender::select_source::SelectSourceView::new(event_tx.clone());
    main_view_stack.add_child(select_source_view.main_widget());

    main_view_stack.add_child(primary_view.main_widget());

    let window = ApplicationWindow::builder()
        .application(app)
        .title("OMSender")
        .child(&main_view_stack)
        .build();

    window.present();

    let pipeline_weak = primary_view.pipeline.downgrade();
    let timeout_id = glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let Some(_pipeline) = pipeline_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        glib::ControlFlow::Continue
    });

    let pipeline_weak = primary_view.pipeline.downgrade();
    let _ = std::thread::spawn(move || {
        let Some(pipeline) = pipeline_weak.upgrade() else {
            panic!("No pipeline");
        };
        pipeline.set_state(gst::State::Playing).unwrap();
    });

    // TODO: Move to otherplace
    let bus = primary_view.pipeline.bus().unwrap();
    let app_weak = app.downgrade();
    let bus_watch = bus
        .add_watch_local(move |_, msg| {
            use gst::MessageView;

            let Some(app) = app_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            match msg.view() {
                MessageView::Eos(..) => app.quit(),
                MessageView::Error(err) => {
                    error!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    app.quit();
                }
                _ => (),
            };

            glib::ControlFlow::Continue
        })
        .expect("Failed to add bus watch");

    let event_tx_clone = event_tx.clone();
    om_common::runtime().spawn(async move {
        debug!("Waiting for the producer to connect...");
        let peer_id = prod_peer_rx.await.unwrap();
        debug!("Producer connected peer_id={peer_id}");
        event_tx_clone
            .send(Event::ProducerConnected(peer_id))
            .await
            .unwrap();
    });

    let tx_clone = tx.clone();
    let pipeline = RefCell::new(Some(primary_view.pipeline.downgrade()));
    glib::spawn_future_local(async move {
        event_loop(
            primary_view,
            event_rx,
            tx_clone,
            selected_tx,
            main_view_stack,
            select_source_view,
        )
        .await;
    });

    let timeout_id = RefCell::new(Some(timeout_id));
    let bus_watch = RefCell::new(Some(bus_watch));
    app.connect_shutdown(move |_| {
        debug!("Shutting down");

        window.close();

        drop(bus_watch.borrow_mut().take());

        if let Some(pipeline) = pipeline.borrow_mut().take() {
            if let Some(pipeline) = pipeline.upgrade() {
                pipeline
                    .set_state(gst::State::Null)
                    .expect("Unable to set the pipeline to the `Null` state");
            }

            if let Some(timeout_id) = timeout_id.borrow_mut().take() {
                timeout_id.remove();
            }

            om_common::runtime().block_on(async {
                if !tx.is_closed() {
                    tx.send(Message::Quit).await.unwrap();
                } else {
                    debug!("Tx was closed, weird");
                }
            });
        }
    });

    om_common::runtime().spawn(session(rx));
}

fn main() -> glib::ExitCode {
    env_logger::Builder::from_default_env()
        .filter_module("om_sender", log::LevelFilter::Debug)
        .init();

    gst::init().unwrap();
    scap_gstreamer::plugin_register_static().unwrap();
    gst_gtk4::plugin_register_static().unwrap();
    gst_rtp::plugin_register_static().unwrap();
    gst_webrtc::plugin_register_static().unwrap();

    let app = Application::builder()
        .application_id("com.github.malba124.OpenMirroring.om-sender")
        .build();

    app.connect_activate(build_ui);

    app.run()
}
