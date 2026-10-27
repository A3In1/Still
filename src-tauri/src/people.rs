use crate::AppState;
use serde::Serialize;
use crate::face_worker::{dot, eligible_sql, normalized, vector};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use tauri::Manager;

#[derive(Serialize)]
pub struct PersonRecord {
    pub id: i64,
    pub name: String,
    pub face_count: i64,
    pub sample_photo_path: Option<String>,
    pub sample_bbox: Option<String>,
    pub sample_thumbnail_b64: Option<String>,
    pub sample_revision: Option<String>,
    pub sample_frame_ms: Option<i64>,
    pub sample_frame_source: Option<String>,
}

#[derive(Serialize)]
pub struct FaceRecord {
    pub id: i64,
    pub photo_id: i64,
    pub photo_path: String,
    pub person_id: Option<i64>,
    pub bbox: String,
    pub confidence: f64,
    pub thumbnail_b64: Option<String>,
    pub source_revision: String,
    pub frame_ms: Option<i64>,
    pub frame_source: Option<String>,
}

// Async + spawn_blocking: sync commands run on the main thread and would freeze the window
// while the face worker holds the catalog lock.
#[tauri::command]
pub async fn get_people(app: tauri::AppHandle) -> Result<Vec<PersonRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || get_people_blocking(&app.state::<AppState>()))
        .await
        .map_err(|e| e.to_string())?
}
fn get_people_blocking(state: &AppState) -> Result<Vec<PersonRecord>, String> {
    if let Some(conn) = state.db_conn.lock().unwrap().as_ref() {
        let mut stmt = conn.prepare(
            "WITH visible AS (
                SELECT f.*,ph.path,COALESCE(ph.fs_mtime_ns,'')||':'||COALESCE(ph.size,0) AS revision FROM faces f JOIN photos ph ON ph.id=f.photo_id
                WHERE f.retired=0 AND ph.available=1 AND ph.excluded=0
             )
             SELECT p.id,p.name,COUNT(DISTINCT f.photo_id),sample.path,sample.bbox,sample.thumbnail,sample.revision,sample.frame_ms,sample.frame_source
             FROM people p LEFT JOIN visible f ON f.person_id=p.id
             LEFT JOIN visible sample ON sample.id=(SELECT id FROM visible WHERE person_id=p.id ORDER BY quality IS NULL,quality DESC,confidence DESC,id LIMIT 1)
             WHERE f.id IS NOT NULL OR EXISTS(SELECT 1 FROM faces old WHERE old.person_id=p.id AND old.retired<>1)
             GROUP BY p.id ORDER BY COUNT(DISTINCT f.photo_id) DESC"
        ).map_err(|e| e.to_string())?;

        let iter = stmt
            .query_map([], |row| {
                Ok(PersonRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    face_count: row.get(2)?,
                    sample_photo_path: row.get(3)?,
                    sample_bbox: row.get(4)?,
                    sample_thumbnail_b64: row.get(5)?,
                    sample_revision: row.get(6)?,
                    sample_frame_ms: row.get(7)?,
                    sample_frame_source: row.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut res = Vec::new();
        for p in iter {
            res.push(p.map_err(|e| e.to_string())?);
        }
        Ok(res)
    } else {
        Err("DB not initialized".into())
    }
}

#[tauri::command]
pub async fn get_faces_by_person(
    person_id: i64,
    app: tauri::AppHandle,
) -> Result<Vec<FaceRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        get_faces_by_person_blocking(person_id, &app.state::<AppState>())
    })
    .await
    .map_err(|e| e.to_string())?
}
fn get_faces_by_person_blocking(person_id: i64, state: &AppState) -> Result<Vec<FaceRecord>, String> {
    if let Some(conn) = state.db_conn.lock().unwrap().as_ref() {
        let mut stmt = conn
            .prepare(
                "SELECT f.id, f.photo_id, ph.path, f.person_id, f.bbox, f.confidence, f.thumbnail, COALESCE(ph.fs_mtime_ns,'')||':'||COALESCE(ph.size,0),f.frame_ms,f.frame_source
             FROM faces f
             JOIN photos ph ON f.photo_id = ph.id
             WHERE f.retired=0 AND f.person_id = ?1 AND ph.available=1 AND ph.excluded=0
             ORDER BY COALESCE(ph.taken_at,ph.mtime) DESC, ph.id DESC, f.id",
            )
            .map_err(|e| e.to_string())?;

        let iter = stmt
            .query_map([person_id], |row| {
                Ok(FaceRecord {
                    id: row.get(0)?,
                    photo_id: row.get(1)?,
                    photo_path: row.get(2)?,
                    person_id: row.get(3)?,
                    bbox: row.get(4)?,
                    confidence: row.get(5)?,
                    thumbnail_b64: row.get(6)?,
                    source_revision: row.get(7)?,
                    frame_ms: row.get(8)?,
                    frame_source: row.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut res = Vec::new();
        for f in iter {
            res.push(f.map_err(|e| e.to_string())?);
        }
        Ok(res)
    } else {
        Err("DB not initialized".into())
    }
}

#[tauri::command]
pub async fn rename_person(
    person_id: i64,
    new_name: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if new_name.trim().is_empty() {
            return Err("Name cannot be empty".into());
        }
        let _guard = state.indexer.gate.lock().unwrap();
        let mut c = state.db_conn.lock().unwrap();
        let result = state.indexer.rename_person(
            c.as_mut().ok_or("DB not initialized")?,
            person_id,
            new_name.trim(),
        );
        drop(c);
        drop(_guard);
        if result.is_ok() {
            crate::face_worker::schedule(&app);
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

fn face_record(row: &rusqlite::Row) -> rusqlite::Result<FaceRecord> {
    Ok(FaceRecord {
        id: row.get(0)?,
        photo_id: row.get(1)?,
        photo_path: row.get(2)?,
        person_id: row.get(3)?,
        bbox: row.get(4)?,
        confidence: row.get(5)?,
        thumbnail_b64: row.get(6)?,
        source_revision: row.get(7)?,
        frame_ms: row.get(8)?,
        frame_source: row.get(9)?,
    })
}

const FACE_RECORD: &str = "SELECT f.id, f.photo_id, ph.path, f.person_id, f.bbox, f.confidence, f.thumbnail, COALESCE(ph.fs_mtime_ns,'')||':'||COALESCE(ph.size,0),f.frame_ms,f.frame_source
             FROM faces f
             JOIN photos ph ON f.photo_id = ph.id";

/// Faces whose best-vs-second-best margin is below this are worth a second look.
const UNCERTAIN_MARGIN: f32 = 0.2;

#[tauri::command]
pub async fn get_uncertain_faces(app: tauri::AppHandle) -> Result<Vec<FaceRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let guard = state.db_conn.lock().unwrap();
        let conn = guard.as_ref().ok_or("DB not initialized")?;
        if let Some(scored) = score_faces(&app, conn, None)? {
            let mut res = Vec::new();
            let mut stmt = conn
                .prepare(&format!("{FACE_RECORD} WHERE f.id=?1"))
                .map_err(|e| e.to_string())?;
            for face in scored.iter().filter(|f| f.margin < UNCERTAIN_MARGIN).take(50) {
                if let Some(record) = stmt
                    .query_row([face.face_id], face_record)
                    .optional()
                    .map_err(|e| e.to_string())?
                {
                    res.push(record);
                }
            }
            return Ok(res);
        }
        let mut stmt = conn.prepare(&format!(
            "{FACE_RECORD}
             WHERE f.retired=0 AND ph.available=1 AND ph.excluded=0 AND f.person_id IS NOT NULL AND f.assignment_source IN ('automatic','cluster') AND f.confidence < 0.8
             ORDER BY f.confidence ASC
             LIMIT 50"
        )).map_err(|e| e.to_string())?;
        let res = stmt
            .query_map([], face_record)
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string());
        res
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn confirm_face(
    face_id: i64,
    person_id: i64,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.indexer.gate.lock().unwrap();
        let mut c = state.db_conn.lock().unwrap();
        let result = state.indexer.review_face(
            c.as_mut().ok_or("DB not initialized")?,
            face_id,
            person_id,
            false,
        );
        drop(c);
        drop(_guard);
        if result.is_ok() {
            crate::face_worker::schedule(&app);
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn reject_face(
    face_id: i64,
    person_id: i64,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.indexer.gate.lock().unwrap();
        let mut c = state.db_conn.lock().unwrap();
        let result = state.indexer.review_face(
            c.as_mut().ok_or("DB not initialized")?,
            face_id,
            person_id,
            true,
        );
        drop(c);
        drop(_guard);
        if result.is_ok() {
            crate::face_worker::schedule(&app);
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn assign_face(
    face_id: i64,
    person_id: Option<i64>,
    new_name: Option<String>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let guard = state.indexer.gate.lock().unwrap();
        let mut catalog = state.db_conn.lock().unwrap();
        let c = catalog.as_mut().ok_or("Catalog unavailable")?;
        let person = match person_id {
            Some(id) => id,
            None => {
                let name = new_name.as_deref().unwrap_or("").trim();
                if name.is_empty() { return Err("Enter a name or choose a person".into()); }
                let folder: String = c.query_row("SELECT p.folder_path FROM photos p JOIN faces f ON f.photo_id=p.id WHERE f.id=?1", [face_id], |r|r.get(0)).map_err(|e|e.to_string())?;
                let local = crate::indexing::open_folder(std::path::Path::new(&folder))?;
                let uuid: String = local.query_row("SELECT lower(hex(randomblob(16)))", [], |r|r.get(0)).map_err(|e|e.to_string())?;
                local.execute("INSERT INTO people(name,portable_id,user_named) VALUES(?1,?2,1)", rusqlite::params![name,uuid]).map_err(|e|e.to_string())?;
                state.indexer.project(c,std::path::Path::new(&folder))?;
                c.query_row("SELECT id FROM people WHERE portable_id=?1",[uuid],|r|r.get(0)).map_err(|e|e.to_string())?
            }
        };
        state.indexer.review_face(c,face_id,person,false)?;
        drop(catalog); drop(guard);
        crate::face_worker::schedule(&app);
        Ok(())
    }).await.map_err(|e|e.to_string())?
}

#[derive(Serialize)]
pub struct FaceRef {
    pub face_id: i64,
    pub photo_id: i64,
    pub photo_path: String,
    pub bbox: String,
    pub revision: String,
    pub frame_ms: Option<i64>,
    pub frame_source: Option<String>,
    pub photo_width: Option<i64>,
    pub photo_height: Option<i64>,
    pub quality: Option<f64>,
}

#[derive(Serialize)]
pub struct PhotoRef {
    pub photo_id: i64,
    pub path: String,
    pub revision: String,
    pub thumbhash: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

#[derive(Serialize)]
pub struct PersonCard {
    pub id: i64,
    pub name: String,
    pub user_named: bool,
    pub photo_count: i64,
    pub latest_taken_at: Option<i64>,
    pub cover: Option<FaceRef>,
    pub context: Vec<PhotoRef>,
}

#[derive(Serialize)]
pub struct ReviewFace {
    pub face_id: i64,
    pub photo_id: i64,
    pub photo_path: String,
    pub bbox: String,
    pub revision: String,
    pub frame_ms: Option<i64>,
    pub frame_source: Option<String>,
    pub person_id: Option<i64>,
    pub person_name: Option<String>,
    pub score: f64,
    pub margin: f64,
}

#[derive(Serialize)]
pub struct MergeSuggestion {
    pub a_id: i64,
    pub a_name: String,
    pub b_id: i64,
    pub b_name: String,
    pub score: f64,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

const VISIBLE_FACE: &str = "f.retired=0 AND ph.available=1 AND ph.excluded=0";
const REVISION: &str = "COALESCE(ph.fs_mtime_ns,'')||':'||COALESCE(ph.size,0)";
/// Candidate-face budget per scoring pass, so review stays interactive on large libraries.
const MAX_SCORED: i64 = 2000;

#[tauri::command]
pub async fn get_people_cards(app: tauri::AppHandle) -> Result<Vec<PersonCard>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let guard = state.db_conn.lock().unwrap();
        let conn = guard.as_ref().ok_or("DB not initialized")?;
        let people: Vec<(i64, String, bool, i64, Option<i64>)> = conn
            .prepare(&format!(
                "SELECT p.id,p.name,COALESCE(p.user_named,0),COUNT(DISTINCT ph.id),MAX(COALESCE(ph.taken_at,ph.mtime))
                 FROM people p LEFT JOIN faces f ON f.person_id=p.id AND f.retired=0
                 LEFT JOIN photos ph ON ph.id=f.photo_id AND ph.available=1 AND ph.excluded=0
                 GROUP BY p.id
                 HAVING COUNT(ph.id)>0 OR EXISTS(SELECT 1 FROM faces old WHERE old.person_id=p.id AND old.retired<>1)
                 ORDER BY COUNT(DISTINCT ph.id) DESC,p.id"
            ))
            .map_err(text)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map_err(text)?
            .collect::<rusqlite::Result<_>>()
            .map_err(text)?;
        let mut cover_q = conn
            .prepare(&format!(
                "SELECT f.id,f.photo_id,ph.path,f.bbox,{REVISION},f.frame_ms,f.frame_source,ph.width,ph.height,f.quality
                 FROM faces f JOIN photos ph ON ph.id=f.photo_id
                 WHERE f.person_id=?1 AND {VISIBLE_FACE}
                 ORDER BY f.quality IS NULL,f.quality DESC,f.confidence DESC,f.id LIMIT 1"
            ))
            .map_err(text)?;
        let mut context_q = conn
            .prepare(&format!(
                "SELECT ph.id,ph.path,{REVISION},ph.thumbhash,ph.width,ph.height FROM photos ph
                 WHERE ph.id IN (SELECT f.photo_id FROM faces f WHERE f.person_id=?1 AND f.retired=0)
                 AND ph.id IS NOT ?2 AND ph.available=1 AND ph.excluded=0
                 ORDER BY COALESCE(ph.taken_at,ph.mtime) DESC,ph.id DESC LIMIT 2"
            ))
            .map_err(text)?;
        let mut cards = Vec::with_capacity(people.len());
        for (id, name, user_named, photo_count, latest_taken_at) in people {
            let cover = cover_q
                .query_row([id], |r| {
                    Ok(FaceRef {
                        face_id: r.get(0)?,
                        photo_id: r.get(1)?,
                        photo_path: r.get(2)?,
                        bbox: r.get(3)?,
                        revision: r.get(4)?,
                        frame_ms: r.get(5)?,
                        frame_source: r.get(6)?,
                        photo_width: r.get(7)?,
                        photo_height: r.get(8)?,
                        quality: r.get(9)?,
                    })
                })
                .optional()
                .map_err(text)?;
            let context = context_q
                .query_map(rusqlite::params![id, cover.as_ref().map(|c| c.photo_id)], |r| {
                    Ok(PhotoRef {
                        photo_id: r.get(0)?,
                        path: r.get(1)?,
                        revision: r.get(2)?,
                        thumbhash: r.get(3)?,
                        width: r.get(4)?,
                        height: r.get(5)?,
                    })
                })
                .map_err(text)?
                .collect::<rusqlite::Result<_>>()
                .map_err(text)?;
            cards.push(PersonCard {
                id,
                name,
                user_named,
                photo_count,
                latest_taken_at,
                cover,
                context,
            });
        }
        Ok(cards)
    })
    .await
    .map_err(text)?
}

/// The embedding space to compare in: the worker's current one when the catalog holds it, else the most common.
fn space(app: &tauri::AppHandle, c: &Connection) -> Result<Option<String>, String> {
    if c.query_row(
        "SELECT 1 FROM sqlite_master WHERE name='recognition_vectors'",
        [],
        |_| Ok(()),
    )
    .optional()
    .map_err(text)?
    .is_none()
    {
        return Ok(None);
    }
    if let Some(version) = crate::face_worker::current_version(app) {
        if c.query_row(
            "SELECT 1 FROM recognition_vectors WHERE model_version=?1 LIMIT 1",
            [&version],
            |_| Ok(()),
        )
        .optional()
        .map_err(text)?
        .is_some()
        {
            return Ok(Some(version));
        }
    }
    c.query_row(
        "SELECT model_version FROM recognition_vectors GROUP BY model_version ORDER BY COUNT(*) DESC LIMIT 1",
        [],
        |r| r.get(0),
    )
    .optional()
    .map_err(text)
}

/// Per-person sums of embeddings. Quality-gated faces define the centroid when a person has any.
struct Centroid {
    good: Vec<f32>,
    good_count: usize,
    all: Vec<f32>,
    count: usize,
}
impl Centroid {
    fn uses_good(&self) -> bool {
        self.good_count > 0
    }
    fn sum(&self) -> (&[f32], usize) {
        if self.uses_good() {
            (&self.good, self.good_count)
        } else {
            (&self.all, self.count)
        }
    }
}
fn centroids(c: &Connection, version: &str) -> Result<HashMap<i64, Centroid>, String> {
    let mut q = c
        .prepare(&format!(
            "SELECT f.person_id,v.vector,{} FROM recognition_vectors v JOIN faces f ON f.source_key=v.source_key JOIN photos ph ON ph.id=f.photo_id
             WHERE v.model_version=?1 AND f.person_id IS NOT NULL AND {VISIBLE_FACE}",
            eligible_sql()
        ))
        .map_err(text)?;
    let rows = q
        .query_map([version], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, bool>(2)?))
        })
        .map_err(text)?;
    let mut res: HashMap<i64, Centroid> = HashMap::new();
    for row in rows {
        let (person, blob, good) = row.map_err(text)?;
        let Some(v) = vector(blob) else { continue };
        let entry = res.entry(person).or_insert_with(|| Centroid {
            good: vec![0.; v.len()],
            good_count: 0,
            all: vec![0.; v.len()],
            count: 0,
        });
        for (a, b) in entry.all.iter_mut().zip(&v) {
            *a += b;
        }
        entry.count += 1;
        if good {
            for (a, b) in entry.good.iter_mut().zip(&v) {
                *a += b;
            }
            entry.good_count += 1;
        }
    }
    Ok(res)
}

struct Scored {
    face_id: i64,
    score: f32,
    margin: f32,
}

/// Scores assigned, not-yet-confirmed faces against their own person (leave-one-out centroid)
/// and the best other person. Lowest margin first. `None` when no embeddings are available.
fn score_faces(
    app: &tauri::AppHandle,
    c: &Connection,
    person: Option<i64>,
) -> Result<Option<Vec<Scored>>, String> {
    let Some(version) = space(app, c)? else {
        return Ok(None);
    };
    let sums = centroids(c, &version)?;
    let means: Vec<(i64, Vec<f32>)> = sums
        .iter()
        .filter_map(|(id, centroid)| normalized(centroid.sum().0).ok().map(|m| (*id, m)))
        .collect();
    let mut q = c
        .prepare(&format!(
            "SELECT f.id,f.person_id,v.vector,{} FROM recognition_vectors v JOIN faces f ON f.source_key=v.source_key JOIN photos ph ON ph.id=f.photo_id
             WHERE v.model_version=?1 AND {VISIBLE_FACE} AND f.person_id IS NOT NULL
             AND (f.person_id=?2 OR (?2 IS NULL AND f.person_id IN (SELECT id FROM people WHERE user_named=1)))
             AND f.assignment_source IN ('automatic','cluster','user_named_group')
             ORDER BY f.confidence ASC,f.id DESC LIMIT ?3",
            eligible_sql()
        ))
        .map_err(text)?;
    let rows = q
        .query_map(rusqlite::params![version, person, MAX_SCORED], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })
        .map_err(text)?;
    let mut res = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        let (face_id, owner, blob, good) = row.map_err(text)?;
        let Some(v) = vector(blob) else { continue };
        if !seen.insert(face_id) {
            continue;
        }
        // Leave this face out of its own person's centroid so it cannot vouch for itself.
        let score = sums
            .get(&owner)
            .and_then(|centroid| {
                let (sum, count) = centroid.sum();
                if good || !centroid.uses_good() {
                    if count <= 1 {
                        return None;
                    }
                    let rest: Vec<f32> = sum.iter().zip(&v).map(|(a, b)| a - b).collect();
                    normalized(&rest).ok()
                } else {
                    normalized(sum).ok()
                }
            })
            .map(|m| dot(&v, &m))
            .unwrap_or(0.);
        let second = means
            .iter()
            .filter(|(id, _)| *id != owner)
            .map(|(_, m)| dot(&v, m))
            .fold(0., f32::max);
        res.push(Scored {
            face_id,
            score,
            margin: score - second,
        });
    }
    res.sort_by(|a, b| a.margin.total_cmp(&b.margin));
    Ok(Some(res))
}

fn review_face_row(
    c: &Connection,
    face: i64,
    score: f64,
    margin: f64,
) -> Result<Option<ReviewFace>, String> {
    c.query_row(
        &format!(
            "SELECT f.id,f.photo_id,ph.path,f.bbox,{REVISION},f.frame_ms,f.frame_source,f.person_id,pe.name
             FROM faces f JOIN photos ph ON ph.id=f.photo_id LEFT JOIN people pe ON pe.id=f.person_id WHERE f.id=?1"
        ),
        [face],
        |r| {
            Ok(ReviewFace {
                face_id: r.get(0)?,
                photo_id: r.get(1)?,
                photo_path: r.get(2)?,
                bbox: r.get(3)?,
                revision: r.get(4)?,
                frame_ms: r.get(5)?,
                frame_source: r.get(6)?,
                person_id: r.get(7)?,
                person_name: r.get(8)?,
                score,
                margin,
            })
        },
    )
    .optional()
    .map_err(text)
}

#[tauri::command]
pub async fn get_review_faces(
    person_id: Option<i64>,
    limit: i64,
    app: tauri::AppHandle,
) -> Result<Vec<ReviewFace>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let guard = state.db_conn.lock().unwrap();
        let conn = guard.as_ref().ok_or("DB not initialized")?;
        let limit = limit.clamp(0, 500) as usize;
        let mut res = Vec::new();
        if let Some(scored) = score_faces(&app, conn, person_id)?.filter(|s| !s.is_empty()) {
            for face in scored {
                if res.len() >= limit {
                    break;
                }
                if let Some(row) =
                    review_face_row(conn, face.face_id, face.score as f64, face.margin as f64)?
                {
                    res.push(row);
                }
            }
            return Ok(res);
        }
        // No embeddings in the catalog yet: fall back to the stored assignment confidence.
        let ids: Vec<(i64, f64)> = conn
            .prepare(&format!(
                "SELECT f.id,COALESCE(f.confidence,0) FROM faces f JOIN photos ph ON ph.id=f.photo_id
                 WHERE {VISIBLE_FACE} AND f.person_id IS NOT NULL
                 AND (f.person_id=?1 OR (?1 IS NULL AND f.person_id IN (SELECT id FROM people WHERE user_named=1)))
                 AND f.assignment_source IN ('automatic','cluster','user_named_group')
                 ORDER BY f.confidence ASC,f.id DESC LIMIT ?2"
            ))
            .map_err(text)?
            .query_map(rusqlite::params![person_id, limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(text)?
            .collect::<rusqlite::Result<_>>()
            .map_err(text)?;
        for (id, confidence) in ids {
            if let Some(row) = review_face_row(conn, id, confidence, confidence)? {
                res.push(row);
            }
        }
        Ok(res)
    })
    .await
    .map_err(text)?
}

