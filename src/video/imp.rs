use std::cell::RefCell;

use gtk4 as gtk;
use gtk::{glib, prelude::*, subclass::prelude::*};
use gst::prelude::*;
use log::{debug, error};

use crate::{models::PlaybackState, runtime, Event, GuiEvent};

#[derive(Debug, Default)]
pub struct Video {
    child: RefCell<Option<gtk::Widget>>,
    pipeline: RefCell<Option<gst::Pipeline>>,
    event_tx: RefCell<Option<async_channel::Sender<()>>>,
}

impl Video {
    pub fn shutdown(&self) {
        debug!("Shutting down video");
    }
}

#[glib::object_subclass]
impl ObjectSubclass for Video {
    const NAME: &'static str = "Video";
    type Type = super::Video;
    type ParentType = gtk::Widget;

    fn class_init(klass: &mut Self::Class) {
        // The layout manager determines how child widgets are laid out.
        // klass.set_layout_manager_type::<gtk::BinLayout>();

        // Make it look like a GTK button.
        // klass.set_css_name("button");

        // Make it appear as a button to accessibility tools.
        // klass.set_accessible_role(gtk::AccessibleRole::Button);

        // klass.install_properties(&[
        //     ParamSpec
        // ]);
    }
}

impl ObjectImpl for Video {
    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();

        let pipeline = gst::Pipeline::new();
        let gtksink = gst::ElementFactory::make("gtk4paintablesink").build().unwrap();
        let playbin = gst::ElementFactory::make("playbin3").build().unwrap();
        let sink = gst::Bin::default();
        let convert = gst::ElementFactory::make("videoconvert").build().unwrap();

        sink.add(&convert).unwrap();
        sink.add(&gtksink).unwrap();
        convert.link(&gtksink).unwrap();
        sink.add_pad(&gst::GhostPad::with_target(&convert.static_pad("sink").unwrap()).unwrap())
            .unwrap();

        playbin.set_property("video-sink", &sink);

        pipeline.add(&playbin).unwrap();

        let video_view = gst_gtk4::RenderWidget::new(&gtksink);

        let pipeline_weak = pipeline.downgrade();
        let event_tx_clone = event_tx.clone();
        let timeout_id = glib::timeout_add_local(std::time::Duration::from_millis(1000), move || {
            let event_tx = event_tx_clone.clone();
            runtime().block_on(async {
                let Some(pipeline) = pipeline_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };

                let position: Option<gst::ClockTime> = pipeline.query_position();
                // let speed = pipeline.query_position();
                let duration: Option<gst::ClockTime> = pipeline.query_duration();
                // let speed = pipeline.
                let state = match pipeline.state(gst::ClockTime::NONE).1 {
                    gst::State::Paused => PlaybackState::Paused,
                    gst::State::Playing => PlaybackState::Playing,
                    _ => PlaybackState::Idle,
                };
                event_tx
                    .send(Event::PlaybackUpdate {
                        time: position.unwrap_or_default().seconds_f64(),
                        duration: duration.unwrap_or_default().seconds_f64(),
                        state,
                        speed: 1.0,
                    })
                    .await
                    .unwrap();
                glib::ControlFlow::Continue
            })
        });

        let bus = pipeline.bus().unwrap();

        debug!("Setting pipeline to `Ready`");

        pipeline
            .set_state(gst::State::Ready)
            .expect("Unable to set the pipeline to the `Ready` state");
        let _app_weak = app.downgrade(); // HACK: removing this makes the gui not show!?
        let pipeline_weak = pipeline.downgrade();
        let get_clone = gui_event_tx.clone();
        let bus_watch = bus
            .add_watch_local(move |_, msg| {
                use gst::MessageView;

                let Some(pipeline) = pipeline_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };

                match msg.view() {
                    MessageView::Eos(..) => {
                        debug!("Reached EOS");
                        pipeline
                            .set_state(gst::State::Ready)
                            .expect("Unable to set pipeline to `Ready` state");
                        glib::spawn_future_local(glib::clone!(
                            #[strong]
                            get_clone,
                            async move {
                                get_clone.send(GuiEvent::Eos).await.unwrap();
                            }
                        ));
                    }
                    MessageView::Error(err) => {
                        error!(
                            "Error from {:?}: {} ({:?})",
                            err.src().map(|s| s.path_string()),
                            err.error(),
                            err.debug()
                        );
                        pipeline
                            .set_state(gst::State::Null)
                            .expect("Unable to set pipeline to `Null` state");
                        // TODO: notify GUI
                    }
                    _ => (),
                };

                glib::ControlFlow::Continue
            })
            .expect("Failed to add bus watch");

        // Create the child label.
        // let label = "Hello world!";
        // let child = gtk::Label::new(Some(label));
        // child.set_parent(&*obj);
        // *self.child.borrow_mut() = Some(child.upcast::<gtk::Widget>());

        // Make it look like a GTK button with a label (as opposed to an icon).
        // obj.add_css_class("text-button");

        // Tell accessibility tools the button has a label.
        // obj.update_property(&[gtk::accessible::Property::Label(label)]);

        // Connect a gesture to handle clicks.
        // let gesture = gtk::GestureClick::new();
        // gesture.connect_released(|gesture, _, _, _| {
        //     gesture.set_state(gtk::EventSequenceState::Claimed);
        //     println!("Button pressed!");
        // });
        // obj.add_controller(gesture);

    }

    fn dispose(&self) {
        // Child widgets need to be manually un-parented in `dispose()`.
        if let Some(child) = self.child.borrow_mut().take() {
            child.unparent();
        }
    }
}

impl WidgetImpl for Video {}
