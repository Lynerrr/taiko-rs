//! Screens that live outside lib.rs (docs/client-refactor-next-steps.md, S5): each one keeps
//! its own state, takes its input and draws itself, behind `Scene::Other`.
//!
//! While a screen handles something it is out of `State::scene`, so it gets the whole app to
//! act on. A `Shadow` of it holds its place meanwhile and answers every question the rest of
//! the app asks a screen as the screen itself would, so code it calls (global keys, a toast)
//! sees the same scene as before. A screen leaves by putting another scene in its place; when
//! the shadow is still there afterwards, the screen goes back.

use std::any::Any;

use crate::*;

/// What `build_frame` worked out for this frame before handling the media keys and the rest,
/// which a screen's frame uses as the `Scene` arm did.
pub(crate) struct Frame<'a> {
    pub(crate) now: Instant,
    /// `State::preview_target`.
    pub(crate) preview_target: Option<usize>,
    /// The player hosts the room and no match is on.
    pub(crate) lobby_host: bool,
    /// "Playfield 1280x720 ...": `State::playfield_label`.
    pub(crate) size_label: &'a str,
}

/// A key pressed, as a screen gets it.
#[derive(Clone, Copy)]
pub(crate) struct Key<'a> {
    pub(crate) code: KeyCode,
    pub(crate) repeat: bool,
    /// The text it types, if any.
    pub(crate) text: Option<&'a str>,
    /// When it was pressed.
    pub(crate) at: Instant,
    pub(crate) window: Option<&'a Window>,
}

pub(crate) trait Screen: Any + Send {
    /// Its name, as the tests call it (`regression::harness::scene_name`).
    #[cfg(test)]
    fn name(&self) -> &'static str;
    /// What the player is doing on it, for the status line friends see.
    fn activity(&self) -> &'static str;
    /// The same for Discord, in the player's language, and the map it is about (or nothing).
    /// None: what song select would say.
    fn presence(&self) -> Option<(String, String)>;
    /// Its presence holds in a multiplayer room and over the menu too, which otherwise say
    /// where the player is.
    fn presence_first(&self) -> bool {
        false
    }
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
    /// It shows a song, its background dimmed by the background dim setting.
    fn dims_background(&self) -> bool {
        false
    }
    /// It is a song or a replay, which leaves the main menu behind by whatever road it opened.
    fn closes_main_menu(&self) -> bool {
        false
    }
    /// On a phone: a finger drags on it as a mouse would.
    fn drags(&self) -> bool {
        false
    }
    /// The playfield area can be moved and resized on it with the mouse.
    fn area_editable(&self) -> bool {
        false
    }
    /// The wheel scales the playfield area on it, whatever the setting says.
    fn wheel_scales_area(&self) -> bool {
        false
    }
    /// It is the map editor or part of it: the friends chip replaces the edge tab, and the
    /// gameplay preset keys are its own.
    fn editor(&self) -> bool {
        false
    }
    /// In the editors' top bar, how far from the right edge the friends chip starts.
    fn chip_from_right(&self) -> Option<f32> {
        None
    }
    /// What friends see the player doing on it, when not the usual for where they are.
    fn friends_activity(&self) -> Option<online::Activity> {
        None
    }
    /// It takes every key as it is: no hotkeys on it.
    fn takes_every_key(&self) -> bool {
        false
    }
    /// It was opened over the multiplayer lobby: the lobby's map is the one it is about.
    fn over_lobby(&self) -> bool {
        false
    }
    /// It is part of song select (`State::in_song_select`): the song list's pointer, the
    /// preview panel and the map's stars work on it.
    fn song_select(&self) -> bool {
        false
    }
    /// The map it is about is song select's (its offset, the map's own skin).
    fn map_from_song_select(&self) -> bool {
        false
    }
    /// It is a tab of the top bar (songs 0, settings 1, multiplayer 2, downloads 3).
    fn tab(&self) -> Option<usize> {
        None
    }
    /// It previews remote maps (the download tab's search results) through song select's
    /// preview.
    fn previews_remote(&self) -> bool {
        false
    }
    /// The idle screen may come up over it.
    fn idle_allowed(&self) -> bool {
        false
    }
    /// The update badge and what's new sit in its corner, and the app may restart there to
    /// update.
    fn update_corner(&self) -> bool {
        false
    }
    /// Which menu screen it is, for telling when the menus change (`State::menu_screen`).
    fn menu_screen(&self) -> Option<u8> {
        None
    }
    /// Settings can open over it.
    fn allows_settings(&self) -> bool {
        false
    }
    /// It covers the screen where the friends tab would go (F9 still opens the panel).
    fn covers_social_tab(&self) -> bool {
        false
    }
    /// Something on it is being typed (a name), so keys are text and Ctrl+V pastes there.
    fn typing(&self) -> bool {
        false
    }
    /// A map loads for it on another thread.
    #[cfg(test)]
    fn loading(&self) -> bool {
        false
    }
    /// A key, with the app it runs in.
    fn key(&mut self, state: &mut State, key: Key);
    /// The pointer moved to (`x`, `y`) in window pixels; Some: it took the move, and the cursor
    /// to show.
    fn pointer_moved(&mut self, _state: &mut State, _x: f64, _y: f64) -> Option<CursorIcon> {
        None
    }
    /// The left button went down or up at the pointer; true: it took the press.
    fn mouse_button(&mut self, _state: &mut State, _pressed: bool) -> bool {
        false
    }
    /// Its frame: brings it up to date and draws it over the background. Some: the
    /// scene to go to once the frame is drawn.
    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene>;
}

