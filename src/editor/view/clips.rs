//! The Effects page: effect clips as blocks on rows under the timeline, the shelf of ready-made
//! effects, and the settings of the clip picked.
//!
//! The shelf is split in tabs by what its effects do. A click on an effect picks it, and a drag on
//! a row then draws it there (a click on a row lays one bar; buttons lay it over the kiai or the
//! notes selected). A drag on empty rows picks every block it touches; Shift or Ctrl clicks add
//! or take one away. Blocks picked move together, Ctrl+D copies them and Delete removes them;
//! their ends resize one, a right click deletes one. Ctrl+drag drags copies, Ctrl+C / X / V copy,
//! cut and paste them at the playhead, Ctrl+A picks all. Blocks never share a stretch of a row.
//! The rows scroll, with a free one always at the bottom.

use super::chrome::wrap;
use super::{Btn, Drag, Field, Keys, Mode, View, fmt_time, inside};
use crate::editor::clips::{
    PRESETS, Shelf, amount_step, describe, listed, rescale_amount, target_about, target_colour,
    target_columns, target_label, targets, trim,
};
use crate::render::{Canvas, accent, rgba};
use crate::tko::clips::{Clip, LANES, Layer, MAX_SPIN, Motion, Notes, Pose, Show, Target, When};
use crate::tko::track::Ease;

/// Rows of clips shown at once (more scroll).
pub(super) const ROWS: u8 = 6;
thread_local! {
    /// Blocks copied with Ctrl+C or Ctrl+X: kept while the game runs, so they paste into
    /// another map too.
    static CLIPBOARD: std::cell::RefCell<Vec<Clip>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Where (from the top of the panel) the strip of motion keys sits, and how tall it is.
const STRIP_Y: f32 = 11.0;
const STRIP_H: f32 = 10.0;
/// Where the rows start, under the title and the motion strip.
const ROWS_TOP: f32 = STRIP_Y + STRIP_H + 1.0;
/// How close (units) to a block's end a press resizes it.
const EDGE: f32 = 4.0;
const DIM: u32 = 0x9a9ab0;

/// What the Effects page can be told to do.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FxAct {
    /// Picks shelf effect `i` to draw on the rows (again: puts it back).
    Preset(usize),
    /// Opens the shelf's tab `k` (of `Shelf::ALL`).
    Tab(u8),
    /// Lays the effect picked on the shelf over the kiai at the playhead (0) or the notes
    /// selected (1).
    Place(u8),
    /// Puts the effect picked on the shelf back.
    Disarm,
    /// Back to the shelf.
    Back,
    /// Opens a dropdown of the clip picked: what it moves (0), its ease (1), its picture (2).
    Menu(u8),
    /// Its picture, by its index in the map folder's (`View::map_pictures`).
    SetPicture(u16),
    /// What it moves, by its index in `targets`.
    SetWhat(u16),
    /// Its ease, by its index in `Ease::ALL`.
    SetEase(u8),
    /// The previous or next thing to move.
    What(i32),
    /// The previous or next group of notes (0 is all of them).
    Group(i32),
    /// Which copy of the playfield it moves (0 the first).
    CopyNo(u8),
    /// For its whole length, every few beats, on notes, on hits, on misses.
    When(u8),
    Notes(Notes),
    Motion(Motion),
    Keep,
    /// Over the kiai it is in, or over the notes selected.
    FitKiai,
    FitSelection,
    Duplicate,
    Delete,
    /// Shows the keyframe and node pages.
    Advanced,
    /// A layer: text or a picture.
    LayerText(bool),
    /// The previous or next picture in the map's folder.
    Picture(i32),
    /// Edit the layer's pose at its end (else at its start).
    EndPose(bool),
    /// The layer moves to its own end pose (else it stays as it starts).
    Moves,
    Behind,
    Colour,
}

/// The colours a layer cycles through.
const LAYER_COLOURS: [u32; 8] = [
    0xffffff, 0xFFD34D, 0xFF6FB0, 0x5BA8FF, 0x5FD49A, 0xFFA040, 0xB36BFF, 0x101016,
];

/// A number of the clip picked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CF {
    Amount,
    Every,
    Length,
    FadeIn,
    FadeOut,
    /// A layer's pose (start or end, `View::fx_end`).
    X,
    Y,
    Size,
    Turn,
    /// A layer's spin, degrees a beat (not part of a pose).
    Spin,
    Seen,
}

impl CF {
    fn label(self) -> &'static str {
        match self {
            CF::Amount => "amount",
            CF::Every => "every (beats)",
            CF::Length => "takes (beats)",
            CF::FadeIn => "in (beats)",
            CF::FadeOut => "out (beats)",
            CF::X => "x",
            CF::Y => "y",
            CF::Size => "size",
            CF::Turn => "turn",
            CF::Spin => "spin /beat",
            CF::Seen => "opacity",
        }
    }
}

/// Where on a block a press landed.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    Body,
    Start,
    End,
}

impl View {
    /// Whether the area under the timeline holds the clip rows.
    pub(super) fn clip_mode(&self) -> bool {
        self.mode == Mode::Effects && self.g_page == 4 && self.ed.is_tko()
    }

    pub(super) fn picked(&self) -> Option<&Clip> {
        self.ed.clip(self.fx_sel?)
    }

    // ---- the panel ----

    /// The clips picked: every one picked with the one whose settings show, else just that one.
    pub(super) fn fx_group(&self) -> Vec<u32> {
        let Some(sel) = self.fx_sel.filter(|&id| self.ed.clip(id).is_some()) else {
            return Vec::new();
        };
        if self.fx_many.contains(&sel) {
            self.fx_many
                .iter()
                .copied()
                .filter(|&id| self.ed.clip(id).is_some())
                .collect()
        } else {
            vec![sel]
        }
    }

    /// Picks clips `ids` (the first shows its settings); none goes back to the shelf.
    pub(super) fn pick_clips(&mut self, ids: &[u32]) {
        self.fx_many = ids.to_vec();
        self.fx_sel = ids.first().copied();
    }

    // ---- the panel ----

    pub(super) fn draw_clips_page(&mut self, c: &mut Canvas, x: f32, mut y: f32, inner: f32) {
        if !self.ed.is_tko() {
            self.draw_tko_explainer(c, x, y, inner);
            return;
        }
        if self.picked().is_none() {
            self.draw_shelf(c, x, y, inner);
            return;
        }
        let n = self.fx_group().len();
        if n > 1 {
            let text = format!(
                "{n} blocks picked: drag one to move them all (Ctrl+drag copies), Ctrl+C / Ctrl+V copy and paste at the playhead, Ctrl+D copies them after, Delete removes them. The settings below are the brightest one's."
            );
            y = self.paragraph(c, x, y, inner, 6.0, 0xffffff, &text) + 2.0;
        }
        self.draw_clip_settings(c, x, y, inner);
    }

    /// The clips page, scrolled when it is taller than the panel (a picture's settings).
    pub(super) fn draw_clips_scrolled(&mut self, c: &mut Canvas, x: f32, top: f32, inner: f32) {
        if self.fx_scroll_for != self.fx_sel {
            self.fx_scroll_for = self.fx_sel;
            self.fx_scroll = 0.0;
        }
        let (scroll, most) = self.draw_scrolled(c, x, top, inner, self.fx_scroll, |v, c, y, _| {
            v.draw_clips_page(c, x, y, inner)
        });
        (self.fx_scroll, self.fx_page_most) = (scroll, most);
    }

    /// A page of the panel that can be taller than it, drawn `scroll` up from `top`: cut to the
    /// panel, with a bar on its right; what is scrolled out of sight can't be clicked. `page`
    /// draws it from a y, given the scroll. Returns the scroll (less when the page got shorter)
    /// and the most it can be (0 when the page fits).
    pub(super) fn draw_scrolled(
        &mut self,
        c: &mut Canvas,
        x: f32,
        top: f32,
        inner: f32,
        mut scroll: f32,
        page: impl Fn(&mut Self, &mut Canvas, f32, f32),
    ) -> (f32, f32) {
        let [_, py, pw, ph] = self.layout.inspector;
        let bottom = py + ph - 4.0;
        let (shapes, buttons, tips) = (c.shapes.len(), self.buttons.len(), self.tips.len());
        let most = loop {
            page(self, c, top - scroll, scroll);
            let end = c.shapes[shapes..]
                .iter()
                .map(|s| s.center[1] + s.half[1])
                .fold(top, f32::max)
                + scroll;
            let most = (end - bottom + 4.0).max(0.0);
            if most <= 0.0 {
                return (0.0, 0.0);
            }
            if scroll <= most {
                break most;
            }
            // Shorter than it was (another tab): drawn again at the most it can scroll.
            scroll = most;
            c.shapes.truncate(shapes);
            self.buttons.truncate(buttons);
            self.tips.truncate(tips);
        };
        let view = [x - 8.0, top - 2.0, pw, bottom - top + 2.0];
        c.clip_since(shapes, view[0], view[1], view[2], view[3]);
        let seen = |r: &[f32; 4]| r[1] + r[3] * 0.5 >= view[1] && r[1] + r[3] * 0.5 <= bottom;
        let kept: Vec<_> = self
            .buttons
            .drain(buttons..)
            .filter(|(r, _)| seen(r))
            .collect();
        self.buttons.extend(kept);
        let kept: Vec<_> = self.tips.drain(tips..).filter(|(r, _)| seen(r)).collect();
        self.tips.extend(kept);
        // The bar.
        let track = [x + inner + 3.0, top, 3.0, bottom - top];
        let thumb = (track[3] * track[3] / (track[3] + most)).max(12.0);
        let ty = track[1] + (track[3] - thumb) * scroll / most;
        c.rect(track[0], track[1], track[2], track[3], rgba(0xffffff, 0.05));
        c.round_rect(
            [track[0], ty],
            [track[2], thumb],
            1.5,
            rgba(0x7a7a98, 0.9),
            rgba(0, 0.0),
            0.0,
        );
        (scroll, most)
    }

