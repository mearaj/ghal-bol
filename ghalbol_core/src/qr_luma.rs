//! Shared greyscale QR decode for desktop and Android camera / file paths.

/// Decode the first QR in a greyscale buffer. Caller filters for Ghal Bol invites.
pub fn decode_luma(width: usize, height: usize, gray: &[u8]) -> Result<String, String> {
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

/// Decode the first QR in an encoded PNG / JPEG picture.
#[cfg(target_os = "android")]
pub fn decode_image_bytes(bytes: &[u8]) -> Result<String, String> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| format!("could not open picture: {e}"))?
        .to_luma8();
    decode_luma(img.width() as usize, img.height() as usize, img.as_raw())
}

/// True when the payload is a Ghal Bol connect invite (ignore other QRs in view).
pub fn is_ghalbol_invite(text: &str) -> bool {
    crate::connect_invite_v1::parse_connect_invite_uri(text.trim()).is_ok()
}
