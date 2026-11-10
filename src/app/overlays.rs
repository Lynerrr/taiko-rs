//! What is over the screen and takes the keyboard before it: the layers a key walks top-down
//! until one takes it. Each layer says whether it took the key; the first that does ends the
//! key's way, so a layer never has to ask whether one above it is open.

use crate::*;

/// A layer over the screens that may take a key. Not every one is a panel: the window's own
/// keys (volume, Ctrl+O) and the clipboard's sit between them, where they always have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    /// A screenshot being shown: any key closes it.
    Shot,
    /// The exit question.
    Quit,
    /// The crash report from the last run.
    Crash,
    /// The splash: any key skips it.
    Splash,
    /// The guide's page asking for a drum key.
    GuideBinding,
    /// The guide.
    Guide,
    /// Alt+arrows for the volume, Ctrl+O for the settings.
    WindowKeys,
    /// What's new.
    Changelog,
    /// A medal being earned: anything moves it on.
    Unlock,
    /// The medals.
    Medals,
    /// A player's profile.
    Profile,
    /// Ctrl+V and Ctrl+C.
    Clipboard,
    /// The friends overlay (F9), and the keyboard while it has it.
    Social,
    /// Song select's customization tabs.
    Customization,
    /// The mod select's question before a preset is deleted.
    PresetDeleting,
    /// The mod select's preset name being typed.
    PresetNaming,
    /// The mod select.
    ModSelect,
}

/// Over everything: before a key counts for the play (its offset is brought up to date after
/// these).
pub(crate) const OVER_EVERYTHING: [Layer; 3] = [Layer::Shot, Layer::Quit, Layer::Crash];

/// Over the screens, top first.
pub(crate) const OVER_SCREENS: [Layer; 14] = [
    Layer::Splash,
    Layer::GuideBinding,
    Layer::Guide,
    Layer::WindowKeys,
    Layer::Changelog,
    Layer::Unlock,
    Layer::Medals,
    Layer::Profile,
    Layer::Clipboard,
    Layer::Social,
    Layer::Customization,
    Layer::PresetDeleting,
    Layer::PresetNaming,
    Layer::ModSelect,
];

impl State {
    /// Walks `layers` top-down with a key; true when one took it.
    pub(crate) fn layers_key(
        &mut self,
        layers: &[Layer],
        code: KeyCode,
        repeat: bool,
        text: Option<&str>,
        at: Instant,
        window: Option<&Window>,
    ) -> bool {
        layers
            .iter()
            .any(|&layer| self.layer_key(layer, code, repeat, text, at, window))
    }

