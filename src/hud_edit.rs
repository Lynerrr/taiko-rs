//! The interface editor: drag any part of the interface anywhere, on the screen or attached to
//! the playfield, its lane, the drum or the hit point (see `hud`), and keep layouts as presets.
//! The playfield itself can be moved and resized here too, to see what follows it.

use winit::keyboard::KeyCode;

use crate::config::Config;
use crate::game::Game;
use crate::hud::{self, Frames, Layout, MAX_SCALE, MIN_SCALE, Part, Place, Placed, Snapping};
use crate::layout::{Area, Guide};
use crate::render::{Canvas, Hit, accent, rgba};
use crate::settings;

const PANEL_W: f32 = 190.0;
const ROW_H: f32 = 14.0;
/// Preset rows the panel shows at once.
const PRESET_ROWS: usize = 6;
/// How close (virtual units) a dragged part has to come to catch on a line.
const SNAP: f32 = 5.0;
const UNDO_STEPS: usize = 64;
const NAME_MAX: usize = 24;
const SNAP_COLOUR: u32 = 0x4fc3f7;
/// Between the panel's buttons.
const GAP: f32 = 4.0;

/// A button of the editor's panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    AttachX,
    AttachY,
    Smaller,
    Bigger,
    ResetPart,
    SavePreset,
    NewPreset,
    DeletePreset,
    Copy,
    Paste,
    Undo,
    ResetAll,
    SwapSide,
    Done,
}

/// What a key or button leaves for the game to do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Nothing,
    Toast(String),
    Close,
}

#[derive(Clone, Copy)]
struct PartBox {
    place: Place,
    size: [f32; 2],
    at: Placed,
}

#[derive(Clone, Copy)]
struct Drag {
    index: usize,
    grab: [f32; 2],
}

pub struct Editor {
    selected: usize,
    drag: Option<Drag>,
    pub guides: Vec<Guide>,
    boxes: [Option<PartBox>; Part::PLACED.len()],
    frames: Frames,
    panel_hidden: bool,
    panel_left: bool,
    /// The player's own preset being worked on.
    preset: Option<usize>,
    /// A name being typed for a new preset, and the caret in it.
    naming: Option<(String, usize)>,
    undo: Vec<Layout>,
    preset_scroll: usize,
    panel_rect: [f32; 4],
    presets_rect: [f32; 4],
}

impl Editor {
    pub fn new(cfg: &Config) -> Editor {
        Editor {
            selected: 0,
            drag: None,
            guides: Vec::new(),
            boxes: [None; Part::PLACED.len()],
            frames: Frames::new(1.0, Area::FULL),
            panel_hidden: false,
            // Clear of the score and the rest of the top right, what gets moved first.
            panel_left: true,
            preset: cfg.hud_presets.iter().position(|p| p.1 == cfg.hud),
            naming: None,
            undo: Vec::new(),
            preset_scroll: 0,
            panel_rect: [0.0; 4],
            presets_rect: [0.0; 4],
        }
    }

    pub fn naming(&self) -> bool {
        self.naming.is_some()
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    fn part(&self) -> Part {
        Part::PLACED[self.selected]
    }

    fn remember(&mut self, cfg: &Config) {
        if self.undo.last() != Some(&cfg.hud) {
            self.undo.push(cfg.hud);
            if self.undo.len() > UNDO_STEPS {
                self.undo.remove(0);
            }
        }
    }

    /// The topmost-looking part under a point: the smallest one there.
    fn part_at(&self, x: f32, y: f32, cfg: &Config) -> Option<usize> {
        self.boxes
            .iter()
            .enumerate()
            .filter(|&(i, _)| cfg.hud.shown(Part::PLACED[i]))
            .filter_map(|(i, b)| b.map(|b| (i, b.at)))
            .filter(|(_, b)| x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h)
            .min_by(|a, b| (a.1.w * a.1.h).total_cmp(&(b.1.w * b.1.h)))
            .map(|(i, _)| i)
    }

    /// A press on the screen (not on the panel): picks up the part there, if there is one.
    pub fn press(&mut self, x: f32, y: f32, cfg: &Config) -> bool {
        let Some(index) = self.part_at(x, y, cfg) else {
            return false;
        };
        let Some(b) = self.boxes[index] else {
            return false;
        };
        self.selected = index;
        self.remember(cfg);
        self.drag = Some(Drag {
            index,
            grab: [x - b.at.x, y - b.at.y],
        });
        true
    }

    /// The pointer moved with a part held; `free`: without snapping.
    pub fn drag_to(&mut self, x: f32, y: f32, free: bool, cfg: &mut Config) {
        let Some(d) = self.drag else { return };
        let Some(b) = self.boxes[d.index] else { return };
        let snapping = (!free).then(|| self.lines_except(d.index, cfg));
        let to = (x - d.grab[0], y - d.grab[1]);
        let (place, guides) = self.frames.dragged(&b.place, b.at, to, snapping.as_ref());
        self.put(d.index, place, cfg);
        self.guides = guides;
    }

    /// The edges and centres of every other part showing.
    fn lines_except(&self, index: usize, cfg: &Config) -> Snapping {
        let mut s = Snapping {
            tol: SNAP,
            xs: Vec::new(),
            ys: Vec::new(),
        };
        for (i, b) in self.boxes.iter().enumerate() {
            if let Some(b) = b.filter(|_| i != index && cfg.hud.shown(Part::PLACED[i])) {
                s.xs.extend([b.at.x, b.at.x + b.at.w / 2.0, b.at.x + b.at.w]);
                s.ys.extend([b.at.y, b.at.y + b.at.h / 2.0, b.at.y + b.at.h]);
            }
        }
        s
    }

    fn put(&mut self, index: usize, place: Place, cfg: &mut Config) {
        cfg.hud.set_place(Part::PLACED[index], Some(place));
        if let Some(b) = &mut self.boxes[index] {
            b.place = place;
            b.at = self.frames.resolve(&place, b.size, true);
        }
    }

    /// Lets go of the part; true if one was held.
    pub fn release(&mut self, cfg: &Config) -> bool {
        self.guides.clear();
        let held = self.drag.take().is_some();
        if held {
            cfg.save();
        }
        held
    }

    /// Mouse wheel: scrolls the presets under the pointer or resizes the part under it. False:
    /// it's over neither, so it's the playfield's.
    pub fn wheel(&mut self, x: f32, y: f32, notches: i32, cfg: &mut Config) -> bool {
        let inside = |r: [f32; 4]| x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3];
        if !self.panel_hidden && inside(self.presets_rect) {
            let last = self.preset_count(cfg).saturating_sub(PRESET_ROWS);
            let by = self.preset_scroll as i32 - notches;
            self.preset_scroll = by.clamp(0, last as i32) as usize;
            return true;
        }
        if !self.panel_hidden && inside(self.panel_rect) {
            return true;
        }
        let Some(index) = self.part_at(x, y, cfg) else {
            return false;
        };
        self.selected = index;
        self.scale(1.05f32.powi(notches), cfg);
        true
    }

