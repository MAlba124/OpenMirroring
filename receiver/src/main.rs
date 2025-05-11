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
use fcast_lib::packet::Packet;
use log::{debug, error, warn};
use receiver::dispatcher::Dispatcher;
use receiver::pipeline::Pipeline;
use receiver::session::Session;
use receiver::underlays::background::BackgroundUnderlay;
use receiver::underlays::video::VideoUnderlay;
use receiver::Event;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;

use std::net::Ipv4Addr;

slint::include_modules!();

struct Application {
    pipeline: Pipeline,
    event_tx: Sender<Event>,
}

impl Application {
    pub async fn new(appsink: gst::Element, event_tx: Sender<Event>) -> Result<Self> {
        let pipeline = Pipeline::new(appsink, event_tx.clone()).await?;

        tokio::spawn({
            let event_tx = event_tx.clone();
            async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    event_tx.send(Event::SendPlaybackUpdate).await.unwrap();
                }
            }
        });

        Ok(Self { pipeline, event_tx })
    }

    pub async fn run_event_loop(
        self,
        mut event_rx: Receiver<Event>,
        ui_weak: slint::Weak<MainWindow>,
        fin_tx: oneshot::Sender<()>,
    ) -> Result<()> {
        let (updates_tx, _) = tokio::sync::broadcast::channel(10);

        while let Some(event) = event_rx.recv().await {
            match event {
                Event::CreateSessionRequest { stream, id } => {
                    debug!("Got CreateSessionRequest id={id}");
                    tokio::spawn({
                        let event_tx = self.event_tx.clone();
                        let updates_rx = updates_tx.subscribe();
                        async move {
                            if let Err(err) =
                                Session::new(stream, id).run(updates_rx, &event_tx).await
                            {
                                error!("Session exited with error: {err}");
                            }

                            if let Err(err) = event_tx.send(Event::SessionFinished).await {
                                error!("Failed to send SessionFinished: {err}");
                            }
                        }
                    });
                    ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_device_connected();
                    })?;
                }
                Event::SessionFinished => {
                    ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_device_disconnected();
                    })?;
                }
                Event::Pause => {
                    self.pipeline.pause()?;
                    self.event_tx.send(Event::SendPlaybackUpdate).await?; // TODO: directly do this
                }
                Event::Play(play_message) => {
                    self.pipeline.set_playback_uri(&play_message.url.unwrap())?;
                    if let Err(err) = self.pipeline.play_or_resume() {
                        error!("Failed to play_or_resume pipeline: {err}");
                    } else {
                        ui_weak.upgrade_in_event_loop(|ui| {
                            ui.invoke_playback_started();
                        })?;
                        self.event_tx.send(Event::SendPlaybackUpdate).await?; // TODO: directly do this
                    }
                }
                Event::Resume => self.pipeline.play_or_resume()?,
                Event::ResumeOrPause => {
                    let Some(playing) = self.pipeline.is_playing() else {
                        warn!("Pipeline is not in a state that can be resumed or paused");
                        continue;
                    };
                    if let Err(err) = if playing {
                        self.pipeline.pause()
                    } else {
                        self.pipeline.play_or_resume()
                    } {
                        error!("Failed to ResumeOrPause: {err}");
                    }
                    self.event_tx.send(Event::SendPlaybackUpdate).await?; // TODO: directly do this
                }
                Event::Stop => {
                    self.pipeline.stop()?;
                    ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_playback_stopped();
                    })?;
                }
                Event::SetSpeed(set_speed_message) => {
                    self.pipeline.set_speed(set_speed_message.speed)?
                }
                Event::Seek(seek_message) => {
                    self.pipeline.seek(seek_message.time)?;
                    self.event_tx.send(Event::SendPlaybackUpdate).await?; // TODO: directly do this
                }
                Event::SeekPercent(percent) => {
                    let Some(duration) = self.pipeline.get_duration() else {
                        error!("Failed to get playback duration");
                        continue;
                    };
                    if duration.is_zero() {
                        error!("Cannot seek when the duration is zero");
                        continue;
                    }
                    let seek_to = duration.seconds_f64() * (percent as f64 / 100.0);
                    self.pipeline.seek(seek_to)?;
                    self.event_tx.send(Event::SendPlaybackUpdate).await?; // TODO: directly do this
                }
                Event::SetVolume(set_volume_message) => {
                    self.pipeline.set_volume(set_volume_message.volume)
                }
                Event::SendPlaybackUpdate => {
                    let update = self.pipeline.get_playback_state()?;

                    let progress_str = {
                        let time_secs = update.time % 60.0;
                        let time_mins = (update.time / 60.0) % 60.0;
                        let time_hours = update.time / 60.0 / 60.0;

                        let duration_secs = update.duration % 60.0;
                        let duration_mins = (update.duration / 60.0) % 60.0;
                        let duration_hours = update.duration / 60.0 / 60.0;

                        format!(
                            "{:02}:{:02}:{:02} / {:02}:{:02}:{:02}",
                            time_hours as u32,
                            time_mins as u32,
                            time_secs as u32,
                            duration_hours as u32,
                            duration_mins as u32,
                            duration_secs as u32,
                        )
                    };
                    let progress_percent = (update.time / update.duration * 100.0) as f32;
                    let playback_state = {
                        let is_live = self.pipeline.is_live();
                        match update.state {
                            fcast_lib::models::PlaybackState::Playing
                            | fcast_lib::models::PlaybackState::Paused
                                if is_live =>
                            {
                                GuiPlaybackState::Live
                            }
                            fcast_lib::models::PlaybackState::Playing => GuiPlaybackState::Playing,
                            fcast_lib::models::PlaybackState::Paused => GuiPlaybackState::Paused,
                            fcast_lib::models::PlaybackState::Idle => GuiPlaybackState::Loading,
                        }
                    };

                    ui_weak.upgrade_in_event_loop(move |ui| {
                        ui.set_progress_label(progress_str.into());
                        ui.invoke_update_progress_percent(progress_percent);
                        // ui.set_progress_percent(progress_percent);
                        ui.set_playback_state(playback_state);
                    })?;

                    if updates_tx.receiver_count() > 0 {
                        debug!("Sending update ({update:?})");
                        updates_tx.send(Packet::from(update).encode())?;
                    }
                }
                Event::Quit => break,
                Event::PipelineEos => {
                    debug!("Pipeline reached EOS");
                    self.pipeline.stop()?;
                }
                Event::PipelineError => {
                    self.pipeline.stop()?;
                    // TODO: send error message to sessions
                    ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_playback_stopped_with_error("Error unclear (todo)".into());
                    })?;
                }
            }
        }

        // TODO: gracefully shutdown the pipeline

        debug!("Quitting");

        if fin_tx.send(()).is_err() {
            bail!("Failed to send fin");
        }

        Ok(())
    }
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
            std::net::IpAddr::V4(v4) if !v4.is_loopback() => ips.push(v4),
            std::net::IpAddr::V6(v6) if !v6.is_loopback() => {
                debug!("Ignoring IPv6 address ({v6:?})")
            }
            _ => debug!("Ignoring loopback IP address ({ip:?})"),
        }
    }

    let (event_tx, event_rx) = mpsc::channel::<Event>(100);
    let (fin_tx, fin_rx) = oneshot::channel::<()>();

    let mut slint_sink = SlintOpenGLSink::new()?;
    let slint_appsink = slint_sink.video_sink();

    let ui = MainWindow::new()?;
    slint::set_xdg_app_id("com.github.malba124.OpenMirroring.receiver")?;

    ui.window().set_rendering_notifier({
        let ui_weak = ui.as_weak();

        let mut background_underlay: Option<BackgroundUnderlay> = None;
        let mut video_underlay: Option<VideoUnderlay> = None;

        move |state, graphics_api| match state {
            slint::RenderingState::RenderingSetup => {
                let ui_weak = ui_weak.clone();
                slint_sink
                    .connect(graphics_api, move || {
                        ui_weak
                            .upgrade_in_event_loop(move |ui| {
                                ui.window().request_redraw();
                            })
                            .ok();
                    })
                    .unwrap();
                let glow_context = match graphics_api {
                    slint::GraphicsAPI::NativeOpenGL { get_proc_address } => unsafe {
                        glow::Context::from_loader_function_cstr(|s| get_proc_address(s))
                    },
                    _ => unreachable!(),
                };

                let glow_context = std::rc::Rc::new(glow_context);

                background_underlay = Some(BackgroundUnderlay::new(glow_context.clone()).unwrap());
                video_underlay = Some(VideoUnderlay::new(glow_context).unwrap());
            }
            slint::RenderingState::BeforeRendering => {
                let Some(ui) = ui_weak.upgrade() else {
                    error!("Failed to upgrade ui");
                    return;
                };

                // TODO: use let chains when updated to rust 2024
                // TODO: don't render the video when the frame is from the old source (i.e. playback was
                //       stopped, then new source was set and for a brief moment the last displayed frame
                //       of the old source becomes visible.)
                if ui.get_playing() {
                    let Some((texture, size)) = slint_sink.fetch_next_frame_as_texture() else {
                        return;
                    };
                    let Some(underlay) = video_underlay.as_mut() else {
                        return;
                    };

                    let win_size = ui.window().size();
                    underlay.render(texture, win_size.width, win_size.height, size[0], size[1]);
                } else if let Some(underlay) = background_underlay.as_mut() {
                    let window_size = ui.window().size();
                    underlay.render(window_size.width as f32, window_size.height as f32);
                    ui.window().request_redraw();
                }
            }
            slint::RenderingState::RenderingTeardown => {
                slint_sink.deactivate_and_pause().unwrap();
                drop(background_underlay.take());
            }
            _ => (),
        }
    })?;

    common::runtime().spawn({
        let ui_weak = ui.as_weak();
        let event_tx = event_tx.clone();
        async move {
            Application::new(slint_appsink, event_tx)
                .await
                .unwrap()
                .run_event_loop(event_rx, ui_weak, fin_tx)
                .await
                .unwrap();
        }
    });

    let (disp_fin_tx, disp_fin_rx) = tokio::sync::oneshot::channel();
    {
        let event_tx = event_tx.clone();
        runtime().spawn(async move {
            Dispatcher::new(event_tx)
                .await
                .unwrap()
                .run(disp_fin_rx)
                .await
                .unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_resume_or_pause(move || {
            event_tx.blocking_send(Event::ResumeOrPause).unwrap();
        });
    }

    {
        let event_tx = event_tx.clone();
        ui.on_seek_to_percent(move |percent| {
            event_tx.blocking_send(Event::SeekPercent(percent)).unwrap();
        });
    }

    ui.set_label(format!("{ips:?}").into());

    ui.run()?;

    runtime().block_on(async move {
        debug!("Shutting down...");

        disp_fin_tx.send(()).unwrap();
        fin_rx.await.unwrap();
    });

    Ok(())
}
