//! The map editor, editing one difficulty (`editor::view::View`).

use super::screen::{Frame, Key, Screen};
use crate::*;

/// What the editor's frame asks for, done once the frame is drawn.
#[derive(Default)]
pub(crate) struct Out {
    /// Its music again at another speed: the map and the speed.
    pub(crate) reload: Option<(usize, f64)>,
    /// A test play of the map, from a map time.
    pub(crate) test: Option<(usize, Option<f64>)>,
    /// Its folder opened.
    pub(crate) folder: Option<PathBuf>,
    /// A file renamed: from, to.
    pub(crate) renamed: Option<(PathBuf, PathBuf)>,
    /// Maps that came into its folder or left it.
    pub(crate) folder_news: Option<folder_watch::Changes>,
}

/// The map editor.
pub(crate) struct MapEditor {
    pub(crate) view: Box<editor::view::View>,
    pub(crate) out: Out,
}

impl MapEditor {
    pub(crate) fn scene(view: Box<editor::view::View>) -> Scene {
        Scene::Other(Box::new(MapEditor {
            view,
            out: Out::default(),
        }))
    }
}

impl Screen for MapEditor {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Editor"
    }

    fn activity(&self) -> &'static str {
        "map editor"
    }

    fn presence(&self, _state: &State) -> Option<(String, String)> {
        Some((
            tr!("Mapping", "マッピング中", "Делает карту").into(),
            self.view.label(),
        ))
    }

    fn presence_first(&self) -> bool {
        true
    }

    fn friends_activity(&self) -> Option<online::Activity> {
        Some(online::Activity::Editing {
            map: self.view.label(),
        })
    }

    fn editor(&self) -> bool {
        true
    }

    fn dims_background(&self) -> bool {
        true
    }

    fn closes_main_menu(&self) -> bool {
        true
    }

    fn shows_own_background(&self) -> bool {
        true
    }

    fn drags(&self) -> bool {
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
        let view = &mut self.view;
        // A text block's words come as text; Enter, Esc and Backspace still as keys.
        if view.wants_text()
            && let Some(t) = text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
        {
            view.type_text(t);
            return;
        }
        let keys = state.editor_keys();
        match view.key(code, repeat, keys, state.audio.as_ref()) {
            editor::view::KeyResult::Handled => {}
            editor::view::KeyResult::Exit => state.show_select(),
            editor::view::KeyResult::Pass => {
                state.handle_global_key(code, window);
            }
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let view = &mut self.view;
        // Uploading needs the game's account on our own server.
        let token = (state.cfg.online_server == online::DEFAULT_SERVER
            && !state.cfg.online_token.is_empty())
        .then_some(&state.cfg.online_token);
        if view.account.as_ref() != token {
            view.account = token.cloned();
        }
        view.update(frame.now, state.audio.as_ref());
        if let Some(speed) = view.take_rate_request() {
            self.out.reload = Some((view.entry, speed));
        }
        if view.left_pan != state.cfg.editor_left_pan {
            state.cfg.editor_left_pan = view.left_pan;
            state.cfg.save();
        }
        if view.ed.follow_timing != state.cfg.editor_follow_timing {
            state.cfg.editor_follow_timing = view.ed.follow_timing;
            state.cfg.save();
        }
        if view.snap_dirty {
            view.snap_dirty = false;
            (state.cfg.editor_snap_presets, state.cfg.editor_snap_active) = view.snap_config();
            state.cfg.save();
        }
        if let Some(from) = view.take_test_play() {
            self.out.test = Some((view.entry, from));
        }
        self.out.folder = view.take_folder_request();
        self.out.renamed = view.take_renamed();
        // Maps dropped into its folder (or taken out) join the list as they come.
        if let Some(dir) = view.ed.path.parent() {
            if state.folder_watch.as_ref().map(|w| w.folder()) != Some(dir) {
                state.folder_watch = Some(folder_watch::Watch::new(dir, frame.now));
            }
            self.out.folder_news = state.folder_watch.as_mut().and_then(|w| w.poll(frame.now));
        }
        view.draw(canvas, state.cfg.scroll_speed, state.cfg.autoplay);
        if view.take_tour_ended() && !state.cfg.editor_tour_done {
            state.cfg.editor_tour_done = true;
            state.cfg.save();
        }
        if view.take_exit() {
            // It may have been sent just frame.now: ask which sets are custom again.
            state.custom_asked = None;
            return Some(Scene::Select);
        }
        None
    }
}
