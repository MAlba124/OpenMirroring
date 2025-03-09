use gst::prelude::*;
use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow};
use gtk4 as gtk;
use log::{debug, info, trace, warn};
use om_sender::primary::PrimaryView;
use om_sender::select_source::SelectSourceView;
use om_sender::session::session;
use om_sender::signaller::run_server;

use std::cell::RefCell;

use om_sender::{Event, Message};

async fn event_loop(
    mut primary_view: PrimaryView,
    mut event_rx: tokio::sync::mpsc::Receiver<Event>,
    tx: tokio::sync::mpsc::Sender<Message>,
    select_source_tx: tokio::sync::mpsc::Sender<usize>,
    main_view_stack: gtk::Stack,
    select_source_view: SelectSourceView,
    fin_tx: tokio::sync::oneshot::Sender<()>,
) {
    let mut producer_id = None;
    while let Some(event) = event_rx.recv().await {
        debug!("{event:?}");
        match event {
            Event::Quit => break,
            Event::ProducerConnected(id) => producer_id = Some(id),
            Event::Start => {
                // let Some(ref producer_id) = producer_id else {
                //     error!("No producer available for casting");
                //     continue;
                // };
                // tx.send(Message::Play(format!(
                //     // "gstwebrtc://192.168.1.133:8443?peer-id={producer_id}"
                //     "gstwebrtc://127.0.0.1:8443?peer-id={producer_id}"
                // )))
                // .await
                // .unwrap();
                tx.send(Message::Play(primary_view.get_stream_uri()))
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
            Event::SelectSource(idx, sink_type) => {
                select_source_tx.send(idx).await.unwrap();
                main_view_stack.set_visible_child(primary_view.main_widget());
                if sink_type == 0 {
                    primary_view.add_webrtc_sink();
                } else {
                    primary_view.add_hls_sink();
                }
            }
            Event::Packet(packet) => {
                trace!("Unhandled packet: {packet:?}");
            }
        }
    }

    debug!("Quitting");

    primary_view.shutdown();

    fin_tx.send(()).unwrap();
}

fn build_ui(app: &Application) {
    info!("Starting signalling server");
    let (prod_peer_tx, prod_peer_rx) = tokio::sync::oneshot::channel();
    om_common::runtime().spawn(run_server(prod_peer_tx));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<Event>(100);
    let (selected_tx, selected_rx) = tokio::sync::mpsc::channel::<usize>(1);
    let primary_view = om_sender::primary::PrimaryView::new(event_tx.clone(), selected_rx).unwrap();

    let (session_tx, session_rx) = tokio::sync::mpsc::channel::<Message>(100);

    let main_view_stack = gtk::Stack::new();
    let loading_sources_view = om_sender::loading::LoadingSourcesView::new();
    main_view_stack.add_child(loading_sources_view.main_widget());

    let select_source_view = om_sender::select_source::SelectSourceView::new(event_tx.clone());
    main_view_stack.add_child(select_source_view.main_widget());

    main_view_stack.add_child(primary_view.main_widget());

    // **
    // main_view_stack.set_visible_child(primary_view.main_widget());
    // **

    let window = ApplicationWindow::builder()
        .application(app)
        .title("OMSender")
        .child(&main_view_stack)
        .build();

    window.present();

    let bus_watch = primary_view
        .setup_bus_watch(app.downgrade())
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

    let tx_clone = session_tx.clone();
    let pipeline = RefCell::new(Some(primary_view.pipeline.downgrade()));
    let (fin_tx, fin_rx) = tokio::sync::oneshot::channel::<()>();
    glib::spawn_future_local(async move {
        event_loop(
            primary_view,
            event_rx,
            tx_clone,
            selected_tx,
            main_view_stack,
            select_source_view,
            fin_tx,
        )
        .await;
    });

    let bus_watch = RefCell::new(Some(bus_watch));
    let fin_rx = std::sync::Arc::new(std::sync::Mutex::new(Some(fin_rx)));
    app.connect_shutdown(move |_| {
        debug!("Shutting down");

        window.close();

        drop(bus_watch.borrow_mut().take());

        if let Some(pipeline) = pipeline.borrow_mut().take() {
            if let Some(pipeline) = pipeline.upgrade() {
                pipeline.debug_to_dot_file(gst::DebugGraphDetails::ALL, "gstdebug");
                pipeline
                    .set_state(gst::State::Null)
                    .expect("Unable to set the pipeline to the `Null` state");
            }

            // Since the event-loop runs on the main thread and glib does not have a `runtime::block_on`
            // alternative (i think), we need to create a MainLoop that we use to wait for the future
            // spawned below to finish.
            let main_loop = glib::MainLoop::new(None, false); // Lol!

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
                        if let Some(fin_rx) = fin_rx.lock().unwrap().take() {
                            debug!("Waiting for fin signal...");
                            fin_rx.await.unwrap();
                            debug!("Got fin signal")
                        } else {
                            warn!("Missing fin signal receiver");
                        }
                    } else {
                        debug!("Tx was closed, weird");
                    }
                    debug!("Quitting main loop");
                    main_loop.quit();
                }
            ));

            main_loop.run();
        }
    });

    om_common::runtime().spawn(session(session_rx, event_tx));
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
    gst_hlssink3::plugin_register_static().unwrap();
    gst_fmp4::plugin_register_static().unwrap();

    let app = Application::builder()
        .application_id("com.github.malba124.OpenMirroring.om-sender")
        .build();

    app.connect_activate(build_ui);

    app.run()
}
