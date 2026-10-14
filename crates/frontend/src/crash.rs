use crate::store::Settings;
use anyhow::{Context, Result, bail, ensure};
use std::{
    backtrace::Backtrace,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, atomic::{AtomicBool, Ordering}},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tau_net::{CrashFrame, CrashReport, MAX_CRASH_BYTES};

const LOCAL_TRACE: &str = "client-crash.log";
const PENDING_REPORT: &str = "client-crash.pending.json";
const MAX_LOCAL_BYTES: usize = 64 * 1024;
static INSTALLED: AtomicBool = AtomicBool::new(false);
static WRITE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn install(root: PathBuf) {
    if INSTALLED.swap(true, Ordering::AcqRel) { return; }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let saved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let detail = info.to_string();
            let location = info.location().map(|location| (location.file(), location.line()));
            record(&root, "rust.panic", &detail, location, true);
        }));
        if saved.is_err() { eprintln!("Tau crash reporting failed"); }
        previous(info);
    }));
}

pub(crate) fn record_error(root: &Path, detail: &str) {
    record(root, "app.tau.FatalError", detail, None, false);
}

fn record(root: &Path, exception_class: &str, detail: &str, location: Option<(&str, u32)>, panic_report: bool) {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Err(error) = std::fs::create_dir_all(root) {
        eprintln!("Tau could not create its crash directory: {error}");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700));
    }
    let report_id = uuid::Uuid::new_v4().to_string();
    let current = std::thread::current();
    let thread = current.name().unwrap_or("unnamed");
    let backtrace = Backtrace::force_capture().to_string();
    #[cfg(not(target_os = "android"))]
    if panic_report {
        let location = location.map(|(file, line)| format!("{file}:{line}")).unwrap_or_else(|| "unknown location".into());
        crate::telemetry::report_panic(&location, detail, &backtrace);
    }
    #[cfg(target_os = "android")]
    let _ = panic_report;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let mut trace = format!("Tau {} crash {} at {}\nThread: {}\n{}\n{}", env!("CARGO_PKG_VERSION"), report_id, timestamp, thread, detail, backtrace);
    if trace.len() > MAX_LOCAL_BYTES {
        let suffix = "\n[trace truncated]\n";
        let mut end = (MAX_LOCAL_BYTES - suffix.len()).min(trace.len());
        while !trace.is_char_boundary(end) { end -= 1; }
        trace.truncate(end);
        trace.push_str(suffix);
    }
    if let Err(error) = write_atomic(&root.join(LOCAL_TRACE), trace.as_bytes(), true) {
        eprintln!("Tau could not save {LOCAL_TRACE}: {error}");
    }

    let platform = if cfg!(target_os = "android") { "android" } else if cfg!(windows) { "windows" } else { "desktop" };
    let mut report = CrashReport {
        schema: 2,
        report_id,
        platform: platform.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        os_version: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        thread: thread.chars().take(128).collect(),
        exception_class: exception_class.chars().take(192).collect(),
        stack: crash_frames(&backtrace, location),
        selection_range: None,
        causes: vec![],
    };
    let mut encoded = match serde_json::to_vec(&report) {
        Ok(encoded) => encoded,
        Err(error) => {
            eprintln!("Tau could not encode its crash report: {error}");
            return;
        }
    };
    while encoded.len() >= MAX_CRASH_BYTES && !report.stack.is_empty() {
        report.stack.pop();
        encoded = match serde_json::to_vec(&report) {
            Ok(encoded) => encoded,
            Err(error) => {
                eprintln!("Tau could not encode its crash report: {error}");
                return;
            }
        };
    }
    if encoded.len() >= MAX_CRASH_BYTES {
        eprintln!("Tau crash report headers exceed {MAX_CRASH_BYTES} bytes");
        return;
    }
    let pending = root.join(PENDING_REPORT);
    if !pending.exists() && let Err(error) = write_atomic(&pending, &encoded, false) {
        eprintln!("Tau could not save {PENDING_REPORT}: {error}");
    }
}

fn crash_frames(backtrace: &str, location: Option<(&str, u32)>) -> Vec<CrashFrame> {
    let mut frames = vec![];
    if let Some((file, line)) = location {
        frames.push(CrashFrame {
            class_name: "rust.panic".into(),
            method_name: "panic".into(),
            file_name: Some(file.chars().take(192).collect()),
            line_number: line.min(i32::MAX as u32) as i32,
        });
    }
    let mut current = None;
    for raw in backtrace.lines() {
        let line = raw.trim();
        if line.is_empty() || line == "stack backtrace:" { continue; }
        if let Some((index, name)) = line.split_once(": ")
            && index.bytes().all(|byte| byte.is_ascii_digit())
            && frames.len() < 48
        {
            let name = name.trim();
            let (class_name, method_name) = name.rsplit_once("::").unwrap_or(("rust", name));
            frames.push(CrashFrame {
                class_name: class_name.chars().take(192).collect(),
                method_name: method_name.chars().take(192).collect(),
                file_name: None,
                line_number: -1,
            });
            current = Some(frames.len() - 1);
        } else if let Some(path) = line.strip_prefix("at ")
            && let Some(frame) = current.and_then(|index| frames.get_mut(index))
        {
            let mut parts = path.rsplitn(3, ':');
            let _column = parts.next();
            let line = parts.next().and_then(|value| value.parse::<i32>().ok());
            let file = parts.next();
            if let (Some(file), Some(line)) = (file, line) {
                frame.file_name = Some(file.chars().take(192).collect());
                frame.line_number = line;
            }
        }
    }
    if frames.is_empty() {
        frames.push(CrashFrame { class_name:"rust".into(), method_name:"backtrace unavailable".into(), file_name:None, line_number:-1 });
    }
    frames
}

fn write_atomic(path: &Path, bytes: &[u8], replace: bool) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".{}.{}.tmp", path.file_name().unwrap_or_default().to_string_lossy(), uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if replace {
            #[cfg(unix)]
            std::fs::rename(&temporary, path)?;
            #[cfg(not(unix))]
            {
                match std::fs::remove_file(path) { Ok(()) => {}, Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}, Err(error) => return Err(error) }
                std::fs::rename(&temporary, path)?;
            }
        } else {
            match std::fs::hard_link(&temporary, path) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(error) => return Err(error),
            }
            std::fs::remove_file(&temporary)?;
        }
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if temporary.exists() { let _ = std::fs::remove_file(temporary); }
    result
}

pub(crate) async fn upload_pending(root: &Path, settings: &Settings) -> Result<()> {
    let path = root.join(PENDING_REPORT);
    let original = match tokio::fs::read(&path).await {
        Ok(original) => original,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(!original.is_empty() && original.len() < MAX_CRASH_BYTES, "Invalid pending crash report size");
    let report: CrashReport = serde_json::from_slice(&original).context("Invalid pending crash report")?;
    let encoded = serde_json::to_vec(&report)?;
    ensure!(encoded.len() < MAX_CRASH_BYTES, "Invalid pending crash report size");
    let response = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()?
        .post(crate::net::endpoint(settings, &["v1", "telemetry", "crash"])?)
        .bearer_auth(&settings.token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(encoded)
        .send().await?;
    if !response.status().is_success() { bail!("Crash upload failed with HTTP {}", response.status()); }
    match tokio::fs::read(&path).await {
        Ok(current) if current == original => tokio::fs::remove_file(path).await?,
        Ok(_) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/crash.rs"]
mod tests;
