//! The phone's interface: song select, the mods, settings and downloads laid out for fingers.
//!
//! A phone shows the same 480 units as a monitor on a screen a tenth of the size, so these
//! screens keep the game's look with bigger text and rows a thumb can hit, and lists that follow
//! the finger and fly on when thrown. What they do is what the desktop screens do: they draw the
//! same state and answer with the same `Hit`s. Elsewhere a finger still works as the mouse.

use std::time::{Duration, Instant};

use winit::event::TouchPhase;
use winit::keyboard::KeyCode;
use winit::window::Window;

use crate::config::RATE_RANGE;
use crate::render::{Canvas, Hit, HitBox, VIRTUAL_HEIGHT, accent, approach, rgba};
use crate::settings::{self, Control, ITEMS, Kind, SECTIONS};
use crate::{Scene, State, mods, stars};

/// The song list's width on a `w` wide screen.
pub fn select_list_w(w: f32) -> f32 {
    (w * 0.52).round()
}

/// Song select's preview panel beside the list: left edge, top, width, bottom.
pub fn select_panel(w: f32) -> [f32; 4] {
    let x = select_list_w(w) + 10.0;
    [x, TOP, w - x, SELECT_BOTTOM]
}

/// `pc` on a monitor, `phone` on a phone.
pub fn pick(pc: f32, phone: f32) -> f32 {
    if active() { phone } else { pc }
}

/// The panel of a mod's customization: (x, y, w, h), under the tabs on a phone.
pub fn setup_rect(w: f32) -> (f32, f32, f32, f32) {
    if active() {
        (0.0, 50.0, w, 430.0)
    } else {
        (16.0, 44.0, w - 32.0, 410.0)
    }
}

/// The buttons along the bottom of a panel ending at `bottom`: y, width, height, text size.
pub fn setup_button(bottom: f32) -> (f32, f32, f32, f32) {
    if active() {
        (bottom - 50.0, 120.0, 42.0, 12.0)
    } else {
        (bottom - 26.0, 56.0, 16.0, 7.0)
    }
}

/// Text written for a keyboard, said for a finger.
fn for_touch(s: &str) -> String {
    s.replace("Enter to ", "Tap to ")
        .replace("Enter: ", "Tap: ")
        .replace(", the wheel", "")
}

/// `hint` naming keys: nothing on a phone, else as much as fits.
pub fn key_hint(size: f32, room: f32, hint: &str) -> String {
    if active() {
        String::new()
    } else {
        Canvas::fit(size, room, hint)
    }
}

/// What's under a finger at (`x`, `y`). A fingertip is wide and much of the game was made for a
/// mouse: nothing right under it, the nearest button within `FINGER` counts. A panel that only
/// stops clicks doesn't hide what's near on top of it.
pub fn hit_near(hits: &[HitBox], x: f32, y: f32) -> Option<Hit> {
    let mut near: Option<(f32, Hit)> = None;
    for h in hits.iter().rev() {
        if h.contains(x, y) {
            let passive = matches!(h.hit, Hit::Overlay | Hit::SocialPanel | Hit::HudPanel);
            return match near {
                Some((_, hit)) if passive => Some(hit),
                _ => Some(h.hit),
            };
        }
        let [rx, ry, rw, rh] = h.rect;
        let dx = (rx - x).max(x - rx - rw).max(0.0);
        let dy = (ry - y).max(y - ry - rh).max(0.0);
        let d = dx.max(dy);
        if d <= FINGER && near.is_none_or(|(best, _)| d < best) {
            near = Some((d, h.hit));
        }
    }
    near.map(|n| n.1)
}

/// Whether this build has the phone's interface.
pub fn active() -> bool {
    cfg!(target_os = "android")
}

/// Text sizes. At half resolution these are whole font pixels: 2 and 3 of them.
const SMALL: f32 = 12.0;
const MEDIUM: f32 = 18.0;
/// The bar along the top of a screen, and the gap under it.
const BAR_H: f32 = 48.0;
const TOP: f32 = BAR_H + 10.0;
/// A song's row and a difficulty's in the list, and the gap between cards elsewhere.
const SONG_H: f32 = 58.0;
const DIFF_H: f32 = 44.0;
const GAP: f32 = 6.0;
/// Where song select's list and panel end, above its buttons.
pub const SELECT_BOTTOM: f32 = 426.0;
/// How far from a button a finger still presses it.
const FINGER: f32 = 12.0;
/// How far a finger moves before it's a drag and not a tap.
const SLOP: f32 = 9.0;

/// How long a finger stays put to be a long press.
const LONG_PRESS: Duration = Duration::from_millis(450);
/// The pause button at the top of the screen during play: its half width and its height.
const PAUSE_HALF_W: f32 = 34.0;
const PAUSE_H: f32 = 40.0;

/// Whether `at` is on the pause button of a `width` wide screen.
pub fn on_pause_button(at: (f32, f32), width: f32) -> bool {
    active() && (at.0 - width / 2.0).abs() <= PAUSE_HALF_W + 10.0 && at.1 <= PAUSE_H + 10.0
}

/// The way to pause without a key: a phone's back gesture is too easy to miss mid-song.
pub fn draw_pause_button(c: &mut Canvas, finish: bool) {
    if !active() {
        return;
    }
    let x = c.width / 2.0 - PAUSE_HALF_W;
    c.round_rect(
        [x, 0.0],
        [PAUSE_HALF_W * 2.0, PAUSE_H],
        8.0,
        rgba(0x15151d, 0.55),
        rgba(0xffffff, 0.2),
        1.0,
    );
    let mid = c.width / 2.0;
    if finish {
        c.text_centered(mid, 14.0, SMALL, rgba(0xffffff, 0.9), "End");
        return;
    }
    c.rect(mid - 9.0, 11.0, 6.0, 18.0, rgba(0xffffff, 0.75));
    c.rect(mid + 3.0, 11.0, 6.0, 18.0, rgba(0xffffff, 0.75));
}

/// The lists a finger can scroll.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum List {
    Songs,
    Sections,
    Settings,
    Choices,
    Mods,
    Download,
    Board,
}

/// A list's position: held by the finger, flying on from a throw, or easing to a place.
#[derive(Default)]
pub struct Scroll {
    pub pos: f32,
    /// Units per ms.
    speed: f32,
    max: f32,
    held: bool,
    moved_at: Option<Instant>,
    goal: Option<f32>,
}

impl Scroll {
    /// Moves on by `dt_ms`, within what the list holds now.
    fn frame(&mut self, dt_ms: f32, max: f32) {
        self.max = max.max(0.0);
        if self.held {
            return;
        }
        if let Some(goal) = self.goal {
            let goal = goal.clamp(0.0, self.max);
            approach(&mut self.pos, goal, dt_ms, 50.0);
            if (self.pos - goal).abs() < 0.5 {
                self.pos = goal;
                self.goal = None;
            }
            return;
        }
        self.pos += self.speed * dt_ms;
        self.speed *= (-dt_ms / 420.0).exp();
        if self.speed.abs() < 0.01 {
            self.speed = 0.0;
        }
        // Past an end it springs back.
        let inside = self.pos.clamp(0.0, self.max);
        if inside != self.pos {
            self.speed = 0.0;
            approach(&mut self.pos, inside, dt_ms, 60.0);
            if (self.pos - inside).abs() < 0.5 {
                self.pos = inside;
            }
        }
    }

    fn grab(&mut self) {
        self.held = true;
        self.speed = 0.0;
        self.goal = None;
        self.moved_at = None;
    }

    /// The finger moved `dy` down at `at`.
    fn drag(&mut self, dy: f32, at: Instant) {
        // Heavier past the ends.
        let past = self.pos < 0.0 || self.pos > self.max;
        self.pos -= if past { dy * 0.4 } else { dy };
        if let Some(last) = self.moved_at {
            let ms = at.saturating_duration_since(last).as_secs_f32() * 1000.0;
            if ms > 0.5 {
                self.speed = self.speed * 0.6 + (-dy / ms) * 0.4;
            }
        }
        self.moved_at = Some(at);
    }

    fn release(&mut self, at: Instant) {
        self.held = false;
        let resting = self
            .moved_at
            .is_none_or(|t| at.saturating_duration_since(t) > Duration::from_millis(70));
        self.speed = if resting {
            0.0
        } else {
            self.speed.clamp(-6.0, 6.0)
        };
    }

