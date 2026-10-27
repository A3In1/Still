use super::{cache, decode};
use crate::{indexing, AppState};
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager, State};
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn motion_path(state: &AppState, path: &Path) -> Result<PathBuf> {
    let dir = path.parent().ok_or("Invalid path")?;
    let c = indexing::open_folder(dir)?;
    let (partner,offset,length):(Option<String>,Option<i64>,Option<i64>)=c.query_row("SELECT q.path,p.motion_offset,p.motion_length FROM photos p LEFT JOIN photos q ON q.id=p.paired_id AND q.available=1 AND q.excluded=0 WHERE p.path=?1 AND p.available=1 AND p.excluded=0",[path.file_name().unwrap().to_string_lossy()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(err)?;
    if let Some(partner) = partner {
        return Ok(dir.join(partner));
    }
    if let (Some(offset), Some(length)) = (offset, length) {
        let offset = u64::try_from(offset).map_err(err)?;
        let length = u64::try_from(length).map_err(err)?;
        use std::io::{Read, Seek, SeekFrom, Write};
        let output = cache::key(path, "embedded-v1", &state.indexer.cache_dir.join("motion"))?
            .with_extension("mp4");
        if !output.is_file() {
            let temp = cache::temporary(&output);
            let result = (|| {
                let mut source = std::fs::File::open(path).map_err(err)?;
                if offset.checked_add(length).ok_or("Invalid motion range")?
                    > source.metadata().map_err(err)?.len()
                {
                    return Err("Invalid motion range".into());
                }
                source.seek(SeekFrom::Start(offset)).map_err(err)?;
                let mut target = std::fs::File::create(&temp).map_err(err)?;
                let copied = std::io::copy(&mut source.take(length), &mut target).map_err(err)?;
                if copied != length {
                    return Err("Motion source changed".into());
                }
                target.flush().map_err(err)?;
                std::fs::rename(&temp, &output).map_err(err)
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            result?;
        }
        return Ok(output);
    }
    Err("This photo has no available motion component".into())
}
pub fn still_file(state: &AppState, path: &Path, edge: u32) -> Result<PathBuf> {
    let _span = super::perf::Span::new("display");
    let local = indexing::open_folder(path.parent().ok_or("Invalid path")?)?;
    let (key, effect): (Option<i64>, String) = local
        .query_row(
            "SELECT key_time_ms,live_effect FROM photos WHERE path=?1",
            [path.file_name().unwrap().to_string_lossy()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((None, "live".into()));
    if effect == "long_exposure" {
        return super::video::exposure_file(state, path, edge);
    }
    let motion = if key.is_some() {
        Some(motion_path(state, path)?)
    } else {
        None
    };
    let variant = format!(
        "display-v2-{edge}-{key:?}-{effect}-{:?}",
        motion.as_ref().and_then(|p| indexing::stat(p).ok())
    );
    let output = cache::key(path, &variant, &state.indexer.cache_dir.join("display"))?;
    if output.is_file() {
        cache::touch(&output);
        return Ok(output);
    }
    let _permit = state.thumbnail_gate.acquire();
    if output.is_file() {
        cache::touch(&output);
        return Ok(output);
    }
    let temp = cache::temporary(&output);
    let result = (|| {
        if let Some(time) = key {
            return super::video::frame(motion.as_ref().unwrap(), &temp, time, edge);
        }
        decode::open_oriented(path, Some(edge))
            .map_err(err)?
            .to_rgb8()
            .save_with_format(&temp, image::ImageFormat::Jpeg)
            .map_err(err)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result?;
    std::fs::rename(temp, &output).map_err(err)?;
    Ok(output)
}
#[tauri::command]
pub async fn get_display_image(
    path: String,
    edge: Option<u32>,
    app: tauri::AppHandle,
) -> Result<String> {
    tauri::async_runtime::spawn_blocking(move || {
        still_file(
            &app.state::<AppState>(),
            Path::new(&path),
            edge.unwrap_or(2560).clamp(128, 8192),
        )
        .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(err)?
}
/// Rich metadata for the info panel, plus the partner file name of a Live Photo pair.
#[tauri::command]
pub async fn get_media_details(path: String, app: tauri::AppHandle) -> Result<serde_json::Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let partner: Option<(String, String)> = state.db_conn.lock().unwrap().as_ref().and_then(|c| {
            c.query_row("SELECT q.path,COALESCE(p.media_type,'photo') FROM photos p JOIN photos q ON q.id=p.paired_id WHERE p.path=?1", [&path], |r| Ok((r.get(0)?, r.get(1)?))).ok()
        });
        let source = Path::new(&path);
        let video = decode::sniff(source) == decode::Kind::Video;
        let mut details = super::metadata::details(source, video);
        if let (Some((partner, _)), Some(map)) = (partner, details.as_object_mut()) {
            if let Some(name) = Path::new(&partner).file_name() {
                map.insert("paired_file".into(), name.to_string_lossy().into_owned().into());
            }
        }
        Ok(details)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn get_motion_src(path: String, app: tauri::AppHandle) -> Result<String> {
    tauri::async_runtime::spawn_blocking(move||{
        let state=app.state::<AppState>();
        let id:Option<i64>={let guard=state.db_conn.lock().unwrap();guard.as_ref().and_then(|c|c.query_row("SELECT id FROM photos WHERE path=?1 AND media_subtype='motion_embedded' AND available=1 AND excluded=0",[&path],|r|r.get(0)).ok())};
        if let Some(id)=id{return Ok(format!("gmedia://localhost/motion/{id}"));}
        motion_path(&state,Path::new(&path)).map(|p|p.to_string_lossy().into_owned())
    }).await.map_err(err)?
}
async fn edit(
    path: String,
    effect: Option<String>,
    key: Option<Option<i64>>,
    app: tauri::AppHandle,
) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move||{
        let state=app.state::<AppState>();let _gate=state.indexer.gate.lock().unwrap();
        let p=Path::new(&path);let dir=p.parent().ok_or("Invalid path")?;
        let motion=motion_path(&state,p)?;
        if let Some(Some(ms))=key {let duration=super::metadata::probe(&motion,true).anim_duration_ms.ok_or("Motion duration unavailable")?;if ms<0 || ms>=duration{return Err("Choose a frame inside the clip".into());}}
        let local=indexing::open_folder(dir)?;
        let name=p.file_name().unwrap().to_string_lossy();
        if let Some(effect)=effect {local.execute("UPDATE photos SET live_effect=?1 WHERE path=?2",rusqlite::params![effect,name]).map_err(err)?;}
        if let Some(key)=key {
            let tx=local.unchecked_transaction().map_err(err)?;
            tx.execute("UPDATE photos SET key_time_ms=?1 WHERE path=?2",rusqlite::params![key,name]).map_err(err)?;
            // Keep manual records; force inference to compare against the newly selected frame.
            tx.execute("UPDATE face_state SET status='running',pixel_digest=NULL WHERE photo_id=(SELECT id FROM photos WHERE path=?1)",[&name]).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        if let Some(c)=state.db_conn.lock().unwrap().as_mut(){state.indexer.project(c,dir)?;}
        let _=app.emit("index_updated",serde_json::json!({"errors":[]}));
        crate::face_worker::schedule(&app);
        Ok(())
    }).await.map_err(err)?
}
#[tauri::command]
pub async fn set_live_effect(path: String, effect: String, app: tauri::AppHandle) -> Result<()> {
    if !["live", "loop", "bounce", "long_exposure", "off"].contains(&effect.as_str()) {
        return Err("Unknown Live effect".into());
    }
    edit(path, Some(effect), None, app).await
}
#[tauri::command]
pub async fn set_key_frame(
    path: String,
    time_ms: Option<i64>,
    app: tauri::AppHandle,
) -> Result<()> {
    edit(path, None, Some(time_ms), app).await
}
#[tauri::command]
pub async fn get_frame_strip(path: String, app: tauri::AppHandle) -> Result<Vec<(i64, String)>> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let motion = motion_path(&state, Path::new(&path))?;
        let duration = super::metadata::probe(&motion, true)
            .anim_duration_ms
            .ok_or("Duration unavailable")?;
        let _permit = state.thumbnail_gate.acquire();
        (0..12)
            .map(|i| {
                let ms = (duration - 1).max(0) * i / 12;
                let output = cache::key(
                    &motion,
                    &format!("strip-{ms}"),
                    &state.indexer.cache_dir.join("display"),
                )?;
                if !output.is_file() {
                    let temp = cache::temporary(&output);
                    super::video::frame(&motion, &temp, ms, 240)?;
                    std::fs::rename(temp, &output).map_err(err)?;
                }
                Ok((ms, output.to_string_lossy().into_owned()))
            })
            .collect()
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub fn get_media_settings(state: State<'_, AppState>) -> Result<serde_json::Value> {
    let path = state.indexer.data_dir.join("media-settings.json");
    // Stored files from older versions may lack newer keys; defaults fill them in.
    let mut settings = serde_json::json!({"autoplayLive":true,"hoverLive":true,"gifMode":"hover","deepScan":false,"hardwareFaces":false,"cacheGb":2});
    if let Some(serde_json::Value::Object(stored)) = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        settings.as_object_mut().unwrap().extend(stored);
    }
    Ok(settings)
}
#[tauri::command]
pub fn set_media_settings(
    settings: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    if !["always", "hover", "never"].contains(&settings["gifMode"].as_str().unwrap_or(""))
        || ![1, 2, 5, 10].contains(&settings["cacheGb"].as_u64().unwrap_or(0))
        || ["autoplayLive", "hoverLive", "deepScan", "hardwareFaces"]
            .iter()
            .any(|key| !settings[key].is_boolean())
    {
        return Err("Invalid media settings".into());
    }
    let path = state.indexer.data_dir.join("media-settings.json");
    let temp = cache::temporary(&path);
    std::fs::write(&temp, serde_json::to_vec(&settings).map_err(err)?).map_err(err)?;
    std::fs::rename(temp, path).map_err(err)?;
    crate::face_worker::schedule(&app);
    Ok(())
}
