#[allow(dead_code)]
#[path = "../channel.rs"]
mod channel;
fn main() {
    println!("cargo:rerun-if-env-changed=TAU_VERSION");
    let icon = "../../frontend/assets/tau-beta.ico";
    println!("cargo:rerun-if-changed={icon}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join(icon);
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
        .set_icon(icon.to_str().unwrap())
        .set("FileDescription", &format!("{} Setup", channel::NAME))
        .set("ProductName", channel::NAME)
        .set("OriginalFilename", &format!("{} Setup.exe", channel::NAME));
    if let Ok(version) = std::env::var("TAU_VERSION") {
        resource
            .set("FileVersion", &version)
            .set("ProductVersion", &version);
        let v: Vec<u64> = version
            .split('.')
            .map(|v| v.parse().expect("numeric release version"))
            .collect();
        assert_eq!(v.len(), 3);
        let packed = (v[0] << 48) | (v[1] << 32) | (v[2] << 16);
        resource
            .set_version_info(winresource::VersionInfo::FILEVERSION, packed)
            .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
    }
    resource
        .compile()
        .expect("failed to embed Windows resources");
}
