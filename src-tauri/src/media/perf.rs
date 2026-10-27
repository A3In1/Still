use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Instant,
};
static LOG: OnceLock<PathBuf> = OnceLock::new();
static GATE: Mutex<()> = Mutex::new(());
pub fn initialize(dir: PathBuf) {
    if std::env::var("GPHOTOS_PERF").as_deref() == Ok("1") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = LOG.set(dir.join("perf.ndjson"));
    }
}
pub fn log(name: &str, ms: f64) {
    use std::io::Write;
    if !ms.is_finite() {
        return;
    }
    if let Some(path) = LOG.get() {
        let _gate = GATE.lock().unwrap();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{}", serde_json::json!({"span":name,"ms":ms}));
        }
    }
}
pub struct Span(&'static str, Instant);
impl Span {
    pub fn new(name: &'static str) -> Self {
        Self(name, Instant::now())
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        log(self.0, self.1.elapsed().as_secs_f64() * 1000.);
    }
}
#[tauri::command]
pub fn perf_log(name: String, ms: f64) {
    if ["viewer-open", "first-pixels", "motion-start"].contains(&name.as_str()) {
        log(&name, ms);
    }
}