#[tauri::command]
pub async fn batch_review_faces(
    person_id: i64,
    confirm: Vec<i64>,
    reject: Vec<i64>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let guard = state.indexer.gate.lock().unwrap();
        let mut c = state.db_conn.lock().unwrap();
        let catalog = c.as_mut().ok_or("DB not initialized")?;
        let reviews: Vec<(i64, i64, bool)> = confirm
            .iter()
            .map(|f| (*f, person_id, false))
            .chain(reject.iter().map(|f| (*f, person_id, true)))
            .collect();
        let result = state.indexer.review_faces(catalog, &reviews);
        drop(c);
        drop(guard);
        // Earlier reviews in the batch are already saved; re-run recognition for them either way.
        crate::face_worker::schedule(&app);
        result
    })
    .await
    .map_err(text)?
}

#[tauri::command]
pub async fn merge_people(source_id: i64, target_id: i64, app: tauri::AppHandle) -> Result<(), String> {
    if source_id == target_id {
        return Err("Choose two different people".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let guard = state.indexer.gate.lock().unwrap();
        let mut c = state.db_conn.lock().unwrap();
        let catalog = c.as_mut().ok_or("DB not initialized")?;
        let person = |id: i64| {
            catalog
                .query_row(
                    "SELECT name,COALESCE(user_named,0) FROM people WHERE id=?1",
                    [id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
                )
                .map_err(text)
        };
        let (source_name, source_named) = person(source_id)?;
        let (_, target_named) = person(target_id)?;
        let faces: Vec<i64> = catalog
            .prepare("SELECT id FROM faces WHERE person_id=?1 AND retired=0 ORDER BY id")
            .map_err(text)?
            .query_map([source_id], |r| r.get(0))
            .map_err(text)?
            .collect::<rusqlite::Result<_>>()
            .map_err(text)?;
        let result = (|| {
            // Name first, so the moved faces are recorded against a named identity.
            if source_named && !target_named {
                state.indexer.rename_person(catalog, target_id, &source_name)?;
            }
            let reviews: Vec<(i64, i64, bool)> = faces.iter().map(|f| (*f, target_id, false)).collect();
            state.indexer.review_faces(catalog, &reviews)
        })();
        drop(c);
        drop(guard);
        crate::face_worker::schedule(&app);
        result
    })
    .await
    .map_err(text)?
}

#[tauri::command]
pub async fn get_merge_suggestions(
    limit: i64,
    app: tauri::AppHandle,
) -> Result<Vec<MergeSuggestion>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let guard = state.db_conn.lock().unwrap();
        let conn = guard.as_ref().ok_or("DB not initialized")?;
        let Some(version) = space(&app, conn)? else {
            return Ok(vec![]);
        };
        let names: HashMap<i64, (String, bool)> = conn
            .prepare("SELECT id,name,COALESCE(user_named,0) FROM people")
            .map_err(text)?
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
            .map_err(text)?
            .collect::<rusqlite::Result<_>>()
            .map_err(text)?;
        let mut people: Vec<(i64, usize, Vec<f32>)> = centroids(conn, &version)?
            .into_iter()
            .filter(|(id, c)| c.count >= 2 && names.contains_key(id))
            .filter_map(|(id, c)| normalized(c.sum().0).ok().map(|m| (id, c.count, m)))
            .collect();
        // Bound the pairwise pass on very large libraries to the most-photographed people.
        people.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        people.truncate(1500);
        let mut pairs = Vec::new();
        for (i, a) in people.iter().enumerate() {
            for b in &people[i + 1..] {
                let score = dot(&a.2, &b.2);
                if score <= 0.5 {
                    continue;
                }
                let (an, a_named) = &names[&a.0];
                let (bn, b_named) = &names[&b.0];
                // Two people the user named differently are a deliberate distinction.
                if *a_named && *b_named && an != bn {
                    continue;
                }
                pairs.push((score, a.0, b.0));
            }
        }
        pairs.sort_by(|x, y| y.0.total_cmp(&x.0));
        pairs.truncate(limit.clamp(0, 200) as usize);
        Ok(pairs
            .into_iter()
            .map(|(score, a, b)| MergeSuggestion {
                a_id: a,
                a_name: names[&a].0.clone(),
                b_id: b,
                b_name: names[&b].0.clone(),
                score: score as f64,
            })
            .collect())
    })
    .await
    .map_err(text)?
}
