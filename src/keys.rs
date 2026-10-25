//! Hotkeys: every shortcut can be moved to another key (Ctrl, Shift and Alt included) or taken
//! away. The game's handlers know each hotkey by its default key; `State::key` turns a key the
//! player bound into that (`Bindings::resolve`), and a default key that was moved or removed
//! does nothing where its hotkey works.

use winit::keyboard::KeyCode;

use crate::config::{key_name, parse_key};

/// A key with the modifiers held for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chord {
    pub key: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    pub const fn plain(key: KeyCode) -> Chord {
        Chord { key, ctrl: false, shift: false, alt: false }
    }
    const fn ctrl(key: KeyCode) -> Chord {
        Chord { key, ctrl: true, shift: false, alt: false }
    }
    const fn shift(key: KeyCode) -> Chord {
        Chord { key, ctrl: false, shift: true, alt: false }
    }

    /// "Ctrl+Shift+F3".
    pub fn name(self) -> String {
        let mut s = String::new();
        for (on, m) in [(self.ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+")] {
            if on {
                s.push_str(m);
            }
        }
        s + &key_name(self.key)
    }

    pub fn parse(s: &str) -> Option<Chord> {
        let mut c = Chord::plain(KeyCode::F35);
        let mut rest = s.trim();
        loop {
            let lower = rest.to_ascii_lowercase();
            if lower.starts_with("ctrl+") {
                c.ctrl = true;
            } else if lower.starts_with("alt+") {
                c.alt = true;
            } else if lower.starts_with("shift+") {
                c.shift = true;
            } else {
                break;
            }
            rest = &rest[rest.find('+')? + 1..];
        }
        c.key = parse_key(rest)?;
        Some(c)
    }

    /// Whether pressing `pressed` sets it off. With `shift_free` Shift may be held or not (the
    /// hotkey uses it to go the other way, or further), unless the chord itself needs it.
    fn fits(self, pressed: Chord, shift_free: bool) -> bool {
        self.key == pressed.key && self.ctrl == pressed.ctrl && self.alt == pressed.alt && (self.shift == pressed.shift || (shift_free && !self.shift))
    }
}

/// Where a hotkey works.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    /// Song select.
    Select,
    /// A song being played (not paused).
    Play,
    Paused,
    Results,
    /// Anything else (the hotkeys that work anywhere still do).
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hotkey {
    ScrollSlower,
    ScrollFaster,
    OffsetDown,
    OffsetUp,
    VisualOffsetDown,
    VisualOffsetUp,
    PresentMode,
    FpsCap,
    FpsOverlay,
    Fullscreen,
    Quit,
    Friends,
    Update,
    Mods,
    Settings,
    Random,
    SpeedDown,
    SpeedUp,
    PlayFrom,
    Preview,
    Download,
    Sort,
    Collections,
    Pause,
    SkipIntro,
    QuickRetry,
    Restart,
    ApplyOffset,
    MapInfo,
    Retry,
    WatchReplay,
    ExportVideo,
}

const ANYWHERE: &[Place] = &[Place::Select, Place::Play, Place::Paused, Place::Results, Place::Other];

impl Hotkey {
    pub const ALL: [Hotkey; 32] = [
        Hotkey::Pause,
        Hotkey::SkipIntro,
        Hotkey::QuickRetry,
        Hotkey::Restart,
        Hotkey::ApplyOffset,
        Hotkey::MapInfo,
        Hotkey::Mods,
        Hotkey::Settings,
        Hotkey::Random,
        Hotkey::SpeedDown,
        Hotkey::SpeedUp,
        Hotkey::PlayFrom,
        Hotkey::Preview,
        Hotkey::Download,
        Hotkey::Sort,
        Hotkey::Collections,
        Hotkey::Retry,
        Hotkey::WatchReplay,
        Hotkey::ExportVideo,
        Hotkey::ScrollSlower,
        Hotkey::ScrollFaster,
        Hotkey::OffsetDown,
        Hotkey::OffsetUp,
        Hotkey::VisualOffsetDown,
        Hotkey::VisualOffsetUp,
        Hotkey::Friends,
        Hotkey::Fullscreen,
        Hotkey::PresentMode,
        Hotkey::FpsCap,
        Hotkey::FpsOverlay,
        Hotkey::Update,
        Hotkey::Quit,
    ];

