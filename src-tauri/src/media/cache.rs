use std::{
    path::{Path, PathBuf},
    sync::{Condvar, Mutex},
};
pub struct Pool {
    active: Mutex<usize>,
    ready: Condvar,
    limit: usize,
}
pub struct Permit<'a>(&'a Pool);

/// Puts the calling thread in macOS's "utility" QoS class: it prefers efficiency cores and
/// yields to anything the user is doing, so long background passes stay cool.
pub fn low_priority_thread() {
    #[cfg(target_os = "macos")]
    unsafe {
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(class: u32, relative_priority: i32) -> i32;
        }
        const QOS_CLASS_UTILITY: u32 = 0x11;
        pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
    }
}
impl Default for Pool {
    fn default() -> Self {
        Self {
            active: Mutex::new(0),
            ready: Condvar::new(),
            limit: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(2)
                .saturating_sub(1)
                .max(1),
        }
    }
}
impl Pool {
    pub fn acquire(&self) -> Permit<'_> {
        let mut n = self.active.lock().unwrap();
        while *n >= self.limit {
            n = self.ready.wait(n).unwrap();
        }
        *n += 1;
        Permit(self)
    }
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self.0.active.lock().unwrap() -= 1;
        self.0.ready.notify_one();
    }
}
pub fn key(path: &Path, variant: &str, dir: &Path) -> Result<PathBuf, String> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    variant.hash(&mut h);
    let m = std::fs::metadata(path).map_err(|e| e.to_string())?;
    m.len().hash(&mut h);
    m.modified().ok().hash(&mut h);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{:x}.jpg", h.finish())))
}
pub fn temporary(path: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    path.with_extension(format!(
        "{}.{}.pending.jpg",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

pub fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::open(path) {
        let _ =
            file.set_times(std::fs::FileTimes::new().set_accessed(std::time::SystemTime::now()));
    }
}
pub fn sweep(root: &Path, quota: u64) -> Result<(), String> {
    let mut files = Vec::new();
    let mut total = 0u64;
    for name in ["motion", "anim", "display"] {
        let Ok(entries) = std::fs::read_dir(root.join(name)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() || path.to_string_lossy().contains(".pending.") {
                continue;
            }
            let used = meta
                .accessed()
                .or_else(|_| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            total = total.saturating_add(meta.len());
            files.push((used, path, meta.len()));
        }
    }
    files.sort_by_key(|f| f.0);
    for (used, path, size) in files {
        if total <= quota {
            break;
        }
        // A grace period keeps a just-returned URL and an active short clip alive.
        if used.elapsed().unwrap_or_default().as_secs() < 600 {
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}
