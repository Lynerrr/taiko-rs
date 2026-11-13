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
    /// The window's own: the volume (Alt+arrows, Alt+wheel, the wheel over its notes) and
    /// Ctrl+O for the settings.
    WindowKeys,
    /// The terms, to be agreed to.
    Terms,
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
pub(crate) const OVER_SCREENS: [Layer; 15] = [
    Layer::Splash,
    Layer::GuideBinding,
    Layer::Guide,
    Layer::WindowKeys,
    Layer::Terms,
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

/// The layers the wheel walks, top first, on every screen but the editor's (which take it
/// before them, unless the friends overlay is open).
pub(crate) const WHEEL_LAYERS: [Layer; 6] = [
    Layer::WindowKeys,
    Layer::Terms,
    Layer::Changelog,
    Layer::Medals,
    Layer::Social,
    Layer::ModSelect,
];

/// The layers the left button walks, top first, after the screenshot (over everything) and
/// what the editors' own drags and the editor take before them.
pub(crate) const BUTTON_LAYERS: [Layer; 6] = [
    Layer::Social,
    Layer::Splash,
    Layer::Unlock,
    Layer::Medals,
    Layer::Profile,
    Layer::ModSelect,
];

/// The layers drawn under the toasts, bottom first.
pub(crate) const DRAW_UNDER_TOASTS: [Layer; 8] = [
    Layer::WindowKeys,
    Layer::Social,
    Layer::Guide,
    Layer::Profile,
    Layer::Medals,
    Layer::Unlock,
    Layer::Changelog,
    Layer::Terms,
];

/// The layers drawn over everything but the splash, bottom first.
pub(crate) const DRAW_ON_TOP: [Layer; 2] = [Layer::Crash, Layer::Quit];

impl State {
    /// Draws `layers`, bottom first.
    pub(crate) fn draw_layers(&mut self, layers: &[Layer], canvas: &mut Canvas, now: Instant) {
        for &layer in layers {
            self.draw_layer(layer, canvas, now);
        }
    }

    fn draw_layer(&mut self, layer: Layer, canvas: &mut Canvas, now: Instant) {
        match layer {
            Layer::WindowKeys => {
                if !self.in_song() {
                    self.volume.hold(now);
                }
                if self.volume.visible(now) {
                    self.volume.draw(canvas, &self.cfg, now);
                }
            }
            Layer::Social => {
                if self.overlay_allowed() && self.splash.is_none() {
                    let logged_in = self.mp_menu.account.is_some();
                    let (lobby, lobby_unread) = match &self.mp {
                        Some(s) => (Some(&s.chat[..]), s.chat_unread),
                        None => (None, 0),
                    };
                    // The screen behind darkens, but not a play being watched.
                    let dim = !self.in_song() && !self.in_editor();
                    let animate = self.cfg.animations;
                    let tab = self
                        .social_tab_middle()
                        .filter(|_| logged_in)
                        .map(|to| self.social.glide_tab(to, canvas.dt_ms, animate));
                    if self.social.open {
                        social::draw(
                            canvas,
                            &self.social,
                            logged_in,
                            lobby,
                            lobby_unread,
                            dim,
                            self.in_editor(),
                            tab,
                        );
                    } else if self.social.closing().is_some() {
                        let hits = canvas.hits.len();
                        social::draw(
                            canvas,
                            &self.social,
                            logged_in,
                            lobby,
                            lobby_unread,
                            dim,
                            self.in_editor(),
                            tab,
                        );
                        canvas.hits.truncate(hits);
                    } else if let Some(middle) = tab {
                        social::draw_chip(canvas, &self.social, middle, lobby_unread);
                    }
                    if let Some(x) = self.editor_chip_x(canvas.width)
                        && logged_in
                    {
                        social::draw_editor_chip(canvas, &self.social, x, lobby_unread);
                    }
                    social::draw_notes(canvas, &self.social, self.in_editor());
                }
            }
            Layer::Guide => {
                if let Some(page) = self.guide.page.filter(|_| self.splash.is_none()) {
                    let hold = self
                        .guide_hold_left()
                        .map(|left| left.as_secs_f32().ceil() as u32);
                    guide::draw(canvas, page, self.guide.binding, hold, &self.cfg);
                }
            }
            Layer::Profile => {
                if let Some(p) = &self.profile {
                    p.draw(canvas);
                }
            }
            Layer::Medals => {
                if let Some(view) = &self.medals {
                    view.draw(canvas);
                }
            }
            Layer::Unlock => self.unlock.draw(canvas),
            Layer::Changelog => {
                if let Some(view) = &self.changelog {
                    view.draw(canvas);
                }
            }
            Layer::Terms => {
                if self.terms_showing()
                    && let Some(view) = &self.terms
                {
                    view.draw(canvas);
                }
            }
            Layer::Crash => self.draw_crash(canvas),
            Layer::Quit => self.draw_quit(canvas, now),
            _ => {}
        }
    }
}

impl State {
    /// Walks `layers` top-down with the left button going down (`pressed`) or up; true when
    /// one took it.
    pub(crate) fn layers_button(
        &mut self,
        layers: &[Layer],
        pressed: bool,
        window: Option<&Window>,
    ) -> bool {
        layers
            .iter()
            .any(|&layer| self.layer_button(layer, pressed, window))
    }

    /// Gives the left button to `layer`; true when it took it.
    fn layer_button(&mut self, layer: Layer, pressed: bool, window: Option<&Window>) -> bool {
        match layer {
            Layer::Shot => {
                if self.shot_view.is_none() {
                    return false;
                }
                if pressed {
                    self.close_shot();
                }
            }
            Layer::Social => {
                if !pressed || self.splash.is_some() {
                    return false;
                }
                let (vx, vy) = self.mouse_virtual();
                let width = render::VIRTUAL_HEIGHT * self.window_w / self.window_h.max(1.0);
                let [px, py, pw, ph] = social::panel_rect(width);
                let on_panel =
                    self.social.open && vx >= px && vx <= px + pw && vy >= py && vy <= py + ph;
                let on_social = matches!(
                    self.hit_at(vx, vy),
                    Some(hit::Hit::SocialToggle | hit::Hit::SocialNote(_))
                );
                if on_panel || on_social {
                    if on_panel {
                        self.social.typing = false;
                    }
                    if let Some(hit) = self.hit_at(vx, vy) {
                        self.click(hit, window);
                    }
                    return true;
                }
                if !self.social.open {
                    return false;
                }
                self.social.close();
            }
            Layer::Splash => {
                let Some(s) = &mut self.splash else {
                    return false;
                };
                if pressed {
                    s.skip(Instant::now(), self.audio.as_ref(), "a left click");
                }
            }
            Layer::Unlock => {
                if !self.unlock.showing() {
                    return false;
                }
                if pressed {
                    self.unlock.skip();
                }
            }
            Layer::Medals => {
                if self.medals.is_none() {
                    return false;
                }
                // Its tabs and its Back button.
                if pressed {
                    let (vx, vy) = self.mouse_virtual();
                    if let Some(hit @ (hit::Hit::Key(_) | hit::Hit::MedalTab(_))) =
                        self.hit_at(vx, vy)
                    {
                        self.click(hit, window);
                    }
                }
            }
            Layer::Profile => {
                if self.profile.is_none() {
                    return false;
                }
                // Only its buttons.
                if pressed {
                    let (vx, vy) = self.mouse_virtual();
                    if let Some(hit @ hit::Hit::Key(_)) = self.hit_at(vx, vy) {
                        self.click(hit, window);
                    }
                }
            }
            Layer::ModSelect => {
                if self.mod_select.is_none() {
                    return false;
                }
                // Only the mod select's own chips and buttons; nothing under it.
                if pressed {
                    self.mod_select_click(window);
                }
            }
            _ => return false,
        }
        true
    }

    /// A click on the mod select (or the customization tabs, or a preset being named or
    /// deleted over it): only their own chips and buttons take it.
    fn mod_select_click(&mut self, window: Option<&Window>) {
        let (vx, vy) = self.mouse_virtual();
        let hit = self.hit_at(vx, vy);
        let ours = if self.customization_tab().is_some() {
            use hit::Hit as H;
            matches!(
                hit,
                Some(
                    H::UniverseCell(..)
                        | H::UniversePreset(_)
                        | H::UniversePresetDelete(_)
                        | H::ShapeshiftCell(..)
                        | H::ShapeshiftMove(_)
                        | H::TactCell(_)
                        | H::CustomTab(_)
                        | H::Key(_)
                )
            )
        } else if self.preset_naming.is_some() || self.preset_deleting.is_some() {
            // Naming or deleting a preset: only the window's own buttons (Save or
            // Delete is Enter, Cancel or Keep is Esc) take a click.
            matches!(hit, Some(hit::Hit::Key(KeyCode::Enter | KeyCode::Escape)))
        } else {
            use hit::Hit as H;
            matches!(
                hit,
                Some(H::ModToggle(_) | H::ModPreset(_) | H::ModPresetSave | H::Key(_))
            )
        };
        if let Some(hit) = hit.filter(|_| ours) {
            self.click(hit, window);
        }
    }
}

impl State {
    /// Walks `layers` top-down with the wheel turned `notches`; true when one took it.
    pub(crate) fn layers_wheel(&mut self, layers: &[Layer], notches: i32) -> bool {
        layers.iter().any(|&layer| self.layer_wheel(layer, notches))
    }

    /// Gives the wheel to `layer`; true when it took it.
    fn layer_wheel(&mut self, layer: Layer, notches: i32) -> bool {
        match layer {
            // Alt anywhere; with the pointer moved onto one of the volume's notes, also
            // without Alt.
            Layer::WindowKeys => {
                if !(self.volume.hovering(Instant::now()) || self.modifiers_alt) {
                    return false;
                }
                self.turn_volume(notches);
            }
            Layer::Changelog => {
                let Some(view) = &mut self.changelog else {
                    return false;
                };
                view.scroll_by(-notches * 3);
            }
            Layer::Terms => {
                if !self.terms_showing() {
                    return false;
                }
                if let Some(view) = &mut self.terms {
                    view.scroll_by(-notches * 3);
                }
            }
            Layer::Medals => {
                let Some(view) = &mut self.medals else {
                    return false;
                };
                view.wheel(notches);
            }
            Layer::Social => {
                if !self.social.open || self.modifiers_alt {
                    return false;
                }
                let (vx, vy) = self.mouse_virtual();
                let width = render::VIRTUAL_HEIGHT * self.window_w / self.window_h.max(1.0);
                let [px, py, pw, ph] = social::panel_rect(width);
                if !(vx >= px && vx <= px + pw && vy >= py && vy <= py + ph) {
                    return false;
                }
                self.social.wheel(notches, self.mp.is_some());
            }
            Layer::ModSelect => {
                let Some(cur) = self.mod_select.filter(|_| !self.modifiers_alt) else {
                    return false;
                };
                self.mod_select = Some(mods::step(cur, 0, -notches));
            }
            _ => return false,
        }
        true
    }

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
            Layer::Terms => {
                if !self.terms_showing() {
                    return false;
                }
                let scrolls = matches!(
                    code,
                    KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::PageUp | KeyCode::PageDown
                );
                // A held key doesn't answer: Enter held from the form mustn't agree.
                if (!repeat || scrolls)
                    && let Some(answer) = self.terms.as_mut().and_then(|v| v.key(code))
                {
                    self.terms_answer(answer);
                }
            }
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

/// The exit question and the last run's crash report.
impl State {
    /// The crash question is up: after the splash, over everything but the exit question.
    pub(crate) fn crash_shown(&self) -> bool {
        self.crash.is_some() && self.splash.is_none() && !self.quit_asked
    }
    /// A key while "taiko.rs crashed last time" is up: copy the report, or not.
    pub(crate) fn crash_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter
            | KeyCode::NumpadEnter
            | KeyCode::KeyC
            | KeyCode::Digit1
            | KeyCode::Numpad1 => {
                let Some(report) = self.crash.take() else {
                    return;
                };
                let text = if clipboard::set(&report) {
                    tr!(
                        "crash log copied: paste it in #bugs on Discord",
                        "クラッシュログをコピーしました: Discordの#bugsに貼ってください",
                        "лог краша скопирован: вставьте его в #bugs в Discord"
                    )
                } else {
                    tr!(
                        "couldn't copy it: send crash.log from the game's data folder",
                        "コピーできませんでした: データフォルダのcrash.logを送ってください",
                        "не удалось скопировать: пришлите crash.log из папки данных игры"
                    )
                };
                self.toast = Some(Toast {
                    text: text.into(),
                    until: Instant::now() + Duration::from_secs(6),
                });
            }
            KeyCode::Escape | KeyCode::Backspace | KeyCode::Digit2 | KeyCode::Numpad2 => {
                self.crash = None
            }
            _ => {}
        }
    }
    /// "taiko.rs crashed last time": the report to copy and send in.
    pub(crate) fn draw_crash(&self, c: &mut Canvas) {
        if !self.crash_shown() {
            return;
        }
        let w = c.width;
        c.rect(0.0, 0.0, w, render::VIRTUAL_HEIGHT, rgba(0x000000, 0.6));
        let (bw, bh) = (380.0f32.min(w - 32.0), 132.0);
        let (x, y) = ((w - bw) / 2.0, (render::VIRTUAL_HEIGHT - bh) / 2.0);
        c.round_rect(
            [x, y],
            [bw, bh],
            10.0,
            rgba(0x1d1d27, 1.0),
            rgba(accent(), 1.0),
            1.5,
        );
        c.text_centered(
            w / 2.0,
            y + 18.0,
            12.0,
            rgba(0xffffff, 1.0),
            tr!(
                "taiko.rs crashed last time",
                "前回taiko.rsがクラッシュしました",
                "taiko.rs вылетел в прошлый раз"
            ),
        );
        let room = bw - 32.0;
        let lines = if self.crash_sent {
            [
                tr!(
                    "Its report went to the developers by itself.",
                    "レポートは自動で開発者に送られました。",
                    "Отчёт уже ушёл разработчикам сам."
                ),
                tr!(
                    "To say what you were doing, paste the log in #bugs.",
                    "何をしていたか伝えるには、ログを#bugsに貼ってください。",
                    "Чтобы рассказать, что вы делали, вставьте лог в #bugs."
                ),
            ]
        } else {
            [
                tr!(
                    "Copy the crash log and paste it in #bugs",
                    "クラッシュログをコピーして",
                    "Скопируйте лог краша и вставьте его"
                ),
                tr!(
                    "on the taiko.rs Discord, so it gets fixed.",
                    "Discordの#bugsに貼ってください。",
                    "в #bugs в Discord taiko.rs, чтобы мы починили."
                ),
            ]
        };
        for (k, line) in lines.iter().enumerate() {
            let size = Canvas::fit_size(8.0, 6.5, room, line);
            c.text_centered(
                w / 2.0,
                y + 42.0 + 13.0 * k as f32,
                size,
                rgba(0xc8c8d8, 1.0),
                &Canvas::fit(size, room, line),
            );
        }
        let gap = 10.0;
        let button = (bw - 36.0 - gap) / 2.0;
        let copy = tr!(
            "1. Copy log  (Enter)",
            "1. コピー  (Enter)",
            "1. Копировать  (Enter)"
        );
        let close = tr!("2. Close  (Esc)", "2. 閉じる  (Esc)", "2. Закрыть  (Esc)");
        c.button(
            x + 18.0,
            y + 88.0,
            button,
            26.0,
            copy,
            9.0,
            hit::Hit::Key(KeyCode::KeyC),
            true,
        );
        c.button(
            x + 18.0 + button + gap,
            y + 88.0,
            button,
            26.0,
            close,
            9.0,
            hit::Hit::Key(KeyCode::Escape),
            false,
        );
    }
    /// The exit question over everything, sliding down from above the screen.
    pub(crate) fn draw_quit(&self, c: &mut Canvas, now: Instant) {
        if !self.quit_asked {
            return;
        }
        // The main menu's look: one panel, the two answers as notes on it (Exit a don, Stay a kat),
        // the selector on the one Enter picks, or on the one under the pointer.
        let w = c.width;
        let slide = self.quit_slide(now);
        c.rect(
            0.0,
            0.0,
            w,
            render::VIRTUAL_HEIGHT,
            rgba(0x000000, 0.6 * slide),
        );
        let (bw, bh) = (340.0f32.min(w - 32.0), 124.0);
        let (x, rest) = ((w - bw) / 2.0, (render::VIRTUAL_HEIGHT - bh) / 2.0);
        let y = -bh + (rest + bh) * slide;
        style::panel(c, x, y, bw, bh);
        let title = tr!(
            "Exit taiko.rs?",
            "taiko.rsを終了しますか?",
            "Выйти из taiko.rs?"
        );
        let size = Canvas::fit_size(16.0, 10.0, bw - 28.0, title);
        style::title(
            c,
            w / 2.0 - Canvas::text_width(size, title) / 2.0,
            y + 18.0,
            size,
            0xffffff,
            title,
        );
        let gap = 12.0;
        let button = (bw - 36.0 - gap) / 2.0;
        let colours = crate::game::note_colours();
        let options = [
            (
                x + 18.0,
                tr!("Exit", "終了", "Выйти"),
                "1 / Enter",
                colours.don(),
                hit::Hit::Key(KeyCode::KeyY),
            ),
            (
                x + 18.0 + button + gap,
                tr!("Stay", "戻る", "Остаться"),
                "2 / Esc",
                colours.kat(),
                hit::Hit::Key(KeyCode::KeyN),
            ),
        ];
        let oy = y + 62.0;
        let hovered = options
            .iter()
            .position(|o| c.hovered(o.0, oy, button, 40.0));
        let on = hovered.unwrap_or(0);
        for (k, (ox, label, keys, col, hit)) in options.into_iter().enumerate() {
            let selected = k == on;
            if selected {
                c.round_rect(
                    [ox, oy],
                    [button, 40.0],
                    8.0,
                    rgba(style::selector_colour(), 0.12 + 0.05 * style::pulse()),
                    rgba(0, 0.0),
                    0.0,
                );
                // Its easing would trail behind the panel: until that's still, it just stays on.
                let around = [ox - 3.0, oy - 3.0, button + 6.0, 46.0];
                if slide < 1.0 {
                    style::brackets(c, around, 1.0);
                } else {
                    style::selector(c, "quit", around);
                }
            }
            c.hit(ox, oy, button, 40.0, hit);
            let (nx, ny, r) = (ox + 20.0, oy + 20.0, if selected { 10.0 } else { 8.0 });
            style::note(c, nx, ny, r, col, selected, 0.6);
            let room = button - 44.0;
            c.text(
                ox + 38.0,
                oy + 9.0,
                11.0,
                rgba(if selected { 0xffffff } else { 0xa8a8bc }, 1.0),
                &Canvas::fit(11.0, room, label),
            );
            c.text(
                ox + 38.0,
                oy + 24.0,
                6.0,
                rgba(0x8a8aa0, 1.0),
                &Canvas::fit(6.0, room, keys),
            );
        }
    }
    /// A key while "Exit taiko.rs?" is up: yes, no, or nothing. 1 and 2 answer like osu!'s.
    pub(crate) fn quit_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter
            | KeyCode::NumpadEnter
            | KeyCode::KeyY
            | KeyCode::Digit1
            | KeyCode::Numpad1 => self.quit = true,
            KeyCode::KeyQ if self.modifiers_ctrl => self.quit = true,
            KeyCode::Escape
            | KeyCode::KeyN
            | KeyCode::Backspace
            | KeyCode::Digit2
            | KeyCode::Numpad2 => self.quit_asked = false,
            _ => {}
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
            | Layer::Terms
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
        assert_eq!(all.iter().map(|&l| counted(l)).sum::<usize>(), 18);
    }

    /// The overlays don't know which screen is under them: what differs by screen is asked of
    /// `State` (docs/client-refactor-next-steps.md, S4).
    #[test]
    fn overlays_do_not_name_the_screens() {
        let scene = ["Scene", "::"].concat();
        let named: Vec<&str> = include_str!("overlays.rs")
            .lines()
            .filter(|l| l.contains(&scene))
            .collect();
        assert!(named.is_empty(), "overlays name screens: {named:?}");
    }
}
