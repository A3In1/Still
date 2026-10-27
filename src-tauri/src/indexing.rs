//! Portable folder records are authoritative. The app-cache DB is only a projection.
use crate::{db, scanner};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const PARSER_VERSION: i64 = 3;
/// Bump when project() copies new columns; every folder is then projected in full once.
const PROJECTION: i64 = 2;

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub struct Indexer {
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub gate: Mutex<()>,
}

#[derive(Default, Serialize, Debug)]
pub struct ScanReport {
    pub checked: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub missing: usize,
    pub folders_refreshed: usize,
    pub errors: Vec<String>,
}

pub fn open_folder(dir: &Path) -> Result<Connection> {
    if !dir.is_dir() {
        return Err(format!("Folder offline: {}", dir.display()));
    }
    let data = dir.join(".data");
    if data.is_symlink() {
        return Err("Refusing a symlinked .data directory".into());
    }
    std::fs::create_dir_all(&data).map_err(err)?;
    let path = data.join("library.db");
    if path.is_symlink() {
        return Err("Refusing a symlinked folder database".into());
    }
    let conn = db::init_db(path.to_str().ok_or("Non-UTF8 database path")?).map_err(err)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS folder_meta (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1), folder_id TEXT NOT NULL, version INTEGER NOT NULL);
        INSERT OR IGNORE INTO folder_meta(singleton,folder_id,version) VALUES(1,lower(hex(randomblob(16))),1);
        CREATE TABLE IF NOT EXISTS file_state (
            photo_id INTEGER PRIMARY KEY REFERENCES photos(id), mtime_ns TEXT NOT NULL,
            size INTEGER NOT NULL, generation INTEGER NOT NULL, parser_version INTEGER NOT NULL,
            file_key TEXT);
        CREATE TABLE IF NOT EXISTS scan_state (singleton INTEGER PRIMARY KEY, generation INTEGER NOT NULL);
        INSERT OR IGNORE INTO scan_state VALUES(1,0);
        CREATE TABLE IF NOT EXISTS face_embeddings (
            face_id INTEGER NOT NULL REFERENCES faces(id), model_version TEXT NOT NULL,
            preprocessing_version TEXT NOT NULL, dimensions INTEGER NOT NULL, vector BLOB NOT NULL,
            PRIMARY KEY(face_id,model_version,preprocessing_version));
        CREATE TABLE IF NOT EXISTS face_rejections (face_uuid TEXT NOT NULL, person_uuid TEXT NOT NULL,
            PRIMARY KEY(face_uuid,person_uuid));") .map_err(err)?;
    let version: i64 = conn
        .query_row("SELECT version FROM folder_meta", [], |r| r.get(0))
        .map_err(err)?;
    if version != 1 {
        return Err(format!("Unsupported folder database version {version}"));
    }
    let columns = conn
        .prepare("PRAGMA table_info(folder_meta)")
        .map_err(err)?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(err)?;
    for (column, definition) in [
        ("revision", "INTEGER NOT NULL DEFAULT 1"),
        ("full_rev", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !columns.iter().any(|c| c == column) {
            conn.execute(
                &format!("ALTER TABLE folder_meta ADD COLUMN {column} {definition}"),
                [],
            )
            .map_err(err)?;
        }
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS face_state(
        photo_id INTEGER PRIMARY KEY REFERENCES photos(id), input_revision TEXT NOT NULL,
        model_version TEXT NOT NULL, status TEXT NOT NULL, pixel_digest TEXT,
        error TEXT, attempts INTEGER NOT NULL DEFAULT 0, retry_after INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX IF NOT EXISTS face_state_due ON face_state(status,retry_after);
        CREATE INDEX IF NOT EXISTS photos_recent ON photos(COALESCE(taken_at,mtime) DESC, id DESC);",
    )
    .map_err(err)?;
    change_tracking(&conn)?;
    for (column, definition) in [
        ("decoder_version", "INTEGER NOT NULL DEFAULT 1"),
        ("scan_depth", "TEXT NOT NULL DEFAULT 'still'"),
    ] {
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(face_state)")
            .map_err(err)?
            .query_map([], |r| r.get(1))
            .map_err(err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(err)?;
        if !columns.iter().any(|c| c == column) {
            conn.execute(
                &format!("ALTER TABLE face_state ADD COLUMN {column} {definition}"),
                [],
            )
            .map_err(err)?;
        }
    }
    conn.execute_batch(
        "PRAGMA temp_store=FILE; CREATE TEMP TABLE seen(photo_id INTEGER PRIMARY KEY)",
    )
    .map_err(err)?;
    Ok(conn)
}

/// Rows carry the folder revision of their last change so projection can be incremental.
/// Deletes cannot be replayed from rows; they set `full_rev` to force one full projection.
fn change_tracking(conn: &Connection) -> Result<()> {
    let current = |c: &Connection| -> Result<bool> {
        c.query_row(
            "SELECT COUNT(*)=15 FROM sqlite_master WHERE type='trigger' AND name LIKE 'rev\\_%' ESCAPE '\\'
            AND (sql LIKE '%changed_rev%' OR sql LIKE '%full_rev%')",
            [],
            |r| r.get(0),
        )
        .map_err(err)
    };
    if current(conn)? {
        return Ok(());
    }
    conn.execute_batch("BEGIN IMMEDIATE").map_err(err)?;
    let result = (|| -> Result<()> {
        if current(conn)? {
            return Ok(());
        }
        let bump = "UPDATE folder_meta SET revision=revision+1";
        let mark = |table: &str, id: &str| {
            format!("UPDATE {table} SET changed_rev=(SELECT revision FROM folder_meta) WHERE id={id}")
        };
        let mut sql = String::new();
        for table in ["photos", "people", "faces"] {
            let columns: Vec<String> = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .map_err(err)?
                .query_map([], |r| r.get(1))
                .map_err(err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(err)?;
            if !columns.iter().any(|c| c == "changed_rev") {
                sql += &format!("ALTER TABLE {table} ADD COLUMN changed_rev INTEGER;");
            }
            let m = mark(table, "NEW.id");
            // Marking a row is itself an UPDATE; the WHEN clause keeps it from re-bumping.
            sql += &format!(
                "CREATE INDEX IF NOT EXISTS {table}_changed ON {table}(changed_rev);
                DROP TRIGGER IF EXISTS rev_{table}_INSERT; DROP TRIGGER IF EXISTS rev_{table}_UPDATE; DROP TRIGGER IF EXISTS rev_{table}_DELETE;
                CREATE TRIGGER rev_{table}_INSERT AFTER INSERT ON {table} BEGIN {bump}; {m}; END;
                CREATE TRIGGER rev_{table}_UPDATE AFTER UPDATE ON {table} WHEN NEW.changed_rev IS OLD.changed_rev BEGIN {bump}; {m}; END;
                CREATE TRIGGER rev_{table}_DELETE AFTER DELETE ON {table} BEGIN {bump},full_rev=revision+1; END;"
            );
        }
        // Face results reach recognition vectors through their photo and face rows.
        for (table, target, key) in [
            ("face_state", "photos", "photo_id"),
            ("face_embeddings", "faces", "face_id"),
        ] {
            for (operation, row) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")] {
                let m = mark(target, &format!("{row}.{key}"));
                sql += &format!(
                    "DROP TRIGGER IF EXISTS rev_{table}_{operation};
                    CREATE TRIGGER rev_{table}_{operation} AFTER {operation} ON {table} BEGIN {bump}; {m}; END;"
                );
            }
        }
        conn.execute_batch(&sql).map_err(err)
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(err),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

pub const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september",
    "october", "november", "december",
];

/// Catalog full-text index; rowid is photos.id. Rows exist for every catalog photo and
/// queries filter visibility, so availability changes never need a search rewrite.
pub fn prepare_search(c: &Connection) -> Result<()> {
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE name='photo_search')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if exists {
        return Ok(());
    }
    c.execute_batch("CREATE VIRTUAL TABLE main.photo_search USING fts5(path_tokens, folder, city, country, people_names, date_words, tokenize='unicode61 remove_diacritics 2')")
        .map_err(err)?;
    reindex_search(c, "true", &[])
}

/// Rewrites the search rows of catalog photos matching `filter` (a WHERE over main.photos).
pub fn reindex_search(c: &Connection, filter: &str, p: &[&dyn rusqlite::ToSql]) -> Result<()> {
    let ts = "COALESCE(taken_at,mtime),'unixepoch','localtime'";
    let months: String = MONTHS
        .iter()
        .enumerate()
        .map(|(i, m)| format!(" WHEN {} THEN '{m} {}'", i + 1, &m[..3]))
        .collect();
    c.execute(
        &format!("DELETE FROM main.photo_search WHERE rowid IN (SELECT id FROM main.photos WHERE {filter})"),
        p,
    )
    .map_err(err)?;
    c.execute(&format!("INSERT INTO main.photo_search(rowid,path_tokens,folder,city,country,people_names,date_words)
        SELECT id,path,substr(folder_path,length(rtrim(folder_path,replace(folder_path,'/','')))+1),city,country,
        (SELECT group_concat(name,' ') FROM (SELECT DISTINCT pe.name FROM main.faces f JOIN main.people pe ON pe.id=f.person_id
            WHERE f.photo_id=photos.id AND f.retired=0 AND pe.user_named=1)),
        (CASE CAST(strftime('%m',{ts}) AS INTEGER){months} ELSE '' END)||' '||COALESCE(strftime('%Y',{ts}),'')
        FROM main.photos WHERE {filter}"), p).map_err(err)?;
    Ok(())
}

pub fn stat(path: &Path) -> Result<(String, i64, String)> {
    let m = std::fs::metadata(path).map_err(err)?;
    let t = m
        .modified()
        .map_err(err)?
        .duration_since(UNIX_EPOCH)
        .map_err(err)?;
    #[cfg(unix)]
    let key = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", m.dev(), m.ino())
    };
    #[cfg(not(unix))]
    let key = String::new();
    Ok((t.as_nanos().to_string(), m.len() as i64, key))
}

impl Indexer {
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir).map_err(err)?;
        std::fs::create_dir_all(cache_dir.join(scanner::THUMB_DIR)).map_err(err)?;
        let legacy = [cache_dir.join("thumbnails"), cache_dir.join("thumbs-v3")];
        std::thread::spawn(move || {
            for directory in legacy {
                let _ = std::fs::remove_dir_all(directory);
            }
        });
        let this = Self {
            data_dir,
            cache_dir,
            gate: Mutex::new(()),
        };
        let settings = this.settings()?;
        settings.execute_batch("CREATE TABLE IF NOT EXISTS roots(path TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS pending_names(person_uuid TEXT PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS pending_faces(face_uuid TEXT PRIMARY KEY, folder TEXT NOT NULL,
                person_uuid TEXT NOT NULL, name TEXT NOT NULL, rejected INTEGER NOT NULL);") .map_err(err)?;
        Ok(this)
    }
    fn settings(&self) -> Result<Connection> {
        let c = Connection::open(self.data_dir.join("settings.db")).map_err(err)?;
        c.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        Ok(c)
    }
    pub fn roots(&self) -> Result<Vec<String>> {
        self.settings()?
            .prepare("SELECT path FROM roots ORDER BY path")
            .map_err(err)?
            .query_map([], |r| r.get(0))
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)
    }
    pub fn register(&self, path: &str) -> Result<PathBuf> {
        let p = std::fs::canonicalize(path).map_err(err)?;
        if p.starts_with(&self.cache_dir)
            || p.starts_with(&self.data_dir)
            || !p.is_dir()
            || p.components().any(|c| c.as_os_str() == ".data")
        {
            return Err("Choose a photo directory, not an index directory".into());
        }
        self.settings()?
            .execute(
                "INSERT OR IGNORE INTO roots VALUES(?1)",
                [p.to_str().ok_or("Non-UTF8 folder")?],
            )
            .map_err(err)?;
        Ok(p)
    }
    pub fn project(&self, catalog: &mut Connection, dir: &Path) -> Result<bool> {
        // Replay edits before publishing worker results, including names saved during inference.
        let local = open_folder(dir)?;
        self.apply_pending(&local, dir)?;
        drop(local);
        let folder = dir.to_str().ok_or("Non-UTF8 folder")?;
        prepare_search(catalog)?;
        catalog
            .execute(
                "ATTACH DATABASE ?1 AS folder",
                [dir.join(".data/library.db").to_str().unwrap()],
            )
            .map_err(err)?;
        let result = (|| {
            catalog.execute_batch("CREATE TABLE IF NOT EXISTS folder_sources(path TEXT PRIMARY KEY, folder_id TEXT NOT NULL, revision INTEGER NOT NULL)").map_err(err)?;
            let has_version = catalog
                .prepare("PRAGMA table_info(folder_sources)")
                .map_err(err)?
                .query_map([], |r| r.get::<_, String>(1))
                .map_err(err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(err)?
                .iter()
                .any(|c| c == "projection");
            if !has_version {
                catalog
                    .execute_batch("ALTER TABLE folder_sources ADD COLUMN projection INTEGER NOT NULL DEFAULT 0")
                    .map_err(err)?;
            }
            let (folder_id, revision, full_rev): (String, i64, i64) = catalog
                .query_row(
                    "SELECT folder_id,revision,full_rev FROM folder.folder_meta",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(err)?;
            let previous: Option<(String, i64, i64)> = catalog
                .query_row(
                    "SELECT folder_id,revision,projection FROM folder_sources WHERE path=?1",
                    [folder],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(err)?;
            if previous == Some((folder_id.clone(), revision, PROJECTION)) {
                return Ok(false);
            }
            // Rows changed after the last projected revision; deletes, another folder identity,
            // a restored (older) database or a new projection format all need a full pass.
            let since = previous
                .filter(|p| {
                    p.0 == folder_id && p.2 == PROJECTION && p.1 <= revision && full_rev <= p.1
                })
                .map(|p| p.1);
            let old_locations: Vec<String> = catalog
                .prepare("SELECT path FROM folder_sources WHERE folder_id=?1 AND path<>?2")
                .map_err(err)?
                .query_map(params![folder_id, folder], |r| r.get(0))
                .map_err(err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(err)?;
            // Filters are integer literals, never user text.
            let changed = |alias: &str| match since {
                Some(n) => format!("{alias}.changed_rev>{n}"),
                None => "true".into(),
            };
            let face_rows = match since {
                Some(n) => format!("f.id IN (SELECT id FROM folder.faces WHERE changed_rev>{n}
                    UNION SELECT x.id FROM folder.faces x JOIN folder.photos y ON y.id=x.photo_id WHERE y.changed_rev>{n})"),
                None => "true".into(),
            };
            let vector_rows = match since {
                Some(n) => format!("f.id IN (SELECT id FROM folder.faces WHERE changed_rev>{n}
                    UNION SELECT x.id FROM folder.faces x JOIN folder.photos y ON y.id=x.photo_id WHERE y.changed_rev>{n}
                    UNION SELECT x.id FROM folder.faces x JOIN folder.people y ON y.id=x.person_id WHERE y.changed_rev>{n})"),
                None => "true".into(),
            };
            let tx = catalog.transaction().map_err(err)?;
            for old in old_locations {
                if matches!(Path::new(&old).try_exists(), Ok(false)) {
                    tx.execute("UPDATE photos SET available=0 WHERE folder_path=?1", [&old])
                        .map_err(err)?;
                    tx.execute("DELETE FROM folder_sources WHERE path=?1", [&old])
                        .map_err(err)?;
                }
            }
            tx.execute(
                &format!("INSERT INTO people(name,cluster_label,portable_id,user_named)
                SELECT name,cluster_label,portable_id,user_named FROM folder.people lpe WHERE {}
                ON CONFLICT(portable_id) DO UPDATE SET name=excluded.name,user_named=excluded.user_named", changed("lpe")),
                [],
            )
            .map_err(err)?;
            tx.execute(&format!("INSERT INTO photos(path,size,mtime,taken_at,city,country,lat,lng,width,height,media_type,duration,portable_id,folder_path,available,excluded,fs_mtime_ns)
                SELECT rtrim(?1,'/')||'/'||path,size,mtime,taken_at,city,country,lat,lng,width,height,media_type,duration,portable_id,?1,available,excluded,fs_mtime_ns FROM folder.photos lp WHERE {}
                ON CONFLICT(path) DO UPDATE SET size=excluded.size,mtime=excluded.mtime,taken_at=excluded.taken_at,
                city=excluded.city,country=excluded.country,lat=excluded.lat,lng=excluded.lng,width=excluded.width,height=excluded.height,
                media_type=excluded.media_type,duration=excluded.duration,portable_id=excluded.portable_id,
                folder_path=excluded.folder_path,available=excluded.available,excluded=excluded.excluded,fs_mtime_ns=excluded.fs_mtime_ns,
                thumb_path=CASE WHEN photos.fs_mtime_ns=excluded.fs_mtime_ns AND photos.size=excluded.size THEN photos.thumb_path ELSE NULL END,
                thumbhash=CASE WHEN photos.fs_mtime_ns=excluded.fs_mtime_ns AND photos.size=excluded.size THEN photos.thumbhash ELSE NULL END", changed("lp")), [folder]).map_err(err)?;
            tx.execute(&format!("UPDATE photos SET (media_format,media_subtype,pair_role,content_id,still_time_ms,motion_offset,motion_length,frame_count,anim_duration_ms,live_effect,key_time_ms,color_profile,hdr)=
                (lp.media_format,lp.media_subtype,lp.pair_role,lp.content_id,lp.still_time_ms,lp.motion_offset,lp.motion_length,lp.frame_count,lp.anim_duration_ms,lp.live_effect,lp.key_time_ms,lp.color_profile,lp.hdr)
                FROM folder.photos lp WHERE lp.portable_id=photos.portable_id AND photos.folder_path=?1 AND {}", changed("lp")), [folder]).map_err(err)?;
            // A partner's rename changes its catalog row, so either side changing re-resolves the pair.
            let pair_rows = match since {
                Some(n) => format!("(lp.changed_rev>{n} OR partner.changed_rev>{n})"),
                None => "true".into(),
            };
            tx.execute(&format!("UPDATE photos SET paired_id=cp.id FROM folder.photos lp LEFT JOIN folder.photos partner ON partner.id=lp.paired_id
                LEFT JOIN main.photos cp ON cp.path=rtrim(?1,'/')||'/'||partner.path
                WHERE lp.portable_id=photos.portable_id AND photos.folder_path=?1 AND {pair_rows}"), [folder]).map_err(err)?;
            // Renamed rows are projected at the new path; the old path remains a tombstone.
            let renamed = match since {
                Some(n) => format!("AND portable_id IN (SELECT portable_id FROM folder.photos WHERE changed_rev>{n})"),
                None => String::new(),
            };
            tx.execute(
                &format!("UPDATE photos SET available=0 WHERE folder_path=?1 AND available=1 {renamed} AND NOT EXISTS
                (SELECT 1 FROM folder.photos q WHERE q.path=substr(photos.path,length(rtrim(?1,'/'))+2))"),
                [folder],
            )
            .map_err(err)?;
            tx.execute(&format!("INSERT INTO faces(photo_id,person_id,bbox,confidence,thumbnail,portable_id,source_key,assignment_source,retired,detection_score,frame_ms,frame_source,quality,blur,face_px)
                SELECT cp.id,cperson.id,f.bbox,f.confidence,f.thumbnail,f.portable_id,rtrim(?1,'/')||'/'||f.portable_id,f.assignment_source,f.retired,f.detection_score,f.frame_ms,f.frame_source,f.quality,f.blur,f.face_px
                FROM folder.faces f JOIN folder.photos lp ON lp.id=f.photo_id
                JOIN main.photos cp ON cp.path=rtrim(?1,'/')||'/'||lp.path
                LEFT JOIN folder.people person ON person.id=f.person_id
                LEFT JOIN main.people cperson ON cperson.portable_id=person.portable_id WHERE {face_rows}
                ON CONFLICT(source_key) DO UPDATE SET photo_id=excluded.photo_id,person_id=excluded.person_id,
                bbox=excluded.bbox,confidence=excluded.confidence,assignment_source=excluded.assignment_source,retired=excluded.retired,detection_score=excluded.detection_score,frame_ms=excluded.frame_ms,frame_source=excluded.frame_source,
                quality=excluded.quality,blur=excluded.blur,face_px=excluded.face_px"), [folder]).map_err(err)?;
            tx.execute_batch("CREATE TABLE IF NOT EXISTS recognition_vectors(
                source_key TEXT PRIMARY KEY, folder_path TEXT NOT NULL, face_uuid TEXT NOT NULL,
                person_uuid TEXT, name TEXT, user_named INTEGER NOT NULL, source TEXT NOT NULL,
                model_version TEXT NOT NULL, vector BLOB NOT NULL);
                CREATE INDEX IF NOT EXISTS recognition_person ON recognition_vectors(person_uuid,source);
                CREATE TEMP TABLE IF NOT EXISTS search_dirty(id INTEGER PRIMARY KEY);
                DELETE FROM temp.search_dirty;") .map_err(err)?;
            if since.is_some() {
                tx.execute(&format!("DELETE FROM recognition_vectors WHERE source_key IN
                    (SELECT rtrim(?1,'/')||'/'||f.portable_id FROM folder.faces f WHERE {vector_rows})"), [folder]).map_err(err)?;
            } else {
                tx.execute(
                    "DELETE FROM recognition_vectors WHERE folder_path=?1",
                    [folder],
                )
                .map_err(err)?;
            }
            tx.execute(&format!("INSERT INTO recognition_vectors
                SELECT rtrim(?1,'/')||'/'||f.portable_id,?1,f.portable_id,p.portable_id,p.name,COALESCE(p.user_named,0),f.assignment_source,e.model_version,e.vector
                FROM folder.faces f JOIN folder.photos ph ON ph.id=f.photo_id
                JOIN folder.face_state s ON s.photo_id=ph.id
                JOIN folder.face_embeddings e ON e.face_id=f.id LEFT JOIN folder.people p ON p.id=f.person_id
                WHERE f.retired=0 AND ph.available=1 AND ph.excluded=0 AND s.input_revision=ph.fs_mtime_ns||':'||ph.size||COALESCE(':key:'||ph.key_time_ms,'')
                AND e.model_version=(SELECT model_version FROM folder.face_state WHERE photo_id=ph.id) AND {vector_rows}"), [folder]).map_err(err)?;
            // Search rows: changed photos (incl. rename tombstones) and photos whose people changed.
            if let Some(n) = since {
                tx.execute(&format!("INSERT OR IGNORE INTO temp.search_dirty SELECT id FROM main.photos WHERE folder_path=?1
                    AND portable_id IN (SELECT portable_id FROM folder.photos WHERE changed_rev>{n})"), [folder]).map_err(err)?;
                tx.execute(&format!("INSERT OR IGNORE INTO temp.search_dirty SELECT cf.photo_id FROM main.faces cf WHERE cf.source_key IN
                    (SELECT rtrim(?1,'/')||'/'||f.portable_id FROM folder.faces f WHERE {vector_rows})"), [folder]).map_err(err)?;
            } else {
                tx.execute("INSERT OR IGNORE INTO temp.search_dirty SELECT id FROM main.photos WHERE folder_path=?1", [folder]).map_err(err)?;
            }
            reindex_search(&tx, "id IN (SELECT id FROM temp.search_dirty)", &[])?;
            tx.execute("INSERT INTO folder_sources(path,folder_id,revision,projection) VALUES(?1,?2,?3,?4) ON CONFLICT(path) DO UPDATE SET folder_id=excluded.folder_id,revision=excluded.revision,projection=excluded.projection",params![folder,folder_id,revision,PROJECTION]).map_err(err)?;
            tx.commit().map_err(err)?;
            Ok(true)
        })();
        let detach = catalog.execute_batch("DETACH DATABASE folder").map_err(err);
        match (result, detach) {
            (Ok(changed), Ok(())) => Ok(changed),
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }
    pub fn scan(
        &self,
        root: &Path,
        catalog: &Mutex<Option<Connection>>,
        mut progress: impl FnMut(&ScanReport),
    ) -> ScanReport {
        // The gate is held per folder (and released between batches), never for a whole scan,
        // so face jobs and user edits on other folders make progress during long scans.
        let mut report = ScanReport::default();
        if !root.is_dir() {
            report
                .errors
                .push(format!("Folder offline or unavailable: {}", root.display()));
            return report;
        }
        // Stream traversal, with one direct-folder connection and one decoded image at a time.
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            match self.scan_folder(&dir, catalog, &mut stack, &mut report, &mut progress) {
                Ok(()) => {}
                Err(e) => report.errors.push(format!("{}: {e}", dir.display())),
            }
            progress(&report);
        }
        // A removed descendant directory is different from an unavailable registered root.
        // Only reconcile known descendants after successfully opening the root itself.
        if root.is_dir() {
            let _guard = self.gate.lock().unwrap();
            if let Some(c) = catalog.lock().unwrap().as_mut() {
                let result = (|| -> Result<()> {
                    let folders:Vec<String>=c.prepare("SELECT DISTINCT folder_path FROM photos WHERE folder_path IS NOT NULL AND available=1").map_err(err)?
                        .query_map([],|r|r.get(0)).map_err(err)?.collect::<rusqlite::Result<_>>().map_err(err)?;
                    for folder in folders {
                        let p = Path::new(&folder);
                        if p.starts_with(root) && matches!(p.try_exists(), Ok(false)) {
                            report.missing+=c.execute("UPDATE photos SET available=0 WHERE folder_path=?1 AND available=1",[&folder]).map_err(err)?;
                            c.execute("DELETE FROM folder_sources WHERE path=?1", [&folder])
                                .map_err(err)?;
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    report.errors.push(e);
                }
            }
        }
        report
    }
    fn scan_folder(
        &self,
        dir: &Path,
        catalog: &Mutex<Option<Connection>>,
        stack: &mut Vec<PathBuf>,
        report: &mut ScanReport,
        progress: &mut impl FnMut(&ScanReport),
    ) -> Result<()> {
        // Newest files first, so recent photos reach the timeline (and faces) before older ones.
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(err)?
            .map(|e| {
                let modified = e.as_ref().ok().and_then(|e| e.metadata().ok()).and_then(|m| m.modified().ok());
                (modified, e)
            })
            .collect();
        entries.sort_by(|a, b| b.0.cmp(&a.0));
        let mut subdirs = Vec::new();
        let mut gate = Some(self.gate.lock().unwrap());
        let mut local = if dir.join(".data/library.db").is_file() {
            Some(open_folder(dir)?)
        } else {
            None
        };
        let generation = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(err)?
            .as_nanos()
            .min(i64::MAX as u128) as i64;
        let mut complete = true;
        let mut batch_files = 0usize;
        let mut batch_indexed = 0usize;
        let mut batch_started = Instant::now();
        let mut last_projection = Instant::now();
        let mut projected_files = report.indexed;
        if let Some(c) = &local {
            self.apply_pending(c, dir)?;
        }
        let result = (|| -> Result<()> {
            for (_, entry) in entries {
                let first_projection = report.folders_refreshed == 0;
                let projection_due = report.indexed - projected_files
                    >= if first_projection { 25 } else { 500 }
                    || last_projection.elapsed()
                        >= if first_projection {
                            Duration::from_millis(300)
                        } else {
                            Duration::from_secs(2)
                        };
                let mut checkpoint = false;
                if let Some(c) = local.as_ref() {
                    if !c.is_autocommit()
                        && (batch_files >= 200
                            || batch_started.elapsed() >= Duration::from_millis(250)
                            || projection_due)
                    {
                        c.execute_batch("COMMIT").map_err(err)?;
                        batch_files = 0;
                        batch_indexed = 0;
                        checkpoint = true;
                    }
                    if projection_due {
                        // Publish committed rows; pairing still waits for complete folder
                        // enumeration so later same-stem files cannot create false pairs.
                        if let Some(catalog) = catalog.lock().unwrap().as_mut() {
                            if self.project(catalog, dir)? {
                                report.folders_refreshed += 1;
                            }
                        }
                        last_projection = Instant::now();
                        projected_files = report.indexed;
                        progress(report);
                        checkpoint = true;
                    }
                }
                if checkpoint {
                    // No transaction is open here; let a waiting face job or edit take the gate.
                    drop(gate.take());
                    std::thread::sleep(Duration::from_millis(1));
                    gate = Some(self.gate.lock().unwrap());
                }
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        complete = false;
                        report.errors.push(err(e));
                        continue;
                    }
                };
                if entry.file_name() == ".data" {
                    continue;
                }
                let ft = match entry.file_type() {
                    Ok(f) => f,
                    Err(e) => {
                        complete = false;
                        report.errors.push(err(e));
                        continue;
                    }
                };
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    if !entry.path().starts_with(&self.cache_dir)
                        && !entry.path().starts_with(&self.data_dir)
                    {
                        subdirs.push(entry.path());
                    }
                    continue;
                }
                if !ft.is_file() || !scanner::is_supported(&entry.path()) {
                    continue;
                }
                if local.is_none() {
                    let c = open_folder(dir)?;
                    self.apply_pending(&c, dir)?;
                    local = Some(c);
                }
                let c = local.as_mut().unwrap();
                if c.is_autocommit() {
                    c.execute_batch("BEGIN IMMEDIATE").map_err(err)?;
                    batch_started = Instant::now();
                }
                batch_files += 1;
                let path = entry.path();
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "Non-UTF8 filename")?;
                let before = match stat(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        complete = false;
                        report.errors.push(e);
                        continue;
                    }
                };
                report.checked += 1;
                let mut existing: Option<(i64,String,i64,i64,bool)> = c.query_row(
                "SELECT p.id,COALESCE(s.mtime_ns,''),COALESCE(s.size,-1),COALESCE(s.parser_version,0),p.excluded FROM photos p LEFT JOIN file_state s ON s.photo_id=p.id WHERE p.path=?1",
                [&name], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(err)?;
                // Same-device rename: only reuse a single matching inode/stat when old name is absent.
                if existing.is_none() && !before.2.is_empty() {
                    let candidates: Vec<(i64,String)> = c.prepare("SELECT p.id,p.path FROM photos p JOIN file_state s ON s.photo_id=p.id WHERE s.file_key=?1 AND s.mtime_ns=?2 AND s.size=?3").map_err(err)?
                    .query_map(params![before.2,before.0,before.1],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?.collect::<rusqlite::Result<_>>().map_err(err)?;
                    if candidates.len() == 1 && !dir.join(&candidates[0].1).exists() {
                        c.execute(
                            "UPDATE photos SET path=?1 WHERE id=?2",
                            params![name, candidates[0].0],
                        )
                        .map_err(err)?;
                        existing = Some((candidates[0].0, before.0.clone(), before.1, 0, false));
                    }
                }
                if let Some((id, mtime, size, version, excluded)) = &existing {
                    if *excluded
                        || (mtime == &before.0 && *size == before.1 && *version == PARSER_VERSION)
                    {
                        c.execute("INSERT OR IGNORE INTO seen VALUES(?1)", [id])
                            .map_err(err)?;
                        c.execute("UPDATE file_state SET file_key=?1 WHERE photo_id=?2 AND file_key IS NOT ?1",params![before.2,id]).map_err(err)?;
                        c.execute(
                            "UPDATE photos SET available=1 WHERE id=?1 AND available=0",
                            [id],
                        )
                        .map_err(err)?;
                        report.unchanged += 1;
                        continue;
                    }
                }
                let kind = crate::media::decode::sniff(&path);
                let Some((mut record, rich)) = scanner::parse_single_file(&path, kind) else {
                    complete = false;
                    report
                        .errors
                        .push(format!("Could not index {}", path.display()));
                    continue;
                };
                if stat(&path)? != before {
                    complete = false;
                    report
                        .errors
                        .push(format!("File changed during indexing: {}", path.display()));
                    continue;
                }
                let format = format!("{kind:?}");
                record.path = name.clone();
                record.thumb_path = None;
                c.execute("INSERT INTO photos(path,size,mtime,taken_at,city,country,lat,lng,width,height,media_type,duration,portable_id,available,fs_mtime_ns)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,lower(hex(randomblob(16))),1,?13)
                ON CONFLICT(path) DO UPDATE SET size=excluded.size,mtime=excluded.mtime,taken_at=excluded.taken_at,city=excluded.city,country=excluded.country,
                lat=excluded.lat,lng=excluded.lng,width=excluded.width,height=excluded.height,media_type=excluded.media_type,duration=excluded.duration,available=1,fs_mtime_ns=excluded.fs_mtime_ns",
                params![record.path,record.size,record.mtime,record.taken_at,record.city,record.country,record.lat,record.lng,record.width,record.height,record.media_type,record.duration,before.0]).map_err(err)?;
                c.execute("UPDATE photos SET media_subtype=CASE WHEN pair_role='still' THEN 'live' ELSE ?2 END,content_id=?3,frame_count=?4,anim_duration_ms=?5,motion_offset=?6,motion_length=?7,still_time_ms=?8,color_profile=?9 WHERE path=?1",
                params![name,rich.media_subtype,rich.content_id,rich.frame_count,rich.anim_duration_ms,rich.motion_offset,rich.motion_length,rich.still_time_ms,rich.color_profile]).map_err(err)?;
                c.execute(
                    "UPDATE photos SET media_format=?1,hdr=?3 WHERE path=?2",
                    params![format, name, rich.hdr.unwrap_or(false)],
                )
                .map_err(err)?;
                if record.media_type == "video" {
                    c.execute("UPDATE face_state SET status='running' WHERE photo_id=(SELECT paired_id FROM photos WHERE path=?1)",[&name]).map_err(err)?;
                }
                c.execute("INSERT INTO file_state(photo_id,mtime_ns,size,generation,parser_version,file_key)
                SELECT id,?2,?3,?4,?6,?5 FROM photos WHERE path=?1
                ON CONFLICT(photo_id) DO UPDATE SET mtime_ns=excluded.mtime_ns,size=excluded.size,generation=excluded.generation,parser_version=excluded.parser_version,file_key=excluded.file_key",
                params![name,before.0,before.1,generation,before.2,PARSER_VERSION]).map_err(err)?;
                c.execute(
                    "INSERT OR IGNORE INTO seen SELECT id FROM photos WHERE path=?1",
                    [&name],
                )
                .map_err(err)?;
                report.indexed += 1;
                batch_indexed += 1;
            }
            if let Some(c) = local.as_ref() {
                if !c.is_autocommit() {
                    c.execute_batch("COMMIT").map_err(err)?;
                    batch_indexed = 0;
                }
                if complete {
                    report.missing += c.execute("UPDATE photos SET available=0 WHERE available=1 AND excluded=0 AND id NOT IN
                    (SELECT photo_id FROM seen)",[]).map_err(err)?;
                }
                crate::media::metadata::pair(c)?;
                if let Some(catalog) = catalog.lock().unwrap().as_mut() {
                    if self.project(catalog, dir)? {
                        report.folders_refreshed += 1;
                    }
                }
            }
            Ok(())
        })();
        // The stack pops last-pushed first: push oldest first so the newest folder is next.
        stack.extend(subdirs.into_iter().rev());
        if result.is_err() {
            // SQLite rolls back unfinished work when this folder connection closes.
            // Do not report that uncommitted batch as successfully indexed.
            report.indexed -= batch_indexed;
        }
        result
    }
    fn apply_pending(&self, local: &Connection, dir: &Path) -> Result<()> {
        let settings = self.settings()?;
        let tx = local.unchecked_transaction().map_err(err)?;
        let local = &tx;
        let names: Vec<(String, String)> = settings
            .prepare("SELECT person_uuid,name FROM pending_names")
            .map_err(err)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(err)?;
        for (id, name) in names {
            local
                .execute(
                    "UPDATE people SET name=?1,user_named=1 WHERE portable_id=?2 AND (name<>?1 OR user_named=0)",
                    params![name, id],
                )
                .map_err(err)?;
        }
        local.execute("UPDATE faces SET assignment_source='user_named_group' WHERE assignment_source IN ('cluster','automatic') AND person_id IN (SELECT id FROM people WHERE user_named=1) AND assignment_source='cluster'",[]).map_err(err)?;
        let edits: Vec<(String, String, String, bool)> = settings
            .prepare(
                "SELECT face_uuid,person_uuid,name,rejected FROM pending_faces WHERE folder=?1",
            )
            .map_err(err)?
            .query_map([dir.to_str().unwrap()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(err)?;
        for (face, person, name, rejected) in edits {
            let named: bool = settings
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pending_names WHERE person_uuid=?1)",
                    [&person],
                    |r| r.get(0),
                )
                .map_err(err)?;
            local.execute("INSERT INTO people(name,portable_id,user_named) VALUES(?1,?2,?3) ON CONFLICT(portable_id) DO UPDATE SET user_named=MAX(people.user_named,excluded.user_named) WHERE people.user_named<excluded.user_named",params![name,person,named]).map_err(err)?;
            if rejected {
                local
                    .execute(
                        "INSERT OR IGNORE INTO face_rejections VALUES(?1,?2)",
                        params![face, person],
                    )
                    .map_err(err)?;
                local.execute("UPDATE faces SET person_id=NULL,assignment_source='user_rejected' WHERE portable_id=?1 AND (person_id IS NOT NULL OR assignment_source<>'user_rejected')",[&face]).map_err(err)?;
            } else {
                local.execute("UPDATE faces SET person_id=(SELECT id FROM people WHERE portable_id=?1),confidence=1,assignment_source='user_confirmed' WHERE portable_id=?2 AND (person_id IS NOT (SELECT id FROM people WHERE portable_id=?1) OR assignment_source<>'user_confirmed')",params![person,face]).map_err(err)?;
            }
        }
        tx.commit().map_err(err)
    }
    pub fn rename_person(&self, catalog: &mut Connection, id: i64, name: &str) -> Result<()> {
        let uuid: String = catalog
            .query_row("SELECT portable_id FROM people WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(err)?;
        self.settings()?.execute("INSERT INTO pending_names VALUES(?1,?2) ON CONFLICT(person_uuid) DO UPDATE SET name=excluded.name",params![uuid,name]).map_err(err)?;
        let folders:Vec<String>=catalog.prepare("SELECT DISTINCT ph.folder_path FROM photos ph JOIN faces f ON f.photo_id=ph.id WHERE f.person_id=?1 AND ph.folder_path IS NOT NULL").map_err(err)?
            .query_map([id],|r|r.get(0)).map_err(err)?.collect::<rusqlite::Result<_>>().map_err(err)?;
        for folder in folders {
            let dir = Path::new(&folder);
            let c = open_folder(dir)?;
            self.apply_pending(&c, dir)?;
            self.project(catalog, dir)?;
        }
        catalog
            .execute(
                "UPDATE people SET name=?1,user_named=1 WHERE id=?2",
                params![name, id],
            )
            .map_err(err)?;
        prepare_search(catalog)?;
        reindex_search(
            catalog,
            "id IN (SELECT photo_id FROM main.faces WHERE person_id=?1)",
            &[&id],
        )
    }
    pub fn review_face(
        &self,
        catalog: &mut Connection,
        face: i64,
        person: i64,
        rejected: bool,
    ) -> Result<()> {
        self.review_faces(catalog, &[(face, person, rejected)])
    }
    /// Records every review first, then replays and projects each touched folder once.
    pub fn review_faces(&self, catalog: &mut Connection, reviews: &[(i64, i64, bool)]) -> Result<()> {
        let mut folders = std::collections::BTreeSet::new();
        for &(face, person, rejected) in reviews {
            folders.insert(self.record_review(catalog, face, person, rejected)?);
        }
        for folder in folders {
            let dir = Path::new(&folder);
            let c = open_folder(dir)?;
            self.apply_pending(&c, dir)?;
            drop(c);
            self.project(catalog, dir)?;
        }
        Ok(())
    }
    fn record_review(
        &self,
        catalog: &Connection,
        face: i64,
        person: i64,
        rejected: bool,
    ) -> Result<String> {
        let (uuid,folder):(String,String)=catalog.query_row("SELECT f.portable_id,p.folder_path FROM faces f JOIN photos p ON p.id=f.photo_id WHERE f.id=?1",[face],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
        let (person_uuid, name, named): (String, String, bool) = catalog
            .query_row(
                "SELECT portable_id,name,user_named FROM people WHERE id=?1",
                [person],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(err)?;
        if named {
            self.settings()?.execute("INSERT INTO pending_names VALUES(?1,?2) ON CONFLICT(person_uuid) DO UPDATE SET name=excluded.name",params![person_uuid,name]).map_err(err)?;
        }
        self.settings()?.execute("INSERT INTO pending_faces VALUES(?1,?2,?3,?4,?5) ON CONFLICT(face_uuid) DO UPDATE SET folder=excluded.folder,person_uuid=excluded.person_uuid,name=excluded.name,rejected=excluded.rejected",params![uuid,folder,person_uuid,name,rejected]).map_err(err)?;
        Ok(folder)
    }
}
