use tokio::sync::mpsc::Sender;

use gst::{glib, subclass::prelude::ObjectSubclassIsExt};

pub enum Request {
    Delete,
    Add(Vec<u8>),
}

pub struct ChannelElement {
    pub location: String,
    pub request: Request,
}

mod imp {
    use std::cell::RefCell;

    use gio::subclass::prelude::*;

    use super::*;

    #[derive(Default)]
    pub struct FakeFileWriter {
        pub location: RefCell<String>,
        pub data: RefCell<Vec<u8>>,
        pub tx: RefCell<Option<Sender<ChannelElement>>>,
    }

    impl ObjectImpl for FakeFileWriter {}

    #[glib::object_subclass]
    impl ObjectSubclass for FakeFileWriter {
        const NAME: &'static str = "FakeFileWriter";
        type Type = super::FakeFileWriter;
        type ParentType = gio::OutputStream;
    }

    impl OutputStreamImpl for FakeFileWriter {
        fn write(
            &self,
            buffer: &[u8],
            _cancellable: Option<&gio::Cancellable>,
        ) -> Result<usize, glib::Error> {
            self.data.borrow_mut().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn close(&self, _cancellable: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
            match (*self.tx.borrow_mut()).take() {
                Some(tx) => {
                    let location = self.location.borrow().clone();
                    let data = self.data.borrow().clone();
                    match tokio::runtime::Handle::try_current() {
                        Ok(rt_handle) => {
                            rt_handle.spawn(async move {
                                tx.send(ChannelElement {
                                    location,
                                    request: Request::Add(data),
                                })
                                .await
                                .unwrap();
                            });
                        }
                        Err(_) => {
                            tx.blocking_send(ChannelElement {
                                location,
                                request: Request::Add(data),
                            })
                            .map_err(|err| {
                                glib::Error::new(glib::FileError::Failed, &err.to_string())
                            })?;
                        }
                    }
                    Ok(())
                }
                None => Err(glib::Error::new(glib::FileError::Failed, "Missing tx")),
            }
        }

        fn flush(&self, _cancellable: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
            Ok(())
        }

        fn splice(
            &self,
            input_stream: &gio::InputStream,
            flags: gio::OutputStreamSpliceFlags,
            cancellable: Option<&gio::Cancellable>,
        ) -> Result<usize, glib::Error> {
            self.parent_splice(input_stream, flags, cancellable)
        }
    }
}

glib::wrapper! {
    pub struct FakeFileWriter(ObjectSubclass<imp::FakeFileWriter>) @extends gio::OutputStream;
}

impl FakeFileWriter {
    pub fn new(location: String, tx: Sender<ChannelElement>) -> Self {
        let self_: Self = glib::Object::new();

        let imp = self_.imp();

        *imp.location.borrow_mut() = location;
        *imp.tx.borrow_mut() = Some(tx);

        self_
    }
}
