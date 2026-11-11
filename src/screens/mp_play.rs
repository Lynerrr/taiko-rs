//! A multiplayer match being played: the room's map with everyone's scores alongside, the
//! intro skipped by vote, and Defense's shop. Its game is the one in `State::play`.

use super::mp_results::MpResults;
use super::screen::{Frame, Key, Screen};
use crate::*;

/// The match up.
pub(crate) struct MpPlay {
    /// Esc was pressed once at this time: a second one soon after forfeits.
    quit_armed: Option<Instant>,
}

impl MpPlay {
    pub(crate) fn scene() -> Scene {
        Scene::Other(Box::new(MpPlay { quit_armed: None }))
    }
}

impl Screen for MpPlay {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "MpPlay"
    }

    fn activity(&self) -> &'static str {
        "multiplayer match"
    }

    /// The match's own (`State::presence_text` asks `in_match`).
    fn presence(&self, _state: &State) -> Option<(String, String)> {
        None
    }

    fn dims_background(&self) -> bool {
        true
    }

    fn in_room(&self) -> bool {
        true
    }

    fn in_song(&self) -> bool {
        true
    }

    fn in_match(&self) -> bool {
        true
    }

    fn holds_game(&self) -> bool {
        true
    }

    fn area_editable(&self) -> bool {
        true
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            at,
            window,
            ..
        } = key;
        // Drum keys during a match go straight to the game.
        if let Some(game) = state.play.as_deref_mut()
            && let Some(k) = state.cfg.keys.iter().position(|&k| k == code)
        {
            if !repeat && game.replay().is_none() {
                game.press(k, at, sound(&state.audio), state.play_offset);
            }
            return;
        }
        if repeat || state.handle_global_key(code, window) {
            return;
        }
        let quit_armed = &mut self.quit_armed;
        if let Some(game) = state.play.as_deref_mut() {
            match code {
                KeyCode::KeyO => state.apply_live_offset(),
                KeyCode::Tab => {
                    state.cfg.mp_scoreboard = !state.cfg.mp_scoreboard;
                    state.cfg.save();
                    state.toast(
                        if state.cfg.mp_scoreboard {
                            "scoreboard shown (Tab)"
                        } else {
                            "scoreboard hidden (Tab)"
                        }
                        .into(),
                    );
                }
                // Asks the room to skip the intro: it's skipped once every player has asked.
                KeyCode::Space => {
                    if game.can_skip()
                        && let Some(s) = &mut state.mp
                    {
                        s.ask_to_skip();
                    }
                }
                // Defense: 1-5 buy for the team, T takes the turn over.
                KeyCode::Digit1
                | KeyCode::Digit2
                | KeyCode::Digit3
                | KeyCode::Digit4
                | KeyCode::Digit5
                | KeyCode::KeyT
                    if game.defense().is_some() =>
                {
                    let t = game.time();
                    let item = match code {
                        KeyCode::Digit1 => Some(net::DefenseItem::Slow),
                        KeyCode::Digit2 => Some(net::DefenseItem::Shield),
                        KeyCode::Digit3 => Some(net::DefenseItem::Heal),
                        KeyCode::Digit4 => Some(net::DefenseItem::DonTower),
                        KeyCode::Digit5 => Some(net::DefenseItem::KatTower),
                        _ => None,
                    };
                    let team = game.team_state().unwrap_or_default();
                    let can_tag = game.can_tag_in(t);
                    let shop = game.defense().is_some_and(|d| d.rules.shop);
                    let act = match (item, code) {
                        (Some(_), _) if !shop => {
                            state.toast("the host turned the shop off".into());
                            None
                        }
                        (Some(item), _) if team.gold < item.price(&team) => {
                            state.toast(format!(
                                "{}: {} gold, the team has {}",
                                item.name(),
                                item.price(&team),
                                team.gold
                            ));
                            None
                        }
                        (Some(net::DefenseItem::DonTower | net::DefenseItem::KatTower), _)
                            if game.tower_active(t) =>
                        {
                            state.toast("a tower is already up".into());
                            None
                        }
                        (Some(net::DefenseItem::Heal), _) if team.lives >= team.max => {
                            state.toast("the team has all its lives".into());
                            None
                        }
                        (Some(item), _) => {
                            let until = if matches!(
                                item,
                                net::DefenseItem::DonTower | net::DefenseItem::KatTower
                            ) {
                                game.tower_until(t).min(f64::MAX) // JSON has no infinity
                            } else {
                                0.0
                            };
                            Some(net::DefenseAct::Buy { item, at: t, until })
                        }
                        (None, KeyCode::KeyT) if can_tag => Some(net::DefenseAct::TagIn { at: t }),
                        (None, KeyCode::KeyT) => {
                            state.toast(
                                "can't take this turn over (yours, already taken, or ending)"
                                    .into(),
                            );
                            None
                        }
                        (None, _) => None,
                    };
                    if let (Some(act), Some(s)) = (act, &state.mp) {
                        s.client.send(net::ClientMsg::Defense(act));
                    }
                }
                KeyCode::Escape => {
                    // No pausing in a match; a second Esc forfeits.
                    if quit_armed.is_some_and(|t| t.elapsed() < Duration::from_millis(1500)) {
                        if let Some(s) = &state.mp {
                            s.client
                                .send(net::ClientMsg::Finished(multiplayer::final_from(
                                    game.stats(),
                                    true,
                                )));
                        }
                        state.stop_audio();
                        state.scene = MpResults::scene();
                    } else {
                        *quit_armed = Some(Instant::now());
                        state.toast("press Esc again to forfeit".into());
                    }
                }
                _ => {}
            }
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let game = state.play.as_deref_mut()?;
        let (now, hidden, visual_offset) = (frame.now, frame.hidden, frame.visual_offset);
        let mut next = None;
        let done = game.update(now, sound(&state.audio), state.play_offset);
        if state.auto || game.mods().has(mods::Mod::Autoplay) {
            game.set_autoplay_style(state.cfg.autoplay);
            game.autoplay(sound(&state.audio));
        }
        game.view.show_offset_hint = state.cfg.show_offset_hint && state.cfg.show_map_info;
        game.view.offset_key = state
            .cfg
            .hotkeys
            .get(keys::Hotkey::ApplyOffset)
            .map(keys::Chord::name);
        game.view.kiai_effects = state.cfg.kiai_effects;
        game.view.hidden = hidden;
        game.view.hud = state.cfg.hud;
        game.view.hud_area = state.cfg.playfield_area;
        game.set_ur_scaled_hint(state.cfg.offset_hint_ur_scaled);
        game.set_visual_offset(visual_offset);
        if let Some(team) = state.mp.as_ref().and_then(|s| s.team) {
            game.set_team(team);
        }
        let live = multiplayer::live_from(game);
        let mp = &state.mp;
        draw_game_area(
            canvas,
            &state.cfg,
            &mut state.frame_clip,
            &state.grip,
            &state.hover,
            state.bg_brightness > 0.0,
            |c| {
                game.draw_playfield(c, game.scroll_speed());
            },
        );
        let touch_controls = state.touch_used || mobile::active();
        game.draw_hud_with_intro(canvas, touch_controls);
        // The intro is skipped once every player asks: how many have.
        if let Some(s) = mp.as_ref().filter(|s| {
            game.can_skip() && (!touch_controls || s.skip_asked || s.skip_votes.is_some())
        }) {
            multiplayer::draw_skip_votes(canvas, s);
        }
        if let Some(s) = mp
            .as_ref()
            .filter(|_| state.cfg.mp_scoreboard && !hidden.has(hud::Part::Scoreboard))
        {
            game.placed_from(canvas, hud::Part::Scoreboard, [12.0, 276.0], |c| {
                multiplayer::draw_scoreboard(c, s, &live);
            });
        }
        if let Some(s) = &mut state.mp {
            if s.last_live.elapsed() >= Duration::from_millis(100) {
                s.last_live = now;
                s.client.send(net::ClientMsg::Live(live));
                if game.inputs().len() > s.inputs_sent {
                    let inputs = game.inputs()[s.inputs_sent..].to_vec();
                    s.inputs_sent = game.inputs().len();
                    s.client.send(net::ClientMsg::Inputs {
                        inputs,
                        anti_mash: true,
                    });
                }
            }
            if done {
                // Failing (SD / PF) ends the match for you like forfeiting does.
                let failed = game.failed_early();
                s.client
                    .send(net::ClientMsg::Finished(multiplayer::final_from(
                        game.stats(),
                        failed,
                    )));
                if failed && let Some(a) = &state.audio {
                    a.stop();
                }
                next = Some(MpResults::scene());
            }
        }

        next
    }
}
