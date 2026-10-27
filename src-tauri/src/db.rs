use rusqlite::{params, Connection, Result};

pub fn init_db(db_path: &str) -> Result<Connection> {
    let conn = Connection::open(db_path)?;

    // Create photos table with media_type and thumb_path columns
    conn.execute(
        "CREATE TABLE IF NOT EXISTS photos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT UNIQUE NOT NULL,
            size INTEGER,
            mtime INTEGER,
            taken_at INTEGER,
            thumbhash TEXT,
            city TEXT,
            country TEXT,
            lat REAL,
            lng REAL,
            width INTEGER,
            height INTEGER,
            media_type TEXT DEFAULT 'photo',
            thumb_path TEXT,
            duration INTEGER
        )",
        [],
    )?;

    // Migrate: add media_type if missing (for existing DBs)
    let _ = conn.execute(
        "ALTER TABLE photos ADD COLUMN media_type TEXT DEFAULT 'photo'",
        [],
    );
    let _ = conn.execute("ALTER TABLE photos ADD COLUMN thumb_path TEXT", []);
    let _ = conn.execute("ALTER TABLE photos ADD COLUMN duration INTEGER", []);

    // Create people table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS people (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            cluster_label INTEGER
        )",
        [],
    )?;

    // Create faces table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS faces (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            photo_id INTEGER NOT NULL,
            person_id INTEGER,
            bbox TEXT,
            confidence REAL,
            thumbnail TEXT,
            FOREIGN KEY(photo_id) REFERENCES photos(id),
            FOREIGN KEY(person_id) REFERENCES people(id)
        )",
        [],
    )?;

    // Catalog projections retain portable IDs; integer IDs remain the UI's local handles.
    for (table, column, definition) in [
        ("photos", "thumbhash", "TEXT"),
        ("photos", "media_format", "TEXT"),
        ("photos", "media_subtype", "TEXT NOT NULL DEFAULT 'still'"),
        ("photos", "pair_role", "TEXT"),
        ("photos", "paired_id", "INTEGER"),
        ("photos", "content_id", "TEXT"),
        ("photos", "still_time_ms", "INTEGER"),
        ("photos", "motion_offset", "INTEGER"),
        ("photos", "motion_length", "INTEGER"),
        ("photos", "frame_count", "INTEGER"),
        ("photos", "anim_duration_ms", "INTEGER"),
        ("photos", "live_effect", "TEXT NOT NULL DEFAULT 'live'"),
        ("photos", "key_time_ms", "INTEGER"),
        ("photos", "color_profile", "TEXT"),
        ("photos", "hdr", "INTEGER NOT NULL DEFAULT 0"),
        ("faces", "frame_ms", "INTEGER"),
        ("faces", "frame_source", "TEXT"),
        ("photos", "portable_id", "TEXT"),
        ("photos", "folder_path", "TEXT"),
        ("photos", "available", "INTEGER NOT NULL DEFAULT 1"),
        ("photos", "excluded", "INTEGER NOT NULL DEFAULT 0"),
        ("photos", "fs_mtime_ns", "TEXT"),
        ("people", "portable_id", "TEXT"),
        ("people", "user_named", "INTEGER NOT NULL DEFAULT 0"),
        ("faces", "retired", "INTEGER NOT NULL DEFAULT 0"),
        ("faces", "detection_score", "REAL"),
        ("faces", "portable_id", "TEXT"),
        ("faces", "source_key", "TEXT"),
        ("faces", "quality", "REAL"),
        ("faces", "blur", "REAL"),
        ("faces", "face_px", "REAL"),
        (
            "faces",
            "assignment_source",
            "TEXT NOT NULL DEFAULT 'legacy'",
        ),
    ] {
        let exists: bool = conn
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>>>()?
            .iter()
            .any(|c| c == column);
        if !exists {
            conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
    }
    conn.execute_batch("UPDATE photos SET portable_id=lower(hex(randomblob(16))) WHERE portable_id IS NULL;
        UPDATE people SET portable_id=lower(hex(randomblob(16))) WHERE portable_id IS NULL;
        UPDATE faces SET portable_id=lower(hex(randomblob(16))) WHERE portable_id IS NULL;
        CREATE UNIQUE INDEX IF NOT EXISTS people_portable ON people(portable_id);
        CREATE UNIQUE INDEX IF NOT EXISTS face_source ON faces(source_key);
        CREATE INDEX IF NOT EXISTS photos_folder ON photos(folder_path);
        CREATE INDEX IF NOT EXISTS photos_portable ON photos(portable_id);
        CREATE INDEX IF NOT EXISTS photos_timeline ON photos(available, excluded, COALESCE(taken_at,mtime) DESC, id DESC);
        CREATE INDEX IF NOT EXISTS faces_photo ON faces(photo_id);
        CREATE INDEX IF NOT EXISTS faces_person ON faces(person_id);")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // WAL mode for performance
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // WAL keeps transactions atomic; NORMAL avoids syncing each metadata batch.
    conn.pragma_update(None, "synchronous", "NORMAL")?;

    Ok(conn)
}

pub struct MediaRecord {
    pub path: String,
    pub size: i64,
    pub mtime: i64,
    pub taken_at: Option<i64>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub media_type: String,
    pub thumb_path: Option<String>,
    pub duration: Option<i64>,
}

pub fn insert_photos(conn: &mut Connection, records: &[MediaRecord]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO photos (path, size, mtime, taken_at, city, country, lat, lng, width, height, media_type, thumb_path, duration)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(path) DO UPDATE SET
                size = excluded.size,
                mtime = excluded.mtime,
                taken_at = excluded.taken_at,
                city = excluded.city,
                country = excluded.country,
                lat = excluded.lat,
                lng = excluded.lng,
                width = excluded.width,
                height = excluded.height,
                media_type = excluded.media_type,
                thumb_path = COALESCE(excluded.thumb_path, photos.thumb_path),
                duration = excluded.duration"
        )?;
        for r in records {
            stmt.execute(params![
                r.path,
                r.size,
                r.mtime,
                r.taken_at,
                r.city,
                r.country,
                r.lat,
                r.lng,
                r.width,
                r.height,
                r.media_type,
                r.thumb_path,
                r.duration
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}
