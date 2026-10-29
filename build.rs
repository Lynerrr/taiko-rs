// The Windows exe's own icon (Explorer, the taskbar, shortcuts): assets/taiko-rs.ico as a
// resource. The window's icon at run time is set apart from this (`logo::window_icon`).
fn main() {
    println!("cargo:rerun-if-changed=assets/taiko-rs.rc");
    println!("cargo:rerun-if-changed=assets/taiko-rs.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/taiko-rs.rc", embed_resource::NONE).manifest_required().expect("the exe's icon (assets/taiko-rs.rc)");
    }
}
