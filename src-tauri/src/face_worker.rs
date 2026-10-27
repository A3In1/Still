//! Inference runs outside database locks. Only this Rust service commits detections.
use crate::{indexing, AppState};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    time::Duration,
};
use tauri::{Emitter, Manager};
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
const PREPROCESS: &str = "oriented-rgb-max2048-v1";
// Raised when decoding changes; DECODER-stale HEIF/AVIF rows are re-queued once.
const DECODER: i64 = 3;
// Recorded in face_state.decoder_version. Bumped when RAW/JPEG XL moved to native decoding, so rows
// that PIL could not read are retried once; other formats are unaffected (they compare against DECODER).
const STATE_DECODER: i64 = 4;
const DIM: usize = 512;
const MAX_UNKNOWN: usize = 100_000;
/// Largest batch the Python worker's HDBSCAN accepts (`MAX_CLUSTER` in face_pipeline.py).
const MAX_CLUSTER: usize = 2048;
// Quality gate: smaller or blurrier faces are stored and may be recognised, but never seed clusters or references.
pub(crate) const MIN_FACE_PX: f64 = 48.;
pub(crate) const MIN_BLUR: f64 = 40.;
const MAX_EXEMPLARS: usize = 12;
const EXEMPLAR_POOL: usize = 48;
/// SQL predicate over a `faces` row aliased `f`; rows scanned before quality was stored stay eligible.
pub(crate) fn eligible_sql() -> String {
    format!("(f.face_px IS NULL OR f.face_px>={MIN_FACE_PX}) AND (f.blur IS NULL OR f.blur>={MIN_BLUR})")
}

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub running: bool,
    pub processed: usize,
    pub failed: usize,
    pub message: String,
}
#[derive(Default)]
pub struct Service {
    running: AtomicBool,
    cancelled: AtomicBool,
    requested: AtomicBool,
    child: Mutex<Option<Child>>,
    status: Mutex<Status>,
}
impl Drop for Service {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
fn publish(app: &tauri::AppHandle, message: impl Into<String>) {
    let state = app.state::<AppState>();
    let mut status = state.face_service.status.lock().unwrap();
    status.message = message.into();
    let _ = app.emit("face_progress", status.clone());
}
#[tauri::command]
pub fn get_face_scan_status(state: tauri::State<'_, AppState>) -> Status {
    state.face_service.status.lock().unwrap().clone()
}
#[tauri::command]
pub fn cancel_face_scan(state: tauri::State<'_, AppState>) {
    state.face_service.cancelled.store(true, Ordering::Release);
    state.face_service.requested.store(false, Ordering::Release);
    if let Some(child) = state.face_service.child.lock().unwrap().as_mut() {
        let _ = child.kill();
    }
}
#[tauri::command]
pub async fn start_face_scan(app: tauri::AppHandle) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        {
            let _guard = state.indexer.gate.lock().unwrap();
            for folder in folders(&state)? {
                if !Path::new(&folder).is_dir() {
                    continue;
                }
                let local = indexing::open_folder(Path::new(&folder))?;
                local
                    .execute(
                        "UPDATE face_state SET attempts=0,retry_after=0 WHERE status='failed'",
                        [],
                    )
                    .map_err(err)?;
            }
        }
        state.face_service.cancelled.store(false, Ordering::Release);
        schedule(&app);
        Ok(())
    })
    .await
    .map_err(err)?
}

pub fn schedule(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    if state.face_service.cancelled.load(Ordering::Acquire) {
        return;
    }
    state.face_service.requested.store(true, Ordering::Release);
    if state.face_service.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        *state.face_service.status.lock().unwrap() = Status {
            running: true,
            message: "Checking for new faces…".into(),
            ..Default::default()
        };
        loop {
            state.face_service.requested.store(false, Ordering::Release);
            let result = run(&app);
            if let Some(mut child) = state.face_service.child.lock().unwrap().take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            match result {
                Ok(()) => {}
                Err(e) => {
                    publish(&app, e);
                    break;
                }
            }
            if state.face_service.cancelled.load(Ordering::Acquire) {
                publish(&app, "Face indexing paused");
                break;
            }
            if !state.face_service.requested.swap(false, Ordering::AcqRel) {
                let failed = state.face_service.status.lock().unwrap().failed;
                publish(
                    &app,
                    if failed == 0 {
                        "People are up to date".into()
                    } else {
                        format!("Face indexing finished; {failed} photos could not be processed")
                    },
                );
                break;
            }
        }
        state.face_service.status.lock().unwrap().running = false;
        state.face_service.running.store(false, Ordering::Release);
        let status = state.face_service.status.lock().unwrap().clone();
        let _ = app.emit("face_progress", status);
        // Close the scheduling race when a scan completes while this worker is winding down.
        if state.face_service.requested.swap(false, Ordering::AcqRel)
            && !state.face_service.cancelled.load(Ordering::Acquire)
        {
            schedule(&app);
        }
    });
}