    /// Brings `top`..`bottom` of the list into a view `view` tall.
    fn show(&mut self, top: f32, bottom: f32, view: f32) {
        if self.held {
            return;
        }
        if top < self.pos {
            self.goal = Some(top);
        } else if bottom > self.pos + view {
            self.goal = Some(if bottom - top > view {
                top
            } else {
                bottom - view
            });
        }
    }
}

/// A finger on a list: a tap if it stays put, else it scrolls.
struct Gesture {
    id: u64,
    /// The list it scrolls; None: a screen made for a wheel, which gets notches.
    list: Option<List>,
    /// The drag not yet turned into notches.
    wheel: f32,
    /// On no list, a drag turns the wheel (a screen made for one); else it's nothing.
    wheels: bool,
    start: (f32, f32),
    /// Where it came down in the window's pixels, and when.
    pixels: (f64, f64),
    since: Instant,
    /// Held long enough to count as a right click: done with.
    long: bool,
    last_y: f32,
    moved: bool,
}

/// The phone interface's own state.
#[derive(Default)]
pub struct Ui {
    songs: Scroll,
    sections: Scroll,
    settings: Scroll,
    choices: Scroll,
    mods: Scroll,
    download: Scroll,
    board: Scroll,
    /// The speed panel is up.
    rate_open: bool,
    /// Song select's right side shows the leaderboard, and how far it has slid there (0 to 1).
    board_open: bool,
    board_slide: f32,
    /// The map and board the leaderboard's position is for.
    board_of: Option<(u64, bool)>,
    /// The lists on screen this frame, later ones on top.
    regions: Vec<([f32; 4], List)>,
    gesture: Option<Gesture>,
    /// What was selected last frame, to bring a new selection into view.
    song_cursor: Option<usize>,
    setting_cursor: Option<usize>,
    choices_open: bool,
}

impl Ui {
    fn scroll(&mut self, list: List) -> &mut Scroll {
        match list {
            List::Songs => &mut self.songs,
            List::Sections => &mut self.sections,
            List::Settings => &mut self.settings,
            List::Choices => &mut self.choices,
            List::Mods => &mut self.mods,
            List::Download => &mut self.download,
            List::Board => &mut self.board,
        }
    }

    pub fn flip_board(&mut self) {
        self.board_open = !self.board_open;
    }

    /// Moves the slide toward the page shown and gives how far it is at the leaderboard (0 to 1).
    pub fn slide_board(&mut self, dt_ms: f32) -> f32 {
        let goal = if self.board_open { 1.0 } else { 0.0 };
        approach(&mut self.board_slide, goal, dt_ms, 45.0);
        if (self.board_slide - goal).abs() < 0.004 {
            self.board_slide = goal;
        }
        self.board_slide
    }

    /// A new frame: its screens say again which lists are up.
    pub fn begin_frame(&mut self) {
        self.regions.clear();
    }

    fn region_at(&self, x: f32, y: f32) -> Option<List> {
        self.regions
            .iter()
            .rev()
            .find(|([rx, ry, rw, rh], _)| x >= *rx && x <= rx + rw && y >= *ry && y <= ry + rh)
            .map(|r| r.1)
    }
}

/// The bar's button out of a screen; returns where the bar's room starts.
fn back_button(c: &mut Canvas, label: &str, hit: Hit) -> f32 {
    c.button(0.0, 0.0, 96.0, BAR_H - 2.0, label, SMALL, hit, false);
    96.0 + 8.0
}

/// A search box in the bar: `text` or, while empty, `hint`.
fn search_box(c: &mut Canvas, x: f32, w: f32, text: &str, hint: &str, focused: bool, hit: Hit) {
    c.round_rect(
        [x, 0.0],
        [w, BAR_H - 2.0],
        6.0,
        rgba(0x1d1d27, 0.95),
        rgba(if focused { accent() } else { 0x3a3a4e }, 1.0),
        1.0,
    );
    let room = w - 24.0;
    let y = (BAR_H - 2.0 - SMALL) / 2.0;
    if text.is_empty() {
        c.text(x + 12.0, y, SMALL, rgba(0x8a8aa0, 1.0), hint);
    } else {
        let shown = Canvas::fit_end(SMALL, room, text);
        c.text(x + 12.0, y, SMALL, rgba(0xffffff, 1.0), &shown);
    }
    c.hit(x, 0.0, w, BAR_H - 2.0, hit);
}

