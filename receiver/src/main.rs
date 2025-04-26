use anyhow::{bail, Result};
use common::runtime;
use common::video_preview::software::SlintSwSink;
use fcast_lib::models::PlaybackUpdateMessage;
use fcast_lib::packet::Packet;
use gst::{prelude::*, SeekFlags};
use log::{debug, error, warn};
use receiver::dispatcher::Dispatcher;
use receiver::pipeline::Pipeline;
use receiver::session::Session;
use receiver::{AtomicF64, Event, GuiEvent};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;

use std::cell::RefCell;
use std::net::Ipv4Addr;
// use std::sync::{mpsc, Arc};

// use gtk::prelude::*;
// use gtk::{glib, Application, ApplicationWindow};
// use gtk4 as gtk;

slint::include_modules!();

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

// async fn event_loop(
//     mut rx: tokio::sync::mpsc::Receiver<Event>,
//     tx: tokio::sync::mpsc::Sender<Event>,
//     gui_tx: tokio::sync::mpsc::Sender<GuiEvent>,
// ) {
//     let (updates_tx, _) = tokio::sync::broadcast::channel(100);

//     while let Some(event) = rx.recv().await {
//         debug!("Got event: {event:?}");
//         match event {
//             Event::CreateSessionRequest { stream, id } => {
//                 debug!("Got CreateSessionRequest id={id}");
//                 runtime().spawn(Session::new(stream, tx.clone(), id).work(updates_tx.subscribe()));
//             }
//             Event::Pause => gui_tx.send(GuiEvent::Pause).await.unwrap(),
//             Event::Play(play_message) => gui_tx.send(GuiEvent::Play(play_message)).await.unwrap(),
//             Event::Resume => gui_tx.send(GuiEvent::Resume).await.unwrap(),
//             Event::Stop => gui_tx.send(GuiEvent::Stop).await.unwrap(),
//             Event::SetSpeed(set_speed_message) => gui_tx
//                 .send(GuiEvent::SetSpeed(set_speed_message.speed))
//                 .await
//                 .unwrap(),
//             Event::Seek(seek_message) => gui_tx
//                 .send(GuiEvent::Seek(seek_message.time))
//                 .await
//                 .unwrap(),
//             Event::SetVolume(set_volume_message) => gui_tx
//                 .send(GuiEvent::SetVolume(set_volume_message.volume))
//                 .await
//                 .unwrap(),
//             Event::PlaybackUpdate {
//                 time,
//                 duration,
//                 state,
//                 speed,
//             } => {
//                 let packet = Packet::from(PlaybackUpdateMessage {
//                     generation: current_time_millis(),
//                     time,
//                     duration,
//                     state,
//                     speed,
//                 });
//                 let encoded_packet = packet.encode();
//                 if updates_tx.receiver_count() > 0 {
//                     updates_tx.send(encoded_packet).unwrap();
//                 }
//             }
//         }
//     }
// }

// async fn gui_event_loop(
//     video_view: receiver::video::VideoView,
//     label_view: gtk::Label,
//     mut gui_event_rx: tokio::sync::mpsc::Receiver<GuiEvent>,
//     stack: gtk::Stack,
//     playback_speed: Arc<AtomicF64>,
// ) -> Result<(), gst::StateChangeError> {
//     while let Some(event) = gui_event_rx.recv().await {
//         match event {
//             GuiEvent::Play(play_message) => {
//                 stack.set_visible_child(video_view.main_widget());
//                 video_view.pipeline.set_state(gst::State::Ready)?;
//                 video_view
//                     .playbin
//                     .set_property("uri", play_message.url.unwrap());
//                 video_view.pipeline.set_state(gst::State::Playing)?;
//             }
//             GuiEvent::Eos => {
//                 // stack_clone.set_visible_child(&label_view_clone);
//                 debug!("EOS");
//             }
//             GuiEvent::Pause => {
//                 video_view.pipeline.set_state(gst::State::Paused)?;
//                 debug!("Playback paused");
//             }
//             GuiEvent::Resume => {
//                 video_view.pipeline.set_state(gst::State::Playing)?;
//                 debug!("Playback resumed");
//             }
//             GuiEvent::Stop => {
//                 video_view.pipeline.set_state(gst::State::Null)?;
//                 video_view.playbin.set_property("uri", "");
//                 stack.set_visible_child(&label_view);
//                 debug!("Playback stopped");
//             }
//             GuiEvent::SetVolume(new_volume) => {
//                 video_view
//                     .playbin
//                     .set_property("volume", new_volume.clamp(0.0, 1.0));
//             }
//             GuiEvent::Seek(seek_to) => {
//                 if video_view
//                     .pipeline
//                     .seek_simple(
//                         SeekFlags::ACCURATE | SeekFlags::FLUSH,
//                         gst::ClockTime::from_seconds_f64(seek_to),
//                     )
//                     .is_err()
//                 {
//                     error!("Failed to seek to={seek_to}");
//                     // TODO: send error message
//                 }
//             }
//             GuiEvent::SetSpeed(new_speed) => {
//                 let Some(position) = video_view.pipeline.query_position::<gst::ClockTime>() else {
//                     error!("Failed to get playback position");
//                     // TODO: send error message
//                     continue;
//                 };

