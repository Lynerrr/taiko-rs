//! The screens, each in its own module (docs/client-architecture.md, 4.5). For now they hold
//! what lib.rs drew and did for each screen; the screens themselves follow (step 7).

pub(crate) mod calibrate_done;
pub(crate) mod calibration;
pub(crate) mod download;
pub(crate) mod editor_home;
pub(crate) mod hud_editor;
pub(crate) mod layout_edit;
pub(crate) mod loading;
pub(crate) mod lobby;
pub(crate) mod map_editor;
pub(crate) mod menu;
pub(crate) mod mp_menu;
pub(crate) mod mp_play;
pub(crate) mod mp_results;
pub(crate) mod mp_wait;
pub(crate) mod play;
pub(crate) mod preview;
pub(crate) mod results;
pub(crate) mod screen;
pub(crate) mod select;
pub(crate) mod settings;
pub(crate) mod spectate;
pub(crate) mod video_edit;
pub(crate) mod watch;

use crate::Scene;

pub(crate) fn fmt_time(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Whether `scene` shows its own map's background, which the song select's preview leaves be.
pub(crate) fn shows_own_background(scene: &Scene) -> bool {
    match scene {
        Scene::Other(s) => s.shows_own_background(),
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