fn panel(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    c.round_rect(
        [x, y],
        [w, h],
        10.0,
        rgba(0x15151d, 0.88),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
}

/// `text` broken into lines `w` wide, `lines` at most.
pub(crate) fn wrap(size: f32, w: f32, text: &str, lines: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if Canvas::text_width(size, &next) <= w || line.is_empty() {
            line = next;
        } else {
            out.push(std::mem::replace(&mut line, word.to_string()));
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.len() > lines {
        let rest = out[lines - 1..].join(" ");
        out.truncate(lines - 1);
        out.push(Canvas::fit(size, w, &rest));
    }
    out
}

impl State {
    /// Song select's panel on a `w` wide screen, clear of the friends' tab.
    pub(crate) fn mobile_select_panel(&self, w: f32) -> [f32; 4] {
        let [x, top, pw, bottom] = select_panel(w);
        [x, top, pw - self.tab_room(), bottom]
    }

    /// What a screen keeps clear on its right for the friends' tab, when it's there.
    fn tab_room(&self) -> f32 {
        if self.mp_menu.account.is_some() && self.social_tab_middle().is_some() {
            crate::social::TAB_ROOM
        } else {
            0.0
        }
    }

    /// One of the phone's screens with a bar along the top is up: the corner chips would sit on it.
    pub(crate) fn mobile_bar(&self) -> bool {
        active()
            && (self.mod_select.is_some()
                || self.medals.is_some()
                || self.player.is_some()
                || self.profile.is_some()
                || matches!(
                    self.scene,
                    Scene::Settings { .. }
                        | Scene::Download
                        | Scene::Results { .. }
                        | Scene::MpMenu
                        | Scene::Lobby
                        | Scene::MpResults
                        | Scene::HudEdit { .. }
                        | Scene::LayoutEdit { .. }
                        | Scene::Calibrate { .. }
                        | Scene::CalibrateDone { .. }
                        | Scene::Preview { .. }
                )
                || (matches!(self.scene, Scene::Select) && !self.on_menu()))
    }

    /// Something made for a wheel is up: a finger dragging over it turns the wheel.
    fn mobile_wheel_screen(&self) -> bool {
        self.changelog.is_some()
            || self.medals.is_some()
            || self.profile.is_some()
            || self.social.open
            || self
                .player
                .as_ref()
                .is_some_and(|p| !crate::player::tall() || p.list_open)
            || matches!(
                self.scene,
                Scene::MpMenu | Scene::Lobby | Scene::MpResults | Scene::Results { .. }
            )
    }

    /// A finger on one of the phone's lists. True if it's taken: a tap is only known when the
    /// finger comes up, so buttons in a list click then, and a drag never clicks them.
    pub(crate) fn mobile_touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f64,
        y: f64,
        at: Instant,
        window: Option<&Window>,
    ) -> bool {
        if !active() {
            return false;
        }
        let k = VIRTUAL_HEIGHT / self.window_h.max(1.0);
        let (vx, vy) = (x as f32 * k, y as f32 * k);
        match phase {
            TouchPhase::Started => {
                if self.mob.gesture.is_some() || self.touch_mouse.is_some() {
                    return false;
                }
                // Screens made of things to drag, and bars and timelines, are pressed at once
                // and dragged, as with a mouse.
                let dragged = matches!(
                    self.scene,
                    Scene::LayoutEdit { .. }
                        | Scene::HudEdit { .. }
                        | Scene::Editor(_)
                        | Scene::EditorHome(_)
                        | Scene::Preview { .. }
                        | Scene::VideoEdit(_)
                ) || matches!(
                    self.hit_at(vx, vy),
                    Some(
                        Hit::SettingLevel(_)
                            | Hit::MusicBar
                            | Hit::PlayerVolume
                            | Hit::SongScrollbar
                    )
                ) || self
                    .preview
                    .graph_rect()
                    .is_some_and(|[x, y, w, h]| vx >= x && vx <= x + w && vy >= y && vy <= y + h);
                if dragged {
                    return false;
                }
                // Everything else is a tap only once the finger comes up where it went down: a
                // finger resting or sliding presses nothing.
                let list = self.mob.region_at(vx, vy);
                let wheels = list.is_none() && self.mobile_wheel_screen();
                if let Some(list) = list {
                    self.mob.scroll(list).grab();
                } else {
                    // The wheel goes to what's under the pointer.
                    self.mouse_moved(x, y);
                }
                self.mob.gesture = Some(Gesture {
                    id,
                    list,
                    wheel: 0.0,
                    wheels,
                    start: (vx, vy),
                    pixels: (x, y),
                    since: at,
                    long: false,
                    last_y: vy,
                    moved: false,
                });
                true
            }
            TouchPhase::Moved => {
                let Some(g) = self.mob.gesture.as_mut().filter(|g| g.id == id) else {
                    return false;
                };
                let far = (vy - g.start.1).abs().max((vx - g.start.0).abs()) > SLOP;
                if !g.moved && far {
                    g.moved = true;
                    g.last_y = vy;
                }
                if g.moved {
                    let (dy, list) = (vy - g.last_y, g.list);
                    g.last_y = vy;
                    if let Some(list) = list {
                        self.mob.scroll(list).drag(dy, at);
                    } else if g.wheels {
                        g.wheel += dy;
                        let total = g.wheel;
                        let step = if self.changelog.is_some() { 60.0 } else { 26.0 };
                        let notches = (total / step).trunc();
                        if notches != 0.0 {
                            g.wheel -= notches * step;
                            self.wheel(notches as i32);
                        }
                    }
                }
                true
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some(g) = self.mob.gesture.take_if(|g| g.id == id) else {
                    return false;
                };
                if let Some(list) = g.list {
                    self.mob.scroll(list).release(at);
                }
                // A finger held there a while and lifted isn't a tap either.
                let quick = at.saturating_duration_since(g.since) < LONG_PRESS;
                if !g.moved && !g.long && quick && phase == TouchPhase::Ended {
                    self.mouse_moved(x, y);
                    #[cfg(target_os = "android")]
                    self.keyboard_tap();
                    self.mouse_button(true, window);
                    self.mouse_button(false, window);
                }
                true
            }
        }
    }

    /// A finger held still on a list is a right click: a song's menu, a mod made a favourite.
    pub(crate) fn mobile_long_press(&mut self, now: Instant) {
        // Only a song's row has more to it than a tap (its menu); elsewhere a finger left resting
        // is nothing.
        let Some(g) = self.mob.gesture.as_mut().filter(|g| {
            g.list == Some(List::Songs)
                && !g.moved
                && !g.long
                && now.saturating_duration_since(g.since) >= LONG_PRESS
        }) else {
            return;
        };
        g.long = true;
        let (x, y) = g.pixels;
        self.mouse_moved(x, y);
        self.right_button(true);
        self.right_button(false);
    }

    /// Song select, laid out as on a monitor with everything bigger: the list on the left, the
    /// preview panel on the right (drawn by the caller, in `select_panel`), buttons along the
    /// bottom.
    pub(crate) fn draw_mobile_select(&mut self, c: &mut Canvas) {
        let w = c.width;
        let back = if self.select.to_menu {
            "< MENU"
        } else {
            "< BACK"
        };
        let x = back_button(c, back, Hit::ToMenu);
        let list_w = select_list_w(w);
        let sort_w = 168.0;
        let search_w = list_w - x - sort_w - 8.0;
        let (filter, searching) = (self.select.filter.clone(), self.select.searching);
        search_box(
            c,
            x,
            search_w,
            &filter,
            "search...",
            searching,
            Hit::TextField,
        );
        let sort = format!("Sort: {}", self.select.sort.name());
        let sx = x + search_w + 8.0;
        c.button(
            sx,
            0.0,
            sort_w,
            BAR_H - 2.0,
            &sort,
            SMALL,
            Hit::SortList,
            false,
        );
        let groups = self.select.song_groups().len();
        let summary = if self.picking_lobby_map {
            "Pick the lobby's map".to_string()
        } else if self.pick_to_edit {
            "Pick a map to edit".to_string()
        } else {
            format!("{groups} songs, {} maps", self.select.rows().len())
        };
        let room = w - list_w - 20.0;
        let summary = Canvas::fit(SMALL, room, &summary);
        let sy = (BAR_H - 2.0 - SMALL) / 2.0;
        c.text_right(w - 4.0, sy, SMALL, rgba(0x9a9ab0, 1.0), &summary);

        // The list's frame, as on a monitor.
        let h = SELECT_BOTTOM - TOP;
        c.round_rect(
            [0.0, TOP],
            [list_w, h],
            8.0,
            rgba(0x15151d, 0.82),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
        self.draw_mobile_songs(c, 4.0, TOP + 4.0, list_w - 8.0, h - 8.0);

        let mods = self.select.mods_label.clone();
        let label = if mods.is_empty() {
            "Mods".to_string()
        } else {
            format!("Mods: {mods}")
        };
        let play = if self.picking_lobby_map {
            "Pick"
        } else if self.pick_to_edit {
            "Edit"
        } else {
            "Play"
        };
        let by = SELECT_BOTTOM + 8.0;
        let bh = VIRTUAL_HEIGHT - by;
        // The other screens are a step back, on the main menu.
        let page = if self.mob.board_open {
            "< Preview"
        } else {
            "Leaderboard >"
        };
        let speed = format!("{:.2}x", self.cfg.rate);
        let changed = (self.cfg.rate - 1.0).abs() > 1e-6;
        let buttons: [(&str, Hit, bool, f32); 5] = [
            (&label, Hit::ModsButton, !mods.is_empty(), 1.0),
            ("Random", Hit::Key(KeyCode::F2), false, 1.0),
            (&speed, Hit::RateOpen, changed, 0.8),
            (page, Hit::BoardPage, self.mob.board_open, 1.3),
            (play, Hit::Key(KeyCode::Enter), true, 1.3),
        ];
        let total: f32 = buttons.iter().map(|b| b.3).sum();
        let unit = (w - 8.0 * (buttons.len() as f32 - 1.0)) / total;
        let mut bx = 0.0;
        for (name, hit, lit, share) in buttons {
            c.button(bx, by, unit * share, bh, name, SMALL, hit, lit);
            bx += unit * share + 8.0;
        }
    }

    /// The speed being changed, and whether it keeps the pitch: the player's, else the game's.
    fn rate_now(&self) -> (f64, bool) {
        match &self.player {
            Some(p) => p.rate(),
            None => (self.cfg.rate, self.cfg.rate_keep_pitch),
        }
    }

    fn set_rate(&mut self, rate: f64, keep_pitch: bool) {
        let rate = ((rate * 100.0).round() / 100.0).clamp(RATE_RANGE.0, RATE_RANGE.1);
        if let Some(p) = &mut self.player {
            p.set_custom_rate(rate, keep_pitch);
        } else {
            self.cfg.rate = rate;
            self.cfg.rate_keep_pitch = keep_pitch;
            self.cfg.save();
        }
    }

    /// A press in the speed panel.
    pub(crate) fn mobile_rate(&mut self, hit: Hit) {
        let (rate, keep) = self.rate_now();
        match hit {
            Hit::RateOpen => self.mob.rate_open = true,
            Hit::RateClose => self.mob.rate_open = false,
            Hit::RateStep(by) => self.set_rate(rate + by as f64 / 100.0, keep),
            Hit::RateSet(to) => self.set_rate(to as f64 / 100.0, keep),
            Hit::RateKeepPitch => self.set_rate(rate, !keep),
            Hit::PitchStep(by) => {
                if let Some(p) = &mut self.player {
                    p.move_pitch(by);
                }
            }
            _ => {}
        }
    }

    /// The speed panel, over song select or the player: steps, presets and the pitch.
    pub(crate) fn draw_mobile_rate(&mut self, c: &mut Canvas) {
        let shown =
            matches!(self.scene, Scene::Select) && !(self.on_menu() && self.player.is_none());
        if !self.mob.rate_open || !shown || self.mod_select.is_some() {
            self.mob.rate_open = false;
            return;
        }
        self.mob.regions.clear();
        let upright = c.width < VIRTUAL_HEIGHT;
        // Upright a unit is twice as big.
        let (text, big, row) = if upright {
            (5.0, 7.0, 34.0)
        } else {
            (SMALL, MEDIUM, 48.0)
        };
        let (rate, keep) = self.rate_now();
        let semitones = self.player.as_ref().map(|p| p.semitones);
        let w = c.width;
        c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x000000, 0.6));
        c.hit(0.0, 0.0, w, VIRTUAL_HEIGHT, Hit::RateClose);
        let pw = w.min(460.0);
        let gap = row * 0.2;
        let rows = if semitones.is_some() { 5.0 } else { 4.0 };
        let ph = rows * (row + gap) + gap + 8.0;
        let (px, py) = ((w - pw) / 2.0, (VIRTUAL_HEIGHT - ph) / 2.0);
        c.round_rect(
            [px, py],
            [pw, ph],
            10.0,
            rgba(0x15151d, 0.99),
            rgba(accent(), 0.9),
            1.5,
        );
        c.hit(px, py, pw, ph, Hit::Overlay);
        let (ix, iw) = (px + gap + 2.0, pw - 2.0 * gap - 4.0);
        let mut y = py + gap + 4.0;
        let ty = |y: f32, size: f32| y + (row - size) / 2.0;
        c.text(ix, ty(y, big), big, rgba(0xffffff, 1.0), "Speed");
        let done_w = iw * 0.28;
        c.button(
            ix + iw - done_w,
            y,
            done_w,
            row,
            "Done",
            text,
            Hit::RateClose,
            true,
        );
        y += row + gap;
        // The speed itself between its steps.
        let step_w = iw * 0.24;
        c.button(
            ix,
            y,
            step_w,
            row,
            "-",
            big,
            Hit::RateStep(-5),
            rate > RATE_RANGE.0,
        );
        let value = format!("{rate:.2}x");
        c.text_centered(ix + iw / 2.0, ty(y, big), big, rgba(0xffffff, 1.0), &value);
        let plus = Hit::RateStep(5);
        c.button(
            ix + iw - step_w,
            y,
            step_w,
            row,
            "+",
            big,
            plus,
            rate < RATE_RANGE.1,
        );
        y += row + gap;
        let presets = [75, 100, 125, 150, 200];
        let chip = (iw - gap * 4.0) / 5.0;
        for (k, to) in presets.into_iter().enumerate() {
            let label = format!("{:.2}", to as f64 / 100.0);
            let on = (rate - to as f64 / 100.0).abs() < 1e-6;
            let x = ix + k as f32 * (chip + gap);
            c.button(x, y, chip, row, &label, text, Hit::RateSet(to), on);
        }
        y += row + gap;
        c.text(ix, ty(y, text), text, rgba(0xc8c8d8, 1.0), "Keep pitch");
        let (sw, sh) = (row * 1.3, row * 0.66);
        let (sx, sy) = (ix + iw - sw, y + (row - sh) / 2.0);
        let track = if keep { accent() } else { 0x3a3a4e };
        c.round_rect(
            [sx, sy],
            [sw, sh],
            sh / 2.0,
            rgba(track, 1.0),
            rgba(0, 0.0),
            0.0,
        );
        let knob = if keep {
            sx + sw - sh / 2.0
        } else {
            sx + sh / 2.0
        };
        let white = rgba(0xffffff, 1.0);
        c.circle(knob, sy + sh / 2.0, sh / 2.0 - 3.0, white, white, 0.0);
        c.hit(ix, y, iw, row, Hit::RateKeepPitch);
        y += row + gap;
        if let Some(semitones) = semitones {
            c.text(ix, ty(y, text), text, rgba(0xc8c8d8, 1.0), "Pitch");
            let bw = iw * 0.2;
            let right = ix + iw;
            c.button(
                right - 2.0 * bw - iw * 0.2,
                y,
                bw,
                row,
                "-",
                big,
                Hit::PitchStep(-1),
                true,
            );
            let mid = right - bw - iw * 0.1;
            let label = format!("{semitones:+}");
            c.text_centered(mid, ty(y, text), text, rgba(0xffffff, 1.0), &label);
            c.button(right - bw, y, bw, row, "+", big, Hit::PitchStep(1), true);
        }
    }

    /// Song select's right side, two pages side by side: the preview, and the leaderboard the
    /// button slides to. `[x, top, w, bottom]`.
    pub(crate) fn draw_mobile_panels(
        &mut self,
        c: &mut Canvas,
        [x, top, w, bottom]: [f32; 4],
        stars: Option<f32>,
        status: Option<(&str, u32)>,
        liked: Option<bool>,
    ) {
        let t = self.mob.slide_board(c.dt_ms);
        let step = w + 24.0;
        let h = bottom - top;
        self.leaderboard.clear();
        self.online_leaderboard.clear();
        if t < 1.0 {
            let start = c.shapes.len();
            if let Some(clip) = self.preview.draw_panel(
                c,
                &self.library,
                x - t * step,
                top,
                w,
                h,
                self.cfg.scroll_speed,
                stars,
                Some(("Play from", "Shift+Enter")),
                status,
                liked,
            ) {
                self.frame_clip = Some(clip);
            }
            c.clip_since(start, x - 2.0, top - 2.0, w + 4.0, h + 4.0);
        } else {
            self.preview.hide_panel();
        }
        if t > 0.0 {
            let start = c.shapes.len();
            self.draw_mobile_board(c, x + (1.0 - t) * step, top, w, h);
            c.clip_since(start, x - 2.0, top - 2.0, w + 4.0, h + 4.0);
        }
    }

    /// The selected map's leaderboard, at full size: local or online, a row for each play.
    fn draw_mobile_board(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
        const TAB_H: f32 = 42.0;
        const ROW_H: f32 = 60.0;
        panel(c, x, y, w, h);
        let online = self.board_online;
        let tab_w = 104.0;
        let (ty, inner) = (y + 8.0, w - 16.0);
        let local = Hit::BoardTab(false);
        c.button(x + 8.0, ty, tab_w, TAB_H, "Local", SMALL, local, !online);
        let net = Hit::BoardTab(true);
        c.button(
            x + 16.0 + tab_w,
            ty,
            tab_w,
            TAB_H,
            "Online",
            SMALL,
            net,
            online,
        );
        let site_w = 112.0;
        let site = Hit::BoardWebsite;
        c.button(
            x + w - 8.0 - site_w,
            ty,
            site_w,
            TAB_H,
            "Website",
            SMALL,
            site,
            false,
        );
        let list_y = ty + TAB_H + 8.0;
        let list_h = y + h - 8.0 - list_y;
        let relax = self.cfg.mods.relax();
        let Some(hash) = self.select.selected().map(|i| self.library[i].hash()) else {
            return;
        };
        if self.mob.board_of != Some((hash, online)) {
            self.mob.board_of = Some((hash, online));
            self.mob.board = Scroll::default();
        }
        // Each play: rank and player, the score, then how it went.
        struct Line {
            name: String,
            score: u64,
            how: String,
            when: String,
            mine: bool,
        }
        let extras = |rate: f64, mods: String, pp: Option<f64>| {
            let mut out = String::new();
            if let Some(pp) = pp {
                out += &format!("  {pp:.0}pp");
            }
            if (rate - 1.0).abs() > 1e-6 {
                out += &format!("  {rate:.2}x");
            }
            if !mods.is_empty() {
                out += &format!("  +{mods}");
            }
            out
        };
        let mut note = None;
        let lines: Vec<Line> = if online {
            let me = self.cfg.online_user.clone();
            let board = self.online_board(hash, relax).cloned();
            let rows = match board {
                None => {
                    note = Some("loading...".to_string());
                    Vec::new()
                }
                Some(Err(e)) => {
                    note = Some(format!("can't reach the server: {e}"));
                    Vec::new()
                }
                Some(Ok(rows)) => rows,
            };
            if rows.is_empty() && note.is_none() {
                note = Some("No online scores yet.".to_string());
            }
            let lines = rows
                .iter()
                .map(|r| Line {
                    name: r.player.clone(),
                    score: r.score,
                    how: format!(
                        "{:.2}%  {}x{}",
                        r.accuracy * 100.0,
                        r.max_combo,
                        extras(r.rate, r.mods.concat(), r.pp)
                    ),
                    when: crate::misses_and_age(r.miss, r.date),
                    mine: r.player.eq_ignore_ascii_case(&me),
                })
                .collect();
            self.online_leaderboard = rows;
            lines
        } else {
            let rows = crate::scores::top(&self.scores, hash, usize::MAX, relax);
            if rows.is_empty() {
                note = Some("No plays yet: finish one to set a score.".to_string());
            }
            self.leaderboard = rows.iter().map(|r| r.file.clone()).collect();
            rows.iter()
                .map(|r| Line {
                    name: r.player.clone(),
                    score: r.score,
                    how: format!(
                        "{:.2}%  {}x{}",
                        r.accuracy * 100.0,
                        r.max_combo,
                        extras(r.rate, r.mods.clone(), r.pp)
                    ),
                    when: crate::misses_and_age(r.miss, r.date),
                    mine: false,
                })
                .collect()
        };
        if let Some(note) = note {
            for (k, line) in wrap(SMALL, inner - 16.0, &note, 3).iter().enumerate() {
                let ly = list_y + 30.0 + k as f32 * 20.0;
                c.text_centered(x + w / 2.0, ly, SMALL, rgba(0x9a9ab0, 1.0), line);
            }
            return;
        }
        let dt = c.dt_ms;
        let height = lines.len() as f32 * ROW_H - GAP;
        self.mob.board.frame(dt, height - list_h);
        self.mob.regions.push(([x, list_y, w, list_h], List::Board));
        let scroll = self.mob.board.pos;
        let start = c.shapes.len();
        let rx = x + 8.0;
        for (k, line) in lines.iter().enumerate() {
            let ry = list_y + k as f32 * ROW_H - scroll;
            if ry > list_y + list_h || ry + ROW_H < list_y {
                continue;
            }
            c.round_rect(
                [rx, ry],
                [inner, ROW_H - GAP],
                7.0,
                rgba(if line.mine { 0x2b2b3d } else { 0x1d1d27 }, 0.95),
                rgba(if line.mine { accent() } else { 0x2a2a38 }, 1.0),
                1.0,
            );
            let score = format!("{:08}", line.score);
            let score_w = Canvas::text_width(SMALL, &score);
            let first = if k == 0 { 0xFFD34D } else { 0xffffff };
            let place = format!("#{}  ", k + 1);
            let colour = rgba(first, 1.0);
            let pw = c.text(rx + 10.0, ry + 8.0, SMALL, colour, &place);
            let room = inner - score_w - 36.0 - pw;
            crate::social::styled_name(
                c,
                rx + 10.0 + pw,
                ry + 8.0,
                SMALL,
                &line.name,
                colour,
                room,
            );
            c.text_right(
                rx + inner - 10.0,
                ry + 8.0,
                SMALL,
                rgba(0xffffff, 1.0),
                &score,
            );
            let when_w = Canvas::text_width(SMALL, &line.when);
            let how = Canvas::fit(SMALL, inner - when_w - 36.0, &line.how);
            c.text(rx + 10.0, ry + 30.0, SMALL, rgba(0xc8c8d8, 1.0), &how);
            let when_y = ry + 30.0;
            c.text_right(
                rx + inner - 10.0,
                when_y,
                SMALL,
                rgba(0x8a8aa0, 1.0),
                &line.when,
            );
            let hit_top = ry.max(list_y);
            let hit_bottom = (ry + ROW_H - GAP).min(list_y + list_h);
            if hit_bottom > hit_top {
                let hit = if online {
                    Hit::OnlineScoreRow(k)
                } else {
                    Hit::ScoreRow(k)
                };
                c.hit(rx, hit_top, inner, hit_bottom - hit_top, hit);
            }
        }
        c.clip_since(start, x, list_y, w, list_h);
    }

    /// The song list: a row for each song, the selected one open on its difficulties.
    fn draw_mobile_songs(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
        let rate = self.play_speed().0;
        let groups = self.select.song_groups().to_vec();
        let rows = self.select.rows().to_vec();
        let cursor = self.select.cursor();
        let open = groups.iter().position(|g| g.contains(&cursor));
        let open_h = open.map_or(0.0, |g| groups[g].len() as f32 * DIFF_H);
        let song_top = |g: usize| {
            g as f32 * SONG_H
                + if open.is_some_and(|o| g > o) {
                    open_h
                } else {
                    0.0
                }
        };
        let height = groups.len() as f32 * SONG_H + open_h;
        // A selection made elsewhere (Random, coming back) comes into view.
        if self.mob.song_cursor != Some(cursor) {
            self.mob.song_cursor = Some(cursor);
            if let Some(g) = open {
                let bottom = song_top(g) + SONG_H + open_h;
                self.mob.songs.show(song_top(g), bottom, h);
            }
        }
        let dt = c.dt_ms;
        self.mob.songs.frame(dt, height - h);
        self.mob.regions.push(([x, y, w, h], List::Songs));
        let scroll = self.mob.songs.pos;
        if groups.is_empty() {
            let text = if self.select.filter.is_empty() {
                "No maps yet. Get some in Download, or open an .osz with taiko.rs."
            } else {
                "No maps match the search."
            };
            for (k, line) in wrap(SMALL, w - 40.0, text, 4).iter().enumerate() {
                let ly = y + 60.0 + k as f32 * 20.0;
                c.text_centered(x + w / 2.0, ly, SMALL, rgba(0xc8c8d8, 1.0), line);
            }
            return;
        }
        let start = c.shapes.len();
        // The first song on screen: rows are the same height down to the open one.
        let first = match open {
            Some(o) if scroll > song_top(o) + SONG_H + open_h => {
                o + 1 + ((scroll - song_top(o + 1).min(scroll)) / SONG_H) as usize
            }
            Some(o) => ((scroll / SONG_H) as usize).min(o),
            None => (scroll / SONG_H) as usize,
        };
        let colours = crate::multiverse::note_colours();
        for (g, range) in groups.iter().enumerate().skip(first) {
            let sy = y + song_top(g) - scroll;
            if sy > y + h {
                break;
            }
            let is_open = open == Some(g);
            let lead = &self.library[rows[range.start]];
            if is_open {
                c.round_rect(
                    [x, sy],
                    [w, SONG_H - 2.0],
                    6.0,
                    rgba(accent(), 0.14),
                    rgba(0, 0.0),
                    0.0,
                );
            } else {
                c.rect(
                    x + 8.0,
                    sy + SONG_H - 1.0,
                    w - 16.0,
                    1.0,
                    rgba(0x2a2a38, 1.0),
                );
            }
            crate::style::note(
                c,
                x + 24.0,
                sy + SONG_H / 2.0,
                if is_open { 15.0 } else { 13.0 },
                colours.don(),
                false,
                if is_open { 1.0 } else { 0.7 },
            );
            let tx = x + 48.0;
            let room = w - 60.0;
            let count = range.len();
            let tag = if count == 1 {
                "1 diff".to_string()
            } else {
                format!("{count} diffs")
            };
            let tag_w = Canvas::text_width(SMALL, &tag) + 14.0;
            let title = Canvas::fit(MEDIUM, room - tag_w, &lead.meta.title);
            c.text(tx, sy + 7.0, MEDIUM, rgba(0xffffff, 1.0), &title);
            let by = format!("{}  |  mapped by {}", lead.meta.artist, lead.meta.creator);
            let by = Canvas::fit(SMALL, room, &by);
            c.text(tx, sy + 34.0, SMALL, rgba(0x9a9ab0, 1.0), &by);
            let tag_y = sy + 10.0;
            c.text_right(x + w - 10.0, tag_y, SMALL, rgba(0x9a9ab0, 1.0), &tag);
            let hit_y = sy.max(y);
            let hit_h = (sy + SONG_H).min(y + h) - hit_y;
            if hit_h > 0.0 {
                c.hit(x, hit_y, w, hit_h, Hit::SongRow(range.start));
            }
            if !is_open {
                continue;
            }
            for (k, row) in range.clone().enumerate() {
                let dy = sy + SONG_H + k as f32 * DIFF_H;
                if dy > y + h || dy + DIFF_H < y {
                    continue;
                }
                let i = rows[row];
                let on = row == cursor;
                let (dx, dw) = (x + 26.0, w - 30.0);
                if on {
                    c.round_rect(
                        [dx, dy + 2.0],
                        [dw, DIFF_H - 4.0],
                        4.0,
                        rgba(crate::style::selector_colour(), 0.14),
                        rgba(0, 0.0),
                        0.0,
                    );
                    let frame = [dx, dy + 2.0, dw, DIFF_H - 4.0];
                    crate::style::selector(c, "mobile-difficulty", frame);
                }
                let found = self.stars.get(i, rate);
                let py = dy + (DIFF_H - SMALL - 6.0) / 2.0;
                let pill = stars::draw_pill(c, dx + dw - 10.0, py, SMALL, found);
                let name = &self.library[i].meta.version;
                let name = Canvas::fit(MEDIUM, dw - pill - 34.0, name);
                let ny = dy + (DIFF_H - MEDIUM) / 2.0;
                let col = if on { 0xffffff } else { 0xc8c8d8 };
                c.text(dx + 12.0, ny, MEDIUM, rgba(col, 1.0), &name);
                let top = dy.max(y);
                let bottom = (dy + DIFF_H).min(y + h);
                if bottom > top {
                    c.hit(dx, top, dw, bottom - top, Hit::SongRow(row));
                }
            }
        }
        c.clip_since(start, x - 2.0, y, w + 4.0, h);
    }

    /// Settings: the sections down the side and the rows of the one open, or the search's.
    pub(crate) fn draw_mobile_settings(&mut self, c: &mut Canvas) {
        let w = c.width;
        let x = back_button(c, "< BACK", Hit::Key(KeyCode::Escape));
        crate::style::title(c, x + 6.0, 12.0, MEDIUM, 0xffffff, "SETTINGS");
        let box_w = (w * 0.34).min(340.0);
        let search = self.settings_ui.search.clone();
        search_box(
            c,
            w - box_w,
            box_w,
            &search,
            "Search settings",
            !search.is_empty(),
            Hit::SettingsSearch,
        );
        let dt = c.dt_ms;
        let section = self.settings_ui.section();
        // The sections.
        const SIDE_W: f32 = 212.0;
        const SECTION_H: f32 = 56.0;
        let side_h = VIRTUAL_HEIGHT - TOP;
        let max = SECTIONS.len() as f32 * SECTION_H - GAP - side_h;
        self.mob.sections.frame(dt, max);
        self.mob
            .regions
            .push(([0.0, TOP, SIDE_W, side_h], List::Sections));
        let scroll = self.mob.sections.pos;
        let start = c.shapes.len();
        let searching = !search.is_empty();
        for i in 0..SECTIONS.len() {
            let y = TOP + i as f32 * SECTION_H - scroll;
            if y > VIRTUAL_HEIGHT || y + SECTION_H < TOP {
                continue;
            }
            let on = i == section && !searching;
            c.round_rect(
                [0.0, y],
                [SIDE_W, SECTION_H - GAP],
                8.0,
                rgba(if on { 0x34344a } else { 0x15151d }, 0.92),
                rgba(if on { accent() } else { 0x2a2a38 }, 1.0),
                1.0,
            );
            let name = Canvas::fit(SMALL, SIDE_W - 28.0, settings::section_name(i));
            let col = if on { 0xffffff } else { 0xc8c8d8 };
            let ty = y + (SECTION_H - GAP - SMALL) / 2.0;
            c.text(14.0, ty, SMALL, rgba(col, 1.0), &name);
            let top = y.max(TOP);
            let bottom = (y + SECTION_H - GAP).min(VIRTUAL_HEIGHT);
            if bottom > top {
                c.hit(0.0, top, SIDE_W, bottom - top, Hit::SettingsSection(i));
            }
        }
        c.clip_since(start, 0.0, TOP, SIDE_W, side_h);

        // The rows, and what the selected one does under them.
        const ROW_H: f32 = 62.0;
        const HEAD_H: f32 = 34.0;
        const HELP_H: f32 = 52.0;
        let lx = SIDE_W + 10.0;
        let lw = w - lx;
        let list_h = VIRTUAL_HEIGHT - TOP - HELP_H - 6.0;
        let lines = self.settings_ui.listed(section);
        let mut height = 0.0;
        let mut at = Vec::with_capacity(lines.len());
        for line in &lines {
            at.push(height);
            height += match line {
                settings::Listed::Heading(_) => HEAD_H,
                settings::Listed::Row(_) => ROW_H,
            };
        }
        let cursor = self.settings_ui.cursor;
        if self.mob.setting_cursor != Some(section) {
            self.mob.setting_cursor = Some(section);
            self.mob.settings.pos = 0.0;
        }
        self.mob.settings.frame(dt, height - GAP - list_h);
        self.mob
            .regions
            .push(([lx, TOP, lw, list_h], List::Settings));
        let scroll = self.mob.settings.pos;
        let start = c.shapes.len();
        // The bars' common track.
        let track_w = (lw * 0.36).min(260.0);
        let track_x = lx + lw - track_w - 74.0;
        self.settings_ui.set_track(track_x, track_w);
        for (line, top) in lines.iter().zip(at) {
            let y = TOP + top - scroll;
            if y > TOP + list_h || y + ROW_H < TOP {
                continue;
            }
            let i = match *line {
                settings::Listed::Heading(name) => {
                    c.text(lx + 6.0, y + 12.0, SMALL, rgba(accent(), 1.0), name);
                    continue;
                }
                settings::Listed::Row(i) => i,
            };
            self.draw_mobile_setting(c, i, [lx, y, lw, ROW_H - GAP], TOP, TOP + list_h);
        }
        c.clip_since(start, lx - 2.0, TOP, lw + 4.0, list_h);
        panel(c, lx, VIRTUAL_HEIGHT - HELP_H, lw, HELP_H);
        let help = for_touch(ITEMS[cursor].1.help());
        for (k, line) in wrap(SMALL, lw - 28.0, &help, 2).iter().enumerate() {
            let y = VIRTUAL_HEIGHT - HELP_H + 8.0 + k as f32 * 19.0;
            c.text(lx + 14.0, y, SMALL, rgba(0xa8a8bc, 1.0), line);
        }
    }

    /// A row of settings: what it is on the left, how it's changed on the right.
    fn draw_mobile_setting(&self, c: &mut Canvas, i: usize, r: [f32; 4], top: f32, bottom: f32) {
        let [x, y, w, h] = r;
        let item = ITEMS[i].1;
        let on = i == self.settings_ui.cursor;
        c.round_rect(
            [x, y],
            [w, h],
            8.0,
            rgba(if on { 0x2b2b3d } else { 0x1d1d27 }, 0.92),
            rgba(if on { accent() } else { 0x2a2a38 }, 1.0),
            1.0,
        );
        let hit_top = y.max(top);
        let hit_bottom = (y + h).min(bottom);
        if hit_bottom <= hit_top {
            return;
        }
        c.hit(x, hit_top, w, hit_bottom - hit_top, Hit::SettingRow(i));
        let mid = y + (h - SMALL) / 2.0;
        let control = self.setting_control(item);
        let kind = item.kind();
        let editing = self
            .settings_ui
            .editing
            .as_ref()
            .filter(|e| e.item == item)
            .map(|e| format!("{}_", e.text));
        let value = editing.unwrap_or_else(|| for_touch(&self.setting_value(item)));
        let right = x + w - 14.0;
        let mut label_room = w - 28.0;
        match control {
            Control::Switch(state) => {
                let (sw, sh) = (62.0, 32.0);
                let (sx, sy) = (right - sw, y + (h - sh) / 2.0);
                let track = if state { accent() } else { 0x3a3a4e };
                c.round_rect(
                    [sx, sy],
                    [sw, sh],
                    sh / 2.0,
                    rgba(track, 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
                let knob = if state {
                    sx + sw - sh / 2.0
                } else {
                    sx + sh / 2.0
                };
                let white = rgba(0xffffff, 1.0);
                c.circle(knob, sy + sh / 2.0, sh / 2.0 - 4.0, white, white, 0.0);
                label_room -= sw + 12.0;
            }
            Control::Level(level) => {
                let (track_left, track_w) = self.settings_ui.track_rect();
                let ty = y + h / 2.0;
                c.round_rect(
                    [track_left, ty - 4.0],
                    [track_w, 8.0],
                    4.0,
                    rgba(0x3a3a4e, 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
                let filled = track_w * level.clamp(0.0, 1.0);
                c.round_rect(
                    [track_left, ty - 4.0],
                    [filled, 8.0],
                    4.0,
                    rgba(accent(), 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
                let white = rgba(0xffffff, 1.0);
                c.circle(track_left + filled, ty, 13.0, white, white, 0.0);
                c.text_right(right, mid, SMALL, rgba(0xffffff, 1.0), &value);
                // Room around the track for a thumb.
                let hit_h = hit_bottom - hit_top;
                c.hit(
                    track_left - 18.0,
                    hit_top,
                    track_w + 36.0,
                    hit_h,
                    Hit::SettingLevel(i),
                );
                label_room = track_left - 24.0 - x - 14.0;
            }
            _ => {
                let step = matches!(kind, Kind::Value | Kind::List);
                let steps_w = if step { 2.0 * 50.0 + 6.0 } else { 0.0 };
                let swatch = if let Control::Swatch(col) = control {
                    let sx = right - steps_w - 34.0;
                    c.round_rect(
                        [sx, y + (h - 24.0) / 2.0],
                        [24.0, 24.0],
                        5.0,
                        rgba(col, 1.0),
                        rgba(0xffffff, 0.4),
                        1.0,
                    );
                    34.0
                } else {
                    0.0
                };
                let vr = right - steps_w - swatch - if step { 10.0 } else { 0.0 };
                let value_room = (w * 0.42 - steps_w).max(90.0);
                let shown = Canvas::fit(SMALL, value_room, &value);
                let shown_w = Canvas::text_width(SMALL, &shown);
                c.text_right(vr, mid, SMALL, rgba(0xffffff, 1.0), &shown);
                label_room = vr - shown_w - 16.0 - (x + 14.0);
                if step {
                    let by = y + (h - 44.0) / 2.0;
                    let lx = right - steps_w;
                    c.button(
                        lx,
                        by,
                        50.0,
                        44.0,
                        "<",
                        MEDIUM,
                        Hit::SettingStep(i, -1),
                        false,
                    );
                    let rx = right - 50.0;
                    c.button(
                        rx,
                        by,
                        50.0,
                        44.0,
                        ">",
                        MEDIUM,
                        Hit::SettingStep(i, 1),
                        false,
                    );
                }
            }
        }
        let label = Canvas::fit(SMALL, label_room.max(40.0), item.label());
        c.text(x + 14.0, mid, SMALL, rgba(0xffffff, 1.0), &label);
    }

    /// A setting's list of choices, over the settings.
    pub(crate) fn draw_mobile_choices(&mut self, c: &mut Canvas) {
        let Some(p) = &self.settings_ui.choices else {
            self.mob.choices_open = false;
            return;
        };
        let (w, dt) = (c.width, c.dt_ms);
        c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x000000, 0.7));
        c.hit(0.0, 0.0, w, VIRTUAL_HEIGHT, Hit::Key(KeyCode::Escape));
        const ROW_H: f32 = 58.0;
        let pw = (w * 0.62).min(640.0);
        let px = (w - pw) / 2.0;
        panel(c, px, 0.0, pw, VIRTUAL_HEIGHT);
        c.hit(px, 0.0, pw, VIRTUAL_HEIGHT, Hit::Overlay);
        let title = Canvas::fit(MEDIUM, pw - 150.0, p.item.label());
        c.text(px + 16.0, 16.0, MEDIUM, rgba(0xffffff, 1.0), &title);
        let close = Hit::Key(KeyCode::Escape);
        c.button(
            px + pw - 104.0,
            6.0,
            96.0,
            44.0,
            "Close",
            SMALL,
            close,
            false,
        );
        let (top, h) = (58.0, VIRTUAL_HEIGHT - 58.0 - 8.0);
        let (names, current, cursor) = (p.names.clone(), p.current, p.cursor);
        if !self.mob.choices_open {
            self.mob.choices_open = true;
            self.mob.choices = Scroll::default();
            let at = cursor as f32 * ROW_H;
            self.mob.choices.pos = (at - h / 2.0).max(0.0);
        }
        self.mob
            .choices
            .frame(dt, names.len() as f32 * ROW_H - GAP - h);
        self.mob.regions.push(([px, top, pw, h], List::Choices));
        let scroll = self.mob.choices.pos;
        let start = c.shapes.len();
        for (i, name) in names.iter().enumerate() {
            let y = top + i as f32 * ROW_H - scroll;
            if y > top + h || y + ROW_H < top {
                continue;
            }
            let on = current == Some(i);
            let (x, rw) = (px + 10.0, pw - 20.0);
            c.round_rect(
                [x, y],
                [rw, ROW_H - GAP],
                8.0,
                rgba(if on { 0x34344a } else { 0x1d1d27 }, 0.95),
                rgba(if on { accent() } else { 0x2a2a38 }, 1.0),
                1.0,
            );
            let name = Canvas::fit(SMALL, rw - 28.0, name);
            let ty = y + (ROW_H - GAP - SMALL) / 2.0;
            c.text(x + 14.0, ty, SMALL, rgba(0xffffff, 1.0), &name);
            let hit_top = y.max(top);
            let hit_bottom = (y + ROW_H - GAP).min(top + h);
            if hit_bottom > hit_top {
                c.hit(x, hit_top, rw, hit_bottom - hit_top, Hit::ChoiceRow(i));
            }
        }
        c.clip_since(start, px, top, pw, h);
    }

    /// The mods: a tile for each, under its group's heading, over whatever screen is open.
    pub(crate) fn draw_mobile_mods(&mut self, c: &mut Canvas, picked: mods::Mods) {
        use mods::{ALL, GROUPS};
        let (w, dt) = (c.width, c.dt_ms);
        c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x0b0b10, 0.97));
        c.hit(0.0, 0.0, w, VIRTUAL_HEIGHT, Hit::Overlay);
        let done = Hit::Key(KeyCode::Escape);
        c.button(
            w - 118.0,
            0.0,
            118.0,
            BAR_H - 2.0,
            "Done",
            SMALL,
            done,
            true,
        );
        let clear = Hit::Key(KeyCode::Backspace);
        c.button(
            w - 226.0,
            0.0,
            100.0,
            BAR_H - 2.0,
            "Clear",
            SMALL,
            clear,
            false,
        );
        let mut left = w - 226.0;
        if !mods::customizable(picked).is_empty() {
            left -= 148.0;
            let custom = Hit::Key(KeyCode::KeyC);
            c.button(
                left,
                0.0,
                140.0,
                BAR_H - 2.0,
                "Customize",
                SMALL,
                custom,
                false,
            );
        }
        crate::style::title(c, 6.0, 12.0, MEDIUM, 0xffffff, "MODS");
        let unsaved = if picked.unranked() {
            "  (not saved)"
        } else {
            ""
        };
        let score = format!("score x{:.2}{unsaved}", picked.multiplier());
        let room = left - 112.0 - 12.0;
        let score = Canvas::fit(SMALL, room, &score);
        c.text(112.0, 16.0, SMALL, rgba(0xc8c8d8, 1.0), &score);
        // The last mod touched says what it does.
        const HELP_H: f32 = 34.0;
        let list_h = VIRTUAL_HEIGHT - TOP - HELP_H;
        const COLS: usize = 4;
        const TILE_H: f32 = 62.0;
        const HEAD_H: f32 = 28.0;
        let tile_w = (w - GAP * (COLS as f32 - 1.0)) / COLS as f32;
        let block = |n: usize| HEAD_H + n.div_ceil(COLS) as f32 * TILE_H + 4.0;
        let height: f32 = GROUPS.iter().map(|g| block(g.2.len())).sum();
        self.mob.mods.frame(dt, height - list_h);
        self.mob.regions.push(([0.0, TOP, w, list_h], List::Mods));
        let scroll = self.mob.mods.pos;
        let start = c.shapes.len();
        let mut gy = TOP - scroll;
        for (title, colour, list) in GROUPS.iter() {
            c.text(4.0, gy + 6.0, SMALL, rgba(*colour, 1.0), title);
            for (k, &m) in list.iter().enumerate() {
                let x = (k % COLS) as f32 * (tile_w + GAP);
                let y = gy + HEAD_H + (k / COLS) as f32 * TILE_H;
                if y > TOP + list_h || y + TILE_H < TOP {
                    continue;
                }
                let index = ALL.iter().position(|&a| a == m).unwrap_or(0);
                let on = picked.has(m);
                let blocked = picked.incompatible(m);
                let tint = if blocked { 0x555560 } else { *colour };
                c.round_rect(
                    [x, y],
                    [tile_w, TILE_H - GAP],
                    8.0,
                    rgba(if on { 0x2b2b3d } else { 0x1d1d27 }, 0.95),
                    rgba(if on { tint } else { 0x2a2a38 }, 1.0),
                    if on { 2.0 } else { 1.0 },
                );
                let badge_w = 54.0;
                c.round_rect(
                    [x + 8.0, y + 10.0],
                    [badge_w, TILE_H - GAP - 20.0],
                    6.0,
                    rgba(tint, if on { 1.0 } else { 0.0 }),
                    rgba(tint, 1.0),
                    1.5,
                );
                let badge = if on { 0x15151d } else { tint };
                let ty = y + (TILE_H - GAP - SMALL) / 2.0;
                let short = Canvas::fit(SMALL, badge_w - 6.0, m.acronym());
                c.text_centered(x + 8.0 + badge_w / 2.0, ty, SMALL, rgba(badge, 1.0), &short);
                let name = Canvas::fit(SMALL, tile_w - badge_w - 28.0, m.name());
                let col = if blocked { 0x6f6f86 } else { 0xffffff };
                c.text(x + badge_w + 18.0, ty, SMALL, rgba(col, 1.0), &name);
                let top = y.max(TOP);
                let bottom = (y + TILE_H - GAP).min(TOP + list_h);
                if bottom > top {
                    c.hit(x, top, tile_w, bottom - top, Hit::ModToggle(index));
                }
            }
            gy += block(list.len());
        }
        c.clip_since(start, 0.0, TOP, w, list_h);
        let about = self
            .mod_select
            .and_then(|i| ALL.get(i))
            .map_or("", |m| m.description());
        let line = Canvas::fit(SMALL, w - 12.0, about);
        let y = VIRTUAL_HEIGHT - HELP_H + 12.0;
        c.text(4.0, y, SMALL, rgba(0xa8a8bc, 1.0), &line);
    }

    /// Download: searching the mirrors, a row for each set found.
    pub(crate) fn draw_mobile_download(
        &mut self,
        c: &mut Canvas,
        installed: &std::collections::HashSet<u32>,
        custom: &std::collections::HashSet<u32>,
    ) {
        use crate::browse::Source;
        let (w, dt) = (c.width - self.tab_room(), c.dt_ms);
        let x = back_button(c, "< BACK", Hit::Key(KeyCode::Escape));
        let (source_w, status_w, sort_w) = (120.0, 192.0, 180.0);
        let search_w = w - x - source_w - status_w - sort_w - 24.0;
        let query = self.browse.query.clone();
        search_box(c, x, search_w, &query, "Search maps", false, Hit::TextField);
        // osu! or taiko.rs: a tap switches.
        let source = self.browse.source();
        let other = match source {
            Source::Osu => Source::Ours,
            Source::Ours => Source::Osu,
        };
        let other_at = Source::ALL.iter().position(|&s| s == other).unwrap_or(0);
        c.button(
            x + search_w + 8.0,
            0.0,
            source_w,
            BAR_H - 2.0,
            source.name(),
            SMALL,
            Hit::DlSource(other_at),
            true,
        );
        // Status and order step through the source's own (the hits index `categories` and
        // `sorts`).
        let cats = self.browse.categories();
        let at = cats
            .iter()
            .position(|&k| k == self.browse.category)
            .unwrap_or(0);
        let next = (at + 1) % cats.len();
        let sx = x + search_w + 8.0 + source_w + 8.0;
        let label = cats[at].name();
        c.button(
            sx,
            0.0,
            status_w,
            BAR_H - 2.0,
            label,
            SMALL,
            Hit::DlCategory(next),
            false,
        );
        let sorts = self.browse.sorts();
        let at = sorts
            .iter()
            .position(|&k| k == self.browse.sort)
            .unwrap_or(0);
        let next = (at + 1) % sorts.len();
        // The website's lists name some orders their own way (sent, not ranked).
        let sort = if self.browse.category.is_custom() {
            sorts[at].name_for_ours()
        } else {
            sorts[at].name()
        };
        let label = format!("Sort: {sort}");
        let hit = Hit::DlSort(next);
        c.button(
            sx + status_w + 8.0,
            0.0,
            sort_w,
            BAR_H - 2.0,
            &label,
            SMALL,
            hit,
            false,
        );

        const ROW_H: f32 = 78.0;
        let h = VIRTUAL_HEIGHT - TOP;
        let count = self.browse.results.len();
        self.mob.download.frame(dt, count as f32 * ROW_H - GAP - h);
        self.mob.regions.push(([0.0, TOP, w, h], List::Download));
        let scroll = self.mob.download.pos;
        if scroll + h + 3.0 * ROW_H > count as f32 * ROW_H {
            self.browse.load_more();
        }
        if count == 0 {
            let status = self.browse.status.clone();
            let text = if status.is_empty() {
                "Search for a song, artist or mapper.".to_string()
            } else {
                status
            };
            for (k, line) in wrap(SMALL, w * 0.7, &text, 3).iter().enumerate() {
                let y = TOP + 80.0 + k as f32 * 20.0;
                c.text_centered(w / 2.0, y, SMALL, rgba(0xa8a8bc, 1.0), line);
            }
        }
        let start = c.shapes.len();
        for (i, set) in self.browse.results.iter().enumerate() {
            let y = TOP + i as f32 * ROW_H - scroll;
            if y > TOP + h || y + ROW_H < TOP {
                continue;
            }
            let on = i == self.browse.cursor;
            c.round_rect(
                [0.0, y],
                [w, ROW_H - GAP],
                8.0,
                rgba(if on { 0x2b2b3d } else { 0x1d1d27 }, 0.92),
                rgba(if on { accent() } else { 0x2a2a38 }, 1.0),
                1.0,
            );
            let top = y.max(TOP);
            let bottom = (y + ROW_H - GAP).min(VIRTUAL_HEIGHT);
            if bottom <= top {
                continue;
            }
            c.hit(0.0, top, w, bottom - top, Hit::DlRow(i));
            // The state on the right, under the thumb.
            let (aw, ax) = (132.0, w - 132.0 - 10.0);
            let job = self.downloads.iter().find(|j| j.set.same(set));
            let ty = y + (ROW_H - GAP - SMALL) / 2.0;
            let here = match set.custom {
                Some(_) => custom.contains(&set.id),
                None => installed.contains(&set.id),
            };
            if here {
                c.text_centered(ax + aw / 2.0, ty, SMALL, rgba(0x86E05A, 1.0), "Installed");
            } else if let Some(job) = job {
                let text = job
                    .fraction()
                    .map_or("Starting".to_string(), |f| format!("{:.0}%", f * 100.0));
                c.text_centered(ax + aw / 2.0, ty, SMALL, rgba(0xffffff, 1.0), &text);
            } else {
                c.button(
                    ax,
                    y + 10.0,
                    aw,
                    ROW_H - GAP - 20.0,
                    "Get",
                    SMALL,
                    Hit::DlGet(i),
                    on,
                );
            }
            // Custom sets aren't rated by the website: their status goes there.
            let hi = set
                .diffs
                .last()
                .map(|d| d.stars)
                .filter(|_| set.custom.is_none());
            let pill = match &set.custom {
                Some(cs) => {
                    let (label, colour) = if cs.approved {
                        ("Approved", 0x86E05A)
                    } else {
                        ("Custom", 0xB28CFF)
                    };
                    let lw = Canvas::text_width(SMALL, label);
                    c.text(ax - 12.0 - lw, ty, SMALL, rgba(colour, 1.0), label);
                    lw
                }
                None => stars::draw_pill(c, ax - 12.0, ty, SMALL, hi),
            };
            let room = ax - pill - 40.0;
            let title = Canvas::fit(MEDIUM, room, &set.title);
            c.text(14.0, y + 12.0, MEDIUM, rgba(0xffffff, 1.0), &title);
            let by = format!("{}  |  {}", set.artist, set.creator);
            let by = Canvas::fit(SMALL, room, &by);
            c.text(14.0, y + 44.0, SMALL, rgba(0xa8a8bc, 1.0), &by);
        }
        c.clip_since(start, 0.0, TOP, w, h);
    }
}
