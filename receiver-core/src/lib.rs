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

use anyhow::{Context, Result, bail};
use fcast_lib::models::{
    PlayMessage, PlaybackUpdateMessage, SeekMessage, SetSpeedMessage, SetVolumeMessage,
    VolumeUpdateMessage,
};
// use clap::Parser;
use fcast_lib::packet::Packet;
use gst::glib::base64_encode;
use gst::glib::object::Cast;
use gst::prelude::{ElementExt, GstBinExtManual};
use log::{debug, error, warn};
use pipeline::Pipeline;
use session::{Session, SessionId};
use slint::wgpu_26::wgpu;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::{broadcast, oneshot};

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use slint;

pub mod pipeline;
pub mod session;

#[derive(Debug)]
pub enum Event {
    Pause,
    Play(PlayMessage),
    Resume,
    Stop,
    SetSpeed(SetSpeedMessage),
    Seek(SeekMessage),
    SetVolume(SetVolumeMessage),
    Quit,
    PipelineEos,
    PipelineError,
    SessionFinished,
    ResumeOrPause,
    SeekPercent(f32),
    PipelineStateChanged(gst::State),
    ToggleDebug,
}

#[macro_export]
macro_rules! log_if_err {
    ($res:expr) => {
        if let Err(err) = $res {
            error!("{err}");
        }
    };
}

const FCAST_TCP_PORT: u16 = 46899;
const SENDER_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

slint::include_modules!();

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub struct WgpuSink {
    appsink: gst_app::AppSink,
    pub sinkbin: gst::Bin,
}

impl WgpuSink {
    const TEXTURE_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::COPY_DST
        .union(wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::RENDER_ATTACHMENT));

    fn create_texture(
        device: &wgpu::Device,
        size: wgpu::Extent3d,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video frame texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: Self::TEXTURE_USAGE,
            view_formats: &[],
        })
    }

    pub fn new() -> Result<Self> {
        let appsink = gst_app::AppSink::builder()
            .drop(true)
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .format_list([
                        gst_video::VideoFormat::Rgba,
                        gst_video::VideoFormat::Bgra,
                        // gst_video::VideoFormat::Nv12, // TODO: support this (and more) yuv format for less copies and processing upstream
                    ])
                    .pixel_aspect_ratio(gst::Fraction::new(1, 1))
                    .build(),
            )
            .enable_last_sample(false)
            .max_buffers(1u32)
            .build();

        let videoconvert = gst::ElementFactory::make("videoconvert").build()?;
        let sinkbin = gst::Bin::new();

        sinkbin.add_many([&videoconvert, appsink.upcast_ref()])?;
        gst::Element::link_many([&videoconvert, appsink.upcast_ref()])?;

        let videoconvert_pad = videoconvert
            .static_pad("sink")
            .ok_or(anyhow::anyhow!("Could not get sink pad from videoconvert"))?;
        let ghost = gst::GhostPad::with_target(&videoconvert_pad)?;
        sinkbin.add_pad(&ghost)?;

        Ok(Self { appsink, sinkbin })
    }

    pub fn connect(
        &mut self,
        ui_weak: slint::Weak<MainWindow>,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<()> {
        let texture_back = Arc::new(std::sync::Mutex::new(None::<wgpu::Texture>));
        self.appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink
                        .pull_sample()
                        .map_err(|_| gst::FlowError::Flushing)?;

                    let Some(buffer) = sample.buffer() else {
                        error!("Could not get buffer from pulled sample");
                        return Err(gst::FlowError::Error);
                    };

                    let Some(info) = sample
                        .caps()
                        .and_then(|caps| gst_video::VideoInfo::from_caps(caps).ok())
                    else {
                        error!("Got invalid caps");
                        return Err(gst::FlowError::NotNegotiated);
                    };

                    let readable = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let frame = readable.as_slice();

                    let pixel_format = match info.format() {
                        gst_video::VideoFormat::Rgba => wgpu::TextureFormat::Rgba8Unorm,
                        gst_video::VideoFormat::Bgra => wgpu::TextureFormat::Bgra8Unorm,
                        _ => {
                            error!("Got unsupportedformat: {:?}", info.format());
                            return Err(gst::FlowError::NotNegotiated);
                        }
                    };

                    let expected_size = info.width() as usize * info.height() as usize * 4;
                    if expected_size != frame.len() {
                        error!(
                            "Frame is not correct size: expected {expected_size}, got {}",
                            frame.len()
                        );
                        return Err(gst::FlowError::Error);
                    }

                    let frame_size = wgpu::Extent3d {
                        width: info.width(),
                        height: info.height(),
                        depth_or_array_layers: 1,
                    };

                    let texture_usage = wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT;

                    let texture = {
                        let maybe_texture = texture_back.lock().unwrap().take();
                        if let Some(texture) = maybe_texture {
                            if texture.format() != wgpu::TextureFormat::Rgba8Unorm
                                || texture.width() != info.width()
                                || texture.height() != info.height()
                                || texture.dimension() != wgpu::TextureDimension::D2
                                || texture.usage() != texture_usage
                            {
                                Self::create_texture(&device, frame_size, pixel_format)
                            } else {
                                texture
                            }
                        } else {
                            Self::create_texture(&device, frame_size, pixel_format)
                        }
                    };

                    queue.write_texture(
                        texture.as_image_copy(),
                        frame,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(info.width() * 4),
                            rows_per_image: Some(info.height()),
                        },
                        frame_size,
                    );

                    let texture_back = Arc::clone(&texture_back);

                    ui_weak
                        .upgrade_in_event_loop(move |ui| {
                            match slint::Image::try_from(texture) {
                                Ok(frame) => {
                                    let mut texture_back = texture_back.lock().unwrap();
                                    *texture_back = ui
                                        .global::<Bridge>()
                                        .get_video_frame()
                                        .to_wgpu_26_texture();

                                    ui.global::<Bridge>().set_video_frame(frame)
                                }
                                Err(err) => {
                                    error!("Failed to create image from texture: {err}")
                                }
                            };
                        })
                        .map_err(|_| gst::FlowError::Error)?;

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        Ok(())
    }
}

