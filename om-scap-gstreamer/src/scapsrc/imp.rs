// Copyright (C) 2024-2025 Marcus L. Hanestad <marlhan@proton.me>

use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;

use gst::glib;
use gst::prelude::*;

use gst_base::prelude::BaseSrcExt;
use gst_base::subclass::base_src::CreateSuccess;
use gst_base::subclass::prelude::*;
use gst_video::VideoFrameExt;
use scap::capturer::Capturer;
use scap::capturer::Pts;
use scap::frame::FrameInfo;

const DEFAULT_FPS: u32 = 25;
const DEFAULT_SHOW_CURSOR: bool = true;
const DEFAULT_PERFORM_INTERNAL_PREROLL: bool = false;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "scapsrc",
        gst::DebugColorFlags::empty(),
        Some("Scap screencast source"),
    )
});

enum Event {
    NewCaps(gst::Caps),
    Frame(gst::Buffer),
}

#[inline]
const fn frame_format_to_gst(format: &scap::frame::FrameFormat) -> gst_video::VideoFormat {
    match format {
        scap::frame::FrameFormat::RGBx => gst_video::VideoFormat::Rgbx,
        scap::frame::FrameFormat::XBGR => gst_video::VideoFormat::Xbgr,
        scap::frame::FrameFormat::BGRx => gst_video::VideoFormat::Bgrx,
        scap::frame::FrameFormat::BGRA => gst_video::VideoFormat::Bgra,
        scap::frame::FrameFormat::RGBA => gst_video::VideoFormat::Rgba,
    }
}

struct Settings {
    pub show_cursor: bool,
    pub fps: u32,
    pub perform_internal_preroll: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_cursor: DEFAULT_SHOW_CURSOR,
            fps: DEFAULT_FPS,
            perform_internal_preroll: DEFAULT_PERFORM_INTERNAL_PREROLL,
        }
    }
}

#[derive(Default)]
struct State {
    info: Option<gst_video::VideoInfo>,
    width: i32,
    height: i32,
}

pub struct ScapSrc {
    settings: Mutex<Settings>,
    capturer: Mutex<Option<Capturer>>,
    state: Arc<Mutex<State>>,
    event_rx: Mutex<Option<Receiver<Event>>>,
    buffer_pool: Arc<Mutex<gst::BufferPool>>,
}

impl Default for ScapSrc {
    fn default() -> Self {
        Self {
            settings: Mutex::new(Default::default()),
            capturer: Mutex::new(None),
            state: Arc::new(Mutex::new(Default::default())),
            event_rx: Mutex::new(None),
            buffer_pool: Arc::new(Mutex::new(gst_video::VideoBufferPool::new().upcast())),
        }
    }
}

#[glib::object_subclass]
impl ObjectSubclass for ScapSrc {
    const NAME: &'static str = "ScapSrc";
    type Type = super::ScapSrc;
    type ParentType = gst_base::PushSrc;
}

impl ObjectImpl for ScapSrc {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
            vec![
                glib::ParamSpecUInt::builder("fps")
                    .nick("Frames per second")
                    .blurb("Rate to capture screen at")
                    .minimum(1)
                    .default_value(DEFAULT_FPS)
                    .mutable_ready()
                    .build(),
                glib::ParamSpecBoolean::builder("show-cursor")
                    .nick("Show cursor")
                    .blurb("Whether to capture the cursor or not")
                    .default_value(DEFAULT_SHOW_CURSOR)
                    .mutable_ready()
                    .build(),
                glib::ParamSpecBoolean::builder("perform-internal-preroll")
                    .nick("Perform internal preroll")
                    .blurb("Pull one frame from the capture source before format negotiation")
                    .default_value(DEFAULT_PERFORM_INTERNAL_PREROLL)
                    .mutable_ready()
                    .build(),
            ]
        });

        &PROPERTIES
    }

    fn constructed(&self) {
        self.parent_constructed();

        let obj = self.obj();
        obj.set_live(true);
        obj.set_format(gst::Format::Time);
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        match pspec.name() {
            "fps" => {
                let mut settings = self.settings.lock().unwrap();
                let new_fps = value.get().expect("type checked upstream");

                gst::info!(
                    CAT,
                    imp = self,
                    "fps was changed from `{}` to `{}`",
                    settings.fps,
                    new_fps
                );

                settings.fps = new_fps;
            }
            "show-cursor" => {
                let mut settings = self.settings.lock().unwrap();
                let new_show_cursor = value.get().expect("type checked upstream");

                gst::info!(
                    CAT,
                    imp = self,
                    "show-cursor was changed from `{}` to `{}`",
                    settings.show_cursor,
                    new_show_cursor
                );

                settings.show_cursor = new_show_cursor;
            }
            "perform-internal-preroll" => {
                let mut settings = self.settings.lock().unwrap();
                let new_perf_internal_preroll = value.get().expect("type checked upstream");

                gst::info!(
                    CAT,
                    imp = self,
                    "perform-internal-preroll was changed from `{}` to `{}`",
                    settings.perform_internal_preroll,
                    new_perf_internal_preroll,
                );

                settings.perform_internal_preroll = new_perf_internal_preroll;
            }
            _ => unimplemented!(),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "fps" => {
                let settings = self.settings.lock().unwrap();
                settings.fps.to_value()
            }
            "show-cursor" => {
                let settings = self.settings.lock().unwrap();
                settings.show_cursor.to_value()
            }
            "perform-internal-preroll" => {
                let settings = self.settings.lock().unwrap();
                settings.perform_internal_preroll.to_value()
            }
            _ => unimplemented!(),
        }
    }

    fn signals() -> &'static [glib::subclass::Signal] {
        static SIGNALS: LazyLock<Vec<glib::subclass::Signal>> = LazyLock::new(|| {
            vec![glib::subclass::Signal::builder("select-source")
                .param_types([Vec::<String>::static_type()])
                .return_type::<u64>()
                .build()]
        });

        SIGNALS.as_ref()
    }
}

