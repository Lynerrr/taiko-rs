// The Windows exe's icon (Explorer, the taskbar, shortcuts), version details and manifest:
// assets/taiko-rs.rc. The window's icon at run time is set apart from this (`logo::window_icon`).
fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/taiko-rs.rc");
    println!("cargo:rerun-if-changed=assets/taiko-rs.ico");
    println!("cargo:rerun-if-changed=assets/taiko-rs.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let part = |name: &str| std::env::var(format!("CARGO_PKG_VERSION_{name}")).expect("cargo sets the version");
        let (major, minor, patch) = (part("MAJOR"), part("MINOR"), part("PATCH"));
        let macros = [format!("VERSION_NUMBERS={major},{minor},{patch},0"), format!("VERSION_TEXT={major}.{minor}.{patch}")];
        embed_resource::compile("assets/taiko-rs.rc", macros).manifest_required().expect("the exe's resources (assets/taiko-rs.rc)");
    }
}
