use std::sync::{Arc, Mutex};

use gst::glib;
use gst::prelude::*;
use log::{debug, error};
use sink::TransmissionSink;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::Event;

mod sink;

use sink::{hls::HlsSink, webrtc::WebrtcSink};

use gst_gl::prelude::*;

// Taken from the slint gstreamer example at: https://github.com/slint-ui/slint/blob/2edd97bf8b8dc4dc26b578df6b15ea3297447444/examples/gstreamer-player/egl_integration.rs
pub struct SlintOpenGLSink {
    appsink: gst_app::AppSink,
    glsink: gst::Element,
    next_frame: Arc<Mutex<Option<(gst_video::VideoInfo, gst::Buffer)>>>,
    current_frame: Mutex<Option<gst_gl::GLVideoFrame<gst_gl::gl_video_frame::Readable>>>,
    gst_gl_context: Option<gst_gl::GLContext>,
}

impl SlintOpenGLSink {
    pub fn new() -> Self {
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
            .build()
            .expect("Fatal: Unable to create glsink");

        Self {
            appsink,
            glsink,
            next_frame: Default::default(),
            current_frame: Default::default(),
            gst_gl_context: None,
        }
    }

    pub fn video_sink(&self) -> gst::Element {
        self.glsink.clone().upcast()
    }