    fn scale(&mut self, by: f32, cfg: &mut Config) {
        let Some(b) = self.boxes[self.selected] else {
            return;
        };
        self.remember(cfg);
        let mut place = b.place;
        place.scale = (place.scale * by).clamp(MIN_SCALE, MAX_SCALE);
        self.put(self.selected, place, cfg);
        cfg.save();
    }

    fn nudge(&mut self, dx: f32, dy: f32, cfg: &mut Config) {
        let Some(b) = self.boxes[self.selected] else {
            return;
        };
        self.remember(cfg);
        let to = (b.at.x + dx, b.at.y + dy);
        let (place, _) = self.frames.dragged(&b.place, b.at, to, None);
        self.put(self.selected, place, cfg);
        cfg.save();
    }

    /// Attaches the selected part's axis to the next target, leaving it where it is.
    fn cycle_target(&mut self, horizontal: bool, cfg: &mut Config) {
        let Some(b) = self.boxes[self.selected] else {
            return;
        };
        self.remember(cfg);
        let axis = if horizontal { b.place.x } else { b.place.y };
        let place = self
            .frames
            .attached(&b.place, b.at, horizontal, axis.target.next());
        self.put(self.selected, place, cfg);
        cfg.save();
    }

    pub fn toggle_part(&mut self, index: usize, cfg: &mut Config) {
        self.remember(cfg);
        self.selected = index;
        cfg.hud.off.toggle(Part::PLACED[index]);
        cfg.save();
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index;
    }

    fn preset_count(&self, cfg: &Config) -> usize {
        hud::builtin_presets().len() + cfg.hud_presets.len()
    }

    /// The preset at `i` of the panel's list: the game's, then the player's.
    fn preset_at(&self, i: usize, cfg: &Config) -> Option<(String, Layout)> {
        let builtin = hud::builtin_presets();
        match builtin.get(i) {
            Some((name, layout)) => Some((name.to_string(), *layout)),
            None => cfg.hud_presets.get(i - builtin.len()).cloned(),
        }
    }

    pub fn load_preset(&mut self, i: usize, cfg: &mut Config) {
        let Some((_, layout)) = self.preset_at(i, cfg) else {
            return;
        };
        self.remember(cfg);
        cfg.hud = layout;
        self.preset = i.checked_sub(hud::builtin_presets().len());
        cfg.save();
    }

    /// `[` and `]`: the preset before or after the one in use.
    fn step_preset(&mut self, by: i32, cfg: &mut Config) {
        let n = self.preset_count(cfg) as i32;
        let now = (0..n as usize)
            .find(|&i| self.preset_at(i, cfg).is_some_and(|p| p.1 == cfg.hud))
            .map_or(if by > 0 { -1 } else { 0 }, |i| i as i32);
        let next = (now + by).rem_euclid(n.max(1)) as usize;
        self.load_preset(next, cfg);
        // Scrolled just far enough to show it.
        let lowest = next.saturating_sub(PRESET_ROWS - 1);
        self.preset_scroll = self.preset_scroll.clamp(lowest, next);
    }