struct Worker {
    input: ChildStdin,
    replies: mpsc::Receiver<Result<Value>>,
}
impl Worker {
    fn start(app: &tauri::AppHandle, runtime: &Path, version: &str) -> Result<Self> {
        let executable = runtime.join("gphotos-face").join(if cfg!(windows) {
            "gphotos-face.exe"
        } else {
            "gphotos-face"
        });
        let mut command = Command::new(&executable);
        if version.ends_with("-coreml") {
            command.env("GPHOTOS_FACE_COREML", "1").env(
                "GPHOTOS_FACE_CACHE",
                app.state::<AppState>().indexer.cache_dir.join("coreml"),
            );
        }
        // Run the model as a background process: macOS keeps it on the efficiency cores with
        // throttled I/O, which is slower but keeps the laptop cool. Plain `nice` is the fallback.
        #[cfg(target_os = "macos")]
        unsafe {
            use std::os::unix::process::CommandExt;
            unsafe extern "C" {
                fn setpriority(which: i32, who: u32, priority: i32) -> i32;
            }
            const PRIO_PROCESS: i32 = 0;
            const PRIO_DARWIN_PROCESS: i32 = 4;
            const PRIO_DARWIN_BG: i32 = 0x1000;
            command.pre_exec(|| {
                setpriority(PRIO_PROCESS, 0, 10);
                setpriority(PRIO_DARWIN_PROCESS, 0, PRIO_DARWIN_BG);
                Ok(())
            });
        }
        let mut child = command
            .arg(runtime.join("models"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                format!("Could not start bundled face worker: {e}. Build the face runtime first.")
            })?;
        let input = child.stdin.take().ok_or("Worker input unavailable")?;
        let output = child.stdout.take().ok_or("Worker output unavailable")?;
        *app.state::<AppState>().face_service.child.lock().unwrap() = Some(child);
        let (tx, replies) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                // Bound worker responses, including malformed/crashed-worker output.
                let mut bytes = Vec::new();
                let read = (&mut reader).take(4_000_000).read_until(b'\n', &mut bytes);
                match read {
                    Ok(0) => break,
                    Ok(_) if bytes.len() < 4_000_000 => {
                        if tx
                            .send(serde_json::from_slice(&bytes).map_err(err))
                            .is_err()
                        {
                            break;
                        }
                    }
                    _ => {
                        let _ = tx.send(Err("Invalid or oversized face-worker output".into()));
                        break;
                    }
                }
            }
        });
        let w = Self { input, replies };
        let ready = w.receive()?;
        if ready.get("model").and_then(Value::as_str) != Some(version) {
            return Err("Face worker/model version mismatch".into());
        }
        if ready["decoder_version"].as_i64().unwrap_or(1) < DECODER {
            return Err("Rebuild the face runtime to enable rich-media decoding".into());
        }
        Ok(w)
    }
    fn receive(&self) -> Result<Value> {
        let v = self
            .replies
            .recv_timeout(Duration::from_secs(180))
            .map_err(|_| "Face worker stopped or exceeded its time limit".to_string())??;
        if let Some(e) = v.get("error").and_then(Value::as_str) {
            return Err(e.into());
        }
        Ok(v)
    }
    fn call(&mut self, request: Value) -> Result<Value> {
        serde_json::to_writer(&mut self.input, &request).map_err(err)?;
        self.input.write_all(b"\n").map_err(err)?;
        self.input.flush().map_err(err)?;
        self.receive()
    }
}
fn runtime(app: &tauri::AppHandle) -> Result<PathBuf> {
    #[cfg(debug_assertions)]
    {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/face-runtime");
        if path.join("models/manifest.json").exists() {
            return Ok(path);
        }
    }
    Ok(app.path().resource_dir().map_err(err)?.join("face-runtime"))
}
#[derive(Deserialize)]
struct Detection {
    bbox: [f32; 4],
    score: f32,
    embedding: Vec<f32>,
    #[serde(default)]
    frame_ms: Option<i64>,
    #[serde(default)]
    frame_source: Option<String>,
    // Older bundled workers omit these; Rust derives face_px and a proxy quality from the box.
    #[serde(default)]
    quality: Option<f32>,
    #[serde(default)]
    blur: Option<f32>,
    #[serde(default)]
    face_px: Option<f32>,
}
impl Detection {
    fn measures(&self) -> (f32, Option<f32>, f32) {
        let (w, h) = (
            (self.bbox[2] - self.bbox[0]).max(0.),
            (self.bbox[3] - self.bbox[1]).max(0.),
        );
        let finite = |v: Option<f32>| v.filter(|x| x.is_finite() && *x >= 0.);
        (
            finite(self.quality).unwrap_or(self.score.max(0.) * w * h),
            finite(self.blur),
            finite(self.face_px).unwrap_or(w.min(h)),
        )
    }
}
#[derive(Deserialize)]
struct DetectionResult {
    unchanged: bool,
    digest: String,
    faces: Vec<Detection>,
}
#[derive(Clone)]
struct Job {
    id: i64,
    path: PathBuf,
    revision: String,
    digest: Option<String>,
    subtype: String,
    key: Option<i64>,
    duration: Option<i64>,
    deep: bool,
}
fn revision(mtime: &str, size: i64) -> String {
    format!("{mtime}:{size}")
}
fn frame_revision(mtime: &str, size: i64, key: Option<i64>) -> String {
    let base = revision(mtime, size);
    if let Some(key) = key {
        format!("{base}:key:{key}")
    } else {
        base
    }
}
fn bytes(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|x| x.to_le_bytes()).collect()
}
pub(crate) fn vector(blob: Vec<u8>) -> Option<Vec<f32>> {
    if blob.len() != DIM * 4 {
        return None;
    }
    let v: Vec<f32> = blob
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    if v.iter().all(|x| x.is_finite()) {
        Some(v)
    } else {
        None
    }
}
pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
pub(crate) fn normalized(v: &[f32]) -> Result<Vec<f32>> {
    let n = dot(v, v).sqrt();
    if v.len() != DIM || !n.is_finite() || n < 0.01 {
        return Err("Invalid face embedding".into());
    }
    Ok(v.iter().map(|x| x / n).collect())
}
fn folders(state: &AppState) -> Result<Vec<String>> {
    let c = state.db_conn.lock().unwrap();
    let c = c.as_ref().ok_or("Catalog unavailable")?;
    let result = c.prepare("SELECT folder_path FROM photos WHERE available=1 AND excluded=0 AND folder_path IS NOT NULL GROUP BY folder_path ORDER BY MAX(COALESCE(taken_at,mtime)) DESC").map_err(err)?
        .query_map([],|r|r.get(0)).map_err(err)?.collect::<rusqlite::Result<Vec<_>>>().map_err(err);
    result
}
fn setting(state: &AppState, key: &str) -> bool {
    std::fs::read(state.indexer.data_dir.join("media-settings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v[key].as_bool())
        .unwrap_or(false)
}
fn model_version(app: &tauri::AppHandle, runtime: &Path) -> Result<String> {
    let state = app.state::<AppState>();
    let manifest:Value=serde_json::from_slice(&std::fs::read(runtime.join("models/manifest.json")).map_err(|_|"Face runtime is not bundled. Run the face runtime build with the gphotos environment.".to_string())?).map_err(err)?;
    // Hardware (CoreML) embeddings are a separate vector space and never mix with CPU ones.
    Ok(format!(
        "{}{}",
        manifest["version"]
            .as_str()
            .ok_or("Invalid face model manifest")?,
        if setting(&state, "hardwareFaces") {
            "-coreml"
        } else {
            ""
        }
    ))
}
/// The embedding space new detections use, for comparisons made outside the worker.
pub(crate) fn current_version(app: &tauri::AppHandle) -> Option<String> {
    runtime(app).and_then(|r| model_version(app, &r)).ok()
}
fn run(app: &tauri::AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let dirs = folders(&state)?;
    if dirs.is_empty() {
        return Ok(());
    }
    let runtime = runtime(app)?;
    let version = &model_version(app, &runtime)?;
    let mut worker = None;
    let mut known = references(&state, version)?;
    let mut pending = 0;
    for dir in dirs {
        if !Path::new(&dir).is_dir() {
            continue;
        }
        let deep = setting(&state, "deepScan");
        // Keyset cursor over (capture time, id), newest first.
        let mut after = (i64::MAX, i64::MAX);
        loop {
            if state.face_service.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            let job = {
                let _guard = state.indexer.gate.lock().unwrap();
                let c = indexing::open_folder(Path::new(&dir))?;
                c.query_row("SELECT p.id,p.path,p.fs_mtime_ns,p.size,CASE WHEN s.model_version=?2 THEN s.pixel_digest ELSE NULL END,p.media_subtype,p.key_time_ms,p.anim_duration_ms,COALESCE(p.taken_at,p.mtime)
                    FROM photos p LEFT JOIN face_state s ON s.photo_id=p.id
                    WHERE (COALESCE(p.taken_at,p.mtime)<?1 OR (COALESCE(p.taken_at,p.mtime)=?1 AND p.id<?6)) AND p.available=1 AND p.excluded=0 AND p.media_subtype IN ('still','animated','live','motion_embedded') AND COALESCE(p.pair_role,'')<>'motion'
                    AND (s.photo_id IS NULL OR s.input_revision<>p.fs_mtime_ns||':'||p.size||COALESCE(':key:'||p.key_time_ms,'') OR s.model_version<>?2 OR s.status='running'
                    OR (s.decoder_version<?4 AND ((s.status='failed' AND p.media_format IN ('Heif','Avif')) OR p.media_subtype='animated' OR p.media_format IN ('Heif','Avif'))) OR (s.decoder_version<?5 AND p.media_format IN ('Raw','Jxl')) OR (?3 AND p.media_subtype IN ('live','motion_embedded') AND s.scan_depth<>'deep')
                    OR (s.status='failed' AND s.attempts<3 AND s.retry_after<=unixepoch())) ORDER BY COALESCE(p.taken_at,p.mtime) DESC,p.id DESC LIMIT 1",params![after.0,version,deep,DECODER,STATE_DECODER,after.1],|r|{
                        let name:String=r.get(1)?;let mtime:String=r.get(2)?;let size:i64=r.get(3)?;
                        Ok((Job{id:r.get(0)?,path:Path::new(&dir).join(name),revision:frame_revision(&mtime,size,r.get(6)?),digest:r.get(4)?,subtype:r.get(5)?,key:r.get(6)?,duration:r.get(7)?,deep},r.get::<_,i64>(8)?))
                    }).optional().map_err(err)?
            };
            let Some((job, ts)) = job else {
                break;
            };
            after = (ts, job.id);
            let before = match indexing::stat(&job.path) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if frame_revision(&before.0, before.1, job.key) != job.revision {
                continue;
            }
            if worker.is_none() {
                publish(app, "Loading face recognition…");
                worker = Some(Worker::start(app, &runtime, version)?);
            }
            publish(
                app,
                format!(
                    "Finding faces in {}",
                    job.path.file_name().unwrap_or_default().to_string_lossy()
                ),
            );
            let motion_before = if (job.deep || job.key.is_some())
                && ["live", "motion_embedded"].contains(&job.subtype.as_str())
            {
                crate::media::commands::motion_path(&state, &job.path)
                    .ok()
                    .and_then(|p| indexing::stat(&p).ok().map(|stat| (p, stat)))
            } else {
                None
            };
            let raw = state
                .indexer
                .cache_dir
                .join("face-raw")
                .join(format!("{}.rgb", job.id));
            let reply = (|| {
                let _span = crate::media::perf::Span::new("detect");
                let mut frames = Vec::new();
                if job.subtype == "animated" {
                    let duration = job.duration.unwrap_or(1000).max(1);
                    for ms in crate::media::video::animation_samples(&job.path, duration) {
                        let file =
                            crate::media::video::source_frame(&state, &job.path, ms, "animated")?;
                        frames.push(json!({"path":file,"frame_ms":ms,"frame_source":"animated"}));
                    }
                } else {
                    if let Some(ms) = job.key {
                        let file =
                            crate::media::video::source_frame(&state, &job.path, ms, "motion")?;
                        frames.push(json!({"path":file,"frame_ms":ms,"frame_source":"motion"}));
                    } else {
                        if matches!(
                            crate::media::decode::sniff(&job.path),
                            crate::media::decode::Kind::Heif
                                | crate::media::decode::Kind::Avif
                                | crate::media::decode::Kind::Raw
                                | crate::media::decode::Kind::Jxl
                        ) {
                            let metadata = crate::media::metadata::probe(&job.path, false);
                            let pixels = crate::media::decode::open_oriented(&job.path, Some(2048))
                                .map_err(err)?
                                .to_rgb8();
                            let (width, height) = pixels.dimensions();
                            let mut hash = xxhash_rust::xxh3::Xxh3::new();
                            hash.update(&width.to_le_bytes());
                            hash.update(&height.to_le_bytes());
                            hash.update(pixels.as_raw());
                            let digest = format!("x3:{:032x}", hash.digest128());
                            if frames.is_empty()
                                && !(job.deep
                                    && ["live", "motion_embedded"].contains(&job.subtype.as_str()))
                                && job.digest.as_deref() == Some(&digest)
                            {
                                return Ok(json!({"unchanged":true,"digest":digest,"faces":[]}));
                            }
                            std::fs::create_dir_all(raw.parent().unwrap()).map_err(err)?;
                            std::fs::write(&raw, pixels.as_raw()).map_err(err)?;
                            frames.push(json!({"raw":raw,"width":width,"height":height,"digest":digest,"frame_ms":null,"frame_source":"still","coordinate_width":metadata.width,"coordinate_height":metadata.height}));
                        } else {
                            frames.push(
                                json!({"path":job.path,"frame_ms":null,"frame_source":"still"}),
                            );
                        }
                    }
                    if job.deep && ["live", "motion_embedded"].contains(&job.subtype.as_str()) {
                        let motion = crate::media::commands::motion_path(&state, &job.path)?;
                        let duration = crate::media::metadata::probe(&motion, true)
                            .anim_duration_ms
                            .ok_or("Motion duration unavailable")?;
                        for i in 1..4 {
                            let ms = duration * i / 4;
                            let file =
                                crate::media::video::source_frame(&state, &job.path, ms, "motion")?;
                            frames.push(json!({"path":file,"frame_ms":ms,"frame_source":"motion"}));
                        }
                    }
                }
                worker.as_mut().unwrap().call(json!({"op":"detect","path":job.path,"pixel_digest":job.digest,"frames":frames}))
            })();
            let _ = std::fs::remove_file(&raw);
            if state.face_service.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            if motion_before
                .as_ref()
                .map(|(path, stat)| indexing::stat(path).ok().as_ref() != Some(stat))
                .unwrap_or(false)
            {
                continue;
            }
            let result =
                reply.and_then(|r| serde_json::from_value::<DetectionResult>(r).map_err(err));
            let _guard = state.indexer.gate.lock().unwrap();
            let mut local = indexing::open_folder(Path::new(&dir))?;
            if indexing::stat(&job.path).ok().as_ref() != Some(&before) {
                continue;
            }
            let current:Option<(String,i64,Option<i64>)>=local.query_row("SELECT fs_mtime_ns,size,key_time_ms FROM photos WHERE id=?1 AND available=1 AND excluded=0",[job.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
            if current.map(|v| frame_revision(&v.0, v.1, v.2)).as_deref() != Some(&job.revision) {
                continue;
            }
            match result {
                Ok(result) => {
                    commit(&mut local, &job, result, version, &known)?;
                    pending += 1;
                    state.face_service.status.lock().unwrap().processed += 1;
                    if pending >= 32 {
                        if let Some(c) = state.db_conn.lock().unwrap().as_mut() {
                            state.indexer.project(c, Path::new(&dir))?;
                        }
                        known = references(&state, version)?;
                        pending = 0;
                        let _ = app.emit("people_updated", ());
                    }
                }
                Err(e) => {
                    local.execute("INSERT INTO face_state(photo_id,input_revision,model_version,status,error,attempts,retry_after) VALUES(?1,?2,?3,'failed',?4,1,unixepoch()+600)
                        ON CONFLICT(photo_id) DO UPDATE SET input_revision=excluded.input_revision,model_version=excluded.model_version,status='failed',error=excluded.error,
                        attempts=CASE WHEN face_state.input_revision=excluded.input_revision AND face_state.model_version=excluded.model_version THEN face_state.attempts+1 ELSE 1 END,retry_after=excluded.retry_after",params![job.id,job.revision,version,e]).map_err(err)?;
                    local
                        .execute(
                            "UPDATE face_state SET decoder_version=?2 WHERE photo_id=?1",
                            params![job.id, STATE_DECODER],
                        )
                        .map_err(err)?;
                    state.face_service.status.lock().unwrap().failed += 1;
                    publish(
                        app,
                        format!("Could not process {}: {e}", job.path.display()),
                    );
                    // A broken/blocked subprocess should not fail every remaining photo.
                    if state
                        .face_service
                        .child
                        .lock()
                        .unwrap()
                        .as_mut()
                        .and_then(|c| c.try_wait().ok())
                        .flatten()
                        .is_some()
                        || e.contains("time limit")
                    {
                        return Err(e);
                    }
                }
            }
        }
        {
            let _guard = state.indexer.gate.lock().unwrap();
            if let Some(c) = state.db_conn.lock().unwrap().as_mut() {
                if state.indexer.project(c, Path::new(&dir))? {
                    let _ = app.emit("people_updated", ());
                }
            }
            pending = 0;
            known = references(&state, version)?;
        }
    }
    // Classification also runs after confirmations, without rereading original images.
    classify_pending(app, version)?;
    cluster_pending(app, version, &runtime, &mut worker)?;
    Ok(())
}

struct Reference {
    uuid: String,
    name: String,
    vectors: Vec<Vec<f32>>,
}
fn references(state: &AppState, version: &str) -> Result<Vec<Reference>> {
    let guard = state.db_conn.lock().unwrap();
    let c = guard.as_ref().ok_or("Catalog unavailable")?;
    if c.query_row(
        "SELECT 1 FROM sqlite_master WHERE name='recognition_vectors'",
        [],
        |_| Ok(()),
    )
    .optional()
    .map_err(err)?
    .is_none()
    {
        return Ok(vec![]);
    }
    // Best faces first (NULL quality last) so each person's candidate pool holds their clearest faces.
    let mut query=c.prepare(&format!("SELECT v.person_uuid,v.name,v.face_uuid,v.vector,{} FROM recognition_vectors v JOIN faces f ON f.source_key=v.source_key JOIN photos p ON p.id=f.photo_id
        WHERE v.source='user_confirmed' AND v.user_named=1 AND v.model_version=?1 AND f.retired=0 AND p.available=1 AND p.excluded=0
        ORDER BY v.person_uuid,f.quality IS NULL,f.quality DESC,f.id", eligible_sql())).map_err(err)?;
    let rows = query
        .query_map([version], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Vec<u8>>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })
        .map_err(err)?;
    struct Pool {
        uuid: String,
        name: String,
        good: Vec<Vec<f32>>,
        poor: Vec<Vec<f32>>,
        good_sum: Vec<f32>,
        all_sum: Vec<f32>,
    }
    let mut pools: Vec<Pool> = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        let (uuid, name, face, blob, good) = row.map_err(err)?;
        if !seen.insert(face) {
            continue;
        }
        let Some(v) = vector(blob) else {
            continue;
        };
        if pools.last().map(|r| r.uuid.as_str()) != Some(&uuid) {
            if pools.len() >= 2048 {
                break;
            }
            pools.push(Pool {
                uuid,
                name,
                good: vec![],
                poor: vec![],
                good_sum: vec![0.; DIM],
                all_sum: vec![0.; DIM],
            });
        }
        let p = pools.last_mut().unwrap();
        for (a, b) in p.all_sum.iter_mut().zip(&v) {
            *a += b;
        }
        if good {
            for (a, b) in p.good_sum.iter_mut().zip(&v) {
                *a += b;
            }
            if p.good.len() < EXEMPLAR_POOL {
                p.good.push(v);
            }
        } else if p.poor.len() < EXEMPLAR_POOL {
            p.poor.push(v);
        }
    }
    let mut refs = Vec::with_capacity(pools.len());
    for p in pools {
        // A person known only from gated faces still needs references to be recognised at all.
        let (candidates, sum) = if p.good.is_empty() {
            (p.poor, p.all_sum)
        } else {
            (p.good, p.good_sum)
        };
        let mut vectors = farthest_points(candidates, MAX_EXEMPLARS);
        if let Ok(mean) = normalized(&sum) {
            vectors.push(mean);
        }
        refs.push(Reference {
            uuid: p.uuid,
            name: p.name,
            vectors,
        });
    }
    Ok(refs)
}
/// Starts from the best-quality face, then repeatedly keeps the face least similar to those already kept.
fn farthest_points(mut pool: Vec<Vec<f32>>, k: usize) -> Vec<Vec<f32>> {
    if pool.len() <= k {
        return pool;
    }
    let mut chosen = vec![pool.swap_remove(0)];
    // Nearest similarity of each remaining candidate to the chosen set.
    let mut nearest: Vec<f32> = pool.iter().map(|v| dot(v, &chosen[0])).collect();
    while chosen.len() < k && !pool.is_empty() {
        let (i, _) = nearest
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        let next = pool.swap_remove(i);
        nearest.swap_remove(i);
        for (s, v) in nearest.iter_mut().zip(&pool) {
            *s = s.max(dot(v, &next));
        }
        chosen.push(next);
    }
    chosen
}
fn classify<'a>(
    v: &[f32],
    refs: &'a [Reference],
    rejected: &HashSet<String>,
) -> Option<(&'a Reference, f32)> {
    let mut best = None;
    let mut second = -1.0;
    for r in refs {
        if rejected.contains(&r.uuid) {
            continue;
        }
        let score = r.vectors.iter().map(|p| dot(v, p)).fold(-1.0, f32::max);
        if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
            second = best.map(|(_, s)| s).unwrap_or(-1.0);
            best = Some((r, score));
        } else {
            second = second.max(score);
        }
    }
    best.filter(|(_, s)| *s >= 0.65 && *s - second >= 0.08)
}
fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let inter =
        (a[2].min(b[2]) - a[0].max(b[0])).max(0.) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.);
    let union = (a[2] - a[0]).max(0.) * (a[3] - a[1]).max(0.)
        + (b[2] - b[0]).max(0.) * (b[3] - b[1]).max(0.)
        - inter;
    if union > 0. {
        inter / union
    } else {
        0.
    }
}
fn commit(
    c: &mut rusqlite::Connection,
    job: &Job,
    result: DetectionResult,
    version: &str,
    refs: &[Reference],
) -> Result<()> {
    if result.faces.len() > 128 || ![64, 35].contains(&result.digest.len()) {
        return Err("Invalid detection result".into());
    }
    let tx = c.transaction().map_err(err)?;
    if !result.unchanged {
        let old:Vec<(i64,String,String,String,Option<Vec<u8>>,Option<i64>,Option<String>)>=tx.prepare("SELECT f.id,f.portable_id,f.bbox,f.assignment_source,e.vector,f.frame_ms,f.frame_source FROM faces f LEFT JOIN face_embeddings e ON e.face_id=f.id AND e.model_version=?2 WHERE f.photo_id=?1 AND f.retired<>1").map_err(err)?
            .query_map(params![job.id,version],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(err)?.collect::<rusqlite::Result<_>>().map_err(err)?;
        tx.execute("UPDATE faces SET retired=CASE WHEN assignment_source IN ('user_confirmed','user_named_group','user_rejected','legacy') THEN 2 ELSE 1 END WHERE photo_id=?1",[job.id]).map_err(err)?;
        let mut used = HashSet::new();
        let has_manual = old.iter().any(|f| {
            matches!(
                f.3.as_str(),
                "user_confirmed" | "user_named_group" | "user_rejected" | "legacy"
            )
        });
        for detection in result.faces {
            let v = normalized(&detection.embedding)?;
            let (quality, blur, face_px) = detection.measures();
            if detection.bbox.iter().any(|x| !x.is_finite()) || !detection.score.is_finite() {
                return Err("Invalid face coordinates".into());
            }
            let mut matches: Vec<(usize, f32)> = old
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    if used.contains(&i) {
                        return None;
                    }
                    if f.6.as_deref().unwrap_or("still")
                        != detection.frame_source.as_deref().unwrap_or("still")
                        || match (f.5, detection.frame_ms) {
                            (None, None) => false,
                            (Some(a), Some(b)) => (a - b).abs() > 100,
                            _ => true,
                        }
                    {
                        return None;
                    }
                    let bbox: [f32; 4] = serde_json::from_str(&f.2).ok()?;
                    let previous = vector(f.4.clone()?)?;
                    let score = dot(&v, &previous);
                    if iou(&bbox, &detection.bbox) >= 0.5 && score >= 0.85 {
                        Some((i, score))
                    } else {
                        None
                    }
                })
                .collect();
            matches.sort_by(|a, b| b.1.total_cmp(&a.1));
            let matched = matches
                .first()
                .filter(|m| matches.get(1).map(|s| m.1 - s.1 > 0.05).unwrap_or(true));
            let id = if let Some((i, _)) = matched {
                used.insert(*i);
                let id = old[*i].0;
                tx.execute(
                    "UPDATE faces SET bbox=?1,detection_score=?2,retired=0 WHERE id=?3",
                    params![
                        serde_json::to_string(&detection.bbox).map_err(err)?,
                        detection.score,
                        id
                    ],
                )
                .map_err(err)?;
                id
            } else {
                let known = if has_manual {
                    None
                } else {
                    classify(&v, refs, &HashSet::new())
                };
                let person = if let Some((r, _)) = known {
                    tx.execute("INSERT INTO people(name,portable_id,user_named) VALUES(?1,?2,1) ON CONFLICT(portable_id) DO NOTHING",params![r.name,r.uuid]).map_err(err)?;
                    Some(
                        tx.query_row(
                            "SELECT id FROM people WHERE portable_id=?1",
                            [&r.uuid],
                            |r| r.get::<_, i64>(0),
                        )
                        .map_err(err)?,
                    )
                } else {
                    None
                };
                tx.execute("INSERT INTO faces(photo_id,person_id,bbox,confidence,portable_id,assignment_source,detection_score) VALUES(?1,?2,?3,?4,lower(hex(randomblob(16))),?5,?6)",params![job.id,person,serde_json::to_string(&detection.bbox).map_err(err)?,known.map(|(_,s)|s).unwrap_or(0.),if known.is_some(){"automatic"}else if has_manual{"needs_review"}else{"unassigned"},detection.score]).map_err(err)?;
                tx.last_insert_rowid()
            };
            tx.execute(
                "UPDATE faces SET frame_ms=?1,frame_source=?2,quality=?3,blur=?4,face_px=?5 WHERE id=?6",
                params![detection.frame_ms, detection.frame_source, quality, blur, face_px, id],
            )
            .map_err(err)?;
            tx.execute("INSERT INTO face_embeddings VALUES(?1,?2,?3,512,?4) ON CONFLICT(face_id,model_version,preprocessing_version) DO UPDATE SET vector=excluded.vector",params![id,version,PREPROCESS,bytes(&v)]).map_err(err)?;
        }
    }
    tx.execute("INSERT INTO face_state(photo_id,input_revision,model_version,status,pixel_digest) VALUES(?1,?2,?3,'complete',?4)
        ON CONFLICT(photo_id) DO UPDATE SET input_revision=excluded.input_revision,model_version=excluded.model_version,status='complete',pixel_digest=excluded.pixel_digest,error=NULL,attempts=0,retry_after=0",params![job.id,job.revision,version,result.digest]).map_err(err)?;
    tx.execute(
        "UPDATE face_state SET decoder_version=?3,scan_depth=?1 WHERE photo_id=?2",
        params![
            if job.deep && ["live", "motion_embedded"].contains(&job.subtype.as_str()) {
                "deep"
            } else {
                "still"
            },
            job.id,
            STATE_DECODER
        ],
    )
    .map_err(err)?;
    // Zero-face and metadata-only completions must also invalidate the derived projection.
    tx.execute("UPDATE folder_meta SET revision=revision+1", [])
        .map_err(err)?;
    tx.commit().map_err(err)
}