struct Application {
    pipeline: Pipeline,
    event_tx: Sender<Event>,
    ui_weak: slint::Weak<MainWindow>,
    updates_tx: broadcast::Sender<Arc<Vec<u8>>>,
    mdns: mdns_sd::ServiceDaemon,
    last_sent_update: Instant,
    debug_mode: bool,
}

impl Application {
    pub async fn new(
        appsink: gst::Element,
        event_tx: Sender<Event>,
        ui_weak: slint::Weak<MainWindow>,
    ) -> Result<Self> {
        let pipeline = Pipeline::new(appsink, event_tx.clone()).await?;
        let (updates_tx, _) = broadcast::channel(10);

        // TODO: IPv6?
        let mdns = {
            let daemon = mdns_sd::ServiceDaemon::new()?;

            let ips = common::net::get_all_ip_addresses()
                .into_iter()
                .filter(|a| a.is_ipv4() && !a.is_loopback())
                .collect::<Vec<IpAddr>>();

            if ips.is_empty() {
                bail!("No addresses available to use for mDNS discovery");
            }

            let name = format!(
                "OpenMirroring-{}",
                gethostname::gethostname().to_string_lossy()
            );

            let service = mdns_sd::ServiceInfo::new(
                "_fcast._tcp.local.",
                &name,
                &format!("{name}.local."),
                ips.as_slice(),
                FCAST_TCP_PORT,
                None::<std::collections::HashMap<String, String>>,
            )?;

            daemon.register(service)?;

            daemon
        };

        Ok(Self {
            pipeline,
            event_tx,
            ui_weak,
            updates_tx,
            mdns,
            last_sent_update: Instant::now() - SENDER_UPDATE_INTERVAL,
            debug_mode: false,
        })
    }

