use log::{debug, error, trace};
use sender::discovery::discover;
use sender::pipeline::SlintOpenGLSink;
use sender::session::session;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::{self, oneshot};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use sender::{pipeline, Event, Message};

slint::include_modules!();

type GstEglContext = Arc<Mutex<Option<(gst_gl::GLContext, gst_gl_egl::GLDisplayEGL)>>>;

struct Application {
    pipeline: pipeline::Pipeline,
    ui_weak: slint::Weak<MainWindow>,
    event_tx: Sender<Event>,
    session_tx: Sender<Message>,
    select_source_tx: Sender<usize>,
    selected_source: bool,
    receivers: HashMap<String, Vec<SocketAddr>>,
    appsink: gst::Element,
    gst_egl_context: GstEglContext,
}

impl Application {
    pub async fn new(
        ui_weak: slint::Weak<MainWindow>,
        event_tx: Sender<Event>,
        session_tx: Sender<Message>,
        appsink: gst::Element,
        gst_egl_context: GstEglContext,
    ) -> Self {
        let (select_source_tx, pipeline) = Self::new_pipeline(
            &ui_weak,
            event_tx.clone(),
            appsink.clone(),
            Arc::clone(&gst_egl_context),
        )
        .await;

        Self {
            pipeline,
            ui_weak,
            event_tx,
            session_tx,
            select_source_tx,
            selected_source: false,
            receivers: HashMap::new(),
            appsink,
            gst_egl_context,
        }
    }

    async fn new_pipeline(
        ui_weak: &slint::Weak<MainWindow>,
        event_tx: Sender<Event>,
        appsink: gst::Element,
        gst_egl_context: GstEglContext,
    ) -> (Sender<usize>, pipeline::Pipeline) {
        let (selected_tx, selected_rx) = sync::mpsc::channel::<usize>(1);

        let (pipeline_tx, pipeline_rx) = oneshot::channel();
        ui_weak
            .upgrade_in_event_loop({
                move |_ui| {
                    let pipeline =
                        pipeline::Pipeline::new(event_tx, selected_rx, appsink, gst_egl_context)
                            .unwrap();
                    if pipeline_tx.send(pipeline).is_err() {
                        panic!("Failed to send pipeline");
                    }
                }
            })
            .unwrap();

        (selected_tx, pipeline_rx.await.unwrap())
    }