    /// The wheel over the clips page scrolls it, when it is taller than the panel. False when it
    /// isn't its.
    pub(super) fn wheel_clips_page(&mut self, notches: i32) -> bool {
        let (mx, my) = self.mouse;
        if self.mode != super::Mode::Effects
            || self.g_page != 4
            || self.fx_page_most <= 0.0
            || !inside(self.layout.inspector, mx, my)
        {
            return false;
        }
        self.fx_scroll = (self.fx_scroll - notches as f32 * 20.0).clamp(0.0, self.fx_page_most);
        true
    }

    fn draw_shelf(&mut self, c: &mut Canvas, x: f32, y: f32, inner: f32) {
        // The tabs, each underlined in the colour of what its effects move.
        let tabs: Vec<(String, Btn, bool)> = Shelf::ALL
            .iter()
            .enumerate()
            .map(|(k, t)| {
                (
                    t.label().to_string(),
                    Btn::Fx(FxAct::Tab(k as u8)),
                    self.fx_tab == *t,
                )
            })
            .collect();
        let first = self.buttons.len();
        let mut y = self.wrap_row(c, x, y, inner, &tabs);
        for (k, t) in Shelf::ALL.iter().enumerate() {
            if let Some((r, _)) = self.buttons.get(first + k).copied() {
                let on = self.fx_tab == *t;
                c.rect(
                    r[0] + 3.0,
                    r[1] + r[3] - 3.0,
                    r[2] - 6.0,
                    1.5,
                    rgba(t.colour(), if on { 1.0 } else { 0.4 }),
                );
            }
        }
        c.rect(x, y - 1.0, inner, 1.0, rgba(0x2a2a38, 1.0));
        y += 3.0;
        // Its effects.
        let list = self.fx_tab.presets();
        let items: Vec<(String, Btn, bool)> = list
            .iter()
            .map(|&i| {
                (
                    PRESETS[i].name.to_string(),
                    Btn::Fx(FxAct::Preset(i)),
                    self.fx_armed == Some(i),
                )
            })
            .collect();
        let first = self.buttons.len();
        y = self.wrap_row(c, x, y, inner, &items);
        for (k, &i) in list.iter().enumerate() {
            if let Some((r, _)) = self.buttons.get(first + k).copied() {
                let p = &PRESETS[i];
                let key = if k < 9 {
                    format!("\nKey {}: record it live (press again to end).", k + 1)
                } else {
                    String::new()
                };
                self.tips.push((
                    r,
                    format!(
                        "{}\n{}\nClick, then drag on a row to place it.{key}",
                        p.name, p.about
                    ),
                ));
            }
        }
        // What to do with the one picked.
        match self.fx_armed.and_then(|i| PRESETS.get(i)) {
            Some(p) => {
                let text = format!(
                    "{}: drag on a row below to place it, or click a row for one bar (Shift keeps it picked). Esc puts it back.",
                    p.name
                );
                y = self.paragraph(c, x, y, inner, 6.0, 0xffffff, &text);
                let mut row = Vec::new();
                if self.ed.kiai_around(self.ed.time).is_some() {
                    row.push((
                        "Over the kiai here".to_string(),
                        Btn::Fx(FxAct::Place(0)),
                        false,
                    ));
                }
                if self.selection_span().is_some() {
                    row.push((
                        "Over the notes selected".to_string(),
                        Btn::Fx(FxAct::Place(1)),
                        false,
                    ));
                }
                row.push(("Cancel".to_string(), Btn::Fx(FxAct::Disarm), false));
                self.wrap_row(c, x, y + 1.0, inner, &row);
            }
            None => {
                let hint = "Pick an effect, then drag on a row below to place it. Drag over empty rows to pick several blocks (Shift adds), then Ctrl+drag, Ctrl+D or Ctrl+C / Ctrl+V copies them. Keys 1-9 record this tab's effects live while the song plays.";
                self.paragraph(c, x, y, inner, 6.0, DIM, hint);
            }
        }
    }

    /// A number of the picked clip: its label, the value (click to type), and - and +.
    fn clip_number(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, f: CF, value: f64) -> f32 {
        let typing = matches!(&self.typing, Some((Field::Fx(tf), _)) if *tf == f);
        let text = match &self.typing {
            Some((Field::Fx(tf), t)) if *tf == f => format!("{}: {t}_", f.label()),
            _ => format!("{} {}", f.label(), trim(value)),
        };
        let lw = w - 32.0;
        self.button(c, [x, y, lw, 15.0], &text, Btn::Edit(Field::Fx(f)), typing);
        self.button(
            c,
            [x + lw + 2.0, y, 14.0, 15.0],
            "-",
            Btn::Dec(Field::Fx(f)),
            false,
        );
        self.button(
            c,
            [x + lw + 17.0, y, 14.0, 15.0],
            "+",
            Btn::Inc(Field::Fx(f)),
            false,
        );
        x + w + 6.0
    }

    fn draw_clip_settings(&mut self, c: &mut Canvas, x: f32, mut y: f32, inner: f32) {
        let Some(clip) = self.picked().cloned() else {
            return;
        };
        if let Some(layer) = clip.layer.as_deref() {
            return self.draw_layer_settings(c, x, y, inner, &clip, layer);
        }
        let target = clip.target();
        // What it moves, from a menu.
        let what = target.map_or_else(|| clip.what.clone(), target_label);
        let open = self.fx_drop.is_some_and(|d| d.0 == 0);
        let mut row = vec![
            ("< Effects".to_string(), Btn::Fx(FxAct::Back), false),
            ("<".to_string(), Btn::Fx(FxAct::What(-1)), false),
            (format!("   {what}   v"), Btn::Fx(FxAct::Menu(0)), open),
            (">".to_string(), Btn::Fx(FxAct::What(1)), false),
        ];
        if matches!(target, Some(Target::Notes(..))) {
            let groups = self.ed.gimmick().map_or(0, |g| g.groups.len());
            if groups > 0 {
                row.push((
                    match clip.group {
                        0 => "all notes".to_string(),
                        g => format!("group {g}"),
                    },
                    Btn::Fx(FxAct::Group(1)),
                    false,
                ));
            }
        }
        if let Some(Target::Copy(k, _)) = target {
            for n in 0..crate::tko::clips::MAX_COPIES {
                row.push((
                    format!("copy {}", n + 1),
                    Btn::Fx(FxAct::CopyNo(n as u8)),
                    k == n,
                ));
            }
        }
        let first = self.buttons.len();
        self.row(c, x, y, &row);
        if let (Some(t), Some((r, _))) = (target, self.buttons.get(first + 2).copied()) {
            c.circle(
                r[0] + 8.0,
                r[1] + r[3] / 2.0,
                2.5,
                rgba(target_colour(t), 1.0),
                rgba(0x000000, 0.0),
                0.0,
            );
            self.tips.push((
                r,
                format!(
                    "{}\n{}\nClick for everything a clip can move.",
                    target_label(t),
                    target_about(t)
                ),
            ));
        }
        y += 19.0;
        // When.
        let kind = match clip.when {
            When::Span => 0,
            When::Beats { .. } => 1,
            When::Notes { .. } => 2,
            When::Hits { .. } => 3,
            When::Misses => 4,
        };
        let whens = [
            "the whole time",
            "every few beats",
            "on notes",
            "on hits",
            "on misses",
        ];
        let mut items: Vec<(String, Btn, bool)> = whens
            .iter()
            .enumerate()
            .map(|(i, w)| (w.to_string(), Btn::Fx(FxAct::When(i as u8)), kind == i))
            .collect();
        if let When::Notes { notes } | When::Hits { notes } = clip.when {
            for n in Notes::ALL {
                items.push((n.label().to_string(), Btn::Fx(FxAct::Notes(n)), notes == n));
            }
        }
        y = self.wrap_row(c, x, y, inner, &items);
        // How much, and how.
        let col = ((inner - 12.0) / 3.0).max(60.0);
        let mut cx = self.clip_number(c, x, y, col, CF::Amount, clip.amount);
        if clip.when == When::Span {
            cx = self.clip_number(c, cx, y, col, CF::FadeIn, clip.fade_in);
            self.clip_number(c, cx, y, col, CF::FadeOut, clip.fade_out);
        } else {
            if let When::Beats { every } = clip.when {
                cx = self.clip_number(c, cx, y, col, CF::Every, every);
            }
            self.clip_number(c, cx, y, col, CF::Length, clip.length);
        }
        y += 19.0;
        let mut items: Vec<(String, Btn, bool)> = Vec::new();
        if clip.when != When::Span {
            for m in Motion::ALL {
                items.push((
                    m.label().to_string(),
                    Btn::Fx(FxAct::Motion(m)),
                    clip.motion == m,
                ));
            }
        }
        items.push((
            format!("ease: {}  v", clip.ease.label()),
            Btn::Fx(FxAct::Menu(1)),
            self.fx_drop.is_some_and(|d| d.0 == 1),
        ));
        items.push(("keep after".to_string(), Btn::Fx(FxAct::Keep), clip.keep));
        y = self.wrap_row(c, x, y, inner, &items);
        let items = [
            ("Fit to kiai".to_string(), Btn::Fx(FxAct::FitKiai), false),
            (
                "Fit to selection".to_string(),
                Btn::Fx(FxAct::FitSelection),
                false,
            ),
            (
                "Copy after  Ctrl+D".to_string(),
                Btn::Fx(FxAct::Duplicate),
                false,
            ),
            ("Delete".to_string(), Btn::Fx(FxAct::Delete), false),
        ];
        y = self.wrap_row(c, x, y, inner, &items);
        let beats = (clip.end - clip.start) / self.ed.beat_at(clip.start);
        let info = format!(
            "{} to {}  ({} beats)",
            fmt_time(clip.start),
            fmt_time(clip.end),
            trim((beats * 100.0).round() / 100.0)
        );
        c.text(x, y, 6.0, rgba(DIM, 1.0), &Canvas::fit(6.0, inner, &info));
    }

