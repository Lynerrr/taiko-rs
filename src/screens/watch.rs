//! Watching a friend's play as it goes, a couple of seconds behind (`WatchView` keeps the
//! stream, its game and the map being fetched).

use super::screen::{Frame, Key, Screen};
use crate::*;

/// A friend's play being watched.
pub(crate) struct Watch {
    pub(crate) view: Box<WatchView>,
}

impl Watch {
    pub(crate) fn scene(view: Box<WatchView>) -> Scene {
        Scene::Other(Box::new(Watch { view }))
    }
}

impl Screen for Watch {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Watch"
    }

    fn activity(&self) -> &'static str {
        "watching a friend"
    }

    fn presence(&self, _state: &State) -> Option<(String, String)> {
        let v = &self.view;
        let who = &v.player;
        Some((
            tr!("Watching {who}", "{who}を観戦中", "Смотрит {who}"; [] [who]),
            v.play.as_ref().map(|p| p.map.label()).unwrap_or_default(),
        ))
    }

    fn presence_first(&self) -> bool {
        true
    }

    fn friends_activity(&self) -> Option<online::Activity> {
        Some(online::Activity::Watching {
            name: self.view.player.clone(),
        })
    }

    fn dims_background(&self) -> bool {
        true
    }

    fn closes_main_menu(&self) -> bool {
        true
    }

    fn in_song(&self) -> bool {
        true
    }

    fn watching(&self) -> bool {
        true
    }

    fn watched_map(&self) -> Option<usize> {
        self.view.entry
    }

    #[cfg(test)]
    fn loading(&self) -> bool {
        self.view.rx.is_busy()
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            window,
            ..
        } = key;
        if code == KeyCode::Escape {
            if !repeat {
                state.stop_watching();
            }
            return;
        }
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
        state.handle_global_key(code, window);
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let v = &mut self.view;
        let scroll = state.cfg.scroll_speed;
        match &mut v.game {
            Some(g) => {
                g.view.kiai_effects = state.cfg.kiai_effects;
                g.view.hidden = frame.hidden;
                g.view.hud = state.cfg.hud;
                g.view.hud_area = state.cfg.playfield_area;
                g.update(frame.now, sound(&state.audio), state.play_offset);
                draw_game_area(
                    canvas,
                    &state.cfg,
                    &mut state.frame_clip,
                    &state.grip,
                    &state.hover,
                    state.bg_brightness > 0.0,
                    |c| {
                        g.draw_playfield(c, scroll);
                    },
                );
                g.draw_hud(canvas);
                if v.held {
                    canvas.text_centered(
                        canvas.width / 2.0,
                        300.0,
                        9.0,
                        rgba(0x9a9ab0, 1.0),
                        &format!("{} paused", v.player),
                    );
                }
            }
            None => {
                // The map they're playing, on its way.
                let got = v
                    .play
                    .as_ref()
                    .filter(|_| v.fetching && v.entry.is_none())
                    .and_then(|p| map_download(&state.downloads, &p.map));
                let looking = v.fetching && v.entry.is_none() && state.lobby_lookup.is_busy();
                let text = got.map_or_else(|| v.status(), download_text);
                let w = canvas.width;
                canvas.text_centered(w / 2.0, 220.0, 12.0, rgba(0xffffff, 1.0), &text);
                if got.is_some() || looking {
                    let bw = (w - 48.0).min(300.0);
                    let at = (w - bw) / 2.0;
                    multiplayer::draw_fetch_bar(canvas, at, 240.0, bw, got.flatten());
                }
            }
        }
        canvas.text_centered(
            canvas.width / 2.0,
            470.0,
            7.0,
            rgba(0x9a9ab0, 1.0),
            "Esc: stop watching   F9: friends",
        );

        None
    }
}
