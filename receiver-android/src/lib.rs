use receiver_core::slint;

#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    log_panics::init();

    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
    );

    receiver_core::slint::BackendSelector::new()
        .require_wgpu_26(receiver_core::slint::wgpu_26::WGPUConfiguration::default())
        .select()
        .unwrap();

    slint::android::init(app).unwrap();

    #[cfg(debug_assertions)]
    unsafe {
        std::env::set_var("GST_DEBUG_NO_COLOR", "true");
        std::env::set_var("GST_DEBUG", "4");
    }

    receiver_core::run().unwrap();
}