    /// The settings of a text or picture block: what it shows, where (at its start, and at its
    /// end if it moves), how it comes and goes, and how it pulses.
    fn draw_layer_settings(
        &mut self,
        c: &mut Canvas,
        x: f32,
        mut y: f32,
        inner: f32,
        clip: &Clip,
        layer: &Layer,
    ) {
        let text = matches!(layer.show, Show::Text { .. });
        let mut row = vec![
            ("< Effects".to_string(), Btn::Fx(FxAct::Back), false),
            ("Text".to_string(), Btn::Fx(FxAct::LayerText(true)), text),
            (
                "Picture".to_string(),
                Btn::Fx(FxAct::LayerText(false)),
                !text,
            ),
        ];
        match &layer.show {
            Show::Text { text } => {
                let shown = match &self.typing {
                    Some((Field::FxText, t)) if self.typing_fresh => {
                        format!("{t}  (type to replace)")
                    }
                    Some((Field::FxText, t)) => format!("{t}_"),
                    _ => format!("{text}  (click to type)"),
                };
                row.push((shown, Btn::Edit(Field::FxText), self.typing.is_some()));
            }
            Show::Picture { file } => {
                row.push(("<".to_string(), Btn::Fx(FxAct::Picture(-1)), false));
                let label = if file.is_empty() {
                    "no picture: put one in the map folder or drop it here".to_string()
                } else {
                    format!("{file}  v")
                };
                let open = self.fx_drop.is_some_and(|d| d.0 == 2);
                row.push((label, Btn::Fx(FxAct::Menu(2)), open || !file.is_empty()));
                row.push((">".to_string(), Btn::Fx(FxAct::Picture(1)), false));
            }
        }
        y = self.wrap_row(c, x, y, inner, &row);
        let moves = layer.to.is_some();
        let end = self.fx_end && moves;
        let mut items = vec![("moves".to_string(), Btn::Fx(FxAct::Moves), moves)];
        if moves {
            items.push((
                "edit start pose".to_string(),
                Btn::Fx(FxAct::EndPose(false)),
                !end,
            ));
            items.push((
                "edit end pose".to_string(),
                Btn::Fx(FxAct::EndPose(true)),
                end,
            ));
        }
        items.push((
            "behind the notes".to_string(),
            Btn::Fx(FxAct::Behind),
            layer.behind,
        ));
        let first = self.buttons.len();
        items.push(("colour".to_string(), Btn::Fx(FxAct::Colour), false));
        y = self.wrap_row(c, x, y, inner, &items);
        let pose_tips: Vec<[f32; 4]> = self.buttons[first..]
            .iter()
            .filter(|(_, b)| matches!(b, Btn::Fx(FxAct::EndPose(_)) | Btn::Fx(FxAct::Moves)))
            .map(|(r, _)| *r)
            .collect();
        for r in pose_tips {
            self.tips.push((
                r,
                "A moving block glides from its start pose to its end pose over its length. These pick which of the two the numbers below and a drag in the preview change; the preview frames both.".to_string(),
            ));
        }
        if let Some((r, _)) = self.buttons.get(first + items.len() - 1).copied() {
            c.round_rect(
                [r[0] + r[2] - 9.0, r[1] + 4.0],
                [6.0, r[3] - 8.0],
                1.5,
                rgba(layer.colour, 1.0),
                rgba(0x000000, 0.8),
                1.0,
            );
        }
        let col = ((inner - 12.0) / 3.0).max(60.0);
        let p = if end {
            layer.to.unwrap_or(layer.from)
        } else {
            layer.from
        };
        let mut cx = self.clip_number(c, x, y, col, CF::X, p.x);
        cx = self.clip_number(c, cx, y, col, CF::Y, p.y);
        self.clip_number(c, cx, y, col, CF::Size, p.size);
        y += 19.0;
        cx = self.clip_number(c, x, y, col, CF::Turn, p.turn);
        cx = self.clip_number(c, cx, y, col, CF::Spin, layer.spin);
        self.clip_number(c, cx, y, col, CF::Seen, p.alpha);
        y += 19.0;
        // Repeating, it pulses.
        if clip.when != When::Span {
            self.clip_number(c, x, y, col, CF::Amount, clip.amount);
            y += 19.0;
        }
        let whens = [
            "stays",
            "pulses every few beats",
            "pulses on notes",
            "pulses on hits",
        ];
        let kind = match clip.when {
            When::Span => 0,
            When::Beats { .. } => 1,
            When::Notes { .. } => 2,
            _ => 3,
        };
        let items: Vec<(String, Btn, bool)> = whens
            .iter()
            .enumerate()
            .map(|(i, w)| (w.to_string(), Btn::Fx(FxAct::When(i as u8)), kind == i))
            .collect();
        y = self.wrap_row(c, x, y, inner, &items);
        cx = self.clip_number(c, x, y, col, CF::FadeIn, clip.fade_in);
        cx = self.clip_number(c, cx, y, col, CF::FadeOut, clip.fade_out);
        if let When::Beats { every } = clip.when {
            self.clip_number(c, cx, y, col, CF::Every, every);
        }
        y += 19.0;
        let items = [
            (
                format!("ease: {}  v", clip.ease.label()),
                Btn::Fx(FxAct::Menu(1)),
                self.fx_drop.is_some_and(|d| d.0 == 1),
            ),
            ("keep after".to_string(), Btn::Fx(FxAct::Keep), clip.keep),
            ("Fit to kiai".to_string(), Btn::Fx(FxAct::FitKiai), false),
            (
                "Copy after  Ctrl+D".to_string(),
                Btn::Fx(FxAct::Duplicate),
                false,
            ),
            ("Delete".to_string(), Btn::Fx(FxAct::Delete), false),
        ];
        y = self.wrap_row(c, x, y, inner, &items);
        let hint = if end {
            "Drag the preview to place it at its end: Shift turns, Ctrl sizes."
        } else {
            "Drag the preview to place it: Shift turns, Ctrl sizes."
        };
        c.text(x, y, 6.0, rgba(DIM, 1.0), &Canvas::fit(6.0, inner, hint));
    }

    /// From the first selected note to the end of the last.
    fn selection_span(&self) -> Option<(f64, f64)> {
        let sel = self.selected_objects();
        let a = sel.iter().map(|o| o.time).fold(f64::INFINITY, f64::min);
        let b = sel
            .iter()
            .map(|o| self.ed.end_of(o))
            .fold(f64::NEG_INFINITY, f64::max);
        (a.is_finite() && b > a).then_some((a, b + 1.0))
    }

    /// Lays preset `i` over `start..end` from row `row` on, and picks its first clip.
    fn place_preset(&mut self, i: usize, start: f64, end: f64, row: Option<u8>) {
        let Some(p) = PRESETS.get(i) else { return };
        let beats = (end - start) / self.ed.beat_at(start);
        let mut clips = (p.make)(start, end, beats);
        let first_picture = self.map_pictures().into_iter().next();
        for c in &mut clips {
            if let Some(l) = c.layer.as_deref_mut()
                && let Show::Picture { file } = &mut l.show
                && file.is_empty()
            {
                match &first_picture {
                    Some(f) => *file = f.clone(),
                    None => {
                        self.status =
                            "drop a .png or .jpg on the editor (or put it in the map's folder) first, then pick Picture again"
                                .into();
                        return;
                    }
                }
            }
        }
        // From the row asked for (the top one else), moved to the nearest rows that are free
        // over that stretch: blocks never sit on top of each other.
        let n = clips.len().clamp(1, LANES as usize) as u8;
        let row = row.unwrap_or(0).min(LANES - n);
        for (k, c) in clips.iter_mut().enumerate() {
            c.lane = (row + k as u8).min(LANES - 1);
        }
        let ids = self.ed.add_clips_free(clips, false);
        if let Some(&id) = ids.first() {
            self.pick_clips(&[id]);
            let lanes: Vec<u8> = ids
                .iter()
                .filter_map(|&k| self.ed.clip(k).map(|c| c.lane))
                .collect();
            if let (Some(&lo), Some(&hi)) = (lanes.iter().min(), lanes.iter().max()) {
                self.show_row(hi);
                self.show_row(lo);
            }
            self.status = format!("{} from {} to {}", p.name, fmt_time(start), fmt_time(end));
            self.mark_stale();
        }
    }

    /// The key (from 1) that records shelf effect `i` live: its place in its tab.
    fn live_key_of(i: usize) -> Option<usize> {
        let p = PRESETS.get(i)?;
        let at = p.shelf.presets().iter().position(|&j| j == i)?;
        (at < 9).then_some(at + 1)
    }

    /// Key `1 + key` on the clip rows: starts recording that effect of the open tab at the
    /// playhead, or ends the one recording there and drops it over that stretch (one undo step).
    pub(super) fn live_key(&mut self, key: usize) {
        let Some(&i) = self.fx_tab.presets().get(key) else {
            return;
        };
        let Some(p) = PRESETS.get(i) else { return };
        let now = self.ed.snapped(self.ed.time);
        if let Some(k) = self.fx_live.iter().position(|(j, _)| *j == i) {
            let (_, from) = self.fx_live.remove(k);
            let (a, b) = (from.min(now), from.max(now));
            let b = if b - a < 1.0 {
                a + self.ed.beat_at(a)
            } else {
                b
            };
            self.place_preset(i, a, b, None);
            self.status = format!(
                "{} recorded from {} to {}",
                p.name,
                fmt_time(a),
                fmt_time(b)
            );
        } else {
            self.fx_live.push((i, now));
            self.status = format!("recording {}: press {} again to end it", p.name, key + 1);
        }
    }

