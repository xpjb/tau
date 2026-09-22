fn main() {
    println!("cargo:rerun-if-changed=assets/tau-beta.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/tau-beta.ico")
        .set("FileDescription", "Tau Beta")
        .set("ProductName", "Tau Beta")
        .set("OriginalFilename", "Tau Beta.exe");
    resource
        .compile()
        .expect("failed to embed Tau Beta's Windows resources");
}
