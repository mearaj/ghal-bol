//! Native camera capture → I420 [`RawVideoFrame`] stream feeding the video engine.
//!
//! Android: Camera2 in `:p2p`. Desktop: nokhwa in the daemon when available, else
//! Flutter `camera` plugin pushes frames via [`super::desktop_video`].

use tokio::sync::mpsc;

use super::session::VideoControls;
use super::RawVideoFrame;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const BACKEND_NONE: u8 = 0;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const BACKEND_NOKHWA: u8 = 1;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const BACKEND_FLUTTER: u8 = 2;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static DESKTOP_CAPTURE_BACKEND: AtomicU8 = AtomicU8::new(BACKEND_NONE);

/// Which desktop capture path is active (`none`, `nokhwa`, `flutter`).
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub fn desktop_capture_backend() -> &'static str {
    match DESKTOP_CAPTURE_BACKEND.load(Ordering::Relaxed) {
        BACKEND_NOKHWA => "nokhwa",
        BACKEND_FLUTTER => "flutter",
        _ => "none",
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn desktop_capture_backend() -> &'static str {
    "none"
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub fn reset_desktop_capture_backend() {
    DESKTOP_CAPTURE_BACKEND.store(BACKEND_NONE, Ordering::Relaxed);
}

/// Start camera capture, returning a receiver of I420 frames.
#[cfg(target_os = "android")]
pub fn spawn_camera_capture(
    controls: VideoControls,
) -> Result<mpsc::Receiver<RawVideoFrame>, String> {
    super::android_video::spawn(controls)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub fn spawn_camera_capture(
    controls: VideoControls,
) -> Result<mpsc::Receiver<RawVideoFrame>, String> {
    reset_desktop_capture_backend();
    if let Some(rx) = nokhwa_capture::try_spawn(controls.clone()) {
        DESKTOP_CAPTURE_BACKEND.store(BACKEND_NOKHWA, Ordering::Relaxed);
        crate::p2p::native_log::info(
            "call_video",
            "desktop capture backend=nokhwa (daemon-owned camera)".to_string(),
        );
        return Ok(rx);
    }
    crate::p2p::native_log::info(
        "call_video",
        "desktop capture backend=flutter (UI pushes frames — nokhwa unavailable)".to_string(),
    );
    DESKTOP_CAPTURE_BACKEND.store(BACKEND_FLUTTER, Ordering::Relaxed);
    super::desktop_video::spawn(controls)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "android"
)))]
pub fn spawn_camera_capture(
    _controls: VideoControls,
) -> Result<mpsc::Receiver<RawVideoFrame>, String> {
    Err("camera capture not available on this platform".to_string())
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod nokhwa_capture {
    use super::*;
    use std::sync::atomic::Ordering;

    use crate::call_video::quality::{CAP_HEIGHT, CAP_WIDTH};
    const CAP_FPS: u32 = 30;

    const DECODE_FORMATS: &[nokhwa::utils::FrameFormat] = &[
        nokhwa::utils::FrameFormat::YUYV,
        nokhwa::utils::FrameFormat::MJPEG,
        nokhwa::utils::FrameFormat::NV12,
        nokhwa::utils::FrameFormat::RAWRGB,
    ];

    fn try_open_at(index: nokhwa::utils::CameraIndex, label: &str) -> Result<nokhwa::Camera, String> {
        use nokhwa::pixel_format::RgbFormat;
        use nokhwa::utils::{CameraFormat, RequestedFormat, RequestedFormatType, Resolution};
        use nokhwa::Camera;

        let attempts: &[(&str, RequestedFormat)] = &[
            (
                "highest_fps",
                RequestedFormat::with_formats(
                    RequestedFormatType::HighestFrameRate(CAP_FPS),
                    DECODE_FORMATS,
                ),
            ),
            (
                "highest_res",
                RequestedFormat::with_formats(
                    RequestedFormatType::HighestResolution(Resolution::new(CAP_WIDTH, CAP_HEIGHT)),
                    DECODE_FORMATS,
                ),
            ),
            (
                "auto",
                RequestedFormat::new::<RgbFormat>(RequestedFormatType::None),
            ),
        ];
        let mut last_err = String::from("no camera attempts");
        for (strategy, requested) in attempts {
            match Camera::new(index.clone(), *requested) {
                Ok(cam) => {
                    let fmt = cam.camera_format();
                    crate::p2p::native_log::info(
                        "call_video",
                        format!(
                            "camera opened {label} strategy={strategy} {}x{} {} {}fps",
                            fmt.width(),
                            fmt.height(),
                            fmt.format(),
                            fmt.frame_rate(),
                        ),
                    );
                    return Ok(cam);
                }
                Err(e) => last_err = format!("{label}/{strategy}: {e}"),
            }
        }
        for fmt in DECODE_FORMATS {
            let requested = RequestedFormat::with_formats(
                RequestedFormatType::Closest(CameraFormat::new(
                    Resolution::new(CAP_WIDTH, CAP_HEIGHT),
                    *fmt,
                    CAP_FPS,
                )),
                DECODE_FORMATS,
            );
            match Camera::new(index.clone(), requested) {
                Ok(cam) => {
                    let picked = cam.camera_format();
                    crate::p2p::native_log::info(
                        "call_video",
                        format!(
                            "camera opened {label} strategy=closest_{fmt} {}x{} {} {}fps",
                            picked.width(),
                            picked.height(),
                            picked.format(),
                            picked.frame_rate(),
                        ),
                    );
                    return Ok(cam);
                }
                Err(e) => last_err = format!("{label}/closest_{fmt}: {e}"),
            }
        }
        Err(last_err)
    }

    fn open_camera() -> Result<nokhwa::Camera, String> {
        use nokhwa::utils::{ApiBackend, CameraIndex};

        let mut indices: Vec<CameraIndex> = Vec::new();
        if let Ok(devices) = nokhwa::query(ApiBackend::Auto) {
            for (i, dev) in devices.iter().enumerate() {
                crate::p2p::native_log::info(
                    "call_video",
                    format!("camera device[{i}]: {}", dev.human_name()),
                );
                indices.push(CameraIndex::Index(i as u32));
            }
        }
        if indices.is_empty() {
            indices.push(CameraIndex::Index(0));
        }

        let mut last_err = String::from("no camera indices");
        for (i, index) in indices.iter().enumerate() {
            match try_open_at(index.clone(), &format!("index={i}")) {
                Ok(cam) => return Ok(cam),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    /// Probe-open the camera on a worker thread; returns `None` when nokhwa cannot access a device.
    pub fn try_spawn(controls: VideoControls) -> Option<mpsc::Receiver<RawVideoFrame>> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let (tx, rx) = mpsc::channel::<RawVideoFrame>(4);
        std::thread::Builder::new()
            .name("ghalbol-camera".to_string())
            .spawn(move || {
                use nokhwa::pixel_format::RgbFormat;
                let mut camera = match open_camera() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if let Err(e) = camera.open_stream() {
                    let _ = ready_tx.send(Err(format!("stream: {e}")));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                crate::p2p::native_log::info("call_video", "nokhwa camera capture started".to_string());
                while !controls.stop.load(Ordering::Relaxed) {
                    let frame = match camera.frame() {
                        Ok(f) => f,
                        Err(e) => {
                            crate::p2p::native_log::warn("call_video", format!("camera frame: {e}"));
                            break;
                        }
                    };
                    let decoded = match frame.decode_image::<RgbFormat>() {
                        Ok(img) => img,
                        Err(e) => {
                            crate::p2p::native_log::warn(
                                "call_video",
                                format!("camera decode failed: {e}"),
                            );
                            continue;
                        }
                    };
                    let (w, h) = (decoded.width(), decoded.height());
                    let (w, h) = (w & !1, h & !1);
                    if w == 0 || h == 0 {
                        continue;
                    }
                    let i420 = rgb_to_i420(
                        decoded.as_raw(),
                        decoded.width() as usize,
                        w as usize,
                        h as usize,
                    );
                    let _ = tx.try_send(RawVideoFrame { width: w, height: h, data: i420 });
                }
                let _ = camera.stop_stream();
                crate::p2p::native_log::info("call_video", "nokhwa camera capture ended".to_string());
            })
            .ok()?;
        match ready_rx.recv_timeout(std::time::Duration::from_secs(3)) {
            Ok(Ok(())) => Some(rx),
            Ok(Err(e)) => {
                crate::p2p::native_log::warn(
                    "call_video",
                    format!("nokhwa camera probe failed: {e}"),
                );
                None
            }
            _ => {
                crate::p2p::native_log::warn(
                    "call_video",
                    "nokhwa camera probe timed out".to_string(),
                );
                None
            }
        }
    }

    fn rgb_to_i420(rgb: &[u8], src_w: usize, w: usize, h: usize) -> Vec<u8> {
        let mut out = vec![0u8; w * h + 2 * ((w / 2) * (h / 2))];
        let (y_plane, uv) = out.split_at_mut(w * h);
        let (u_plane, v_plane) = uv.split_at_mut((w / 2) * (h / 2));
        for y in 0..h {
            for x in 0..w {
                let p = (y * src_w + x) * 3;
                let r = rgb[p] as f32;
                let g = rgb[p + 1] as f32;
                let b = rgb[p + 2] as f32;
                y_plane[y * w + x] = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;
                if y % 2 == 0 && x % 2 == 0 {
                    let cx = x / 2;
                    let cy = y / 2;
                    let u = (-0.169 * r - 0.331 * g + 0.5 * b + 128.0).clamp(0.0, 255.0) as u8;
                    let v = (0.5 * r - 0.419 * g - 0.081 * b + 128.0).clamp(0.0, 255.0) as u8;
                    u_plane[cy * (w / 2) + cx] = u;
                    v_plane[cy * (w / 2) + cx] = v;
                }
            }
        }
        out
    }
}
