use std::io::{Read, Seek, SeekFrom};
use tauri::{
    http::{Request, Response},
    Manager,
};
fn response(status: u16, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .body(body)
        .unwrap()
}
pub fn motion(app: &tauri::AppHandle, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    if request.method() == "OPTIONS" {
        return Response::builder()
            .status(204)
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Allow-Headers", "Range")
            .header("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS")
            .body(vec![])
            .unwrap();
    }
    if request.method() != "GET" && request.method() != "HEAD" {
        return response(405, vec![]);
    }
    let Ok(decoded) = urlencoding::decode(request.uri().path()) else {
        return response(404, vec![]);
    };
    let route = decoded.trim_start_matches('/');
    let state = app.state::<crate::AppState>();
    let row: Option<(String, u64, Option<u64>)> = if let Some(id) =
        route.strip_prefix("motion/").and_then(|v| v.parse::<i64>().ok())
    {
        let guard = state.db_conn.lock().unwrap();
        guard.as_ref().and_then(|c|c.query_row("SELECT path,motion_offset,motion_length FROM photos WHERE id=?1 AND available=1 AND excluded=0 AND media_subtype='motion_embedded'",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)? as u64,Some(r.get::<_,i64>(2)? as u64)))).ok())
    } else if let Some(id) = route.strip_prefix("video/").and_then(|v| v.parse::<i64>().ok()) {
        // Whole video files; the type comes from the extension because QuickTime headers
        // (`wide`/`free`/`skip` atoms) are not always recognised by content sniffing.
        let guard = state.db_conn.lock().unwrap();
        guard.as_ref().and_then(|c|c.query_row("SELECT path FROM photos WHERE id=?1 AND available=1 AND excluded=0 AND media_type='video'",[id],|r|Ok((r.get::<_,String>(0)?,0,None))).ok())
    } else {
        None
    };
    let Some((path, offset, length)) = row else {
        return response(404, vec![]);
    };
    let mime = match std::path::Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .as_deref()
    {
        Some("mov" | "qt") if length.is_none() => "video/quicktime",
        Some("webm") if length.is_none() => "video/webm",
        Some("3gp") if length.is_none() => "video/3gpp",
        Some("ts") if length.is_none() => "video/mp2t",
        _ => "video/mp4",
    };
    let Ok(mut file) = std::fs::File::open(path) else {
        return response(404, vec![]);
    };
    let Some(length) = length.or_else(|| file.metadata().ok().map(|m| m.len())) else {
        return response(404, vec![]);
    };
    if length == 0
        || offset
            .checked_add(length)
            .map(|end| end > file.metadata().map(|m| m.len()).unwrap_or(0))
            .unwrap_or(true)
    {
        return response(410, vec![]);
    }
    let unsatisfiable = || {
        Response::builder()
            .status(416)
            .header("Content-Range", format!("bytes */{length}"))
            .header("Access-Control-Allow-Origin", "*")
            .body(vec![])
            .unwrap()
    };
    let range = request.headers().get("Range");
    let (start, end) = if let Some(range) = range {
        let Some((start, end)) = range
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("bytes="))
            .and_then(|v| v.split_once('-'))
        else {
            return unsatisfiable();
        };
        if start.is_empty() {
            let Some(suffix) = end.parse::<u64>().ok().filter(|v| *v > 0) else {
                return unsatisfiable();
            };
            (length.saturating_sub(suffix), length - 1)
        } else {
            let Ok(start) = start.parse::<u64>() else {
                return unsatisfiable();
            };
            let end = if end.is_empty() {
                length - 1
            } else {
                let Ok(end) = end.parse::<u64>() else {
                    return unsatisfiable();
                };
                end.min(length - 1)
            };
            if start >= length || end < start {
                return unsatisfiable();
            }
            (start, end.min(start.saturating_add(4 * 1024 * 1024 - 1)))
        }
    } else if length > 64 * 1024 * 1024 {
        // Media elements always send Range; a plain GET of a large file gets its first chunk.
        (0, 4 * 1024 * 1024 - 1)
    } else {
        (0, length - 1)
    };
    let count = end - start + 1;
    if count > 64 * 1024 * 1024 {
        return response(413, vec![]);
    }
    let mut body = Vec::new();
    if request.method() != "HEAD" {
        if file.seek(SeekFrom::Start(offset + start)).is_err()
            || file.take(count).read_to_end(&mut body).is_err()
            || body.len() as u64 != count
        {
            return response(500, vec![]);
        }
    }
    let mut builder = Response::builder()
        .status(if range.is_some() || count < length { 206 } else { 200 })
        .header("Content-Type", mime)
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", count)
        .header("Access-Control-Allow-Origin", "*");
    if range.is_some() || count < length {
        builder = builder.header("Content-Range", format!("bytes {start}-{end}/{length}"));
    }
    builder.body(body).unwrap()
}