    fn start_naming(&mut self, cfg: &Config) {
        let mut n = cfg.hud_presets.len() + 1;
        let taken = |name: &str| cfg.hud_presets.iter().any(|p| p.0 == name);
        let mut name = tr!("Layout {n}", "レイアウト {n}", "Раскладка {n}";);
        while taken(&name) {
            n += 1;
            name = tr!("Layout {n}", "レイアウト {n}", "Раскладка {n}";);
        }
        let caret = name.len();
        self.naming = Some((name, caret));
    }

    /// Keeps the layout under the name typed (one already taken is replaced).
    pub fn finish_naming(&mut self, cfg: &mut Config) {
        let Some((name, _)) = self.naming.take() else {
            return;
        };
        let name = hud::clean_name(&name);
        if name.is_empty() {
            return;
        }
        let at = match cfg.hud_presets.iter().position(|p| p.0 == name) {
            Some(i) => {
                cfg.hud_presets[i].1 = cfg.hud;
                i
            }
            None => {
                cfg.hud_presets.push((name, cfg.hud));
                cfg.hud_presets.len() - 1
            }
        };
        self.preset = Some(at);
        let row = hud::builtin_presets().len() + at;
        self.preset_scroll = row.saturating_sub(PRESET_ROWS - 1);
        cfg.save();
    }

    pub fn press_button(&mut self, button: Button, cfg: &mut Config) -> Outcome {
        match button {
            Button::AttachX => self.cycle_target(true, cfg),
            Button::AttachY => self.cycle_target(false, cfg),
            Button::Smaller => self.scale(1.0 / 1.1, cfg),
            Button::Bigger => self.scale(1.1, cfg),
            Button::ResetPart => {
                self.remember(cfg);
                cfg.hud.set_place(self.part(), None);
                cfg.save();
            }
            Button::SavePreset => match self.preset.filter(|&i| i < cfg.hud_presets.len()) {
                Some(i) => {
                    cfg.hud_presets[i].1 = cfg.hud;
                    cfg.save();
                    let name = cfg.hud_presets[i].0.clone();
                    return Outcome::Toast(
                        tr!("saved {name}", "{name}を保存しました", "сохранено: {name}";),
                    );
                }
                None => self.start_naming(cfg),
            },
            Button::NewPreset => self.start_naming(cfg),
            Button::DeletePreset => {
                if let Some(i) = self.preset.filter(|&i| i < cfg.hud_presets.len()) {
                    let (name, _) = cfg.hud_presets.remove(i);
                    self.preset = None;
                    let last = self.preset_count(cfg).saturating_sub(PRESET_ROWS);
                    self.preset_scroll = self.preset_scroll.min(last);
                    cfg.save();
                    return Outcome::Toast(
                        tr!("deleted {name}", "{name}を削除しました", "удалено: {name}";),
                    );
                }
            }
            Button::Copy => {
                return Outcome::Toast(if crate::clipboard::set(&cfg.hud.format()) {
                    tr!(
                        "layout copied: paste it to share it",
                        "レイアウトをコピーしました",
                        "раскладка скопирована: вставьте, чтобы поделиться"
                    )
                    .into()
                } else {
                    tr!(
                        "can't reach the clipboard",
                        "クリップボードを使えません",
                        "нет доступа к буферу обмена"
                    )
                    .into()
                });
            }
            Button::Paste => {
                let pasted = crate::clipboard::get().and_then(|t| Layout::parse(&t));
                let Some(layout) = pasted else {
                    return Outcome::Toast(
                        tr!(
                            "no layout on the clipboard",
                            "クリップボードにレイアウトがありません",
                            "в буфере обмена нет раскладки"
                        )
                        .into(),
                    );
                };
                self.remember(cfg);
                cfg.hud = layout;
                self.preset = None;
                cfg.save();
            }
            Button::Undo => {
                if let Some(layout) = self.undo.pop() {
                    cfg.hud = layout;
                    cfg.save();
                }
            }
            Button::ResetAll => {
                self.remember(cfg);
                cfg.hud = Layout::default();
                self.preset = None;
                cfg.save();
            }
            Button::SwapSide => self.panel_left = !self.panel_left,
            Button::Done => return Outcome::Close,
        }
        Outcome::Nothing
    }