    pub fn connect(
        &mut self,
        graphics_api: &slint::GraphicsAPI<'_>,
        next_frame_available_notifier: Box<dyn Fn() + Send>,
    ) -> (gst_gl::GLContext, gst_gl_egl::GLDisplayEGL) {
        let egl = match graphics_api {
            slint::GraphicsAPI::NativeOpenGL { get_proc_address } => {
                glutin_egl_sys::egl::Egl::load_with(|symbol| {
                    get_proc_address(&std::ffi::CString::new(symbol).unwrap())
                })
            }
            _ => panic!("unsupported graphics API"),
        };

        let (gst_gl_context, gst_gl_display) = unsafe {
            let platform = gst_gl::GLPlatform::EGL;

            let egl_display = egl.GetCurrentDisplay();
            let display = gst_gl_egl::GLDisplayEGL::with_egl_display(egl_display as usize).unwrap();
            let native_context = egl.GetCurrentContext();

            (
                gst_gl::GLContext::new_wrapped(
                    &display,
                    native_context as _,
                    platform,
                    gst_gl::GLContext::current_gl_api(platform).0,
                )
                .expect("unable to create wrapped GL context"),
                display,
            )
        };

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

        (gst_gl_context, gst_gl_display)
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

pub struct Pipeline {
    inner: gst::Pipeline,
    tx_sink: Arc<tokio::sync::Mutex<Option<Box<dyn TransmissionSink>>>>,
    tee: gst::Element,
    preview_queue: gst::Element,
    preview_appsink: gst::Element,
}

impl Pipeline {
    pub fn new(
        event_tx: Sender<Event>,
        selected_rx: Receiver<usize>,
        preview_appsink: gst::Element,
        gst_egl_context: Arc<Mutex<Option<(gst_gl::GLContext, gst_gl_egl::GLDisplayEGL)>>>,
    ) -> Result<Self, gst::glib::BoolError> {
        let tee = gst::ElementFactory::make("tee").build()?;
        let src = gst::ElementFactory::make("scapsrc")
            .property("perform-internal-preroll", true)
            .build()?;
        let preview_queue = gst::ElementFactory::make("queue")
            .name("preview_queue")
            .property("max-size-time", 0u64)
            .property("max-size-buffers", 0u32)
            .property("max-size-bytes", 0u32)
            .property_from_str("leaky", "downstream")
            .property("silent", true) // Don't emit signals, can give better perf.
            .build()?;

        let pipeline = gst::Pipeline::new();

        let tx_sink = Arc::new(tokio::sync::Mutex::new(None::<Box<dyn TransmissionSink>>));

        let bus = pipeline.bus().unwrap();

        let pipeline_weak = pipeline.downgrade();
        let tx_sink_clone = Arc::clone(&tx_sink);
        bus.set_sync_handler(move |_, msg| {
            use gst::MessageView;

            match msg.view() {
                MessageView::StateChanged(state_changed) => {
                    let Some(pipeline) = pipeline_weak.upgrade() else {
                        return gst::BusSyncReply::Drop;
                    };

                    if state_changed.src() == Some(pipeline.upcast_ref())
                        && state_changed.old() == gst::State::Paused
                        && state_changed.current() == gst::State::Playing
                    {
                        let tx_sink = tx_sink_clone.clone();
                        common::runtime().spawn(async move {
                            let mut s = tx_sink.lock().await;
                            if let Some(ref mut sink) = *s {
                                sink.playing().await;
                            }
                        });
                    }
                }
                MessageView::Eos(..) => (), // app.quit()), TODO
                MessageView::Error(err) => {
                    error!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    // app.quit(); TODO
                }
                MessageView::NeedContext(ctx) => {
                    let ctx_type = ctx.context_type();

                    if ctx_type == *gst_gl::GL_DISPLAY_CONTEXT_TYPE {
                        if let Some(element) = msg
                            .src()
                            .and_then(|source| source.downcast_ref::<gst::Element>())
                        {
                            let g = gst_egl_context.lock().unwrap();
                            if let Some((_, gst_gl_display)) = g.as_ref() {
                                let gst_context = gst::Context::new(ctx_type, true);
                                gst_context.set_gl_display(gst_gl_display);
                                element.set_context(&gst_context);
                            }
                        }
                    } else if ctx_type == "gst.gl.app_context" {
                        if let Some(element) = msg
                            .src()
                            .and_then(|source| source.downcast_ref::<gst::Element>())
                        {
                            let mut gst_context = gst::Context::new(ctx_type, true);
                            {
                                let g = gst_egl_context.lock().unwrap();
                                if let Some((gst_gl_context, _)) = g.as_ref() {
                                    let gst_context = gst_context.get_mut().unwrap();
                                    let structure = gst_context.structure_mut();
                                    structure.set("context", gst_gl_context);
                                }
                            }
                            element.set_context(&gst_context);
                        }
                    }
                }
                _ => (),
            };

            gst::BusSyncReply::Drop
        });

        // https://gitlab.freedesktop.org/gstreamer/gstreamer/-/issues/3993
        src.static_pad("src").unwrap().add_probe(
            gst::PadProbeType::QUERY_UPSTREAM.union(gst::PadProbeType::PUSH),
            |_pad, info| match info.query_mut().map(|query| query.view_mut()) {
                Some(gst::QueryViewMut::Latency(latency)) => {
                    let (_live, min, max) = latency.result();
                    latency.set(false, min, max);
                    gst::PadProbeReturn::Handled
                }
                _ => gst::PadProbeReturn::Pass,
            },
        );

        let selected_rx = Arc::new(Mutex::new(selected_rx));
        let event_tx_clone = event_tx.clone();
        src.connect("select-source", false, move |vals| {
            let event_tx = event_tx_clone.clone();
            let selected_rx = Arc::clone(&selected_rx);

            let sources = vals[1].get::<Vec<String>>().unwrap();
            event_tx.blocking_send(Event::Sources(sources)).unwrap();
            let mut selected_rx = selected_rx.lock().unwrap();
            let res = selected_rx.blocking_recv().unwrap() as u64;
            Some(res.to_value())
        });

        pipeline.add_many([&src, &tee, &preview_queue, &preview_appsink])?;

        gst::Element::link_many([&src, &tee])?;
        gst::Element::link_many([&preview_queue, &preview_appsink])?;

        let tee_preview_pad = tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let queue_preview_pad = preview_queue
            .static_pad("sink")
            .map_or_else(|| Err(glib::bool_error!("`static_pad()` failed")), Ok)?;
        tee_preview_pad
            .link(&queue_preview_pad)
            .map_err(|err| glib::bool_error!("{err}"))?;

        let _ = std::thread::spawn({
            let pipeline = pipeline.clone();
            move || {
                debug!("Starting pipeline");
                pipeline.set_state(gst::State::Playing).unwrap();
            }
        });

        Ok(Self {
            inner: pipeline,
            tx_sink,
            tee,
            preview_queue,
            preview_appsink,
        })
    }

    pub async fn shutdown(&self) {
        self.inner.set_state(gst::State::Null).unwrap();
        self.preview_queue.unlink(&self.preview_appsink);
        self.inner.remove(&self.preview_appsink).unwrap();

        let mut tx_sink = self.tx_sink.lock().await;
        if let Some(sink) = &mut (*tx_sink) {
            sink.shutdown();
        }
    }

    pub async fn add_webrtc_sink(
        &mut self,
    ) -> Result<(), glib::BoolError> {
        let tee_pad = self.tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let webrtc = WebrtcSink::new(&self.inner, tee_pad)?;
        let mut s = self.tx_sink.lock().await;
        *s = Some(Box::new(webrtc));

        debug!("Added WebRTC sink");

        Ok(())
    }

    pub async fn add_hls_sink(&mut self) -> Result<(), glib::BoolError> {
        let tee_pad = self.tee.request_pad_simple("src_%u").map_or_else(
            || Err(glib::bool_error!("`request_pad_simple()` failed")),
            Ok,
        )?;
        let hls = HlsSink::new(&self.inner, tee_pad)?;
        let mut s = self.tx_sink.lock().await;
        *s = Some(Box::new(hls));

        debug!("Added HLS sink");

        Ok(())
    }

    pub async fn remove_transmission_sink(&self) {
        let mut tx_sink = self.tx_sink.lock().await;
        if let Some(sink) = &mut (*tx_sink) {
            sink.shutdown();
            sink.unlink(&self.inner).unwrap();
        }
        *tx_sink = None;
    }

    pub async fn get_play_msg(&self) -> Option<crate::Message> {
        let tx_sink = self.tx_sink.lock().await;
        if let Some(sink) = &(*tx_sink) {
            sink.get_play_msg()
        } else {
            None
        }
    }
}
