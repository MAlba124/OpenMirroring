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

use anyhow::{anyhow, bail, Result};
use common::runtime;
use common::video::opengl::SlintOpenGLSink;
use fcast_lib::packet::Packet;
use log::{debug, error};
use receiver::dispatcher::Dispatcher;
use receiver::pipeline::Pipeline;
use receiver::session::Session;
use receiver::Event;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;

use std::net::Ipv4Addr;

slint::include_modules!();

use glow::HasContext;

// Modified from https://github.com/slint-ui/slint/blob/master/examples/opengl_underlay/main.rs
struct EGLUnderlay {
    gl: glow::Context,
    program: glow::Program,
    effect_time_location: glow::UniformLocation,
    width_location: glow::UniformLocation,
    height_location: glow::UniformLocation,
    vbo: glow::Buffer,
    vao: glow::VertexArray,
    start_time: std::time::Instant,
}

impl EGLUnderlay {
    fn new(gl: glow::Context) -> Result<Self> {
        unsafe {
            let program = gl.create_program().expect("Cannot create program");

            // TODO: try out https://www.shadertoy.com/view/sldGDf or any from wallpaper tag on shader toy https://www.shadertoy.com/results?query=tag%3Dwallpaper
            let (vertex_shader_source, fragment_shader_source) = (
                include_str!("../shaders/vertex.glsl"),
                include_str!("../shaders/fragment.glsl"),
            );

            let shader_sources = [
                (glow::VERTEX_SHADER, vertex_shader_source),
                (glow::FRAGMENT_SHADER, fragment_shader_source),
            ];

            let mut shaders = Vec::with_capacity(shader_sources.len());

            for (shader_type, shader_source) in shader_sources.iter() {
                let shader = gl
                    .create_shader(*shader_type)
                    .expect("Cannot create shader");
                gl.shader_source(shader, shader_source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    panic!("{}", gl.get_shader_info_log(shader));
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }

            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                panic!("{}", gl.get_program_info_log(program));
            }

            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }

            let effect_time_location = gl
                .get_uniform_location(program, "effect_time")
                .ok_or(anyhow!("Failed to get uniform location of `effect_time`"))?;
            let width_location = gl
                .get_uniform_location(program, "win_width")
                .ok_or(anyhow!("Failed to get uniform location of `win_width`"))?;
            let height_location = gl
                .get_uniform_location(program, "win_height")
                .ok_or(anyhow!("Failed to get uniform location of `win_height`"))?;
            let position_location = gl
                .get_attrib_location(program, "position")
                .ok_or(anyhow!("Failed to get uniform location of `position`"))?;

            let vbo = gl.create_buffer().expect("Cannot create buffer");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));

            let vertices = [
                -1.0f32, 1.0f32, -1.0f32, -1.0f32, 1.0f32, 1.0f32, 1.0f32, -1.0f32,
            ];

            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices.align_to().1, glow::STATIC_DRAW);

            let vao = gl.create_vertex_array().map_err(|err| anyhow!("{err}"))?;
            gl.bind_vertex_array(Some(vao));
            gl.enable_vertex_attrib_array(position_location);
            gl.vertex_attrib_pointer_f32(position_location, 2, glow::FLOAT, false, 8, 0);

            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);

            Ok(Self {
                gl,
                program,
                effect_time_location,
                vbo,
                vao,
                start_time: std::time::Instant::now(),
                width_location,
                height_location,
            })
        }
    }

    fn render(&mut self, width: f32, height: f32) {
        unsafe {
            let gl = &self.gl;

            gl.use_program(Some(self.program));

            let old_buffer =
                std::num::NonZeroU32::new(gl.get_parameter_i32(glow::ARRAY_BUFFER_BINDING) as u32)
                    .map(glow::NativeBuffer);

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let old_vao =
                std::num::NonZeroU32::new(gl.get_parameter_i32(glow::VERTEX_ARRAY_BINDING) as u32)
                    .map(glow::NativeVertexArray);

            gl.bind_vertex_array(Some(self.vao));

            let elapsed = self.start_time.elapsed().as_secs_f32() / 1.5;
            gl.uniform_1_f32(Some(&self.effect_time_location), elapsed);

            // TODO: room for optimization?
            gl.uniform_1_f32(Some(&self.width_location), width);
            gl.uniform_1_f32(Some(&self.height_location), height);

            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

            gl.bind_buffer(glow::ARRAY_BUFFER, old_buffer);
            gl.bind_vertex_array(old_vao);
            gl.use_program(None);
        }
    }
}

impl Drop for EGLUnderlay {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.vbo);
        }
    }
}

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

                            event_tx.send(Event::SessionFinished).await.unwrap();
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
                Event::Pause => self.pipeline.pause()?,
                Event::Play(play_message) => {
                    self.pipeline.set_playback_uri(&play_message.url.unwrap())?;
                    if let Err(err) = self.pipeline.play_or_resume() {
                        error!("Failed to play_or_resume pipeline: {err}");
                    } else {
                        ui_weak.upgrade_in_event_loop(|ui| {
                            ui.invoke_playback_started();
                        })?
                    }
                }
                Event::Resume => self.pipeline.play_or_resume()?,
                Event::Stop => {
                    self.pipeline.stop()?;
                    ui_weak.upgrade_in_event_loop(|ui| {
                        ui.invoke_playback_stopped();
                    })?;
                }
                Event::SetSpeed(set_speed_message) => {
                    self.pipeline.set_speed(set_speed_message.speed)?
                }
                Event::Seek(seek_message) => self.pipeline.seek(seek_message.time)?,
                Event::SetVolume(set_volume_message) => {
                    self.pipeline.set_volume(set_volume_message.volume)
                }
                Event::SendPlaybackUpdate => {
                    if updates_tx.receiver_count() > 0 {
                        let update = self.pipeline.get_playback_state()?;
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

    let mut underlay = None;

    ui.window().set_rendering_notifier({
        let ui_weak = ui.as_weak();

        let new_frame_cb = |ui: MainWindow, new_frame| {
            ui.set_preview_frame(new_frame);
        };

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
                underlay = match EGLUnderlay::new(glow_context) {
                    Ok(underlay) => Some(underlay),
                    Err(err) => {
                        error!("Failed to crate GL underlay: {err}");
                        None
                    }
                }
            }
            slint::RenderingState::BeforeRendering => {
                let Some(ui) = ui_weak.upgrade() else {
                    error!("Failed to upgrade ui");
                    return;
                };

                if ui.get_playing() {
                    if let Some(next_frame) = slint_sink.fetch_next_frame() {
                        new_frame_cb(ui, next_frame);
                    }
                } else {
                    if let Some(underlay) = underlay.as_mut() {
                        let window_size = ui.window().size();
                        underlay.render(window_size.width as f32, window_size.height as f32);
                        ui.window().request_redraw();
                    }
                }
            }
            slint::RenderingState::RenderingTeardown => {
                slint_sink.deactivate_and_pause();
                drop(underlay.take());
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

    ui.set_label(format!("{ips:?}").into());

    ui.run()?;

    runtime().block_on(async move {
        debug!("Shutting down...");

        disp_fin_tx.send(()).unwrap();
        fin_rx.await.unwrap();
    });

    Ok(())
}
