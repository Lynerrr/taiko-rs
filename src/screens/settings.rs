//! The settings screen, and what it shows and parses: the words of its values and choices,
//! its folder paths, and what a typed setting means.

use std::path::PathBuf;

use winit::keyboard::KeyCode;

use super::screen::{Frame, Key, Screen};
use crate::config;
use crate::*;

/// A setting that's on.
pub(crate) fn on_text() -> &'static str {
    tr!("on", "オン", "вкл")
}

pub(crate) fn off_text() -> &'static str {
    tr!("off", "オフ", "выкл")
}

/// No key, no choice.
pub(crate) fn none_text() -> &'static str {
    tr!("none", "なし", "нет")
}

pub(crate) fn picking_text() -> &'static str {
    tr!("picking...", "選択中...", "выбор...")
}

pub(crate) fn enter_to_start() -> &'static str {
    tr!("Enter to start", "Enterで開始", "Enter — начать")
}

/// taiko.rs's own skin.
pub(crate) fn built_in_text() -> &'static str {
    tr!("built-in", "内蔵", "встроенный")
}

pub(crate) fn scroll_in_select() -> &'static str {
    tr!(
        "change the scroll speed in song select, not during a song",
        "スクロール速度は曲中ではなく選曲画面で変えてください",
        "меняйте скорость прокрутки в выборе песни, не в песне"
    )
}

/// The list's choice to type a folder's path.
pub(crate) fn type_path() -> String {
    tr!("Type a path", "パスを入力", "Ввести путь").into()
}

/// The list's choice to pick a folder in the system's dialog.
pub(crate) fn browse_text() -> String {
    tr!("Browse...", "参照...", "Обзор...").into()
}

/// The library entry for a lobby map: same artist/title/difficulty, then the exact file hash.
/// Only those few candidates are hashed, so this stays cheap with a huge library.
/// A path for a settings row: its last two folders.
pub(crate) fn short_path(dir: &std::path::Path) -> String {
    let parts: Vec<_> = dir.components().collect();
    if parts.len() <= 3 {
        return dir.display().to_string();
    }
    let tail: PathBuf = parts[parts.len() - 2..].iter().collect();
    format!("...{}{}", std::path::MAIN_SEPARATOR, tail.display())
}

/// A character typed into a playfield size: a digit, or `x` between width and height.
pub(crate) fn size_char(code: KeyCode) -> Option<char> {
    use KeyCode::*;
    let digits = [
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    ];
    let numpad = [
        Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
    ];
    if let Some(d) = digits.iter().chain(numpad.iter()).position(|&k| k == code) {
        return char::from_digit(d as u32 % 10, 10);
    }
    matches!(code, KeyX | Space | NumpadMultiply).then_some('x')
}

/// A typed setting value as a number: units after it ("ms", "%", "x") are fine, a comma works
/// as the decimal point, and "off" / "unlimited" are 0.
pub(crate) fn parse_number(text: &str) -> Option<f64> {
    let t = text.trim().to_ascii_lowercase();
    if ["off", "unlimited", "none"].contains(&t.as_str()) {
        return Some(0.0);
    }
    let num: String = t
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '+' | '-'))
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    num.parse().ok().filter(|n: &f64| n.is_finite())
}

/// What the offset scale setting shows for `scale`.
pub(crate) fn offset_scale_name(scale: config::OffsetScale) -> &'static str {
    match scale {
        config::OffsetScale::Osu => "osu!",
        config::OffsetScale::Sound => tr!("By sound", "音基準", "По звуку"),
    }
}

/// The settings screen; the scene it was opened over is `State::behind`, where code that asks
/// about it (a song paused behind it, the menu it came from) finds it while it runs.
pub(crate) struct SettingsScreen;

impl SettingsScreen {
    pub(crate) fn scene() -> Scene {
        Scene::Other(Box::new(SettingsScreen))
    }
}

impl Screen for SettingsScreen {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Settings"
    }

    fn activity(&self) -> &'static str {
        "settings"
    }

    fn presence(&self, _state: &State) -> Option<(String, String)> {
        Some((
            tr!("Tweaking settings", "設定を調整中", "Копается в настройках").into(),
            String::new(),
        ))
    }

    fn top_bar(&self) -> bool {
        true
    }

    fn idle_allowed(&self) -> bool {
        true
    }

    fn menu_screen(&self) -> Option<u8> {
        Some(3)
    }

    fn settings(&self) -> bool {
        true
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            text,
            window,
            ..
        } = key;
        if repeat
            && !matches!(
                code,
                KeyCode::ArrowUp
                    | KeyCode::ArrowDown
                    | KeyCode::Backspace
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            )
        {
            return;
        }
        if state.handle_global_key(code, window) {
            return;
        }
        if !state.settings_type(code, text) {
            state.settings_key(code, window);
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _frame: &Frame) -> Option<Scene> {
        let from_game = state
            .behind_settings()
            .is_some_and(|prev| matches!(prev, Scene::Play { .. }));
        state.settings_ui.krokodil = state.is_krokodil();
        if mobile::active() {
            state.draw_mobile_settings(canvas);
            state.draw_mobile_choices(canvas);
        } else {
            settings::tab_bar(canvas, 1, if from_game { "game paused" } else { "" });
            let searching = !state.settings_ui.search.is_empty();
            let footer = if state
                .settings_ui
                .editing
                .as_ref()
                .is_some_and(|e| matches!(e.item, Item::SongsFolder | Item::DataFolder))
            {
                "type the folder's path   Ctrl+V paste   Enter set   Esc cancel".to_string()
            } else if state.settings_ui.editing.is_some() {
                "type a value   Enter set   Esc cancel".to_string()
            } else {
                let enter = match state.settings_ui.item().kind() {
                    Kind::List => "Enter list",
                    Kind::Value => "Enter type a value",
                    Kind::Toggle => "Enter toggle",
                    Kind::Key => "Enter rebind",
                    Kind::Action => "Enter go",
                };
                let back = if searching {
                    "Esc clear the search"
                } else if from_game {
                    "Esc back to pause menu"
                } else {
                    "Esc or Tab back"
                };
                format!("Type to search   Up/Down select   Left/Right change   {enter}   {back}")
            };
            let this = &*state;
            this.settings_ui.draw(
                canvas,
                &|item| this.setting_value(item),
                &|item| this.setting_control(item),
                &footer,
            );
            if let Some(p) = &state.settings_ui.choices {
                p.draw(canvas);
            }
        }

        None
    }
}
