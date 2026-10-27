//! Call video pictures for the UI: the latest local and remote frames as small PNGs.
//! Desktop and Android share this; capture and decode live in `call_video`.

use std::sync::Mutex;

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
    if let Some((frame, frame_gen)) =
        crate::call_video::latest_local_preview(call_id, slot.local_gen)
    {
        slot.local_gen = frame_gen;
        slot.local_png = rgba_to_png(
            &crate::call_video::i420_to_rgba(&frame),
            frame.width,
            frame.height,
        );
        local_out = slot.local_png.clone();
    }
    if let Some((frame, frame_gen)) =
        crate::call_video::latest_decoded_frame(call_id, slot.remote_gen)
    {
        slot.remote_gen = frame_gen;
        slot.remote_png = rgba_to_png(
            &crate::call_video::i420_to_rgba(&frame),
            frame.width,
            frame.height,
        );
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
