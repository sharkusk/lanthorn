//! Blorb pictures for Version 6: sizes for `picture_data` (ZMSD §15) and the
//! PNG/JPEG bytes a terminal host can draw.

use std::path::Path;

/// How a `Pict` resource is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rect,
    Png,
    Jpeg,
}

/// One Blorb picture, in the archive's own pixels.
#[derive(Clone, Debug)]
pub struct Pict {
    pub number: u16,
    pub width: u16,
    pub height: u16,
    pub kind: Kind,
    pub bytes: Vec<u8>,
}

/// Standard window from a Blorb `Reso` chunk, then each Pict.
pub fn load(path: &Path) -> (Option<(u16, u16)>, Vec<Pict>) {
    let Some((blorb, _)) = blorb::resolve_resource_blorb(path) else {
        return (None, Vec::new());
    };
    let std = blorb.std_window();
    let mut picts = Vec::new();
    for entry in blorb.resources() {
        if &entry.usage != b"Pict" {
            continue;
        }
        let Some((ty, data)) = blorb.resource(b"Pict", entry.number) else {
            continue;
        };
        let Some((kind, w, h)) = classify(ty, data) else {
            continue;
        };
        let Ok(number) = u16::try_from(entry.number) else {
            continue;
        };
        picts.push(Pict {
            number,
            width: w,
            height: h,
            kind,
            bytes: data.to_vec(),
        });
    }
    (std, picts)
}


fn classify(ty: &[u8; 4], data: &[u8]) -> Option<(Kind, u16, u16)> {
    match ty {
        b"Rect" => rect_dims(data).map(|(w, h)| (Kind::Rect, w, h)),
        b"PNG " => png_ihdr(data).map(|(w, h)| (Kind::Png, w, h)),
        b"JPEG" => jpeg_sof(data).map(|(w, h)| (Kind::Jpeg, w, h)),
        _ => png_ihdr(data)
            .map(|(w, h)| (Kind::Png, w, h))
            .or_else(|| jpeg_sof(data).map(|(w, h)| (Kind::Jpeg, w, h)))
            .or_else(|| rect_dims(data).map(|(w, h)| (Kind::Rect, w, h))),
    }
}


fn clamp_u16(v: u32) -> Option<u16> {
    u16::try_from(v).ok()
}

/// Blorb `Rect`: two big-endian u32s, width then height.
fn rect_dims(data: &[u8]) -> Option<(u16, u16)> {
    if data.len() < 8 {
        return None;
    }
    let w = u32::from_be_bytes(data[0..4].try_into().ok()?);
    let h = u32::from_be_bytes(data[4..8].try_into().ok()?);
    Some((clamp_u16(w)?, clamp_u16(h)?))
}

/// PNG `IHDR` width and height. Signature plus the first chunk.
fn png_ihdr(data: &[u8]) -> Option<(u16, u16)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if data.len() < 24 || !data.starts_with(SIG) {
        return None;
    }
    if &data[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(data[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(data[20..24].try_into().ok()?);
    Some((clamp_u16(w)?, clamp_u16(h)?))
}

/// JPEG SOF0/SOF1/SOF2: height then width, big-endian u16.
fn jpeg_sof(data: &[u8]) -> Option<(u16, u16)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            return None;
        }
        while i < data.len() && data[i] == 0xFF {
            i += 1;
        }
        if i >= data.len() {
            return None;
        }
        let marker = data[i];
        i += 1;
        if marker == 0xD9 || marker == 0xDA {
            return None;
        }
        if i + 1 >= data.len() {
            return None;
        }
        let len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
        if len < 2 || i + len > data.len() {
            return None;
        }
        if matches!(marker, 0xC0..=0xC2) && len >= 7 {
            let h = u16::from_be_bytes([data[i + 3], data[i + 4]]);
            let w = u16::from_be_bytes([data[i + 5], data[i + 6]]);
            return Some((w, h));
        }
        i += len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_chunk_is_width_then_height() {
        let mut b = [0u8; 8];
        b[0..4].copy_from_slice(&320u32.to_be_bytes());
        b[4..8].copy_from_slice(&200u32.to_be_bytes());
        assert_eq!(rect_dims(&b), Some((320, 200)));
        assert_eq!(classify(b"Rect", &b), Some((Kind::Rect, 320, 200)));
    }

    #[test]
    fn png_ihdr_names_the_pixel_size() {
        let mut png = Vec::from(*b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&48u32.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0]);
        assert_eq!(png_ihdr(&png), Some((64, 48)));
        assert_eq!(classify(b"PNG ", &png), Some((Kind::Png, 64, 48)));
    }

    #[test]
    fn jpeg_sof0_names_the_pixel_size() {
        // SOI + SOF0. The SOF length word counts itself, so the slice must
        // cover `i + len` exclusive of the marker pair.
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 8];
        jpg.extend_from_slice(&120u16.to_be_bytes());
        jpg.extend_from_slice(&80u16.to_be_bytes());
        jpg.extend_from_slice(&[1, 0x11, 0, 0]);
        assert_eq!(jpeg_sof(&jpg), Some((80, 120)));
        assert_eq!(classify(b"JPEG", &jpg), Some((Kind::Jpeg, 80, 120)));
    }

    #[test]
    fn missing_blorb_is_empty() {
        let (std, picts) = load(Path::new("/no/such/story.z6"));
        assert_eq!(std, None);
        assert!(picts.is_empty());
    }
}
