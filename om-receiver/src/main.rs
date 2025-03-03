use fcast_lib::models::{PlaybackState, PlaybackUpdateMessage};
use fcast_lib::packet::Packet;
use gst::{prelude::*, SeekFlags};
use log::{debug, error, warn};
use om_common::runtime;
use om_receiver::dispatcher::Dispatcher;
use om_receiver::session::Session;
use om_receiver::{AtomicF64, Event, GuiEvent};

use std::cell::RefCell;
use std::net::Ipv4Addr;
use std::sync::Arc;

use gst::glib::WeakRef;
use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow};
use gtk4 as gtk;

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

async fn event_loop(
    mut rx: tokio::sync::mpsc::Receiver<Event>,
    tx: tokio::sync::mpsc::Sender<Event>,
    gui_tx: tokio::sync::mpsc::Sender<GuiEvent>,
) {
    let (updates_tx, _) = tokio::sync::broadcast::channel(100);

    while let Some(event) = rx.recv().await {
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
            }
        }
    }
}

fn create_pipeline() -> (gst::Pipeline, gst::Element, gst_gtk4::RenderWidget) {
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

    (pipeline, playbin, video_view)
}

fn setup_timeout(
    pipeline_weak: WeakRef<gst::Pipeline>,
    event_tx: tokio::sync::mpsc::Sender<Event>,
    playback_speed: Arc<AtomicF64>,
) -> glib::SourceId {
    glib::timeout_add_local(std::time::Duration::from_millis(1000), move || {
        let event_tx = event_tx.clone();
        runtime().block_on(async {
            let Some(pipeline) = pipeline_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            let position: Option<gst::ClockTime> = pipeline.query_position();
            let duration: Option<gst::ClockTime> = pipeline.query_duration();
            let speed = playback_speed.load(std::sync::atomic::Ordering::SeqCst);
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
                    speed,
                })
                .await
                .unwrap();
            glib::ControlFlow::Continue
        })
    })
}

fn on_bus_msg(
    msg: &gst::Message,
    pipeline_weak: &WeakRef<gst::Pipeline>,
    gui_event_tx: &tokio::sync::mpsc::Sender<GuiEvent>,
) -> Result<(), glib::ControlFlow> {
    use gst::MessageView;

    let Some(pipeline) = pipeline_weak.upgrade() else {
        return Err(glib::ControlFlow::Break);
    };

    match msg.view() {
        MessageView::Eos(..) => {
            debug!("Reached EOS");
            pipeline
                .set_state(gst::State::Ready)
                .map_err(|_| glib::ControlFlow::Break)?;
            let get_clone = gui_event_tx.clone();
            runtime().spawn(async move {
                get_clone.send(GuiEvent::Eos).await.unwrap();
            });
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
                .map_err(|_| glib::ControlFlow::Break)?;
            // TODO: notify GUI
        }
        _ => (),
    };

    Ok(())
}

