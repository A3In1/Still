use rusqlite::{Connection, OptionalExtension};
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

pub mod db;
pub mod face_worker;
pub mod indexing;
pub mod media;
pub mod people;
pub mod scanner;
pub mod updates;

pub struct AppState {
    pub db_conn: Mutex<Option<Connection>>,
    pub indexer: indexing::Indexer,
    pub face_service: face_worker::Service,
    pub watchers: Mutex<std::collections::HashMap<String, IndexWatcher>>,
    pub thumbnail_gate: std::sync::Arc<media::cache::Pool>,
}

pub struct IndexWatcher {
    _watcher: Option<notify::RecommendedWatcher>,
    stop: Arc<AtomicBool>,
}
impl Drop for IndexWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[derive(serde::Serialize)]
pub struct PhotoRecord {
    pub id: i32,
    pub path: String,
    pub mtime: i64,
    pub city: Option<String>,
    pub country: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: Option<i64>,
    pub media_type: String,
    pub thumb_path: Option<String>,
    pub duration: Option<i64>,
    pub source_revision: String,
    pub media_subtype: String,
    pub pair_role: Option<String>,
    pub paired_id: Option<i64>,
    pub content_id: Option<String>,
    pub still_time_ms: Option<i64>,
    pub motion_offset: Option<i64>,
    pub motion_length: Option<i64>,
    pub frame_count: Option<i64>,
    pub anim_duration_ms: Option<i64>,
    pub live_effect: String,
    pub key_time_ms: Option<i64>,
    pub color_profile: Option<String>,
    pub hdr: i64,
    pub thumbhash: Option<String>,
}

use notify::{RecursiveMode, Watcher};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[tauri::command]
fn get_indexed_folders(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state.indexer.roots()
}

#[tauri::command]
fn get_photo_count(state: State<'_, AppState>) -> Result<i64, String> {
    let guard = state.db_conn.lock().unwrap();
    guard
        .as_ref()
        .ok_or("Database unavailable")?
        .query_row(
            "SELECT COUNT(*) FROM photos WHERE available=1 AND excluded=0 AND COALESCE(pair_role,'')<>'motion'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
}

fn reconcile(app: &tauri::AppHandle, path: &std::path::Path) -> indexing::ScanReport {
    let state = app.state::<AppState>();
    let mut projected = 0;
    let report = state.indexer.scan(path, &state.db_conn, |r| {
        if r.folders_refreshed > projected {
            projected = r.folders_refreshed;
            let _ = app.emit("index_updated", r);
            media::warm::schedule(app);
        }
        let _ = app.emit(
            "scan_progress",
            serde_json::json!({"done":r.checked,"total":0}),
        );
    });
    let _ = app.emit(
        "index_status",
        serde_json::json!({"path":path.to_string_lossy(),"errors":report.errors}),
    );
    if report.folders_refreshed > 0 || report.missing > 0 || !report.errors.is_empty() {
        let _ = app.emit("index_updated", &report);
    }
    face_worker::schedule(app);
    media::warm::schedule(app);
    report
}

#[tauri::command]
async fn start_scan(path: String, app: tauri::AppHandle) -> Result<indexing::ScanReport, String> {
    let root = app.state::<AppState>().indexer.register(&path)?;
    start_watcher(root.to_string_lossy().into_owned(), app.clone())?;
    tauri::async_runtime::spawn_blocking(move || reconcile(&app, &root))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn start_watcher(path: String, app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let root = match state.indexer.register(&path) {
        Ok(root) => root,
        Err(_e) if state.indexer.roots()?.contains(&path) => std::path::PathBuf::from(&path),
        Err(e) => return Err(e),
    };
    let key = root.to_string_lossy().to_string();
    let mut watchers = state.watchers.lock().unwrap();
    if watchers.contains_key(&key) {
        return Ok(());
    }
    let dirty = Arc::new(AtomicBool::new(false));
    let flag = dirty.clone();
    let watcher_result =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(e)
                if !e.kind.is_access()
                    && e.paths
                        .iter()
                        .any(|p| !p.components().any(|c| c.as_os_str() == ".data")) =>
            {
                flag.store(true, Ordering::Release);
            }
            Err(_) => {
                flag.store(true, Ordering::Release);
            }
            _ => {}
        })
        .and_then(|mut watcher| {
            watcher.watch(&root, RecursiveMode::Recursive)?;
            Ok(watcher)
        });
    let watcher = match watcher_result {
        Ok(watcher) => Some(watcher),
        Err(e) => {
            let _=app.emit("index_updated",serde_json::json!({"errors":[format!("{}: live watching unavailable ({e}); checking every minute",root.display())]}));
            None
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    watchers.insert(
        key,
        IndexWatcher {
            _watcher: watcher,
            stop: stop.clone(),
        },
    );
    // Coalesce arbitrary event bursts into bounded state; never queue one task per file event.
    let handle = app.clone();
    std::thread::spawn(move || {
        let mut ticks = 0;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if stop.load(Ordering::Acquire) {
                break;
            }
            ticks += 1;
            if dirty.swap(false, Ordering::AcqRel) || ticks >= 30 {
                ticks = 0;
                reconcile(&handle, &root);
            }
        }
    });
    Ok(())
}

#[tauri::command]
async fn initialize_indexing(
    legacy_path: Option<String>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let mut paths = app.state::<AppState>().indexer.roots()?;
    if paths.is_empty() {
        if let Some(p) = legacy_path {
            paths.push(p);
        }
    }
    // Watch before reconciling, so writes during startup are not missed.
    for path in paths {
        if let Err(e) = start_watcher(path.clone(), app.clone()) {
            let _ = app.emit(
                "index_updated",
                serde_json::json!({"errors":[format!("{}: {}",path,e)]}),
            );
        }
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            reconcile(&handle, std::path::Path::new(&path))
        });
    }
    Ok(())
}

fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    r * c
}

const PHOTO_COLUMNS: &str = "id, path, COALESCE(taken_at, mtime), city, country, lat, lng, width, height, size+COALESCE((SELECT partner.size FROM photos partner WHERE partner.id=photos.paired_id AND photos.pair_role='still'),0), COALESCE(media_type,'photo'), thumb_path, duration, COALESCE(fs_mtime_ns,'')||COALESCE((SELECT ':'||partner.fs_mtime_ns FROM photos partner WHERE partner.id=photos.paired_id AND photos.pair_role='still'),''),media_subtype,pair_role,paired_id,content_id,still_time_ms,motion_offset,motion_length,frame_count,anim_duration_ms,live_effect,key_time_ms,color_profile,hdr,thumbhash";
const VISIBLE: &str = "available=1 AND excluded=0 AND COALESCE(pair_role,'')<>'motion'";
const TIMELINE: &str = "ORDER BY COALESCE(taken_at, mtime) DESC, id DESC";

fn photo_row(row: &rusqlite::Row) -> rusqlite::Result<PhotoRecord> {
    Ok(PhotoRecord {
        id: row.get(0)?,
        path: row.get(1)?,
        mtime: row.get(2)?,
        city: row.get(3)?,
        country: row.get(4)?,
        lat: row.get(5)?,
        lng: row.get(6)?,
        width: row.get(7)?,
        height: row.get(8)?,
        size: row.get(9)?,
        media_type: row.get(10)?,
        thumb_path: row.get(11)?,
        duration: row.get(12)?,
        source_revision: row.get(13)?,
        media_subtype: row.get(14)?,
        pair_role: row.get(15)?,
        paired_id: row.get(16)?,
        content_id: row.get(17)?,
        still_time_ms: row.get(18)?,
        motion_offset: row.get(19)?,
        motion_length: row.get(20)?,
        frame_count: row.get(21)?,
        anim_duration_ms: row.get(22)?,
        live_effect: row.get(23)?,
        key_time_ms: row.get(24)?,
        color_profile: row.get(25)?,
        hdr: row.get(26)?,
        thumbhash: row.get(27)?,
    })
}

