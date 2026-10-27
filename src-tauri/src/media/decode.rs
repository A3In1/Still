use std::{io::Read, path::Path};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Webp,
    Gif,
    Heif,
    Avif,
    Tiff,
    Bmp,
    Raw,
    Jxl,
    Video,
    Unknown,
}
/// Camera RAW containers; ImageIO decodes them (or their embedded preview) on macOS.
pub const RAW_EXTS: &[&str] = &[
    "raw", "dng", "cr2", "cr3", "nef", "nrw", "arw", "srf", "raf", "orf", "rw2", "pef", "srw",
];
fn raw_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| RAW_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}
pub fn sniff(path: &Path) -> Kind {
    let mut bytes = [0u8; 64];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut bytes))
        .unwrap_or(0);
    let b = &bytes[..n];
    if b.starts_with(b"\xff\xd8\xff") {
        Kind::Jpeg
    } else if b.starts_with(b"\x89PNG") {
        Kind::Png
    } else if b.starts_with(b"GIF8") {
        Kind::Gif
    } else if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        Kind::Webp
    } else if b.starts_with(b"\xff\x0a") || b.get(4..8) == Some(b"JXL ") {
        Kind::Jxl
    } else if b.starts_with(b"FUJIFILMCCD-RAW")
        || [b"IIRO".as_slice(), b"IIRS", b"MMOR", b"IIU\0"].iter().any(|m| b.starts_with(m))
    {
        Kind::Raw
    } else if b.get(4..8) == Some(b"ftyp") {
        if b.get(8..12) == Some(b"crx ") {
            Kind::Raw
        } else if b.windows(4).any(|s| s == b"avif" || s == b"avis") {
            Kind::Avif
        } else if b.windows(4).any(|s| {
            [
                b"heic".as_slice(),
                b"heix",
                b"hevc",
                b"hevx",
                b"mif1",
                b"msf1",
            ]
            .contains(&s)
        }) {
            Kind::Heif
        } else {
            Kind::Video
        }
    } else if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") {
        // DNG/CR2/NEF/ARW/PEF/SRW are TIFF containers; the image crate cannot develop them.
        if raw_extension(path) {
            Kind::Raw
        } else {
            Kind::Tiff
        }
    } else if b.starts_with(b"BM") {
        Kind::Bmp
    } else {
        Kind::Unknown
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn gphotos_decode(
        path: *const std::ffi::c_char,
        output: *const std::ffi::c_char,
        edge: u32,
    ) -> i32;
    fn gphotos_frame(
        path: *const std::ffi::c_char,
        output: *const std::ffi::c_char,
        ms: i64,
        edge: u32,
    ) -> i32;
}
#[cfg(target_os = "macos")]
pub fn native_file(path: &Path, output: &Path, edge: u32, ms: Option<i64>) -> Result<(), String> {
    use std::ffi::CString;
    let path = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    let output = CString::new(output.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    let ok = unsafe {
        match ms {
            Some(t) => gphotos_frame(path.as_ptr(), output.as_ptr(), t, edge),
            None => gphotos_decode(path.as_ptr(), output.as_ptr(), edge),
        }
    };
    if ok == 0 {
        Err("The system codec could not decode this media".into())
    } else {
        Ok(())
    }
}
pub fn open_oriented(
    path: &Path,
    edge: Option<u32>,
) -> Result<image::DynamicImage, image::ImageError> {
    let _span = super::perf::Span::new("decode");
    use image::ImageDecoder;
    // Reduced-size previews use ImageIO for every still format. Keep the full-size
    // JPEG face/crop path unchanged so stored oriented coordinates retain their contract.
    #[cfg(target_os = "macos")]
    if edge.is_some() || matches!(sniff(path), Kind::Heif | Kind::Avif | Kind::Raw | Kind::Jxl) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let extension = if edge.map(|edge| edge <= 512).unwrap_or(false) {
            "png"
        } else {
            "jpg"
        };
        let temp = std::env::temp_dir().join(format!(
            "gphotos-decode-{}-{}.{extension}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let result = native_file(path, &temp, edge.unwrap_or(0), None)
            .map_err(std::io::Error::other)
            .map_err(image::ImageError::IoError)
            .and_then(|_| image::open(&temp));
        let _ = std::fs::remove_file(temp);
        if result.is_ok() {
            return result;
        }
        // Unsupported native codecs can still use the existing image-crate decoder.
        // Never apply EXIF a second time to successful ImageIO output.
    }
    if let Some(edge) = edge.filter(|edge| *edge <= 512) {
        if let Some(image) = embedded_thumbnail(path, edge) {
            return Ok(image);
        }
    }
    let mut decoder = image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(if let Some(edge) = edge {
        image.thumbnail(edge, edge)
    } else {
        image
    })
}

/// Best-effort fallback for JPEG/TIFF EXIF thumbnails. Offsets refer to the TIFF
/// buffer returned by kamadak-exif, never to unchecked positions in the original.
fn embedded_thumbnail(path: &Path, edge: u32) -> Option<image::DynamicImage> {
    use exif::{In, Tag};
    let mut file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let exif = exif::Reader::new().read_from_container(&mut file).ok()?;
    let start = exif
        .get_field(Tag::JPEGInterchangeFormat, In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    let length = exif
        .get_field(Tag::JPEGInterchangeFormatLength, In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    let bytes = exif.buf().get(start..start.checked_add(length)?)?;
    let mut image = image::load_from_memory(bytes).ok()?;
    if image.width().max(image.height()) < 256 {
        return None;
    }
    let orientation = exif
        .get_field(Tag::Orientation, In::THUMBNAIL)
        .or_else(|| exif.get_field(Tag::Orientation, In::PRIMARY))
        .and_then(|f| f.value.get_uint(0))
        .and_then(|v| u8::try_from(v).ok())
        .and_then(image::metadata::Orientation::from_exif);
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Some(image.thumbnail(edge, edge))
}
