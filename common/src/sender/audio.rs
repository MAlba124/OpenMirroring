// TODO: try pipewiredeviceprovider and use pulsedeviceprovider as fallback
// TODO: monitor for changes
#[cfg(target_os = "linux")]
pub fn get_pulse_dev() -> anyhow::Result<gst::Device> {
    use anyhow::bail;
    use gst::prelude::*;

    let provider = gst::DeviceProviderFactory::by_name("pulsedeviceprovider").ok_or(
        anyhow::anyhow!("Could not find pulse device provider factory"),
    )?;

    provider.start()?;
    let devices = provider.devices();
    provider.stop();

    for device in devices {
        if !device.has_classes("Audio/Sink") {
            continue;
        }
        let Some(props) = device.properties() else {
            continue;
        };
        if props.get::<bool>("is-default") == Ok(true) {
            return Ok(device);
        }
    }

    bail!("No device found")
}
