#[cfg(not(target_os = "android"))]
fn main() -> Result<(), String> {
    tau_frontend::run()
}
#[cfg(target_os = "android")]
fn main() {}
