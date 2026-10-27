//! Android camera capture inside this process, via the NDK Camera2 API.
//! Frames are YUV_420_888 from `AImageReader` and converted to I420 for the encoder.

#[cfg(target_os = "android")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(target_os = "android")]
use std::sync::{Mutex, OnceLock};
#[cfg(target_os = "android")]
use tokio::sync::mpsc;

#[cfg(target_os = "android")]
use super::RawVideoFrame;
#[cfg(target_os = "android")]
use super::quality::{CAP_HEIGHT, CAP_WIDTH};
#[cfg(target_os = "android")]
use super::session::VideoControls;

#[cfg(target_os = "android")]
static FRAME_TX: OnceLock<Mutex<Option<mpsc::Sender<RawVideoFrame>>>> = OnceLock::new();
#[cfg(target_os = "android")]
static CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "android")]
static FRAMES_RX: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "android")]
fn frame_tx() -> &'static Mutex<Option<mpsc::Sender<RawVideoFrame>>> {
    FRAME_TX.get_or_init(|| Mutex::new(None))
}

/// Pack Android `YUV_420_888` planes into I420 (`Y` then `U` then `V`).
#[cfg(any(target_os = "android", test))]
pub fn yuv420888_to_i420(
    width: u32,
    height: u32,
    y: &[u8],
    y_row: usize,
    u: &[u8],
    u_row: usize,
    u_pix: usize,
    v: &[u8],
    v_row: usize,
    v_pix: usize,
) -> Option<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 || y_row < w || u_pix == 0 || v_pix == 0 {
        return None;
    }
    let y_need = y_row * (h - 1) + w;
    let uv_h = h / 2;
    let uv_w = w / 2;
    let u_need = u_row * (uv_h - 1) + (uv_w - 1) * u_pix + 1;
    let v_need = v_row * (uv_h - 1) + (uv_w - 1) * v_pix + 1;
    if y.len() < y_need || u.len() < u_need || v.len() < v_need {
        return None;
    }
    let mut out = vec![0u8; w * h + 2 * uv_w * uv_h];
    for row in 0..h {
        let src = row * y_row;
        let dst = row * w;
        out[dst..dst + w].copy_from_slice(&y[src..src + w]);
    }
    let u_off = w * h;
    let v_off = u_off + uv_w * uv_h;
    for row in 0..uv_h {
        for col in 0..uv_w {
            out[u_off + row * uv_w + col] = u[row * u_row + col * u_pix];
            out[v_off + row * uv_w + col] = v[row * v_row + col * v_pix];
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planar_yuv_packs_to_i420() {
        let y = [1u8, 2, 3, 4];
        let u = [5u8];
        let v = [6u8];
        let out = yuv420888_to_i420(2, 2, &y, 2, &u, 1, 1, &v, 1, 1).unwrap();
        assert_eq!(&out[..4], &[1, 2, 3, 4]);
        assert_eq!(out[4], 5);
        assert_eq!(out[5], 6);
    }

    #[test]
    fn semi_planar_chroma_uses_pixel_stride() {
        let y = [9u8, 8, 7, 6];
        let uv = [1u8, 2];
        let out = yuv420888_to_i420(2, 2, &y, 2, &uv, 2, 2, &uv[1..], 2, 2).unwrap();
        assert_eq!(out[4], 1);
        assert_eq!(out[5], 2);
    }
}

#[cfg(target_os = "android")]
mod ndk_camera {
    use std::os::raw::c_void;
    use std::sync::{Mutex, OnceLock};

    use crate::android_camera::{
        AImage, AImage_delete, AImage_getPlaneData, AImage_getPlanePixelStride,
        AImage_getPlaneRowStride, AImageReader, AImageReader_acquireLatestImage, Camera, Facing,
    };

    use super::*;

    static LIVE: OnceLock<Mutex<Option<Camera>>> = OnceLock::new();

    fn live() -> &'static Mutex<Option<Camera>> {
        LIVE.get_or_init(|| Mutex::new(None))
    }

    unsafe extern "C" fn on_image(_ctx: *mut c_void, reader: *mut AImageReader) {
        unsafe {
        if !CAPTURE_ACTIVE.load(Ordering::Relaxed) {
            return;
        }
        let mut image = std::ptr::null_mut();
        if AImageReader_acquireLatestImage(reader, &mut image) != 0 || image.is_null() {
            return;
        }
        let frame = read_i420(image);
        AImage_delete(image);
        let Some(frame) = frame else {
            return;
        };
        if let Ok(guard) = frame_tx().lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.try_send(frame);
                let n = FRAMES_RX.fetch_add(1, Ordering::Relaxed) + 1;
                if n == 1 {
                    crate::p2p::native_log::info("call_video", "android camera first frame");
                }
            }
        }
        }
    }

    fn read_i420(image: *mut AImage) -> Option<RawVideoFrame> {
        unsafe {
        let mut y_ptr = std::ptr::null_mut();
        let mut u_ptr = std::ptr::null_mut();
        let mut v_ptr = std::ptr::null_mut();
        let mut y_len = 0;
        let mut u_len = 0;
        let mut v_len = 0;
        let mut y_row = 0;
        let mut u_row = 0;
        let mut v_row = 0;
        let mut u_pix = 0;
        let mut v_pix = 0;
        if AImage_getPlaneData(image, 0, &mut y_ptr, &mut y_len) != 0
            || AImage_getPlaneData(image, 1, &mut u_ptr, &mut u_len) != 0
            || AImage_getPlaneData(image, 2, &mut v_ptr, &mut v_len) != 0
            || AImage_getPlaneRowStride(image, 0, &mut y_row) != 0
            || AImage_getPlaneRowStride(image, 1, &mut u_row) != 0
            || AImage_getPlaneRowStride(image, 2, &mut v_row) != 0
            || AImage_getPlanePixelStride(image, 1, &mut u_pix) != 0
            || AImage_getPlanePixelStride(image, 2, &mut v_pix) != 0
        {
            return None;
        }
        if y_ptr.is_null() || u_ptr.is_null() || v_ptr.is_null() {
            return None;
        }
        let y = std::slice::from_raw_parts(y_ptr, y_len.max(0) as usize);
        let u = std::slice::from_raw_parts(u_ptr, u_len.max(0) as usize);
        let v = std::slice::from_raw_parts(v_ptr, v_len.max(0) as usize);
        let data = yuv420888_to_i420(
            CAP_WIDTH,
            CAP_HEIGHT,
            y,
            y_row.max(0) as usize,
            u,
            u_row.max(0) as usize,
            u_pix.max(1) as usize,
            v,
            v_row.max(0) as usize,
            v_pix.max(1) as usize,
        )?;
        Some(RawVideoFrame {
            width: CAP_WIDTH,
            height: CAP_HEIGHT,
            data,
        })
        }
    }

    pub fn start() -> Result<(), String> {
        stop();
        let camera = Camera::open(Facing::Front, CAP_WIDTH as i32, CAP_HEIGHT as i32, on_image)?;
        if let Ok(mut g) = live().lock() {
            *g = Some(camera);
        }
        Ok(())
    }

    pub fn stop() {
        // Take under the lock; the camera tears down when dropped outside it.
        let camera = live().lock().ok().and_then(|mut g| g.take());
        drop(camera);
    }
}

/// Start camera capture. Desktop uses nokhwa; Android uses the NDK camera.
#[cfg(target_os = "android")]
pub fn spawn(controls: VideoControls) -> Result<mpsc::Receiver<RawVideoFrame>, String> {
    let (tx, rx) = mpsc::channel::<RawVideoFrame>(4);
    if let Ok(mut g) = frame_tx().lock() {
        *g = Some(tx);
    }
    CAPTURE_ACTIVE.store(true, Ordering::Relaxed);
    if let Err(e) = ndk_camera::start() {
        CAPTURE_ACTIVE.store(false, Ordering::Relaxed);
        return Err(e);
    }
    tokio::spawn(async move {
        while !controls.is_stopped() {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        stop_capture();
    });
    Ok(rx)
}

#[cfg(target_os = "android")]
pub fn stop_capture() {
    CAPTURE_ACTIVE.store(false, Ordering::Relaxed);
    FRAMES_RX.store(0, Ordering::Relaxed);
    ndk_camera::stop();
    if let Ok(mut g) = frame_tx().lock() {
        g.take();
    }
}

