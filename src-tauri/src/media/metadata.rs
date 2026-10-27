use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};
#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub media_subtype: String,
    pub content_id: Option<String>,
    pub frame_count: Option<i64>,
    pub anim_duration_ms: Option<i64>,
    pub motion_offset: Option<i64>,
    pub motion_length: Option<i64>,
    pub still_time_ms: Option<i64>,
    pub color_profile: Option<String>,
    pub hdr: Option<bool>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub taken_at: Option<i64>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    /// ImageIO's EXIF DateTimeOriginal; covers RAW containers kamadak-exif cannot read.
    pub exif_datetime: Option<String>,
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn gphotos_metadata(path: *const std::ffi::c_char, video: i32) -> *mut std::ffi::c_char;
    fn gphotos_free(value: *mut std::ffi::c_char);
    fn gphotos_volume(path: *const std::ffi::c_char, available: *mut i64, total: *mut i64) -> i32;
    fn gphotos_details(path: *const std::ffi::c_char, video: i32) -> *mut std::ffi::c_char;
}
/// Camera, lens, exposure and codec details for the info panel; absent keys are unknown.
pub fn details(path: &Path, video: bool) -> serde_json::Value {
    #[cfg(target_os = "macos")]
    if let Ok(path) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
        unsafe {
            let ptr = gphotos_details(path.as_ptr(), video as i32);
            if !ptr.is_null() {
                let value = serde_json::from_slice(std::ffi::CStr::from_ptr(ptr).to_bytes())
                    .unwrap_or_default();
                gphotos_free(ptr);
                return value;
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (path, video);
    serde_json::json!({})
}
/// (available, total) bytes of the volume holding `path`; macOS only.
pub fn volume(path: &Path) -> Option<(u64, u64)> {
    #[cfg(target_os = "macos")]
    {
        let path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok()?;
        let (mut available, mut total) = (0i64, 0i64);
        if unsafe { gphotos_volume(path.as_ptr(), &mut available, &mut total) } == 1 {
            return Some((available.max(0) as u64, total.max(0) as u64));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
    None
}
pub fn probe(path: &Path, video: bool) -> Metadata {
    let mut meta = Metadata::default();
    #[cfg(target_os = "macos")]
    if let Ok(path) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
        unsafe {
            let ptr = gphotos_metadata(path.as_ptr(), video as i32);
            if !ptr.is_null() {
                meta = serde_json::from_slice(std::ffi::CStr::from_ptr(ptr).to_bytes())
                    .unwrap_or_default();
                gphotos_free(ptr);
            }
        }
    }
    meta.media_subtype = if video {
        "video"
    } else if meta.frame_count.unwrap_or(1) > 1 {
        "animated"
    } else {
        "still"
    }
    .into();
    if !video {
        // XMP is bounded; never load the appended video into memory to index a photo.
        if let Ok(mut f) = std::fs::File::open(path) {
            let size = f.metadata().map(|m| m.len()).unwrap_or(0);
            let mut header = vec![0; size.min(1024 * 1024) as usize];
            if let Some((offset, length)) = samsung_range(&mut f, size) {
                meta.motion_offset = Some(offset as i64);
                meta.motion_length = Some(length as i64);
                meta.media_subtype = "motion_embedded".into();
            }
            let _ = f.seek(SeekFrom::Start(0));
            if f.read_exact(&mut header).is_ok() {
                let text = String::from_utf8_lossy(&header);
                let length = attribute(&text, "MicroVideoOffset").or_else(|| {
                    let pos = text
                        .find("Semantic=\"MotionPhoto\"")
                        .or_else(|| text.find("Semantic='MotionPhoto'"))?;
                    let start = text[..pos].rfind('<')?;
                    let end = pos + text[pos..].find('>')?;
                    attribute(&text[start..end], "Length")
                });
                if let Some(length) = length.filter(|n| *n > 12 && (*n as u64) < size) {
                    let offset = size - length as u64;
                    let mut magic = [0u8; 12];
                    if f.seek(SeekFrom::Start(offset)).is_ok()
                        && f.read_exact(&mut magic).is_ok()
                        && &magic[4..8] == b"ftyp"
                    {
                        meta.motion_offset = Some(offset as i64);
                        meta.motion_length = Some(length);
                        meta.media_subtype = "motion_embedded".into();
                        meta.still_time_ms = attribute(&text, "MotionPhotoPresentationTimestampUs")
                            .filter(|t| *t >= 0)
                            .map(|t| t / 1000);
                    }
                }
            }
        }
    }
    meta
}
fn attribute(text: &str, key: &str) -> Option<i64> {
    let after = &text[text.find(&format!("{key}="))? + key.len() + 1..];
    let quote = after.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    after[1..].split(quote).next()?.parse().ok()
}

pub fn pair(conn: &rusqlite::Connection) -> Result<(), String> {
    let _span = super::perf::Span::new("pair");
    use rusqlite::params;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    // Only mutate changed pairs; unchanged reconciles must not invalidate the projection.
    tx.execute_batch("UPDATE photos SET paired_id=NULL,pair_role=NULL,media_subtype=CASE WHEN media_type='video' THEN 'video' WHEN motion_offset IS NOT NULL THEN 'motion_embedded' WHEN frame_count>1 THEN 'animated' ELSE 'still' END
        WHERE paired_id IS NOT NULL AND (available=0 OR excluded=1 OR NOT EXISTS(SELECT 1 FROM photos q WHERE q.id=photos.paired_id AND q.available=1 AND q.excluded=0 AND q.paired_id=photos.id AND (photos.content_id IS NULL OR q.content_id IS NULL OR photos.content_id=q.content_id)));").map_err(|e|e.to_string())?;
    let rows:Vec<(i64,String,String,Option<String>,i64,Option<i64>)> = tx.prepare("SELECT id,path,media_type,content_id,COALESCE(taken_at,mtime),anim_duration_ms FROM photos WHERE available=1 AND excluded=0 AND paired_id IS NULL AND media_subtype IN ('still','video') ORDER BY id").map_err(|e|e.to_string())?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<_>>().map_err(|e|e.to_string())?;
    let mut used = std::collections::HashSet::new();
    for strong in [true, false] {
        for still in rows.iter().filter(|r| r.2 == "photo") {
            if used.contains(&still.0) {
                continue;
            }
            let matches: Vec<_> = rows
                .iter()
                .filter(|m| {
                    m.2 == "video"
                        && !used.contains(&m.0)
                        && Path::new(&m.1)
                            .extension()
                            .map(|s| s.eq_ignore_ascii_case("mov"))
                            .unwrap_or(false)
                })
                .filter(|m| {
                    if strong {
                        still.3.as_ref().filter(|s| !s.is_empty()).is_some() && still.3 == m.3
                    } else {
                        (still.3.is_none() || m.3.is_none())
                            && m.5.map(|d| d > 0 && d <= 4000).unwrap_or(false)
                            && (still.4 - m.4).abs() <= 3
                            && Path::new(&still.1)
                                .file_stem()
                                .unwrap_or_default()
                                .eq_ignore_ascii_case(
                                    Path::new(&m.1).file_stem().unwrap_or_default(),
                                )
                    }
                })
                .collect();
            if matches.len() != 1 {
                continue;
            }
            let motion = matches[0];
            if !strong
                && rows
                    .iter()
                    .filter(|r| {
                        r.2 == "photo"
                            && Path::new(&r.1)
                                .file_stem()
                                .unwrap_or_default()
                                .eq_ignore_ascii_case(
                                    Path::new(&motion.1).file_stem().unwrap_or_default(),
                                )
                            && (r.4 - motion.4).abs() <= 3
                    })
                    .count()
                    != 1
            {
                continue;
            }

            // Ambiguous content IDs are never paired by enumeration order.
            if strong
                && rows
                    .iter()
                    .filter(|r| r.2 == "photo" && r.3 == still.3)
                    .count()
                    != 1
            {
                continue;
            }
            tx.execute("UPDATE photos SET paired_id=?1,pair_role='still',media_subtype='live',anim_duration_ms=?2 WHERE id=?3",params![motion.0,motion.5,still.0]).map_err(|e|e.to_string())?;
            tx.execute(
                "UPDATE photos SET paired_id=?1,pair_role='motion' WHERE id=?2",
                params![still.0, motion.0],
            )
            .map_err(|e| e.to_string())?;
            used.insert(still.0);
            used.insert(motion.0);
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

// SEF footer/directory layout: AndroidX Media3 SefReader. MotionPhoto payload/mpv2
// layout: ExifTool's Samsung.pm. All offsets are validated against the file size.
fn samsung_range(file: &mut std::fs::File, size: u64) -> Option<(u64, u64)> {
    let mut footer = [0u8; 8];
    file.seek(SeekFrom::Start(size.checked_sub(8)?)).ok()?;
    file.read_exact(&mut footer).ok()?;
    if &footer[4..] != b"SEFT" {
        return None;
    }
    let length = u32::from_le_bytes(footer[..4].try_into().ok()?) as u64 + 8;
    if !(20..=65536).contains(&length) {
        return None;
    }
    let base = size.checked_sub(length)?;
    file.seek(SeekFrom::Start(base)).ok()?;
    let mut directory = vec![0u8; length as usize];
    file.read_exact(&mut directory).ok()?;
    if &directory[..4] != b"SEFH" {
        return None;
    }
    for entry in directory[12..directory.len() - 8].chunks_exact(12) {
        let back = u32::from_le_bytes(entry[4..8].try_into().ok()?) as u64;
        let bytes = u32::from_le_bytes(entry[8..12].try_into().ok()?) as u64;
        let Some(start) = base.checked_sub(back) else {
            continue;
        };
        if bytes < 12 || start.checked_add(bytes)? > base {
            continue;
        }
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let name_len = u32::from_le_bytes(header[4..8].try_into().ok()?) as u64;
        if name_len != 16 || bytes < 8 + name_len + 12 {
            continue;
        }
        let mut name = [0u8; 16];
        file.read_exact(&mut name).ok()?;
        if &name != b"MotionPhoto_Data" {
            continue;
        }
        let mut payload = [0u8; 12];
        file.read_exact(&mut payload).ok()?;
        let (offset, len) = if &payload[..4] == b"mpv2" {
            (
                u32::from_be_bytes(payload[4..8].try_into().ok()?) as u64,
                u32::from_be_bytes(payload[8..12].try_into().ok()?) as u64,
            )
        } else {
            (start + 8 + name_len, bytes - 8 - name_len)
        };
        if len < 12 || offset.checked_add(len)? > size {
            continue;
        }
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut payload).ok()?;
        if &payload[4..8] == b"ftyp" {
            return Some((offset, len));
        }
    }
    None
}
