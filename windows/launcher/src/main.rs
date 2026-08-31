#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    use std::path::PathBuf;
    use std::process::Command;

    use std::os::windows::process::CommandExt;

    let result = (|| {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "Windows did not provide LOCALAPPDATA".to_owned())?;
        let root = local.join("Tau");
        let current = std::fs::read_to_string(root.join("current.txt"))
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
        let java = version_root.join("runtime").join("bin").join("javaw.exe");
        if !java.is_file() {
            return Err(format!("Tau runtime is missing: {}", java.display()));
        }
        let classpath = version_root.join("app").join("lib").join("*");
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
        use std::ptr::null_mut;

        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MB_ICONERROR, MB_OK, MessageBoxW,
        };

        let message = error
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let title = "Tau"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        unsafe {
            MessageBoxW(
                null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Tau's launcher targets Windows");
}
