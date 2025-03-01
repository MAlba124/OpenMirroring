use fcast_lib::models::{self, Header};
use fcast_lib::packet::Packet;
use gst::prelude::*;
use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow};
use gtk4 as gtk;
use log::{debug, error, info, warn};
use om_sender::signaller::run_server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use std::cell::RefCell;

const GST_WEBRTC_MIME_TYPE: &str = "application/x-gst-webrtc";

type ProducerId = String;

#[derive(Debug)]
enum Message {
    Play(String),
    Quit,
    Stop,
}

#[derive(Debug)]
enum Event {
    ProducerConnected(ProducerId),
    Start,
    Stop,
    SelectInput,
    EnablePreview,
    DisablePreview,
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

fn build_ui(app: &Application) {
    info!("Starting signalling server");
    let (prod_peer_tx, prod_peer_rx) = tokio::sync::oneshot::channel();
    om_common::runtime().spawn(run_server(prod_peer_tx));

    let tee = gst::ElementFactory::make("tee").build().unwrap();
    let gtksink = gst::ElementFactory::make("gtk4paintablesink")
        .name("gtksink")
        .build()
        .unwrap();
    let preview_queue = gst::ElementFactory::make("queue")
        .name("preview_queue")
        .build()
        .unwrap();
    // let src = gst::ElementFactory::make("scapsrc")
    //     .property("perform-internal-preroll", true)
    //     .build()
    //     .unwrap();
    let src = gst::ElementFactory::make("videotestsrc")
        .build()
        .unwrap();

    let preview_convert = gst::ElementFactory::make("videoconvert")
        .name("preview_convert")
        .build()
        .unwrap();
    let webrtcsink = gst::ElementFactory::make("webrtcsink")
        .property("signalling-server-host", "127.0.0.1")
        .property("signalling-server-port", 8443u32)
        .build()
        .unwrap();
    let webrtcsink_queue = gst::ElementFactory::make("queue")
        .name("webrtcsink_queue")
        .build()
        .unwrap();
    let webrtcsink_convert = gst::ElementFactory::make("videoconvert")
        .name("webrtcsink_convert")
        .build()
        .unwrap();

    let pipeline = gst::Pipeline::new();
    pipeline
        .add_many([
            &src,
            &tee,
            &preview_queue,
            &preview_convert,
            &gtksink,
            &webrtcsink_queue,
            &webrtcsink_convert,
            &webrtcsink,
        ])
        .unwrap();

    gst::Element::link_many([&src, &tee]).unwrap();
    gst::Element::link_many([&preview_queue, &preview_convert, &gtksink]).unwrap();
    gst::Element::link_many([&webrtcsink_queue, &webrtcsink_convert, &webrtcsink]).unwrap();

    let tee_preview_pad = tee.request_pad_simple("src_%u").unwrap();
    let queue_preview_pad = preview_queue.static_pad("sink").unwrap();
    tee_preview_pad.link(&queue_preview_pad).unwrap();

    let tee_webrtcsink_pad = tee.request_pad_simple("src_%u").unwrap();
    let queue_webrtcsink_pad = webrtcsink_queue.static_pad("sink").unwrap();
    tee_webrtcsink_pad.link(&queue_webrtcsink_pad).unwrap();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let preview_stack = gtk::Stack::new();
    let preview_disabled_label = gtk::Label::new(Some("Preview disabled"));
    let no_preview_label = gtk::Label::new(Some("Preview not available"));
    let gst_widget = gst_gtk4::RenderWidget::new(&gtksink);

    preview_stack.add_child(&no_preview_label);
    preview_stack.add_child(&gst_widget);
    preview_stack.add_child(&preview_disabled_label);

    vbox.append(&preview_stack);

    let (tx, rx) = tokio::sync::mpsc::channel::<Message>(100);
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<Event>(100);

    let select_button = gtk::Button::builder().label("Select input").build();
    let event_tx_clone = event_tx.clone();
    select_button.connect_clicked(move |_| {
        glib::spawn_future_local(glib::clone!(
            #[strong]
            event_tx_clone,
            async move {
                event_tx_clone.send(Event::SelectInput).await.unwrap();
            }
        ));
    });
    vbox.append(&select_button);

    let start_button = gtk::Button::builder().label("Start casting").build();
    let event_tx_clone = event_tx.clone();
    start_button.connect_clicked(move |_| {
        glib::spawn_future_local(glib::clone!(
            #[strong]
            event_tx_clone,
            async move {
                event_tx_clone.send(Event::Start).await.unwrap();
            }
        ));
    });
    vbox.append(&start_button);

    let stop_button = gtk::Button::builder().label("Stop casting").build();
    let event_tx_clone = event_tx.clone();
    stop_button.connect_clicked(move |_| {
        glib::spawn_future_local(glib::clone!(
            #[strong]
            event_tx_clone,
            async move {
                event_tx_clone.send(Event::Stop).await.unwrap();
            }
        ));
    });
    vbox.append(&stop_button);

    let enable_preview = gtk::CheckButton::builder()
        .active(true)
        .label("Enable preview")
        .build();

    let event_tx_clone = event_tx.clone();
    enable_preview.connect_toggled(move |btn| {
        let new = btn.property::<bool>("active");
        glib::spawn_future_local(
            glib::clone!(
                #[strong] event_tx_clone,
                async move {
                    match new {
                        true => event_tx_clone.send(Event::EnablePreview).await.unwrap(),
                        false => event_tx_clone.send(Event::DisablePreview).await.unwrap(),
                    }
                }
            )
        );
    });

    vbox.append(&enable_preview);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("OMSender")
        .child(&vbox)
        .build();

    window.present();

    let pipeline_weak = pipeline.downgrade();
    let timeout_id = glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let Some(_pipeline) = pipeline_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        glib::ControlFlow::Continue
    });

    let bus = pipeline.bus().unwrap();

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

    let pipeline_weak = pipeline.downgrade();
    let tx_clone = tx.clone();
    let preview_stack_clone = preview_stack.clone();
    let gst_widget_clone = gst_widget.clone();
    let preview_disabled_label_clone = preview_disabled_label.clone();
    glib::spawn_future_local(async move {
        let mut producer_id = None;
        while let Some(event) = event_rx.recv().await {
            let Some(pipeline) = pipeline_weak.upgrade() else {
                panic!("Pipeline = bozo");
            };
            match event {
                Event::ProducerConnected(id) => producer_id = Some(id),
                Event::SelectInput => {
                    pipeline
                        .set_state(gst::State::Playing)
                        .expect("Unable to set the pipeline to the `Playing` state");
                    preview_stack_clone.set_visible_child(&gst_widget_clone);
                }
                Event::Start => {
                    let Some(ref producer_id) = producer_id else {
                        error!("No producer available for casting");
                        continue;
                    };
                    tx_clone
                        .send(Message::Play(format!(
                            "gstwebrtc://127.0.0.1:8443?peer-id={producer_id}"
                        )))
                        .await
                        .unwrap();
                }
                Event::Stop => {
                    tx_clone.send(Message::Stop).await.unwrap();
                }
                Event::EnablePreview => {
                    preview_stack_clone.set_visible_child(&gst_widget_clone);
                }
                Event::DisablePreview => {
                    preview_stack_clone.set_visible_child(&preview_disabled_label_clone);
                }
            }
        }
    });

    let timeout_id = RefCell::new(Some(timeout_id));
    let pipeline = RefCell::new(Some(pipeline));
    let bus_watch = RefCell::new(Some(bus_watch));
    app.connect_shutdown(move |_| {
        debug!("Shutting down");

        window.close();

        drop(bus_watch.borrow_mut().take());
        if let Some(pipeline) = pipeline.borrow_mut().take() {
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
    });

    om_common::runtime().spawn(session(rx));
}

fn main() -> glib::ExitCode {
    env_logger::Builder::from_default_env()
        .filter_module("om_sender", log::LevelFilter::Debug)
        .init();

    gst::init().unwrap();
    scapgst::plugin_register_static().unwrap();
    gst_gtk4::plugin_register_static().unwrap();
    gst_rtp::plugin_register_static().unwrap();
    gst_webrtc::plugin_register_static().unwrap();

    let app = Application::builder()
        .application_id("com.github.malba124.OpenMirroring.om-sender")
        .build();

    app.connect_activate(build_ui);

    app.run()
}
