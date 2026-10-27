//! Android QR invite scan via NDK Camera2 (back camera). Preview + rqrr on the Y plane.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

const CAP_W: i32 = 640;
const CAP_H: i32 = 480;

enum Scan {
    Idle,
    Busy,
    Ready(Result<String, String>),
}

fn scan_slot() -> &'static Mutex<Scan> {
    static S: OnceLock<Mutex<Scan>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Scan::Idle))
}

fn cancel_flag() -> &'static AtomicBool {
    static C: OnceLock<AtomicBool> = OnceLock::new();
    C.get_or_init(|| AtomicBool::new(false))
}

/// Newest preview frame; [`take_qr_preview_rgba`] empties the slot so each frame is shown once.
struct Preview {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

fn preview_slot() -> &'static Mutex<Option<Preview>> {
    static P: OnceLock<Mutex<Option<Preview>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

fn publish_preview_y(y: &[u8], width: u32, height: u32, y_row: usize) {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || y_row < w || y.len() < y_row * (h - 1) + w {
        return;
    }
    // Same modest preview width as desktop keeps per-frame texture uploads cheap.
    let step = w.div_ceil(320).max(1);
    let (out_w, out_h) = (w / step, h / step);
    let mut rgba = vec![0u8; out_w * out_h * 4];
    for row in 0..out_h {
        for col in 0..out_w {
            let g = y[row * step * y_row + col * step];
            let o = (row * out_w + col) * 4;
            rgba[o] = g;
            rgba[o + 1] = g;
            rgba[o + 2] = g;
            rgba[o + 3] = 255;
        }
    }
    if let Ok(mut slot) = preview_slot().lock() {
        *slot = Some(Preview {
            w: out_w as u32,
            h: out_h as u32,
            rgba,
        });
    }
}

fn gray_from_y(y: &[u8], width: usize, height: usize, y_row: usize) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || y_row < width || y.len() < y_row * (height - 1) + width {
        return None;
    }
    let mut gray = vec![0u8; width * height];
    for row in 0..height {
        gray[row * width..row * width + width]
            .copy_from_slice(&y[row * y_row..row * y_row + width]);
    }
    Some(gray)
}

mod ndk {
    use super::*;
    use std::os::raw::c_void;

    use crate::android_camera::{
        AImage_delete, AImage_getPlaneData, AImage_getPlaneRowStride, AImageReader,
        AImageReader_acquireLatestImage, Camera, Facing,
    };

    fn live() -> &'static Mutex<Option<Camera>> {
        static L: OnceLock<Mutex<Option<Camera>>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(None))
    }

    static CAPTURE: AtomicBool = AtomicBool::new(false);
    static DECODE_BUSY: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn on_image(_ctx: *mut c_void, reader: *mut AImageReader) {
        unsafe {
            if !CAPTURE.load(Ordering::Relaxed) || cancel_flag().load(Ordering::Relaxed) {
                return;
            }
            let mut image = std::ptr::null_mut();
            if AImageReader_acquireLatestImage(reader, &mut image) != 0 || image.is_null() {
                return;
            }
            let mut y_ptr = std::ptr::null_mut();
            let mut y_len = 0;
            let mut y_row = 0;
            let ok = AImage_getPlaneData(image, 0, &mut y_ptr, &mut y_len) == 0
                && AImage_getPlaneRowStride(image, 0, &mut y_row) == 0
                && !y_ptr.is_null()
                && y_row > 0;
            if !ok {
                AImage_delete(image);
                return;
            }
            let y = std::slice::from_raw_parts(y_ptr, y_len.max(0) as usize);
            let row = y_row.max(0) as usize;
            publish_preview_y(y, CAP_W as u32, CAP_H as u32, row);
            if let Some(gray) = gray_from_y(y, CAP_W as usize, CAP_H as usize, row) {
                // Drop frames while a decode is in flight (never stall the camera callback).
                if !DECODE_BUSY.swap(true, Ordering::AcqRel) {
                    std::thread::Builder::new()
                        .name("ghalbol-qr-decode".into())
                        .spawn(move || {
                            if cancel_flag().load(Ordering::Relaxed) {
                                DECODE_BUSY.store(false, Ordering::Release);
                                return;
                            }
                            if let Ok(text) =
                                crate::qr_luma::decode_luma(CAP_W as usize, CAP_H as usize, &gray)
                            {
                                if crate::qr_luma::is_ghalbol_invite(&text) {
                                    crate::p2p::native_log::info(
                                        "qr",
                                        format!("android invite QR ({} chars)", text.len()),
                                    );
                                    if let Ok(mut slot) = scan_slot().lock() {
                                        *slot = Scan::Ready(Ok(text));
                                    }
                                    cancel_flag().store(true, Ordering::SeqCst);
                                    CAPTURE.store(false, Ordering::Relaxed);
                                }
                            }
                            DECODE_BUSY.store(false, Ordering::Release);
                        })
                        .ok();
                }
            }
            AImage_delete(image);
        }
    }

    pub fn start_camera() -> Result<(), String> {
        stop_camera();
        let camera = Camera::open(Facing::Back, CAP_W, CAP_H, on_image)?;
        if let Ok(mut g) = live().lock() {
            *g = Some(camera);
        }
        CAPTURE.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn stop_camera() {
        CAPTURE.store(false, Ordering::Relaxed);
        // Take under the lock; the camera tears down when dropped outside it.
        let camera = live().lock().ok().and_then(|mut g| g.take());
        drop(camera);
    }
}

pub fn start_qr_scan() -> Result<(), String> {
    cancel_flag().store(true, Ordering::SeqCst);
    // Brief wait if a prior scan is tearing down.
    for _ in 0..40 {
        if let Ok(slot) = scan_slot().lock() {
            if !matches!(*slot, Scan::Busy) {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    cancel_flag().store(false, Ordering::SeqCst);
    {
        let mut slot = scan_slot().lock().map_err(|e| e.to_string())?;
        if matches!(*slot, Scan::Busy) {
            return Err("Camera is still stopping — try again".into());
        }
        *slot = Scan::Busy;
    }
    if let Ok(mut p) = preview_slot().lock() {
        *p = None;
    }
    crate::p2p::native_log::info("qr", "android scan start — opening back camera");
    if let Err(e) = ndk::start_camera() {
        if let Ok(mut slot) = scan_slot().lock() {
            *slot = Scan::Idle;
        }
        return Err(e);
    }
    Ok(())
}

pub fn take_qr_scan() -> Option<Result<String, String>> {
    let mut slot = scan_slot().lock().ok()?;
    match std::mem::replace(&mut *slot, Scan::Idle) {
        Scan::Ready(result) => {
            ndk::stop_camera();
            Some(result)
        }
        other => {
            *slot = other;
            None
        }
    }
}

pub fn take_qr_preview_rgba() -> Option<(u32, u32, Vec<u8>)> {
    let mut slot = preview_slot().lock().ok()?;
    let preview = slot.take()?;
    Some((preview.w, preview.h, preview.rgba))
}

pub fn stop_qr_scan() {
    cancel_flag().store(true, Ordering::SeqCst);
    ndk::stop_camera();
    if let Ok(mut slot) = scan_slot().lock() {
        if matches!(*slot, Scan::Busy) {
            *slot = Scan::Ready(Err("Scan stopped".into()));
        }
    }
    if let Ok(mut p) = preview_slot().lock() {
        *p = None;
    }
}
