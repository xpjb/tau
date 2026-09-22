#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
#[path = "../../channel.rs"]
mod channel;

#[cfg(windows)]
fn main() {
    use std::{
        fs,
        os::windows::process::CommandExt,
        path::{Component, Path, PathBuf},
        process::Command,
    };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let root =
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
                .join(channel::NAME);
        let current = fs::read_to_string(root.join("current.txt"))?;
        let key = current.trim();
        if key.is_empty()
            || key.len() > 128
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
            || !matches!(
                Path::new(key).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err("Invalid active version. Run Tau Beta setup again.".into());
        }
        let version = root.join("versions").join(key);
        Command::new(version.join("app").join(channel::EXE))
            .args(std::env::args_os().skip(1))
            .current_dir(&version)
            .creation_flags(0x0800_0000)
            .spawn()?;
        Ok(())
    })();
    if let Err(error) = result {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let message = format!("Tau Beta could not start. Run its setup again.\n\n{error}")
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let title = channel::NAME
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Tau Beta's launcher targets Windows");
}
