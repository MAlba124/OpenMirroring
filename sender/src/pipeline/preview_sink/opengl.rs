use anyhow::Result;
use std::sync::{Arc, Mutex};

use gst_gl::prelude::*;
use log::error;

// Taken from the slint gstreamer example at: https://github.com/slint-ui/slint/blob/2edd97bf8b8dc4dc26b578df6b15ea3297447444/examples/gstreamer-player/egl_integration.rs
pub struct SlintOpenGLSink {
    appsink: gst_app::AppSink,
    glsink: gst::Element,
    next_frame: Arc<Mutex<Option<(gst_video::VideoInfo, gst::Buffer)>>>,
    current_frame: Mutex<Option<gst_gl::GLVideoFrame<gst_gl::gl_video_frame::Readable>>>,
    gst_gl_context: Option<gst_gl::GLContext>,
}

impl SlintOpenGLSink {
    pub fn new() -> Result<Self> {
        let appsink = gst_app::AppSink::builder()
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .features([gst_gl::CAPS_FEATURE_MEMORY_GL_MEMORY])
                    .format(gst_video::VideoFormat::Rgba)
                    .field("texture-target", "2D")
                    .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
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
            let display = gst_gl_egl::GLDisplayEGL::with_egl_display(egl_display as usize).unwrap();
            let native_context = egl.GetCurrentContext();

            Ok((
                gst_gl::GLContext::new_wrapped(
                    &display,
                    native_context as _,
                    platform,
                    gst_gl::GLContext::current_gl_api(platform).0,
                )
                .expect("unable to create wrapped GL context"),
                display.upcast(),
            ))
        }
    }

    #[cfg(target_os = "windows")]
    fn get_wgl_ctx(
        graphics_api: &slint::GraphicsAPI<'_>,
    ) -> Result<(gst_gl::GLContext, gst_gl::GLDisplay)> {
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

        let Some(wrapped_context) =
            (unsafe { gst_gl::GLContext::new_wrapped(&gst_display, gl_ctx, platform, gl_api) })
        else {
            bail!("Failed to create wrapped GL context");
        };

        Ok((wrapped_context, gst_display))
    }

    pub fn connect(
        &mut self,
        graphics_api: &slint::GraphicsAPI<'_>,
        next_frame_available_notifier: Box<dyn Fn() + Send>,
        // ) -> (gst_gl::GLContext, gst_gl_egl::GLDisplayEGL) {
    ) -> Result<(gst_gl::GLContext, gst_gl::GLDisplay)> {
        // let egl = match graphics_api {
        //     slint::GraphicsAPI::NativeOpenGL { get_proc_address } => {
        //         glutin_egl_sys::egl::Egl::load_with(|symbol| {
        //             get_proc_address(&std::ffi::CString::new(symbol).unwrap())
        //         })
        //     }
        //     _ => panic!("unsupported graphics API"),
        // };

        // let (gst_gl_context, gst_gl_display) = {
        //     #[cfg(target_os = "linux")]
        //     let platform = gst_gl::GLPlatform::EGL;
        //     // #[cfg(target_os = "windows")]
        //     {}

        //     let egl_display = unsafe { egl.GetCurrentDisplay() };
        //     let display = unsafe {
        //         gst_gl_egl::GLDisplayEGL::with_egl_display(egl_display as usize).unwrap()
        //     };
        //     let native_context = unsafe { egl.GetCurrentContext() };

        //     (
        //         unsafe {
        //             gst_gl::GLContext::new_wrapped(
        //                 &display,
        //                 native_context as _,
        //                 platform,
        //                 gst_gl::GLContext::current_gl_api(platform).0,
        //             )
        //             .expect("unable to create wrapped GL context")
        //         },
        //         display,
        //     )
        // };

        #[cfg(target_os = "linux")]
        let (gst_gl_context, gst_gl_display) = Self::get_egl_ctx(graphics_api)?;
        #[cfg(target_os = "windows")]
        let (gst_gl_context, gst_gl_display) = Self::get_wgl_ctx(graphics_api)?;

        gst_gl_context
            .activate(true)
            .expect("could not activate GStreamer GL context");
        gst_gl_context
            .fill_info()
            .expect("failed to fill GL info for wrapped context");

        self.gst_gl_context = Some(gst_gl_context.clone());

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

        Ok((gst_gl_context, gst_gl_display))
    }

    pub fn fetch_next_frame(&self) -> Option<slint::Image> {
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
            .map(|(frame, texture)| unsafe {
                slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(
                    texture,
                    [frame.width(), frame.height()].into(),
                )
                .build()
            })
    }

    pub fn deactivate_and_pause(&self) {
        self.current_frame.lock().unwrap().take();
        self.next_frame.lock().unwrap().take();

        if let Some(context) = &self.gst_gl_context {
            context
                .activate(false)
                .expect("could not activate GStreamer GL context");
        }
    }
}
