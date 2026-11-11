//! The multiplayer menu: the server, the account and the rooms to join or make. Its form is
//! `State::mp_menu`, which other screens read too (the account's name).

use super::screen::{Frame, Key, Screen};
use crate::*;

/// Where the multiplayer tab sits in the top bar.
const MULTIPLAYER_TAB: usize = 2;

/// The multiplayer menu up.
pub(crate) struct MpMenu;

impl MpMenu {
    pub(crate) fn scene() -> Scene {
        Scene::Other(Box::new(MpMenu))
    }
}

impl Screen for MpMenu {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "MpMenu"
    }

    fn activity(&self) -> &'static str {
        "multiplayer menu"
    }

    fn presence(&self, _state: &State) -> Option<(String, String)> {
        Some((
            tr!("Multiplayer menu", "マルチのメニュー", "Меню мультиплеера").into(),
            String::new(),
        ))
    }

    fn top_bar(&self) -> bool {
        true
    }

    fn allows_settings(&self) -> bool {
        true
    }

    fn tab(&self) -> Option<usize> {
        Some(MULTIPLAYER_TAB)
    }

    fn idle_allowed(&self) -> bool {
        true
    }

    fn update_corner(&self) -> bool {
        true
    }

    fn menu_screen(&self) -> Option<u8> {
        Some(5)
    }

    fn keeps_preset_keys(&self) -> bool {
        true
    }

    fn finger_scrolls(&self) -> bool {
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
        // The form takes typed text, so only function keys act as shortcuts here.
        let function_key = matches!(
            code,
            KeyCode::F1
                | KeyCode::F2
                | KeyCode::F3
                | KeyCode::F4
                | KeyCode::F5
                | KeyCode::F6
                | KeyCode::F7
                | KeyCode::F8
                | KeyCode::F9
                | KeyCode::F10
                | KeyCode::F11
                | KeyCode::F12
        );
        if !(function_key && !repeat && state.handle_global_key(code, window)) {
            state.mp_menu_key(code, text);
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _frame: &Frame) -> Option<Scene> {
        multiplayer::draw_menu(
            canvas,
            &state.mp_menu,
            &state.cfg.online_server,
            &state.mp_status,
            state.mp_connecting.is_some(),
        );
        None
    }
}
