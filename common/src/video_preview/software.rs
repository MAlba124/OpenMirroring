use anyhow::{bail, Result};
use gst::prelude::*;
use gst_video::prelude::*;

fn try_gstreamer_video_frame_to_pixel_buffer(
    frame: &gst_video::VideoFrame<gst_video::video_frame::Readable>,
) -> Result<slint::SharedPixelBuffer<slint::Rgb8Pixel>> {
    match frame.format() {
        gst_video::VideoFormat::Rgb => {
            let mut slint_pixel_buffer =
                slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(frame.width(), frame.height());
            frame
                .buffer()
                .copy_to_slice(0, slint_pixel_buffer.make_mut_bytes())
                .expect("Unable to copy to slice!"); // Copies!
            Ok(slint_pixel_buffer)
        }
        _ => {
            bail!(
                "Cannot convert frame to a slint RGB frame because it is format {}",
                frame.format().to_str()
            )
        }
    }
}

pub struct SlintSwSink {
    pub bin: gst::Bin,
}

impl SlintSwSink {
    pub fn new<Ui>(ui_weak: slint::Weak<Ui>, new_frame_cb: fn(Ui, slint::Image)) -> Result<Self>
    where
        Ui: slint::ComponentHandle + 'static,
    {
        let bin = gst::Bin::new();
        let convert = gst::ElementFactory::make("videoconvert").build()?;
        let scale = gst::ElementFactory::make("videoscale").build()?;
        let appsink = gst_app::AppSink::builder()
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .format(gst_video::VideoFormat::Rgb)
                    .width(512)
                    .height(256)
                    .build(),
            )
            .enable_last_sample(false)
            .max_buffers(1u32)
            .build();

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer_owned().unwrap(); // Probably copies!
                    let caps = sample.caps().unwrap();
                    let video_info =
                        gst_video::VideoInfo::from_caps(caps).expect("couldn't build video info!");
                    let video_frame =
                        gst_video::VideoFrame::from_buffer_readable(buffer, &video_info).unwrap();
                    let slint_frame = try_gstreamer_video_frame_to_pixel_buffer(&video_frame)
                        .expect("Unable to convert the video frame to a slint video frame!");

                    ui_weak
                        .upgrade_in_event_loop(move |app| {
                            new_frame_cb(app, slint::Image::from_rgb8(slint_frame))
                        })
                        .unwrap();

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        bin.add_many(&[&convert, &scale, appsink.upcast_ref()])?;
        gst::Element::link_many(&[&convert, &scale, appsink.upcast_ref()])?;

        let Some(convert_pad) = convert.static_pad("sink") else {
            anyhow::bail!("Unable to get static sink pad from preview convert");
        };

        let bin_sink_ghost_pad = gst::GhostPad::with_target(&convert_pad)?;
        bin_sink_ghost_pad.set_active(true)?;
        bin.add_pad(&bin_sink_ghost_pad)?;

        Ok(Self { bin })
    }
}
