//! What lib.rs showed on the main menu: the update badge.

/// The badge for an update to install: the build called `build` (`Status::Available::name`).
pub(crate) fn update_badge(build: &str) -> String {
    tr!("update {build}  Ctrl+U", "アップデート {build}  Ctrl+U", "обновить {build}  Ctrl+U"; [] [build])
}

/// The badge while the update downloads (`p` 0..1).
pub(crate) fn updating_badge(p: f32) -> String {
    let p = p * 100.0;
    tr!("updating {p:.0}%", "更新中 {p:.0}%", "обновление {p:.0}%"; [] [p])
}
