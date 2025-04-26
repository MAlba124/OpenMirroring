use std::sync::{Arc, Mutex};

pub mod opengl;

pub type GstGlContext = Arc<Mutex<Option<(gst_gl::GLContext, gst_gl::GLDisplay)>>>;
