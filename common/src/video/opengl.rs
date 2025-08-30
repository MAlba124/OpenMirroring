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

use anyhow::{Result, bail};
use std::{
    num::NonZero,
    sync::{Arc, Mutex},
};

use gst_gl::prelude::*;
use log::error;

// Taken partially from the slint gstreamer example at: https://github.com/slint-ui/slint/blob/2edd97bf8b8dc4dc26b578df6b15ea3297447444/examples/gstreamer-player/egl_integration.rs
pub struct SlintOpenGLSink {
    appsink: gst_app::AppSink,
    glsink: gst::Element,
    next_frame: Arc<Mutex<Option<(gst_video::VideoInfo, gst::Buffer)>>>,
    current_frame: Mutex<Option<gst_gl::GLVideoFrame<gst_gl::gl_video_frame::Readable>>>,
    gst_gl_context: Option<gst_gl::GLContext>,
}

fn is_on_wayland() -> Result<bool> {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        Ok(true)
    } else if std::env::var("DISPLAY").is_ok() {
        Ok(false)
    } else {
        bail!("Unsupported platform")
    }
}

impl SlintOpenGLSink {
    pub fn new() -> Result<Self> {
        let appsink = gst_app::AppSink::builder()
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .features([gst_gl::CAPS_FEATURE_MEMORY_GL_MEMORY])
                    .format(gst_video::VideoFormat::Rgba)
                    .field("texture-target", "2D")
                    .width_range(1..i32::MAX)
                    .height_range(1..i32::MAX)
                    .build(),
            )
            .enable_last_sample(false)
            .max_buffers(1u32)
            .build();

        let glsink = gst::ElementFactory::make("glsinkbin")
            .property("sink", &appsink)
            .build()?;