fn photos_where(
    conn: &Connection,
    filter: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<PhotoRecord>, String> {
    conn.prepare(&format!("SELECT {PHOTO_COLUMNS} FROM photos WHERE {filter}"))
        .map_err(|e| e.to_string())?
        .query_map(params, photo_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
}

/// Lowercased word tokens, split like FTS5's unicode61 tokenizer.
fn search_tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Every token must prefix-match some word (AND); tokens are alphanumeric, so quoting is safe.
fn fts_match(tokens: &[String], column: Option<&str>) -> String {
    let terms = tokens
        .iter()
        .map(|t| format!("\"{t}\"*"))
        .collect::<Vec<_>>()
        .join(" ");
    match column {
        Some(c) => format!("{c} : ({terms})"),
        None => terms,
    }
}

#[tauri::command]
fn get_photos(
    offset: usize,
    limit: usize,
    query: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<PhotoRecord>, String> {
    let guard = state.db_conn.lock().unwrap();
    let conn = guard.as_ref().ok_or("Database not initialized")?;
    let query_str = query.unwrap_or_default().trim().to_lowercase();
    let tokens = search_tokens(&query_str);
    if tokens.is_empty() {
        return photos_where(
            conn,
            &format!("{VISIBLE} {TIMELINE} LIMIT ?1 OFFSET ?2"),
            rusqlite::params![limit as i64, offset as i64],
        );
    }
    let fts = fts_match(&tokens, None);
    // A query naming a city also matches photos within 50 km of it.
    let center: Option<(f64, f64)> = if query_str.chars().count() > 2 {
        conn.query_row(
            "SELECT lat, lng FROM photos WHERE id IN (SELECT rowid FROM photo_search WHERE photo_search MATCH ?1)
            AND LOWER(city)=?2 AND lat IS NOT NULL AND lng IS NOT NULL LIMIT 1",
            rusqlite::params![format!("city : \"{}\"", tokens.join(" ")), query_str],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
    } else {
        None
    };
    let text = "id IN (SELECT rowid FROM photo_search WHERE photo_search MATCH ?1)";
    let Some((lat, lng)) = center else {
        return photos_where(
            conn,
            &format!("{VISIBLE} AND {text} {TIMELINE} LIMIT ?2 OFFSET ?3"),
            rusqlite::params![fts, limit as i64, offset as i64],
        );
    };
    // Bounding-box prefilter in SQL; the exact radius check runs only on those candidates.
    let dlat = 50.0 / 110.574 + 0.01;
    let cos = lat.to_radians().cos();
    let dlng = if cos > 0.01 { 50.0 / (111.320 * cos) + 0.01 } else { 360.0 };
    let (west, east) = if lng - dlng < -180.0 || lng + dlng > 180.0 {
        (-180.0, 180.0)
    } else {
        (lng - dlng, lng + dlng)
    };
    let mut stmt = conn
        .prepare(&format!("SELECT id, lat, lng, {text} FROM photos WHERE {VISIBLE}
            AND ({text} OR (lat BETWEEN ?2 AND ?3 AND lng BETWEEN ?4 AND ?5)) {TIMELINE}"))
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map(
            rusqlite::params![fts, lat - dlat, lat + dlat, west, east],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<f64>>(1)?,
                    r.get::<_, Option<f64>>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .filter_map(|r| match r {
            Ok((id, Some(a), Some(b), text)) if text || haversine(lat, lng, a, b) < 50.0 => {
                Some(Ok(id))
            }
            Ok((id, _, _, true)) => Some(Ok(id)),
            Ok(_) => None,
            Err(e) => Some(Err(e.to_string())),
        })
        .skip(offset)
        .take(limit)
        .collect::<Result<Vec<i64>, String>>()?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    photos_where(conn, &format!("id IN ({list}) {TIMELINE}"), [])
}

#[derive(serde::Deserialize)]
pub struct TimelineCursor {
    pub ts: i64,
    pub id: i64,
}

/// Keyset page in timeline order; `cursor` is the last photo of the previous page.
#[tauri::command]
fn get_photos_after(
    cursor: Option<TimelineCursor>,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<PhotoRecord>, String> {
    let guard = state.db_conn.lock().unwrap();
    let conn = guard.as_ref().ok_or("Database not initialized")?;
    match cursor {
        // Written so the photos_timeline index seeks to the cursor instead of scanning to it.
        Some(c) => photos_where(
            conn,
            &format!("{VISIBLE} AND COALESCE(taken_at, mtime)<=?1 AND (COALESCE(taken_at, mtime)<?1 OR id<?2) {TIMELINE} LIMIT ?3"),
            rusqlite::params![c.ts, c.id, limit as i64],
        ),
        None => photos_where(
            conn,
            &format!("{VISIBLE} {TIMELINE} LIMIT ?1"),
            [limit as i64],
        ),
    }
}

#[derive(serde::Serialize)]
pub struct TimelineDay {
    pub date_key: String,
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub count: i64,
    pub place: Option<String>,
    pub first_ts: i64,
}

#[tauri::command]
fn get_timeline_summary(state: State<'_, AppState>) -> Result<Vec<TimelineDay>, String> {
    let guard = state.db_conn.lock().unwrap();
    let conn = guard.as_ref().ok_or("Database not initialized")?;
    let mut stmt = conn
        .prepare(&format!(
            "WITH v AS (SELECT date(COALESCE(taken_at,mtime),'unixepoch','localtime') k, COALESCE(taken_at,mtime) ts,
                CASE WHEN COALESCE(city,'')<>'' THEN city||COALESCE(', '||NULLIF(country,''),'') END place FROM photos WHERE {VISIBLE}),
            days AS (SELECT k, COUNT(*) n, MAX(ts) first_ts FROM v GROUP BY k),
            places AS (SELECT k, place, ROW_NUMBER() OVER (PARTITION BY k ORDER BY COUNT(*) DESC, place) r
                FROM v WHERE place IS NOT NULL GROUP BY k, place)
            SELECT days.k, days.n, days.first_ts, places.place FROM days LEFT JOIN places ON places.k=days.k AND places.r=1
            WHERE days.k IS NOT NULL ORDER BY days.k DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut days = Vec::new();
    for row in rows {
        let (key, count, first_ts, place) = row.map_err(|e| e.to_string())?;
        let mut parts = key.split('-').map(|p| p.parse::<u32>().unwrap_or(0));
        let (year, month, day) = (
            parts.next().unwrap_or(0) as i32,
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        );
        days.push(TimelineDay {
            date_key: key,
            year,
            month,
            day,
            count,
            place,
            first_ts,
        });
    }
    Ok(days)
}

#[derive(serde::Serialize)]
pub struct LibraryStats {
    pub photos: i64,
    pub videos: i64,
    pub library_bytes: i64,
    /// On-disk size of the registered folders, excluding their `.data` index directories.
    pub folder_bytes: u64,
    pub cache_bytes: u64,
    pub disk_free_bytes: Option<u64>,
    pub disk_total_bytes: Option<u64>,
}

fn directory_bytes(root: &std::path::Path, skip_index: bool) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => {
                    if !(skip_index && entry.file_name() == ".data") {
                        stack.push(entry.path())
                    }
                }
                Ok(t) if t.is_file() => {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0)
                }
                _ => {}
            }
        }
    }
    total
}

#[tauri::command]
async fn get_library_stats(app: tauri::AppHandle) -> Result<LibraryStats, String> {
    // The cache walk touches every thumbnail; reuse a recent total.
    static CACHE_BYTES: Mutex<Option<(std::time::Instant, u64)>> = Mutex::new(None);
    static FOLDER_BYTES: Mutex<Option<(std::time::Instant, Vec<String>, u64)>> = Mutex::new(None);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (photos, videos, library_bytes): (i64, i64, i64) = {
            let guard = state.db_conn.lock().unwrap();
            let conn = guard.as_ref().ok_or("Database not initialized")?;
            conn.query_row(
                &format!("SELECT COUNT(*) FILTER (WHERE {VISIBLE} AND COALESCE(media_type,'photo')<>'video'),
                    COUNT(*) FILTER (WHERE {VISIBLE} AND media_type='video'),
                    COALESCE(SUM(size) FILTER (WHERE available=1 AND excluded=0),0) FROM photos"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| e.to_string())?
        };
        let cached = *CACHE_BYTES.lock().unwrap();
        let cache_bytes = match cached {
            Some((at, bytes)) if at.elapsed() < std::time::Duration::from_secs(60) => bytes,
            _ => {
                let bytes = directory_bytes(&state.indexer.cache_dir, false);
                *CACHE_BYTES.lock().unwrap() = Some((std::time::Instant::now(), bytes));
                bytes
            }
        };
        let roots = state.indexer.roots().unwrap_or_default();
        let cached = FOLDER_BYTES.lock().unwrap().clone();
        let folder_bytes = match cached {
            Some((at, r, bytes)) if r == roots && at.elapsed() < std::time::Duration::from_secs(60) => bytes,
            _ => {
                let bytes = roots
                    .iter()
                    .map(|r| directory_bytes(std::path::Path::new(r), true))
                    .sum();
                *FOLDER_BYTES.lock().unwrap() = Some((std::time::Instant::now(), roots, bytes));
                bytes
            }
        };
        let volume = state
            .indexer
            .roots()
            .ok()
            .and_then(|r| r.into_iter().next())
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| state.indexer.cache_dir.clone());
        let disk = media::metadata::volume(&volume);
        Ok(LibraryStats {
            photos,
            videos,
            library_bytes,
            folder_bytes,
            cache_bytes,
            disk_free_bytes: disk.map(|d| d.0),
            disk_total_bytes: disk.map(|d| d.1),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
pub struct PersonSuggestion {
    pub id: i64,
    pub name: String,
    pub photo_count: i64,
}
#[derive(serde::Serialize)]
pub struct PlaceSuggestion {
    pub name: String,
    pub count: i64,
}
#[derive(serde::Serialize)]
pub struct DateSuggestion {
    pub label: String,
    pub year: i32,
    pub month: Option<u32>,
    pub count: i64,
}
#[derive(serde::Serialize)]
pub struct FolderSuggestion {
    pub path: String,
    pub name: String,
    pub count: i64,
}
#[derive(serde::Serialize, Default)]
pub struct SearchSuggestions {
    pub people: Vec<PersonSuggestion>,
    pub places: Vec<PlaceSuggestion>,
    pub dates: Vec<DateSuggestion>,
    pub folders: Vec<FolderSuggestion>,
}

#[tauri::command]
fn search_suggest(query: String, state: State<'_, AppState>) -> Result<SearchSuggestions, String> {
    let guard = state.db_conn.lock().unwrap();
    let conn = guard.as_ref().ok_or("Database not initialized")?;
    let tokens = search_tokens(&query);
    let mut out = SearchSuggestions::default();
    if tokens.is_empty() {
        return Ok(out);
    }
    let e = |e: rusqlite::Error| e.to_string();
    let words = tokens.join(" ");
    let like = words.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    out.people = conn
        .prepare("SELECT p.id, p.name, COUNT(DISTINCT f.photo_id) n FROM people p
            JOIN faces f ON f.person_id=p.id AND f.retired=0
            JOIN photos ph ON ph.id=f.photo_id AND ph.available=1 AND ph.excluded=0
            WHERE p.user_named=1 AND (p.name LIKE ?1 ESCAPE '\\' OR p.name LIKE ?2 ESCAPE '\\')
            GROUP BY p.id ORDER BY n DESC, p.name LIMIT 5")
        .map_err(e)?
        .query_map(rusqlite::params![format!("{like}%"), format!("% {like}%")], |r| {
            Ok(PersonSuggestion { id: r.get(0)?, name: r.get(1)?, photo_count: r.get(2)? })
        })
        .map_err(e)?
        .collect::<rusqlite::Result<_>>()
        .map_err(e)?;
    let candidates_for = |tokens: &[String], column: &str| {
        format!("{VISIBLE} AND id IN (SELECT rowid FROM photo_search WHERE photo_search MATCH '{}')", fts_match(tokens, Some(column)))
    };
    let candidates = |column: &str| candidates_for(&tokens, column);
    out.places = conn
        .prepare(&format!("SELECT city, COUNT(*) n FROM photos WHERE {} AND COALESCE(city,'')<>''
            GROUP BY city ORDER BY n DESC, city LIMIT 5", candidates("city")))
        .map_err(e)?
        .query_map([], |r| Ok(PlaceSuggestion { name: r.get(0)?, count: r.get(1)? }))
        .map_err(e)?
        .collect::<rusqlite::Result<_>>()
        .map_err(e)?;
    out.folders = conn
        .prepare(&format!("SELECT folder_path, COUNT(*) n FROM photos WHERE {} AND folder_path IS NOT NULL
            GROUP BY folder_path ORDER BY n DESC, folder_path LIMIT 5", candidates("folder")))
        .map_err(e)?
        .query_map([], |r| {
            let path: String = r.get(0)?;
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            Ok(FolderSuggestion { path, name, count: r.get(1)? })
        })
        .map_err(e)?
        .collect::<rusqlite::Result<_>>()
        .map_err(e)?;
    // Dates: a month-name prefix (3+ letters) and/or a year (prefix of up to 4 digits).
    let month = tokens.iter().find_map(|t| {
        (t.len() >= 3)
            .then(|| indexing::MONTHS.iter().position(|m| m.starts_with(t.as_str())))
            .flatten()
            .map(|i| i as u32 + 1)
    });
    let year = tokens
        .iter()
        .find(|t| t.len() <= 4 && t.chars().all(|c| c.is_ascii_digit()))
        .cloned();
    if month.is_some() || year.is_some() {
        let ts = "COALESCE(taken_at,mtime),'unixepoch','localtime'";
        // date_words holds "march mar 2024"; it narrows the rows before strftime runs.
        let terms: Vec<String> = month
            .map(|m| indexing::MONTHS[m as usize - 1].to_string())
            .into_iter()
            .chain(year.clone())
            .collect();
        let mut stmt = conn
            .prepare(&format!("SELECT CAST(y AS INTEGER), CAST(m AS INTEGER), COUNT(*) n FROM
                (SELECT strftime('%Y',{ts}) y, {} m FROM photos WHERE {})
                WHERE y LIKE ?1 AND (?2 IS NULL OR CAST(m AS INTEGER)=?2)
                GROUP BY y, m ORDER BY y DESC, m DESC LIMIT 5",
                if month.is_some() { format!("strftime('%m',{ts})") } else { "NULL".into() },
                candidates_for(&terms, "date_words")))
            .map_err(e)?;
        out.dates = stmt
            .query_map(
                rusqlite::params![format!("{}%", year.as_deref().unwrap_or("")), month],
                |r| {
                    let year: i32 = r.get(0)?;
                    let month: Option<u32> = r.get(1)?;
                    let label = match month {
                        Some(m) => {
                            let name = indexing::MONTHS[(m.clamp(1, 12) - 1) as usize];
                            format!("{}{} {year}", name[..1].to_uppercase(), &name[1..])
                        }
                        None => year.to_string(),
                    };
                    Ok(DateSuggestion { label, year, month, count: r.get(2)? })
                },
            )
            .map_err(e)?
            .collect::<rusqlite::Result<_>>()
            .map_err(e)?;
    }
    Ok(out)
}

/// Cache hits bypass both the catalog and the expensive image-generation queue.
#[tauri::command]
async fn get_thumbnail(path: String, state: State<'_, AppState>) -> Result<String, String> {
    let gate = state.thumbnail_gate.clone();
    let cache_dir = state.indexer.cache_dir.join(scanner::THUMB_DIR);
    let source_path = path.clone();
    let (thumbnail, revision) = tauri::async_runtime::spawn_blocking(move || {
        let _span = media::perf::Span::new("thumb");
        let source = std::path::Path::new(&path);
        let cached = scanner::thumbnail_cache_path(source, &cache_dir)
            .ok_or("Preview source unavailable")?;
        if cached.is_file() {
            return Ok::<_, String>((
                scanner::Thumbnail {
                    path: cached.to_string_lossy().into_owned(),
                    thumbhash: None,
                },
                None,
            ));
        }
        let _permit = gate.acquire();
        let before = indexing::stat(source)?;
        let thumbnail = scanner::thumbnail(source, &cache_dir)
            .ok_or("Preview unavailable for this file format")?;
        if indexing::stat(source)? != before {
            return Err("Preview source changed while decoding".into());
        }
        Ok((thumbnail, Some(before)))
    })
    .await
    .map_err(|e| e.to_string())??;
    // Reuse the hash computed before encoding. Cache hits retain their lock-free path.
    if let (Some(hash), Some((mtime, size, _))) = (&thumbnail.thumbhash, revision) {
        if let Some(conn) = state.db_conn.lock().unwrap().as_ref() {
            conn.execute(
                "UPDATE photos SET thumbhash=?1 WHERE path=?2 AND fs_mtime_ns=?3 AND size=?4",
                rusqlite::params![hash, source_path, mtime, size],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(thumbnail.path)
}

/// Returns a file:// URI for the original file (used for full-res lightbox and video playback).
#[tauri::command]
fn get_file_uri(path: String) -> Result<String, String> {
    Ok(format!("asset://localhost/{}", urlencoding::encode(&path)))
}

#[tauri::command]
async fn get_face_thumbnail(
    path: String,
    bbox: String,
    frame_ms: Option<i64>,
    frame_source: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let cache = state.indexer.cache_dir.join("face-crops");
    let gate = state.thumbnail_gate.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let path = if let Some(ms) = frame_ms {
            media::video::source_frame(
                &app.state::<AppState>(),
                std::path::Path::new(&path),
                ms,
                frame_source.as_deref().unwrap_or("animated"),
            )?
            .to_string_lossy()
            .into_owned()
        } else {
            path
        };
        let cached = scanner::face_cache_path(std::path::Path::new(&path), &bbox, &cache);
        if !cached.is_file() {
            let _permit = gate.acquire();
            if !cached.is_file() {
                let mut coords: Vec<f32> =
                    serde_json::from_str(&bbox).map_err(|e| e.to_string())?;
                if coords.len() != 4 || coords.iter().any(|c| !c.is_finite()) {
                    return Err("Invalid face bounds".into());
                }
                let source = std::path::Path::new(&path);
                let heif = matches!(
                    media::decode::sniff(source),
                    media::decode::Kind::Heif
                        | media::decode::Kind::Avif
                        | media::decode::Kind::Raw
                        | media::decode::Kind::Jxl
                );
                let img =
                    media::decode::open_oriented(source, if heif { Some(4096) } else { None })
                        .map_err(|e| e.to_string())?;
                if heif {
                    let metadata = media::metadata::probe(source, false);
                    if let (Some(w), Some(h)) = (metadata.width, metadata.height) {
                        if w > 0 && h > 0 {
                            coords[0] *= img.width() as f32 / w as f32;
                            coords[2] *= img.width() as f32 / w as f32;
                            coords[1] *= img.height() as f32 / h as f32;
                            coords[3] *= img.height() as f32 / h as f32;
                        }
                    }
                }
                let x = coords[0].max(0.0).min(img.width() as f32) as u32;
                let y = coords[1].max(0.0).min(img.height() as f32) as u32;
                let right = coords[2].max(0.0).min(img.width() as f32) as u32;
                let bottom = coords[3].max(0.0).min(img.height() as f32) as u32;
                if right <= x || bottom <= y {
                    return Err("Face bounds outside image".into());
                }
                let cropped = img
                    .crop_imm(x, y, right - x, bottom - y)
                    .thumbnail(150, 150);
                let temporary = media::cache::temporary(&cached);
                cropped
                    .save_with_format(&temporary, image::ImageFormat::Jpeg)
                    .map_err(|e| e.to_string())?;
                std::fs::rename(temporary, &cached).map_err(|e| e.to_string())?;
            }
        }
        Ok(cached.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_image_metadata(path: String) -> Result<scanner::ImageMetadata, String> {
    scanner::extract_metadata(&path).ok_or_else(|| "Failed to extract metadata".to_string())
}

#[tauri::command]
async fn update_image_metadata(
    path: String,
    new_date: Option<String>,
    new_lat: Option<f64>,
    new_lng: Option<f64>,
    app: tauri::AppHandle,
) -> Result<Option<(String, String)>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.indexer.gate.lock().unwrap();
        scanner::update_metadata(&path, new_date.clone(), new_lat, new_lng)?;
        let p = std::path::Path::new(&path);
        let dir = p.parent().ok_or("Invalid photo path")?;
        let local = indexing::open_folder(dir)?;
        let filename = p
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Invalid filename")?;
        let mut resolved_location = None;
        if let Some(mut date) = new_date {
            if date.len() == 16 {
                date.push_str(":00");
            }
            let dt =
                chrono::NaiveDateTime::parse_from_str(&date.replace('T', " "), "%Y-%m-%d %H:%M:%S")
                    .map_err(|e| e.to_string())?;
            local
                .execute(
                    "UPDATE photos SET taken_at=?1 WHERE path=?2",
                    rusqlite::params![dt.and_utc().timestamp(), filename],
                )
                .map_err(|e| e.to_string())?;
        }
        if let (Some(lat), Some(lng)) = (new_lat, new_lng) {
            let record = scanner::GEOCODER.search((lat, lng));
            let city = record.record.name.to_string();
            let country = record.record.cc.to_string();
            local
                .execute(
                    "UPDATE photos SET lat=?1,lng=?2,city=?3,country=?4 WHERE path=?5",
                    rusqlite::params![lat, lng, city, country, filename],
                )
                .map_err(|e| e.to_string())?;
            resolved_location = Some((city, country));
        }
        // Preserve the actual filesystem timestamp; changing capture date must never set file mtime.
        // The next reconciliation refreshes the stat/parser state after the metadata writer finishes.
        if let Some(c) = state.db_conn.lock().unwrap().as_mut() {
            state.indexer.project(c, dir)?;
        }
        Ok(resolved_location)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn rename_image(
    old_path: String,
    new_name: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.indexer.gate.lock().unwrap();
        let p = std::path::Path::new(&old_path);
        let dir = p.parent().ok_or("Invalid path")?;
        let candidate = std::path::Path::new(&new_name);
        if new_name.trim().is_empty()
            || candidate.components().count() != 1
            || !matches!(
                candidate.components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err("Enter a filename without directory components".into());
        }
        let mut name = new_name;
        if !name.contains('.') {
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                name = format!("{name}.{ext}");
            }
        }
        let new_path = dir.join(&name);
        if new_path.exists() {
            return Err("A file with that name already exists".into());
        }
        let local = indexing::open_folder(dir)?;
        let partner: Option<String> = local.query_row("SELECT q.path FROM photos p LEFT JOIN photos q ON q.id=p.paired_id WHERE p.path=?1", [p.file_name().unwrap().to_string_lossy()], |r|r.get(0)).unwrap_or(None);
        let partner_move = partner.map(|old| {
            let old=dir.join(old);
            let new=dir.join(format!("{}.{}", new_path.file_stem().unwrap().to_string_lossy(),old.extension().unwrap_or_default().to_string_lossy()));
            (old,new)
        });
        if partner_move.as_ref().map(|(_,new)|new.exists() || new==&new_path).unwrap_or(false) {return Err("The paired video destination already exists".into());}
        let tx=local.unchecked_transaction().map_err(|e|e.to_string())?;
        std::fs::rename(p, &new_path).map_err(|e| e.to_string())?;
        if let Some((old,new))=&partner_move {
            if let Err(e)=std::fs::rename(old,new) {
                std::fs::rename(&new_path,p).map_err(|rollback|format!("Rename failed: {e}; rollback failed: {rollback}"))?;
                return Err(e.to_string());
            }
        }
        let result=(|| -> Result<(),String> {
            tx.execute("UPDATE photos SET path=?1 WHERE path=?2",rusqlite::params![name,p.file_name().unwrap().to_string_lossy()]).map_err(|e|e.to_string())?;
            if let Some((old,new))=&partner_move {tx.execute("UPDATE photos SET path=?1 WHERE path=?2",rusqlite::params![new.file_name().unwrap().to_string_lossy(),old.file_name().unwrap().to_string_lossy()]).map_err(|e|e.to_string())?;}
            tx.commit().map_err(|e|e.to_string())
        })();
        if let Err(e)=result {
            let first=std::fs::rename(&new_path,p);
            let second=partner_move.as_ref().map(|(old,new)|std::fs::rename(new,old));
            return Err(format!("{e}; rename rollback: {first:?}, {second:?}"));
        }
        if let Some(c) = state.db_conn.lock().unwrap().as_mut() {
            state.indexer.project(c, dir)?;
        }
        Ok(new_path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn delete_photo(
    path: String,
    delete_from_disk: bool,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.indexer.gate.lock().unwrap();
        let p = std::path::Path::new(&path);
        let dir = p.parent().ok_or("Invalid path")?;
        let local = indexing::open_folder(dir)?;
        let name=p.file_name().unwrap().to_string_lossy();
        let partner:Option<String>=local.query_row("SELECT q.path FROM photos p LEFT JOIN photos q ON q.id=p.paired_id WHERE p.path=?1",[&name],|r|r.get(0)).unwrap_or(None);
        let mut files=vec![p.to_path_buf()];if let Some(partner)=partner{files.push(dir.join(partner));}
        if delete_from_disk {
            // Stage both files first so a permission error on the second leaves the pair intact.
            let mut staged=Vec::new();
            for file in &files {
                let temp=media::cache::temporary(file);
                if let Err(e)=std::fs::rename(file,&temp) {
                    for (original,temp) in staged.iter().rev(){std::fs::rename(temp,original).map_err(|r|format!("Delete failed: {e}; rollback failed: {r}"))?;}
                    return Err(e.to_string());
                }
                staged.push((file.clone(),temp));
            }
            for (_,temp) in &staged { std::fs::remove_file(temp).map_err(|e|format!("Could not remove staged file {}: {e}",temp.display()))?; }
        }
        let tx=local.unchecked_transaction().map_err(|e|e.to_string())?;
        for file in &files {tx.execute("UPDATE photos SET available=0,excluded=1 WHERE path=?1",[file.file_name().unwrap().to_string_lossy()]).map_err(|e|e.to_string())?;}
        tx.commit().map_err(|e|e.to_string())?;
        if let Some(c) = state.db_conn.lock().unwrap().as_mut() {
            state.indexer.project(c, dir)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("gmedia", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(media::protocol::motion(&app, request))
            });
        })
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .menu(updates::menu)
        .on_menu_event(|app, event| {
            if event.id() == updates::MENU_ID {
                updates::on_click(app);
            }
        })
        .setup(|app| {
            media::perf::initialize(app.path().app_log_dir()?);
            let data_dir = app.path().app_data_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            let indexer = indexing::Indexer::new(data_dir, cache_dir.clone())
                .map_err(std::io::Error::other)?;
            let conn = db::init_db(
                cache_dir
                    .join("catalog.db")
                    .to_str()
                    .ok_or("Invalid cache path")?,
            )?;
            scanner::prepare_thumbnail_cache(&conn)?;
            indexing::prepare_search(&conn).map_err(std::io::Error::other)?;
            app.manage(AppState {
                db_conn: Mutex::new(Some(conn)),
                indexer,
                face_service: face_worker::Service::default(),
                watchers: Mutex::new(std::collections::HashMap::new()),
                thumbnail_gate: Arc::new(media::cache::Pool::default()),
            });
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                let state = handle.state::<AppState>();
                let gb = std::fs::read(state.indexer.data_dir.join("media-settings.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|v| v["cacheGb"].as_u64())
                    .unwrap_or(2)
                    .clamp(1, 10);
                let _ = media::cache::sweep(&state.indexer.cache_dir, gb * 1024 * 1024 * 1024);
            });
            #[cfg(not(debug_assertions))]
            updates::check(app.handle().clone(), false);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_scan,
            initialize_indexing,
            get_indexed_folders,
            get_photo_count,
            start_watcher,
            get_photos,
            get_photos_after,
            get_timeline_summary,
            get_library_stats,
            search_suggest,
            media::perf::perf_log,
            get_thumbnail,
            media::commands::get_display_image,
            media::commands::get_media_details,
            media::commands::get_motion_src,
            media::commands::set_live_effect,
            media::commands::set_key_frame,
            media::commands::get_frame_strip,
            media::commands::get_media_settings,
            media::commands::set_media_settings,
            media::video::render_live_effect,
            media::video::get_anim_preview,
            media::video::export_live,
            get_file_uri,
            get_face_thumbnail,
            get_image_metadata,
            update_image_metadata,
            rename_image,
            delete_photo,
            people::get_people,
            people::get_faces_by_person,
            people::rename_person,
            people::get_uncertain_faces,
            people::confirm_face,
            people::assign_face,
            people::reject_face,
            people::get_people_cards,
            people::get_review_faces,
            people::batch_review_faces,
            people::merge_people,
            people::get_merge_suggestions,
            face_worker::start_face_scan,
            face_worker::cancel_face_scan,
            face_worker::get_face_scan_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