    /// Its default key first (what the game's handlers know it by), then other keys that do it
    /// too (the numpad's - and +).
    pub fn defaults(self) -> &'static [Chord] {
        use KeyCode::*;
        match self {
            Hotkey::ScrollSlower => const { &[Chord::plain(F3)] },
            Hotkey::ScrollFaster => const { &[Chord::plain(F4)] },
            Hotkey::OffsetDown => const { &[Chord::plain(Minus), Chord::plain(NumpadSubtract)] },
            Hotkey::OffsetUp => const { &[Chord::plain(Equal), Chord::plain(NumpadAdd)] },
            Hotkey::VisualOffsetDown => const { &[Chord::ctrl(Minus), Chord::ctrl(NumpadSubtract)] },
            Hotkey::VisualOffsetUp => const { &[Chord::ctrl(Equal), Chord::ctrl(NumpadAdd)] },
            Hotkey::PresentMode => const { &[Chord::plain(F7)] },
            Hotkey::FpsCap => const { &[Chord::plain(F8)] },
            Hotkey::FpsOverlay => const { &[Chord::plain(F10)] },
            Hotkey::Fullscreen => const { &[Chord::plain(F11)] },
            Hotkey::Quit => const { &[Chord::plain(F12)] },
            Hotkey::Friends => const { &[Chord::plain(F9)] },
            Hotkey::Update => const { &[Chord::ctrl(KeyU)] },
            Hotkey::Mods => const { &[Chord::plain(F1)] },
            Hotkey::Settings => const { &[Chord::plain(Tab)] },
            Hotkey::Random => const { &[Chord::plain(F2)] },
            Hotkey::SpeedDown => const { &[Chord::plain(BracketLeft)] },
            Hotkey::SpeedUp => const { &[Chord::plain(BracketRight)] },
            Hotkey::PlayFrom => const { &[Chord::shift(Enter)] },
            Hotkey::Preview => const { &[Chord::ctrl(Enter)] },
            Hotkey::Download => const { &[Chord::ctrl(KeyD)] },
            Hotkey::Sort => const { &[Chord::ctrl(KeyS)] },
            Hotkey::Collections => const { &[Chord::ctrl(KeyL)] },
            Hotkey::Pause => const { &[Chord::plain(Escape)] },
            Hotkey::SkipIntro => const { &[Chord::plain(Space)] },
            Hotkey::QuickRetry => const { &[Chord::plain(Backquote)] },
            Hotkey::Restart => const { &[Chord::ctrl(KeyR)] },
            Hotkey::ApplyOffset => const { &[Chord::plain(KeyO)] },
            Hotkey::MapInfo => const { &[Chord::plain(Tab)] },
            Hotkey::Retry => const { &[Chord::plain(Enter)] },
            Hotkey::WatchReplay => const { &[Chord::plain(KeyR)] },
            Hotkey::ExportVideo => const { &[Chord::plain(KeyV)] },
        }
    }

    pub fn default(self) -> Chord {
        self.defaults()[0]
    }

    /// Shift held with it does more of it, or the other way (Shift+F2 goes back, Shift + - / +
    /// moves 5 ms): it stays a modifier wherever the hotkey is bound.
    fn shift_free(self) -> bool {
        matches!(
            self,
            Hotkey::OffsetDown | Hotkey::OffsetUp | Hotkey::VisualOffsetDown | Hotkey::VisualOffsetUp | Hotkey::Random | Hotkey::SpeedDown | Hotkey::SpeedUp | Hotkey::Sort
        )
    }

    pub fn places(self) -> &'static [Place] {
        match self {
            Hotkey::Mods
            | Hotkey::Settings
            | Hotkey::Random
            | Hotkey::SpeedDown
            | Hotkey::SpeedUp
            | Hotkey::PlayFrom
            | Hotkey::Preview
            | Hotkey::Download
            | Hotkey::Sort
            | Hotkey::Collections => &[Place::Select],
            Hotkey::Pause | Hotkey::ApplyOffset => &[Place::Play, Place::Paused],
            Hotkey::SkipIntro | Hotkey::QuickRetry | Hotkey::MapInfo => &[Place::Play],
            Hotkey::Restart => &[Place::Play, Place::Paused, Place::Results],
            Hotkey::Retry | Hotkey::WatchReplay | Hotkey::ExportVideo => &[Place::Results],
            _ => ANYWHERE,
        }
    }

    /// For the settings list.
    pub fn label(self) -> &'static str {
        match self {
            Hotkey::ScrollSlower => "Scroll speed down",
            Hotkey::ScrollFaster => "Scroll speed up",
            Hotkey::OffsetDown => "Offset - (map / all)",
            Hotkey::OffsetUp => "Offset + (map / all)",
            Hotkey::VisualOffsetDown => "Visual offset -",
            Hotkey::VisualOffsetUp => "Visual offset +",
            Hotkey::PresentMode => "Next present mode",
            Hotkey::FpsCap => "Next FPS cap",
            Hotkey::FpsOverlay => "FPS overlay",
            Hotkey::Fullscreen => "Fullscreen",
            Hotkey::Quit => "Quit taiko-rs",
            Hotkey::Friends => "Friends and chat",
            Hotkey::Update => "Update",
            Hotkey::Mods => "Song select: mods",
            Hotkey::Settings => "Song select: settings",
            Hotkey::Random => "Song select: random (Shift: back)",
            Hotkey::SpeedDown => "Song select: speed -",
            Hotkey::SpeedUp => "Song select: speed +",
            Hotkey::PlayFrom => "Song select: play from the flag",
            Hotkey::Preview => "Song select: autoplay preview",
            Hotkey::Download => "Song select: download maps",
            Hotkey::Sort => "Song select: sort",
            Hotkey::Collections => "Song select: collections",
            Hotkey::Pause => "Song: pause",
            Hotkey::SkipIntro => "Song: skip intro",
            Hotkey::QuickRetry => "Song: quick retry",
            Hotkey::Restart => "Song / results: restart",
            Hotkey::ApplyOffset => "Song: apply offset suggestion",
            Hotkey::MapInfo => "Song: offset / UR line",
            Hotkey::Retry => "Results: retry",
            Hotkey::WatchReplay => "Results: watch replay",
            Hotkey::ExportVideo => "Results: export video",
        }
    }

    /// For the config: `hotkey.<name> = <chord>`.
    pub fn key(self) -> &'static str {
        match self {
            Hotkey::ScrollSlower => "scroll_slower",
            Hotkey::ScrollFaster => "scroll_faster",
            Hotkey::OffsetDown => "offset_down",
            Hotkey::OffsetUp => "offset_up",
            Hotkey::VisualOffsetDown => "visual_offset_down",
            Hotkey::VisualOffsetUp => "visual_offset_up",
            Hotkey::PresentMode => "present_mode",
            Hotkey::FpsCap => "fps_cap",
            Hotkey::FpsOverlay => "fps_overlay",
            Hotkey::Fullscreen => "fullscreen",
            Hotkey::Quit => "quit",
            Hotkey::Friends => "friends",
            Hotkey::Update => "update",
            Hotkey::Mods => "mods",
            Hotkey::Settings => "settings",
            Hotkey::Random => "random",
            Hotkey::SpeedDown => "speed_down",
            Hotkey::SpeedUp => "speed_up",
            Hotkey::PlayFrom => "play_from",
            Hotkey::Preview => "preview",
            Hotkey::Download => "download",
            Hotkey::Sort => "sort",
            Hotkey::Collections => "collections",
            Hotkey::Pause => "pause",
            Hotkey::SkipIntro => "skip_intro",
            Hotkey::QuickRetry => "quick_retry",
            Hotkey::Restart => "restart",
            Hotkey::ApplyOffset => "apply_offset",
            Hotkey::MapInfo => "map_info",
            Hotkey::Retry => "retry",
            Hotkey::WatchReplay => "watch_replay",
            Hotkey::ExportVideo => "export_video",
        }
    }

    fn index(self) -> usize {
        Hotkey::ALL.iter().position(|&h| h == self).unwrap_or(0)
    }
}