/// Holds a screen's place while the screen handles something, answering as it would.
struct Shadow {
    #[cfg(test)]
    name: &'static str,
    activity: &'static str,
    presence: Option<(String, String)>,
    presence_first: bool,
    background_dim: Option<f32>,
    top_bar: bool,
    calibrating: bool,
    dims_background: bool,
    closes_main_menu: bool,
    drags: bool,
    area_editable: bool,
    wheel_scales_area: bool,
    covers_social_tab: bool,
    over_lobby: bool,
    song_select: bool,
    map_from_song_select: bool,
    allows_settings: bool,
    tab: Option<usize>,
    previews_remote: bool,
    idle_allowed: bool,
    update_corner: bool,
    menu_screen: Option<u8>,
    typing: bool,
    editor: bool,
    chip_from_right: Option<f32>,
    friends_activity: Option<online::Activity>,
    takes_every_key: bool,
    #[cfg(test)]
    loading: bool,
}

impl Shadow {
    fn of(screen: &dyn Screen) -> Shadow {
        Shadow {
            #[cfg(test)]
            name: screen.name(),
            activity: screen.activity(),
            presence: screen.presence(),
            presence_first: screen.presence_first(),
            background_dim: screen.background_dim(),
            top_bar: screen.top_bar(),
            calibrating: screen.calibrating(),
            dims_background: screen.dims_background(),
            closes_main_menu: screen.closes_main_menu(),
            drags: screen.drags(),
            area_editable: screen.area_editable(),
            wheel_scales_area: screen.wheel_scales_area(),
            covers_social_tab: screen.covers_social_tab(),
            over_lobby: screen.over_lobby(),
            song_select: screen.song_select(),
            map_from_song_select: screen.map_from_song_select(),
            allows_settings: screen.allows_settings(),
            tab: screen.tab(),
            previews_remote: screen.previews_remote(),
            idle_allowed: screen.idle_allowed(),
            update_corner: screen.update_corner(),
            menu_screen: screen.menu_screen(),
            typing: screen.typing(),
            editor: screen.editor(),
            chip_from_right: screen.chip_from_right(),
            friends_activity: screen.friends_activity(),
            takes_every_key: screen.takes_every_key(),
            #[cfg(test)]
            loading: screen.loading(),
        }
    }
}

impl Screen for Shadow {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        self.name
    }
    fn activity(&self) -> &'static str {
        self.activity
    }
    fn presence(&self) -> Option<(String, String)> {
        self.presence.clone()
    }
    fn presence_first(&self) -> bool {
        self.presence_first
    }
    fn background_dim(&self) -> Option<f32> {
        self.background_dim
    }
    fn top_bar(&self) -> bool {
        self.top_bar
    }
    fn calibrating(&self) -> bool {
        self.calibrating
    }
    fn dims_background(&self) -> bool {
        self.dims_background
    }
    fn closes_main_menu(&self) -> bool {
        self.closes_main_menu
    }
    fn drags(&self) -> bool {
        self.drags
    }
    fn area_editable(&self) -> bool {
        self.area_editable
    }
    fn wheel_scales_area(&self) -> bool {
        self.wheel_scales_area
    }
    fn covers_social_tab(&self) -> bool {
        self.covers_social_tab
    }
    fn over_lobby(&self) -> bool {
        self.over_lobby
    }
    fn song_select(&self) -> bool {
        self.song_select
    }
    fn map_from_song_select(&self) -> bool {
        self.map_from_song_select
    }
    fn allows_settings(&self) -> bool {
        self.allows_settings
    }
    fn tab(&self) -> Option<usize> {
        self.tab
    }
    fn previews_remote(&self) -> bool {
        self.previews_remote
    }
    fn idle_allowed(&self) -> bool {
        self.idle_allowed
    }
    fn update_corner(&self) -> bool {
        self.update_corner
    }
    fn menu_screen(&self) -> Option<u8> {
        self.menu_screen
    }
    fn typing(&self) -> bool {
        self.typing
    }
    fn editor(&self) -> bool {
        self.editor
    }
    fn chip_from_right(&self) -> Option<f32> {
        self.chip_from_right
    }
    fn friends_activity(&self) -> Option<online::Activity> {
        self.friends_activity.clone()
    }
    fn takes_every_key(&self) -> bool {
        self.takes_every_key
    }
    #[cfg(test)]
    fn loading(&self) -> bool {
        self.loading
    }
    // The screen itself has the input and the frame (`State::with_screen`).
    fn key(&mut self, _: &mut State, _: Key) {}
    fn draw(&mut self, _: &mut State, _: &mut Canvas, _: &Frame) -> Option<Scene> {
        None
    }
}