    /// Gives a key to `layer`; true when it took it.
    fn layer_key(
        &mut self,
        layer: Layer,
        code: KeyCode,
        repeat: bool,
        text: Option<&str>,
        at: Instant,
        window: Option<&Window>,
    ) -> bool {
        match layer {
            Layer::Shot => {
                if self.shot_view.is_none() {
                    return false;
                }
                if !repeat {
                    self.close_shot();
                }
            }
            Layer::Quit => {
                if !self.quit_asked {
                    return false;
                }
                if !repeat {
                    self.quit_key(code);
                }
            }
            Layer::Crash => {
                if !self.crash_shown() {
                    return false;
                }
                if !repeat {
                    self.crash_key(code);
                }
            }
            Layer::Splash => {
                let Some(s) = &mut self.splash else {
                    return false;
                };
                // Any key skips the splash and does nothing else.
                if !repeat {
                    s.skip(at, self.audio.as_ref(), &format!("key {code:?}"));
                }
            }
            Layer::GuideBinding => {
                let Some(slot) = self.guide.binding.filter(|_| self.guide.page.is_some()) else {
                    return false;
                };
                let modifier = matches!(
                    code,
                    KeyCode::ShiftLeft
                        | KeyCode::ShiftRight
                        | KeyCode::ControlLeft
                        | KeyCode::ControlRight
                        | KeyCode::AltLeft
                        | KeyCode::AltRight
                        | KeyCode::SuperLeft
                        | KeyCode::SuperRight
                );
                if repeat || modifier {
                    return true;
                }
                self.guide.binding = None;
                if code != KeyCode::Escape && self.bind_drum_key(slot, code) {
                    self.cfg.save();
                }
            }
            Layer::Guide => {
                if self.guide.page.is_none() {
                    return false;
                }
                match code {
                    KeyCode::Enter
                    | KeyCode::NumpadEnter
                    | KeyCode::Space
                    | KeyCode::ArrowRight => self.guide_action(guide::Action::Next),
                    KeyCode::ArrowLeft | KeyCode::Backspace => {
                        self.guide_action(guide::Action::Back)
                    }
                    KeyCode::Escape => self.guide_action(guide::Action::Close),
                    KeyCode::KeyC if self.guide.page == Some(guide::OFFSET_PAGE) => {
                        self.guide_action(guide::Action::Calibrate)
                    }
                    KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4
                        if self.guide.page == Some(0) =>
                    {
                        let slot = [
                            KeyCode::Digit1,
                            KeyCode::Digit2,
                            KeyCode::Digit3,
                            KeyCode::Digit4,
                        ]
                        .iter()
                        .position(|&k| k == code);
                        self.guide.binding = slot;
                    }
                    _ => {}
                }
            }
            Layer::WindowKeys => return self.window_key(code, repeat),
            Layer::Changelog => {
                let Some(view) = &mut self.changelog else {
                    return false;
                };
                if (!repeat
                    || matches!(
                        code,
                        KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::PageUp | KeyCode::PageDown
                    ))
                    && view.key(code)
                {
                    self.changelog = None;
                }
            }
            Layer::Unlock => {
                if !self.unlock.showing() {
                    return false;
                }
                self.unlock.skip();
            }
            Layer::Medals => {
                let Some(view) = &mut self.medals else {
                    return false;
                };
                match code {
                    KeyCode::Escape | KeyCode::Backspace => self.medals = None,
                    KeyCode::ArrowLeft => view.step_tab(-1),
                    KeyCode::ArrowRight | KeyCode::Tab => view.step_tab(1),
                    KeyCode::ArrowUp => view.wheel(1),
                    KeyCode::ArrowDown => view.wheel(-1),
                    KeyCode::PageUp => view.wheel(5),
                    KeyCode::PageDown => view.wheel(-5),
                    _ => {}
                }
            }
            Layer::Profile => {
                if self.profile.is_none() {
                    return false;
                }
                // A profile is over everything: any of these closes it, the rest do nothing.
                if matches!(
                    code,
                    KeyCode::Escape | KeyCode::Backspace | KeyCode::Enter | KeyCode::NumpadEnter
                ) {
                    self.profile = None;
                } else if code == KeyCode::KeyM {
                    let name = self.profile.take().map(|p| p.name);
                    self.open_medals(name);
                } else if code == KeyCode::KeyB
                    && let Some(p) = &self.profile
                {
                    let url = profile_url(&p.name);
                    if !open_url(&url) {
                        self.toast(url);
                    }
                }
            }
            Layer::Clipboard => return self.clipboard_key(code, repeat, at, window),
            Layer::Social => return self.social_layer_key(code, repeat, text, window),
            Layer::Customization => {
                if self.customization_tab().is_none() {
                    return false;
                }
                self.customization_key(code, text, repeat);
            }
            Layer::PresetDeleting => {
                if self.mod_select.is_none() || self.preset_deleting.is_none() {
                    return false;
                }
                self.preset_deleting_key(code);
            }
            Layer::PresetNaming => {
                if self.mod_select.is_none() || self.preset_naming.is_none() {
                    return false;
                }
                self.preset_naming_key(code, text);
            }
            Layer::ModSelect => {
                if self.mod_select.is_none() {
                    return false;
                }
                if !repeat
                    || matches!(
                        code,
                        KeyCode::ArrowUp
                            | KeyCode::ArrowDown
                            | KeyCode::ArrowLeft
                            | KeyCode::ArrowRight
                    )
                {
                    self.mod_select_key(code);
                }
            }
        }
        true
    }

    /// Alt+Left/Right picks what the volume turns, Alt+Up/Down turns it (held, it keeps
    /// turning), Ctrl+O opens the settings; being typed or bound, they are just keys.
    fn window_key(&mut self, code: KeyCode, repeat: bool) -> bool {
        let free = self.settings_ui.binding.is_none() && self.settings_ui.editing.is_none();
        if !free {
            return false;
        }
        if self.modifiers_alt
            && !self.modifiers_ctrl
            && matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight)
        {
            self.volume.pick_next(
                if code == KeyCode::ArrowLeft { -1 } else { 1 },
                Instant::now(),
            );
            return true;
        }
        if self.modifiers_alt
            && !self.modifiers_ctrl
            && matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown)
        {
            self.turn_volume(if code == KeyCode::ArrowUp { 1 } else { -1 });
            return true;
        }
        if code == KeyCode::KeyO
            && self.modifiers_ctrl
            && !self.modifiers_shift
            && !self.modifiers_alt
        {
            if !repeat {
                self.settings_anywhere();
            }
            return true;
        }
        false
    }

    /// Ctrl+V types the clipboard's text into whatever takes text here; Ctrl+C copies what's
    /// being typed, else what's picked.
    fn clipboard_key(
        &mut self,
        code: KeyCode,
        repeat: bool,
        at: Instant,
        window: Option<&Window>,
    ) -> bool {
        if self.modifiers_ctrl && code == KeyCode::KeyV && self.takes_text() {
            if !repeat {
                let text = clipboard::get()
                    .map(|t| clipboard::one_line(&t))
                    .unwrap_or_default();
                if !text.is_empty() {
                    self.modifiers_ctrl = false;
                    self.key_inner(KeyCode::KeyV, false, Some(&text), at, window);
                    self.modifiers_ctrl = true;
                }
            }
            return true;
        }
        if self.modifiers_ctrl
            && code == KeyCode::KeyC
            && !repeat
            && let Some(text) = self.copy_text()
        {
            if clipboard::set(&text) {
                self.toast(format!("copied: {text}"));
            } else {
                self.toast("no clipboard to copy to".into());
            }
            return true;
        }
        false
    }

    /// The friends overlay: F9 anywhere outside songs; while open it takes the keyboard
    /// (function keys still work).
    fn social_layer_key(
        &mut self,
        code: KeyCode,
        repeat: bool,
        text: Option<&str>,
        window: Option<&Window>,
    ) -> bool {
        if code == KeyCode::F9 && !repeat {
            self.toggle_social();
            return true;
        }
        if self.social.open && !self.social_keys() && code == KeyCode::Escape {
            self.social_key(code, text);
            return true;
        }
        if self.social_keys() {
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
                    | KeyCode::F10
                    | KeyCode::F11
                    | KeyCode::F12
            );
            if !(function_key && self.handle_global_key(code, window)) {
                self.social_key(code, text);
            }
            return true;
        }
        false
    }
}

