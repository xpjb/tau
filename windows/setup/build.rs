fn main() {
    println!("cargo:rerun-if-changed=../../assets/tau.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../assets/tau.ico");
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(icon.to_str().unwrap())
        .set("FileDescription", "Tau Setup")
        .set("ProductName", "Tau")
        .set("OriginalFilename", "Tau-Setup.exe");
    resource.compile().expect("failed to embed Tau's Windows resources");
}
