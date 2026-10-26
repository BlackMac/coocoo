//! Image types Coocoo stores (card images, user message and tool result images) and their pixel
//! size.

/// Directory of timeline images (user messages, tool results) inside an agent's data directory.
pub(crate) const DIR: &str = "images";

/// File extension for an accepted image type (no SVG: it can carry scripts).
pub(crate) fn ext(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => return None,
    })
}

/// MIME type for a file extension [`ext`] produces.
pub(crate) fn mime(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    })
}

/// Width and height from a PNG, JPEG, GIF or WebP header.
pub(crate) fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let be16 = |at: usize| Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let le16 = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let be32 = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let le24 = |at: usize| {
        let b = bytes.get(at..at + 3)?;
        Some(b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16)
    };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.get(12..16) == Some(b"IHDR") {
        return Some((be32(16)?, be32(20)?));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some((le16(6)?, le16(8)?));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return match bytes.get(12..16)? {
            // Lossy: frame tag, start code, then 14-bit width and height.
            b"VP8 " if bytes.get(23..26) == Some(&[0x9d, 0x01, 0x2a]) => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            // Lossless: signature byte, then 14-bit width-1 and height-1.
            b"VP8L" if bytes.get(20) == Some(&0x2f) => {
                let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            // Extended: 24-bit canvas width-1 and height-1.
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        };
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        // Walk the marker segments to the first start-of-frame.
        let mut at = 2;
        loop {
            while *bytes.get(at)? != 0xff {
                at += 1;
            }
            while *bytes.get(at)? == 0xff {
                at += 1;
            }
            let marker = *bytes.get(at)?;
            at += 1;
            match marker {
                0xd8 | 0x01 | 0xd0..=0xd7 => continue,
                0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => return Some((be16(at + 5)?, be16(at + 3)?)),
                _ => at += be16(at)? as usize,
            }
        }
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal PNG header for `width`×`height` (enough for [`dimensions`]).
    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    #[test]
    fn dimensions_from_headers() {
        assert_eq!(dimensions(&png(1568, 980)), Some((1568, 980)));
        assert_eq!(dimensions(b"GIF89a\x10\x02\x20\x01"), Some((528, 288)));
        // WebP lossy (VP8), lossless (VP8L) and extended (VP8X).
        let mut vp8 = b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0\0\x9d\x01\x2a".to_vec();
        vp8.extend_from_slice(&[0x20, 0x06, 0x15, 0x04]);
        assert_eq!(dimensions(&vp8), Some((1568, 1045)));
        let bits: u32 = (1568 - 1) | (1045 - 1) << 14;
        let mut vp8l = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
        vp8l.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(dimensions(&vp8l), Some((1568, 1045)));
        let mut vp8x = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        vp8x.extend_from_slice(&[0x1f, 0x06, 0x00, 0x14, 0x04, 0x00]);
        assert_eq!(dimensions(&vp8x), Some((1568, 1045)));
        // JPEG: APP0 segment, then SOF0 with height 980, width 1568.
        let jpeg = [
            &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0, 0][..],
            &[0xff, 0xc0, 0x00, 0x11, 0x08, 0x03, 0xd4, 0x06, 0x20],
        ]
        .concat();
        assert_eq!(dimensions(&jpeg), Some((1568, 980)));
        assert_eq!(dimensions(b"not an image"), None);
        assert_eq!(dimensions(&[0xff, 0xd8, 0xff]), None, "truncated JPEG");
    }
}
