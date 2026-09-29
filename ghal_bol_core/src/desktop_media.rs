//! Desktop camera pictures for the call UI, and QR decode from the camera or a file.

use std::sync::Mutex;

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::{Camera, query};

struct Pictures {
    local_gen: u64,
    local_png: Option<Vec<u8>>,
    remote_gen: u64,
    remote_png: Option<Vec<u8>>,
}

fn pictures() -> &'static Mutex<Pictures> {
    static P: std::sync::OnceLock<Mutex<Pictures>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        Mutex::new(Pictures {
            local_gen: 0,
            local_png: None,
            remote_gen: 0,
            remote_png: None,
        })
    })
}

/// Latest local and remote call pictures as PNG, or empty when that side has no new frame.
pub fn call_picture_pngs(call_id: &str) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let mut slot = match pictures().lock() {
        Ok(g) => g,
        Err(_) => return (None, None),
    };
    let mut local_out = None;
    let mut remote_out = None;
    if let Some((frame, frame_gen)) = crate::call_video::latest_local_preview(call_id, slot.local_gen) {
        slot.local_gen = frame_gen;
        slot.local_png = rgba_to_png(&crate::call_video::i420_to_rgba(&frame), frame.width, frame.height);
        local_out = slot.local_png.clone();
    }
    if let Some((frame, frame_gen)) = crate::call_video::latest_decoded_frame(call_id, slot.remote_gen) {
        slot.remote_gen = frame_gen;
        slot.remote_png = rgba_to_png(&crate::call_video::i420_to_rgba(&frame), frame.width, frame.height);
        remote_out = slot.remote_png.clone();
    }
    (local_out, remote_out)
}

pub fn clear_call_pictures() {
    if let Ok(mut slot) = pictures().lock() {
        *slot = Pictures {
            local_gen: 0,
            local_png: None,
            remote_gen: 0,
            remote_png: None,
        };
    }
}

fn rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let (rgba, width, height) = downscale(rgba, width, height, 320);
    if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
        return None;
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(&rgba).ok()?;
    drop(writer);
    Some(out)
}

fn downscale(src: &[u8], width: u32, height: u32, max_w: u32) -> (Vec<u8>, u32, u32) {
    if width <= max_w || width == 0 || height == 0 {
        return (src.to_vec(), width, height);
    }
    let dst_w = max_w;
    let dst_h = ((height as u64 * dst_w as u64) / width as u64).max(1) as u32;
    let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for y in 0..dst_h {
        let sy = (y as u64 * height as u64 / dst_h as u64) as u32;
        for x in 0..dst_w {
            let sx = (x as u64 * width as u64 / dst_w as u64) as u32;
            let si = ((sy as usize) * (width as usize) + sx as usize) * 4;
            let di = ((y as usize) * (dst_w as usize) + x as usize) * 4;
            if si + 3 < src.len() {
                dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
    (dst, dst_w, dst_h)
}

enum Scan {
    Idle,
    Busy,
    Ready(Result<String, String>),
}

fn scan_slot() -> &'static Mutex<Scan> {
    static S: std::sync::OnceLock<Mutex<Scan>> = std::sync::OnceLock::new();
    S.get_or_init(|| Mutex::new(Scan::Idle))
}

fn qr_cancel() -> &'static std::sync::atomic::AtomicBool {
    static C: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    &C
}

struct PreviewFrame {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    frame_gen: u64,
    served: u64,
}

fn qr_preview() -> &'static Mutex<PreviewFrame> {
    static P: std::sync::OnceLock<Mutex<PreviewFrame>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        Mutex::new(PreviewFrame {
            rgba: Vec::new(),
            width: 0,
            height: 0,
            frame_gen: 0,
            served: 0,
        })
    })
}