    /// An example for the manual: a few shelf effects and a title laid over the map's first
    /// kiai (or the first 16 beats of notes), one undo step, to play and pick apart.
    pub(super) fn showcase(&mut self) {
        if !self.ed.is_tko() {
            return;
        }
        let first_kiai = self
            .ed
            .chart
            .timing
            .iter()
            .find(|p| p.kiai())
            .and_then(|p| self.ed.kiai_around(p.time));
        let (a, b) = first_kiai.unwrap_or_else(|| {
            let t = self.ed.chart.objects.first().map_or(0.0, |o| o.time);
            (t, t + 16.0 * self.ed.beat_at(t))
        });
        let beat = self.ed.beat_at(a);
        let parts: [(&str, f64, f64); 5] = [
            ("Fly-in text", a, (a + 8.0 * beat).min(b)),
            ("Every 2nd beat", a, b),
            ("Zigzag lane", a, b),
            ("Big note slam", a, b),
            ("Kat flash", a, b),
        ];
        let mut clips = Vec::new();
        for (name, s, e) in parts {
            let Some(p) = PRESETS.iter().find(|p| p.name == name) else {
                continue;
            };
            let beats = (e - s) / self.ed.beat_at(s);
            for mut c in (p.make)(s, e, beats) {
                c.lane = (clips.len() as u8).min(LANES - 1);
                if let Some(l) = c.layer.as_deref_mut() {
                    l.show = Show::Text {
                        text: "Showtime!".into(),
                    };
                }
                clips.push(c);
            }
        }
        let ids = self.ed.add_clips(clips);
        self.pick_clips(&[]);
        if !ids.is_empty() {
            self.set_time(a);
            self.mark_stale();
            self.status = format!(
                "an example from {} to {}: play it with Space, click a block to see how it's made, Ctrl+Z takes it all away",
                fmt_time(a),
                fmt_time(b)
            );
        }
    }

    /// The words of the text block picked.
    pub(super) fn layer_text(&self) -> Option<String> {
        match &self.picked()?.layer.as_deref()?.show {
            Show::Text { text } => Some(text.clone()),
            Show::Picture { .. } => None,
        }
    }

    pub(super) fn layer_text_set(&mut self, typed: &str) {
        let Some(id) = self.fx_sel else { return };
        let typed: String = typed
            .chars()
            .filter(|c| !c.is_control())
            .take(crate::tko::clips::MAX_TEXT)
            .collect();
        let changed = self.ed.set_clip(id, false, |c| {
            if let Some(l) = c.layer.as_deref_mut()
                && let Show::Text { text } = &mut l.show
            {
                *text = typed;
            }
        });
        if changed {
            self.mark_stale();
        }
    }