    pub fn key(
        &mut self,
        code: KeyCode,
        text: Option<&str>,
        (shift, ctrl): (bool, bool),
        cfg: &mut Config,
    ) -> Outcome {
        if let Some((name, caret)) = &mut self.naming {
            match code {
                KeyCode::Escape => self.naming = None,
                KeyCode::Enter | KeyCode::NumpadEnter => self.finish_naming(cfg),
                KeyCode::Backspace => settings::caret_backspace(name, caret),
                KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Delete => {
                    settings::caret_nav(name, caret, code);
                }
                _ => {
                    if let Some(t) = text.filter(|t| !ctrl && t.chars().all(|c| !c.is_control())) {
                        settings::caret_insert(name, caret, t, NAME_MAX);
                    }
                }
            }
            return Outcome::Nothing;
        }
        let step = if shift { 10.0 } else { 1.0 };
        let n = Part::PLACED.len();
        match code {
            KeyCode::ArrowLeft => self.nudge(-step, 0.0, cfg),
            KeyCode::ArrowRight => self.nudge(step, 0.0, cfg),
            KeyCode::ArrowUp => self.nudge(0.0, -step, cfg),
            KeyCode::ArrowDown => self.nudge(0.0, step, cfg),
            KeyCode::Tab if shift => self.selected = (self.selected + n - 1) % n,
            KeyCode::Tab => self.selected = (self.selected + 1) % n,
            KeyCode::Equal | KeyCode::NumpadAdd => self.scale(1.1, cfg),
            KeyCode::Minus | KeyCode::NumpadSubtract => self.scale(1.0 / 1.1, cfg),
            KeyCode::KeyX => self.cycle_target(true, cfg),
            KeyCode::KeyY => self.cycle_target(false, cfg),
            KeyCode::KeyH => self.toggle_part(self.selected, cfg),
            KeyCode::KeyP => self.panel_hidden = !self.panel_hidden,
            KeyCode::KeyZ if ctrl => return self.press_button(Button::Undo, cfg),
            KeyCode::KeyC if ctrl => return self.press_button(Button::Copy, cfg),
            KeyCode::KeyV if ctrl => return self.press_button(Button::Paste, cfg),
            KeyCode::KeyS if ctrl => return self.press_button(Button::SavePreset, cfg),
            KeyCode::KeyR if shift => return self.press_button(Button::ResetAll, cfg),
            KeyCode::KeyR => return self.press_button(Button::ResetPart, cfg),
            KeyCode::BracketLeft => self.step_preset(-1, cfg),
            KeyCode::BracketRight => self.step_preset(1, cfg),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape => return Outcome::Close,
            _ => {}
        }
        Outcome::Nothing
    }

    /// Over the parts `game` has just drawn: their outlines and the panel.
    pub fn draw(&mut self, c: &mut Canvas, cfg: &Config, game: &Game) {
        self.frames = Frames::new(c.width, cfg.playfield_area);
        for (i, part) in Part::PLACED.into_iter().enumerate() {
            let (place, size, at) = game.part_box(c, part);
            self.boxes[i] = Some(PartBox { place, size, at });
        }
        if self.drag.is_some() {
            self.draw_targets(c);
        }
        let hovered = self.part_at(c.mouse.0, c.mouse.1, cfg);
        for (i, b) in self.boxes.iter().enumerate() {
            let Some(b) = b.filter(|_| cfg.hud.shown(Part::PLACED[i])) else {
                continue;
            };
            let (fill, edge) = if i == self.selected {
                (rgba(accent(), 0.14), rgba(accent(), 1.0))
            } else if hovered == Some(i) {
                (rgba(0xffffff, 0.06), rgba(0xffffff, 0.7))
            } else {
                (rgba(0, 0.0), rgba(0xffffff, 0.22))
            };
            c.round_rect([b.at.x, b.at.y], [b.at.w, b.at.h], 2.0, fill, edge, 1.0);
        }
        if cfg.hud.shown(self.part())
            && let Some(b) = self.boxes[self.selected]
        {
            self.draw_attachment(c, b);
        }
        if self.drag.is_none() && !self.panel_hidden {
            self.draw_panel(c, cfg);
        } else {
            self.panel_rect = [0.0; 4];
            self.presets_rect = [0.0; 4];
        }
        if self.drag.is_none() {
            self.draw_help(c);
        }
    }

    /// What a part can snap to, while one is being dragged.
    fn draw_targets(&self, c: &mut Canvas) {
        for target in hud::Target::ALL.into_iter().skip(1) {
            let [x, y, w, h] = self.frames.rect(target);
            c.round_rect(
                [x, y],
                [w, h],
                2.0,
                rgba(0, 0.0),
                rgba(SNAP_COLOUR, 0.45),
                1.0,
            );
        }
    }

    /// A line from the selected part to where it hangs, and what that is.
    fn draw_attachment(&self, c: &mut Canvas, b: PartBox) {
        let (hr, vr) = (
            self.frames.rect(b.place.x.target),
            self.frames.rect(b.place.y.target),
        );
        let (ax, ay) = (hr[0] + b.place.x.at * hr[2], vr[1] + b.place.y.at * vr[3]);
        let (px, py) = (
            b.at.x + b.place.x.pivot * b.at.w,
            b.at.y + b.place.y.pivot * b.at.h,
        );
        let colour = rgba(SNAP_COLOUR, 0.9);
        c.rect(ax.min(px), py - 0.5, (ax - px).abs(), 1.0, colour);
        c.rect(ax - 0.5, ay.min(py), 1.0, (ay - py).abs(), colour);
        c.circle(ax, ay, 2.5, colour, colour, 0.0);
        c.circle(px, py, 2.0, rgba(0xffffff, 1.0), rgba(0xffffff, 1.0), 0.0);

        let text = format!(
            "{}  -  {} / {}",
            self.part().label(),
            b.place.x.target.label(),
            b.place.y.target.label()
        );
        let w = Canvas::text_width(7.0, &text);
        let x = b.at.x.clamp(4.0, (c.width - w - 8.0).max(4.0));
        let y = if b.at.y > 18.0 {
            b.at.y - 15.0
        } else {
            b.at.y + b.at.h + 3.0
        };
        c.round_rect(
            [x - 4.0, y - 3.0],
            [w + 8.0, 15.0],
            2.0,
            rgba(0x000000, 0.75),
            rgba(0, 0.0),
            0.0,
        );
        c.text(x, y, 7.0, rgba(0xffffff, 1.0), &text);
    }

