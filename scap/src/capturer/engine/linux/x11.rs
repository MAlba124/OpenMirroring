// TODO: on gnome when a window is captured and made fullscreen/not fullscreen it crashes.

use std::{
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    thread::JoinHandle,
};

use anyhow::{bail, Result};

use log::error;
use xcb::{x, Xid};

use crate::{
    capturer::{OnFormatChangedCb, OnFrameCb, Options},
    frame::{self, FrameInfo},
    targets::{linux::get_default_x_display, LinuxDisplay, LinuxWindow},
    Target,
};

use super::LinuxCapturerImpl;

struct ShmBuf {
    pub id: u32,
    data: *mut u8,
    pub size: usize,
}

impl ShmBuf {
    pub fn null() -> Self {
        Self {
            id: 0,
            data: std::ptr::null_mut(),
            size: 0,
        }
    }

    pub fn new(size: usize) -> Self {
        // Last 9 bits is access permissions
        let id = unsafe { libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o6_0_0) };

        if id == -1 {
            todo!();
        }

        let data = unsafe { libc::shmat(id, std::ptr::null(), 0) as *mut u8 };

        if data.is_null() {
            todo!();
        }

        Self {
            id: id as u32,
            data,
            size,
        }
    }

    pub fn slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.data, self.size) }
    }

    pub fn slice_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.data, self.size) }
    }
}

impl Drop for ShmBuf {
    fn drop(&mut self) {
        unsafe {
            libc::shmdt(self.data as *const libc::c_void);
            libc::shmctl(self.id as i32, libc::IPC_RMID, std::ptr::null_mut());
        }
    }
}

fn current_time_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

pub struct X11Capturer {
    capturer_join_handle: Option<JoinHandle<Result<(), xcb::Error>>>,
    capturer_state: Arc<AtomicU8>,
}

