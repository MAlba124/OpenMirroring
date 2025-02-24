use fcast_iced::dispatcher::Dispatcher;
use fcast_iced::models::{PlayMessage, PlaybackState, PlaybackUpdateMessage};
use fcast_iced::packet::Packet;
use fcast_iced::session::Session;
use fcast_iced::{runtime, Event};
use gst::{prelude::*, SeekFlags};
use log::{debug, error};

use std::cell::RefCell;

use gst::glib::clone;
use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow};
use gtk4 as gtk;

// #[derive(Debug)]
// enum PlaybackEvent {
// }

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[derive(Debug)]
enum GuiEvent {
    Play(PlayMessage),
    Eos,
    Pause,
    Resume,
    Stop,
    SetSpeed(f64),
    Seek(f64),
    SetVolume(f64),
}

async fn event_loop(
    rx: async_channel::Receiver<Event>,
    tx: async_channel::Sender<Event>,
    gui_tx: async_channel::Sender<GuiEvent>,
) {
    let (updates_tx, _) = tokio::sync::broadcast::channel(100);

    while let Ok(event) = rx.recv().await {
        debug!("Got event: {event:?}");
        match event {
            Event::CreateSessionRequest {
                net_stream_mutex,
                id,
            } => {
                debug!("Got CreateSessionRequest id={id}");
                let stream = net_stream_mutex
                    .lock()
                    .unwrap()
                    .take()
                    .expect("Always Some");
                runtime().spawn(Session::new(stream, tx.clone(), id).work(updates_tx.subscribe()));
            }
            Event::Pause => gui_tx.send(GuiEvent::Pause).await.unwrap(),
            Event::Play(play_message) => gui_tx.send(GuiEvent::Play(play_message)).await.unwrap(),
            Event::Resume => gui_tx.send(GuiEvent::Resume).await.unwrap(),
            Event::Stop => gui_tx.send(GuiEvent::Stop).await.unwrap(),
            Event::SetSpeed(set_speed_message) => gui_tx
                .send(GuiEvent::SetSpeed(set_speed_message.speed))
                .await
                .unwrap(),
            Event::Seek(seek_message) => gui_tx
                .send(GuiEvent::Seek(seek_message.time))
                .await
                .unwrap(),
            Event::SetVolume(set_volume_message) => gui_tx
                .send(GuiEvent::SetVolume(set_volume_message.volume))
                .await
                .unwrap(),
            Event::PlaybackUpdate {
                time,
                duration,
                state,
                speed,
            } => {
                let packet = Packet::from(PlaybackUpdateMessage {
                    generation: current_time_millis(),
                    time,
                    duration,
                    state,
                    speed,
                });
                let encoded_packet = packet.encode();
                if updates_tx.receiver_count() > 0 {
                    updates_tx.send(encoded_packet).unwrap();
                }
            } // Event::Playback(_playback_event) => todo!(),
        }
    }
}

