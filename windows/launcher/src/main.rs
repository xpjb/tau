#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(any(windows, test))]
use std::ffi::{OsStr, OsString};
#[cfg(any(windows, test))]
use std::fs;
#[cfg(any(windows, test))]
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(any(windows, test))]
use std::path::{Component, Path, PathBuf};

#[cfg(windows)]
const ATTACHMENT_ARCHIVE_BYTES: u64 = 50_000_000;
#[cfg(windows)]
const ATTACHMENT_EXPANDED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
#[cfg(windows)]
const ATTACHMENT_MAX_ENTRIES: usize = 20_000;

#[cfg(windows)]
const RUNTIME_KEY: &str = "temurin-21.0.12.1+1-x64";
#[cfg(windows)]
const RUNTIME_URL: &str = "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/OpenJDK21U-jre_x64_windows_hotspot_21.0.12.1_1.zip";
#[cfg(windows)]
const RUNTIME_SHA256: &str = "d35f31e712f0fcf6ac5a093edc90204fbff22f720ba3950bd09d331d5e621636";
#[cfg(windows)]
const RUNTIME_ARCHIVE_BYTES: u64 = 48_999_141;

#[cfg(any(windows, test))]
fn archive_components(path: &Path) -> Result<Vec<OsString>, String> {
    let mut names = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err("ZIP contains an unsafe path".to_owned());
        };
        let name = name
            .to_str()
            .ok_or_else(|| "ZIP contains a name Windows cannot represent".to_owned())?;
        if name.is_empty()
            || name.ends_with(' ')
            || name.ends_with('.')
            || name.chars().any(|character| {
                (character as u32) < 32
                    || matches!(character, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            })
        {
            return Err("ZIP contains an unsafe Windows name".to_owned());
        }
        let device = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
        if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || device
                .strip_prefix("COM")
                .is_some_and(|number| matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
            || device
                .strip_prefix("LPT")
                .is_some_and(|number| matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        {
            return Err("ZIP contains a reserved Windows name".to_owned());
        }
        names.push(OsString::from(name));
    }
    if names.is_empty() {
        return Err("ZIP contains an empty path".to_owned());
    }
    Ok(names)
}

#[cfg(any(windows, test))]
fn validate_central_directory(path: &Path, max_entries: usize) -> Result<(), String> {
    let archive_file = fs::File::open(path)
        .map_err(|error| format!("ZIP cannot be opened: {error}"))?;
    let archive = zip::ZipArchive::new(archive_file)
        .map_err(|error| format!("ZIP is invalid: {error}"))?;
    let mut file = fs::File::open(path)
        .map_err(|error| format!("ZIP cannot be opened: {error}"))?;
    file.seek(SeekFrom::Start(archive.central_directory_start()))
        .map_err(|error| format!("ZIP central directory cannot be read: {error}"))?;
    let mut names = std::collections::HashSet::new();
    let mut count = 0_usize;
    loop {
        let mut signature = [0_u8; 4];
        file.read_exact(&mut signature)
            .map_err(|error| format!("ZIP central directory is truncated: {error}"))?;
        match u32::from_le_bytes(signature) {
            0x0201_4b50 => {
                let mut header = [0_u8; 42];
                file.read_exact(&mut header)
                    .map_err(|error| format!("ZIP central entry is truncated: {error}"))?;
                let name_bytes = u16::from_le_bytes([header[24], header[25]]) as usize;
                let extra_bytes = u16::from_le_bytes([header[26], header[27]]) as u64;
                let comment_bytes = u16::from_le_bytes([header[28], header[29]]) as u64;
                if name_bytes == 0 {
                    return Err("ZIP contains an empty path".to_owned());
                }
                let mut name = vec![0_u8; name_bytes];
                file.read_exact(&mut name)
                    .map_err(|error| format!("ZIP entry name is truncated: {error}"))?;
                if !names.insert(name) {
                    return Err("ZIP contains a duplicate path".to_owned());
                }
                count += 1;
                if count > max_entries {
                    return Err(format!("ZIP contains more than {max_entries} entries"));
                }
                let skip = extra_bytes
                    .checked_add(comment_bytes)
                    .and_then(|bytes| i64::try_from(bytes).ok())
                    .ok_or_else(|| "ZIP central entry is too large".to_owned())?;
                file.seek(SeekFrom::Current(skip))
                    .map_err(|error| format!("ZIP central entry is truncated: {error}"))?;
            }
            0x0605_4b50 | 0x0606_4b50 | 0x0505_4b50 => break,
            _ => return Err("ZIP central directory is invalid".to_owned()),
        }
    }
    if count == 0 {
        return Err("ZIP is empty".to_owned());
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn inspect_zip_archive(path: &Path, max_entries: usize) -> Result<Option<OsString>, String> {
    validate_central_directory(path, max_entries)?;
    let archive_file = fs::File::open(path)
        .map_err(|error| format!("ZIP cannot be opened: {error}"))?;
    let mut archive = zip::ZipArchive::new(archive_file)
        .map_err(|error| format!("ZIP is invalid: {error}"))?;
    if archive.is_empty() {
        return Err("ZIP is empty".to_owned());
    }
    if archive.len() > max_entries {
        return Err(format!("ZIP contains more than {max_entries} entries"));
    }

    let mut common_root: Option<OsString> = None;
    let mut one_root = true;
    let mut root_is_file = false;
    let mut root_is_directory = false;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("ZIP entry is invalid: {error}"))?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| "ZIP contains an unsafe path".to_owned())?;
        let components = archive_components(&enclosed)?;
        let first = &components[0];
        if let Some(root) = common_root.as_ref() {
            if root != first {
                one_root = false;
            }
        } else {
            common_root = Some(first.clone());
        }
        if components.len() > 1 || entry.is_dir() {
            root_is_directory = true;
        } else {
            root_is_file = true;
        }
    }

    Ok((one_root && root_is_directory && !root_is_file)
        .then_some(common_root)
        .flatten())
}