/// What a key press does once the bindings have had their say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resolved {
    /// What it always did.
    Same,
    /// What this default key of a hotkey does (the player moved the hotkey to the key pressed).
    As(Chord),
    /// Nothing: the key's hotkey was moved or removed.
    Nothing,
}

/// The key of each hotkey (`Hotkey::ALL` order); None = removed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Bindings(pub [Option<Chord>; Hotkey::ALL.len()]);

impl Default for Bindings {
    fn default() -> Self {
        Bindings(Hotkey::ALL.map(|h| Some(h.default())))
    }
}

impl Bindings {
    pub fn get(&self, h: Hotkey) -> Option<Chord> {
        self.0[h.index()]
    }

    /// Binds `h` to `chord` (None removes it). Another hotkey it would clash with (on the same
    /// key where both work) loses its key, and is returned.
    pub fn set(&mut self, h: Hotkey, chord: Option<Chord>) -> Option<Hotkey> {
        let mut bumped = None;
        if let Some(c) = chord {
            for other in Hotkey::ALL {
                let shared = other.places().iter().any(|p| h.places().contains(p));
                if other != h && shared && self.get(other).is_some_and(|o| o.fits(c, other.shift_free()) || c.fits(o, h.shift_free())) {
                    self.0[other.index()] = None;
                    bumped = Some(other);
                }
            }
        }
        self.0[h.index()] = chord;
        bumped
    }