#[derive(Clone)]
struct Unknown {
    folder: String,
    uuid: String,
    person: Option<String>,
    vector: Vec<f32>,
}
/// `gated` keeps only faces that pass the quality gate (clustering); classification sees every face.
/// `regroup` keeps only faces still waiting for a group: never grouped, or alone in a one-face group.
fn unknowns(state: &AppState, version: &str, gated: bool, regroup: bool) -> Result<Vec<Unknown>> {
    let guard = state.db_conn.lock().unwrap();
    let c = guard.as_ref().ok_or("Catalog unavailable")?;
    let gate = if gated { format!("AND {}", eligible_sql()) } else { String::new() };
    let waiting = if regroup {
        "AND (v.source='unassigned' OR v.person_uuid IS NULL OR (SELECT COUNT(*) FROM recognition_vectors w WHERE w.person_uuid=v.person_uuid AND w.model_version=?1)=1)"
    } else {
        ""
    };
    let mut q=c.prepare(&format!("SELECT v.folder_path,v.face_uuid,v.person_uuid,v.vector FROM recognition_vectors v JOIN faces f ON f.source_key=v.source_key JOIN photos p ON p.id=f.photo_id
        WHERE v.model_version=?1 AND v.user_named=0 AND v.source IN ('unassigned','cluster') AND f.retired=0 AND p.available=1 AND p.excluded=0 {gate} {waiting}
        ORDER BY CASE WHEN v.source='unassigned' THEN 0 ELSE 1 END,f.id DESC LIMIT ?2")).map_err(err)?;
    let rows = q
        .query_map(params![version, MAX_UNKNOWN as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Vec<u8>>(3)?,
            ))
        })
        .map_err(err)?;
    let mut res = vec![];
    let mut seen = HashSet::new();
    for row in rows {
        let (folder, uuid, person, blob) = row.map_err(err)?;
        if !seen.insert(uuid.clone()) {
            continue;
        }
        if let Some(vector) = vector(blob) {
            res.push(Unknown {
                folder,
                uuid,
                person,
                vector,
            });
        }
    }
    Ok(res)
}

