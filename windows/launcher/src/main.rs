#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(windows)]
const RUNTIME_KEY: &str = "temurin-21.0.12.1+1-x64";
#[cfg(windows)]
const RUNTIME_URL: &str = "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/OpenJDK21U-jre_x64_windows_hotspot_21.0.12.1_1.zip";
#[cfg(windows)]
const RUNTIME_SHA256: &str = "d35f31e712f0fcf6ac5a093edc90204fbff22f720ba3950bd09d331d5e621636";
#[cfg(windows)]
const RUNTIME_ARCHIVE_BYTES: u64 = 48_999_141;

#[cfg(windows)]
fn main() {
    use std::ffi::OsString;
    use std::fs;
    use std::io::{Read, Write};
    use std::path::{Component, PathBuf};
    use std::process::Command;

    use sha2::{Digest, Sha256};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Com::Urlmon::URLDownloadToFileW;

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
