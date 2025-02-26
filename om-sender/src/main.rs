use fcast_lib::models::{self, Header};
use fcast_lib::packet::Packet;
use gst::prelude::*;
use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow};
use gtk4 as gtk;
use log::{debug, info};
use om_sender::signaller::run_server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use std::cell::RefCell;

#[derive(Debug)]
enum Message {
    Play(String),
    Quit,
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

async fn session(rx: async_channel::Receiver<Message>) {
    let mut stream = TcpStream::connect("127.0.0.1:46899").await.unwrap();
    loop {
        tokio::select! {
            packet = read_packet_from_stream(&mut stream) => {
                debug!("{packet:?}");
            }
            msg = rx.recv() => match msg {
                Ok(msg) => {
                    debug!("{msg:?}");
                    match msg {
                        Message::Play(url) => {
                            let packet = Packet::from(
                                models::PlayMessage { container: "todo".to_owned(), url: Some(url), content: None, time: None, speed: None, headers: None }
                            );
                            send_packet(&mut stream, packet).await.unwrap();
                        }
                        Message::Quit => break,
                    }
                }
                Err(err) => panic!("{err}"),
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
    let src = gst::ElementFactory::make("scapsrc")
        .property("perform-internal-preroll", true)
        .build()
        .unwrap();
    // let src = gst::ElementFactory::make("videotestsrc")
    //     .build()
    //     .unwrap();

    // let sink = gst::Bin::default();
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

    let gst_widget = gst_gtk4::RenderWidget::new(&gtksink);
    vbox.append(&gst_widget);

    let label = gtk::Label::new(Some("Position: 00:00:00"));
    vbox.append(&label);

    let (tx, rx) = async_channel::unbounded::<Message>();

    let button = gtk::Button::builder().label("Test").build();
    // let tx_clone = tx.clone();
    button.connect_clicked(move |_| {
        glib::spawn_future_local(glib::clone!(
            // #[strong]
            // tx_clone,
            async move {
                // tx_clone.send(Message::Play("file://".to_owned())).await.unwrap();
            }
        ));
    });
    vbox.append(&button);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("My GTK App")
        .child(&vbox)
        .build();

    window.present();

    let pipeline_weak = pipeline.downgrade();
    let timeout_id = glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let Some(pipeline) = pipeline_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };

        let position = pipeline.query_position::<gst::ClockTime>();
        label.set_text(&format!("Position: {:.0}", position.display()));
        glib::ControlFlow::Continue
    });

    let bus = pipeline.bus().unwrap();

    pipeline
        .set_state(gst::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");

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
                    println!(
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

    let tx_clone = tx.clone();
    om_common::runtime().spawn(async move {
        debug!("Waiting for the producer to connect...");
        let peer_id = prod_peer_rx.await.unwrap();
        debug!("Producer connected peer_id={peer_id}");
        tx_clone
            .send(Message::Play(format!(
                "gstwebrtc://127.0.0.1:8443?peer-id={peer_id}"
            )))
            .await
            .unwrap();
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
        .application_id("org.example.helloworld")
        .build();

    app.connect_activate(build_ui);

    app.run()
}