/// An unnamed group of two or more faces, as a running sum of its good-quality embeddings.
struct Group {
    uuid: String,
    sum: Vec<f32>,
    mean: Vec<f32>,
}
impl Group {
    fn add(&mut self, v: &[f32]) {
        for (a, b) in self.sum.iter_mut().zip(v) {
            *a += b;
        }
        if let Ok(mean) = normalized(&self.sum) {
            self.mean = mean;
        }
    }
}
fn unnamed_groups(state: &AppState, version: &str) -> Result<Vec<Group>> {
    let guard = state.db_conn.lock().unwrap();
    let c = guard.as_ref().ok_or("Catalog unavailable")?;
    let mut q = c.prepare(&format!("SELECT v.person_uuid,v.face_uuid,v.vector FROM recognition_vectors v JOIN faces f ON f.source_key=v.source_key JOIN photos p ON p.id=f.photo_id
        WHERE v.model_version=?1 AND v.user_named=0 AND v.person_uuid IS NOT NULL AND f.retired=0 AND p.available=1 AND p.excluded=0 AND {}",
        eligible_sql())).map_err(err)?;
    let rows = q
        .query_map([version], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?)))
        .map_err(err)?;
    let mut sums: HashMap<String, (Vec<f32>, usize)> = HashMap::new();
    let mut seen = HashSet::new();
    for row in rows {
        let (person, face, blob) = row.map_err(err)?;
        if !seen.insert(face) {
            continue;
        }
        let Some(v) = vector(blob) else { continue };
        let entry = sums.entry(person).or_insert_with(|| (vec![0.; DIM], 0));
        for (a, b) in entry.0.iter_mut().zip(&v) {
            *a += b;
        }
        entry.1 += 1;
    }
    Ok(sums
        .into_iter()
        .filter(|(_, (_, count))| *count >= 2)
        .filter_map(|(uuid, (sum, _))| normalized(&sum).ok().map(|mean| Group { uuid, sum, mean }))
        .collect())
}

