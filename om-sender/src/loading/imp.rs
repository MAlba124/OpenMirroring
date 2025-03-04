use std::cell::RefCell;

use gtk4 as gtk;
use gtk::{glib, prelude::*, subclass::prelude::*};

#[derive(Debug, Default)]
pub struct LoadingView {
    child: RefCell<Option<gtk::Widget>>,
}

#[glib::object_subclass]
impl ObjectSubclass for LoadingView {
    const NAME: &'static str = "LoadingView";
    type Type = super::LoadingView;
    type ParentType = gtk::Widget;

    fn class_init(klass: &mut Self::Class) {
        klass.set_layout_manager_type::<gtk::BinLayout>();
    }
}

impl ObjectImpl for LoadingView {
    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();

        let child = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();

        let spinner = gtk::Spinner::builder().spinning(true).build();
        let label = gtk::Label::new(Some("Loading sources..."));

        child.append(&spinner);
        child.append(&label);

        child.set_parent(&*obj);
        *self.child.borrow_mut() = Some(child.upcast::<gtk::Widget>());
    }

    fn dispose(&self) {
        if let Some(child) = self.child.borrow_mut().take() {
            child.unparent();
        }
    }
}

impl WidgetImpl for LoadingView {}
