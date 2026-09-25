fn main() {
    println!("cargo:rerun-if-changed=assets/tau-beta.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Non-debug releases do not ship PDBs. Do not ask the MSVC linker to
    // assemble debug data from SDK objects whose vendor PDBs are absent.
    // Keep normal debug/symbol-enabled builds unchanged; this is not /IGNORE.
    if std::env::var("PROFILE").as_deref() == Ok("release")
        && std::env::var("DEBUG").as_deref() == Ok("false")
    {
        println!("cargo:rustc-link-arg=/DEBUG:NONE");
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
