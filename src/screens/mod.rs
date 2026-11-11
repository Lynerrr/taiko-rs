//! The screens, each in its own module (docs/client-architecture.md, 4.5). For now they hold
//! what lib.rs drew and did for each screen; the screens themselves follow (step 7).

pub(crate) mod calibrate_done;
pub(crate) mod menu;
pub(crate) mod play;
pub(crate) mod results;
pub(crate) mod screen;
pub(crate) mod select;
pub(crate) mod settings;

use crate::Scene;

pub(crate) fn fmt_time(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Whether `scene` shows its own map's background, which the song select's preview leaves be.
pub(crate) fn shows_own_background(scene: &Scene) -> bool {
    match scene {
        Scene::Editor(_) | Scene::Loading { .. } | Scene::Play { .. } | Scene::Results { .. } => {
            true
        }
        Scene::Settings { prev } => shows_own_background(prev),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    /// What lib.rs drew and did for each screen stays with the screen.
    #[test]
    fn each_screen_s_drawing_lives_with_the_screen() {
        let lib = include_str!("../lib.rs");
        for (piece, file) in [
            (
                "fn draw_leaderboard(",
                include_str!("select/leaderboard.rs"),
            ),
            (
                "fn draw_online_leaderboard(",
                include_str!("select/leaderboard.rs"),
            ),
            ("fn draw_preset_naming(", include_str!("select/presets.rs")),
            (
                "fn draw_preview_chrome(",
                include_str!("select/preview_chrome.rs"),
            ),
            ("fn results_job(", include_str!("results.rs")),
            ("fn submit_score(", include_str!("results.rs")),
            ("fn draw_replay_bar(", include_str!("play.rs")),
            ("fn draw_spectate_all(", include_str!("play.rs")),
        ] {
            assert!(!lib.contains(piece), "`{piece}` is back in lib.rs");
            assert!(file.contains(piece), "`{piece}` isn't with its screen");
        }
    }
}
