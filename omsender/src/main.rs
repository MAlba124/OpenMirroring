use gst::prelude::*;
use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow};
use gtk4 as gtk;

use std::cell::RefCell;

fn build_ui(app: &Application) {
    let pipeline = gst::Pipeline::new();

    let gtksink = gst::ElementFactory::make("gtk4paintablesink")
        .build()
        .unwrap();

    let src = gst::ElementFactory::make("scapsrc")
        .property("perform-internal-preroll", true)
        .build()
        .unwrap();

    let sink = gst::Bin::default();
    let convert = gst::ElementFactory::make("videoconvert").build().unwrap();

    sink.add(&convert).unwrap();
    sink.add(&gtksink).unwrap();
    convert.link(&gtksink).unwrap();

    sink.add_pad(&gst::GhostPad::with_target(&convert.static_pad("sink").unwrap()).unwrap())
        .unwrap();

    let sink = sink.upcast();
    pipeline.add_many([&src, &sink]).unwrap();
    gst::Element::link_many([&src, &sink]).unwrap();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let gst_widget = gst_gtk4::RenderWidget::new(&gtksink);
    vbox.append(&gst_widget);

    let label = gtk::Label::new(Some("Position: 00:00:00"));
    vbox.append(&label);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("My GTK App")
        .child(&vbox)
        .build();

    window.present();

    let pipeline_weak = pipeline.downgrade();
    let timeout_id = glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let Some(pipeline) = pipeline_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };

        let position = pipeline.query_position::<gst::ClockTime>();
        label.set_text(&format!("Position: {:.0}", position.display()));
        glib::ControlFlow::Continue
    });

    let bus = pipeline.bus().unwrap();

    pipeline
        .set_state(gst::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");

    let app_weak = app.downgrade();
    let bus_watch = bus
        .add_watch_local(move |_, msg| {
            use gst::MessageView;

            let Some(app) = app_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            match msg.view() {
                MessageView::Eos(..) => app.quit(),
                MessageView::Error(err) => {
                    println!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    app.quit();
                }
                _ => (),
            };

            glib::ControlFlow::Continue
        })
        .expect("Failed to add bus watch");

    let timeout_id = RefCell::new(Some(timeout_id));
    let pipeline = RefCell::new(Some(pipeline));
    let bus_watch = RefCell::new(Some(bus_watch));
    app.connect_shutdown(move |_| {
        window.close();

        drop(bus_watch.borrow_mut().take());
        if let Some(pipeline) = pipeline.borrow_mut().take() {
            pipeline
                .set_state(gst::State::Null)
                .expect("Unable to set the pipeline to the `Null` state");
        }

        if let Some(timeout_id) = timeout_id.borrow_mut().take() {
            timeout_id.remove();
        }
    });
}

fn main() -> glib::ExitCode {
    gst::init().unwrap();
    scapgst::plugin_register_static().unwrap();
    gst_gtk4::plugin_register_static().unwrap();

    let app = Application::builder()
        .application_id("org.example.helloworld")
        .build();

    app.connect_activate(build_ui);

    app.run()
}
