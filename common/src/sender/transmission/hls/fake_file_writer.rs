// Copyright (C) 2025 Marcus L. Hanestad <marlhan@proton.me>
//
// This file is part of OpenMirroring.
//
// OpenMirroring is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// OpenMirroring is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with OpenMirroring.  If not, see <https://www.gnu.org/licenses/>.

use crossbeam_channel::Sender;

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
                    if let Err(err) = tx.send(ChannelElement {
                        location,
                        request: Request::Add(data),
                    }) {
                        log::debug!("Failed to send fake file: {err}");
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
