mod imp;

use gtk4 as gtk;
use gtk::glib;

glib::wrapper! {
    pub struct LoadingView(ObjectSubclass<imp::LoadingView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for LoadingView {
    fn default() -> Self {
        glib::Object::new()
    }
}