    /// The player's own keys, for the config (only those that differ from the defaults).
    pub fn config_lines(&self) -> String {
        let mut out = String::from("# hotkeys moved or removed: hotkey.<name> = a key with Ctrl+ / Alt+ / Shift+ before it, or none\n");
        for h in Hotkey::ALL {
            let c = self.get(h);
            if c != Some(h.default()) {
                out.push_str(&format!("hotkey.{} = {}\n", h.key(), c.map(Chord::name).unwrap_or_else(|| "none".into())));
            }
        }
        out
    }

    /// One `hotkey.<name> = ...` line of the config.
    pub fn parse_line(&mut self, name: &str, value: &str) {
        let Some(h) = Hotkey::ALL.iter().copied().find(|h| h.key() == name) else { return };
        if value.eq_ignore_ascii_case("none") {
            self.0[h.index()] = None;
        } else if let Some(c) = Chord::parse(value) {
            self.0[h.index()] = Some(c);
        }
    }

    /// What `pressed` does in `place`.
    pub fn resolve(&self, pressed: Chord, place: Place) -> Resolved {
        let here = || Hotkey::ALL.into_iter().filter(|h| h.places().contains(&place));
        for h in here() {
            let Some(mine) = self.get(h) else { continue };
            if mine.fits(pressed, h.shift_free()) {
                if mine == h.default() {
                    return Resolved::Same;
                }
                let mut to = h.default();
                to.shift |= h.shift_free() && pressed.shift && !mine.shift;
                return Resolved::As(to);
            }
        }
        if here().any(|h| self.get(h) != Some(h.default()) && h.defaults().iter().any(|d| d.fits(pressed, h.shift_free()))) {
            return Resolved::Nothing;
        }
        Resolved::Same
    }
}

