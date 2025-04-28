// Copyright (C) 2025 Marcus L. Hanestad <marlhan@proton.me>
//
// This file is part of OpenMirroring.
//
// OpenMirroring is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// OpenMirroring is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with OpenMirroring.  If not, see <https://www.gnu.org/licenses/>.

use anyhow::{bail, Result};
use common::runtime;
use common::video::opengl::SlintOpenGLSink;
use common::video::GstGlContext;
use fcast_lib::models::PlaybackUpdateMessage;
use fcast_lib::packet::Packet;
use log::{debug, warn};
use receiver::dispatcher::Dispatcher;
use receiver::pipeline::Pipeline;
use receiver::session::Session;
use receiver::Event;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

slint::include_modules!();

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

async fn event_loop(
    mut event_rx: Receiver<Event>,
    event_tx: Sender<Event>,
    ui_weak: slint::Weak<MainWindow>,
    fin_tx: oneshot::Sender<()>,
    appsink: gst::Element,
    gst_gl_context: GstGlContext,
) -> Result<()> {
    let (updates_tx, _) = tokio::sync::broadcast::channel(10);

    let pipeline = Pipeline::new(appsink, gst_gl_context, event_tx.clone())?;

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

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .require_opengl()
        .select()?;

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

    let mut slint_sink = SlintOpenGLSink::new()?;
    let slint_appsink = slint_sink.video_sink();
    let gst_gl_context = Arc::new(Mutex::new(None::<(gst_gl::GLContext, gst_gl::GLDisplay)>));

    let ui = MainWindow::new()?;
    slint::set_xdg_app_id("com.github.malba124.OpenMirroring.receiver")?;

    ui.window().set_rendering_notifier({
        let gst_gl_context = Arc::clone(&gst_gl_context);
        let ui_weak = ui.as_weak();

        let new_frame_cb = |ui: MainWindow, new_frame| {
            ui.set_preview_frame(new_frame);
        };

        move |state, graphics_api| match state {
            slint::RenderingState::RenderingSetup => {
                let ui_weak = ui_weak.clone();
                let mut gst_gl_context = gst_gl_context.lock().unwrap();
                *gst_gl_context = Some(
                    slint_sink
                        .connect(
                            graphics_api,
                            Box::new(move || {
                                ui_weak
                                    .upgrade_in_event_loop(move |ui| {
                                        ui.window().request_redraw();
                                    })
                                    .ok();
                            }),
                        )
                        .unwrap(),
                );
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
    })?;

    common::runtime().spawn(event_loop(
        event_rx,
        event_tx.clone(),
        ui.as_weak(),
        fin_tx,
        slint_appsink,
        gst_gl_context,
    ));

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