fn is_shadow(screen: &dyn Screen) -> bool {
    let any: &dyn Any = screen;
    any.is::<Shadow>()
}

/// Which screen is up, for noticing a change of screen: the variant, and for the screens
/// outside lib.rs (all `Scene::Other`) which one.
pub(crate) type SceneKind = (std::mem::Discriminant<Scene>, Option<std::any::TypeId>);

impl Scene {
    /// Which screen this is; two scenes of the same kind are the same screen.
    pub(crate) fn kind(&self) -> SceneKind {
        let screen = self.screen().map(|s| {
            let any: &dyn Any = s;
            any.type_id()
        });
        (std::mem::discriminant(self), screen)
    }

    /// The screen up, if it is one of those outside lib.rs.
    pub(crate) fn screen(&self) -> Option<&dyn Screen> {
        match self {
            Scene::Other(s) => Some(s.as_ref()),
            _ => None,
        }
    }

    /// The screen up is a `T` (not a shadow holding its place).
    pub(crate) fn screen_is<T: Screen>(&self) -> bool {
        self.screen().is_some_and(|s| {
            let any: &dyn Any = s;
            any.is::<T>()
        })
    }

    /// The screen up, when it is a `T` (and not a shadow holding its place).
    pub(crate) fn screen_mut<T: Screen>(&mut self) -> Option<&mut T> {
        match self {
            Scene::Other(s) => {
                let any: &mut dyn Any = s.as_mut();
                any.downcast_mut::<T>()
            }
            _ => None,
        }
    }
}

impl State {
    /// Runs `f` on the screen up, when it is one outside lib.rs, with the app around it; a
    /// shadow holds its place meanwhile. None when no such screen is up.
    pub(crate) fn with_screen<R>(
        &mut self,
        f: impl FnOnce(&mut dyn Screen, &mut State) -> R,
    ) -> Option<R> {
        let shadow = match &self.scene {
            Scene::Other(s) if !is_shadow(s.as_ref()) => Shadow::of(s.as_ref()),
            _ => return None,
        };
        let Scene::Other(mut screen) =
            std::mem::replace(&mut self.scene, Scene::Other(Box::new(shadow)))
        else {
            unreachable!("matched above");
        };
        let out = f(screen.as_mut(), self);
        if self.scene.screen().is_some_and(is_shadow) {
            self.scene = Scene::Other(screen);
        }
        Some(out)
    }

    /// Gives a key to the screen up when it is one outside lib.rs; false when it isn't.
    pub(crate) fn screen_key(&mut self, key: Key) -> bool {
        self.with_screen(|s, state| s.key(state, key)).is_some()
    }

    /// The frame of the screen up when it is one outside lib.rs; Some: the scene to go to.
    pub(crate) fn draw_screen(&mut self, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        self.with_screen(|s, state| s.draw(state, canvas, frame))
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    /// The editor's front door (and the video editor, the same way) says what it is in a
    /// room as well: it was matched before the room's presence when it was a `Scene` arm.
    #[test]
    fn a_screen_that_said_itself_in_a_room_still_does() {
        let mut s = layout_tests::state();
        s.mp = Some(layout_tests::session(false));
        s.open_editor_home();
        let own = s.scene.screen().and_then(|h| h.presence());
        assert!(own.is_some(), "the front door is up");
        assert_eq!(s.presence_text(), own);
    }

    /// Going from one screen outside lib.rs to another is a change of screen, though both are
    /// `Scene::Other`: the new one fades in and its animations start over, as when each was a
    /// variant of its own.
    #[test]
    fn going_between_two_moved_screens_is_a_change_of_screen() {
        let mut s = layout_tests::state();
        s.cfg.animations = true;
        // The library's scan comes in on the first frame.
        let mut at = Instant::now();
        s.build_frame(&mut Canvas::default(), at, 853.3);
        s.open_layout_editor();
        for _ in 0..20 {
            at += Duration::from_millis(50);
            s.build_frame(&mut Canvas::default(), at, 853.3);
        }
        assert!(
            s.scene
                .screen_mut::<screens::layout_edit::LayoutEdit>()
                .is_some()
        );
        assert_eq!(s.screen_fade, None, "the first screen has faded in");
        s.open_hud_editor();
        at += Duration::from_millis(50);
        s.build_frame(&mut Canvas::default(), at, 853.3);
        assert!(
            s.scene
                .screen_mut::<screens::hud_editor::HudEdit>()
                .is_some()
        );
        assert_eq!(s.scene_since, at, "its animations start now");
        assert!(s.screen_fade.is_some(), "the second one fades in");
    }

    /// The screens moved out of lib.rs stay out: no `Scene` variant of their own, no state of
    /// theirs in lib.rs.
    /// Every screen moved out so far.
    const MOVED: &[&str] = &[
        "CalibrateDone",
        "VideoEdit",
        "LayoutEdit",
        "HudEdit",
        "EditorHome",
        "Preview",
        "Download",
    ];

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
