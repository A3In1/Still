use super::{cache, commands};
use std::path::Path;
use tauri::Manager;
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn gphotos_render(
        path: *const std::ffi::c_char,
        output: *const std::ffi::c_char,
        bounce: i32,
        edge: u32,
        animated: i32,
    ) -> i32;
}
pub fn frame(path: &Path, output: &Path, ms: i64, edge: u32) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        super::decode::native_file(path, output, edge, Some(ms))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, output, ms, edge);
        Err("Video decoder is unavailable on this platform".into())
    }
}
fn render(path: &Path, output: &Path, effect: &str, edge: u32, animated: bool) -> Result<()> {
    let _span = super::perf::Span::new("render");
    #[cfg(target_os = "macos")]
    {
        let path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(err)?;
        let output = std::ffi::CString::new(output.to_string_lossy().as_bytes()).map_err(err)?;
        if unsafe {
            gphotos_render(
                path.as_ptr(),
                output.as_ptr(),
                if animated {
                    2
                } else if effect == "bounce" {
                    1
                } else {
                    0
                },
                edge,
                animated as i32,
            )
        } == 0
        {
            Err("Could not render media".into())
        } else {
            Ok(())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, output, effect, edge, animated);
        Err("Video renderer is unavailable on this platform".into())
    }
}
pub fn long_exposure(path: &Path, output: &Path, edge: u32) -> Result<()> {
    let _span = super::perf::Span::new("render");
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn gphotos_exposure(
                path: *const std::ffi::c_char,
                output: *const std::ffi::c_char,
                edge: u32,
            ) -> i32;
        }
        let path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(err)?;
        let output = std::ffi::CString::new(output.to_string_lossy().as_bytes()).map_err(err)?;
        if unsafe { gphotos_exposure(path.as_ptr(), output.as_ptr(), edge) } == 0 {
            Err("Could not render Long Exposure".into())
        } else {
            Ok(())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, output, edge);
        Err("Long Exposure renderer is unavailable on this platform".into())
    }
}