/// Keys that type a character (song select's search takes them as letters, not hotkeys). The
/// brackets aren't: they're song select's speed keys.
pub fn types_text(key: KeyCode) -> bool {
    use KeyCode::*;
    let name = format!("{key:?}");
    name.starts_with("Key") || name.starts_with("Digit") || matches!(key, Space | Minus | Equal | Backslash | Semicolon | Quote | Comma | Period | Slash | Backquote)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: KeyCode, ctrl: bool, shift: bool) -> Chord {
        Chord { key, ctrl, shift, alt: false }
    }

    #[test]
    fn chords_round_trip_through_their_names() {
        for c in [Chord::plain(KeyCode::F3), Chord::ctrl(KeyCode::KeyU), Chord { key: KeyCode::KeyK, ctrl: true, shift: true, alt: true }] {
            assert_eq!(Chord::parse(&c.name()), Some(c), "{}", c.name());
        }
        assert_eq!(Chord::parse("ctrl+shift+f2"), Some(Chord { key: KeyCode::F2, ctrl: true, shift: true, alt: false }));
        assert_eq!(Chord::parse("Ctrl+"), None);
    }

    #[test]
    fn defaults_do_what_they_always_did() {
        let b = Bindings::default();
        assert_eq!(b.resolve(press(KeyCode::F3, false, false), Place::Select), Resolved::Same);
        assert_eq!(b.resolve(press(KeyCode::KeyQ, false, false), Place::Play), Resolved::Same);
    }

    #[test]
    fn a_moved_hotkey_answers_to_its_new_key_only() {
        let mut b = Bindings::default();
        b.set(Hotkey::Random, Some(Chord::ctrl(KeyCode::KeyX)));
        assert_eq!(b.resolve(press(KeyCode::KeyX, true, false), Place::Select), Resolved::As(Chord::plain(KeyCode::F2)));
        // Shift still sends it back.
        assert_eq!(b.resolve(press(KeyCode::KeyX, true, true), Place::Select), Resolved::As(Chord::shift(KeyCode::F2)));
        assert_eq!(b.resolve(press(KeyCode::F2, false, false), Place::Select), Resolved::Nothing);
        assert_eq!(b.resolve(press(KeyCode::F2, false, true), Place::Select), Resolved::Nothing);
        // Only where the hotkey works.
        assert_eq!(b.resolve(press(KeyCode::KeyX, true, false), Place::Play), Resolved::Same);
        assert_eq!(b.resolve(press(KeyCode::F2, false, false), Place::Other), Resolved::Same);
    }

    #[test]
    fn a_removed_hotkey_does_nothing_and_its_numpad_key_neither() {
        let mut b = Bindings::default();
        b.set(Hotkey::OffsetDown, None);
        assert_eq!(b.resolve(press(KeyCode::Minus, false, false), Place::Play), Resolved::Nothing);
        assert_eq!(b.resolve(press(KeyCode::NumpadSubtract, false, true), Place::Play), Resolved::Nothing);
        assert_eq!(b.resolve(press(KeyCode::Minus, true, false), Place::Play), Resolved::Same, "Ctrl - is the visual offset's");
    }

    #[test]
    fn binding_a_taken_key_takes_it_from_the_other_hotkey() {
        let mut b = Bindings::default();
        assert_eq!(b.set(Hotkey::Fullscreen, Some(Chord::plain(KeyCode::F3))), Some(Hotkey::ScrollSlower));
        assert_eq!(b.get(Hotkey::ScrollSlower), None);
        // Tab is settings on song select and the offset line in a song: no clash.
        assert_eq!(b.set(Hotkey::MapInfo, Some(Chord::plain(KeyCode::Tab))), None);
        assert_eq!(b.get(Hotkey::Settings), Some(Chord::plain(KeyCode::Tab)));
    }

    #[test]
    fn only_changed_hotkeys_go_in_the_config() {
        let mut b = Bindings::default();
        assert_eq!(b.config_lines().lines().count(), 1);
        b.set(Hotkey::Pause, Some(Chord::plain(KeyCode::KeyP)));
        b.set(Hotkey::Quit, None);
        let mut again = Bindings::default();
        for line in b.config_lines().lines().filter(|l| !l.starts_with('#')) {
            let (k, v) = line.split_once('=').unwrap();
            again.parse_line(k.trim().strip_prefix("hotkey.").unwrap(), v.trim());
        }
        assert_eq!(again, b);
    }
}