    fn notify_updates(&mut self) -> Result<()> {
        let pipeline_playback_state = match self.pipeline.get_playback_state() {
            Ok(s) => s,
            Err(err) => {
                error!("Failed to get playback state: {err}");
                return Ok(());
            }
        };

        let progress_str = {
            let update = &pipeline_playback_state;
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
        let progress_percent =
            (pipeline_playback_state.time / pipeline_playback_state.duration * 100.0) as f32;
        let playback_state = {
            let is_live = self.pipeline.is_live();
            use fcast_lib::models::PlaybackState;
            match pipeline_playback_state.state {
                PlaybackState::Playing | PlaybackState::Paused if is_live => GuiPlaybackState::Live,
                PlaybackState::Playing => GuiPlaybackState::Playing,
                PlaybackState::Paused => GuiPlaybackState::Paused,
                PlaybackState::Idle => GuiPlaybackState::Loading,
            }
        };

        self.ui_weak.upgrade_in_event_loop(move |ui| {
            ui.global::<Bridge>()
                .set_progress_label(progress_str.into());
            ui.invoke_update_progress_percent(progress_percent);
            ui.global::<Bridge>().set_playback_state(playback_state);
        })?;

        if self.updates_tx.receiver_count() > 0
            && self.last_sent_update.elapsed() >= SENDER_UPDATE_INTERVAL
        {
            let update = PlaybackUpdateMessage {
                generation: current_time_millis(),
                time: Some(pipeline_playback_state.time),
                duration: Some(pipeline_playback_state.duration),
                state: pipeline_playback_state.state,
                speed: Some(pipeline_playback_state.speed),
            };
            debug!("Sending update ({update:?})");
            self.updates_tx
                .send(Arc::new(Packet::from(update).encode()))?;
            self.last_sent_update = Instant::now();
        }

        Ok(())
    }

    /// Returns `true` if the event loop should exit
    async fn handle_event(&mut self, event: Event) -> Result<bool> {
        match event {
            Event::SessionFinished => {
                self.ui_weak.upgrade_in_event_loop(|ui| {
                    ui.invoke_device_disconnected();
                })?;
            }
            Event::Pause => {
                self.pipeline.pause().context("failed to pause pipeline")?;
                self.notify_updates()
                    .context("failed to notify about updates")?;
            }
            Event::Play(play_message) => {
                let Some(url) = play_message.url else {
                    error!("Play message does not contain a URL");
                    return Ok(false);
                };

                if let Err(err) = self.pipeline.set_playback_uri(&url) {
                    use pipeline::SetPlaybackUriError;
                    match err {
                        SetPlaybackUriError::PipelineStateChange(state_change_error) => {
                            return Err(state_change_error.into());
                        }
                        _ => {
                            error!("Failed to set playback URI: {err}");
                            return Ok(false);
                        }
                    }
                }
                if let Err(err) = self.pipeline.play_or_resume() {
                    error!("Failed to play_or_resume pipeline: {err}");
                } else {
                    self.ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_playback_started();
                    })?;
                    self.notify_updates()
                        .context("failed to notify about updates")?;
                }
            }
            Event::Resume => self
                .pipeline
                .play_or_resume()
                .context("failed to play or resume pipeline")?,
            Event::ResumeOrPause => {
                let Some(playing) = self.pipeline.is_playing() else {
                    warn!("Pipeline is not in a state that can be resumed or paused");
                    return Ok(false);
                };
                if let Err(err) = if playing {
                    self.pipeline.pause()
                } else {
                    self.pipeline.play_or_resume()
                } {
                    error!("Failed to play or resume: {err}");
                }
                self.notify_updates()
                    .context("failed to notify about updates")?;
            }
            Event::Stop => {
                self.pipeline.stop()?;
                self.ui_weak.upgrade_in_event_loop(|ui| {
                    ui.invoke_playback_stopped();
                })?;
            }
            Event::SetSpeed(set_speed_message) => self
                .pipeline
                .set_speed(set_speed_message.speed)
                .context("failed to set speed")?,
            Event::Seek(seek_message) => {
                if let Err(err) = self.pipeline.seek(seek_message.time) {
                    error!("Seek error: {err}");
                    return Ok(false);
                }
                self.notify_updates()?;
            }
            Event::SeekPercent(percent) => {
                let Some(duration) = self.pipeline.get_duration() else {
                    error!("Failed to get playback duration");
                    return Ok(false);
                };
                if duration.is_zero() {
                    error!("Cannot seek when the duration is zero");
                    return Ok(false);
                }
                let seek_to = duration.seconds_f64() * (percent as f64 / 100.0);
                if let Err(err) = self.pipeline.seek(seek_to) {
                    error!("Seek error: {err}");
                    return Ok(false);
                }
                self.notify_updates()?;
            }
            Event::SetVolume(set_volume_message) => {
                self.pipeline.set_volume(set_volume_message.volume);
                self.ui_weak.upgrade_in_event_loop(move |ui| {
                    ui.global::<Bridge>()
                        .set_volume(set_volume_message.volume as f32);
                })?;
                if self.updates_tx.receiver_count() > 0 {
                    let update = VolumeUpdateMessage {
                        generation: current_time_millis(),
                        volume: set_volume_message.volume,
                    };
                    debug!("Sending update ({update:?})");
                    self.updates_tx
                        .send(Arc::new(Packet::from(update).encode()))?;
                    self.last_sent_update = Instant::now();
                }
            }
            Event::Quit => return Ok(true),
            Event::PipelineEos => {
                debug!("Pipeline reached EOS");
                // self.pipeline.stop()?;
                // self.ui_weak.upgrade_in_event_loop(|ui| {
                //     ui.invoke_playback_stopped();
                // })?;
            }
            Event::PipelineError => {
                self.pipeline.stop().context("failed to stop pipeline")?;
                // TODO: send error message to sessions
                self.ui_weak.upgrade_in_event_loop(|ui| {
                    ui.invoke_playback_stopped_with_error("Error unclear (todo)".into());
                })?;
            }
            Event::PipelineStateChanged(state) => match state {
                gst::State::Paused | gst::State::Playing => self
                    .notify_updates()
                    .context("failed to notify about updates")?,
                _ => (),
            },
            Event::ToggleDebug => self.debug_mode = !self.debug_mode,
        }