        Ok(Self {
            appsink,
            glsink,
            next_frame: Default::default(),
            current_frame: Default::default(),
            gst_gl_context: None,
        })
    }

    pub fn video_sink(&self) -> gst::Element {
        self.glsink.clone().upcast()
    }

    #[cfg(target_os = "linux")]
    fn get_egl_ctx(
        graphics_api: &slint::GraphicsAPI<'_>,
    ) -> Result<(gst_gl::GLContext, gst_gl::GLDisplay)> {
        let egl = match graphics_api {
            slint::GraphicsAPI::NativeOpenGL { get_proc_address } => {
                glutin_egl_sys::egl::Egl::load_with(|symbol| {
                    get_proc_address(&std::ffi::CString::new(symbol).unwrap())
                })
            }
            _ => anyhow::bail!("Unsupported graphics API"),
        };

        let platform = gst_gl::GLPlatform::EGL;

        unsafe {
            let egl_display = egl.GetCurrentDisplay();
            let display = gst_gl_egl::GLDisplayEGL::with_egl_display(egl_display as usize)?;
            let native_context = egl.GetCurrentContext();

            Ok((
                gst_gl::GLContext::new_wrapped(
                    &display,
                    native_context as _,
                    platform,
                    gst_gl::GLContext::current_gl_api(platform).0,
                )
                .ok_or(anyhow::anyhow!("unable to create wrapped GL context"))?,
                display.upcast(),
            ))
        }
    }

    #[cfg(target_os = "linux")]
    fn get_glx_ctx(
        graphics_api: &slint::GraphicsAPI<'_>,
    ) -> Result<(gst_gl::GLContext, gst_gl::GLDisplay)> {
        let glx = match graphics_api {
            slint::GraphicsAPI::NativeOpenGL { get_proc_address } => {
                glutin_glx_sys::glx::Glx::load_with(|symbol| {
                    get_proc_address(&std::ffi::CString::new(symbol).unwrap())
                })
            }
            _ => anyhow::bail!("Unsupported graphics API"),
        };

        let platform = gst_gl::GLPlatform::GLX;

        unsafe {
            let glx_display = glx.GetCurrentDisplay();
            let display = gst_gl_x11::GLDisplayX11::with_display(glx_display as usize)?;
            let native_context = glx.GetCurrentContext();

            Ok((
                gst_gl::GLContext::new_wrapped(
                    &display,
                    native_context as _,
                    platform,
                    gst_gl::GLContext::current_gl_api(platform).0,
                )
                .ok_or(anyhow::anyhow!("unable to create wrapped GL context"))?,
                display.upcast(),
            ))
        }
    }

    #[cfg(target_os = "windows")]
    fn get_wgl_ctx() -> Result<(gst_gl::GLContext, gst_gl::GLDisplay)> {
        use anyhow::bail;

        let platform = gst_gl::GLPlatform::WGL;
        let gl_api = gst_gl::GLAPI::OPENGL3;
        let gl_ctx = gst_gl::GLContext::current_gl_context(platform);

        if gl_ctx == 0 {
            bail!("Failed to create GL context");
        }

        let Some(gst_display) = gst_gl::GLDisplay::with_type(gst_gl::GLDisplayType::WIN32) else {
            bail!("Failed to create GLDisplay of type WIN32");
        };

        gst_display.filter_gl_api(gl_api);

        unsafe {
            Ok((
                gst_gl::GLContext::new_wrapped(&gst_display, gl_ctx, platform, gl_api)
                    .ok_or(anyhow::anyhow!("unable to create wrapped GL context"))?,
                gst_display,
            ))
        }
    }

    pub fn connect<F>(
        &mut self,
        graphics_api: &slint::GraphicsAPI<'_>,
        next_frame_available_notifier: F,
    ) -> Result<()>
    where
        F: Fn() + Send + 'static,
    {
        #[cfg(target_os = "linux")]
        let (gst_gl_context, gst_gl_display) = {
            match is_on_wayland() {
                // NOTE: If error: assume KMS
                Ok(true) | Err(_) => Self::get_egl_ctx(graphics_api)?,
                Ok(false) => Self::get_glx_ctx(graphics_api)?
            }
        };
        #[cfg(target_os = "windows")]
        let (gst_gl_context, gst_gl_display) = Self::get_wgl_ctx()?;

        gst_gl_context
            .activate(true)
            .expect("could not activate GStreamer GL context");
        gst_gl_context
            .fill_info()
            .expect("failed to fill GL info for wrapped context");

        self.gst_gl_context = Some(gst_gl_context.clone());

        let display_ctx = gst::Context::new(gst_gl::GL_DISPLAY_CONTEXT_TYPE, true);
        display_ctx.set_gl_display(&gst_gl_display);
        self.glsink.set_context(&display_ctx);

        let mut app_ctx = gst::Context::new("gst.gl.app_context", true);
        let app_ctx_mut = app_ctx.get_mut().unwrap();
        let structure = app_ctx_mut.structure_mut();
        structure.set("context", gst_gl_context.clone());
        self.glsink.set_context(&app_ctx);

        let next_frame_ref = self.next_frame.clone();

        self.appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink
                        .pull_sample()
                        .map_err(|_| gst::FlowError::Flushing)?;

                    let mut buffer = sample.buffer_owned().unwrap();
                    {
                        let context = match (buffer.n_memory() > 0)
                            .then(|| buffer.peek_memory(0))
                            .and_then(|m| m.downcast_memory_ref::<gst_gl::GLBaseMemory>())
                            .map(|m| m.context())
                        {
                            Some(context) => context.clone(),
                            None => {
                                error!("Got non-GL memory");
                                return Err(gst::FlowError::Error);
                            }
                        };

                        // Sync point to ensure that the rendering in this context will be complete by the time the
                        // Slint created GL context needs to access the texture.
                        if let Some(meta) = buffer.meta::<gst_gl::GLSyncMeta>() {
                            meta.set_sync_point(&context);
                        } else {
                            let buffer = buffer.make_mut();
                            let meta = gst_gl::GLSyncMeta::add(buffer, &context);
                            meta.set_sync_point(&context);
                        }
                    }

                    let Some(info) = sample
                        .caps()
                        .and_then(|caps| gst_video::VideoInfo::from_caps(caps).ok())
                    else {
                        error!("Got invalid caps");
                        return Err(gst::FlowError::NotNegotiated);
                    };

                    let next_frame_ref = next_frame_ref.clone();
                    *next_frame_ref.lock().unwrap() = Some((info, buffer));

                    next_frame_available_notifier();

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        Ok(())
    }

    /// -> (texture id, [width, height])
    pub fn fetch_next_frame_as_texture(&self) -> Option<(NonZero<u32>, [u32; 2])> {
        if let Some((info, buffer)) = self.next_frame.lock().unwrap().take() {
            let sync_meta = buffer.meta::<gst_gl::GLSyncMeta>().unwrap();
            sync_meta.wait(self.gst_gl_context.as_ref().unwrap());

            if let Ok(frame) = gst_gl::GLVideoFrame::from_buffer_readable(buffer, &info) {
                *self.current_frame.lock().unwrap() = Some(frame);
            }
        }

        self.current_frame
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|frame| {
                frame
                    .texture_id(0)
                    .ok()
                    .and_then(|id| id.try_into().ok())
                    .map(|texture| (frame, texture))
            })
            .map(|(frame, texture)| (texture, [frame.width(), frame.height()]))
    }

    pub fn fetch_next_frame(&self) -> Option<slint::Image> {
        self.fetch_next_frame_as_texture()
            .map(|(texture, size)| unsafe {
                slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(texture, size.into())
                    .build()
            })
    }

    pub fn deactivate_and_pause(&self) -> Result<()> {
        self.current_frame.lock().unwrap().take();
        self.next_frame.lock().unwrap().take();

        if let Some(context) = &self.gst_gl_context {
            context.activate(false)?
        }

        Ok(())
    }
}
