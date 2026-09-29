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

/// Opens the camera on a worker thread and looks for a QR code for a few seconds.
pub fn start_qr_scan() -> Result<(), String> {
    {
        let mut slot = scan_slot().lock().map_err(|e| e.to_string())?;
        if matches!(*slot, Scan::Busy) {
            return Ok(());
        }
        *slot = Scan::Busy;
    }
    std::thread::Builder::new()
        .name("ghal_bol-qr".into())
        .spawn(|| {
            let result = scan_camera_blocking();
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

fn scan_camera_blocking() -> Result<String, String> {
    let mut camera = open_camera()?;
    camera.open_stream().map_err(|e| format!("camera stream: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut last = String::from("No QR code in view");
    while std::time::Instant::now() < deadline {
        let frame = match camera.frame() {
            Ok(f) => f,
            Err(e) => {
                last = format!("camera frame: {e}");
                continue;
            }
        };
        let decoded = match frame.decode_image::<RgbFormat>() {
            Ok(img) => img,
            Err(e) => {
                last = format!("camera decode: {e}");
                continue;
            }
        };
        let (w, h) = (decoded.width() as usize, decoded.height() as usize);
        let raw = decoded.as_raw();
        let mut gray = vec![0u8; w * h];
        for i in 0..w * h {
            let o = i * 3;
            if o + 2 < raw.len() {
                gray[i] = ((raw[o] as u16 + raw[o + 1] as u16 + raw[o + 2] as u16) / 3) as u8;
            }
        }
        match decode_luma(w, h, &gray) {
            Ok(text) => {
                let _ = camera.stop_stream();
                return Ok(text);
            }
            Err(e) => last = e,
        }
    }
    let _ = camera.stop_stream();
    Err(last)
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
    let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
    let mut indices = Vec::new();
    if let Ok(devices) = query(nokhwa::utils::ApiBackend::Auto) {
        for (i, _) in devices.iter().enumerate() {
            indices.push(CameraIndex::Index(i as u32));
        }
    }
    if indices.is_empty() {
        indices.push(CameraIndex::Index(0));
    }
    let mut last = String::from("no camera");
    for index in indices {
        match Camera::new(index, requested) {
            Ok(cam) => return Ok(cam),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
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
}
