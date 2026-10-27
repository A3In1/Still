//! Background pass that pre-generates grid thumbnails and their ThumbHash placeholders.
use crate::{scanner, AppState};
use rayon::prelude::*;
use std::{
    collections::HashSet,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::{Emitter, Manager};

const BATCH: usize = 64;

lazy_static::lazy_static! {
    /// Background previews get two low-priority threads instead of every core, so indexing a
    /// large library does not pin the CPU. Previews requested by scrolling are not limited by this.
    static ref POOL: rayon::ThreadPool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .thread_name(|i| format!("preview-warm-{i}"))
        .start_handler(|_| super::cache::low_priority_thread())
        .build()
        .expect("preview thread pool");
}
static RUNNING: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn schedule(app: &tauri::AppHandle) {
    REQUESTED.store(true, Ordering::Release);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut worked = false;
        while REQUESTED.swap(false, Ordering::AcqRel) {
            worked |= run(&app).unwrap_or(false);
        }
        RUNNING.store(false, Ordering::Release);
        // Projections can request more work as the previous run is exiting.
        // Hand off that request rather than waiting for the entire scan to end.
        if REQUESTED.load(Ordering::Acquire) {
            schedule(&app);
        }
        // One refresh at the end; the frontend only reloads photos, so this cannot re-trigger the pass.
        if worked {
            let _ = app.emit("index_updated", serde_json::json!({}));
        }
    });
}

type Row = (i64, String, Option<String>);

fn run(app: &tauri::AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let dir = state.indexer.cache_dir.join(scanner::THUMB_DIR);
    let mut failed = HashSet::new();
    let mut worked = false;
    loop {
        let rows: Vec<Row> = {
            let guard = state.db_conn.lock().unwrap();
            let Some(conn) = guard.as_ref() else {
                return Ok(worked);
            };
            // Rows using display derivatives (key frame / long exposure) keep their NULL hash.
            let mut stmt = conn.prepare("SELECT id,path,fs_mtime_ns FROM photos WHERE thumbhash IS NULL AND available=1 AND excluded=0
                AND COALESCE(pair_role,'')<>'motion' AND media_type IN ('photo','video') AND key_time_ms IS NULL AND live_effect<>'long_exposure'
                ORDER BY COALESCE(taken_at,mtime) DESC,id DESC LIMIT ?1").map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([(BATCH + failed.len()) as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            rows.into_iter()
                .filter(|r| !failed.contains(&r.0))
                .take(BATCH)
                .collect()
        };
        if rows.is_empty() {
            return Ok(worked);
        }
        let hashes: Vec<(&Row, Option<String>)> = POOL.install(|| {
            rows.par_iter()
                .map(|row| {
                    let _permit = state.thumbnail_gate.acquire();
                    let hash = scanner::thumbnail(Path::new(&row.1), &dir).and_then(|thumb| {
                        thumb.thumbhash.or_else(|| {
                            image::open(&thumb.path)
                                .ok()
                                .map(|image| scanner::thumbhash(&image))
                        })
                    });
                    (row, hash)
                })
                .collect()
        });
        let guard = state.db_conn.lock().unwrap();
        let Some(conn) = guard.as_ref() else {
            return Ok(worked);
        };
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (row, hash) in hashes {
            match hash {
                // The revision guard drops hashes computed for a file that changed meanwhile.
                Some(hash) => {
                    tx.execute(
                        "UPDATE photos SET thumbhash=?1 WHERE id=?2 AND fs_mtime_ns IS ?3",
                        rusqlite::params![hash, row.0, row.2],
                    )
                    .map_err(|e| e.to_string())?;
                    worked = true;
                }
                None => {
                    failed.insert(row.0);
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
    }
}