    fn draw_panel(&mut self, c: &mut Canvas, cfg: &Config) {
        let x = if self.panel_left {
            8.0
        } else {
            c.width - PANEL_W - 8.0
        };
        // Under the FPS overlay in the corner.
        let (y0, h) = (18.0, 442.0);
        self.panel_rect = [x, y0, PANEL_W, h];
        crate::style::panel(c, x, y0, PANEL_W, h);
        // Clicks on the panel never reach the parts or the playfield behind it.
        c.hit(x, y0, PANEL_W, h, Hit::HudPanel);
        let (ix, iw) = (x + 8.0, PANEL_W - 16.0);
        let mut y = y0 + 7.0;
        let title = tr!("INTERFACE", "インターフェース", "ИНТЕРФЕЙС");
        c.text(ix, y, 9.0, rgba(0xffffff, 1.0), title);
        let side = if self.panel_left { ">" } else { "<" };
        button(
            c,
            [ix + iw - 16.0, y - 2.0, 16.0, 12.0],
            side,
            true,
            Button::SwapSide,
        );
        y += 15.0;

        for (i, part) in Part::PLACED.into_iter().enumerate() {
            let on = cfg.hud.shown(part);
            if i == self.selected {
                c.round_rect(
                    [ix - 2.0, y],
                    [iw + 4.0, ROW_H - 1.0],
                    3.0,
                    rgba(accent(), 0.22),
                    rgba(0, 0.0),
                    0.0,
                );
            } else if c.hovered(ix, y, iw, ROW_H - 1.0) {
                c.round_rect(
                    [ix - 2.0, y],
                    [iw + 4.0, ROW_H - 1.0],
                    3.0,
                    rgba(0x262634, 0.7),
                    rgba(0, 0.0),
                    0.0,
                );
            }
            c.hit(ix, y, iw, ROW_H - 1.0, Hit::HudRow(i));
            c.hit(ix, y, 13.0, ROW_H - 1.0, Hit::HudToggle(i));
            let fill = if on { accent() } else { 0x15151d };
            c.round_rect(
                [ix + 1.0, y + 2.0],
                [9.0, 9.0],
                2.0,
                rgba(fill, 1.0),
                rgba(0x6f6f86, 1.0),
                1.0,
            );
            let colour = if on { 0xe8e8f0 } else { 0x6f6f86 };
            let label = Canvas::fit(7.5, iw - 18.0, part.label());
            c.text(ix + 15.0, y + 3.0, 7.5, rgba(colour, 1.0), &label);
            y += ROW_H;
        }
        y += 4.0;

        if let Some(b) = self.boxes[self.selected] {
            let describe = |a: hud::Axis, name: &str, stops: [&str; 3]| {
                let stop = stops[(a.at * 2.0).round() as usize % 3];
                format!("{name}: {} {stop}", a.target.label())
            };
            let lr = [
                tr!("left", "左", "слева"),
                tr!("centre", "中央", "центр"),
                tr!("right", "右", "справа"),
            ];
            let tb = [
                tr!("top", "上", "сверху"),
                tr!("centre", "中央", "центр"),
                tr!("bottom", "下", "снизу"),
            ];
            let fit = |s: String| Canvas::fit(7.0, iw - 8.0, &s);
            button(
                c,
                [ix, y, iw, 13.0],
                &fit(describe(b.place.x, "X", lr)),
                true,
                Button::AttachX,
            );
            y += 13.0 + GAP;
            button(
                c,
                [ix, y, iw, 13.0],
                &fit(describe(b.place.y, "Y", tb)),
                true,
                Button::AttachY,
            );
            y += 13.0 + GAP;
            let reset = tr!("Reset", "戻す", "Сброс");
            let size = format!("{:.0}%", b.place.scale * 100.0);
            button(c, [ix, y, 16.0, 13.0], "-", true, Button::Smaller);
            c.text_centered(ix + 40.0, y + 3.0, 7.5, rgba(0xe8e8f0, 1.0), &size);
            button(c, [ix + 64.0, y, 16.0, 13.0], "+", true, Button::Bigger);
            let moved = cfg.hud.place(self.part()).is_some();
            button(
                c,
                [ix + 88.0, y, iw - 88.0, 13.0],
                reset,
                moved,
                Button::ResetPart,
            );
            y += 19.0;
        }

        c.text(
            ix,
            y,
            7.5,
            rgba(0x9a9ab0, 1.0),
            tr!("PRESETS", "プリセット", "ПРЕСЕТЫ"),
        );
        y += 11.0;
        self.presets_rect = [ix, y, iw, PRESET_ROWS as f32 * ROW_H];
        let count = self.preset_count(cfg);
        self.preset_scroll = self.preset_scroll.min(count.saturating_sub(PRESET_ROWS));
        for row in 0..PRESET_ROWS {
            let i = self.preset_scroll + row;
            let ry = y + row as f32 * ROW_H;
            let Some((name, layout)) = self.preset_at(i, cfg) else {
                continue;
            };
            let current = layout == cfg.hud;
            if current || c.hovered(ix, ry, iw, ROW_H - 1.0) {
                let (col, a) = if current {
                    (accent(), 0.22)
                } else {
                    (0x262634, 0.7)
                };
                c.round_rect(
                    [ix - 2.0, ry],
                    [iw + 4.0, ROW_H - 1.0],
                    3.0,
                    rgba(col, a),
                    rgba(0, 0.0),
                    0.0,
                );
            }
            c.hit(ix, ry, iw, ROW_H - 1.0, Hit::HudPreset(i));
            let colour = if current { 0xffffff } else { 0xc8c8d8 };
            let name = Canvas::fit(7.5, iw - 4.0, &name);
            c.text(ix + 3.0, ry + 3.0, 7.5, rgba(colour, 1.0), &name);
        }
        y += PRESET_ROWS as f32 * ROW_H + 3.0;

        let third = (iw - 2.0 * GAP) / 3.0;
        let half = (iw - GAP) / 2.0;
        let own = self.preset.is_some_and(|i| i < cfg.hud_presets.len());
        if let Some((name, caret)) = &self.naming {
            c.round_rect(
                [ix, y],
                [iw, 13.0],
                3.0,
                rgba(0x15151d, 1.0),
                rgba(accent(), 1.0),
                1.0,
            );
            c.text(ix + 4.0, y + 3.0, 7.5, rgba(0xffffff, 1.0), name);
            let cx = ix + 4.0 + settings::caret_x(7.5, name, *caret);
            c.rect(cx, y + 2.0, 1.0, 9.0, rgba(0xffffff, 0.9));
        } else {
            let save = tr!("Save", "保存", "Сохр.");
            let new = tr!("New", "新規", "Новый");
            let delete = tr!("Delete", "削除", "Удалить");
            button(c, [ix, y, third, 13.0], save, true, Button::SavePreset);
            button(
                c,
                [ix + third + GAP, y, third, 13.0],
                new,
                true,
                Button::NewPreset,
            );
            button(
                c,
                [ix + 2.0 * (third + GAP), y, third, 13.0],
                delete,
                own,
                Button::DeletePreset,
            );
        }
        y += 13.0 + GAP;
        let copy = tr!("Copy", "コピー", "Копировать");
        let paste = tr!("Paste", "貼り付け", "Вставить");
        button(c, [ix, y, half, 13.0], copy, true, Button::Copy);
        button(
            c,
            [ix + half + GAP, y, half, 13.0],
            paste,
            true,
            Button::Paste,
        );
        y += 13.0 + GAP;
        let undo = tr!("Undo", "元に戻す", "Отменить");
        let reset_all = tr!("Reset all", "全て戻す", "Сбросить всё");
        button(
            c,
            [ix, y, half, 13.0],
            undo,
            !self.undo.is_empty(),
            Button::Undo,
        );
        button(
            c,
            [ix + half + GAP, y, half, 13.0],
            reset_all,
            true,
            Button::ResetAll,
        );
        y += 17.0;
        button(
            c,
            [ix, y, iw, 15.0],
            tr!("Done", "完了", "Готово"),
            true,
            Button::Done,
        );
    }

