mod imp;

use gtk4 as gtk;
use gtk::glib;

glib::wrapper! {
    pub struct Video(ObjectSubclass<imp::Video>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Video {
    fn default() -> Self {
        glib::Object::new()
    }
}