        Ok(false)
    }

    pub async fn run_event_loop(
        mut self,
        mut event_rx: Receiver<Event>,
        fin_tx: oneshot::Sender<()>,
    ) -> Result<()> {
        let dispatch_listener = TcpListener::bind("0.0.0.0:46899").await?;

        let mut session_id: SessionId = 0;
        let mut update_interval = tokio::time::interval(Duration::from_millis(100));

        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    if let Some(event) = event {
                        debug!("Got event: {event:?}");
                        match self.handle_event(event).await {
                            Ok(true) => break,
                            Err(err) => error!("Handle event error: {err}"),
                            _ => (),
                        }
                    } else {
                        break;
                    }
                }
                _ = update_interval.tick() => {
                    // if self.pipeline.is_playing() == Some(true) {
                        self.notify_updates()?;
                    // }
                }
                session = dispatch_listener.accept() => {
                    let (stream, _) = session?;

                    debug!("New connection id={session_id}");

                    tokio::spawn({
                        let id = session_id;
                        let event_tx = self.event_tx.clone();
                        let updates_rx = self.updates_tx.subscribe();
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

                    self.ui_weak.upgrade_in_event_loop(move |ui| {
                        ui.invoke_device_connected();
                    })?;

                    session_id += 1;
                }
            }
        }

        self.pipeline.stop()?;

        debug!("Quitting");

        if fin_tx.send(()).is_err() {
            bail!("Failed to send fin");
        }

        self.mdns.shutdown()?;

        Ok(())
    }
}

#[derive(clap::Parser)]
#[command(version)]
struct CliArgs {
    // Disable animated background. Reduces resource usage
    // #[arg(short = 'b', long, default_value_t = false)]
    // no_background: bool,
}

