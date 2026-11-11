//! Screenshots in the chat: the last F12 goes up to the website as a JPEG, and its link goes in the
//! chat, which new games show as a screenshot to open and older ones as a link. (Not in `online`:
//! the server shares that file, and has no use for images.)

use crate::online::WEBSITE;

/// A chat line that's a screenshot (`shot_url`): its id. Only the website's own, so that a line
/// can't make the game fetch something from anywhere else.
pub fn shot_in(text: &str) -> Option<u128> {
    let id = text.trim().strip_prefix(WEBSITE)?.strip_prefix("/shot/")?;
    (id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| u128::from_str_radix(id, 16).ok())?
}

pub fn shot_url(id: u128) -> String {
    format!("{WEBSITE}/shot/{id:032x}")
}

/// The longest a screenshot gets sent (it's for looking at in chat, not printing).
const SHOT_WIDTH: u32 = 1600;
/// The most read back.
const SHOT_MAX_BYTES: u64 = 3 << 20;
const SHOT_MAX_SIDE: u32 = 4096;

/// `rgb` as the JPEG that's sent: at most `SHOT_WIDTH` wide, and under what the server takes.
pub fn shot_jpeg(w: u32, h: u32, rgb: Vec<u8>) -> Result<Vec<u8>, String> {
    let image = image::RgbImage::from_raw(w, h, rgb).ok_or("the screenshot is broken")?;
    let image = if w > SHOT_WIDTH {
        let height = (h as u64 * SHOT_WIDTH as u64 / w as u64).max(1) as u32;
        image::imageops::resize(
            &image,
            SHOT_WIDTH,
            height,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    for quality in [85, 70, 50] {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
            .encode_image(&image)
            .map_err(|e| e.to_string())?;
        if out.len() < 2 << 20 {
            return Ok(out);
        }
    }
    Err("that screenshot is too big to send".into())
}

/// Uploads a JPEG with the game's `token`; the id it's known by now.
pub fn upload_shot(token: &str, jpeg: Vec<u8>) -> Result<u128, String> {
    crate::offline::internet()?;
    let reply = ureq::post(format!("{WEBSITE}/shot"))
        .config()
        .http_status_as_error(false)
        .build()
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "image/jpeg")
        .send(&jpeg[..]);
    let mut reply = reply.map_err(|e| e.to_string())?;
    let ok = reply.status().is_success();
    let text = reply
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    if !ok {
        return Err(text);
    }
    u128::from_str_radix(text.trim(), 16).map_err(|_| "the server's answer made no sense".into())
}

/// A screenshot as RGBA: width, height and pixels.
pub type Pixels = (u32, u32, Vec<u8>);

/// Fetches a screenshot.
pub fn fetch_shot(id: u128) -> Result<Pixels, String> {
    crate::offline::internet()?;
    use std::io::Read;
    let mut reply = ureq::get(shot_url(id)).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    reply
        .body_mut()
        .as_reader()
        .take(SHOT_MAX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    decode_shot(&bytes)
}

fn decode_shot(bytes: &[u8]) -> Result<Pixels, String> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(SHOT_MAX_SIDE);
    limits.max_image_height = Some(SHOT_MAX_SIDE);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    Ok((image.width(), image.height(), image.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screenshots_link_is_known_only_from_our_own_site() {
        let id = 0x0123456789abcdef0123456789abcdef_u128;
        assert_eq!(shot_in(&shot_url(id)), Some(id));
        assert_eq!(shot_in(&format!(" {} ", shot_url(id))), Some(id));
        let elsewhere = shot_url(id).replace(WEBSITE, "https://example.com");
        for text in [
            elsewhere.as_str(),
            "look at this",
            &format!("{}/shot/nothex", WEBSITE),
            &format!("{} and more", shot_url(id)),
            &format!("{}/shot/../../a", WEBSITE),
        ] {
            assert_eq!(shot_in(text), None, "{text}");
        }
    }

    #[test]
    fn a_screenshot_goes_out_small_enough_and_comes_back() {
        let (w, h) = (1920, 1080);
        let rgb: Vec<u8> = (0..w * h * 3).map(|i| (i * 31 % 251) as u8).collect();
        let jpeg = shot_jpeg(w, h, rgb).unwrap();
        assert!(jpeg.len() < 2 << 20);
        let (bw, bh, px) = decode_shot(&jpeg).unwrap();
        assert_eq!((bw, bh), (1600, 900));
        assert_eq!(px.len(), (bw * bh * 4) as usize);
        assert!(decode_shot(b"not a jpeg").is_err());
        assert!(shot_jpeg(10, 10, vec![0; 5]).is_err());
    }
}