async fn on_event(
    pipeline_weak: WeakRef<gst::Pipeline>,
    video_view: gst_gtk4::RenderWidget,
    label_view: gtk::Label,
    playbin: gst::Element,
    mut gui_event_rx: tokio::sync::mpsc::Receiver<GuiEvent>,
    stack: gtk::Stack,
    playback_speed: Arc<AtomicF64>,
) -> Result<(), gst::StateChangeError> {
    while let Some(event) = gui_event_rx.recv().await {
        let Some(pipeline) = pipeline_weak.upgrade() else {
            panic!("Pipeline = bozo");
        };
        match event {
            GuiEvent::Play(play_message) => {
                stack.set_visible_child(&video_view);
                pipeline.set_state(gst::State::Ready)?;
                playbin.set_property("uri", play_message.url.unwrap());
                pipeline.set_state(gst::State::Playing)?;
            }
            GuiEvent::Eos => {
                // stack_clone.set_visible_child(&label_view_clone);
                debug!("EOS");
            }
            GuiEvent::Pause => {
                pipeline.set_state(gst::State::Paused)?;
                debug!("Playback paused");
            }
            GuiEvent::Resume => {
                pipeline.set_state(gst::State::Playing)?;
                debug!("Playback resumed");
            }
            GuiEvent::Stop => {
                pipeline.set_state(gst::State::Null)?;
                playbin.set_property("uri", "");
                stack.set_visible_child(&label_view);
                debug!("Playback stopped");
            }
            GuiEvent::SetVolume(new_volume) => {
                playbin.set_property("volume", new_volume.clamp(0.0, 1.0));
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
                    // TODO: send error message
                }
            }
            GuiEvent::SetSpeed(new_speed) => {
                let Some(position) = pipeline.query_position::<gst::ClockTime>() else {
                    error!("Failed to get playback position");
                    // TODO: send error message
                    continue;
                };

                // https://gstreamer.freedesktop.org/documentation/tutorials/basic/playback-speed.html?gi-language=c
                let res = if new_speed > 0.0 {
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
                };
                if res.is_err() {
                    error!("Failed to set speed new_speed={new_speed}");
                    // TODO: send error message
                } else {
                    debug!("Successfully set playback speed to {new_speed}");
                    playback_speed.store(new_speed, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
    }

    Ok(())
}

fn build_ui(app: &Application) {
    let mut ips: Vec<Ipv4Addr> = Vec::new();
    for iface in pnet_datalink::interfaces() {
        for ip in iface.ips {
            match ip {
                ipnetwork::IpNetwork::V4(v4) => ips.push(v4.ip()),
                ipnetwork::IpNetwork::V6(v6) => warn!("Found IPv6 address ({v6:?}), ignoring"),
            }
        }
    }

    let (pipeline, playbin, video_view) = create_pipeline();
    let label_view = gtk::Label::new(Some(&format!("Listening on {ips:?}:46899")));
    let stack = gtk::Stack::new();
    stack.add_child(&label_view);
    stack.add_child(&video_view);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("OMReceiver")
        .child(&stack)
        .build();

    window.present();

    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<Event>(100);
    let (gui_event_tx, gui_event_rx) = tokio::sync::mpsc::channel::<GuiEvent>(100);

    let playback_speed = Arc::new(AtomicF64::new(1.0));
    let timeout_id = setup_timeout(
        pipeline.downgrade(),
        event_tx.clone(),
        Arc::clone(&playback_speed),
    );

    let bus = pipeline.bus().unwrap();

    pipeline
        .set_state(gst::State::Ready)
        .expect("Unable to set the pipeline to the `Ready` state");

    let _app_weak = app.downgrade(); // HACK: removing this makes the gui not show!?
    let pipeline_weak = pipeline.downgrade();
    let get_clone = gui_event_tx.clone();
    let bus_watch = bus
        .add_watch_local(move |_, msg| {
            if let Err(err) = on_bus_msg(msg, &pipeline_weak, &get_clone) {
                return err;
            }
            glib::ControlFlow::Continue
        })
        .expect("Failed to add bus watch");

    let event_tx_clone = event_tx.clone();
    runtime().spawn(async move {
        Dispatcher::new("0.0.0.0:46899", event_tx_clone)
            .await
            .unwrap()
            .run()
            .await
            .unwrap();
    });

    runtime().spawn(async move {
        event_loop(event_rx, event_tx, gui_event_tx).await;
    });

    let stack_clone = stack.clone();
    let video_view_clone = video_view.clone();
    let label_view_clone = label_view.clone();
    let playbin_clone = playbin.clone();
    let pipeline_weak = pipeline.downgrade();
    glib::spawn_future_local(async move {
        on_event(
            pipeline_weak,
            video_view_clone,
            label_view_clone,
            playbin_clone,
            gui_event_rx,
            stack_clone,
            playback_speed,
        )
        .await
        .unwrap();
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
        .filter_module("om_receiver", log::LevelFilter::Debug)
        .init();

    gst::init().unwrap();
    gst_webrtc::plugin_register_static().unwrap();
    gst_rtp::plugin_register_static().unwrap();
    gst_gtk4::plugin_register_static().unwrap();

    let app = Application::builder()
        .application_id("com.github.malba124.OpenMirroring.om-receiver")
        .build();

    app.connect_activate(build_ui);

    app.run()
}