/// Similarity a waiting face needs to join an existing unnamed group, and its lead over the runner-up.
/// Deliberately stricter than splitting: a wrong join mixes two people, a missed one is a merge suggestion.
const JOIN_SCORE: f32 = 0.6;
const JOIN_MARGIN: f32 = 0.05;

/// Best group for a face, skipping groups the user said this face is not.
fn best_group(v: &[f32], groups: &[Group], rejected: &HashSet<(String, String)>, face: &str) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;
    let mut second = -1.0f32;
    for (i, g) in groups.iter().enumerate() {
        if rejected.contains(&(face.to_string(), g.uuid.clone())) {
            continue;
        }
        let score = dot(v, &g.mean);
        match best {
            Some((_, s)) if score <= s => second = second.max(score),
            _ => {
                second = best.map(|(_, s)| s).unwrap_or(-1.0);
                best = Some((i, score));
            }
        }
    }
    best.filter(|(_, s)| *s >= JOIN_SCORE && *s - second >= JOIN_MARGIN)
}

fn classify_pending(app: &tauri::AppHandle, version: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let _guard = state.indexer.gate.lock().unwrap();
    let refs = references(&state, version)?;
    if refs.is_empty() {
        return Ok(());
    }
    // One connection and one transaction per folder: opening a folder DB runs its migrations.
    let mut folders: HashMap<String, rusqlite::Connection> = HashMap::new();
    let mut touched = HashSet::new();
    for face in unknowns(&state, version, false, false)? {
        if state.face_service.cancelled.load(Ordering::Acquire) {
            break;
        }
        if !folders.contains_key(&face.folder) {
            let c = indexing::open_folder(Path::new(&face.folder))?;
            c.execute_batch("BEGIN IMMEDIATE").map_err(err)?;
            folders.insert(face.folder.clone(), c);
        }
        let c = &folders[&face.folder];
        let rejected: HashSet<String> = c
            .prepare("SELECT person_uuid FROM face_rejections WHERE face_uuid=?1")
            .map_err(err)?
            .query_map([&face.uuid], |r| r.get(0))
            .map_err(err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(err)?;
        if let Some((r, score)) = classify(&face.vector, &refs, &rejected) {
            c.execute("INSERT INTO people(name,portable_id,user_named) VALUES(?1,?2,1) ON CONFLICT(portable_id) DO NOTHING",params![r.name,r.uuid]).map_err(err)?;
            c.execute("UPDATE faces SET person_id=(SELECT id FROM people WHERE portable_id=?1),assignment_source='automatic',confidence=?2 WHERE portable_id=?3 AND retired=0 AND assignment_source IN ('unassigned','cluster')",params![r.uuid,score,face.uuid]).map_err(err)?;
            touched.insert(face.folder.clone());
        }
    }
    for (_, c) in folders {
        c.execute_batch("COMMIT").map_err(err)?;
    }
    for folder in touched {
        if let Some(catalog) = state.db_conn.lock().unwrap().as_mut() {
            state.indexer.project(catalog, Path::new(&folder))?;
        }
    }
    Ok(())
}
/// Incremental grouping, so every unknown face gets a group however large the library is:
/// 1. a face waiting for a group joins the closest existing unnamed group when it clearly matches;
/// 2. the rest are grouped with HDBSCAN in chunks the worker accepts, and each new group is
///    immediately available for the next chunk to join, so a person split across chunks reconnects.
/// Faces already in a group of two or more are left alone, so groups stay stable between runs.
fn cluster_pending(
    app: &tauri::AppHandle,
    version: &str,
    runtime: &Path,
    worker: &mut Option<Worker>,
) -> Result<()> {
    let state = app.state::<AppState>();
    let (mut groups, candidates) = {
        let _guard = state.indexer.gate.lock().unwrap();
        (unnamed_groups(&state, version)?, unknowns(&state, version, true, true)?)
    };
    if candidates.is_empty() {
        return Ok(());
    }
    let mut rejected: HashSet<(String, String)> = HashSet::new();
    for folder in candidates.iter().map(|f| f.folder.as_str()).collect::<HashSet<_>>() {
        let c = indexing::open_folder(Path::new(folder))?;
        let rows: Vec<(String, String)> = c
            .prepare("SELECT face_uuid,person_uuid FROM face_rejections")
            .map_err(err)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(err)?;
        rejected.extend(rows);
    }
    // face index -> chosen group uuid (None = keep as its own one-face group)
    let mut chosen: Vec<Option<String>> = vec![None; candidates.len()];
    let mut waiting: Vec<usize> = (0..candidates.len()).collect();
    let mut grouping_announced = false;
    while !waiting.is_empty() {
        if state.face_service.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        // 1. Join existing groups.
        let mut rest = Vec::with_capacity(waiting.len());
        for i in waiting {
            let face = &candidates[i];
            if let Some((g, _)) = best_group(&face.vector, &groups, &rejected, &face.uuid) {
                groups[g].add(&face.vector);
                chosen[i] = Some(groups[g].uuid.clone());
            } else {
                rest.push(i);
            }
        }
        if rest.len() < 2 {
            break;
        }
        // 2. Group one chunk of the remaining faces.
        let chunk: Vec<usize> = rest.iter().copied().take(MAX_CLUSTER).collect();
        waiting = rest.into_iter().skip(MAX_CLUSTER).collect();
        if worker.is_none() {
            *worker = Some(Worker::start(app, runtime, version)?);
        }
        if !grouping_announced {
            publish(app, "Grouping unknown faces…");
            grouping_announced = true;
        }
        let result = worker.as_mut().unwrap().call(
            json!({"op":"cluster","vectors":chunk.iter().map(|&i|&candidates[i].vector).collect::<Vec<_>>()}),
        )?;
        let labels: Vec<i64> = serde_json::from_value(result["labels"].clone()).map_err(err)?;
        if labels.len() != chunk.len() {
            return Err("Invalid cluster response".into());
        }
        let mut members: HashMap<i64, Vec<usize>> = HashMap::new();
        for (&i, &label) in chunk.iter().zip(&labels) {
            if label >= 0 {
                members.entry(label).or_default().push(i);
            }
        }
        let mut labels_sorted: Vec<i64> = members.keys().copied().collect();
        labels_sorted.sort();
        let mut taken = HashSet::new();
        for label in labels_sorted {
            let faces = &members[&label];
            // Reuse one existing one-face identity so its name/rejections carry over; never reuse twice.
            let uuid = faces
                .iter()
                .filter_map(|&i| candidates[i].person.clone())
                .find(|p| taken.insert(p.clone()))
                .unwrap_or_else(new_uuid);
            let mut group = Group { uuid: uuid.clone(), sum: vec![0.; DIM], mean: vec![] };
            for &i in faces {
                group.add(&candidates[i].vector);
                chosen[i] = Some(uuid.clone());
            }
            if !group.mean.is_empty() {
                groups.push(group);
            }
        }
    }

    let _guard = state.indexer.gate.lock().unwrap();
    let mut folders: HashMap<String, rusqlite::Connection> = HashMap::new();
    let mut touched = HashSet::new();
    for (face, group) in candidates.iter().zip(chosen) {
        if state.face_service.cancelled.load(Ordering::Acquire) {
            break;
        }
        // Noise keeps (or gets) its own one-face identity; it can still join a group on a later run.
        let person = match group.or_else(|| face.person.clone()) {
            Some(p) => p,
            None => new_uuid(),
        };
        if face.person.as_deref() == Some(person.as_str()) {
            continue;
        }
        if !folders.contains_key(&face.folder) {
            let c = indexing::open_folder(Path::new(&face.folder))?;
            c.execute_batch("BEGIN IMMEDIATE").map_err(err)?;
            folders.insert(face.folder.clone(), c);
        }
        let c = &folders[&face.folder];
        let name = format!("Person #{}", &person[..person.len().min(6)]);
        c.execute("INSERT INTO people(name,portable_id,user_named) VALUES(?1,?2,0) ON CONFLICT(portable_id) DO NOTHING",params![name,person]).map_err(err)?;
        // A name/confirmation may have been saved while grouping ran. Never overwrite it.
        let changed = c.execute("UPDATE faces SET person_id=(SELECT id FROM people WHERE portable_id=?1),assignment_source='cluster',confidence=0
            WHERE portable_id=?2 AND retired=0 AND assignment_source IN ('unassigned','cluster')
            AND person_id IS NOT (SELECT id FROM people WHERE portable_id=?1)
            AND NOT EXISTS(SELECT 1 FROM people p WHERE p.id=faces.person_id AND p.user_named=1)
            AND NOT EXISTS(SELECT 1 FROM face_rejections WHERE face_uuid=?2 AND person_uuid=?1)
            AND NOT EXISTS(SELECT 1 FROM people WHERE portable_id=?1 AND user_named=1)",params![person,face.uuid]).map_err(err)?;
        if changed > 0 {
            touched.insert(face.folder.clone());
        }
    }
    for (_, c) in folders {
        c.execute_batch("COMMIT").map_err(err)?;
    }
    if touched.is_empty() {
        return Ok(());
    }
    for folder in touched {
        if let Some(c) = state.db_conn.lock().unwrap().as_mut() {
            state.indexer.project(c, Path::new(&folder))?;
        }
    }
    let _ = app.emit("people_updated", ());
    Ok(())
}
fn new_uuid() -> String {
    let mut bytes = [0u8; 16];
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let hash = xxhash_rust::xxh3::xxh3_128(&[seed.to_le_bytes().as_slice(), &n.to_le_bytes(), &std::process::id().to_le_bytes()].concat());
    bytes.copy_from_slice(&hash.to_le_bytes());
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