/// Run the main app.
///
/// Slint and friends are assumed to be initialized by the platform specific target.
pub fn run() -> Result<()> {
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

    // TODO: fix, base64? format?
    let device_url = format!(
        "fcast://r/{}",
        base64_encode(
            format!(
                r#"{{"name":"Test","addresses":[{}],"services":[{{"port":46899,"type":1}}]}}"#,
                ips.iter()
                    .map(|addr| format!("\"{}\"", addr))
                    .collect::<Vec<String>>()
                    .join(","),
            )
            .as_bytes()
        )
        .as_str(),
    );

    let qr_code = qrcode::QrCode::new(device_url)?;
    let qr_code_dims = qr_code.width() as u32;
    let qr_code_colors = qr_code.into_colors();
    let mut qr_code_pixels =
        slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(qr_code_dims, qr_code_dims);
    qr_code_pixels.make_mut_slice().copy_from_slice(
        &qr_code_colors
            .into_iter()
            .map(|px| match px {
                qrcode::Color::Light => slint::Rgb8Pixel::new(0xFF, 0xFF, 0xFF),
                qrcode::Color::Dark => slint::Rgb8Pixel::new(0x0, 0x0, 0x0),
            })
            .collect::<Vec<slint::Rgb8Pixel>>(),
    );
    let qr_code_image = slint::Image::from_rgb8(qr_code_pixels);

    let (event_tx, event_rx) = mpsc::channel::<Event>(100);
    let (fin_tx, fin_rx) = oneshot::channel::<()>();

    let mut slint_sink = WgpuSink::new()?;
    let slint_appsink = slint_sink.sinkbin.clone().upcast::<gst::Element>();

    let ui = MainWindow::new()?;

    ui.global::<Bridge>().set_qr_code(qr_code_image);

    #[cfg(debug_assertions)]
    ui.global::<Bridge>().set_is_debugging(true);

    ui.window().set_rendering_notifier({
        let ui_weak = ui.as_weak();

        move |state, graphics_api| {
            if let slint::RenderingState::RenderingSetup = state {
                debug!("Got graphics API: {graphics_api:?}");
                let ui_weak = ui_weak.clone();
                match graphics_api {
                    slint::GraphicsAPI::WGPU26 { device, queue, .. } => {
                        slint_sink
                            .connect(ui_weak, device.clone(), queue.clone())
                            .unwrap(); // TODO: gracefully handle error (show in UI maybe?)
                    }
                    _ => panic!("Unsupported graphics api ({graphics_api:?})"),
                }
            }
        }
    })?;

    common::runtime().spawn({
        let ui_weak = ui.as_weak();
        let event_tx = event_tx.clone();
        async move {
            Application::new(slint_appsink, event_tx, ui_weak)
                .await
                .unwrap()
                .run_event_loop(event_rx, fin_tx)
                .await
                .unwrap();
        }
    });

    ui.global::<Bridge>().on_resume_or_pause({
        let event_tx = event_tx.clone();
        move || {
            log_if_err!(event_tx.blocking_send(Event::ResumeOrPause));
        }
    });

    ui.global::<Bridge>().on_seek_to_percent({
        let event_tx = event_tx.clone();
        move |percent| {
            log_if_err!(event_tx.blocking_send(Event::SeekPercent(percent)));
        }
    });

    ui.global::<Bridge>().on_toggle_fullscreen({
        let ui_weak = ui.as_weak();
        move || {
            let ui = ui_weak
                .upgrade()
                .expect("callbacks always get called from the event loop");
            ui.window().set_fullscreen(!ui.window().is_fullscreen());
        }
    });

    ui.global::<Bridge>().on_set_volume({
        let event_tx = event_tx.clone();
        move |volume| {
            log_if_err!(event_tx.blocking_send(Event::SetVolume(SetVolumeMessage {
                volume: volume as f64,
            })));
        }
    });

    ui.global::<Bridge>().on_force_quit(move || {
        log_if_err!(slint::quit_event_loop());
    });

    ui.global::<Bridge>().on_debug_toggled({
        let event_tx = event_tx.clone();
        move || {
            log_if_err!(event_tx.blocking_send(Event::ToggleDebug));
        }
    });

    ui.global::<Bridge>().set_label(format!("{ips:?}").into());

    ui.run()?;

    debug!("Shutting down...");

    common::runtime().block_on(async move {
        event_tx.send(Event::Quit).await.unwrap();
        fin_rx.await.unwrap();
    });

    Ok(())
}