fn derivative(
    state: &crate::AppState,
    path: &Path,
    effect: &str,
    animated: bool,
) -> Result<String> {
    let source = if animated {
        path.to_path_buf()
    } else {
        commands::motion_path(state, path)?
    };
    let output = cache::key(
        &source,
        &format!("{effect}-v2"),
        &state
            .indexer
            .cache_dir
            .join(if animated { "anim" } else { "motion" }),
    )?
    .with_extension("mp4");
    if !output.is_file() {
        let _permit = state.thumbnail_gate.acquire();
        if !output.is_file() {
            let temp = cache::temporary(&output).with_extension("mp4");
            let result = render(
                &source,
                &temp,
                effect,
                if animated { 512 } else { 1440 },
                animated,
            );
            if result.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            result?;
            std::fs::rename(temp, &output).map_err(err)?;
        }
    }
    cache::touch(&output);
    Ok(output.to_string_lossy().into_owned())
}
#[tauri::command]
pub async fn get_anim_preview(path: String, app: tauri::AppHandle) -> Result<String> {
    tauri::async_runtime::spawn_blocking(move || {
        derivative(
            &app.state::<crate::AppState>(),
            Path::new(&path),
            "anim",
            true,
        )
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn render_live_effect(
    path: String,
    effect: String,
    app: tauri::AppHandle,
) -> Result<String> {
    if !["loop", "bounce", "long_exposure"].contains(&effect.as_str()) {
        return Err("Unknown render effect".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::AppState>();
        if effect == "long_exposure" {
            return exposure_file(&state, Path::new(&path), 2048)
                .map(|p| p.to_string_lossy().into_owned());
        }
        derivative(&state, Path::new(&path), &effect, false)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn export_live(
    path: String,
    destination: String,
    kind: String,
    app: tauri::AppHandle,
) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::AppState>();
        let path = Path::new(&path);
        let source = match kind.as_str() {
            "still" => commands::still_file(&state, path, 8192)?,
            "video" => {
                let local = crate::indexing::open_folder(path.parent().ok_or("Invalid path")?)?;
                let effect: String = local
                    .query_row(
                        "SELECT live_effect FROM photos WHERE path=?1",
                        [path.file_name().unwrap().to_string_lossy()],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                if effect == "loop" || effect == "bounce" {
                    derivative(&state, path, &effect, false)?.into()
                } else {
                    commands::motion_path(&state, path)?
                }
            }
            "gif" => {
                let local = crate::indexing::open_folder(path.parent().ok_or("Invalid path")?)?;
                let effect: String = local
                    .query_row(
                        "SELECT live_effect FROM photos WHERE path=?1",
                        [path.file_name().unwrap().to_string_lossy()],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                let motion = if effect == "loop" || effect == "bounce" {
                    std::path::PathBuf::from(derivative(&state, path, &effect, false)?)
                } else {
                    commands::motion_path(&state, path)?
                };
                let duration = super::metadata::probe(&motion, true)
                    .anim_duration_ms
                    .ok_or("Duration unavailable")?
                    .min(10000);
                let output = cache::key(
                    &motion,
                    "export-gif-v1",
                    &state.indexer.cache_dir.join("motion"),
                )?
                .with_extension("gif");
                let temp = cache::temporary(&output);
                let framefile = cache::temporary(&temp);
                let result = (|| {
                    let mut encoder = image::codecs::gif::GifEncoder::new(
                        std::fs::File::create(&temp).map_err(err)?,
                    );
                    encoder
                        .set_repeat(image::codecs::gif::Repeat::Infinite)
                        .map_err(err)?;
                    for ms in (0..duration).step_by(100) {
                        frame(&motion, &framefile, ms, 480)?;
                        let image = image::open(&framefile).map_err(err)?.to_rgba8();
                        encoder
                            .encode_frame(image::Frame::from_parts(
                                image,
                                0,
                                0,
                                image::Delay::from_numer_denom_ms(100, 1),
                            ))
                            .map_err(err)?;
                    }
                    drop(encoder);
                    std::fs::rename(&temp, &output).map_err(err)
                })();
                let _ = std::fs::remove_file(framefile);
                let _ = std::fs::remove_file(temp);
                result?;
                output
            }
            _ => return Err("Unknown export format".into()),
        };
        // Never overwrite an original or an existing export silently.
        let mut destination = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(err)?;
        std::io::copy(
            &mut std::fs::File::open(source).map_err(err)?,
            &mut destination,
        )
        .map_err(err)?;
        Ok(())
    })
    .await
    .map_err(err)?
}

pub fn source_frame(
    state: &crate::AppState,
    path: &Path,
    ms: i64,
    source: &str,
) -> Result<std::path::PathBuf> {
    let input = if source == "motion" {
        commands::motion_path(state, path)?
    } else {
        path.to_path_buf()
    };
    let output = cache::key(
        &input,
        &format!("face-frame-v1-{source}-{ms}"),
        &state.indexer.cache_dir.join("display"),
    )?;
    if !output.is_file() {
        let temp = cache::temporary(&output);
        let result = if source == "motion" {
            frame(&input, &temp, ms, 0)
        } else {
            #[cfg(target_os = "macos")]
            {
                unsafe extern "C" {
                    fn gphotos_animation_frame(
                        path: *const std::ffi::c_char,
                        output: *const std::ffi::c_char,
                        ms: i64,
                        edge: u32,
                    ) -> i32;
                }
                let input =
                    std::ffi::CString::new(input.to_string_lossy().as_bytes()).map_err(err)?;
                let out = std::ffi::CString::new(temp.to_string_lossy().as_bytes()).map_err(err)?;
                if unsafe { gphotos_animation_frame(input.as_ptr(), out.as_ptr(), ms, 0) } == 0 {
                    Err("Could not decode animation frame".into())
                } else {
                    Ok(())
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err("Animation decoder unavailable".into())
            }
        };
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result?;
        std::fs::rename(temp, &output).map_err(err)?;
    }
    Ok(output)
}

pub fn exposure_file(
    state: &crate::AppState,
    path: &Path,
    edge: u32,
) -> Result<std::path::PathBuf> {
    let source = commands::motion_path(state, path)?;
    let output = cache::key(
        &source,
        &format!("exposure-v2-{}", edge.min(2048)),
        &state.indexer.cache_dir.join("motion"),
    )?;
    if !output.is_file() {
        let _permit = state.thumbnail_gate.acquire();
        if !output.is_file() {
            let temp = cache::temporary(&output);
            let result = long_exposure(&source, &temp, edge);
            if result.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            result?;
            std::fs::rename(temp, &output).map_err(err)?;
        }
    }
    cache::touch(&output);
    Ok(output)
}

pub fn animation_samples(path: &Path, duration: i64) -> Vec<i64> {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn gphotos_animation_samples(path: *const std::ffi::c_char) -> *mut std::ffi::c_char;
            fn gphotos_free(ptr: *mut std::ffi::c_char);
        }
        if let Ok(path) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
            unsafe {
                let ptr = gphotos_animation_samples(path.as_ptr());
                if !ptr.is_null() {
                    let result = serde_json::from_slice::<Vec<i64>>(
                        std::ffi::CStr::from_ptr(ptr).to_bytes(),
                    )
                    .ok();
                    gphotos_free(ptr);
                    if let Some(result) = result.filter(|v| !v.is_empty() && v.len() <= 8) {
                        return result;
                    }
                }
            }
        }
    }
    (0..8).map(|i| (duration.max(1) - 1) * i / 7).collect()
}
