//! User-owned downloads, not the disposable hashed content cache.
use crate::store::SavedDownload;
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub fn name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let clean = base
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(160)
        .collect::<String>();
    let clean = clean.trim().trim_end_matches('.');
    let stem = clean.split('.').next().unwrap_or("");
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if clean.is_empty() || clean == "." || clean == ".." {
        "tau-attachment".into()
    } else if reserved.iter().any(|r| stem.eq_ignore_ascii_case(r)) {
        format!("_{clean}")
    } else {
        clean.into()
    }
}
pub fn mime(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpeg" | "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" | "md" | "log" => "text/plain",
        "zip" => "application/zip",
        "apk" => "application/vnd.android.package-archive",
        _ => "application/octet-stream",
    }
}
pub fn use_saved(path: &Path, action: crate::app::SavedAction) -> Result<()> {
    ensure!(path.is_file(), "The downloaded file no longer exists");
    match action {
        crate::app::SavedAction::Open => open::that(path).map_err(Into::into),
        crate::app::SavedAction::Show => {
            #[cfg(windows)]
            {
                use std::ffi::OsString;
                let mut selection = OsString::from("/select,");
                selection.push(path.as_os_str());
                std::process::Command::new("explorer.exe")
                    .arg(selection)
                    .spawn()?;
                Ok(())
            }
            #[cfg(not(windows))]
            {
                open::that(path.parent().context("No Downloads folder")?).map_err(Into::into)
            }
        }
        crate::app::SavedAction::Extract => {
            let folder = extract_zip(path)?;
            open::that(folder)?;
            Ok(())
        }
    }
}
/// Extract into a new sibling folder, never over an existing folder. Reject
/// symlinks, traversal, duplicate/case-colliding names and decompression bombs.
pub fn extract_zip(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip")),
        "Not a ZIP download"
    );
    ensure!(
        path.metadata()?.len() <= 50_000_000,
        "Archive exceeds the 50 MB download limit"
    );
    let parent = path.parent().context("Download has no parent folder")?;
    let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
    ensure!(archive.len() <= 2048, "Archive contains too many files");
    let staging = tempfile::Builder::new()
        .prefix(".tau-extract-")
        .tempdir_in(parent)?;
    let mut names = std::collections::HashSet::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        ensure!(
            !file.is_symlink()
                && (file.is_dir() || file.is_file())
                && file
                    .unix_mode()
                    .is_none_or(|mode| matches!(mode & 0o170000, 0 | 0o040000 | 0o100000)),
            "Archive contains an unsupported file type"
        );
        let relative = file
            .enclosed_name()
            .context("Archive path escapes the extraction folder")?;
        ensure!(
            relative.components().all(|c| match c {
                std::path::Component::Normal(name) => {
                    let text = name.to_string_lossy();
                    !text.is_empty()
                        && !text.contains([':', '\\'])
                        && !text.ends_with([' ', '.'])
                        && super_safe_component(&text)
                }
                _ => false,
            }),
            "Archive contains an unsafe path"
        );
        let folded = relative.to_string_lossy().to_lowercase();
        ensure!(names.insert(folded), "Archive contains duplicate files");
        total = total
            .checked_add(file.size())
            .context("Archive size overflow")?;
        ensure!(total <= 512 * 1024 * 1024, "Archive expands beyond 512 MB");
        let target = staging.path().join(relative);
        if file.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        fs::create_dir_all(target.parent().unwrap())?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        let expected = file.size();
        let count = io::copy(&mut file.take(expected + 1), &mut output)?;
        ensure!(count == expected, "Archive file length mismatch");
        output.sync_all()?;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let stem = name(&stem);
    let stage = staging.keep();
    for index in 1..=10_000 {
        let folder = parent.join(if index == 1 {
            stem.clone()
        } else {
            format!("{stem} ({index})")
        });
        if folder.exists() {
            continue;
        }
        match fs::rename(&stage, &folder) {
            Ok(()) => return Ok(folder),
            Err(_) if folder.exists() => continue,
            Err(error) => {
                let _ = fs::remove_dir_all(stage);
                return Err(error.into());
            }
        }
    }
    let _ = fs::remove_dir_all(stage);
    anyhow::bail!("Too many extraction folders")
}
fn super_safe_component(text: &str) -> bool {
    let safe = name(text);
    safe == text && text != "." && text != ".."
}
pub fn default_directory() -> Result<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    let home = home.context("Home folder unavailable; cannot locate Downloads/Tau")?;
    Ok(PathBuf::from(home).join("Downloads").join("Tau"))
}
/// Copy to a temporary file and publish without ever replacing another download.
/// Returns only after the file is durable. A cancelled/failed attempt leaves no
/// partially visible output in Downloads/Tau.
pub fn save_into(directory: &Path, source: &Path, raw_name: &str) -> Result<SavedDownload> {
    let safe = name(raw_name);
    let before = source
        .metadata()
        .context("Downloaded file is unavailable")?;
    ensure!(
        before.is_file() && before.len() <= 50_000_000,
        "Downloaded file exceeds 50 MB"
    );
    fs::create_dir_all(directory)?;
    let input = fs::File::open(source)?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    ensure!(
        io::copy(&mut input.take(50_000_001), &mut temp)? == before.len(),
        "Downloaded file changed while saving"
    );
    ensure!(
        source.metadata()?.len() == before.len()
            && source.metadata()?.modified()? == before.modified()?,
        "Downloaded file changed while saving"
    );
    temp.as_file().sync_all()?;
    let dot = safe.rfind('.').filter(|i| *i > 0).unwrap_or(safe.len());
    let (stem, ext) = safe.split_at(dot);
    for index in 1..=10_000 {
        let filename = if index == 1 {
            safe.clone()
        } else {
            format!("{stem} ({index}){ext}")
        };
        let target = directory.join(filename);
        match temp.persist_noclobber(&target) {
            Ok(_) => {
                #[cfg(unix)]
                fs::File::open(directory)?.sync_all()?;
                return Ok(SavedDownload {
                    location: target.to_string_lossy().into_owned(),
                    reference: target.to_string_lossy().into_owned(),
                    mime_type: mime(&safe).into(),
                });
            }
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                temp = error.file;
            }
            Err(error) => return Err(error.error.into()),
        }
    }
    anyhow::bail!("Too many files with this name in Downloads/Tau")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn archive(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, content) in entries {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }
    #[test]
    fn zip_extracts_to_distinct_folders_and_rejects_escape_and_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bundle.zip");
        archive(&path, &[("notes/readme.txt", b"safe")]);
        let first = extract_zip(&path).unwrap();
        let second = extract_zip(&path).unwrap();
        assert_eq!(fs::read(first.join("notes/readme.txt")).unwrap(), b"safe");
        assert_eq!(second.file_name().unwrap(), "bundle (2)");
        archive(&path, &[("../outside", b"bad")]);
        assert!(extract_zip(&path).is_err());
        assert!(!dir.path().join("outside").exists());
        archive(&path, &[("FILE", b"first"), ("file", b"second")]);
        assert!(extract_zip(&path).is_err());
    }
    #[test]
    fn remembered_export_is_scoped_and_survives_a_store_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().join("local")).unwrap();
        let saved = SavedDownload {
            location: "Downloads/Tau/report.txt".into(),
            reference: "record".into(),
            mime_type: "text/plain".into(),
        };
        store
            .record_download("account", "lineage", "chat", "file", &saved)
            .unwrap();
        drop(store);
        let store = crate::store::Store::open(dir.path().join("local")).unwrap();
        assert_eq!(
            store
                .saved_download("account", "lineage", "chat", "file")
                .unwrap(),
            Some(saved)
        );
        assert_eq!(
            store
                .saved_download("account", "new-source", "chat", "file")
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .saved_download("another-account", "lineage", "chat", "file")
                .unwrap(),
            None
        );
        store
            .forget_download("account", "lineage", "chat", "file")
            .unwrap();
        assert_eq!(
            store
                .saved_download("account", "lineage", "chat", "file")
                .unwrap(),
            None
        );
    }
    #[test]
    fn saves_in_downloads_without_overwriting_and_sanitizes_names() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in");
        fs::write(&input, b"hello").unwrap();
        let out = dir.path().join("Downloads/Tau");
        let first = save_into(&out, &input, "../../CON.zip").unwrap();
        let second = save_into(&out, &input, "..\\CON.zip").unwrap();
        assert_eq!(Path::new(&first.reference).file_name().unwrap(), "_CON.zip");
        assert_eq!(
            Path::new(&second.reference).file_name().unwrap(),
            "_CON (2).zip"
        );
        assert_eq!(fs::read(first.reference).unwrap(), b"hello");
        assert_eq!(fs::read(second.reference).unwrap(), b"hello");
        assert_eq!(name("../.."), "tau-attachment");
        assert_eq!(mime("image.PNG"), "image/png");
    }
}