fn build_ui(app: &Application) {
    debug!("Building UI");

    let pipeline = gst::Pipeline::new();
    let gtksink = gst::ElementFactory::make("gtk4paintablesink")
        .build()
        .unwrap();

    let playbin = gst::ElementFactory::make("playbin3").build().unwrap();

    let sink = gst::Bin::default();
    let convert = gst::ElementFactory::make("videoconvert").build().unwrap();

    sink.add(&convert).unwrap();
    sink.add(&gtksink).unwrap();
    convert.link(&gtksink).unwrap();

    sink.add_pad(&gst::GhostPad::with_target(&convert.static_pad("sink").unwrap()).unwrap())
        .unwrap();

    playbin.set_property("video-sink", &sink);

    pipeline.add(&playbin).unwrap();

    let video_view = gst_gtk4::RenderWidget::new(&gtksink);

    let label_view = gtk::Label::new(Some("Listening on localhost:46899"));

    let stack = gtk::Stack::new();
    stack.add_named(&label_view, Some("text_view"));
    stack.add_named(&video_view, Some("video_view"));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("OMReceiver")
        .child(&stack)
        .build();

    window.present();

    let (event_tx, event_rx) = async_channel::unbounded::<Event>();
    let (gui_event_tx, gui_event_rx) = async_channel::unbounded::<GuiEvent>();

    let pipeline_weak = pipeline.downgrade();
    let event_tx_clone = event_tx.clone();
    let timeout_id = glib::timeout_add_local(std::time::Duration::from_millis(1000), move || {
        let event_tx = event_tx_clone.clone();
        runtime().block_on(async {
            let Some(pipeline) = pipeline_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            let position: Option<gst::ClockTime> = pipeline.query_position();
            // let speed = pipeline.query_position();
            let duration: Option<gst::ClockTime> = pipeline.query_duration();
            // let speed = pipeline.
            let state = match pipeline.state(gst::ClockTime::NONE).1 {
                gst::State::Paused => PlaybackState::Paused,
                gst::State::Playing => PlaybackState::Playing,
                _ => PlaybackState::Idle,
            };
            event_tx
                .send(Event::PlaybackUpdate {
                    time: position.unwrap_or_default().seconds_f64(),
                    duration: duration.unwrap_or_default().seconds_f64(),
                    state,
                    speed: 1.0,
                })
                .await
                .unwrap();
            glib::ControlFlow::Continue
        })
    });

    let bus = pipeline.bus().unwrap();

    debug!("Setting pipeline to `Ready`");

    pipeline
        .set_state(gst::State::Ready)
        .expect("Unable to set the pipeline to the `Ready` state");
    let _app_weak = app.downgrade(); // HACK: removing this makes the gui not show!?
    let pipeline_weak = pipeline.downgrade();
    let get_clone = gui_event_tx.clone();
    let bus_watch = bus
        .add_watch_local(move |_, msg| {
            use gst::MessageView;

            let Some(pipeline) = pipeline_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            match msg.view() {
                MessageView::Eos(..) => {
                    debug!("Reached EOS");
                    pipeline
                        .set_state(gst::State::Ready)
                        .expect("Unable to set pipeline to `Ready` state");
                    glib::spawn_future_local(glib::clone!(
                        #[strong]
                        get_clone,
                        async move {
                            get_clone.send(GuiEvent::Eos).await.unwrap();
                        }
                    ));
                }
                MessageView::Error(err) => {
                    error!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    pipeline
                        .set_state(gst::State::Null)
                        .expect("Unable to set pipeline to `Null` state");
                    // TODO: notify GUI
                }
                _ => (),
            };

            glib::ControlFlow::Continue
        })
        .expect("Failed to add bus watch");

    runtime().spawn(clone!(
        #[strong]
        event_tx,
        async move {
            Dispatcher::new("127.0.0.1:46899", event_tx)
                .await
                .unwrap()
                .run()
                .await
                .unwrap();
        }
    ));

    runtime().spawn(clone!(
        #[strong]
        event_rx,
        #[strong]
        event_tx,
        #[strong]
        gui_event_tx,
        async move {
            event_loop(event_rx, event_tx, gui_event_tx).await;
        }
    ));

    let stack_clone = stack.clone();
    let video_view_clone = video_view.clone();
    let label_view_clone = label_view.clone();
    let playbin_clone = playbin.clone();
    let pipeline_weak = pipeline.downgrade();
    glib::spawn_future_local(async move {
        while let Ok(event) = gui_event_rx.recv().await {
            let Some(pipeline) = pipeline_weak.upgrade() else {
                panic!("Pipeline = bozo");
            };
            match event {
                GuiEvent::Play(_play_message) => {
                    stack_clone.set_visible_child(&video_view_clone);
                    pipeline
                        .set_state(gst::State::Ready)
                        .expect("Unable to set the pipeline to the `Ready` state");
                    playbin_clone.set_property("uri", &_play_message.url.unwrap());
                    pipeline
                        .set_state(gst::State::Playing)
                        .expect("Unable to set the pipeline to the `Playing` state");
                }
                GuiEvent::Eos => {
                    // stack_clone.set_visible_child(&label_view_clone);
                    debug!("EOS");
                }
                GuiEvent::Pause => {
                    pipeline
                        .set_state(gst::State::Paused)
                        .expect("Unable to set the pipeline to `Pause` state");
                    debug!("Playback paused");
                }
                GuiEvent::Resume => {
                    pipeline
                        .set_state(gst::State::Playing)
                        .expect("Unable to set the pipeline to `Playing` state");
                    debug!("Playback resumed");
                }
                GuiEvent::Stop => {
                    pipeline
                        .set_state(gst::State::Null)
                        .expect("Unable to set the pipeline to `Playing` state");
                    playbin_clone.set_property("uri", "");
                    stack_clone.set_visible_child(&label_view_clone);
                    debug!("Playback stopped");
                }
                GuiEvent::SetVolume(new_volume) => {
                    playbin_clone.set_property("volume", new_volume.clamp(0.0, 1.0));
                }
                GuiEvent::Seek(seek_to) => {
                    if pipeline
                        .seek_simple(
                            SeekFlags::ACCURATE | SeekFlags::FLUSH,
                            gst::ClockTime::from_seconds_f64(seek_to),
                        )
                        .is_err()
                    {
                        error!("Failed to seek to={seek_to}");
                    }
                }
                GuiEvent::SetSpeed(new_speed) => {
                    let Some(position) = pipeline.query_position::<gst::ClockTime>() else {
                        error!("Failed to get playback position");
                        continue;
                    };

                    // https://gstreamer.freedesktop.org/documentation/tutorials/basic/playback-speed.html?gi-language=c
                    if if new_speed > 0.0 {
                        pipeline.seek(
                            new_speed,
                            SeekFlags::ACCURATE | SeekFlags::FLUSH,
                            gst::SeekType::Set,
                            position,
                            gst::SeekType::End,
                            gst::ClockTime::ZERO,
                        )
                    } else {
                        pipeline.seek(
                            new_speed,
                            SeekFlags::ACCURATE | SeekFlags::FLUSH,
                            gst::SeekType::Set,
                            gst::ClockTime::ZERO,
                            gst::SeekType::End,
                            position,
                        )
                    }
                    .is_err()
                    {
                        error!("Failed to set speed new_speed={new_speed}");
                    }
                }
            }
        }
    });

    let timeout_id = RefCell::new(Some(timeout_id));
    let pipeline = RefCell::new(Some(pipeline));
    let bus_watch = RefCell::new(Some(bus_watch));
    app.connect_shutdown(move |_| {
        debug!("Closing window and shutting down gst pipeline");
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
    });
}

fn main() -> glib::ExitCode {
    env_logger::Builder::from_default_env()
        .filter_module("fcast_iced", log::LevelFilter::Debug)
        .init();

    gst::init().unwrap();
    gst_webrtc::plugin_register_static().unwrap();
    gst_rtp::plugin_register_static().unwrap();
    gst_gtk4::plugin_register_static().unwrap();

    let app = Application::builder()
        .application_id("org.example.HelloWorld") // TODO: change
        .build();

    app.connect_activate(build_ui);

    debug!("Starting app");

    app.run()
}
