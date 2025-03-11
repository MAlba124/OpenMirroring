// Copyright (C) 2024-2025 Marcus L. Hanestad <marlhan@proton.me>

use std::sync::LazyLock;
use std::sync::Mutex;

use gst::glib;
use gst::prelude::*;

use gst_base::prelude::BaseSrcExt;
use gst_base::subclass::base_src::CreateSuccess;
use gst_base::subclass::prelude::*;
use gst_video::VideoFrameExt;
use scap::capturer::Capturer;

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

fn frame_format_to_gst(format: &scap::frame::FrameFormat) -> gst_video::VideoFormat {
    match format {
        scap::frame::FrameFormat::RGBx => gst_video::VideoFormat::Rgbx,
        scap::frame::FrameFormat::XBGR => gst_video::VideoFormat::Xbgr,
        scap::frame::FrameFormat::BGRx => gst_video::VideoFormat::Bgrx,
        scap::frame::FrameFormat::BGRA => gst_video::VideoFormat::Bgra,
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
    base_time: u64,
    buffer_pool: Option<gst::BufferPool>,
}

pub struct ScapSrc {
    settings: Mutex<Settings>,
    capturer: Mutex<Option<Capturer>>,
    state: Mutex<State>,
}

impl Default for ScapSrc {
    fn default() -> Self {
        Self {
            settings: Mutex::new(Default::default()),
            capturer: Mutex::new(None),
            state: Mutex::new(Default::default()),
        }
    }
}

impl ScapSrc {
    fn ensure_correct_format(&self, frame_info: &scap::frame::Frame) -> Result<(), gst::FlowError> {
        let state = self.state.lock().unwrap();

        let info = match &state.info {
            Some(i) => i,
            None => return Err(gst::FlowError::NotNegotiated),
        };

        let gst_v_format = frame_format_to_gst(&frame_info.format);

        if (state.width, state.height) != (frame_info.width as i32, frame_info.height as i32)
            || info.format() != gst_v_format
        {
            gst::debug!(
                CAT,
                imp = self,
                "Resolutions differ. Will try to renegotiate"
            );

            let new_video_info =
                gst_video::VideoInfo::builder(gst_v_format, frame_info.width, frame_info.height)
                    .build()
                    .map_err(|err| {
                        gst::error!(CAT, imp = self, "Failed to create video info: {err}");
                        gst::FlowError::Error
                    })?;

            let new_caps = new_video_info.to_caps().map_err(|err| {
                gst::error!(CAT, imp = self, "Failed to create caps: {err}");
                gst::FlowError::Error
            })?;

            // Deadlock prevention
            drop(state);

            if let Err(err) = self.obj().set_caps(&new_caps) {
                gst::error!(CAT, imp = self, "Failed to set caps: {err}");
                return Err(gst::FlowError::Error);
            }
        }

        Ok(())
    }

    // https://github.com/sdroege/gst-plugin-rs/blob/826018a3937745d3a921f21da715fba164f57a14/net/ndi/src/ndisrcdemux/imp.rs#L570
    fn create_buffer_pool(&self, video_info: &gst_video::VideoInfo) -> gst::BufferPool {
        let pool = gst_video::VideoBufferPool::new();
        let mut config = pool.config();
        config.set_params(
            Some(&video_info.to_caps().unwrap()),
            video_info.size() as u32,
            0,
            0,
        );
        pool.set_config(config).unwrap();
        pool.set_active(true).unwrap();

        pool.upcast()
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
                    gst_video::VideoFormat::Rgb,
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
            gst::StateChange::NullToReady => {}
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
            gst::StateChange::PlayingToPaused => {}
            gst::StateChange::PausedToReady => {}
            gst::StateChange::ReadyToNull => {}
            gst::StateChange::NullToNull => {}
            gst::StateChange::ReadyToReady => {}
            gst::StateChange::PausedToPaused => {}
            gst::StateChange::PlayingToPlaying => {}
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
            gst::error!(CAT, imp = self, "No capture sources available");
            return Err(gst::error_msg!(
                gst::LibraryError::Init,
                ["No capture sources available"]
            ));
        }

        let source_idx = self.obj().emit_by_name::<u64>(
            "select-source",
            &[&targets.iter().map(|t| t.title()).collect::<Vec<String>>()],
        );

        let mut new_capturer = Capturer::build(scap::capturer::Options {
            fps: settings.fps,
            show_cursor: settings.show_cursor,
            show_highlight: true,
            target: Some(targets[source_idx as usize].clone()),
            output_resolution: scap::capturer::Resolution::Captured,
        })
        .map_err(|err| gst::error_msg!(gst::LibraryError::Init, ["{err}"]))?;

        if settings.perform_internal_preroll {
            gst::info!(CAT, imp = self, "Performing internal preroll");
            new_capturer.start_capture();
            let frame = new_capturer.get_next_frame().map_err(|err| {
                gst::error_msg!(
                    gst::LibraryError::Init,
                    ["Failed to perform internal preroll: {err}"]
                )
            })?;

            let gst_v_format = frame_format_to_gst(&frame.format);

            let video_info = gst_video::VideoInfo::builder(gst_v_format, frame.width, frame.height)
                .build()
                .map_err(|err| {
                    gst::error_msg!(
                        gst::LibraryError::Init,
                        ["Failed to create video info: {err}"]
                    )
                })?;

            // Deadlock prevention
            drop(settings);

            self.obj()
                .set_caps(
                    &video_info
                        .to_caps()
                        .map_err(|err| gst::error_msg!(gst::LibraryError::Init, ["{err}"]))?,
                )
                .map_err(|err| gst::error_msg!(gst::LibraryError::Init, ["{err}"]))?;

            let mut state = self.state.lock().unwrap();
            state.base_time = frame.display_time;
        }

        *capturer = Some(new_capturer);

        gst::debug!(CAT, imp = self, "Capturer created");

        Ok(())
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        match self.capturer.lock().unwrap().take() {
            Some(mut c) => c.stop_capture(),
            None => {
                return Err(gst::error_msg!(
                    gst::LibraryError::Shutdown,
                    ["Missing capturer"]
                ));
            }
        }

        Ok(())
    }

    fn set_caps(&self, caps: &gst::Caps) -> Result<(), gst::LoggableError> {
        let info = gst_video::VideoInfo::from_caps(caps).map_err(|_| {
            gst::loggable_error!(CAT, "Failed to build `VideoInfo` from caps {}", caps)
        })?;

        gst::debug!(CAT, imp = self, "Configuring for caps {}", caps);

        let (new_width, new_height) = (info.width(), info.height());

        self.obj().set_blocksize(4 * new_width * new_height);

        let mut state = self.state.lock().unwrap();

        match &state.buffer_pool {
            Some(pool) => {
                let config = pool.config();
                let params = config.params().unwrap();
                let now_size = info.size();
                if params.0 != Some(info.to_caps().unwrap()) || params.1 as usize != now_size {
                    let new_pool = self.create_buffer_pool(&info);
                    state.buffer_pool = Some(new_pool);
                }
            }
            None => {
                let pool = self.create_buffer_pool(&info);
                state.buffer_pool = Some(pool);
            }
        }

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
        let Some(ref cap) = *self.capturer.lock().unwrap() else {
            return Err(gst::FlowError::NotNegotiated);
        };

        let frame = cap.get_next_frame().map_err(|err| {
            gst::element_error!(
                self.obj(),
                gst::ResourceError::Read,
                ("Failed to get next frame: {err}")
            );
            gst::FlowError::Error
        })?;

        self.ensure_correct_format(&frame)?;

        let mut state = self.state.lock().unwrap();

        let mut buffer = match frame.data {
            scap::frame::FrameData::Vec(vec) => {
                let video_buffer_pool = state.buffer_pool.as_ref().unwrap();

                let gst_buffer = video_buffer_pool.acquire_buffer(None).unwrap();

                let mut vframe =
                    gst_video::VideoFrame::from_buffer_writable(gst_buffer, state.info.as_ref().unwrap())
                        .unwrap();

                let dest_stride = vframe.plane_stride()[0] as usize;
                let dest = vframe.plane_data_mut(0).unwrap();

                for (dest, src) in dest.chunks_exact_mut(dest_stride).zip(vec.chunks_exact(dest_stride)) {
                    dest[..dest_stride].copy_from_slice(&src[..dest_stride]);
                }

                vframe.into_buffer()
            },
            scap::frame::FrameData::PoolBuffer(mut buffer) => {
                let buffer = buffer.take().expect("Always Some");

                let video_buffer_pool = state.buffer_pool.as_ref().unwrap();

                let gst_buffer = video_buffer_pool.acquire_buffer(None).unwrap();

                let mut vframe =
                    gst_video::VideoFrame::from_buffer_writable(gst_buffer, state.info.as_ref().unwrap())
                        .unwrap();

                let dest_stride = vframe.plane_stride()[0] as usize;
                let dest = vframe.plane_data_mut(0).unwrap();

                for (dest, src) in dest.chunks_exact_mut(dest_stride).zip(buffer.data.chunks_exact(dest_stride)) {
                    dest[..dest_stride].copy_from_slice(&src[..dest_stride]);
                }

                let mut pool = cap.pool.lock().unwrap();
                pool.give_back(buffer);
                drop(pool);

                vframe.into_buffer()
            }
        };

        if state.base_time == u64::default() {
            state.base_time = frame.display_time;
        }

        let pts = frame.display_time - state.base_time;

        let buf = buffer.get_mut().unwrap();
        buf.set_pts(gst::ClockTime::from_nseconds(pts));

        Ok(CreateSuccess::NewBuffer(buffer))
    }
}