    fn draw_help(&self, c: &mut Canvas) {
        let help = tr!(
            "Drag: snaps and attaches (Shift: free).  Wheel: size.  Arrows: nudge.  H: show.  P: panel",
            "ドラッグ: 吸着して取り付け (Shift: 自由)。ホイール: サイズ。矢印: 微調整。H: 表示。P: パネル",
            "Тяните: прилипает и крепится (Shift: свободно). Колесо: размер. Стрелки: сдвиг. H: показать. P: панель"
        );
        let panel = if self.panel_hidden {
            0.0
        } else {
            PANEL_W + 16.0
        };
        let room = c.width - panel - 16.0;
        let x = if self.panel_left { panel + 8.0 } else { 8.0 };
        let size = Canvas::fit_size(7.0, 5.0, room, help);
        c.round_rect(
            [x, 462.0],
            [room, 14.0],
            3.0,
            rgba(0x000000, 0.7),
            rgba(0, 0.0),
            0.0,
        );
        let text = Canvas::fit(size, room - 6.0, help);
        c.text_centered(x + room / 2.0, 465.0, size, rgba(0xc8c8d8, 1.0), &text);
    }
}

fn button(c: &mut Canvas, [x, y, w, h]: [f32; 4], label: &str, on: bool, b: Button) {
    let hovered = on && c.hovered(x, y, w, h);
    let fill = if hovered { 0x2f2f40 } else { 0x1d1d28 };
    c.round_rect(
        [x, y],
        [w, h],
        3.0,
        rgba(fill, 1.0),
        rgba(if on { 0x4a4a60 } else { 0x2a2a38 }, 1.0),
        1.0,
    );
    let colour = if on { 0xe8e8f0 } else { 0x55556a };
    let text = Canvas::fit(7.0, w - 4.0, label);
    c.text_centered(
        x + w / 2.0,
        y + (h - 7.0) / 2.0,
        7.0,
        rgba(colour, 1.0),
        &text,
    );
    if on {
        c.hit(x, y, w, h, Hit::HudButton(b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Editor, Config, Game, Canvas) {
        let cfg = Config::default();
        let mut c = Canvas::default();
        c.clear(853.3);
        let ed = Editor::new(&cfg);
        (ed, cfg, Game::hud_sample(), c)
    }

    /// A frame of the editor, as the game draws it.
    fn frame(ed: &mut Editor, cfg: &Config, g: &mut Game, c: &mut Canvas) {
        c.clear(853.3);
        g.hud = cfg.hud;
        g.hud_area = cfg.playfield_area;
        g.hidden = cfg.hud.off;
        g.draw_parts(c);
        ed.draw(c, cfg, g);
    }

    fn index(part: Part) -> usize {
        Part::PLACED.iter().position(|&p| p == part).unwrap()
    }

    fn centre(ed: &Editor, part: Part) -> (f32, f32) {
        let b = ed.boxes[index(part)].unwrap().at;
        (b.x + b.w / 2.0, b.y + b.h / 2.0)
    }

    #[test]
    fn dragging_a_part_moves_it_and_a_smaller_playfield_takes_it_along() {
        let (mut ed, mut cfg, mut g, mut c) = setup();
        cfg.hud.off.toggle(Part::Ur);
        frame(&mut ed, &cfg, &mut g, &mut c);
        let (x, y) = centre(&ed, Part::Score);
        assert!(ed.press(x, y, &cfg));
        // Under the lane, in the middle of the screen.
        let under = crate::game::LANE_Y + crate::game::LANE_H / 2.0 + 14.0 + 2.0;
        ed.drag_to(853.3 / 2.0 + 2.0, under, false, &mut cfg);
        assert!(!ed.guides.is_empty());
        ed.release(&cfg);
        assert!(ed.guides.is_empty());
        let place = cfg.hud.place(Part::Score).unwrap();
        assert_eq!(place.y.target, hud::Target::Lane);
        assert_eq!(place.x.target, hud::Target::Playfield);

        let before = g.part_box(&c, Part::Score).2;
        cfg.playfield_area = Area {
            x: 0.25,
            y: 0.25,
            w: 0.5,
            h: 0.5,
        };
        frame(&mut ed, &cfg, &mut g, &mut c);
        let after = ed.boxes[index(Part::Score)].unwrap().at;
        assert!((after.w - before.w / 2.0).abs() < 0.1);
        assert!(((after.x + after.w / 2.0) - 853.3 / 2.0).abs() < 0.1);
    }

    #[test]
    fn shift_places_a_part_without_snapping() {
        let (mut ed, mut cfg, mut g, mut c) = setup();
        frame(&mut ed, &cfg, &mut g, &mut c);
        let (x, y) = centre(&ed, Part::Accuracy);
        ed.press(x, y, &cfg);
        ed.drag_to(853.3 / 2.0 + 2.0, 300.0, true, &mut cfg);
        assert!(ed.guides.is_empty());
        assert_eq!(
            cfg.hud.place(Part::Accuracy).unwrap().x.target,
            hud::Target::Screen
        );
    }

    #[test]
    fn a_press_off_every_part_leaves_it_to_the_playfield() {
        let (mut ed, cfg, mut g, mut c) = setup();
        frame(&mut ed, &cfg, &mut g, &mut c);
        assert!(!ed.press(600.0, 340.0, &cfg));
        assert!(!ed.dragging());
    }

    #[test]
    fn undo_brings_back_what_a_drag_and_a_preset_changed() {
        let (mut ed, mut cfg, mut g, mut c) = setup();
        frame(&mut ed, &cfg, &mut g, &mut c);
        let start = cfg.hud;
        let (x, y) = centre(&ed, Part::Score);
        ed.press(x, y, &cfg);
        ed.drag_to(300.0, 300.0, true, &mut cfg);
        ed.release(&cfg);
        let dragged = cfg.hud;
        assert_ne!(dragged, start);
        ed.load_preset(1, &mut cfg);
        assert_eq!(ed.press_button(Button::Undo, &mut cfg), Outcome::Nothing);
        assert_eq!(cfg.hud, dragged);
        ed.press_button(Button::Undo, &mut cfg);
        assert_eq!(cfg.hud, start);
    }

    #[test]
    fn the_players_presets_save_replace_and_delete() {
        let (mut ed, mut cfg, mut g, mut c) = setup();
        frame(&mut ed, &cfg, &mut g, &mut c);
        // Nothing of the player's to overwrite yet: Save asks for a name.
        ed.press_button(Button::SavePreset, &mut cfg);
        assert!(ed.naming());
        ed.key(KeyCode::Digit1, Some("!"), (true, false), &mut cfg);
        ed.key(KeyCode::Enter, None, (false, false), &mut cfg);
        assert!(!ed.naming());
        assert_eq!(cfg.hud_presets.len(), 1);
        assert!(cfg.hud_presets[0].0.ends_with('!'));

        ed.toggle_part(index(Part::Ur), &mut cfg);
        ed.press_button(Button::SavePreset, &mut cfg);
        assert_eq!(cfg.hud_presets.len(), 1);
        assert!(cfg.hud_presets[0].1.shown(Part::Ur));

        ed.press_button(Button::DeletePreset, &mut cfg);
        assert!(cfg.hud_presets.is_empty());
        ed.press_button(Button::DeletePreset, &mut cfg);
    }

    #[test]
    fn a_built_in_preset_is_never_overwritten() {
        let (mut ed, mut cfg, ..) = setup();
        ed.load_preset(2, &mut cfg);
        ed.press_button(Button::SavePreset, &mut cfg);
        assert!(
            ed.naming(),
            "saving over the game's own asks for a new name"
        );
        ed.key(KeyCode::Escape, None, (false, false), &mut cfg);
        assert!(!ed.naming() && cfg.hud_presets.is_empty());
    }

    #[test]
    fn the_keys_for_typing_a_name_dont_reach_the_parts() {
        let (mut ed, mut cfg, mut g, mut c) = setup();
        frame(&mut ed, &cfg, &mut g, &mut c);
        ed.press_button(Button::NewPreset, &mut cfg);
        let before = cfg.hud;
        ed.key(KeyCode::KeyH, Some("h"), (false, false), &mut cfg);
        ed.key(KeyCode::ArrowLeft, None, (false, false), &mut cfg);
        assert_eq!(cfg.hud, before);
        assert_eq!(
            ed.key(KeyCode::Escape, None, (false, false), &mut cfg),
            Outcome::Nothing,
            "Esc cancels the name, not the editor"
        );
        assert_eq!(
            ed.key(KeyCode::Escape, None, (false, false), &mut cfg),
            Outcome::Close
        );
    }

    #[test]
    fn stepping_through_presets_wraps_round_and_keeps_the_row_in_view() {
        let (mut ed, mut cfg, ..) = setup();
        for i in 0..5 {
            cfg.hud_presets.push((format!("p{i}"), {
                let mut l = Layout::default();
                l.set_place(
                    Part::Score,
                    Some(Place::new(
                        hud::on_screen(0.0, i as f32),
                        hud::on_screen(0.0, 0.0),
                    )),
                );
                l
            }));
        }
        let n = ed.preset_count(&cfg);
        for _ in 0..n {
            ed.key(KeyCode::BracketRight, None, (false, false), &mut cfg);
            let row = (0..n)
                .find(|&i| ed.preset_at(i, &cfg).unwrap().1 == cfg.hud)
                .unwrap();
            assert!((ed.preset_scroll..ed.preset_scroll + PRESET_ROWS).contains(&row));
        }
        assert_eq!(cfg.hud, Layout::default());
    }

    #[test]
    fn switching_a_part_off_takes_it_off_the_list_of_things_to_grab() {
        let (mut ed, mut cfg, mut g, mut c) = setup();
        frame(&mut ed, &cfg, &mut g, &mut c);
        let (x, y) = centre(&ed, Part::Score);
        ed.toggle_part(index(Part::Score), &mut cfg);
        assert!(!ed.press(x, y, &cfg));
    }

    #[test]
    fn the_counters_show_only_once_added_and_a_hidden_interface_hides_them() {
        let mut g = Game::hud_sample();
        let draw = |g: &Game| {
            let mut c = Canvas::default();
            c.clear(853.3);
            g.draw_parts(&mut c);
            c.shapes.len()
        };
        g.hidden = Layout::default().off;
        let without = draw(&g);
        g.hidden = hud::Parts::NONE;
        assert!(draw(&g) > without);
    }

    #[test]
    fn the_interface_stays_put_until_the_player_moves_it() {
        let g = Game::hud_sample();
        let mut c = Canvas::default();
        c.clear(853.3);
        let at = |p: Part| g.part_box(&c, p).2;
        assert_eq!((at(Part::Score).x + at(Part::Score).w).round(), 835.0);
        assert_eq!(at(Part::Score).y, 18.0);
        assert_eq!(at(Part::Accuracy).y, 50.0);
        assert_eq!(at(Part::Mods).y, 72.0);
        assert_eq!(at(Part::HitError).x, 853.3 / 2.0 - 150.0);
        assert_eq!(at(Part::HitError).y, 408.0);
        assert_eq!((at(Part::Progress).x, at(Part::Progress).w), (0.0, 853.3));
    }
}