impl GstObjectImpl for ScapSrc {}

impl ElementImpl for ScapSrc {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "Scap screencast source",
                "Source/Video",
                "Scap screencast source",
                "Marcus L. Hanestad <marlhan@proton.me>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static PAD_TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            let caps = gst_video::VideoCapsBuilder::new()
                .format_list([
                    gst_video::VideoFormat::Rgbx,
                    gst_video::VideoFormat::Xbgr,
                    gst_video::VideoFormat::Bgrx,
                    gst_video::VideoFormat::Bgrx,
                    gst_video::VideoFormat::Bgra,
                ])
                .build();
            let src_pad_template = gst::PadTemplate::new(
                "src",
                gst::PadDirection::Src,
                gst::PadPresence::Always,
                &caps,
            )
            .unwrap();

            vec![src_pad_template]
        });

        &PAD_TEMPLATES
    }

    fn change_state(
        &self,
        transition: gst::StateChange,
    ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
        gst::debug!(CAT, imp = self, "State transition: {transition:?}");

        let mut res = self.parent_change_state(transition)?;

        match transition {
            gst::StateChange::ReadyToPaused => res = gst::StateChangeSuccess::NoPreroll,
            gst::StateChange::PausedToPlaying => {
                let mut capturer = self.capturer.lock().unwrap();
                match &mut *capturer {
                    Some(c) => c.start_capture(),
                    None => {
                        gst::error!(CAT, imp = self, "Capturer is missing");
                        return Err(gst::StateChangeError);
                    }
                }
                gst::info!(CAT, imp = self, "Capturing engine was started");
            }
            _ => (),
        }

        Ok(res)
    }
}

