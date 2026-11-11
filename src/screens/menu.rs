//! What lib.rs showed on the main menu: the update badge.

use crate::update;

/// The badge for an update to install.
pub(crate) fn update_badge(commit: &str) -> String {
    let build = update::short(commit);
    tr!("update {build}  Ctrl+U", "アップデート {build}  Ctrl+U", "обновить {build}  Ctrl+U"; [] [build])
}

/// The badge while the update downloads (`p` 0..1).
pub(crate) fn updating_badge(p: f32) -> String {
    let p = p * 100.0;
    tr!("updating {p:.0}%", "更新中 {p:.0}%", "обновление {p:.0}%"; [] [p])
}