    // TODO: gracefully handle errors
    pub async fn run_event_loop(
        &mut self,
        mut event_rx: Receiver<Event>,
        fin_tx: oneshot::Sender<()>,
    ) {
        let mut receiver_connected_to = String::new();
        let mut receiver_connecting_to = String::new();
        while let Some(event) = event_rx.recv().await {
            match event {
                Event::Quit => break,
                Event::Start => {
                    let Some(play_msg) = self.pipeline.get_play_msg().await else {
                        error!("Could not get stream uri");
                        continue;
                    };

                    self.session_tx.send(play_msg).await.unwrap();
                    self.ui_weak
                        .upgrade_in_event_loop(|ui| {
                            ui.set_starting_cast(false);
                            ui.set_casting(true);
                        })
                        .unwrap();
                }
                Event::Stop => {
                    self.session_tx.send(Message::Stop).await.unwrap();
                }
                Event::Sources(sources) => {
                    debug!("Available sources: {sources:?}");
                    self.ui_weak
                        .upgrade_in_event_loop(move |ui| {
                            let model = Rc::new(slint::VecModel::<slint::SharedString>::from(
                                sources
                                    .iter()
                                    .map(|s| s.into())
                                    .collect::<Vec<slint::SharedString>>(),
                            ));
                            ui.set_sources_model(model.into());
                        })
                        .unwrap();
                }
                Event::SelectSource(idx) => {
                    self.select_source_tx.send(idx).await.unwrap();
                    self.selected_source = true;

                    if receiver_connected_to.starts_with("OpenMirroring") {
                        self.pipeline
                            .add_webrtc_sink()
                            .await
                            .unwrap();
                    } else if !receiver_connected_to.is_empty() {
                        self.pipeline
                            .add_hls_sink()
                            .await
                            .unwrap();
                    }

                    self.ui_weak
                        .upgrade_in_event_loop(|ui| {
                            ui.set_has_source(true);
                        })
                        .unwrap();
                }
                Event::Packet(packet) => {
                    trace!("Unhandled packet: {packet:?}");
                }
                Event::ReceiverAvailable(receiver) => {
                    if self
                        .receivers
                        .insert(receiver.name.clone(), receiver.addresses)
                        .is_some()
                    {
                        debug!("Receiver dup {}", receiver.name);
                    }

                    let mut receivers_vec = self.receivers.keys().cloned().collect::<Vec<String>>();
                    receivers_vec.sort();
                    let receiver_connected_to = receiver_connected_to.clone();
                    let receiver_connecting_to = receiver_connecting_to.clone();
                    self.ui_weak
                        .upgrade_in_event_loop(move |ui| {
                            let receiver_connected_to = &receiver_connected_to;
                            let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                                receivers_vec.iter().map(|name| ReceiverItem {
                                    name: name.into(),
                                    connected: name == receiver_connected_to,
                                    connecting: *name == receiver_connecting_to,
                                }),
                            ));
                            ui.set_receivers_model(model.into());
                        })
                        .unwrap();
                }
                Event::SelectReceiver(receiver) => {
                    let Some(addresses) = self.receivers.get(&receiver) else {
                        error!("No receiver with id {receiver}");
                        continue;
                    };

                    if receiver.starts_with("OpenMirroring") {
                        self.pipeline
                            .add_webrtc_sink()
                            .await
                            .unwrap();
                    } else {
                        self.pipeline
                            .add_hls_sink()
                            .await
                            .unwrap();
                    }

                    receiver_connecting_to = receiver;

                    self.session_tx
                        .send(Message::Connect(addresses[0]))
                        .await
                        .unwrap();

                    let mut receivers_vec = self.receivers.keys().cloned().collect::<Vec<String>>();
                    receivers_vec.sort();
                    let receiver_connected_to = receiver_connected_to.clone();
                    let receiver_connecting_to = receiver_connecting_to.clone();
                    self.ui_weak
                        .upgrade_in_event_loop(move |ui| {
                            let receiver_connected_to = &receiver_connected_to;
                            let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                                receivers_vec.iter().map(|name| ReceiverItem {
                                    name: name.into(),
                                    connected: name == receiver_connected_to,
                                    connecting: *name == receiver_connecting_to,
                                }),
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

                    let mut receivers_vec = self.receivers.keys().cloned().collect::<Vec<String>>();
                    receivers_vec.sort();
                    let receiver_connected_to = receiver_connected_to.clone();
                    let receiver_connecting_to = receiver_connecting_to.clone();
                    self.ui_weak
                        .upgrade_in_event_loop(move |ui| {
                            let receiver_connected_to = &receiver_connected_to;
                            let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                                receivers_vec.iter().map(|name| ReceiverItem {
                                    name: name.into(),
                                    connected: name == receiver_connected_to,
                                    connecting: *name == receiver_connecting_to,
                                }),
                            ));
                            ui.set_receiver_is_connecting(false);
                            ui.set_receiver_is_connected(true);
                            ui.set_receivers_model(model.into());
                        })
                        .unwrap();
                }
                Event::DisconnectReceiver => {
                    self.session_tx.send(Message::Disconnect).await.unwrap();
                    self.pipeline.remove_transmission_sink().await;
                    receiver_connected_to.clear();
                    let mut receivers_vec = self.receivers.keys().cloned().collect::<Vec<String>>();
                    receivers_vec.sort();
                    self.ui_weak
                        .upgrade_in_event_loop(move |ui| {
                            let model = Rc::new(slint::VecModel::<ReceiverItem>::from_iter(
                                receivers_vec.iter().map(|name| ReceiverItem {
                                    name: name.into(),
                                    connected: false,
                                    connecting: false,
                                }),
                            ));
                            ui.set_receiver_is_connecting(false);
                            ui.set_receiver_is_connected(false);
                            ui.set_receivers_model(model.into());
                        })
                        .unwrap();
                }
                Event::ChangeSource => {
                    if !self.selected_source {
                        self.select_source_tx.send(0).await.unwrap();
                    }

                    self.pipeline.shutdown().await;

                    let (new_select_srouce_tx, new_pipeline) = Self::new_pipeline(
                        &self.ui_weak,
                        self.event_tx.clone(),
                        self.appsink.clone(),
                        Arc::clone(&self.gst_egl_context),
                    )
                    .await;

                    self.pipeline = new_pipeline;
                    self.select_source_tx = new_select_srouce_tx;
                    self.selected_source = false;

                    self.ui_weak
                        .upgrade_in_event_loop(|ui| {
                            ui.set_has_source(false);
                            ui.set_sources_model(
                                Rc::new(slint::VecModel::<slint::SharedString>::default()).into(),
                            );
                        })
                        .unwrap();
                }
            }
        }

        debug!("Quitting");

        if !self.selected_source {
            debug!("Source is not selected, sending fake");
            self.select_source_tx.send(0).await.unwrap();
        }

        self.pipeline.shutdown().await;

        fin_tx.send(()).unwrap();
    }
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
    let (session_tx, session_rx) = sync::mpsc::channel::<Message>(100);
    let (fin_tx, fin_rx) = oneshot::channel::<()>();

    // This sink is used in every consecutively created pipelines
    let mut slint_sink = SlintOpenGLSink::new();
    let slint_appsink = slint_sink.video_sink();
    let gst_egl_context = Arc::new(Mutex::new(
        None::<(gst_gl::GLContext, gst_gl_egl::GLDisplayEGL)>,
    ));

    let ui = MainWindow::new().unwrap();
    slint::set_xdg_app_id("com.github.malba124.OpenMirroring.sender").unwrap();

    ui.window()
        .set_rendering_notifier({
            let gst_egl_context = Arc::clone(&gst_egl_context);
            let ui_weak = ui.as_weak();

            let new_frame_cb = |ui: MainWindow, new_frame| {
                ui.set_preview_frame(new_frame);
            };

            move |state, graphics_api| match state {
                slint::RenderingState::RenderingSetup => {
                    let ui_weak = ui_weak.clone();
                    let mut gst_egl_context = gst_egl_context.lock().unwrap();
                    *gst_egl_context = Some(slint_sink.connect(
                        graphics_api,
                        Box::new(move || {
                            ui_weak
                                .upgrade_in_event_loop(move |ui| {
                                    ui.window().request_redraw();
                                })
                                .ok();
                        }),
                    ));
                }
                slint::RenderingState::BeforeRendering => {
                    if let Some(next_frame) = slint_sink.fetch_next_frame() {
                        new_frame_cb(ui_weak.unwrap(), next_frame);
                    }
                }
                slint::RenderingState::RenderingTeardown => {
                    slint_sink.deactivate_and_pause();
                }
                _ => (),
            }
        })
        .unwrap();

    common::runtime().spawn(session(session_rx, event_tx.clone()));
    common::runtime().spawn(discover(event_tx.clone()));
    common::runtime().spawn({
        let ui_weak = ui.as_weak();
        let event_tx = event_tx.clone();
        let session_tx = session_tx.clone();
        async move {
            let mut app = Application::new(
                ui_weak,
                event_tx,
                session_tx,
                slint_appsink,
                gst_egl_context,
            )
            .await;
            app.run_event_loop(event_rx, fin_tx).await;
        }
    });

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
        let event_tx = event_tx.clone();
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

    {
        let event_tx = event_tx.clone();
        ui.on_disconnect_receiver(move || {
            event_tx.blocking_send(Event::DisconnectReceiver).unwrap();
        });
    }

    {
        ui.on_change_source(move || {
            event_tx.blocking_send(Event::ChangeSource).unwrap();
        });
    }

    ui.run().unwrap();

    common::runtime().block_on(async move {
        if !session_tx.is_closed() {
            session_tx.send(Message::Quit).await.unwrap();
            fin_rx.await.unwrap();
        }
    });
}
