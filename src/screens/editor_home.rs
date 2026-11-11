//! The map editor's front door: a new map from a song, or one to go on with.

use super::screen::{Frame, Key, Screen};
use crate::*;

/// The editor's front door up.
pub(crate) struct EditorHome {
    pub(crate) home: editor::home::Home,
    /// What its last frame asked for, done once the frame is drawn.
    pub(crate) out: Option<editor::home::Out>,
}

impl EditorHome {
    pub(crate) fn scene(home: editor::home::Home) -> Scene {
        Scene::Other(Box::new(EditorHome { home, out: None }))
    }
}

impl State {
    /// The editor's front door: make a map from a song, or go on with one.
    pub(crate) fn open_editor_home(&mut self) {
        self.stop_audio();
        let creator = self.mp_menu.account.clone().unwrap_or_default();
        self.scene = EditorHome::scene(editor::home::Home::new(&self.cfg.editor_recent, &creator));
    }

    /// What the front door asked for.
    pub(crate) fn home_out(&mut self, out: editor::home::Out) {
        use editor::home::Out;
        match out {
            Out::None => {}
            Out::Toast(text) => self.toast(text),
            Out::Leave => self.show_select(),
            Out::PickInLibrary => {
                self.pick_to_edit = true;
                self.main_menu = None;
                self.scene = Scene::Select;
            }
            Out::Open(path) => match self.entry_for(&path) {
                Some(entry) => self.open_editor_at(path, entry),
                None => self.toast(format!("can't open {}", path.display())),
            },
            Out::Tour => {
                let recent = self.cfg.editor_recent.first().map(PathBuf::from);
                match recent
                    .filter(|p| p.is_file())
                    .and_then(|p| Some((self.entry_for(&p)?, p)))
                {
                    Some((entry, path)) => {
                        self.open_editor_at(path, entry);
                        if let Scene::Editor(view) = &mut self.scene {
                            view.start_tour();
                        }
                    }
                    None => self.toast(
                        "make a map first (drop a song on this screen), then take the tour".into(),
                    ),
                }
            }
            Out::Create(plan) => {
                let _ = std::fs::create_dir_all(&self.songs_dir);
                match editor::new_map::create(&plan, &self.songs_dir) {
                    Ok(file) => match self.entry_for(&file) {
                        Some(entry) => {
                            self.toast(format!("made {}", plan.title));
                            self.open_editor_at(file, entry);
                        }
                        None => self.toast("the map was made but can't be opened".into()),
                    },
                    Err(e) => self.toast(format!("can't make the map: {e}")),
                }
            }
        }
    }
}

impl Screen for EditorHome {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "EditorHome"
    }

    fn activity(&self) -> &'static str {
        "map editor menu"
    }

    fn presence_first(&self) -> bool {
        true
    }

    fn presence(&self) -> Option<(String, String)> {
        Some({
            (
                tr!("Starting a map", "譜面作りを開始中", "Начинает карту").into(),
                String::new(),
            )
        })
    }

    fn drags(&self) -> bool {
        true
    }

    fn typing(&self) -> bool {
        self.home.needs_text()
    }

    fn editor(&self) -> bool {
        true
    }

    fn chip_from_right(&self) -> Option<f32> {
        Some(100.0)
    }

    fn friends_activity(&self) -> Option<online::Activity> {
        Some(online::Activity::Editing { map: String::new() })
    }

    fn takes_every_key(&self) -> bool {
        true
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            text,
            at,
            ..
        } = key;
        let typing = self.home.needs_text();
        let movement = matches!(
            code,
            KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Home | KeyCode::End
        );
        if repeat && !typing && !movement {
            return;
        }
        let (ctrl, shift) = (state.modifiers_ctrl, state.modifiers_shift);
        let out = self
            .home
            .key(code, text, ctrl, shift, at, state.audio.as_ref());
        state.home_out(out);
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let out = self.home.update(frame.now, state.audio.as_ref());
        self.home.draw(canvas);
        if !matches!(out, editor::home::Out::None) {
            self.out = Some(out);
        }
        None
    }
}
