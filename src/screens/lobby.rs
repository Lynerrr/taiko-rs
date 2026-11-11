//! A multiplayer room's lobby: its players, the map the host picked, the chat, and the
//! host's keys for the room. The room is the session's (`State::mp`).

use super::mp_menu::MULTIPLAYER_TAB;
use super::screen::{Frame, Key, Screen};
use crate::*;

/// The room's lobby up.
pub(crate) struct Lobby;

impl Lobby {
    pub(crate) fn scene() -> Scene {
        Scene::Other(Box::new(Lobby))
    }
}

impl Screen for Lobby {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Lobby"
    }

    fn activity(&self) -> &'static str {
        "lobby"
    }

    /// The room's (`State::presence_text` says the lobby when in a room).
    fn presence(&self, _state: &State) -> Option<(String, String)> {
        None
    }

    fn top_bar(&self) -> bool {
        true
    }

    /// The multiplayer tab, as the menu is.
    fn tab(&self) -> Option<usize> {
        Some(MULTIPLAYER_TAB)
    }

    fn in_room(&self) -> bool {
        true
    }

    fn room_lobby(&self) -> bool {
        true
    }

    fn idle_allowed(&self) -> bool {
        true
    }

    fn allows_settings(&self) -> bool {
        true
    }

    fn menu_screen(&self) -> Option<u8> {
        Some(6)
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
        // A chat line being typed in the lobby takes the keys (function keys still work).
        if state.mp.as_ref().is_some_and(|s| s.chat_input.is_some()) {
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
                state.lobby_chat_key(code, text);
            }
            return;
        }
        if repeat || state.handle_global_key(code, window) {
            return;
        }
        let Some(s) = &mut state.mp else {
            state.scene = MpMenu::scene();
            return;
        };
        if s.mapguessr {
            state.guessr_key(code);
            return;
        }
        // The host typing a new password (K): it takes the keys until Enter or Esc.
        if let Some(typed) = &mut s.edit_password {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let password = Some(typed.clone()).filter(|p| !p.is_empty());
                    s.edit_password = None;
                    let text = if password.is_some() {
                        tr!(
                            "the lobby has a password now",
                            "ロビーにパスワードを設定しました",
                            "у лобби теперь есть пароль"
                        )
                    } else {
                        tr!(
                            "the lobby is open to all",
                            "ロビーは誰でも入れます",
                            "лобби открыто для всех"
                        )
                    };
                    s.client.send(net::ClientMsg::SetPassword(password));
                    state.toast(text.into());
                }
                KeyCode::Escape => s.edit_password = None,
                KeyCode::Backspace => {
                    typed.pop();
                }
                _ => {
                    for ch in text.unwrap_or("").chars() {
                        if net::drawable_char(ch) && ch != ' ' && typed.chars().count() < 32 {
                            typed.push(ch);
                        }
                    }
                }
            }
            return;
        }
        match code {
            KeyCode::Space | KeyCode::KeyR if s.me().is_some_and(|p| p.spectator) => {
                state.toast("you're spectating: S to play instead".into());
            }
            KeyCode::Space | KeyCode::KeyR => {
                if s.me().is_some_and(|p| p.has_map) {
                    s.ready = !s.me().is_some_and(|p| p.ready);
                    s.client.send(net::ClientMsg::Ready(s.ready));
                } else {
                    state.toast("you don't have this map".into());
                }
            }
            KeyCode::BracketLeft | KeyCode::BracketRight if s.is_host() && !s.in_match => {
                let step = if code == KeyCode::BracketRight {
                    0.05
                } else {
                    -0.05
                } * if state.modifiers_shift { 5.0 } else { 1.0 };
                state.cfg.rate = ((state.cfg.rate + step) * 100.0).round() / 100.0;
                state.cfg.rate = state.cfg.rate.clamp(RATE_RANGE.0, RATE_RANGE.1);
                state.cfg.save();
                let rate = state.cfg.mods.speed(state.cfg.rate, true).0;
                if let Some(mut m) = s.map.clone() {
                    let mods = state.cfg.mods.lobby(m.defense);
                    m.rate = rate;
                    m.mods = mods.list();
                    (m.multiverse, m.shapeshift) = mod_settings(mods);
                    m.seed = lobby_seed(Some(state.cfg.random_seed));
                    s.client.send(net::ClientMsg::SelectMap(m));
                }
                let text = format!("match rate {rate:.2}x");
                state.toast(text);
            }
            KeyCode::KeyT => state.focus_lobby_chat(),
            KeyCode::KeyP if s.map_entry.is_some() => state.open_preview(),
            // Defense: C turns it on or off, 1-5 step its rules.
            KeyCode::KeyC
            | KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
                if s.is_host() && !s.in_match =>
            {
                let Some(mut m) = s.map.clone() else {
                    state.toast("pick a map first (M)".into());
                    return;
                };
                let option = match code {
                    KeyCode::Digit1 => Some(net::DefenseOption::Lives),
                    KeyCode::Digit2 => Some(net::DefenseOption::Bars),
                    KeyCode::Digit3 => Some(net::DefenseOption::TagIns),
                    KeyCode::Digit4 => Some(net::DefenseOption::Shop),
                    KeyCode::Digit5 => Some(net::DefenseOption::Maze),
                    _ => None,
                };
                let text = match (option, m.defense) {
                    (None, Some(_)) => {
                        m.defense = None;
                        tr!(
                            "Defense off: an ordinary match",
                            "ディフェンス オフ",
                            "Оборона выключена: обычный матч"
                        )
                        .to_string()
                    }
                    (None, None) => {
                        m.defense = Some(net::Defense::DEFAULT);
                        tr!(
                            "Defense co-op on: 1-5 change its rules",
                            "ディフェンス協力 オン: 1-5でルール変更",
                            "Оборона включена: 1-5 меняют правила"
                        )
                        .to_string()
                    }
                    (Some(o), Some(d)) => {
                        let d = o.step(d);
                        m.defense = Some(d);
                        format!("{}: {}", o.name(), o.value(&d))
                    }
                    (Some(_), None) => {
                        state.toast(
                            tr!(
                                "turn Defense on first (C)",
                                "先にディフェンスをオン (C)",
                                "сначала включи Оборону (C)"
                            )
                            .into(),
                        );
                        return;
                    }
                };
                // Defense takes the host's mods for everyone; an ordinary match, the speed's.
                let mods = state.cfg.mods.lobby(m.defense);
                m.mods = mods.list();
                (m.multiverse, m.shapeshift) = mod_settings(mods);
                s.client.send(net::ClientMsg::SelectMap(m));
                state.toast(text);
            }
            // The room: K types a new password, L steps how many players it takes.
            KeyCode::KeyK if s.is_host() && !s.in_match => s.edit_password = Some(String::new()),
            KeyCode::KeyL if s.is_host() && !s.in_match => {
                let list = net::RoomSettings::SLOTS;
                let at = list
                    .iter()
                    .position(|&n| n == s.slots)
                    .unwrap_or(list.len() - 1);
                // Never below who's in it already.
                let next = list
                    .iter()
                    .cycle()
                    .skip(at + 1)
                    .take(list.len())
                    .copied()
                    .find(|&n| n as usize >= s.players.len())
                    .unwrap_or(s.slots);
                s.client.send(net::ClientMsg::SetSlots(next));
                state.toast(tr!("the lobby takes {next} players", "ロビーの定員: {next}人", "лобби на {next} игроков";));
            }
            KeyCode::F1 => state.open_mod_select(),
            KeyCode::KeyS
                if s.in_match && s.me().is_some_and(|p| p.status == net::Status::Lobby) =>
            {
                if s.have_map {
                    s.client.send(net::ClientMsg::Spectate(true));
                } else {
                    state.toast("you need the map to spectate".into());
                }
            }
            // Before a match: watch it (and the next ones) instead of playing.
            KeyCode::KeyS if !s.in_match => {
                let on = !s.me().is_some_and(|p| p.spectator);
                s.ready = false;
                s.client.send(net::ClientMsg::Spectator(on));
                state.toast(
                    if on {
                        "you'll spectate the match"
                    } else {
                        "you'll play the match"
                    }
                    .into(),
                );
            }
            // Missing the map: from the host by default (fast, works for any map), the
            // mirrors with M or Shift+D.
            KeyCode::KeyD | KeyCode::KeyM
                if !s.have_map
                    && s.map.is_some()
                    && !s.is_host()
                    && (code == KeyCode::KeyM || state.modifiers_shift) =>
            {
                state.download_lobby_map()
            }
            KeyCode::KeyD | KeyCode::KeyH if !s.have_map && s.map.is_some() && !s.is_host() => {
                if state.host_transfer.is_none() {
                    s.client.send(net::ClientMsg::RequestMap);
                    state.host_map_asked = true;
                    state.host_asked_at = Some(Instant::now());
                    state.toast("getting the map from the host...".into());
                }
            }
            KeyCode::KeyM if s.is_host() && !s.in_match => {
                state.picking_lobby_map = true;
                state.scene = Scene::Select;
            }
            KeyCode::Enter | KeyCode::NumpadEnter if s.is_host() => {
                if s.players.iter().any(|p| p.ready && p.has_map) {
                    s.client.send(net::ClientMsg::StartMatch);
                } else {
                    state.toast("nobody is ready yet".into());
                }
            }
            KeyCode::Tab => state.scene = Scene::Select,
            KeyCode::Escape => {
                let why = if s.owns_room() {
                    "room closed"
                } else {
                    "left the room"
                };
                state.leave_room(why);
            }
            _ => {}
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        match &state.mp {
            Some(s) if s.mapguessr => {
                if let Some(g) = &mut state.guessr {
                    mapguessr::draw_lobby(canvas, s, g, state.cfg.scroll_speed, Instant::now());
                }
            }
            Some(s) => {
                multiplayer::draw_lobby(canvas, s, s.have_map, frame.lobby_download.clone());
            }
            None => return Some(MpMenu::scene()),
        }
        None
    }
}