    /// The pictures in the edited map's folder.
    /// A picture dropped on the editor: copied into the map's folder (pictures travel with the
    /// map from there), and shown by the picked picture block, if one is picked.
    pub fn drop_picture(&mut self, from: &std::path::Path) {
        let Some(dir) = self.ed.path.parent().map(std::path::Path::to_path_buf) else {
            return;
        };
        let Some(name) = from.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            return;
        };
        let to = dir.join(&name);
        let same = to
            .canonicalize()
            .ok()
            .is_some_and(|t| from.canonicalize().ok() == Some(t));
        if !same && let Err(e) = std::fs::copy(from, &to) {
            self.status = format!("couldn't copy {name} into the map's folder: {e}");
            return;
        }
        let picked = self.fx_sel.filter(|&id| {
            self.ed.clips().iter().any(|c| {
                c.id == id
                    && c.layer
                        .as_deref()
                        .is_some_and(|l| matches!(l.show, Show::Picture { .. }))
            })
        });
        match picked {
            Some(id) => {
                self.ed.set_clip(id, false, |c| {
                    if let Some(l) = c.layer.as_deref_mut() {
                        l.show = Show::Picture { file: name.clone() };
                    }
                });
                self.status = format!("{name} is in the map's folder and on the picked block");
                self.mark_stale();
            }
            None => {
                self.status = format!(
                    "{name} is in the map's folder: put a Picture from the Text & pictures tab to show it"
                )
            }
        }
    }

    pub(super) fn map_pictures(&self) -> Vec<String> {
        self.ed
            .path
            .parent()
            .map(crate::pictures::in_folder)
            .unwrap_or_default()
    }

    /// Lays the effect picked on the shelf over `span`, keeping it picked with Shift.
    pub(super) fn place_armed(&mut self, span: Option<(f64, f64)>, why: &str, keep: bool) {
        let Some(i) = self.fx_armed else { return };
        match span {
            Some((a, b)) => {
                self.place_preset(i, a, b, None);
                if !keep {
                    self.fx_armed = None;
                }
            }
            None => self.status = why.into(),
        }
    }

    /// Changes what clip `id` moves, keeping its group of notes and the size of its amount.
    fn set_target(&mut self, id: u32, next: Target) -> bool {
        let group = self.picked().map_or(0, |c| c.group);
        self.ed.set_clip(id, false, |c| {
            let before = c.target();
            if before == Some(next) {
                return;
            }
            c.what = Clip::new(next, c.start, c.end).what;
            c.group = match next {
                Target::Notes(..) if matches!(before, Some(Target::Notes(..))) => group,
                Target::Copy(..) if matches!(before, Some(Target::Copy(..))) => group,
                Target::Copy(..) => 1,
                _ => 0,
            };
            c.amount = rescale_amount(c.amount, before, next);
        })
    }

    pub(super) fn press_fx(&mut self, a: FxAct) {
        // The shelf's own buttons, whatever is picked.
        match a {
            FxAct::Preset(i) => {
                self.pick_clips(&[]);
                self.fx_armed = (self.fx_armed != Some(i)).then_some(i);
                if let Some(p) = self.fx_armed.and_then(|i| PRESETS.get(i)) {
                    self.status = format!("{}: drag on a row to place it", p.name);
                }
                return;
            }
            FxAct::Tab(k) => {
                self.fx_tab = Shelf::ALL[(k as usize).min(Shelf::ALL.len() - 1)];
                return;
            }
            FxAct::Place(k) => {
                let (span, why) = if k == 0 {
                    (
                        self.ed.kiai_around(self.ed.time),
                        "the playhead isn't in a kiai",
                    )
                } else {
                    (self.selection_span(), "select notes on the timeline first")
                };
                self.place_armed(span, why, false);
                return;
            }
            FxAct::Disarm => {
                self.fx_armed = None;
                return;
            }
            FxAct::Back => {
                self.pick_clips(&[]);
                return;
            }
            FxAct::Advanced => {
                self.fx_advanced = !self.fx_advanced;
                return;
            }
            FxAct::Menu(k) => {
                let at = self
                    .buttons
                    .iter()
                    .rev()
                    .find(|(_, b)| *b == Btn::Fx(FxAct::Menu(k)))
                    .map(|(r, _)| *r);
                self.fx_drop = at.map(|r| (k, r));
                return;
            }
            _ => {}
        }
        let Some(id) = self.fx_sel.filter(|&id| self.ed.clip(id).is_some()) else {
            self.pick_clips(&[]);
            return;
        };
        let changed = match a {
            FxAct::Preset(_)
            | FxAct::Tab(_)
            | FxAct::Place(_)
            | FxAct::Disarm
            | FxAct::Back
            | FxAct::Advanced
            | FxAct::Menu(_) => false,
            FxAct::What(_) | FxAct::SetWhat(_)
                if self.picked().is_some_and(|c| c.layer.is_some()) =>
            {
                false
            }
            FxAct::What(d) => {
                let all = targets();
                let now = self.picked().and_then(|c| c.target());
                let at = now
                    .map(listed)
                    .and_then(|t| all.iter().position(|x| *x == t))
                    .unwrap_or(0);
                let next = all[(at as i32 + d).rem_euclid(all.len() as i32) as usize];
                self.set_target(id, next)
            }
            FxAct::SetWhat(k) => match targets().get(k as usize).copied() {
                Some(next) => self.set_target(id, next),
                None => false,
            },
            FxAct::SetEase(k) => match Ease::ALL.get(k as usize).copied() {
                Some(e) if e != Ease::Step => self.ed.set_clip(id, false, |c| c.ease = e),
                _ => false,
            },
            FxAct::CopyNo(k) => self.ed.set_clip(id, false, |c| {
                if matches!(c.target(), Some(Target::Copy(..))) {
                    c.group = k.min(crate::tko::clips::MAX_COPIES as u8 - 1) as u16 + 1;
                }
            }),
            FxAct::Group(d) => {
                let groups = self.ed.gimmick().map_or(0, |g| g.groups.len()) as i32;
                self.ed.set_clip(id, false, |c| {
                    c.group = (c.group as i32 + d).rem_euclid(groups + 1) as u16;
                })
            }
            FxAct::When(k) => self.ed.set_clip(id, false, |c| {
                let notes = match c.when {
                    When::Notes { notes } | When::Hits { notes } => notes,
                    _ => Notes::All,
                };
                c.when = match k {
                    0 => When::Span,
                    1 => When::Beats { every: 1.0 },
                    2 => When::Notes { notes },
                    3 => When::Hits { notes },
                    _ => When::Misses,
                };
            }),
            FxAct::Notes(n) => self.ed.set_clip(id, false, |c| match &mut c.when {
                When::Notes { notes } | When::Hits { notes } => *notes = n,
                _ => {}
            }),
            FxAct::Motion(m) => self.ed.set_clip(id, false, |c| c.motion = m),
            FxAct::Keep => self.ed.set_clip(id, false, |c| c.keep = !c.keep),
            FxAct::FitKiai => match self.picked().and_then(|c| self.ed.kiai_around(c.start)) {
                Some((s, e)) => self.ed.set_clip(id, false, |c| (c.start, c.end) = (s, e)),
                None => {
                    self.status = "this clip doesn't start inside a kiai".into();
                    false
                }
            },
            FxAct::FitSelection => match self.selection_span() {
                Some((s, e)) => self.ed.set_clip(id, false, |c| (c.start, c.end) = (s, e)),
                None => {
                    self.status = "select notes on the timeline first".into();
                    false
                }
            },
            FxAct::Duplicate => {
                // Copies of every block picked, after the stretch they cover; the copies are
                // picked, the copy of this one showing its settings.
                let group = self.fx_group();
                let order: Vec<u32> = self
                    .ed
                    .clips()
                    .iter()
                    .filter(|c| group.contains(&c.id))
                    .map(|c| c.id)
                    .collect();
                let copies = self.ed.duplicate_clips(&order);
                let main = order
                    .iter()
                    .position(|&k| k == id)
                    .and_then(|k| copies.get(k).copied());
                self.fx_many = copies.clone();
                self.fx_sel = main.or(copies.first().copied());
                if let Some(&last) = copies.last()
                    && let Some(t) = self.ed.clip(last).map(|c| c.start)
                    && t > self.view_end()
                {
                    self.status = format!("copied to {}", fmt_time(t));
                }
                !copies.is_empty()
            }
            FxAct::Delete => {
                let group = self.fx_group();
                self.pick_clips(&[]);
                self.ed.delete_clips(&group)
            }
            FxAct::LayerText(text) => {
                let file = self.map_pictures().into_iter().next();
                if !text && file.is_none() {
                    self.status = "put a .png or .jpg in the map's folder first".into();
                    false
                } else {
                    self.ed.set_clip(id, false, |c| {
                        if let Some(l) = c.layer.as_deref_mut() {
                            l.show = match (text, &l.show) {
                                (true, Show::Text { .. }) | (false, Show::Picture { .. }) => {
                                    return;
                                }
                                (true, _) => Show::Text {
                                    text: "Text".into(),
                                },
                                (false, _) => Show::Picture {
                                    file: file.unwrap_or_default(),
                                },
                            };
                        }
                    })
                }
            }
            FxAct::SetPicture(i) => match self.map_pictures().get(i as usize).cloned() {
                Some(name) => self.ed.set_clip(id, false, |c| {
                    if let Some(l) = c.layer.as_deref_mut()
                        && let Show::Picture { file } = &mut l.show
                    {
                        *file = name.clone();
                    }
                }),
                None => false,
            },
            FxAct::Picture(d) => {
                let all = self.map_pictures();
                if all.is_empty() {
                    self.status = "put a .png or .jpg in the map's folder first".into();
                    false
                } else {
                    self.ed.set_clip(id, false, |c| {
                        if let Some(l) = c.layer.as_deref_mut()
                            && let Show::Picture { file } = &mut l.show
                        {
                            let at = all.iter().position(|f| f == file).map_or(-1, |i| i as i32);
                            let next = (at + d).rem_euclid(all.len() as i32) as usize;
                            *file = all[next].clone();
                        }
                    })
                }
            }
            FxAct::EndPose(end) => {
                self.fx_end = end;
                // Editing the end pose makes it its own.
                end && self.ed.set_clip(id, false, |c| {
                    if let Some(l) = c.layer.as_deref_mut()
                        && l.to.is_none()
                    {
                        l.to = Some(l.from);
                    }
                })
            }
            FxAct::Moves => {
                let moves = self
                    .picked()
                    .is_some_and(|c| c.layer.as_ref().is_some_and(|l| l.to.is_some()));
                if moves {
                    self.fx_end = false;
                }
                self.ed.set_clip(id, false, |c| {
                    if let Some(l) = c.layer.as_deref_mut() {
                        l.to = if moves { None } else { Some(l.from) };
                    }
                })
            }
            FxAct::Behind => self.ed.set_clip(id, false, |c| {
                if let Some(l) = c.layer.as_deref_mut() {
                    l.behind = !l.behind;
                }
            }),
            FxAct::Colour => self.ed.set_clip(id, false, |c| {
                if let Some(l) = c.layer.as_deref_mut() {
                    let at = LAYER_COLOURS
                        .iter()
                        .position(|&k| k == l.colour)
                        .unwrap_or(0);
                    l.colour = LAYER_COLOURS[(at + 1) % LAYER_COLOURS.len()];
                }
            }),
        };
        if changed {
            self.mark_stale();
        }
    }

    pub(super) fn clip_change(&mut self, f: CF, dir: f64, keys: Keys) {
        let Some(clip) = self.picked().cloned() else {
            return;
        };
        let k = if keys.shift {
            10.0
        } else if keys.alt {
            0.1
        } else {
            1.0
        };
        let step = match f {
            CF::Amount => clip.target().map_or(1.0, amount_step),
            CF::Every | CF::Length | CF::FadeIn | CF::FadeOut => 0.25,
            CF::X | CF::Y | CF::Size => 2.0,
            CF::Turn => 5.0,
            CF::Spin => 15.0,
            CF::Seen => 0.05,
        } * k;
        let now = self.clip_field(&clip, f);
        let v = ((now + dir * step) * 1000.0).round() / 1000.0;
        self.clip_set(f, v);
    }

    fn clip_field(&self, c: &Clip, f: CF) -> f64 {
        match f {
            CF::Amount => c.amount,
            CF::Every => match c.when {
                When::Beats { every } => every,
                _ => 1.0,
            },
            CF::Length => c.length,
            CF::FadeIn => c.fade_in,
            CF::FadeOut => c.fade_out,
            CF::Spin => c.layer.as_deref().map_or(0.0, |l| l.spin),
            CF::X | CF::Y | CF::Size | CF::Turn | CF::Seen => {
                let Some(l) = c.layer.as_deref() else {
                    return 0.0;
                };
                let p = if self.fx_end {
                    l.to.unwrap_or(l.from)
                } else {
                    l.from
                };
                match f {
                    CF::X => p.x,
                    CF::Y => p.y,
                    CF::Size => p.size,
                    CF::Turn => p.turn,
                    _ => p.alpha,
                }
            }
        }
    }

    pub(super) fn clip_set(&mut self, f: CF, v: f64) {
        let Some(id) = self.fx_sel else { return };
        if !v.is_finite() {
            return;
        }
        let end = self.fx_end;
        let changed = self.ed.set_clip(id, false, |c| match f {
            CF::Amount => c.amount = v,
            CF::Every => {
                c.when = When::Beats {
                    every: v.max(1.0 / 16.0),
                }
            }
            CF::Length => c.length = v.max(1.0 / 64.0),
            CF::FadeIn => c.fade_in = v.max(0.0),
            CF::FadeOut => c.fade_out = v.max(0.0),
            CF::Spin => {
                if let Some(l) = c.layer.as_deref_mut() {
                    l.spin = v.clamp(-MAX_SPIN, MAX_SPIN);
                }
            }
            CF::X | CF::Y | CF::Size | CF::Turn | CF::Seen => {
                if let Some(l) = c.layer.as_deref_mut() {
                    let p = pose_mut(l, end);
                    match f {
                        CF::X => p.x = v,
                        CF::Y => p.y = v,
                        CF::Size => p.size = v.max(1.0),
                        CF::Turn => p.turn = v,
                        _ => p.alpha = v.clamp(0.0, 1.0),
                    }
                }
            }
        });
        if changed {
            self.mark_stale();
        }
    }

    // ---- the rows ----

    /// Whether `y` is on the strip of motion keys above the rows.
    fn in_motion_strip(&self, y: f32) -> bool {
        let top = self.layout.graph[1] + STRIP_Y;
        y >= top && y < top + STRIP_H
    }

    pub(super) fn row_h(&self) -> f32 {
        let [_, _, _, h] = self.layout.graph;
        (h - ROWS_TOP - 2.0) / ROWS as f32
    }

    /// The rows there are: one past the last used, so a free one waits at the bottom, and at
    /// least what fits.
    pub(super) fn clip_row_count(&self) -> u8 {
        let used = self
            .ed
            .clips()
            .iter()
            .map(|c| c.lane + 1)
            .max()
            .unwrap_or(0);
        (used.saturating_add(1)).clamp(ROWS, LANES)
    }

    /// Ctrl+C (`cut`: Ctrl+X) on the blocks picked: kept to paste with Ctrl+V, here or in
    /// another map.
    pub(super) fn copy_clips(&mut self, cut: bool) {
        let group = self.fx_group();
        let picked: Vec<Clip> = self
            .ed
            .clips()
            .iter()
            .filter(|c| group.contains(&c.id))
            .cloned()
            .collect();
        if picked.is_empty() {
            self.status = "pick blocks first".into();
            return;
        }
        let n = picked.len();
        CLIPBOARD.with(|b| *b.borrow_mut() = picked);
        let what = format!("{n} block{}", if n == 1 { "" } else { "s" });
        if cut {
            self.pick_clips(&[]);
            if self.ed.delete_clips(&group) {
                self.mark_stale();
            }
            self.status = format!("cut {what}");
        } else {
            self.status = format!("copied {what}");
        }
    }

    /// Whether Ctrl+C or Ctrl+X kept blocks to paste.
    pub(super) fn has_copied_clips() -> bool {
        CLIPBOARD.with(|b| !b.borrow().is_empty())
    }

    /// Ctrl+V: the blocks copied, from the playhead on with their spacing and rows (the nearest
    /// free rows when those are taken there), picked; the playhead goes to where they end, so
    /// pasting again lays the next copy after them, as in Ableton.
    pub(super) fn paste_copied_clips(&mut self) {
        let copied = CLIPBOARD.with(|b| b.borrow().clone());
        let at = self.ed.snapped(self.ed.time.max(0.0));
        let ids = self.ed.paste_clips(&copied, at);
        if ids.is_empty() {
            self.status = "nothing to paste: copy blocks with Ctrl+C first".into();
            return;
        }
        self.pick_clips(&ids);
        let placed: Vec<&Clip> = ids.iter().filter_map(|&k| self.ed.clip(k)).collect();
        let end = placed.iter().map(|c| c.end).fold(at, f64::max);
        let rows = placed.iter().map(|c| c.lane);
        let (lo, hi) = (rows.clone().min(), rows.max());
        if let (Some(lo), Some(hi)) = (lo, hi) {
            self.show_row(hi);
            self.show_row(lo);
        }
        let n = ids.len();
        self.status = format!(
            "pasted {n} block{} at {}",
            if n == 1 { "" } else { "s" },
            fmt_time(at)
        );
        self.set_time(end);
        self.mark_stale();
    }

    /// Ctrl+A: every block picked.
    pub(super) fn pick_all_clips(&mut self) {
        let all: Vec<u32> = self.ed.clips().iter().map(|c| c.id).collect();
        self.status = format!("picked {} blocks", all.len());
        self.pick_clips(&all);
    }

    /// Scrolls the rows so row `row` shows.
    fn show_row(&mut self, row: u8) {
        if row < self.fx_top {
            self.fx_top = row;
        } else if row >= self.fx_top + ROWS {
            self.fx_top = row + 1 - ROWS;
        }
    }

    /// The wheel over the clip rows scrolls them when there are more than fit. False when it
    /// isn't theirs.
    pub(super) fn wheel_clip_rows(&mut self, notches: i32, keys: Keys) -> bool {
        let (mx, my) = self.mouse;
        let max = self.clip_row_count().saturating_sub(ROWS);
        if !self.clip_mode()
            || keys.ctrl
            || keys.alt
            || max == 0
            || !inside(self.layout.graph, mx, my)
            || my < self.layout.graph[1] + ROWS_TOP
        {
            return false;
        }
        let per = if keys.shift { ROWS as i32 } else { 1 };
        self.fx_top = (self.fx_top as i32 - notches * per).clamp(0, max as i32) as u8;
        true
    }

    pub(super) fn row_y(&self, row: u8) -> f32 {
        self.layout.graph[1] + ROWS_TOP + (row as f32 - self.fx_top as f32) * self.row_h()
    }

    /// The row at `y`; above or under the rows shown, the ones scrolled away there.
    fn row_at(&self, y: f32) -> u8 {
        let k = ((y - self.layout.graph[1] - ROWS_TOP) / self.row_h()).floor() as i32;
        (self.fx_top as i32 + k).clamp(0, LANES as i32 - 1) as u8
    }

    /// The clip at `x, y` and the part of it there. A short block's ends are its outer quarters
    /// and, where no other block is, a little way past it, so it can still be resized.
    fn clip_at(&self, x: f32, y: f32) -> Option<(u32, Part)> {
        let row = self.row_at(y);
        let on_row = || self.ed.clips().iter().rev().filter(move |c| c.lane == row);
        let on = on_row().find_map(|c| {
            let (a, b) = (self.x_of(c.start), self.x_of(c.end));
            if x < a - 1.0 || x > b + 1.0 {
                return None;
            }
            let edge = EDGE.min((b - a) * 0.25);
            let part = if x <= a + edge {
                Part::Start
            } else if x >= b - edge {
                Part::End
            } else {
                Part::Body
            };
            Some((c.id, part))
        });
        on.or_else(|| {
            on_row().find_map(|c| {
                let (a, b) = (self.x_of(c.start), self.x_of(c.end));
                if x >= a - EDGE && x < a {
                    Some((c.id, Part::Start))
                } else if x > b && x <= b + EDGE {
                    Some((c.id, Part::End))
                } else {
                    None
                }
            })
        })
    }

    pub(super) fn draw_clip_rows(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.graph;
        let count = self.clip_row_count();
        self.fx_top = self.fx_top.min(count.saturating_sub(ROWS));
        let shown = self.fx_top..self.fx_top + ROWS;
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x101016, 0.95),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        let start = c.shapes.len();
        let rh = self.row_h();
        let (from, to) = (self.view_start, self.view_end());
        // Kiai, bars and rows behind the clips.
        if let Some(span) = self.ed.kiai_around(self.ed.time) {
            let (a, b) = (self.x_of(span.0).max(x), self.x_of(span.1).min(x + w));
            if b > a {
                c.rect(
                    a,
                    y + ROWS_TOP,
                    b - a,
                    h - ROWS_TOP - 1.0,
                    rgba(0xFFAA00, 0.05),
                );
            }
        }
        for (t, kind) in self.grid() {
            if kind == 2 {
                c.rect(
                    self.x_of(t),
                    y + ROWS_TOP,
                    1.0,
                    h - ROWS_TOP - 1.0,
                    rgba(0xffffff, 0.08),
                );
            }
        }
        for r in shown.clone() {
            let ry = self.row_y(r);
            if r % 2 == 1 {
                c.rect(x + 1.0, ry, w - 2.0, rh, rgba(0xffffff, 0.02));
            }
        }
        // The clips.
        let reds = self.ed.reds();
        let beat = |t: f64| crate::editor::clips::beat_in(&reds, t);
        let group = self.fx_group();
        let clips: Vec<Clip> = self
            .ed
            .clips()
            .iter()
            .filter(|cl| cl.end >= from && cl.start <= to && shown.contains(&cl.lane))
            .cloned()
            .collect();
        for cl in &clips {
            let (a, b) = (self.x_of(cl.start), self.x_of(cl.end));
            let ry = self.row_y(cl.lane) + 1.0;
            let bh = rh - 2.0;
            let col = cl.target().map_or(0x888899, target_colour);
            let on = group.contains(&cl.id);
            let main = self.fx_sel == Some(cl.id);
            let bw = (b - a).max(2.0);
            c.round_rect(
                [a, ry],
                [bw, bh],
                // A short block (zoomed far out) is a sliver, not a pill wider than itself.
                3.0f32.min(bw / 2.0),
                rgba(col, if on { 0.42 } else { 0.24 }),
                rgba(
                    if on { 0xffffff } else { col },
                    if main {
                        1.0
                    } else if on {
                        0.6
                    } else {
                        0.7
                    },
                ),
                if main { 1.5 } else { 1.0 },
            );
            // What it does over its length, as the game will.
            let moments = self.ed.clip_moments(cl, &reds);
            let peak = cl.amount.abs().max(1e-9);
            let steps = if cl.motion == Motion::Step && cl.when != When::Span {
                moments.len().max(1) as f64
            } else {
                1.0
            };
            // Inside the block's border and clear of its rounded corners, every column whole.
            let mut px = (a + 2.0).max(x);
            let stop = (b - 2.0).min(x + w);
            while px < stop {
                let t = self.t_at(px);
                let v = cl.value(t, &moments, beat) / (peak * steps);
                let hh = (v.abs() as f32).min(1.0) * (bh - 4.0) * 0.9;
                let cw = (stop - px).min(1.5);
                if hh > 0.3 {
                    c.rect(px, ry + bh - 2.0 - hh, cw, hh, rgba(col, 0.55));
                }
                px += 1.5;
            }
            if b - a > 24.0 {
                let label = Canvas::fit(5.5, b - a - 6.0, &describe(cl));
                c.text(a.max(x) + 3.0, ry + 2.0, 5.5, rgba(0xffffff, 0.95), &label);
            }
        }
        self.draw_motion_strip(c, y + STRIP_Y, STRIP_H);
        // A clip being drawn.
        if let Drag::FxNew {
            row,
            from: t0,
            to: t1,
        } = self.drag
        {
            let (a, b) = (self.x_of(t0.min(t1)), self.x_of(t0.max(t1)));
            c.round_rect(
                [a, self.row_y(row) + 1.0],
                [(b - a).max(1.0), rh - 2.0],
                3.0,
                rgba(0xffffff, 0.12),
                rgba(0xffffff, 0.7),
                1.0,
            );
        }
        // A box picking clips.
        if let Drag::FxBox { t0, y0, t1, y1, .. } = self.drag {
            let (a, b) = (self.x_of(t0.min(t1)), self.x_of(t0.max(t1)));
            let (top, bottom) = (y0.min(y1), y0.max(y1));
            c.rect(a, top, b - a, bottom - top, rgba(accent(), 0.12));
            for (rx, ry, rw, rhh) in [
                (a, top, b - a, 1.0),
                (a, bottom - 1.0, b - a, 1.0),
                (a, top, 1.0, bottom - top),
                (b - 1.0, top, 1.0, bottom - top),
            ] {
                c.rect(rx, ry, rw, rhh, rgba(accent(), 0.8));
            }
        }
        // Effects being recorded live, growing with the playhead.
        for (k, &(i, from)) in self.fx_live.iter().enumerate() {
            let (a, b) = (
                self.x_of(from.min(self.ed.time)),
                self.x_of(from.max(self.ed.time)),
            );
            let ry = self.row_y((self.fx_top + ROWS - 1).saturating_sub(k as u8)) + 1.0;
            c.round_rect(
                [a, ry],
                [(b - a).max(2.0), rh - 2.0],
                3.0,
                rgba(0xFF4D5E, 0.25),
                rgba(0xFF4D5E, 0.9),
                1.0,
            );
            let name = PRESETS.get(i).map_or("", |p| p.name);
            let key = Self::live_key_of(i).map_or(String::new(), |k| format!("{k}  "));
            let label = Canvas::fit(5.5, (b - a - 6.0).max(40.0), &format!("REC {key}{name}"));
            c.text(a + 3.0, ry + 2.0, 5.5, rgba(0xffffff, 0.95), &label);
        }
        let px = self.x_of(self.ed.time);
        c.rect(px - 0.75, y + 1.0, 1.5, h - 2.0, rgba(0xffffff, 0.8));
        // The scrollbar, when the rows scroll.
        if count > ROWS {
            let (top, span) = (y + ROWS_TOP, h - ROWS_TOP - 2.0);
            let thumb = span * ROWS as f32 / count as f32;
            let at = top + span * self.fx_top as f32 / count as f32;
            c.rect(x + w - 4.0, top, 3.0, span, rgba(0xffffff, 0.05));
            c.round_rect(
                [x + w - 4.0, at],
                [3.0, thumb],
                1.5,
                rgba(0xffffff, 0.35),
                rgba(0x000000, 0.0),
                0.0,
            );
        }
        c.clip_since(start, x, y, w, h);
        let what = match self.fx_armed.and_then(|i| PRESETS.get(i)) {
            Some(p) => format!(
                "placing {}: drag on a row, or click one for a bar  (Esc puts it back)",
                p.name
            ),
            None => "pick an effect on the shelf to place it   drag on empty rows to pick blocks, Shift adds   drag a block to move, its ends to resize, right click deletes".into(),
        };
        let rows = if count > ROWS {
            format!(
                "   rows {}-{} of {count}, the wheel scrolls",
                self.fx_top + 1,
                self.fx_top + ROWS
            )
        } else {
            String::new()
        };
        let label = Canvas::fit(5.5, w - 8.0, &format!("EFFECT CLIPS   {what}{rows}"));
        c.text(x + 4.0, y + 3.0, 5.5, rgba(0x6a6a80, 1.0), &label);
    }

    /// The first move of a Ctrl drag: copies of the blocks in `fx_orig` where they are, as the
    /// first step of the drag (so the copy and its move undo together); the copies are picked
    /// and dragged from then on. Returns the copy of `id`.
    fn start_copy_drag(&mut self, id: u32, grab: f64) -> Option<u32> {
        let from: Vec<u32> = self.fx_orig.iter().map(|o| o.0).collect();
        let copies: Vec<Clip> = from
            .iter()
            .filter_map(|&k| self.ed.clip(k).cloned())
            .collect();
        let ids = self.ed.add_clips_as(copies, true);
        if ids.len() != from.len() {
            return None;
        }
        let main = from.iter().position(|&k| k == id).map(|k| ids[k])?;
        self.fx_orig = self.clip_places(&ids);
        self.fx_many = ids;
        self.fx_sel = Some(main);
        self.drag = Drag::FxMove {
            id: main,
            grab,
            copy: false,
        };
        self.status = format!(
            "copying {} block{}",
            from.len(),
            if from.len() == 1 { "" } else { "s" }
        );
        Some(main)
    }

    /// Where clips `ids` are: id, start, end, row.
    fn clip_places(&self, ids: &[u32]) -> Vec<(u32, f64, f64, u8)> {
        ids.iter()
            .filter_map(|&id| self.ed.clip(id).map(|c| (id, c.start, c.end, c.lane)))
            .collect()
    }

    pub(super) fn clip_rows_down(&mut self, x: f32, y: f32, keys: Keys) {
        self.fx_click = None;
        if self.in_motion_strip(y) || y < self.layout.graph[1] + ROWS_TOP {
            if !self.motion_strip_down(x) {
                self.drag = Drag::Scrub;
                let t = self.t_at(x);
                self.set_time(t);
            }
            return;
        }
        let add = keys.shift || keys.ctrl;
        match self.clip_at(x, y) {
            // Ctrl (or Ctrl+Shift) on a block, as in Ableton: dragging drags copies of it (of
            // all the blocks picked, when it is one of them); a click takes it in or out.
            Some((id, Part::Body)) if keys.ctrl => {
                let Some(cl) = self.ed.clip(id) else { return };
                let grab = self.t_at(x) - cl.start;
                let group = self.fx_group();
                let copied = if group.contains(&id) { group } else { vec![id] };
                self.fx_orig = self.clip_places(&copied);
                self.fx_click = Some(id);
                self.drag = Drag::FxMove {
                    id,
                    grab,
                    copy: true,
                };
            }
            Some((id, part)) => {
                let mut group = self.fx_group();
                if add {
                    // Shift or Ctrl: in or out of the blocks picked.
                    if let Some(k) = group.iter().position(|&g| g == id) {
                        group.remove(k);
                        self.fx_sel = group.last().copied();
                        self.fx_many = group;
                        return;
                    }
                    group.push(id);
                    self.fx_many = group;
                    self.fx_sel = Some(id);
                } else if group.len() > 1 && group.contains(&id) {
                    // One of several picked: they move together, and a click picks it alone.
                    self.fx_sel = Some(id);
                    self.fx_click = Some(id);
                } else {
                    self.pick_clips(&[id]);
                }
                let Some(cl) = self.ed.clip(id) else { return };
                self.drag = match part {
                    Part::Body => {
                        let grab = self.t_at(x) - cl.start;
                        self.fx_orig = self.clip_places(&self.fx_group());
                        Drag::FxMove {
                            id,
                            grab,
                            copy: false,
                        }
                    }
                    Part::Start => Drag::FxEdge { id, end: false },
                    Part::End => Drag::FxEdge { id, end: true },
                };
            }
            None if self.fx_armed.is_some() => {
                let t = self.ed.snapped(self.t_at(x));
                self.drag = Drag::FxNew {
                    row: self.row_at(y),
                    from: t,
                    to: t,
                };
            }
            None => {
                let t = self.t_at(x);
                self.drag = Drag::FxBox {
                    t0: t,
                    y0: y,
                    t1: t,
                    y1: y,
                    add,
                };
            }
        }
    }

    pub(super) fn clip_rows_drag(&mut self, x: f32, y: f32) {
        let t = self.ed.snapped(self.t_at(x).max(0.0));
        match self.drag {
            Drag::FxNew { row, from, .. } => self.drag = Drag::FxNew { row, from, to: t },
            Drag::FxMove { id, grab, copy } => {
                let Some(&(_, s0, _, r0)) = self.fx_orig.iter().find(|o| o.0 == id) else {
                    return;
                };
                let row = self.row_at(y).min(self.clip_row_count());
                let start = self.ed.snapped((self.t_at(x) - grab).max(0.0));
                if start == s0 && row == r0 {
                    return;
                }
                self.fx_click = None;
                let id = if copy {
                    match self.start_copy_drag(id, grab) {
                        Some(copy) => copy,
                        None => return,
                    }
                } else {
                    id
                };
                let from = self.fx_orig.clone();
                if self
                    .ed
                    .move_clips(&from, start - s0, row as i32 - r0 as i32, true)
                {
                    self.mark_stale();
                }
                if let Some(lane) = self.ed.clip(id).map(|c| c.lane) {
                    self.show_row(lane);
                }
            }
            Drag::FxEdge { id, end } => {
                let min = self.ed.beat_at(t) / self.ed.snap.max(1) as f64;
                // An end stops at the next block on the row instead of going over it.
                let (lo, hi) = self.ed.clip_room(id);
                if self.ed.set_clip(id, true, |c| {
                    if end {
                        c.end = t.max(c.start + min).min(hi.max(c.end));
                    } else {
                        c.start = t.min(c.end - min).max(lo.min(c.start));
                    }
                }) {
                    self.mark_stale();
                }
            }
            Drag::FxBox { t0, y0, add, .. } => {
                self.drag = Drag::FxBox {
                    t0,
                    y0,
                    t1: self.t_at(x),
                    y1: y,
                    add,
                };
            }
            _ => {}
        }
    }

    pub(super) fn clip_rows_up(&mut self, drag: Drag, x: f32, keys: Keys) {
        match drag {
            Drag::FxNew { row, from, to } => {
                let Some(armed) = self.fx_armed else { return };
                let (mut a, mut b) = (from.min(to), from.max(to));
                if b - a < 1.0 || (self.x_of(b) - self.x_of(a)) < 4.0 {
                    // A click: one bar from there.
                    a = from;
                    let meter = crate::editor::timing::active_red(&self.ed.chart.timing, a)
                        .map_or(4, |p| p.meter.max(1)) as f64;
                    b = a + meter * self.ed.beat_at(a);
                }
                self.place_preset(armed, a, b, Some(row));
                if !keys.shift {
                    self.fx_armed = None;
                }
            }
            Drag::FxMove { id, copy: true, .. } => {
                // Ctrl and a click, no drag: in or out of the blocks picked.
                self.fx_orig.clear();
                if self.fx_click.take() == Some(id) {
                    let mut group = self.fx_group();
                    if let Some(k) = group.iter().position(|&g| g == id) {
                        group.remove(k);
                        self.fx_sel = group.last().copied();
                        self.fx_many = group;
                    } else {
                        group.push(id);
                        self.fx_many = group;
                        self.fx_sel = Some(id);
                    }
                }
            }
            Drag::FxMove { id, .. } => {
                self.ed.end_drag();
                self.mark_stale();
                if self.fx_click.take() == Some(id) {
                    self.pick_clips(&[id]);
                }
                self.fx_orig.clear();
            }
            Drag::FxEdge { .. } => {
                self.ed.end_drag();
                self.mark_stale();
            }
            Drag::FxBox {
                t0,
                y0,
                t1,
                y1,
                add,
            } => {
                let click = (self.x_of(t1) - self.x_of(t0)).abs() < 3.0 && (y1 - y0).abs() < 3.0;
                if click {
                    // A click on an empty row: nothing picked, the playhead there.
                    if !add {
                        self.pick_clips(&[]);
                        let t = self.ed.snapped(self.t_at(x));
                        self.set_time(t);
                    }
                    return;
                }
                let (a, b) = (t0.min(t1), t0.max(t1));
                let top = self.row_at(y0.min(y1));
                let bottom = self.row_at(y0.max(y1));
                let hit: Vec<u32> = self
                    .ed
                    .clips()
                    .iter()
                    .filter(|c| c.lane >= top && c.lane <= bottom && c.end > a && c.start < b)
                    .map(|c| c.id)
                    .collect();
                let mut ids = if add { self.fx_group() } else { Vec::new() };
                for id in hit {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                let main = self.fx_sel.filter(|s| add && ids.contains(s));
                self.pick_clips(&ids);
                if main.is_some() {
                    self.fx_sel = main;
                }
                self.status = match ids.len() {
                    0 => "no blocks there".into(),
                    1 => "1 block picked".into(),
                    n => format!("{n} blocks picked: drag one to move them all, Ctrl+D copies"),
                };
            }
            _ => {}
        }
    }

    /// A right click on the rows: deletes the clip there. False when it isn't on the rows.
    pub(super) fn clip_rows_right(&mut self, x: f32, y: f32) -> bool {
        if !inside(self.layout.graph, x, y) {
            return false;
        }
        if self.in_motion_strip(y) {
            self.motion_strip_right(x);
            return true;
        }
        if let Some((id, _)) = self.clip_at(x, y)
            && self.ed.delete_clip(id)
        {
            self.fx_many.retain(|&k| k != id);
            if self.fx_sel == Some(id) {
                self.fx_sel = self.fx_many.first().copied();
            }
            self.mark_stale();
        }
        true
    }

    /// The pointer over the rows: resize arrows on a clip's ends, a hand on its body.
    pub(super) fn clip_cursor(&self, x: f32, y: f32) -> Option<winit::window::CursorIcon> {
        use winit::window::CursorIcon as I;
        if !self.clip_mode() || !inside(self.layout.graph, x, y) {
            return None;
        }
        Some(match self.clip_at(x, y) {
            Some((_, Part::Start | Part::End)) => I::EwResize,
            Some((_, Part::Body)) => I::Grab,
            None if self.fx_armed.is_some() => I::Crosshair,
            None => I::Default,
        })
    }

    // ---- the dropdowns ----

    /// The open dropdown of the clip picked: everything it can move in columns by kind, or its
    /// eases.
    pub(super) fn draw_fx_drop(&mut self, c: &mut Canvas) {
        let Some((kind, at)) = self.fx_drop else {
            return;
        };
        let Some(clip) = self.picked().cloned().filter(|_| self.clip_mode()) else {
            self.fx_drop = None;
            return;
        };
        if kind == 1 {
            return self.draw_ease_drop(c, at, clip.ease);
        }
        if kind == 2 {
            let now = match clip.layer.as_deref().map(|l| &l.show) {
                Some(Show::Picture { file }) => file.clone(),
                _ => String::new(),
            };
            return self.draw_picture_drop(c, at, &now);
        }
        let now = clip.target().map(listed);
        let all = targets();
        let cols = target_columns();
        let short = |t: Target| -> String {
            let l = target_label(t);
            match l.split_once(": ") {
                Some((_, rest)) => {
                    let mut ch = rest.chars();
                    ch.next()
                        .map(|f| f.to_uppercase().chain(ch).collect())
                        .unwrap_or_default()
                }
                None => l,
            }
        };
        let cw = cols
            .iter()
            .flat_map(|(head, _, ts)| {
                ts.iter()
                    .map(|t| Canvas::text_width(7.0, &short(*t)))
                    .chain([Canvas::text_width(6.0, head)])
            })
            .fold(60.0, f32::max)
            + 16.0;
        let tall = cols.iter().map(|(_, _, ts)| ts.len()).max().unwrap_or(1);
        let about_h = 30.0;
        let w = (cw * cols.len() as f32 + 8.0).min(c.width - 8.0);
        let cw = (w - 8.0) / cols.len() as f32;
        let h = 16.0 + tall as f32 * 14.0 + about_h;
        let x = at[0].min(c.width - w - 4.0).max(4.0);
        let y = if at[1] + at[3] + h < 476.0 {
            at[1] + at[3] + 1.0
        } else {
            (at[1] - h - 1.0).max(4.0)
        };
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x15151d, 1.0),
            rgba(0x5a5a78, 1.0),
            1.0,
        );
        let mut about = None;
        for (k, (head, col, ts)) in cols.iter().enumerate() {
            let cx = x + 4.0 + k as f32 * cw;
            c.rect(cx + 2.0, y + 12.0, cw - 4.0, 1.0, rgba(*col, 0.8));
            c.text(cx + 4.0, y + 4.0, 6.0, rgba(*col, 1.0), head);
            for (i, t) in ts.iter().enumerate() {
                let r = [cx + 1.0, y + 15.0 + i as f32 * 14.0, cw - 2.0, 13.0];
                let on = now == Some(*t);
                let hover = c.hovered(r[0], r[1], r[2], r[3]);
                if hover {
                    about = Some(*t);
                }
                if on || hover {
                    c.rect(
                        r[0],
                        r[1],
                        r[2],
                        r[3],
                        rgba(if on { 0x4a221b } else { 0x2c2c3c }, 1.0),
                    );
                }
                let label = Canvas::fit(7.0, r[2] - 8.0, &short(*t));
                c.text(
                    r[0] + 5.0,
                    r[1] + 3.0,
                    7.0,
                    rgba(if on { 0xffffff } else { 0xd8d8e8 }, 1.0),
                    &label,
                );
                if let Some(idx) = all.iter().position(|a| a == t) {
                    self.buttons.push((r, Btn::Fx(FxAct::SetWhat(idx as u16))));
                }
            }
        }
        // What the one under the pointer (else the one in use) does.
        let ay = y + h - about_h + 2.0;
        c.rect(x + 4.0, ay - 2.0, w - 8.0, 1.0, rgba(0x2a2a38, 1.0));
        if let Some(t) = about.or(now) {
            let text = format!("{}: {}", target_label(t), target_about(t));
            for (i, line) in wrap(6.0, w - 14.0, &text).iter().take(3).enumerate() {
                c.text(
                    x + 7.0,
                    ay + 1.0 + i as f32 * 8.5,
                    6.0,
                    rgba(DIM, 1.0),
                    line,
                );
            }
        }
    }

    /// Every picture in the map's folder, in columns when there are many, the one under the
    /// pointer shown large beside the list.
    fn draw_picture_drop(&mut self, c: &mut Canvas, at: [f32; 4], now: &str) {
        const ROW: f32 = 14.0;
        let all = self.map_pictures();
        if all.is_empty() {
            self.status =
                "no pictures in the map's folder: drop a .png or .jpg on the editor".into();
            self.fx_drop = None;
            return;
        }
        let below = 476.0 - (at[1] + at[3] + 1.0);
        let above = at[1] - 5.0;
        let room = below.max(above).max(ROW * 4.0);
        let per_col = ((room - 4.0) / ROW).floor().max(1.0) as usize;
        let cols = all.len().div_ceil(per_col);
        let rows = all.len().min(per_col);
        let col_w = all
            .iter()
            .map(|f| Canvas::text_width(7.0, f) + 20.0)
            .fold(90.0, f32::max)
            .min(220.0);
        let (w, h) = (cols as f32 * col_w + 4.0, rows as f32 * ROW + 4.0);
        let x = at[0].min(c.width - w - 4.0).max(4.0);
        let y = if h <= below {
            at[1] + at[3] + 1.0
        } else {
            (at[1] - h - 1.0).max(4.0)
        };
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x15151d, 1.0),
            rgba(0x5a5a78, 1.0),
            1.0,
        );
        let mut hovered = None;
        for (i, f) in all.iter().enumerate() {
            let (col, row) = (i / per_col, i % per_col);
            let r = [
                x + 2.0 + col as f32 * col_w,
                y + 2.0 + row as f32 * ROW,
                col_w - 2.0,
                ROW - 1.0,
            ];
            let on = f == now;
            let hover = c.hovered(r[0], r[1], r[2], r[3]);
            if hover {
                hovered = Some((f.clone(), r));
            }
            if on || hover {
                c.rect(
                    r[0],
                    r[1],
                    r[2],
                    r[3],
                    rgba(if on { 0x4a221b } else { 0x2c2c3c }, 1.0),
                );
            }
            c.text(
                r[0] + 6.0,
                r[1] + 3.0,
                7.0,
                rgba(if on { 0xffffff } else { 0xd8d8e8 }, 1.0),
                &Canvas::fit(7.0, col_w - 12.0, f),
            );
            self.buttons.push((
                r,
                Btn::Fx(FxAct::SetPicture(i.min(u16::MAX as usize) as u16)),
            ));
        }
        // A look at the one under the pointer.
        if let Some((f, r)) = hovered {
            let side = 96.0;
            let px = if x + w + side + 8.0 < c.width {
                x + w + 4.0
            } else {
                (x - side - 4.0).max(4.0)
            };
            let py = (r[1] - side / 2.0).clamp(4.0, 476.0 - side);
            c.round_rect(
                [px, py],
                [side, side],
                3.0,
                rgba(0x0b0b10, 1.0),
                rgba(0x5a5a78, 1.0),
                1.0,
            );
            let (cx, cy) = (px + side / 2.0, py + side / 2.0);
            // Kept inside the box whatever its shape.
            let aspect = c
                .picture_slots
                .get(&f.to_ascii_lowercase())
                .map_or(1.0, |s| s.1)
                .max(0.01);
            let h = (side - 12.0) / aspect.max(1.0);
            if !c.picture(&f, cx, cy, h, 0.0, rgba(0xffffff, 1.0)) {
                c.text_centered(cx, cy - 3.0, 6.0, rgba(0x8a8aa0, 1.0), "loading...");
            }
        }
    }

    fn draw_ease_drop(&mut self, c: &mut Canvas, at: [f32; 4], now: Ease) {
        // Step is for keyframes: a clip with it would do nothing.
        let eases: Vec<(usize, Ease)> = Ease::ALL
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, e)| *e != Ease::Step)
            .collect();
        let w = eases
            .iter()
            .map(|(_, e)| Canvas::text_width(7.0, e.label()) + 26.0)
            .fold(at[2], f32::max);
        let h = eases.len() as f32 * 14.0 + 4.0;
        let x = at[0].min(c.width - w - 4.0);
        let y = if at[1] + at[3] + h < 476.0 {
            at[1] + at[3] + 1.0
        } else {
            (at[1] - h - 1.0).max(4.0)
        };
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x15151d, 1.0),
            rgba(0x5a5a78, 1.0),
            1.0,
        );
        for (row, (i, e)) in eases.into_iter().enumerate() {
            let r = [x + 2.0, y + 2.0 + row as f32 * 14.0, w - 4.0, 13.0];
            let on = e == now;
            let hover = c.hovered(r[0], r[1], r[2], r[3]);
            if on || hover {
                c.rect(
                    r[0],
                    r[1],
                    r[2],
                    r[3],
                    rgba(if on { 0x4a221b } else { 0x2c2c3c }, 1.0),
                );
            }
            c.text(
                r[0] + 6.0,
                r[1] + 3.0,
                7.0,
                rgba(if on { 0xffffff } else { 0xd8d8e8 }, 1.0),
                e.label(),
            );
            self.buttons.push((r, Btn::Fx(FxAct::SetEase(i as u8))));
        }
    }
}

/// The pose of `l` a change goes to: its end pose (`end`, made its own) or its start pose.
pub(super) fn pose_mut(l: &mut Layer, end: bool) -> &mut Pose {
    if end {
        l.to.get_or_insert(l.from)
    } else {
        &mut l.from
    }
}
