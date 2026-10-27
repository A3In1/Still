use exif::{In, Reader as ExifReader, Tag, Value};
use little_exif::exif_tag::ExifTag;
use little_exif::metadata::Metadata;
use little_exif::rational::uR64;
use reverse_geocoder::ReverseGeocoder;
use std::time::SystemTime;

use crate::db::MediaRecord;

lazy_static::lazy_static! {
    pub static ref GEOCODER: ReverseGeocoder = ReverseGeocoder::new();
}

const PHOTO_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "heic", "heif", "hif", "heics", "avif", "apng", "raw", "tiff",
    "tif", "bmp", "gif", "dng", "cr2", "cr3", "nef", "nrw", "arw", "srf", "raf", "orf", "rw2",
    "pef", "srw", "jxl",
];
const VIDEO_EXTS: &[&str] = &["mp4", "mov", "avi", "mkv", "webm", "m4v", "3gp", "ts"];
const THUMB_SIZE: u32 = 512;
/// Versioned independently of face inputs because preview decoding may change.
pub const THUMB_DIR: &str = "thumbs-v4";
const THUMB_QUALITY: f32 = 78.0;
pub const THUMB_VERSION: &str = "thumb-webp512-v4";

pub fn is_supported(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| {
            let ext = s.to_lowercase();
            PHOTO_EXTS.contains(&ext.as_str()) || VIDEO_EXTS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

pub struct Thumbnail {
    pub path: String,
    /// Available for a cold render; cache hits can already have a catalog hash.
    pub thumbhash: Option<String>,
}

pub fn thumbnail(path: &std::path::Path, dir: &std::path::Path) -> Option<Thumbnail> {
    std::fs::create_dir_all(dir).ok()?;
    let cached = thumbnail_cache_path(path, dir)?;
    if cached.is_file() {
        return Some(Thumbnail {
            path: cached.to_string_lossy().into_owned(),
            thumbhash: None,
        });
    }
    let ext = path.extension()?.to_str()?.to_lowercase();
    let kind = crate::media::decode::sniff(path);
    if !matches!(
        kind,
        crate::media::decode::Kind::Video | crate::media::decode::Kind::Unknown
    ) || (kind == crate::media::decode::Kind::Unknown && PHOTO_EXTS.contains(&ext.as_str()))
    {
        generate_photo_thumb(path, dir)
    } else if VIDEO_EXTS.contains(&ext.as_str()) {
        generate_video_thumb(path, dir)
    } else {
        None
    }
}

/// Cache identity changes with the original's stat and crop coordinates.
pub fn face_cache_path(
    path: &std::path::Path,
    bbox: &str,
    cache: &std::path::Path,
) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    bbox.hash(&mut hash);
    if matches!(
        crate::media::decode::sniff(path),
        crate::media::decode::Kind::Heif
            | crate::media::decode::Kind::Avif
            | crate::media::decode::Kind::Raw
            | crate::media::decode::Kind::Jxl
    ) {
        "oriented-v2"
    } else {
        "oriented-v1"
    }
    .hash(&mut hash);
    if let Ok(meta) = std::fs::metadata(path) {
        meta.len().hash(&mut hash);
        meta.modified().ok().hash(&mut hash);
    }
    cache.join(format!("{:x}.jpg", hash.finish()))
}

#[derive(serde::Serialize)]
pub struct ImageMetadata {
    pub file_name: String,
    pub file_size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub datetime: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
}

/// Generate a thumbnail for a photo and save it to the cache dir.
/// Returns the thumb path on success.
fn generate_photo_thumb(path: &std::path::Path, thumb_dir: &std::path::Path) -> Option<Thumbnail> {
    let thumb_path = thumbnail_cache_path(path, thumb_dir)?;

    // Skip if already cached
    if thumb_path.exists() {
        return Some(Thumbnail {
            path: thumb_path.to_string_lossy().into_owned(),
            thumbhash: None,
        });
    }

    let img = crate::media::decode::open_oriented(path, Some(THUMB_SIZE)).ok()?;
    let hash = thumbhash(&img);
    let temporary = crate::media::cache::temporary(&thumb_path);
    write_webp(&img, &temporary)?;
    std::fs::rename(&temporary, &thumb_path).ok()?;
    Some(Thumbnail {
        path: thumb_path.to_string_lossy().into_owned(),
        thumbhash: Some(hash),
    })
}

/// Grid thumbnails are lossy WebP; `image` only encodes lossless WebP, which is larger than JPEG.
fn write_webp(img: &image::DynamicImage, output: &std::path::Path) -> Option<()> {
    let rgba = image::DynamicImage::ImageRgba8(img.thumbnail(THUMB_SIZE, THUMB_SIZE).to_rgba8());
    let encoded = webp::Encoder::from_image(&rgba).ok()?.encode(THUMB_QUALITY);
    std::fs::write(output, &*encoded).ok()
}

/// Hash pixels before encoding: no second WebP decode on a cold thumbnail.
pub fn thumbhash(image: &image::DynamicImage) -> String {
    use base64::Engine;
    let small = image.thumbnail(100, 100).to_rgba8();
    let hash = thumbhash::rgba_to_thumb_hash(
        small.width() as usize,
        small.height() as usize,
        small.as_raw(),
    );
    base64::engine::general_purpose::STANDARD.encode(hash)
}

/// Generate a video thumbnail by using ffmpeg if available.
/// Falls back to None if ffmpeg not installed.
fn generate_video_thumb(path: &std::path::Path, thumb_dir: &std::path::Path) -> Option<Thumbnail> {
    let thumb_path = thumbnail_cache_path(path, thumb_dir)?;

    if thumb_path.exists() {
        return Some(Thumbnail {
            path: thumb_path.to_string_lossy().into_owned(),
            thumbhash: None,
        });
    }

    let temporary = crate::media::cache::temporary(&thumb_path);
    #[cfg(target_os = "macos")]
    {
        crate::media::decode::native_file(path, &temporary, THUMB_SIZE, Some(0)).ok()?;
        return finish_video_thumb(&temporary, &thumb_path);
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Try ffmpeg to extract frame at 1 second (or 0.5s for short clips)
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-ss",
                "00:00:01",
                "-i",
                &path.to_string_lossy(),
                "-frames:v",
                "1",
                "-vf",
                &format!("scale={THUMB_SIZE}:{THUMB_SIZE}:force_original_aspect_ratio=decrease"),
                "-q:v",
                "3",
                "-y",
                &temporary.to_string_lossy(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;

        if status.success() && temporary.exists() {
            finish_video_thumb(&temporary, &thumb_path)
        } else {
            // Try at 0 seconds for very short videos
            let status2 = std::process::Command::new("ffmpeg")
                .args([
                    "-ss",
                    "00:00:00",
                    "-i",
                    &path.to_string_lossy(),
                    "-frames:v",
                    "1",
                    "-vf",
                    &format!(
                        "scale={THUMB_SIZE}:{THUMB_SIZE}:force_original_aspect_ratio=decrease"
                    ),
                    "-q:v",
                    "3",
                    "-y",
                    &temporary.to_string_lossy(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok()?;

            if status2.success() && temporary.exists() {
                finish_video_thumb(&temporary, &thumb_path)
            } else {
                None
            }
        }
    }
}

/// Video frames arrive as a temporary JPEG; re-encode them into the grid's WebP format.
fn finish_video_thumb(frame: &std::path::Path, thumb_path: &std::path::Path) -> Option<Thumbnail> {
    let decoded = image::open(frame).ok();
    let _ = std::fs::remove_file(frame);
    let decoded = decoded?;
    let hash = thumbhash(&decoded);
    write_webp(&decoded, frame)?;
    std::fs::rename(frame, thumb_path).ok()?;
    Some(Thumbnail {
        path: thumb_path.to_string_lossy().into_owned(),
        thumbhash: Some(hash),
    })
}

pub fn extract_metadata(path: &str) -> Option<ImageMetadata> {
    let p = std::path::Path::new(path);
    let file_name = p.file_name()?.to_string_lossy().to_string();
    let metadata = std::fs::metadata(p).ok()?;
    let file_size = metadata.len();

    let mut width = None;
    let mut height = None;
    let mut datetime = None;
    let mut lat = None;
    let mut lng = None;

    if let Ok(file) = std::fs::File::open(p) {
        let mut bufreader = std::io::BufReader::new(file);
        let exifreader = ExifReader::new();
        if let Ok(exif) = exifreader.read_from_container(&mut bufreader) {
            if let Some(w) = exif.get_field(Tag::PixelXDimension, In::PRIMARY) {
                width = w.value.get_uint(0);
            } else if let Some(w) = exif.get_field(Tag::ImageWidth, In::PRIMARY) {
                width = w.value.get_uint(0);
            }
            if let Some(h) = exif.get_field(Tag::PixelYDimension, In::PRIMARY) {
                height = h.value.get_uint(0);
            } else if let Some(h) = exif.get_field(Tag::ImageLength, In::PRIMARY) {
                height = h.value.get_uint(0);
            }
            if let Some(dt) = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY) {
                datetime = Some(dt.value.display_as(dt.tag).to_string().replace("\"", ""));
            } else if let Some(dt) = exif.get_field(Tag::DateTime, In::PRIMARY) {
                datetime = Some(dt.value.display_as(dt.tag).to_string().replace("\"", ""));
            }
            if let (Some(lat_f), Some(lng_f)) = (
                exif.get_field(Tag::GPSLatitude, In::PRIMARY),
                exif.get_field(Tag::GPSLongitude, In::PRIMARY),
            ) {
                let lat_ref_str = exif
                    .get_field(Tag::GPSLatitudeRef, In::PRIMARY)
                    .map(|r| r.value.display_as(r.tag).to_string())
                    .unwrap_or_default();
                let lng_ref_str = exif
                    .get_field(Tag::GPSLongitudeRef, In::PRIMARY)
                    .map(|r| r.value.display_as(r.tag).to_string())
                    .unwrap_or_default();
                lat = convert_gps(&lat_f.value, &lat_ref_str);
                lng = convert_gps(&lng_f.value, &lng_ref_str);
            }
        }
    }

    Some(ImageMetadata {
        file_name,
        file_size,
        width,
        height,
        datetime,
        lat,
        lng,
    })
}

fn convert_to_dms(mut decimal: f64) -> Vec<uR64> {
    decimal = decimal.abs();
    let degrees = decimal.floor();
    let minutes = ((decimal - degrees) * 60.0).floor();
    let seconds = (decimal - degrees - minutes / 60.0) * 3600.0;
    vec![
        uR64::from(degrees),
        uR64::from(minutes),
        uR64::from(seconds),
    ]
}

pub fn update_metadata(
    path: &str,
    new_date: Option<String>,
    new_lat: Option<f64>,
    new_lng: Option<f64>,
) -> Result<(), String> {
    let p = std::path::Path::new(path);
    let mut metadata = Metadata::new_from_path(p).unwrap_or_else(|_| Metadata::new());
    let mut modified = false;

    if let Some(mut date_str) = new_date {
        date_str = date_str.replace("-", ":").replace("T", " ");
        if date_str.len() == 16 {
            date_str.push_str(":00");
        }
        metadata.set_tag(ExifTag::DateTimeOriginal(date_str.clone()));
        modified = true;
    }

    if let (Some(lat), Some(lng)) = (new_lat, new_lng) {
        let lat_ref = if lat >= 0.0 {
            "N".to_string()
        } else {
            "S".to_string()
        };
        let lng_ref = if lng >= 0.0 {
            "E".to_string()
        } else {
            "W".to_string()
        };
        metadata.set_tag(ExifTag::GPSLatitudeRef(lat_ref));
        metadata.set_tag(ExifTag::GPSLongitudeRef(lng_ref));
        metadata.set_tag(ExifTag::GPSLatitude(convert_to_dms(lat)));
        metadata.set_tag(ExifTag::GPSLongitude(convert_to_dms(lng)));
        modified = true;
    }

    if modified {
        metadata.write_to_file(p).map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub fn parse_single_file(
    path: &std::path::Path,
    kind: crate::media::decode::Kind,
) -> Option<(MediaRecord, crate::media::metadata::Metadata)> {
    let ext = path.extension()?.to_str()?.to_lowercase();

    let is_photo = !matches!(
        kind,
        crate::media::decode::Kind::Video | crate::media::decode::Kind::Unknown
    ) || (kind == crate::media::decode::Kind::Unknown
        && PHOTO_EXTS.contains(&ext.as_str()));
    let is_video = kind == crate::media::decode::Kind::Video
        || (!is_photo && VIDEO_EXTS.contains(&ext.as_str()));

    if !is_photo && !is_video {
        return None;
    }

    let fs_meta = std::fs::metadata(path).ok()?;
    let size = fs_meta.len() as i64;
    let mtime = fs_meta
        .modified()
        .unwrap_or(SystemTime::now())
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let rich = crate::media::metadata::probe(path, is_video);
    let mut record = if is_photo {
        let mut taken_at = Some(mtime);
        let mut city = None;
        let mut country = None;
        let mut lat = None;
        let mut lng = None;
        let mut width = None;
        let mut height = None;
        let mut exif_date = false;

        if let Ok(file) = std::fs::File::open(path) {
            let mut bufreader = std::io::BufReader::new(file);
            let exifreader = ExifReader::new();
            if let Ok(exif) = exifreader.read_from_container(&mut bufreader) {
                if let Some(w) = exif.get_field(Tag::PixelXDimension, In::PRIMARY) {
                    width = w.value.get_uint(0);
                } else if let Some(w) = exif.get_field(Tag::ImageWidth, In::PRIMARY) {
                    width = w.value.get_uint(0);
                }
                if let Some(h) = exif.get_field(Tag::PixelYDimension, In::PRIMARY) {
                    height = h.value.get_uint(0);
                } else if let Some(h) = exif.get_field(Tag::ImageLength, In::PRIMARY) {
                    height = h.value.get_uint(0);
                }

                // Use EXIF date if available
                let dt_field = exif
                    .get_field(Tag::DateTimeOriginal, In::PRIMARY)
                    .or_else(|| exif.get_field(Tag::DateTime, In::PRIMARY));
                exif_date = dt_field.is_some();
                if let Some(dt) = dt_field {
                    let s = dt.value.display_as(dt.tag).to_string().replace("\"", "");
                    // Format: "2024:06:15 14:30:00"
                    if let Ok(parsed) =
                        chrono::NaiveDateTime::parse_from_str(&s, "%Y:%m:%d %H:%M:%S")
                    {
                        taken_at = Some(parsed.and_utc().timestamp());
                    }
                }

                if let (Some(lat_f), Some(lng_f)) = (
                    exif.get_field(Tag::GPSLatitude, In::PRIMARY),
                    exif.get_field(Tag::GPSLongitude, In::PRIMARY),
                ) {
                    let lat_ref_str = exif
                        .get_field(Tag::GPSLatitudeRef, In::PRIMARY)
                        .map(|r| r.value.display_as(r.tag).to_string())
                        .unwrap_or_default();
                    let lng_ref_str = exif
                        .get_field(Tag::GPSLongitudeRef, In::PRIMARY)
                        .map(|r| r.value.display_as(r.tag).to_string())
                        .unwrap_or_default();

                    if let (Some(l), Some(g)) = (
                        convert_gps(&lat_f.value, &lat_ref_str),
                        convert_gps(&lng_f.value, &lng_ref_str),
                    ) {
                        lat = Some(l);
                        lng = Some(g);
                        let record = GEOCODER.search((l, g));
                        city = Some(record.record.name.clone().to_string());
                        country = Some(record.record.cc.clone().to_string());
                    }
                }
            }
        }

        if !exif_date {
            if let Some(Ok(parsed)) = rich.exif_datetime.as_deref().map(|s| {
                chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y:%m:%d %H:%M:%S")
            }) {
                taken_at = Some(parsed.and_utc().timestamp());
            }
        }
        let thumb_path = None; // Preview decoding is lazy and independent of metadata indexing.

        MediaRecord {
            path: path.to_string_lossy().to_string(),
            size,
            mtime,
            taken_at,
            city,
            country,
            lat,
            lng,
            width,
            height,
            media_type: "photo".to_string(),
            thumb_path,
            duration: None,
        }
    } else {
        // Video
        let thumb_path = None;
        let duration = rich.anim_duration_ms.map(|ms| ms / 1000);

        MediaRecord {
            path: path.to_string_lossy().to_string(),
            size,
            mtime,
            taken_at: Some(mtime),
            city: None,
            country: None,
            lat: None,
            lng: None,
            width: None,
            height: None,
            media_type: "video".to_string(),
            thumb_path,
            duration,
        }
    };
    record.width = rich.width.or(record.width);
    record.height = rich.height.or(record.height);
    record.taken_at = rich.taken_at.or(record.taken_at);
    if let (Some(lat), Some(lng)) = (rich.lat, rich.lng) {
        record.lat = Some(lat);
        record.lng = Some(lng);
        let place = GEOCODER.search((lat, lng));
        record.city = Some(place.record.name.to_string());
        record.country = Some(place.record.cc.to_string());
    }
    Some((record, rich))
}

fn convert_gps(val: &Value, ref_str: &str) -> Option<f64> {
    let (d, m, s) = match val {
        Value::Rational(ref arr) if arr.len() >= 3 => {
            (arr[0].to_f64(), arr[1].to_f64(), arr[2].to_f64())
        }
        Value::SRational(ref arr) if arr.len() >= 3 => {
            (arr[0].to_f64(), arr[1].to_f64(), arr[2].to_f64())
        }
        _ => return None,
    };
    let mut decimal = d + (m / 60.0) + (s / 3600.0);
    let upper = ref_str.to_uppercase();
    if upper.contains('S') || upper.contains('W') {
        decimal = -decimal;
    }
    Some(decimal)
}

/// Face coordinates and previews share EXIF-oriented image coordinates.
pub fn open_oriented(
    path: impl AsRef<std::path::Path>,
) -> Result<image::DynamicImage, image::ImageError> {
    crate::media::decode::open_oriented(path.as_ref(), None)
}

/// Locate cached previews without decoding an image or taking the generation lock.
pub fn thumbnail_cache_path(
    path: &std::path::Path,
    thumb_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let stem = path.file_stem()?.to_string_lossy();
    let hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        path.to_string_lossy().hash(&mut h);
        THUMB_VERSION.hash(&mut h);
        let meta = std::fs::metadata(path).ok()?;
        meta.len().hash(&mut h);
        meta.modified().ok()?.hash(&mut h);
        h.finish()
    };
    let suffix = if VIDEO_EXTS.contains(&path.extension()?.to_str()?.to_lowercase().as_str()) {
        "_vid"
    } else {
        ""
    };
    let thumb_name = format!("{stem}_{hash}{suffix}.webp");
    Some(thumb_dir.join(&thumb_name))
}

/// Invalidate only disposable catalog preview metadata when its decode policy changes.
/// Folder records and face input revisions are deliberately untouched.
pub fn prepare_thumbnail_cache(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    use rusqlite::OptionalExtension;
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS preview_cache_version(singleton INTEGER PRIMARY KEY CHECK(singleton=1),version TEXT NOT NULL)")?;
    let previous: Option<String> = tx
        .query_row(
            "SELECT version FROM preview_cache_version WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if previous.as_deref() != Some(THUMB_VERSION) {
        tx.execute("UPDATE photos SET thumbhash=NULL,thumb_path=NULL WHERE thumbhash IS NOT NULL OR thumb_path IS NOT NULL", [])?;
        tx.execute("INSERT INTO preview_cache_version VALUES(1,?1) ON CONFLICT(singleton) DO UPDATE SET version=excluded.version", [THUMB_VERSION])?;
    }
    tx.commit()
}