impl BaseSrcImpl for ScapSrc {
    fn start(&self) -> Result<(), gst::ErrorMessage> {
        let mut capturer = self.capturer.lock().unwrap();
        let settings = self.settings.lock().unwrap();

        if let Some(mut capturer) = capturer.take() {
            gst::debug!(CAT, imp = self, "Capturer exists, stopping");
            capturer.stop_capture();
        }

        let targets = scap::get_all_targets();
        if targets.is_empty() {
            gst::error!(CAT, imp = self, "No sources available to capture");
            return Err(gst::error_msg!(
                gst::LibraryError::Init,
                ["No sources available to capture"]
            ));
        }

        let source_idx = self.obj().emit_by_name::<u64>(
            "select-source",
            &[&targets.iter().map(|t| t.title()).collect::<Vec<String>>()],
        );

        let (event_tx, event_rx) = std::sync::mpsc::sync_channel::<Event>(10);

        let event_tx_clone = event_tx.clone();
        let on_format_changed = move |new_format: FrameInfo| {
            let gst_v_format = frame_format_to_gst(&new_format.format);
            let new_video_info =
                gst_video::VideoInfo::builder(gst_v_format, new_format.width, new_format.height)
                    .build()
                    .unwrap();

            let new_caps = new_video_info.to_caps().unwrap();

            event_tx_clone.send(Event::NewCaps(new_caps)).unwrap();
        };

        let buffer_pool_clone = Arc::clone(&self.buffer_pool);
        let state_clone = Arc::clone(&self.state);
        let on_frame = move |pts: Pts, data: &[u8]| {
            let buffer_pool = buffer_pool_clone.lock().unwrap();
            let Ok(buffer) = buffer_pool.acquire_buffer(None) else {
                // TODO: logging
                return;
            };
            drop(buffer_pool);

            let state = state_clone.lock().unwrap();
            let mut vframe =
                gst_video::VideoFrame::from_buffer_writable(buffer, state.info.as_ref().unwrap())
                    .unwrap();
            drop(state);

            let dest_stride = vframe.plane_stride()[0] as usize;
            let dest = vframe.plane_data_mut(0).unwrap();

            for (dest, src) in dest
                .chunks_exact_mut(dest_stride)
                .zip(data.chunks_exact(dest_stride))
            {
                dest[..dest_stride].copy_from_slice(&src[..dest_stride]);
            }

            let mut buffer = vframe.into_buffer();
            buffer
                .get_mut()
                .unwrap()
                .set_pts(gst::ClockTime::from_nseconds(pts));

            //event_tx.send(Event::Frame(buffer)).unwrap();
            // If the channel is full, just drop the frame
            // TODO: handle error
            let _ = event_tx.try_send(Event::Frame(buffer));
        };

        let mut new_capturer = Capturer::build(
            scap::capturer::Options {
                fps: settings.fps,
                show_cursor: settings.show_cursor,
                show_highlight: true,
                target: Some(targets[source_idx as usize].clone()),
                output_resolution: scap::capturer::Resolution::Captured,
            },
            Box::new(on_format_changed),
            Box::new(on_frame),
        )
        .map_err(|err| gst::error_msg!(gst::LibraryError::Init, ["{err}"]))?;

        if settings.perform_internal_preroll {
            // deadlock prevention
            drop(settings);

            gst::info!(CAT, imp = self, "Performing internal preroll");
            new_capturer.start_capture();
            match event_rx.recv().map_err(|err| {
                gst::error_msg!(gst::LibraryError::Init, ["Failed to get format: {err}"])
            })? {
                Event::NewCaps(caps) => self.obj().set_caps(&caps).map_err(|_| {
                    gst::error_msg!(
                        gst::LibraryError::Init,
                        ["Failed to set caps while performing internal preroll"]
                    )
                })?,
                Event::Frame(_) => unreachable!(),
            }
        }

        let mut rx = self.event_rx.lock().unwrap();
        *rx = Some(event_rx);

        *capturer = Some(new_capturer);

        gst::debug!(CAT, imp = self, "Capturer created");

        Ok(())
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        let mut capturer = self.capturer.lock().unwrap().take().ok_or(gst::error_msg!(
            gst::LibraryError::Shutdown,
            ["Missing capturer"]
        ))?;

        capturer.stop_capture();

        Ok(())
    }

    fn set_caps(&self, caps: &gst::Caps) -> Result<(), gst::LoggableError> {
        let info = gst_video::VideoInfo::from_caps(caps).map_err(|_| {
            gst::loggable_error!(CAT, "Failed to build `VideoInfo` from caps {}", caps)
        })?;

        gst::debug!(CAT, imp = self, "Configuring for caps {}", caps);

        let mut buffer_pool = self.buffer_pool.lock().unwrap();
        let config = buffer_pool.config();
        let params = config.params().unwrap();
        let now_size = info.size();
        if params.0 != Some(info.to_caps()?) || params.1 as usize != now_size {
            buffer_pool.set_active(false)?;
            let new_pool = gst_video::VideoBufferPool::new();
            let mut config = new_pool.config();
            config.set_params(Some(&info.to_caps()?), info.size() as u32, 0, 0);
            new_pool.set_config(config)?;
            new_pool.set_active(true)?;
            *buffer_pool = new_pool.upcast();
        }

        let (new_width, new_height) = (info.width(), info.height());
        self.obj().set_blocksize(4 * new_width * new_height);

        let mut state = self.state.lock().unwrap();
        state.info = Some(info);
        state.width = new_width as i32;
        state.height = new_height as i32;

        Ok(())
    }

    fn query(&self, query: &mut gst::QueryRef) -> bool {
        use gst::QueryViewMut;
        let settings = self.settings.lock().unwrap();

        match query.view_mut() {
            QueryViewMut::Caps(q) if settings.perform_internal_preroll => {
                gst::info!(CAT, imp = self, "Returning caps");
                let state = self.state.lock().unwrap();
                if let Some(info) = &state.info.as_ref() {
                    q.set_result(Some(&info.to_caps().unwrap()));
                    true
                } else {
                    false
                }
            }
            _ => {
                drop(settings);
                BaseSrcImplExt::parent_query(self, query)
            }
        }
    }
}

impl PushSrcImpl for ScapSrc {
    fn create(&self, _: Option<&mut gst::BufferRef>) -> Result<CreateSuccess, gst::FlowError> {
        let rx = self.event_rx.lock().map_err(|_| gst::FlowError::Error)?;
        let rx = rx.as_ref().ok_or(gst::FlowError::Error)?;

        loop {
            match rx.recv().map_err(|_| gst::FlowError::Error)? {
                Event::NewCaps(caps) => self
                    .obj()
                    .set_caps(&caps)
                    .map_err(|_| gst::FlowError::Error)?,
                Event::Frame(buffer) => return Ok(CreateSuccess::NewBuffer(buffer)),
            }
        }
    }
}
