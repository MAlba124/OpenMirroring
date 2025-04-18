use std::{
    mem::size_of,
    sync::{
        atomic::{AtomicBool, AtomicU8},
        mpsc::{sync_channel, SyncSender},
        Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

use log::{debug, error, warn};
use pipewire::{self as pw, spa::buffer::DataType};
use pw::{
    context::Context,
    main_loop::MainLoop,
    properties::properties,
    spa::{
        self,
        param::{
            format::{FormatProperties, MediaSubtype, MediaType},
            video::VideoFormat,
            ParamType,
        },
        pod::{Pod, Property},
        sys::{
            spa_buffer, spa_meta_header, SPA_META_Header, SPA_PARAM_META_size, SPA_PARAM_META_type,
        },
        utils::{Direction, SpaTypes},
    },
    stream::{StreamRef, StreamState},
};

use crate::{
    capturer::{OnFormatChangedCb, OnFrameCb, Options},
    frame::FrameInfo,
    targets::get_main_display,
};

use super::{error::LinCapError, LinuxCapturerImpl};

struct ListenerUserData {
    pub stream_state_changed_to_error: Arc<AtomicBool>,
    pub on_format_changed: OnFormatChangedCb,
    pub on_frame: OnFrameCb,
    pub base_time: Option<u64>,
    pub format_height: usize,
}

fn serialize_obj(obj: spa::pod::Object) -> Vec<u8> {
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .unwrap()
    .0
    .into_inner()
}

fn param_changed_callback(
    stream: &StreamRef,
    user_data: &mut ListenerUserData,
    id: u32,
    param: Option<&Pod>,
) {
    let Some(param) = param else {
        return;
    };
    if id != spa::param::ParamType::Format.as_raw() {
        return;
    }
    let (media_type, media_subtype) = match spa::param::format_utils::parse_format(param) {
        Ok(v) => v,
        Err(_) => return,
    };

    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
        return;
    }

    let mut format = spa::param::video::VideoInfoRaw::new();
    format.parse(param).unwrap();

    let color_format = match format.format() {
        VideoFormat::RGBx => crate::frame::FrameFormat::RGBx,
        VideoFormat::xBGR => crate::frame::FrameFormat::XBGR,
        VideoFormat::BGRx => crate::frame::FrameFormat::BGRx,
        VideoFormat::RGBA => crate::frame::FrameFormat::RGBA,
        VideoFormat::BGRA => crate::frame::FrameFormat::BGRA,
        _ => unreachable!(),
    };

    let new_info = FrameInfo {
        format: color_format,
        width: format.size().width,
        height: format.size().height,
    };

    user_data.format_height = format.size().height as usize;

    debug!("New info: {new_info:?}");

    (user_data.on_format_changed)(new_info);

    let buf_obj = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamBuffers,
        spa::param::ParamType::Buffers,
        spa::pod::Property {
            key: spa::sys::SPA_PARAM_BUFFERS_dataType,
            flags: spa::pod::PropertyFlags::empty(),
            value: spa::pod::Value::Int(
                (1 << spa::buffer::DataType::MemFd.as_raw())
                    | (1 << spa::buffer::DataType::MemPtr.as_raw())
            )
        }
    );

    let buf_values = serialize_obj(buf_obj);

    let metas_obj = spa::pod::object!(
        SpaTypes::ObjectParamMeta,
        ParamType::Meta,
        Property::new(
            SPA_PARAM_META_type,
            spa::pod::Value::Id(spa::utils::Id(SPA_META_Header))
        ),
        Property::new(
            SPA_PARAM_META_size,
            spa::pod::Value::Int(size_of::<spa::sys::spa_meta_header>() as i32)
        ),
    );

    let metas_values = serialize_obj(metas_obj);

    let mut params = [
        spa::pod::Pod::from_bytes(&buf_values).unwrap(),
        spa::pod::Pod::from_bytes(&metas_values).unwrap(),
    ];

    stream.update_params(&mut params).unwrap();
}

