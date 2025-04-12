use jni::objects::JObject;
use jni::JavaVM;

use log::debug;
use log::error;

slint::slint! {
    import { Button, VerticalBox } from "std-widgets.slint";

    export component MainWindow inherits Window {
        in-out property<string> label_text: "Hello";

        callback request_start_capture;

        VerticalBox {
            Text {
                text: label_text;
            }

            Button {
                text: "Capture Screen";
                clicked => {
                    root.request_start_capture();
                    label_text = "Running";
                }
            }
        }
    }
}

struct State {
    pub main_window: MainWindow,
}

thread_local! {
    static STATE: core::cell::RefCell<Option<State>> = Default::default();
}

fn init(app: slint::android::AndroidApp) -> State {
    let main_window = MainWindow::new().unwrap();

    main_window.on_request_start_capture(move || {
        debug!("request_start_capture was called");

        let vm = unsafe {
            let ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
            assert!(!ptr.is_null());
            JavaVM::from_raw(ptr).unwrap()
        };
        let activity = unsafe {
            let ptr = app.activity_as_ptr() as *mut jni::sys::_jobject;
            assert!(!ptr.is_null());
            JObject::from_raw(ptr)
        };

        match vm.get_env() {
            Ok(mut env) => {
                match env.call_method(activity, "startScreenCapture", "()V", &[]) {
                    Ok(_) => (),
                    Err(err) => error!("Failed to call `startScreenCapture`: {err}"),
                }
            }
            Err(err) => error!("Failed to get env from VM: {err}"),
        }
    });

    State {
        main_window,
    }
}

#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Debug));

    debug!("Hello from rust");

    let app_clone = app.clone();

    slint::android::init(app).unwrap();

    let state = init(app_clone);

    let main_window = state.main_window.clone_strong();

    STATE.with(|ui| *ui.borrow_mut() = Some(state));

    main_window.run().unwrap();
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn Java_com_github_malba124_openmirroring_android_sender_ScreenCaptureService_nativeProcessFrame<'local>(
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
