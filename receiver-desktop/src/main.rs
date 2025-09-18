fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_module("receiver", common::default_log_level())
        .filter_module("receiver-core", common::default_log_level())
        .init();

    if std::env::var("SLINT_BACKEND") == Err(std::env::VarError::NotPresent) {
        receiver_core::slint::BackendSelector::new()
            .require_wgpu_26(receiver_core::slint::wgpu_26::WGPUConfiguration::default())
            .select()?;
    }

    receiver_core::run()
}