fn state_changed_callback(
    _stream: &StreamRef,
    user_data: &mut ListenerUserData,
    _old: StreamState,
    new: StreamState,
) {
    if let StreamState::Error(e) = new {
        error!("State changed to error({e})");
        user_data
            .stream_state_changed_to_error
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

unsafe fn get_timestamp(buffer: *mut spa_buffer) -> i64 {
    let n_metas = (*buffer).n_metas;
    if n_metas > 0 {
        let mut meta_ptr = (*buffer).metas;
        let metas_end = (*buffer).metas.wrapping_add(n_metas as usize);
        while meta_ptr != metas_end {
            if (*meta_ptr).type_ == SPA_META_Header {
                let meta_header: &mut spa_meta_header =
                    &mut *((*meta_ptr).data as *mut spa_meta_header);
                return meta_header.pts;
            }
            meta_ptr = meta_ptr.wrapping_add(1);
        }
        0
    } else {
        0
    }
}

fn process_callback(stream: &StreamRef, user_data: &mut ListenerUserData) {
    let buffer = unsafe { stream.dequeue_raw_buffer() };
    if !buffer.is_null() {
        'outer: {
            let buffer = unsafe { (*buffer).buffer };
            if buffer.is_null() {
                break 'outer;
            }

            let mut timestamp = unsafe { get_timestamp(buffer) } as u64;

            if let Some(base_time) = user_data.base_time {
                timestamp -= base_time;
            } else {
                user_data.base_time = Some(timestamp);
                timestamp = 0;
            }

            let n_datas = unsafe { (*buffer).n_datas };
            if n_datas < 1 {
                break 'outer;
            }

            let datas = unsafe { (*buffer).datas as *mut spa::buffer::Data };
            if datas.is_null() {
                error!("Data is null");
                break 'outer;
            }
            let data = unsafe { std::slice::from_raw_parts_mut(datas, n_datas as usize) };
            let data = &mut data[0];
            match data.type_() {
                DataType::MemPtr => {
                    (user_data.on_frame)(timestamp, data.data().unwrap());
                }
                DataType::MemFd => {
                    let fd: std::os::fd::RawFd = data.as_raw().fd as i32;
                    let offset = data.chunk().offset() as i64;
                    let stride = data.chunk().stride();
                    let size = user_data.format_height * stride as usize;

                    let ptr = unsafe {
                        libc::mmap(
                            std::ptr::null_mut(),
                            size,
                            libc::PROT_READ,
                            libc::MAP_SHARED,
                            fd,
                            offset,
                        )
                    };

                    let data = unsafe { std::slice::from_raw_parts(ptr as *mut u8, size) };

                    (user_data.on_frame)(timestamp, data);

                    unsafe {
                        libc::munmap(ptr, size);
                    }
                }
                _ => warn!("Got data of type: {:?}, ignoring", data.type_()),
            }
        }
    } else {
        error!("Out of buffers");
    }

    unsafe { stream.queue_raw_buffer(buffer) };
}

fn pipewire_capturer(
    options: Options,
    ready_sender: &SyncSender<bool>,
    stream_id: u32,
    capturer_state: Arc<AtomicU8>,
    stream_state_changed_to_error: Arc<AtomicBool>,
    on_format_changed: OnFormatChangedCb,
    on_frame: OnFrameCb,
) -> Result<(), LinCapError> {
    pw::init();

    let mainloop = MainLoop::new(None)?;
    let context = Context::new(&mainloop)?;
    let core = context.connect(None)?;

    let user_data = ListenerUserData {
        stream_state_changed_to_error: Arc::clone(&stream_state_changed_to_error),
        on_format_changed,
        on_frame,
        base_time: None,
        format_height: 0,
    };

    let stream = pw::stream::Stream::new(
        &core,
        "scap",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let _listener = stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(state_changed_callback)
        .param_changed(param_changed_callback)
        .process(process_callback)
        .register()?;

    let fmt_obj = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::RGBA, // First element is discarded?
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::xBGR,
            spa::param::video::VideoFormat::BGRA,
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                // Default
                width: 128,
                height: 128,
            },
            spa::utils::Rectangle {
                // Min
                width: 1,
                height: 1,
            },
            spa::utils::Rectangle {
                // Max
                width: 4096,
                height: 4096,
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoMaxFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction {
                // Default
                num: options.fps,
                denom: 1,
            },
            spa::utils::Fraction {
                // Min
                num: 0,
                denom: 1,
            },
            spa::utils::Fraction {
                // Max
                num: options.fps,
                denom: 1,
            }
        ),
    );

    let fmt_values = serialize_obj(fmt_obj);

    let mut params = [spa::pod::Pod::from_bytes(&fmt_values).unwrap()];

    stream.connect(
        Direction::Input,
        Some(stream_id),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;

    ready_sender.send(true)?;

    while capturer_state.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        std::thread::sleep(Duration::from_millis(10));
    }

    let pw_loop = mainloop.loop_();

    // User has called Capturer::start() and we start the main loop
    while capturer_state.load(std::sync::atomic::Ordering::SeqCst) == 1
        && /* If the stream state got changed to `Error`, we exit. TODO: tell user that we exited */
          !stream_state_changed_to_error.load(std::sync::atomic::Ordering::SeqCst)
    {
        pw_loop.iterate(Duration::from_millis(100));
    }

    debug!("Finished");

    Ok(())
}

