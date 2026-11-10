//! Screens that live outside lib.rs (docs/client-refactor-next-steps.md, S5): each one keeps
//! its own state, takes its keys and draws itself, behind `Scene::Other`. While a screen takes
//! a key it is out of `State::scene`, so it gets the whole app to act on; it says whether it
//! stays up, or has put the scene that follows it in its place.

use crate::*;

/// What a screen's key leaves the app with.
pub(crate) enum Next {
    /// The screen stays up.
    Stay,
    /// It has put another scene in its place (`State::scene`).
    Left,
}

pub(crate) trait Screen: Send {
    /// Its name, as the tests call it (`regression::harness::scene_name`).
    #[cfg(test)]
    fn name(&self) -> &'static str;
    /// What the player is doing on it, for the status line friends see.
    fn activity(&self) -> &'static str;
    /// The same for Discord, in the player's language.
    fn presence(&self) -> String;
    /// How dark the map's background is behind it, when not the usual.
    fn background_dim(&self) -> Option<f32> {
        None
    }
    /// On a phone: it has a bar along the top, where the corner chips would sit.
    fn top_bar(&self) -> bool {
        false
    }
    /// It is part of calibrating the offset: the guide waits for it to end.
    fn calibrating(&self) -> bool {
        false
    }
    /// A key, with the app it runs in (`state.scene` holds something else meanwhile).
    fn key(
        &mut self,
        state: &mut State,
        code: KeyCode,
        repeat: bool,
        window: Option<&Window>,
    ) -> Next;
    /// Draws it over the background.
    fn draw(&self, state: &State, canvas: &mut Canvas);
}

impl Scene {
    /// The screen up, if it is one of those outside lib.rs.
    pub(crate) fn screen(&self) -> Option<&dyn Screen> {
        match self {
            Scene::Other(s) => Some(s.as_ref()),
            _ => None,
        }
    }
}

impl State {
    /// Gives a key to the screen up when it is one outside lib.rs; false when it isn't.
    pub(crate) fn screen_key(
        &mut self,
        code: KeyCode,
        repeat: bool,
        window: Option<&Window>,
    ) -> bool {
        if !matches!(self.scene, Scene::Other(_)) {
            return false;
        }
        let Scene::Other(mut screen) = std::mem::replace(&mut self.scene, Scene::Select) else {
            unreachable!("checked above");
        };
        if let Next::Stay = screen.key(self, code, repeat, window) {
            self.scene = Scene::Other(screen);
        }
        true
    }

    /// Draws the screen up when it is one outside lib.rs.
    pub(crate) fn draw_screen(&self, canvas: &mut Canvas) {
        if let Some(screen) = self.scene.screen() {
            screen.draw(self, canvas);
        }
    }
}

#[cfg(test)]
mod tests {
    /// The screens moved out of lib.rs stay out: no `Scene` variant of their own, no state of
    /// theirs in lib.rs.
    /// Every screen moved out so far.
    const MOVED: &[&str] = &["CalibrateDone"];

    #[test]
    fn moved_screens_stay_out_of_lib() {
        let lib = include_str!("../lib.rs");
        for name in MOVED {
            let variant = ["Scene::", *name].concat();
            let named: Vec<&str> = lib.lines().filter(|l| l.contains(&variant)).collect();
            assert!(named.is_empty(), "lib.rs names {variant}: {named:?}");
            let field = [*name, " {"].concat();
            assert!(!lib.contains(&field), "lib.rs declares {field}");
        }
    }
}
