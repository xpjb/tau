#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(not(target_os = "android"))]
fn main() {
    if let Err(error) = tau_frontend::run() {
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let message = error.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    message.as_ptr(),
                    windows_sys::core::w!("Tau Beta"),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        #[cfg(not(windows))]
        eprintln!("Tau Beta: {error}");
        std::process::exit(1);
    }
}
#[cfg(target_os = "android")]
fn main() {}