/// Downscale RGB → RGBA into the preview slot (no PNG — UI uploads a live texture).
fn publish_preview_rgb(rgb: &[u8], width: u32, height: u32) {
    if width == 0 || height == 0 {
        return;
    }
    // Modest preview size keeps UI texture uploads cheap on every platform.
    const MAX_W: u32 = 320;
    let (dst_w, dst_h) = if width <= MAX_W {
        (width, height)
    } else {
        let dst_w = MAX_W;
        let dst_h = ((height as u64 * dst_w as u64) / width as u64).max(1) as u32;
        (dst_w, dst_h)
    };
    let mut rgba = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for y in 0..dst_h {
        let sy = (y as u64 * height as u64 / dst_h as u64) as u32;
        for x in 0..dst_w {
            let sx = (x as u64 * width as u64 / dst_w as u64) as u32;
            let si = ((sy as usize) * (width as usize) + sx as usize) * 3;
            let di = ((y as usize) * (dst_w as usize) + x as usize) * 4;
            if si + 2 < rgb.len() {
                rgba[di] = rgb[si];
                rgba[di + 1] = rgb[si + 1];
                rgba[di + 2] = rgb[si + 2];
                rgba[di + 3] = 255;
            }
        }
    }
    if let Ok(mut slot) = qr_preview().lock() {
        slot.rgba = rgba;
        slot.width = dst_w;
        slot.height = dst_h;
        slot.frame_gen = slot.frame_gen.saturating_add(1);
    }
}

fn clear_qr_preview() {
    if let Ok(mut slot) = qr_preview().lock() {
        slot.rgba.clear();
        slot.width = 0;
        slot.height = 0;
        slot.frame_gen = slot.frame_gen.saturating_add(1);
        slot.served = slot.frame_gen;
    }
}

/// Latest preview as raw RGBA `(width, height, pixels)` when a new frame is ready.
/// Prefer this over PNG: Makepad's `load_png_from_data` builds mipmaps on Linux and does
/// not redraw outside a draw event — that is what looked like multi-minute freezes.
pub fn take_qr_preview_rgba() -> Option<(u32, u32, Vec<u8>)> {
    let mut slot = qr_preview().lock().ok()?;
    if slot.frame_gen == slot.served || slot.width == 0 || slot.height == 0 || slot.rgba.is_empty() {
        return None;
    }
    slot.served = slot.frame_gen;
    Some((slot.width, slot.height, slot.rgba.clone()))
}

/// Stop an in-flight camera scan (Back / leaving the Scan sheet).
pub fn stop_qr_scan() {
    qr_cancel().store(true, std::sync::atomic::Ordering::SeqCst);
}