#[cfg(any(windows, test))]
fn extract_zip_archive(
    path: &Path,
    destination: &Path,
    strip_root: Option<&OsStr>,
    max_entries: usize,
    max_expanded_bytes: u64,
) -> Result<(), String> {
    validate_central_directory(path, max_entries)?;
    let archive_file = fs::File::open(path)
        .map_err(|error| format!("ZIP cannot be opened: {error}"))?;
    let mut archive = zip::ZipArchive::new(archive_file)
        .map_err(|error| format!("ZIP is invalid: {error}"))?;
    if archive.is_empty() {
        return Err("ZIP is empty".to_owned());
    }
    if archive.len() > max_entries {
        return Err(format!("ZIP contains more than {max_entries} entries"));
    }

    let mut expanded_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("ZIP entry is invalid: {error}"))?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("ZIP contains a symbolic link".to_owned());
        }
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| "ZIP contains an unsafe path".to_owned())?;
        let components = archive_components(&enclosed)?;
        let first_output_component = if let Some(root) = strip_root {
            if components[0] != root {
                return Err("ZIP does not have the expected root folder".to_owned());
            }
            1
        } else {
            0
        };
        if first_output_component == components.len() {
            if entry.is_dir() {
                continue;
            }
            return Err("ZIP root is both a file and a folder".to_owned());
        }

        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .ok_or_else(|| "ZIP expanded size overflowed".to_owned())?;
        if expanded_bytes > max_expanded_bytes {
            return Err(format!(
                "ZIP expands beyond the {} byte safety limit",
                max_expanded_bytes,
            ));
        }
        let mut output = destination.to_path_buf();
        for component in &components[first_output_component..] {
            output.push(component);
        }
        if entry.is_dir() {
            fs::create_dir_all(&output)
                .map_err(|error| format!("ZIP directory cannot be created: {error}"))?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("ZIP directory cannot be created: {error}"))?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|error| format!("ZIP file cannot be created: {error}"))?;
        let copied = std::io::copy(&mut entry, &mut file)
            .map_err(|error| format!("ZIP file cannot be extracted: {error}"))?;
        if copied != entry.size() {
            return Err("ZIP entry has an inconsistent expanded size".to_owned());
        }
        file.flush()
            .map_err(|error| format!("ZIP file cannot be saved: {error}"))?;
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn activate_extraction(
    staging: &Path,
    parent: &Path,
    folder_name: &OsStr,
) -> Result<PathBuf, String> {
    let folder_text = folder_name.to_string_lossy();
    let mut suffix = 1_u32;
    loop {
        let candidate = if suffix == 1 {
            parent.join(folder_name)
        } else {
            parent.join(format!("{folder_text} ({suffix})"))
        };
        if candidate.exists() {
            suffix = suffix.saturating_add(1);
            continue;
        }
        match fs::rename(staging, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(_error) if candidate.exists() => {
                suffix = suffix.saturating_add(1);
            }
            Err(error) => {
                let _ = fs::remove_dir_all(staging);
                return Err(format!("The extracted folder cannot be activated: {error}"));
            }
        }
    }
}

