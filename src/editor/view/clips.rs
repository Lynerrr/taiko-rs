//! The Effects page: effect clips as blocks on rows under the timeline, the shelf of ready-made
//! effects, and the settings of the clip picked.
//!
//! A click on an effect of the shelf drops it at the playhead (over the kiai the playhead is in,
//! else over the notes selected, else over the next four bars). A drag on an empty row draws one
//! of the effect last used. A block moves by dragging it, its ends resize it, a right click
//! deletes it.

use super::{Btn, Drag, Field, Keys, Mode, View, fmt_time, inside};
use crate::editor::clips::{
    PRESETS, amount_step, describe, target_about, target_colour, target_label, targets, trim,
};
use crate::render::{Canvas, rgba};
use crate::tko::clips::{Clip, Layer, Motion, Notes, Pose, Show, Target, When};
use crate::tko::track::Ease;

/// Rows of clips shown.
pub(super) const ROWS: u8 = 6;
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
    /// Drop shelf effect `i` at the playhead.
    Preset(usize),
    /// Back to the shelf.
    Back,
    /// The previous or next thing to move.
    What(i32),
    /// The previous or next group of notes (0 is all of them).
    Group(i32),
    /// For its whole length, every few beats, on notes, on hits, on misses.
    When(u8),
    Notes(Notes),
    Motion(Motion),
    Ease,
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
            CF::Seen => "seen",
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

    pub(super) fn draw_clips_page(&mut self, c: &mut Canvas, x: f32, y: f32, inner: f32) {
        if !self.ed.is_tko() {
            self.draw_tko_explainer(c, x, y, inner);
            return;
        }
        if self.picked().is_some() {
            self.draw_clip_settings(c, x, y, inner);
        } else {
            self.draw_shelf(c, x, y, inner);
        }
    }

    fn draw_shelf(&mut self, c: &mut Canvas, x: f32, y: f32, inner: f32) {
        let hint = "Click an effect to drop it at the playhead (over the kiai you're in, or the notes selected), or drag on a row below. Keys 1-9 record the first nine live while the song plays: press to start, again to end.";
        let y = self.paragraph(c, x, y, inner, 6.0, DIM, hint);
        let items: Vec<(String, Btn, bool)> = PRESETS
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    p.name.to_string(),
                    Btn::Fx(FxAct::Preset(i)),
                    self.fx_armed == i,
                )
            })
            .collect();
        let first = self.buttons.len();
        self.wrap_row(c, x, y + 2.0, inner, &items);
        for (k, p) in PRESETS.iter().enumerate() {
            if let Some((r, _)) = self.buttons.get(first + k).copied() {
                let key = if k < 9 {
                    format!("\nKey {}: record it live (press again to end).", k + 1)
                } else {
                    String::new()
                };
                self.tips.push((r, format!("{}\n{}{key}", p.name, p.about)));
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
        // What it moves.
        let what = target.map_or_else(|| clip.what.clone(), target_label);
        let mut row = vec![
            ("< Effects".to_string(), Btn::Fx(FxAct::Back), false),
            ("<".to_string(), Btn::Fx(FxAct::What(-1)), false),
            (what, Btn::Fx(FxAct::What(1)), true),
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
        let first = self.buttons.len();
        self.row(c, x, y, &row);
        if let (Some(t), Some((r, _))) = (target, self.buttons.get(first + 2).copied()) {
            self.tips
                .push((r, format!("{}\n{}", target_label(t), target_about(t))));
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
            format!("ease: {}", clip.ease.label()),
            Btn::Fx(FxAct::Ease),
            false,
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
                    Some((Field::FxText, t)) => format!("{t}_"),
                    _ => format!("{text}  (click to type)"),
                };
                row.push((shown, Btn::Edit(Field::FxText), self.typing.is_some()));
            }
            Show::Picture { file } => {
                row.push(("<".to_string(), Btn::Fx(FxAct::Picture(-1)), false));
                row.push((file.clone(), Btn::Fx(FxAct::Picture(1)), true));
                row.push((">".to_string(), Btn::Fx(FxAct::Picture(1)), false));
            }
        }
        y = self.wrap_row(c, x, y, inner, &row);
        let moves = layer.to.is_some();
        let end = self.fx_end && moves;
        let mut items = vec![("moves".to_string(), Btn::Fx(FxAct::Moves), moves)];
        if moves {
            items.push((
                "at its start".to_string(),
                Btn::Fx(FxAct::EndPose(false)),
                !end,
            ));
            items.push(("at its end".to_string(), Btn::Fx(FxAct::EndPose(true)), end));
        }
        items.push((
            "behind the notes".to_string(),
            Btn::Fx(FxAct::Behind),
            layer.behind,
        ));
        let first = self.buttons.len();
        items.push(("colour".to_string(), Btn::Fx(FxAct::Colour), false));
        y = self.wrap_row(c, x, y, inner, &items);
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
        cx = self.clip_number(c, cx, y, col, CF::Seen, p.alpha);
        // Repeating, it pulses.
        if clip.when != When::Span {
            self.clip_number(c, cx, y, col, CF::Amount, clip.amount);
        }
        y += 19.0;
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
                format!("ease: {}", clip.ease.label()),
                Btn::Fx(FxAct::Ease),
                false,
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

    /// Where a new clip goes when dropped at the playhead: the kiai it is in, else the notes
    /// selected, else four bars from the beat the playhead is on.
    fn drop_span(&self) -> (f64, f64) {
        let t = self.ed.time;
        if let Some(span) = self.ed.kiai_around(t) {
            return span;
        }
        if let Some(span) = self.selection_span() {
            return span;
        }
        let start = self.ed.snapped(t);
        (start, start + 16.0 * self.ed.beat_at(start))
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

    /// The first row free over `start..end`.
    fn free_row(&self, start: f64, end: f64, rows: usize) -> u8 {
        (0..ROWS)
            .find(|&r| {
                (0..rows as u8).all(|k| {
                    self.ed
                        .clips()
                        .iter()
                        .all(|c| c.lane.min(ROWS - 1) != r + k || c.end <= start || c.start >= end)
                })
            })
            .unwrap_or(0)
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
                            "put a .png or .jpg in the map's folder first, then pick Picture again"
                                .into();
                        return;
                    }
                }
            }
        }
        let row = row.unwrap_or_else(|| self.free_row(start, end, clips.len()));
        for (k, c) in clips.iter_mut().enumerate() {
            c.lane = (row + k as u8).min(ROWS - 1);
        }
        let ids = self.ed.add_clips(clips);
        self.fx_armed = i;
        if let Some(&id) = ids.first() {
            self.fx_sel = Some(id);
            self.status = format!("{} from {} to {}", p.name, fmt_time(start), fmt_time(end));
            self.mark_stale();
        }
    }

    /// Key `1 + i` on the clip rows: starts recording shelf effect `i` at the playhead, or ends
    /// the one recording there and drops it over that stretch (one undo step).
    pub(super) fn live_key(&mut self, i: usize) {
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
            self.status = format!("recording {}: press {} again to end it", p.name, i + 1);
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
                c.lane = (clips.len() as u8).min(ROWS - 1);
                if let Some(l) = c.layer.as_deref_mut() {
                    l.show = Show::Text {
                        text: "Showtime!".into(),
                    };
                }
                clips.push(c);
            }
        }
        let ids = self.ed.add_clips(clips);
        self.fx_sel = None;
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
    pub(super) fn map_pictures(&self) -> Vec<String> {
        self.ed
            .path
            .parent()
            .map(crate::pictures::in_folder)
            .unwrap_or_default()
    }

    pub(super) fn press_fx(&mut self, a: FxAct) {
        let Some(id) = self.fx_sel.filter(|&id| self.ed.clip(id).is_some()) else {
            match a {
                FxAct::Preset(i) => {
                    let (s, e) = self.drop_span();
                    self.place_preset(i, s, e, None);
                }
                FxAct::Advanced => self.fx_advanced = !self.fx_advanced,
                _ => self.fx_sel = None,
            }
            return;
        };
        let changed = match a {
            FxAct::Preset(i) => {
                self.fx_sel = None;
                let (s, e) = self.drop_span();
                self.place_preset(i, s, e, None);
                false
            }
            FxAct::Back => {
                self.fx_sel = None;
                false
            }
            FxAct::Advanced => {
                self.fx_advanced = !self.fx_advanced;
                false
            }
            FxAct::What(_) if self.picked().is_some_and(|c| c.layer.is_some()) => false,
            FxAct::What(d) => {
                let all = targets();
                let now = self.picked().and_then(|c| c.target());
                let at = now
                    .map(|t| match t {
                        Target::Notes(_, p) => Target::Notes(None, p),
                        t => t,
                    })
                    .and_then(|t| all.iter().position(|x| *x == t))
                    .unwrap_or(0);
                let next = all[(at as i32 + d).rem_euclid(all.len() as i32) as usize];
                let group = self.picked().map_or(0, |c| c.group);
                self.ed.set_clip(id, false, |c| {
                    let fresh = Clip::new(next, c.start, c.end);
                    c.what = fresh.what;
                    c.group = if matches!(next, Target::Notes(..)) {
                        group
                    } else {
                        0
                    };
                    // A sensible amount for what it moves now.
                    c.amount = amount_step(next) * 10.0;
                })
            }
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
            FxAct::Ease => self.ed.set_clip(id, false, |c| {
                let at = Ease::ALL.iter().position(|e| *e == c.ease).unwrap_or(0);
                // Step is for keyframes: a clip with it would do nothing.
                let mut next = Ease::ALL[(at + 1) % Ease::ALL.len()];
                if next == Ease::Step {
                    next = Ease::ALL[(at + 2) % Ease::ALL.len()];
                }
                c.ease = next;
            }),
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
                if let Some(copy) = self.ed.duplicate_clip(id) {
                    self.fx_sel = Some(copy);
                }
                true
            }
            FxAct::Delete => {
                self.fx_sel = None;
                self.ed.delete_clip(id)
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

    pub(super) fn row_y(&self, row: u8) -> f32 {
        self.layout.graph[1] + ROWS_TOP + row.min(ROWS - 1) as f32 * self.row_h()
    }

    fn row_at(&self, y: f32) -> u8 {
        (((y - self.layout.graph[1] - ROWS_TOP) / self.row_h())
            .floor()
            .max(0.0) as u8)
            .min(ROWS - 1)
    }

    fn clip_at(&self, x: f32, y: f32) -> Option<(u32, Part)> {
        let row = self.row_at(y);
        self.ed
            .clips()
            .iter()
            .rev()
            .filter(|c| c.lane.min(ROWS - 1) == row)
            .find_map(|c| {
                let (a, b) = (self.x_of(c.start), self.x_of(c.end));
                if x < a - 1.0 || x > b + 1.0 {
                    return None;
                }
                let part = if b - a > EDGE * 3.0 && x <= a + EDGE {
                    Part::Start
                } else if b - a > EDGE * 3.0 && x >= b - EDGE {
                    Part::End
                } else {
                    Part::Body
                };
                Some((c.id, part))
            })
    }

    pub(super) fn draw_clip_rows(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.graph;
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
        for r in 0..ROWS {
            let ry = self.row_y(r);
            if r % 2 == 1 {
                c.rect(x + 1.0, ry, w - 2.0, rh, rgba(0xffffff, 0.02));
            }
        }
        // The clips.
        let reds = self.ed.reds();
        let beat = |t: f64| crate::editor::clips::beat_in(&reds, t);
        let clips: Vec<Clip> = self
            .ed
            .clips()
            .iter()
            .filter(|cl| cl.end >= from && cl.start <= to)
            .cloned()
            .collect();
        for cl in &clips {
            let (a, b) = (self.x_of(cl.start), self.x_of(cl.end));
            let ry = self.row_y(cl.lane) + 1.0;
            let bh = rh - 2.0;
            let col = cl.target().map_or(0x888899, target_colour);
            let on = self.fx_sel == Some(cl.id);
            c.round_rect(
                [a, ry],
                [(b - a).max(2.0), bh],
                3.0,
                rgba(col, if on { 0.42 } else { 0.24 }),
                rgba(if on { 0xffffff } else { col }, if on { 1.0 } else { 0.7 }),
                1.0,
            );
            // What it does over its length, as the game will.
            let moments = self.ed.clip_moments(cl, &reds);
            let peak = cl.amount.abs().max(1e-9);
            let steps = if cl.motion == Motion::Step && cl.when != When::Span {
                moments.len().max(1) as f64
            } else {
                1.0
            };
            let mut px = a.max(x);
            let stop = b.min(x + w);
            while px < stop {
                let t = self.t_at(px);
                let v = cl.value(t, &moments, beat) / (peak * steps);
                let hh = (v.abs() as f32).min(1.0) * (bh - 4.0) * 0.9;
                if hh > 0.3 {
                    c.rect(px, ry + bh - 2.0 - hh, 1.5, hh, rgba(col, 0.55));
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
        // Effects being recorded live, growing with the playhead.
        for (k, &(i, from)) in self.fx_live.iter().enumerate() {
            let (a, b) = (
                self.x_of(from.min(self.ed.time)),
                self.x_of(from.max(self.ed.time)),
            );
            let ry = self.row_y((ROWS - 1).saturating_sub(k as u8)) + 1.0;
            c.round_rect(
                [a, ry],
                [(b - a).max(2.0), rh - 2.0],
                3.0,
                rgba(0xFF4D5E, 0.25),
                rgba(0xFF4D5E, 0.9),
                1.0,
            );
            let name = PRESETS.get(i).map_or("", |p| p.name);
            let label = Canvas::fit(
                5.5,
                (b - a - 6.0).max(40.0),
                &format!("REC {}  {name}", i + 1),
            );
            c.text(a + 3.0, ry + 2.0, 5.5, rgba(0xffffff, 0.95), &label);
        }
        let px = self.x_of(self.ed.time);
        c.rect(px - 0.75, y + 1.0, 1.5, h - 2.0, rgba(0xffffff, 0.8));
        c.clip_since(start, x, y, w, h);
        let name = PRESETS.get(self.fx_armed).map_or("", |p| p.name);
        c.text(
            x + 4.0,
            y + 3.0,
            5.5,
            rgba(0x6a6a80, 1.0),
            &format!("EFFECT CLIPS   drag on a row: {name}   drag a clip to move, its ends to resize, right click deletes"),
        );
    }

    pub(super) fn clip_rows_down(&mut self, x: f32, y: f32) {
        if self.in_motion_strip(y) {
            if !self.motion_strip_down(x) {
                self.drag = Drag::Scrub;
                let t = self.t_at(x);
                self.set_time(t);
            }
            return;
        }
        match self.clip_at(x, y) {
            Some((id, part)) => {
                self.fx_sel = Some(id);
                let Some(cl) = self.ed.clip(id) else { return };
                self.drag = match part {
                    Part::Body => Drag::FxMove {
                        id,
                        grab: self.t_at(x) - cl.start,
                    },
                    Part::Start => Drag::FxEdge { id, end: false },
                    Part::End => Drag::FxEdge { id, end: true },
                };
            }
            None => {
                let t = self.ed.snapped(self.t_at(x));
                self.drag = Drag::FxNew {
                    row: self.row_at(y),
                    from: t,
                    to: t,
                };
            }
        }
    }

    pub(super) fn clip_rows_drag(&mut self, x: f32, y: f32) {
        let t = self.ed.snapped(self.t_at(x).max(0.0));
        match self.drag {
            Drag::FxNew { row, from, .. } => self.drag = Drag::FxNew { row, from, to: t },
            Drag::FxMove { id, grab } => {
                let row = self.row_at(y);
                let start = self.ed.snapped((self.t_at(x) - grab).max(0.0));
                if self.ed.set_clip(id, true, |c| {
                    let len = c.end - c.start;
                    c.start = start;
                    c.end = start + len;
                    c.lane = row;
                }) {
                    self.mark_stale();
                }
            }
            Drag::FxEdge { id, end } => {
                let min = self.ed.beat_at(t) / self.ed.snap.max(1) as f64;
                if self.ed.set_clip(id, true, |c| {
                    if end {
                        c.end = t.max(c.start + min);
                    } else {
                        c.start = t.min(c.end - min);
                    }
                }) {
                    self.mark_stale();
                }
            }
            _ => {}
        }
    }

    pub(super) fn clip_rows_up(&mut self, drag: Drag, x: f32) {
        match drag {
            Drag::FxNew { row, from, to } => {
                let (a, b) = (from.min(to), from.max(to));
                if b - a >= 1.0 && (self.x_of(b) - self.x_of(a)) >= 4.0 {
                    let armed = self.fx_armed;
                    self.place_preset(armed, a, b, Some(row));
                } else {
                    // A click on an empty row: nothing picked, the playhead there.
                    self.fx_sel = None;
                    let t = self.ed.snapped(self.t_at(x));
                    self.set_time(t);
                }
            }
            Drag::FxMove { .. } | Drag::FxEdge { .. } => {
                self.ed.end_drag();
                self.mark_stale();
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
            if self.fx_sel == Some(id) {
                self.fx_sel = None;
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
            None => I::Crosshair,
        })
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