//                 // https://gstreamer.freedesktop.org/documentation/tutorials/basic/playback-speed.html?gi-language=c
//                 let res = if new_speed > 0.0 {
//                     video_view.pipeline.seek(
//                         new_speed,
//                         SeekFlags::ACCURATE | SeekFlags::FLUSH,
//                         gst::SeekType::Set,
//                         position,
//                         gst::SeekType::End,
//                         gst::ClockTime::ZERO,
//                     )
//                 } else {
//                     video_view.pipeline.seek(
//                         new_speed,
//                         SeekFlags::ACCURATE | SeekFlags::FLUSH,
//                         gst::SeekType::Set,
//                         gst::ClockTime::ZERO,
//                         gst::SeekType::End,
//                         position,
//                     )
//                 };
//                 if res.is_err() {
//                     error!("Failed to set speed new_speed={new_speed}");
//                     // TODO: send error message
//                 } else {
//                     debug!("Successfully set playback speed to {new_speed}");
//                     playback_speed.store(new_speed, std::sync::atomic::Ordering::SeqCst);
//                 }
//             }
//         }
//     }

//     Ok(())
// }

// fn build_ui(app: &Application) {
//     let mut ips: Vec<Ipv4Addr> = Vec::new();
//     for ip in common::net::get_all_ip_addresses() {
//         match ip {
//             std::net::IpAddr::V4(v4) => ips.push(v4),
//             std::net::IpAddr::V6(v6) => warn!("Found IPv6 address ({v6:?}), ignoring"),
//         }
//     }

//     let video_view = receiver::video::VideoView::new().unwrap();
//     let label_view = gtk::Label::new(Some(&format!("Listening on {ips:?}:46899")));
//     let stack = gtk::Stack::new();
//     stack.add_child(&label_view);
//     stack.add_child(video_view.main_widget());

//     let window = ApplicationWindow::builder()
//         .application(app)
//         .title("OMReceiver")
//         .child(&stack)
//         .build();

//     window.present();

//     let (event_tx, event_rx) = tokio::sync::mpsc::channel::<Event>(100);
//     let (gui_event_tx, gui_event_rx) = tokio::sync::mpsc::channel::<GuiEvent>(100);

//     let playback_speed = Arc::new(AtomicF64::new(1.0));
//     let timeout_id = video_view.setup_timeout(event_tx.clone(), Arc::clone(&playback_speed));

//     let _app_weak = app.downgrade(); // HACK: removing this makes the gui not show!?
//     let bus_watch = video_view.setup_bus_watch(gui_event_tx.clone());

//     let event_tx_clone = event_tx.clone();
//     runtime().spawn(async move {
//         Dispatcher::new(event_tx_clone)
//             .await
//             .unwrap()
//             .run()
//             .await
//             .unwrap();
//     });

//     runtime().spawn(async move {
//         event_loop(event_rx, event_tx, gui_event_tx).await;
//     });

//     let stack_clone = stack.clone();
//     let label_view_clone = label_view.clone();
//     let pipeline_weak = video_view.pipeline.downgrade();
//     glib::spawn_future_local(async move {
//         gui_event_loop(
//             video_view,
//             label_view_clone,
//             gui_event_rx,
//             stack_clone,
//             playback_speed,
//         )
//         .await
//         .unwrap();
//     });