#[cfg(windows)]
fn open_with_windows_shell(path: &OsStr) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let operation = "open"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let path = path
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let opened = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if opened as usize <= 32 {
        return Err(format!(
            "Windows could not open the requested item (shell error {}).",
            opened as usize,
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn main() {
    use std::io::Read;
    use std::process::Command;

    use sha2::{Digest, Sha256};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Com::Urlmon::URLDownloadToFileW;

    let mut arguments = std::env::args_os().skip(1);
    let action = arguments.next();
    if action.as_deref() == Some(OsStr::new("--open")) {
        let file = match (arguments.next(), arguments.next()) {
            (Some(file), None) => file,
            _ => {
                show_message("Tau received an invalid file-open request.", true);
                return;
            }
        };
        let path = PathBuf::from(file);
        if !path.is_file() {
            show_message("The downloaded file no longer exists.", true);
            return;
        }
        if let Err(error) = open_with_windows_shell(path.as_os_str()) {
            show_message(&error, true);
        }
        return;
    }
    if action.as_deref() == Some(OsStr::new("--show")) {
        let file = match (arguments.next(), arguments.next()) {
            (Some(file), None) => file,
            _ => {
                show_message("Tau received an invalid show-file request.", true);
                return;
            }
        };
        let path = PathBuf::from(file);
        if !path.is_file() {
            show_message("The downloaded file no longer exists.", true);
            return;
        }
        let mut selection = OsString::from("/select,");
        selection.push(path.as_os_str());
        if let Err(error) = Command::new("explorer.exe")
            .arg(selection)
            .creation_flags(0x0800_0000)
            .spawn()
        {
            show_message(
                &format!("Windows could not show the downloaded file: {error}"),
                true,
            );
        }
        return;
    }
    if action.as_deref() == Some(OsStr::new("--extract-open")) {
        let file = match (arguments.next(), arguments.next()) {
            (Some(file), None) => file,
            _ => {
                show_message("Tau received an invalid ZIP extraction request.", true);
                return;
            }
        };
        let archive_path = PathBuf::from(file);
        let result = (|| {
            if !archive_path.is_file() {
                return Err("The downloaded ZIP no longer exists.".to_owned());
            }
            if !archive_path
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
            {
                return Err("The downloaded file is not a ZIP archive.".to_owned());
            }
            let archive_bytes = fs::metadata(&archive_path)
                .map_err(|error| format!("The downloaded ZIP cannot be inspected: {error}"))?
                .len();
            if archive_bytes > ATTACHMENT_ARCHIVE_BYTES {
                return Err(format!(
                    "The downloaded ZIP exceeds the {} byte safety limit.",
                    ATTACHMENT_ARCHIVE_BYTES,
                ));
            }
            let parent = archive_path
                .parent()
                .filter(|parent| parent.is_dir())
                .ok_or_else(|| "The downloaded ZIP has no usable parent folder.".to_owned())?;
            let common_root = inspect_zip_archive(&archive_path, ATTACHMENT_MAX_ENTRIES)?;
            let folder_name = common_root.clone().unwrap_or_else(|| {
                archive_path
                    .file_stem()
                    .filter(|stem| !stem.is_empty())
                    .unwrap_or_else(|| OsStr::new("archive"))
                    .to_os_string()
            });
            archive_components(Path::new(&folder_name))?;

            let process = std::process::id();
            let mut staging_suffix = 1_u32;
            let staging = loop {
                let name = if staging_suffix == 1 {
                    format!(".tau-extract-{process}")
                } else {
                    format!(".tau-extract-{process}-{staging_suffix}")
                };
                let candidate = parent.join(name);
                match fs::create_dir(&candidate) {
                    Ok(()) => break candidate,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        staging_suffix = staging_suffix.saturating_add(1);
                    }
                    Err(error) => {
                        return Err(format!("The extraction folder cannot be created: {error}"));
                    }
                }
            };
            let extraction = extract_zip_archive(
                &archive_path,
                &staging,
                common_root.as_deref(),
                ATTACHMENT_MAX_ENTRIES,
                ATTACHMENT_EXPANDED_BYTES,
            );
            if let Err(error) = extraction {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }

            let extracted = activate_extraction(&staging, parent, &folder_name)?;
            open_with_windows_shell(extracted.as_os_str())?;
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            show_message(&format!("Tau could not extract the downloaded ZIP: {error}"), true);
        }
        return;
    }

    let result = (|| {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "Windows did not provide LOCALAPPDATA".to_owned())?;
        let root = local.join("Tau");
        let current = fs::read_to_string(root.join("current.txt"))
            .map_err(|error| format!("Tau is not installed correctly: {error}"))?;
        let current = current.trim();
        if current.is_empty()
            || current.len() > 128
            || current
                .chars()
                .any(|character| !character.is_ascii_alphanumeric() && !matches!(character, '.' | '-'))
        {
            return Err("Tau's active version marker is invalid".to_owned());
        }
        let version_root = root.join("versions").join(current);
        let library_directory = version_root.join("app").join("lib");
        if !library_directory.is_dir() {
            return Err(format!(
                "Tau application files are missing: {}",
                library_directory.display(),
            ));
        }

        let runtimes = root.join("runtimes");
        let runtime = runtimes.join(RUNTIME_KEY);
        if !runtime_is_complete(&runtime) {
            show_message(
                "Tau needs to download its private Java 21 runtime once (about 47 MB). Later Tau updates will reuse it.",
                false,
            );
            fs::create_dir_all(&runtimes)
                .map_err(|error| format!("Tau could not create its runtime directory: {error}"))?;
            if runtime.exists() {
                fs::remove_dir_all(&runtime)
                    .map_err(|error| format!("Tau could not replace its incomplete runtime: {error}"))?;
            }

            let process = std::process::id();
            let staging = runtimes.join(format!(".{RUNTIME_KEY}.install-{process}"));
            let archive_path = runtimes.join(format!(".{RUNTIME_KEY}.download-{process}.zip"));
            let _ = fs::remove_dir_all(&staging);
            let _ = fs::remove_file(&archive_path);
            fs::create_dir_all(&staging)
                .map_err(|error| format!("Tau could not stage its runtime: {error}"))?;

            let installation = (|| {
                let url = RUNTIME_URL
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect::<Vec<_>>();
                let destination = archive_path
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect::<Vec<_>>();
                let download = unsafe {
                    URLDownloadToFileW(
                        std::ptr::null_mut(),
                        url.as_ptr(),
                        destination.as_ptr(),
                        0,
                        std::ptr::null_mut(),
                    )
                };
                if download < 0 {
                    return Err(format!("the runtime download failed with Windows error 0x{download:08x}"));
                }
                let archive_size = fs::metadata(&archive_path)
                    .map_err(|error| format!("the runtime download is missing: {error}"))?
                    .len();
                if archive_size != RUNTIME_ARCHIVE_BYTES {
                    return Err(format!(
                        "the runtime download has an unexpected size ({archive_size} bytes)",
                    ));
                }

                let mut archive_file = fs::File::open(&archive_path)
                    .map_err(|error| format!("the runtime download cannot be opened: {error}"))?;
                let mut hasher = Sha256::new();
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let count = archive_file
                        .read(&mut buffer)
                        .map_err(|error| format!("the runtime download cannot be verified: {error}"))?;
                    if count == 0 {
                        break;
                    }
                    hasher.update(&buffer[..count]);
                }
                let archive_hash = hasher
                    .finalize()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                if archive_hash != RUNTIME_SHA256 {
                    return Err("the runtime download failed its SHA-256 check".to_owned());
                }

                let archive_file = fs::File::open(&archive_path)
                    .map_err(|error| format!("the runtime download cannot be opened: {error}"))?;
                let mut archive = zip::ZipArchive::new(archive_file)
                    .map_err(|error| format!("the runtime ZIP is invalid: {error}"))?;
                let mut archive_root: Option<OsString> = None;
                let mut expanded_bytes = 0_u64;
                if archive.len() > 4096 {
                    return Err("the runtime ZIP contains too many entries".to_owned());
                }
                for index in 0..archive.len() {
                    let mut entry = archive
                        .by_index(index)
                        .map_err(|error| format!("the runtime ZIP entry is invalid: {error}"))?;
                    let enclosed = entry
                        .enclosed_name()
                        .ok_or_else(|| "the runtime ZIP contains an unsafe path".to_owned())?;
                    let mut components = enclosed.components();
                    let first = match components.next() {
                        Some(Component::Normal(value)) => value.to_os_string(),
                        _ => return Err("the runtime ZIP contains an unsafe root".to_owned()),
                    };
                    if let Some(expected) = archive_root.as_ref() {
                        if expected != &first {
                            return Err("the runtime ZIP contains multiple roots".to_owned());
                        }
                    } else {
                        archive_root = Some(first);
                    }
                    if entry
                        .unix_mode()
                        .is_some_and(|mode| mode & 0o170000 == 0o120000)
                    {
                        return Err("the runtime ZIP contains a symbolic link".to_owned());
                    }
                    let relative = components.collect::<PathBuf>();
                    if relative.as_os_str().is_empty() {
                        continue;
                    }
                    expanded_bytes = expanded_bytes.saturating_add(entry.size());
                    if expanded_bytes > 256 * 1024 * 1024 {
                        return Err("the runtime exceeds its extraction limit".to_owned());
                    }
                    let output = staging.join(relative);
                    if entry.is_dir() {
                        fs::create_dir_all(&output)
                            .map_err(|error| format!("the runtime directory cannot be created: {error}"))?;
                        continue;
                    }
                    if let Some(parent) = output.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|error| format!("the runtime directory cannot be created: {error}"))?;
                    }
                    let mut file = fs::File::create(&output)
                        .map_err(|error| format!("the runtime file cannot be created: {error}"))?;
                    std::io::copy(&mut entry, &mut file)
                        .map_err(|error| format!("the runtime file cannot be extracted: {error}"))?;
                    file.flush()
                        .map_err(|error| format!("the runtime file cannot be saved: {error}"))?;
                }
                if !staging.join("bin").join("javaw.exe").is_file() {
                    return Err("the runtime ZIP has no javaw.exe".to_owned());
                }
                fs::write(
                    staging.join("archive.sha256"),
                    format!("{RUNTIME_SHA256}\n"),
                )
                .map_err(|error| format!("the runtime marker cannot be saved: {error}"))?;
                if let Err(error) = fs::rename(&staging, &runtime) {
                    if runtime_is_complete(&runtime) {
                        let _ = fs::remove_dir_all(&staging);
                    } else {
                        return Err(format!("the runtime could not be activated: {error}"));
                    }
                }
                Ok::<_, String>(())
            })();
            let _ = fs::remove_file(&archive_path);
            if installation.is_err() {
                let _ = fs::remove_dir_all(&staging);
            }
            installation.map_err(|error| format!("Tau could not install its private runtime: {error}"))?;
        }

        let java = runtime.join("bin").join("javaw.exe");
        let classpath = library_directory.join("*");
        Command::new(java)
            .arg("-Dfile.encoding=UTF-8")
            .arg("-cp")
            .arg(classpath)
            .arg("app.tau.MainKt")
            .current_dir(version_root)
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|error| format!("Tau could not start: {error}"))?;
        Ok::<_, String>(())
    })();

    if let Err(error) = result {
        show_message(&error, true);
    }
}