fn wait_scan_not_busy(timeout: std::time::Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(slot) = scan_slot().lock() {
            if !matches!(*slot, Scan::Busy) {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Opens the camera on a worker thread and looks for a QR until decode or cancel.
pub fn start_qr_scan() -> Result<(), String> {
    // Restart cleanly if Scan again / re-open while a prior worker is still running.
    qr_cancel().store(true, std::sync::atomic::Ordering::SeqCst);
    wait_scan_not_busy(std::time::Duration::from_secs(2));
    qr_cancel().store(false, std::sync::atomic::Ordering::SeqCst);
    {
        let mut slot = scan_slot().lock().map_err(|e| e.to_string())?;
        if matches!(*slot, Scan::Busy) {
            return Err("Camera is still stopping — try again".into());
        }
        *slot = Scan::Busy;
    }
    clear_qr_preview();
    crate::p2p::native_log::info("qr", "scan start — opening camera on worker");
    std::thread::Builder::new()
        .name("ghal_bol-qr".into())
        .spawn(|| {
            let result = scan_camera_blocking();
            match &result {
                Ok(text) => crate::p2p::native_log::info(
                    "qr",
                    format!("decoded invite ({} chars)", text.len()),
                ),
                Err(e) => crate::p2p::native_log::warn("qr", format!("scan finished: {e}")),
            }
            if let Ok(mut slot) = scan_slot().lock() {
                *slot = Scan::Ready(result);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn take_qr_scan() -> Option<Result<String, String>> {
    let mut slot = scan_slot().lock().ok()?;
    match std::mem::replace(&mut *slot, Scan::Idle) {
        Scan::Ready(result) => Some(result),
        other => {
            *slot = other;
            None
        }
    }
}

fn scan_cancelled() -> bool {
    qr_cancel().load(std::sync::atomic::Ordering::SeqCst)
}

fn scan_camera_blocking() -> Result<String, String> {
    let mut camera = open_camera()?;
    camera
        .open_stream()
        .map_err(|e| format!("camera stream: {e}"))?;
    crate::p2p::native_log::info(
        "qr",
        "camera stream open — preview on capture thread, QR on worker",
    );

    // Capacity 1 + try_send: if QR is still working, drop the frame (never stall preview).
    let (gray_tx, gray_rx) = std::sync::mpsc::sync_channel::<(usize, usize, Vec<u8>)>(1);
    let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<String, String>>();
    std::thread::Builder::new()
        .name("ghal_bol-qr-decode".into())
        .spawn(move || {
            while let Ok((w, h, gray)) = gray_rx.recv() {
                if scan_cancelled() {
                    break;
                }
                let Ok(text) = decode_luma(w, h, &gray) else {
                    continue;
                };
                // Keep scanning until the payload is a Ghal Bol connect invite.
                // Other QR codes in view must not stop the session.
                if crate::connect_invite_v1::parse_connect_invite_uri(text.trim()).is_err() {
                    continue;
                }
                crate::p2p::native_log::info(
                    "qr",
                    format!("ghalbol invite QR detected ({} chars)", text.len()),
                );
                let _ = done_tx.send(Ok(text));
                return;
            }
            let _ = done_tx.send(Err("Scan stopped".into()));
        })
        .map_err(|e| e.to_string())?;

    let mut frames: u64 = 0;
    let mut window_frames: u64 = 0;
    let mut window_start = std::time::Instant::now();

    while !scan_cancelled() {
        if let Ok(result) = done_rx.try_recv() {
            let _ = camera.stop_stream();
            return result;
        }
        let frame = match camera.frame() {
            Ok(f) => f,
            Err(e) => {
                crate::p2p::native_log::warn("qr", format!("camera frame: {e}"));
                if scan_cancelled() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
        };
        let decoded = match frame.decode_image::<RgbFormat>() {
            Ok(img) => img,
            Err(e) => {
                // One-line once — do not spam.
                if frames == 0 {
                    crate::p2p::native_log::warn("qr", format!("camera decode: {e}"));
                }
                continue;
            }
        };
        let (w, h) = (decoded.width() as usize, decoded.height() as usize);
        let raw = decoded.as_raw();
        frames = frames.saturating_add(1);
        window_frames = window_frames.saturating_add(1);
        publish_preview_rgb(raw, w as u32, h as u32);
        if frames == 1 {
            crate::p2p::native_log::info("qr", format!("first frame {w}x{h}"));
        }

        // Full-res luma for the QR worker every frame (try_send drops if still busy).
        {
            let mut gray = vec![0u8; w * h];
            for i in 0..w * h {
                let o = i * 3;
                if o + 2 < raw.len() {
                    gray[i] = ((raw[o] as u16 + raw[o + 1] as u16 + raw[o + 2] as u16) / 3) as u8;
                }
            }
            let _ = gray_tx.try_send((w, h, gray));
        }

        if window_start.elapsed() >= std::time::Duration::from_secs(2) {
            let secs = window_start.elapsed().as_secs_f32().max(0.001);
            let fps = window_frames as f32 / secs;
            crate::p2p::native_log::info(
                "qr",
                format!("preview ~{fps:.0} fps (last 2s, total frames={frames})"),
            );
            window_frames = 0;
            window_start = std::time::Instant::now();
        }
    }

    drop(gray_tx);
    let _ = camera.stop_stream();
    clear_qr_preview();
    // Drain QR worker result if it raced with cancel.
    if let Ok(Ok(text)) = done_rx.try_recv() {
        return Ok(text);
    }
    crate::p2p::native_log::info("qr", format!("cancelled after {frames} frames"));
    Err("Scan stopped".into())
}

pub fn decode_qr_file(path: &str) -> Result<String, String> {
    let img = image::open(path)
        .map_err(|e| format!("could not open picture: {e}"))?
        .to_luma8();
    decode_luma(img.width() as usize, img.height() as usize, img.as_raw())
}

fn decode_luma(width: usize, height: usize, gray: &[u8]) -> Result<String, String> {
    if width == 0 || height == 0 || gray.len() < width * height {
        return Err("empty picture".into());
    }
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| {
        gray[y * width + x]
    });
    let grids = prepared.detect_grids();
    if grids.is_empty() {
        return Err("No QR code in that picture".into());
    }
    grids[0]
        .decode()
        .map(|(_meta, text)| text)
        .map_err(|e| format!("QR decode: {e}"))
}

fn open_camera() -> Result<Camera, String> {
    use nokhwa::utils::{CameraFormat, FrameFormat, Resolution};

    // Modest capture size + high FPS — same idea as call video, portable defaults.
    const CAP_WIDTH: u32 = 640;
    const CAP_HEIGHT: u32 = 480;
    const FPS: u32 = 30;
    let decode_formats = [
        FrameFormat::MJPEG,
        FrameFormat::YUYV,
        FrameFormat::RAWRGB,
        FrameFormat::NV12,
    ];

    let mut indices = Vec::new();
    if let Ok(devices) = query(nokhwa::utils::ApiBackend::Auto) {
        crate::p2p::native_log::info("qr", format!("cameras listed: {}", devices.len()));
        for (i, device) in devices.iter().enumerate() {
            let index = device.index().clone();
            crate::p2p::native_log::info(
                "qr",
                format!("camera[{i}] index={index} {}", device.human_name()),
            );
            indices.push(index);
        }
    } else {
        crate::p2p::native_log::warn("qr", "camera query failed — trying index 0");
    }
    if indices.is_empty() {
        indices.push(CameraIndex::Index(0));
    }

    let mut last = String::from("no camera");
    for index in indices {
        // Prefer Closest CAP size, then highest FPS / auto — mirrors call_video.
        let attempts: Vec<(&str, RequestedFormat)> = decode_formats
            .iter()
            .map(|fmt| {
                (
                    "closest",
                    RequestedFormat::with_formats(
                        RequestedFormatType::Closest(CameraFormat::new(
                            Resolution::new(CAP_WIDTH, CAP_HEIGHT),
                            *fmt,
                            FPS,
                        )),
                        &decode_formats,
                    ),
                )
            })
            .chain([
                (
                    "highest_fps",
                    RequestedFormat::with_formats(
                        RequestedFormatType::HighestFrameRate(FPS),
                        &decode_formats,
                    ),
                ),
                (
                    "auto",
                    RequestedFormat::new::<RgbFormat>(RequestedFormatType::None),
                ),
            ])
            .collect();
        for (strategy, requested) in attempts {
            match try_open_once(index.clone(), requested, strategy) {
                Ok(cam) => return Ok(cam),
                Err(e) => {
                    last = e;
                    if last.contains("busy") {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        if let Ok(cam) = try_open_once(
                            index.clone(),
                            RequestedFormat::with_formats(
                                RequestedFormatType::Closest(CameraFormat::new(
                                    Resolution::new(CAP_WIDTH, CAP_HEIGHT),
                                    FrameFormat::MJPEG,
                                    FPS,
                                )),
                                &decode_formats,
                            ),
                            "retry_after_busy",
                        ) {
                            return Ok(cam);
                        }
                    }
                }
            }
        }
    }
    Err(last)
}

fn try_open_once(
    index: CameraIndex,
    requested: RequestedFormat,
    strategy: &str,
) -> Result<Camera, String> {
    match Camera::new(index.clone(), requested) {
        Ok(cam) => {
            let picked = cam.camera_format();
            crate::p2p::native_log::info(
                "qr",
                format!(
                    "camera opened strategy={strategy} {}x{} {} {}fps",
                    picked.width(),
                    picked.height(),
                    picked.format(),
                    picked.frame_rate(),
                ),
            );
            Ok(cam)
        }
        Err(e) => {
            let last = e.to_string();
            crate::p2p::native_log::warn("qr", format!("open {index} {strategy} failed: {last}"));
            Err(last)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_png_roundtrip() {
        let code = qrcode::QrCode::new(b"ghalbol://connect/test-invite").unwrap();
        let image = code
            .render::<image::Luma<u8>>()
            .min_dimensions(240, 240)
            .build();
        let path = std::env::temp_dir().join("ghal-bol-qr-roundtrip.png");
        image.save(&path).unwrap();
        let text = decode_qr_file(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(text, "ghalbol://connect/test-invite");
    }

    #[test]
    fn invite_uri_qr_roundtrip_accepts_parse() {
        use crate::create_keystore_v1;
        let (_ks, id) = create_keystore_v1("pw", None).unwrap();
        let wire = crate::connect_invite_v1::build_connect_invite_wire_map(
            "ghal-bol-chat",
            &id.public_key_hex(),
            Some("ScanTest"),
        )
        .unwrap();
        let uri = crate::connect_invite_v1::connect_invite_https_uri_from_wire_map(&wire).unwrap();
        let code = qrcode::QrCode::new(uri.as_bytes()).unwrap();
        let image = code
            .render::<image::Luma<u8>>()
            .quiet_zone(true)
            .min_dimensions(280, 280)
            .build();
        let text = decode_luma(
            image.width() as usize,
            image.height() as usize,
            image.as_raw(),
        )
        .unwrap();
        assert_eq!(text, uri);
        let parsed = crate::connect_invite_v1::parse_connect_invite_uri(&text).unwrap();
        crate::connect_invite_v1::verify_ghal_bol_connect_invite_value(&parsed).unwrap();
    }
}