#[allow(clippy::too_many_arguments)]
fn draw_cursor(
    conn: &xcb::Connection,
    img: &mut [u8],
    win_x: i16,
    win_y: i16,
    win_width: i16,
    win_height: i16,
    is_win: bool,
    win: &xcb::x::Window,
) -> Result<(), xcb::Error> {
    let cursor_image_cookie = conn.send_request(&xcb::xfixes::GetCursorImage {});
    let cursor_image = conn.wait_for_reply(cursor_image_cookie)?;

    let win_x = win_x as i32;
    let win_y = win_y as i32;

    let win_width = win_width as i32;
    let win_height = win_height as i32;

    let mut cursor_x = cursor_image.x() as i32 - cursor_image.xhot() as i32;
    let mut cursor_y = cursor_image.y() as i32 - cursor_image.yhot() as i32;
    if is_win {
        let disp = conn.get_raw_dpy();
        let mut ncursor_x = 0;
        let mut ncursor_y = 0;
        let mut child_return = 0;
        if unsafe {
            x11::xlib::XTranslateCoordinates(
                disp,
                x11::xlib::XDefaultRootWindow(disp),
                win.resource_id() as u64,
                cursor_image.x() as i32,
                cursor_image.y() as i32,
                &mut ncursor_x,
                &mut ncursor_y,
                &mut child_return,
            )
        } == 0
        {
            return Ok(());
        }
        cursor_x = ncursor_x - cursor_image.xhot() as i32;
        cursor_y = ncursor_y - cursor_image.yhot() as i32;
    }

    if cursor_x >= win_width + win_x
        || cursor_y >= win_height + win_y
        || cursor_x < win_x
        || cursor_y < win_y
    {
        return Ok(());
    }

    let x = cursor_x.max(win_x);
    let y = cursor_y.max(win_y);

    let w = ((cursor_x + cursor_image.width() as i32).min(win_x + win_width) - x) as u32;
    let h = ((cursor_y + cursor_image.height() as i32).min(win_y + win_height) - y) as u32;

    let c_off = (x - cursor_x) as u32;
    let i_off: i32 = x - win_x;

    let stride: u32 = 4;
    let mut cursor_idx: u32 = ((y - cursor_y) * cursor_image.width() as i32) as u32;
    let mut image_idx: u32 = ((y - win_y) * win_width * stride as i32) as u32;

    for _ in 0..h {
        cursor_idx += c_off;
        image_idx += i_off as u32 * stride;
        for _ in 0..w {
            let cursor_pix = cursor_image.cursor_image()[cursor_idx as usize];
            let r = (cursor_pix & 0xFF) as u8;
            let g = ((cursor_pix >> 8) & 0xFF) as u8;
            let b = ((cursor_pix >> 16) & 0xFF) as u8;
            let a = (cursor_pix >> 24) & 0xFF;

            let i = image_idx as usize;
            if a == 0xFF {
                img[i] = r;
                img[i + 1] = g;
                img[i + 2] = b;
            } else if a > 0 {
                let a = 255 - a;
                img[i] = r + ((img[i] as u32 * a + 255 / 2) / 255) as u8;
                img[i + 1] = g + ((img[i + 1] as u32 * a + 255 / 2) / 255) as u8;
                img[i + 2] = b + ((img[i + 2] as u32 * a + 255 / 2) / 255) as u8;
            }

            cursor_idx += 1;
            image_idx += stride;
        }
        cursor_idx += cursor_image.width() as u32 - w - c_off;
        image_idx += (win_width - w as i32 - i_off) as u32 * stride;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn grab(
    conn: &xcb::Connection,
    target: &Target,
    show_cursor: bool,
    base_time: u64,
    current_frame_info: &mut Option<FrameInfo>,
    on_format_changed: &mut OnFormatChangedCb,
    on_frame: &mut OnFrameCb,
    shm_seg: &xcb::shm::Seg,
    shm_buf: &mut ShmBuf,
) -> Result<(), xcb::Error> {
    let (x, y, width, height, window, is_win) = match &target {
        Target::Window(win) => {
            let LinuxWindow::X11 { raw_handle } = win.raw else {
                unreachable!();
            };
            let geom_cookie = conn.send_request(&x::GetGeometry {
                drawable: x::Drawable::Window(raw_handle),
            });
            let geom = conn.wait_for_reply(geom_cookie)?;
            (0, 0, geom.width(), geom.height(), raw_handle, true)
        }
        Target::Display(disp) => {
            let LinuxDisplay::X11 {
                raw_handle,
                width,
                height,
                x_offset,
                y_offset,
            } = disp.raw
            else {
                unreachable!();
            };
            (x_offset, y_offset, width, height, raw_handle, false)
        }
    };

    let frame_size = width as usize * height as usize * 4;

    if shm_buf.size != frame_size {
        if shm_buf.size != 0 {
            conn.send_and_check_request(&xcb::shm::Detach { shmseg: *shm_seg })?;
        }

        *shm_buf = ShmBuf::new(frame_size);

        conn.send_and_check_request(&xcb::shm::Attach {
            shmseg: *shm_seg,
            shmid: shm_buf.id,
            read_only: false,
        })?;
    }

    let img_cookie = conn.send_request(&xcb::shm::GetImage {
        drawable: x::Drawable::Window(window),
        x,
        y,
        width,
        height,
        plane_mask: u32::MAX,
        format: x::ImageFormat::ZPixmap as u8,
        shmseg: *shm_seg,
        offset: 0,
    });

    let img = conn.wait_for_reply(img_cookie)?;

    if img.size() as usize != shm_buf.size {
        todo!();
    }

    let display_time = current_time_nanos() - base_time;

    if show_cursor {
        draw_cursor(
            conn,
            shm_buf.slice_mut(),
            x,
            y,
            width as i16,
            height as i16,
            is_win,
            &window,
        )?;
    }

    let frame_info = FrameInfo {
        format: frame::FrameFormat::BGRx,
        width: width as u32,
        height: height as u32,
    };

    if let Some(current_frame_info) = current_frame_info {
        if *current_frame_info != frame_info {
            (on_format_changed)(frame_info);
        }
    } else {
        (on_format_changed)(frame_info);
    }

    (on_frame)(display_time, shm_buf.slice());

    Ok(())
}

fn query_xfixes_version(conn: &xcb::Connection) -> Result<(), xcb::Error> {
    let cookie = conn.send_request(&xcb::xfixes::QueryVersion {
        client_major_version: xcb::xfixes::MAJOR_VERSION,
        client_minor_version: xcb::xfixes::MINOR_VERSION,
    });
    let _ = conn.wait_for_reply(cookie)?;
    Ok(())
}

impl X11Capturer {
    pub fn new(
        options: Options,
        mut on_format_changed: OnFormatChangedCb,
        mut on_frame: OnFrameCb,
    ) -> Result<Self> {
        let (conn, screen_num) = xcb::Connection::connect_with_xlib_display_and_extensions(
            &[
                xcb::Extension::RandR,
                xcb::Extension::XFixes,
                xcb::Extension::Shm,
            ],
            &[],
        )?;
        query_xfixes_version(&conn)?;
        let setup = conn.get_setup();
        let Some(screen) = setup.roots().nth(screen_num as usize) else {
            bail!("Failed to get setup root");
        };

        let target = match &options.target {
            Some(t) => t.clone(),
            None => Target::Display(
                get_default_x_display(&conn, screen)?,
            ),
        };

        let framerate = options.fps as f32;
        let show_cursor = options.show_cursor;
        let capturer_state = Arc::new(AtomicU8::new(0));
        let capturer_state_clone = Arc::clone(&capturer_state);

        let jh = std::thread::spawn(move || {
            while capturer_state_clone.load(Ordering::Acquire) == 0 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            let shm_seg = conn.generate_id::<xcb::shm::Seg>();
            let mut shm_buf = ShmBuf::null();

            let base_time = current_time_nanos();
            let mut current_format_info = None;
            let frame_time = std::time::Duration::from_secs_f32(1.0 / framerate);
            while capturer_state_clone.load(Ordering::Acquire) == 1 {
                let start = std::time::Instant::now();

                grab(
                    &conn,
                    &target,
                    show_cursor,
                    base_time,
                    &mut current_format_info,
                    &mut on_format_changed,
                    &mut on_frame,
                    &shm_seg,
                    &mut shm_buf,
                )?;

                let elapsed = start.elapsed();
                if elapsed < frame_time {
                    std::thread::sleep(frame_time - start.elapsed());
                }
            }

            Ok(())
        });

        Ok(Self {
            capturer_state,
            capturer_join_handle: Some(jh),
        })
    }
}

impl LinuxCapturerImpl for X11Capturer {
    fn start(&mut self) {
        self.capturer_state.store(1, Ordering::Release);
    }

    fn stop(&mut self) {
        self.capturer_state.store(2, Ordering::Release);
        if let Some(handle) = self.capturer_join_handle.take() {
            if let Err(e) = handle.join().expect("Failed to join capturer thread") {
                error!("Error occured capturing: {e}");
            }
        }
    }
}
