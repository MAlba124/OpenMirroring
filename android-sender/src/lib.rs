use jni::objects::JObject;
use jni::JavaVM;

use log::debug;
use log::error;
use log::info;

mod discovery;

lazy_static::lazy_static! {
    pub static ref EVENT_CHAN: (async_channel::Sender<Event>, async_channel::Receiver<Event>) =
        async_channel::unbounded();
}

#[macro_export]
macro_rules! tx {
    () => {
        crate::EVENT_CHAN.0
    };
}

macro_rules! rx {
    () => {
        crate::EVENT_CHAN.1
    };
}

slint::include_modules!();

pub enum Event {
    CaptureStarted,
    CaptureStopped,
    ReceiverAvailable(String),
}

fn init(app: slint::android::AndroidApp) -> MainWindow {
    let main_window = MainWindow::new().unwrap();

    main_window.on_request_start_capture(move || {
        debug!("request_start_capture was called");

        let vm = unsafe {
            let ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
            assert!(!ptr.is_null(), "JavaVM ptr is null");
            JavaVM::from_raw(ptr).unwrap()
        };
        let activity = unsafe {
            let ptr = app.activity_as_ptr() as *mut jni::sys::_jobject;
            assert!(!ptr.is_null(), "Activity ptr is null");
            JObject::from_raw(ptr)
        };

        match vm.get_env() {
            Ok(mut env) => match env.call_method(activity, "startScreenCapture", "()V", &[]) {
                Ok(_) => (),
                Err(err) => error!("Failed to call `startScreenCapture`: {err}"),
            },
            Err(err) => error!("Failed to get env from VM: {err}"),
        }
    });

    main_window
}

async fn event_loop(handle: slint::Weak<MainWindow>) {
    loop {
        match rx!().recv().await {
            Ok(event) => match event {
                Event::CaptureStarted => {
                    handle
                        .upgrade_in_event_loop(move |handle| {
                            handle.invoke_screen_capture_started();
                        })
                        .unwrap();
                }
                Event::CaptureStopped => {
                    handle
                        .upgrade_in_event_loop(move |handle| {
                            handle.invoke_screen_capture_stopped();
                        })
                        .unwrap();
                }
                Event::ReceiverAvailable(n) => info!("Reveiver available: {n}"),
            },
            Err(err) => {
                error!("Failed to receive event: {err}");
                return;
            }
        }
    }
}

#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
    );

    debug!("Hello from rust");

    gst::init().unwrap();

    debug!("GStreamer version: {:?}", gst::version());

    let app_clone = app.clone();

    slint::android::init(app).unwrap();

    let main_window = init(app_clone);

    let handle_weak = main_window.as_weak();

    common::runtime().spawn(event_loop(handle_weak));

    common::runtime().spawn(discovery::discover());

    main_window.run().unwrap();
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn Java_com_github_malba124_openmirroring_android_sender_ScreenCaptureService_nativeCaptureStarted<
    'local,
>(
    _env: jni::JNIEnv<'local>,
    _class: jni::objects::JClass<'local>,
) {
    debug!("Screen capture was started");
    tx!().send_blocking(Event::CaptureStarted).unwrap();
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn Java_com_github_malba124_openmirroring_android_sender_ScreenCaptureService_nativeCaptureStopped<
    'local,
>(
    _env: jni::JNIEnv<'local>,
    _class: jni::objects::JClass<'local>,
) {
    debug!("Screen capture was stopped");
    tx!().send_blocking(Event::CaptureStopped).unwrap();
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn Java_com_github_malba124_openmirroring_android_sender_ScreenCaptureService_nativeProcessFrame<
    'local,
>(
    _env: jni::JNIEnv<'local>,
    _class: jni::objects::JClass<'local>,
    _buffer: jni::objects::JClass<'local>,
    _width: jni::sys::jint,
    _height: jni::sys::jint,
    _pixel_stride: jni::sys::jint,
    _row_stride: jni::sys::jint,
) {
    log::debug!("Got frame");
}
