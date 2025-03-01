// Copyright (C) 2024-2025 Marcus L. Hanestad <marlhan@proton.me>

use std::sync::LazyLock;
use std::sync::Mutex;

use gst::glib;
use gst::prelude::*;

use gst_base::prelude::BaseSrcExt;
use gst_base::subclass::base_src::CreateSuccess;
use gst_base::subclass::prelude::*;
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

struct FrameInfo {
    width: u32,
    height: u32,
    gst_v_format: gst_video::VideoFormat,
    pts: u64,
}

macro_rules! frame_info {
    ($frame:expr, $gst_fmt:expr) => {
        FrameInfo {
            width: $frame.width as u32,
            height: $frame.height as u32,
            gst_v_format: $gst_fmt,
            pts: $frame.display_time,
        }
    };
}

impl FrameInfo {
    pub fn new(frame: &scap::frame::Frame) -> Option<Self> {
        Some(match frame {
            scap::frame::Frame::RGB(f) => frame_info!(f, gst_video::VideoFormat::Rgb),
            scap::frame::Frame::RGBx(f) => frame_info!(f, gst_video::VideoFormat::Rgbx),
            scap::frame::Frame::XBGR(f) => frame_info!(f, gst_video::VideoFormat::Xbgr),
            scap::frame::Frame::BGRx(f) => frame_info!(f, gst_video::VideoFormat::Bgrx),
            scap::frame::Frame::BGR0(f) => frame_info!(f, gst_video::VideoFormat::Bgrx),
            scap::frame::Frame::BGRA(f) => frame_info!(f, gst_video::VideoFormat::Bgra),
            _ => return None,
        })
    }
}

struct Settings {
    pub show_cursor: bool,
    pub fps: u32,
    pub perform_internal_preroll: bool,
    // pub sel_target_cb: Option<glib::Closure>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_cursor: DEFAULT_SHOW_CURSOR,
            fps: DEFAULT_FPS,
            perform_internal_preroll: DEFAULT_PERFORM_INTERNAL_PREROLL,
            // sel_target_cb: None,
        }
    }
}

#[derive(Default)]
struct State {
    info: Option<gst_video::VideoInfo>,
    width: i32,
    height: i32,
    base_time: u64,
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
    fn ensure_correct_format(&self, frame_info: &FrameInfo) -> Result<(), gst::FlowError> {
        let state = self.state.lock().unwrap();

        let info = match &state.info {
            Some(i) => i,
            None => return Err(gst::FlowError::NotNegotiated),
        };

        if (state.width, state.height) != (frame_info.width as i32, frame_info.height as i32)
            || info.format() != frame_info.gst_v_format
        {
            gst::debug!(
                CAT,
                imp = self,
                "Resolutions differ. Will try to renegotiate"
            );

            let new_video_info = gst_video::VideoInfo::builder(
                frame_info.gst_v_format,
                frame_info.width,
                frame_info.height,
            )
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
                // glib::ParamSpecBoxed::builder::<Option<glib::Closure>>("select-target-cb")
                //     .nick("Select target callback")
                //     .blurb("Function that accepts a list of targets and returns the target that should be captured")
                //     .mutable_ready()
                //     .build(),
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
            // "select-target-cb" => {
            //     let mut settings = self.settings.lock().unwrap();
            //     let new_cb = value.get().expect("type checked upstream");

            //     gst::info!(CAT, imp = self, "Changing select-target-cb");

            //     settings.sel_target_cb = new_cb;
            // }
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
            // "select-target-cb" => {
            //     let settings = self.settings.lock().unwrap();
            //     settings.sel_target_cb.to_value()
            // }
            _ => unimplemented!(),
        }
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

        // TODO: Use settings.sel_target_cb to select the target
        // let targets = scap::get_all_targets();
        // if targets.is_empty() {
        //     return Err(gst::error_msg!(gst::LibraryError::Init, [
        //         "No targets available"
        //     ]));
        // }

        let mut new_capturer = Capturer::build(scap::capturer::Options {
            fps: settings.fps,
            show_cursor: settings.show_cursor,
            show_highlight: true,
            target: None,
            crop_area: None,
            output_type: scap::frame::FrameType::BGR0,
            output_resolution: scap::capturer::Resolution::Captured,
            excluded_targets: None,
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
            let frame_info = FrameInfo::new(&frame).unwrap();
            let video_info = gst_video::VideoInfo::builder(
                frame_info.gst_v_format,
                frame_info.width,
                frame_info.height,
            )
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
            state.base_time = frame_info.pts;
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

        let Some(frame_info) = FrameInfo::new(&frame) else {
            gst::element_error!(
                self.obj(),
                gst::ResourceError::Failed,
                ("Unsupported frame format received")
            );
            return Err(gst::FlowError::Error);
        };

        self.ensure_correct_format(&frame_info)?;

        let mut buffer = match frame {
            scap::frame::Frame::RGB(f) => gst::Buffer::from_slice(f.data),
            scap::frame::Frame::RGBx(f) => gst::Buffer::from_slice(f.data),
            scap::frame::Frame::XBGR(f) => gst::Buffer::from_slice(f.data),
            scap::frame::Frame::BGRx(f) => gst::Buffer::from_slice(f.data),
            scap::frame::Frame::BGR0(f) => gst::Buffer::from_slice(f.data),
            scap::frame::Frame::BGRA(f) => gst::Buffer::from_slice(f.data),
            _ => unreachable!(), // Yuv format should already have returned an error
        };

        let mut state = self.state.lock().unwrap();
        if state.base_time == u64::default() {
            state.base_time = frame_info.pts;
        }

        let pts = frame_info.pts - state.base_time;

        let buf = buffer.get_mut().unwrap();
        buf.set_pts(gst::ClockTime::from_nseconds(pts));

        Ok(CreateSuccess::NewBuffer(buffer))
    }
}