/// How long a new player stays on the guide's welcome and offset pages before moving on.
const GUIDE_HOLD: Duration = Duration::from_secs(4);

/// The guide's own actions, wherever they come from (its keys, its buttons).
impl State {
    pub(crate) fn guide_action(&mut self, action: guide::Action) {
        let Some(page) = self.guide.page else { return };
        if self.guide_hold_left().is_some()
            && matches!(
                action,
                guide::Action::Next | guide::Action::Close | guide::Action::Page(_)
            )
        {
            return;
        }
        self.guide.binding = None;
        let closing = action == guide::Action::Close
            || (action == guide::Action::Next && page + 1 == guide::pages());
        match action {
            // A new player doesn't leave without the page that says to calibrate.
            _ if closing && !self.cfg.guide_seen && !self.guide.read[guide::OFFSET_PAGE] => {
                self.guide.page = Some(guide::OFFSET_PAGE)
            }
            guide::Action::Bind(slot) => self.guide.binding = Some(slot),
            guide::Action::Back => self.guide.page = Some(page.saturating_sub(1)),
            guide::Action::Next if page + 1 < guide::pages() => self.guide.page = Some(page + 1),
            guide::Action::Next | guide::Action::Close => self.guide.page = None,
            guide::Action::Page(i) => self.guide.page = Some(i),
            guide::Action::Calibrate => {
                self.guide.page = None;
                self.guide.after = Some(page + 1);
                self.start_calibration_as(false);
            }
        }
        // Once closed, it doesn't come up by itself again.
        if self.guide.page.is_none() && !self.cfg.guide_seen {
            self.cfg.guide_seen = true;
            self.cfg.save();
        }
    }

    /// What's left of the time a new player stays on this page of the guide.
    pub(crate) fn guide_hold_left(&self) -> Option<Duration> {
        let page = self.guide.page.filter(|&p| {
            !self.cfg.guide_seen && (p == 0 || p == guide::OFFSET_PAGE) && !self.guide.read[p]
        })?;
        let shown = self
            .guide
            .shown
            .filter(|s| s.0 == page)
            .map_or(Duration::ZERO, |s| s.1.elapsed());
        Some(GUIDE_HOLD.saturating_sub(shown)).filter(|left| !left.is_zero())
    }

    /// Times the page on screen: the clock starts once the splash is out of the way.
    pub(crate) fn tick_guide(&mut self) {
        let page = self.guide.page.filter(|_| self.splash.is_none());
        if page != self.guide.shown.map(|s| s.0) {
            self.guide.shown = page.map(|p| (p, Instant::now()));
        }
        if let Some((p, since)) = self.guide.shown
            && since.elapsed() >= GUIDE_HOLD
        {
            self.guide.read[p] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each layer is walked once: a key can't reach one layer twice, or skip one because it was
    /// left out of both lists.
    #[test]
    fn every_layer_is_walked_once() {
        let all = [OVER_EVERYTHING.as_slice(), OVER_SCREENS.as_slice()].concat();
        for (i, layer) in all.iter().enumerate() {
            assert!(!all[i + 1..].contains(layer), "{layer:?} is walked twice");
        }
        // A new layer makes this match fail to compile until it's counted (and placed above).
        let counted = |l: Layer| match l {
            Layer::Shot
            | Layer::Quit
            | Layer::Crash
            | Layer::Splash
            | Layer::GuideBinding
            | Layer::Guide
            | Layer::WindowKeys
            | Layer::Changelog
            | Layer::Unlock
            | Layer::Medals
            | Layer::Profile
            | Layer::Clipboard
            | Layer::Social
            | Layer::Customization
            | Layer::PresetDeleting
            | Layer::PresetNaming
            | Layer::ModSelect => 1,
        };
        assert_eq!(all.iter().map(|&l| counted(l)).sum::<usize>(), 17);
    }
}
