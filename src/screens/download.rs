//! The download tab: searches the mirrors (osu! and taiko.rs) for sets, previews their maps
//! through song select's preview, and downloads them into the Songs folder. Its state is
//! `State::browse` and `State::downloads`, which go on while other screens are up.

use super::screen::{Frame, Key, Screen};
use crate::*;

/// Where the download tab sits in the top bar (songs, settings, multiplayer, downloads).
pub(crate) const DOWNLOAD_TAB: usize = 3;

/// The download tab up.
pub(crate) struct Download;

impl Download {
    pub(crate) fn scene() -> Scene {
        Scene::Other(Box::new(Download))
    }
}

impl State {
    pub(crate) fn open_download(&mut self) {
        self.picking_lobby_map = false;
        self.installed_sets();
        self.browse.opened();
        self.scene = Download::scene();
    }

    pub(crate) fn download_key(
        &mut self,
        code: KeyCode,
        text: Option<&str>,
        window: Option<&Window>,
    ) {
        let function_key = matches!(
            code,
            KeyCode::F1
                | KeyCode::F2
                | KeyCode::F3
                | KeyCode::F4
                | KeyCode::F7
                | KeyCode::F8
                | KeyCode::F9
                | KeyCode::F10
                | KeyCode::F11
                | KeyCode::F12
        );
        if function_key && self.handle_global_key(code, window) {
            return;
        }
        match code {
            KeyCode::ArrowLeft | KeyCode::ArrowRight if self.modifiers_ctrl => self
                .browse
                .step_category(if code == KeyCode::ArrowRight { 1 } else { -1 }),
            KeyCode::ArrowUp | KeyCode::ArrowDown if self.modifiers_ctrl => self
                .browse
                .step_sort(if code == KeyCode::ArrowDown { 1 } else { -1 }),
            KeyCode::Tab if self.modifiers_ctrl => {
                let next = match self.browse.source() {
                    browse::Source::Osu => browse::Source::Ours,
                    browse::Source::Ours => browse::Source::Osu,
                };
                self.browse.set_source(next);
            }
            KeyCode::ArrowUp => self.browse.move_by(-1),
            KeyCode::ArrowDown => self.browse.move_by(1),
            KeyCode::ArrowLeft => self.browse.cycle_diff(-1),
            KeyCode::ArrowRight => self.browse.cycle_diff(1),
            KeyCode::PageUp => self.browse.move_by(-8),
            KeyCode::PageDown => self.browse.move_by(8),
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if let Some(set) = self.browse.selected().cloned() {
                    let here = match set.custom {
                        Some(_) => self.select.custom_folder.values().any(|&id| id == set.id),
                        None => self.installed_sets().contains(&set.id),
                    };
                    if here {
                        self.play_downloaded(&set);
                    } else {
                        self.start_download(set);
                    }
                }
            }
            KeyCode::Escape if self.browse.menu.is_some() => self.browse.menu = None,
            KeyCode::Backspace if self.modifiers_ctrl => self.browse.clear(),
            KeyCode::Backspace => self.browse.backspace(),
            KeyCode::Escape if !self.browse.query.is_empty() => self.browse.clear(),
            KeyCode::Escape | KeyCode::Tab => self.scene = Scene::Select,
            _ => {
                if let Some(text) = text {
                    self.browse.type_text(text);
                }
            }
        }
    }
}

impl Screen for Download {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Download"
    }

    fn activity(&self) -> &'static str {
        "downloads"
    }

    fn presence(&self, _state: &State) -> Option<(String, String)> {
        Some((
            tr!(
                "Looking for new maps",
                "新しい譜面を探しています",
                "Ищет новые карты"
            )
            .into(),
            String::new(),
        ))
    }

    fn top_bar(&self) -> bool {
        true
    }

    fn song_select(&self) -> bool {
        true
    }

    fn allows_settings(&self) -> bool {
        true
    }

    fn tab(&self) -> Option<usize> {
        Some(DOWNLOAD_TAB)
    }

    fn previews_remote(&self) -> bool {
        true
    }

    fn idle_allowed(&self) -> bool {
        true
    }

    fn update_corner(&self) -> bool {
        true
    }

    fn menu_screen(&self) -> Option<u8> {
        Some(4)
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            text,
            window,
            ..
        } = key;
        if !repeat
            || matches!(
                code,
                KeyCode::ArrowUp
                    | KeyCode::ArrowDown
                    | KeyCode::Backspace
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            )
        {
            state.download_key(code, text, window);
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _frame: &Frame) -> Option<Scene> {
        let installed = state.installed_sets.take().unwrap_or_default();
        let dir = state.songs_dir.display().to_string();
        let lw = Select::list_width(canvas.width, state.cfg.song_list_share);
        state.preview.size = preview::PREVIEW_SIZE.1;
        let phone = mobile::active();
        if phone {
            let custom: std::collections::HashSet<u32> =
                state.select.custom_folder.values().copied().collect();
            state.draw_mobile_download(canvas, &installed, &custom);
        } else {
            let custom = &state.select.custom_folder;
            let is_installed = |set: &download::SetInfo| match set.custom {
                Some(_) => custom.values().any(|&id| id == set.id),
                None => installed.contains(&set.id),
            };
            state
                .browse
                .draw(canvas, &state.downloads, &is_installed, &dir, lw);
        }
        state.installed_sets = Some(installed);
        // The selected set's preview, like song select's.
        let x = 24.0 + lw + 12.0;
        let pw = if phone { 0.0 } else { canvas.width - x - 24.0 };
        let stars = state.browse.selected_stars();
        if pw > 160.0 && state.browse.selected().is_some() {
            match state.preview.draw_panel(
                canvas,
                &state.browse.remote,
                x,
                54.0,
                pw,
                380.0,
                state.cfg.scroll_speed,
                stars,
                None,
                None,
                None,
            ) {
                Some(clip) => state.frame_clip = Some(clip),
                None if state.browse.shown.is_none() => {
                    canvas.text_centered(
                        x + pw / 2.0,
                        160.0,
                        8.0,
                        rgba(0x8a8aa0, 1.0),
                        "loading preview...",
                    );
                }
                None => {}
            }
        }
        None
    }
}
