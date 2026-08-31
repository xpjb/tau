#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(windows)]
static PAYLOAD: &[u8] = include_bytes!(env!("TAU_PAYLOAD_ZIP"));
#[cfg(windows)]
static LAUNCHER: &[u8] = include_bytes!(env!("TAU_LAUNCHER_EXE"));
#[cfg(windows)]
const VERSION: &str = env!("TAU_VERSION");

#[cfg(windows)]
fn main() {
    let result = install();
    match result {
        Ok(message) => {
            show_message(&message, false);
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                let _ = std::process::Command::new(
                    std::path::PathBuf::from(local).join("Tau").join("Tau.exe"),
                )
                .spawn();
            }
        }
        Err(error) => show_message(&format!("Tau setup failed.\n\n{error}"), true),
    }
}

#[cfg(windows)]
fn install() -> Result<String, Box<dyn std::error::Error>> {
    use std::fs;
    use std::io::{Cursor, Write};
    use std::path::PathBuf;

    use semver::Version;
    use sha2::{Digest, Sha256};

    let bundled_version = Version::parse(VERSION)?;
    let local = std::env::var_os("LOCALAPPDATA").ok_or("Windows did not provide LOCALAPPDATA")?;
    let root = PathBuf::from(local).join("Tau");
    let versions = root.join("versions");
    fs::create_dir_all(&versions)?;

    let payload_hash = Sha256::digest(PAYLOAD)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let version_key = format!("{VERSION}-{}", &payload_hash[..12]);
    if version_key
        .chars()
        .any(|character| !character.is_ascii_alphanumeric() && !matches!(character, '.' | '-'))
    {
        return Err("Tau's build version cannot be used as an install key".into());
    }

    let current_key = fs::read_to_string(root.join("current.txt"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '-')
                })
        });
    let current_version = current_key.as_ref().and_then(|key| {
        fs::read_to_string(versions.join(key).join("version.txt"))
            .ok()
            .and_then(|value| Version::parse(value.trim()).ok())
    });
    if current_version.as_ref().is_some_and(|current| current > &bundled_version) {
        return Ok(format!(
            "Tau {} is already installed, which is newer than this {} setup.",
            current_version.expect("checked above"),
            bundled_version,
        ));
    }

    let target = versions.join(&version_key);
    let target_is_complete = target.is_dir()
        && fs::read_to_string(target.join("version.txt"))
            .is_ok_and(|value| value.trim() == VERSION)
        && fs::read_to_string(target.join("payload.sha256"))
            .is_ok_and(|value| value.trim() == payload_hash);
    let launcher_is_current = fs::read(root.join("Tau.exe")).is_ok_and(|bytes| bytes == LAUNCHER);
    if current_key.as_deref() == Some(version_key.as_str())
        && target_is_complete
        && launcher_is_current
    {
        return Ok(format!("Tau {VERSION} is already installed."));
    }

    if !target_is_complete {
        if target.exists() {
            return Err(format!(
                "{} exists but is incomplete; remove it and run setup again",
                target.display()
            )
            .into());
        }
        let staging = versions.join(format!(".{version_key}.install-{}", std::process::id()));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir_all(&staging)?;
        let extraction = (|| {
            let mut archive = zip::ZipArchive::new(Cursor::new(PAYLOAD))?;
            let mut expanded_bytes = 0_u64;
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index)?;
                let relative = entry
                    .enclosed_name()
                    .ok_or_else(|| format!("unsafe payload path: {}", entry.name()))?
                    .to_owned();
                if entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
                {
                    return Err(format!("payload contains a symbolic link: {}", entry.name()).into());
                }
                expanded_bytes = expanded_bytes.saturating_add(entry.size());
                if expanded_bytes > 512 * 1024 * 1024 {
                    return Err("Tau payload exceeds its extraction limit".into());
                }
                let output = staging.join(relative);
                if entry.is_dir() {
                    fs::create_dir_all(&output)?;
                    continue;
                }
                if let Some(parent) = output.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut file = fs::File::create(&output)?;
                std::io::copy(&mut entry, &mut file)?;
                file.flush()?;
            }
            if !staging.join("runtime").join("bin").join("javaw.exe").is_file() {
                return Err("Tau payload has no Windows Java runtime".into());
            }
            let library_directory = staging.join("app").join("lib");
            if !fs::read_dir(&library_directory)?.any(|entry| {
                entry
                    .ok()
                    .is_some_and(|entry| entry.path().extension().is_some_and(|value| value == "jar"))
            }) {
                return Err("Tau payload has no application libraries".into());
            }
            fs::write(staging.join("version.txt"), format!("{VERSION}\n"))?;
            fs::write(
                staging.join("payload.sha256"),
                format!("{payload_hash}\n"),
            )?;
            Ok::<_, Box<dyn std::error::Error>>(())
        })();
        if let Err(error) = extraction {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        if let Err(error) = fs::rename(&staging, &target) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
    }

    fs::create_dir_all(&root)?;
    write_atomic(&root.join("Tau.exe"), LAUNCHER)?;
    if let Some(roaming) = std::env::var_os("APPDATA") {
        let programs = PathBuf::from(roaming)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        fs::create_dir_all(&programs)?;
        write_atomic(&programs.join("Tau.exe"), LAUNCHER)?;
    }
    write_atomic(&root.join("current.txt"), format!("{version_key}\n").as_bytes())?;

    let message = match current_version {
        Some(previous) if previous < bundled_version => {
            format!("Tau was updated from {previous} to {bundled_version}.")
        }
        Some(_) => format!("Tau {bundled_version} was repaired and activated."),
        None => format!("Tau {bundled_version} was installed."),
    };
    Ok(message)
}

#[cfg(windows)]
fn write_atomic(
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let temporary = path.with_file_name(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|name| name.to_str()).unwrap_or("tau"),
        std::process::id(),
    ));
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    let source = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let error = std::io::Error::last_os_error();
        let _ = std::fs::remove_file(temporary);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(windows)]
fn show_message(message: &str, error: bool) {
    use std::ptr::null_mut;

    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW,
    };

    let message = message
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let title = "Tau Setup"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MessageBoxW(
            null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | if error { MB_ICONERROR } else { MB_ICONINFORMATION },
        );
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Tau setup targets Windows");
}