pub struct WaylandCapturer {
    capturer_join_handle: Option<JoinHandle<Result<(), LinCapError>>>,
    capturer_state: Arc<AtomicU8>,
    stream_state_changed_to_error: Arc<AtomicBool>,
    // The pipewire stream is deleted when the connection is dropped.
    // That's why we keep it alive
    _connection: Arc<std::sync::Mutex<dbus::blocking::Connection>>,
}

impl WaylandCapturer {
    // TODO: Error handling
    pub fn new(
        options: Options,
        on_format_changed: OnFormatChangedCb,
        on_frame: OnFrameCb,
    ) -> Self {
        let capturer_state = Arc::new(AtomicU8::new(0));
        let stream_state_changed_to_error = Arc::new(AtomicBool::new(false));

        let (stream_id, connection) = match &options.target {
            Some(target) => match target {
                crate::Target::Display(display) => match &display.raw {
                    crate::targets::LinuxDisplay::Wayland { connection } => {
                        let stream_id = display.id;
                        (stream_id, Arc::clone(connection))
                    }
                    crate::targets::LinuxDisplay::X11 { .. } => unreachable!(),
                },
                _ => unreachable!(),
            },
            None => {
                let target = get_main_display();
                match target.raw {
                    crate::targets::LinuxDisplay::Wayland { connection } => {
                        let stream_id = target.id;
                        (stream_id, Arc::clone(&connection))
                    }
                    crate::targets::LinuxDisplay::X11 { .. } => unreachable!(),
                }
            }
        };

        let capturer_state_clone = Arc::clone(&capturer_state);
        let stream_state_changed_to_error_clone = Arc::clone(&stream_state_changed_to_error);
        let (ready_sender, ready_recv) = sync_channel(1);
        let capturer_join_handle = std::thread::spawn(move || {
            let res = pipewire_capturer(
                options,
                &ready_sender,
                stream_id,
                capturer_state_clone,
                stream_state_changed_to_error_clone,
                on_format_changed,
                on_frame,
            );
            if res.is_err() {
                ready_sender.send(false)?;
            }
            res
        });

        if !ready_recv.recv().expect("Failed to receive") {
            panic!("Failed to setup capturer");
        }

        Self {
            capturer_join_handle: Some(capturer_join_handle),
            _connection: connection,
            capturer_state,
            stream_state_changed_to_error,
        }
    }
}

impl LinuxCapturerImpl for WaylandCapturer {
    fn start_capture(&mut self) {
        self.capturer_state
            .store(1, std::sync::atomic::Ordering::SeqCst);
    }

    fn stop_capture(&mut self) {
        self.capturer_state
            .store(2, std::sync::atomic::Ordering::SeqCst);
        if let Some(handle) = self.capturer_join_handle.take() {
            if let Err(e) = handle.join().unwrap() {
                error!("Error occured capturing: {e}");
            }
        }
        self.capturer_state
            .store(0, std::sync::atomic::Ordering::SeqCst);
        self.stream_state_changed_to_error
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