//     let timeout_id = RefCell::new(Some(timeout_id));
//     let pipeline_weak = RefCell::new(Some(pipeline_weak));
//     let bus_watch = RefCell::new(Some(bus_watch));
//     app.connect_shutdown(move |_| {
//         debug!("Closing window and shutting down gst pipeline");
//         window.close();

//         drop(bus_watch.borrow_mut().take());
//         if let Some(pipeline_weak) = pipeline_weak.borrow_mut().take() {
//             if let Some(pipeline) = pipeline_weak.upgrade() {
//                 pipeline
//                     .set_state(gst::State::Null)
//                     .expect("Unable to set the pipeline to the `Null` state");
//             }
//         }

//         if let Some(timeout_id) = timeout_id.borrow_mut().take() {
//             timeout_id.remove();
//         }
//     });
// }

async fn event_loop(
    mut event_rx: Receiver<Event>,
    event_tx: Sender<Event>,
    ui_weak: slint::Weak<MainWindow>,
    fin_tx: oneshot::Sender<()>,
) -> Result<()> {
    let (updates_tx, _) = tokio::sync::broadcast::channel(10);

    let new_frame_cb = |ui: MainWindow, new_frame| {
        ui.set_preview_frame(new_frame);
    };
    let preview = SlintSwSink::new(ui_weak.clone(), new_frame_cb)?;

    let pipeline = Pipeline::new(preview)?;

    while let Some(event) = event_rx.recv().await {
        match event {
            Event::CreateSessionRequest { stream, id } => {
                debug!("Got CreateSessionRequest id={id}");
                runtime()
                    .spawn(Session::new(stream, event_tx.clone(), id).run(updates_tx.subscribe()));
            }
            Event::Pause => pipeline.pause()?,
            Event::Play(play_message) => {
                pipeline.set_playback_uri(&play_message.url.unwrap())?;
                pipeline.play_or_resume()?;
                ui_weak.upgrade_in_event_loop(|ui| {
                    ui.set_playing(true);
                })?
            }
            Event::Resume => pipeline.play_or_resume()?,
            Event::Stop => {
                pipeline.stop()?;
                ui_weak.upgrade_in_event_loop(|ui| {
                    ui.set_playing(false);
                })?;
            }
            Event::SetSpeed(set_speed_message) => pipeline.set_speed(set_speed_message.speed)?,
            Event::Seek(seek_message) => pipeline.seek(seek_message.time)?,
            Event::SetVolume(set_volume_message) => pipeline.set_volume(set_volume_message.volume),
            Event::PlaybackUpdate {
                time,
                duration,
                state,
                speed,
            } => debug!("playback update"),
            Event::Quit => break,
        }
    }

    if fin_tx.send(()).is_err() {
        bail!("Failed to send fin");
    }

    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_module("receiver", common::default_log_level())
        .init();

    gst::init()?;

    let mut ips: Vec<Ipv4Addr> = Vec::new();
    for ip in common::net::get_all_ip_addresses() {
        match ip {
            std::net::IpAddr::V4(v4) => ips.push(v4),
            std::net::IpAddr::V6(v6) => warn!("Found IPv6 address ({v6:?}), ignoring"),
        }
    }

    let (event_tx, event_rx) = mpsc::channel::<Event>(100);
    let (fin_tx, fin_rx) = oneshot::channel::<()>();

    let ui = MainWindow::new()?;
    slint::set_xdg_app_id("com.github.malba124.OpenMirroring.receiver")?;

    common::runtime().spawn(event_loop(event_rx, event_tx.clone(), ui.as_weak(), fin_tx));

    {
        let event_tx = event_tx.clone();
        runtime().spawn(async move {
            Dispatcher::new(event_tx)
                .await
                .unwrap()
                .run()
                .await
                .unwrap();
        });
    }

    ui.set_label(format!("Listening on {ips:?}:46899").into());

    ui.run()?;

    runtime().block_on(async move {
        event_tx.blocking_send(Event::Quit).unwrap();
        fin_rx.await.unwrap();
    });

    Ok(())
}