#[cfg(windows)]
fn runtime_is_complete(path: &std::path::Path) -> bool {
    path.join("bin").join("javaw.exe").is_file()
        && std::fs::read_to_string(path.join("archive.sha256"))
            .is_ok_and(|value| value.trim() == RUNTIME_SHA256)
}

#[cfg(windows)]
fn show_message(message: &str, error: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW,
    };

    let message = message
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let title = "Tau"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | if error { MB_ICONERROR } else { MB_ICONINFORMATION },
        );
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Tau's launcher targets Windows");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use zip::write::SimpleFileOptions;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tau-launcher-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    type ZipEntry<'a> = (&'a str, Option<&'a [u8]>, Option<u32>);

    fn write_zip(path: &Path, entries: &[ZipEntry<'_>]) {
        let file = fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        for (name, content, mode) in entries {
            let mut options = SimpleFileOptions::default();
            if let Some(mode) = mode {
                options = options.unix_permissions(*mode);
            }
            if mode.is_some_and(|mode| mode & 0o170000 == 0o120000) {
                archive
                    .add_symlink(
                        *name,
                        String::from_utf8_lossy(content.unwrap_or_default()),
                        options,
                    )
                    .unwrap();
            } else if let Some(content) = content {
                archive.start_file(*name, options).unwrap();
                archive.write_all(content).unwrap();
            } else {
                archive.add_directory(*name, options).unwrap();
            }
        }
        archive.finish().unwrap();
    }

    #[test]
    fn extracts_a_single_root_without_an_extra_wrapper() {
        let root = TestDirectory::new();
        let archive = root.0.join("project.zip");
        write_zip(
            &archive,
            &[
                ("project/", None, None),
                ("project/src/main.rs", Some(b"fn main() {}"), None),
            ],
        );
        let common = inspect_zip_archive(&archive, 10).unwrap();
        assert_eq!(common.as_deref(), Some(OsStr::new("project")));
        let staging = root.0.join("staging");
        fs::create_dir(&staging).unwrap();
        extract_zip_archive(&archive, &staging, common.as_deref(), 10, 1_024).unwrap();
        assert!(staging.join("src/main.rs").is_file());
        assert!(!staging.join("project").exists());
    }

    #[test]
    fn keeps_loose_roots_and_activates_with_a_collision_suffix() {
        let root = TestDirectory::new();
        let archive = root.0.join("bundle.zip");
        write_zip(
            &archive,
            &[
                ("README.md", Some(b"read me"), None),
                ("src/main.rs", Some(b"fn main() {}"), None),
            ],
        );
        assert_eq!(inspect_zip_archive(&archive, 10).unwrap(), None);
        let staging = root.0.join("staging");
        fs::create_dir(&staging).unwrap();
        extract_zip_archive(&archive, &staging, None, 10, 1_024).unwrap();
        assert!(staging.join("README.md").is_file());
        assert!(staging.join("src/main.rs").is_file());
        fs::create_dir(root.0.join("bundle")).unwrap();
        let activated = activate_extraction(&staging, &root.0, OsStr::new("bundle")).unwrap();
        assert_eq!(activated.file_name(), Some(OsStr::new("bundle (2)")));
        assert!(activated.join("README.md").is_file());
    }

    #[test]
    fn rejects_unsafe_names_links_and_duplicate_files() {
        let root = TestDirectory::new();
        for (number, name) in ["../escape", "..\\escape", "CON.txt", "trailing. "]
            .into_iter()
            .enumerate()
        {
            let archive = root.0.join(format!("unsafe-{number}.zip"));
            write_zip(&archive, &[(name, Some(b"unsafe"), None)]);
            assert!(inspect_zip_archive(&archive, 10).is_err(), "accepted {name}");
        }

        let symlink = root.0.join("symlink.zip");
        write_zip(&symlink, &[("link", Some(b"target"), Some(0o120777))]);
        let destination = root.0.join("symlink-output");
        fs::create_dir(&destination).unwrap();
        assert!(extract_zip_archive(&symlink, &destination, None, 10, 1_024).is_err());

        let duplicate = root.0.join("duplicate.zip");
        write_zip(
            &duplicate,
            &[("same1.txt", Some(b"one"), None), ("same2.txt", Some(b"two"), None)],
        );
        let mut bytes = fs::read(&duplicate).unwrap();
        let mut replacements = 0;
        for index in 0..bytes.len().saturating_sub(8) {
            if &bytes[index..index + 9] == b"same1.txt"
                || &bytes[index..index + 9] == b"same2.txt"
            {
                bytes[index + 4] = b'0';
                replacements += 1;
            }
        }
        assert_eq!(replacements, 4);
        fs::write(&duplicate, bytes).unwrap();
        let destination = root.0.join("duplicate-output");
        fs::create_dir(&destination).unwrap();
        assert!(extract_zip_archive(&duplicate, &destination, None, 10, 1_024).is_err());
    }

    #[test]
    fn enforces_entry_and_expanded_size_limits() {
        let root = TestDirectory::new();
        let archive = root.0.join("limits.zip");
        write_zip(
            &archive,
            &[("one", Some(b"1234"), None), ("two", Some(b"5678"), None)],
        );
        assert!(inspect_zip_archive(&archive, 1).is_err());
        let destination = root.0.join("output");
        fs::create_dir(&destination).unwrap();
        assert!(extract_zip_archive(&archive, &destination, None, 10, 7).is_err());
    }
}
