//! SVT on the velocity graph: the curves the chart's green lines already follow, drawn over the
//! graph with handles to reshape them, a brush that draws a new one across the notes, a volume
//! view, and the panel with SVT's settings.
//!
//! Everything it writes goes through [`svt::plan`], so a curve drawn here, one typed into the panel
//! and one read back from the chart are the same thing (`sv_curves::detect_runs` finds the same
//! curve again). Nothing touches the chart until a drag ends or Apply is pressed, and each is one
//! undo step.

use super::chrome::DIM;
use super::{Btn, Drag, Field, GREEN_LINE, Keys, View, fmt_time};
use crate::editor::chart::TimingPoint;
use crate::editor::sv_curves::{self, Anchor, Curve, MAX_EXPONENT, MIN_EXPONENT, Run, RunKind};
use crate::editor::svt::{self, Segment, Shape, SvtParams};
use crate::editor::timing;
use crate::editor::tools::Plan;
use crate::hit::Canvas;
use crate::render::{accent, rgba};

pub(super) const VOLUME_COL: u32 = 0x4f9dff;
const CURVE_COL: u32 = 0xffc857;
const WHITE: u32 = 0xffffff;

/// A number SVT's panel edits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SF {
    FromSv,
    ToSv,
    FromVol,
    ToVol,
    Exponent,
    Value,
    Offset,
    Buffer,
    Spacing,
    SnapNumer,
    SnapDenom,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SAct {
    /// The Timing inspector's page: 0 the line, 1 the SV tool.
    Tab(u8),
    Brush,
    VolumeView(bool),
    /// The curve or mode, by `SHAPES` index.
    Shape(u8),
    Hits,
    Snaps,
    Lines,
    /// What is written: 0 speed, 1 volume, 2 both.
    Write(u8),
    /// Opens (or closes) the dropdown of a setting: 0 shape, 1 write, 2 place on, 3 presets.
    Drop(u8),
    /// Shows or hides the offset, buffer and spacing rows.
    Advanced,
    /// Lets go of the picked curve.
    Done,
    IgnoreBpm,
    Relative,
    PerPair,
    Apply,
    /// Forget the typed start and end and read them off the chart again.
    TakeChart,
    /// The curve chip at this position of the run list.
    Pick(usize),
    Preset(u8),
}

/// The shapes the panel offers, in order.
const SHAPES: [&str; 9] = [
    "Linear",
    "Geometric",
    "Poly",
    "Sine",
    "Sine in",
    "Sine out",
    "Flat +",
    "Flat x",
    "Fan",
];

fn shape_kind(s: &Shape) -> u8 {
    match s {
        Shape::Between(Curve::Linear) => 0,
        Shape::Between(Curve::TrueExp) => 1,
        Shape::Between(Curve::Exp(_)) => 2,
        Shape::Between(Curve::Sin) => 3,
        Shape::Between(Curve::SinIn) => 4,
        Shape::Between(Curve::SinOut) => 5,
        Shape::Add(_) => 6,
        Shape::Scale(_) => 7,
        Shape::Fan { .. } => 8,
    }
}

fn curve_kind(c: &Curve) -> u8 {
    match c {
        Curve::Linear => 0,
        Curve::TrueExp => 1,
        Curve::Exp(_) => 2,
        Curve::Sin => 3,
        Curve::SinIn => 4,
        Curve::SinOut => 5,
    }
}

fn curve_name(c: &Curve) -> String {
    match c {
        Curve::Linear => "linear".into(),
        Curve::TrueExp => "geometric".into(),
        Curve::Exp(n) => format!("poly {n}"),
        Curve::Sin => "sine".into(),
        Curve::SinIn => "sine in".into(),
        Curve::SinOut => "sine out".into(),
    }
}

/// The runs found in a stretch of the chart.
struct RunCache {
    rev: u64,
    lo: i64,
    hi: i64,
    anchors: Vec<Anchor>,
    runs: Vec<Run>,
}

pub(super) struct Ghost {
    rev: u64,
    params: SvtParams,
    segments: Vec<Segment>,
    pub(super) result: Result<Plan, String>,
}

/// A curve's end being dragged.
pub(super) struct HandleDrag {
    run: Run,
    /// Each line of the curve and the time of the note it is for.
    lines: Vec<(u64, f64)>,
}

/// What a preset of the SV tool's "between selected lines" list sets.
enum PresetShape {
    Curve(Curve),
    /// Multiplies the lines by the tool's value.
    Scale,
}

struct Preset {
    name: &'static str,
    /// Written on every note instead of on the lines already there.
    on_notes: bool,
    shape: PresetShape,
}

const fn preset(name: &'static str, on_notes: bool, shape: PresetShape) -> Preset {
    Preset {
        name,
        on_notes,
        shape,
    }
}

/// The Presets list: a click sets the shape and where it is written, Apply writes it.
#[cfg(test)]
pub(super) const PRESET_COUNT: usize = PRESETS.len();
const PRESETS: [Preset; 10] = [
    preset(
        "Ramp the selected lines",
        false,
        PresetShape::Curve(Curve::Linear),
    ),
    preset(
        "Geometric ramp of the lines",
        false,
        PresetShape::Curve(Curve::TrueExp),
    ),
    preset("Multiply the selected lines", false, PresetShape::Scale),
    preset(
        "A line on every note",
        true,
        PresetShape::Curve(Curve::Linear),
    ),
    preset(
        "Ease in: slow start, fast end",
        false,
        PresetShape::Curve(Curve::Exp(1.6)),
    ),
    preset(
        "Ease out: fast start, settles",
        false,
        PresetShape::Curve(Curve::Exp(0.6)),
    ),
    preset("Smooth S-curve", false, PresetShape::Curve(Curve::Sin)),
    preset(
        "Fast start, slow end (sine)",
        false,
        PresetShape::Curve(Curve::SinIn),
    ),
    preset(
        "Slow start, fast end (sine)",
        false,
        PresetShape::Curve(Curve::SinOut),
    ),
    preset(
        "Curved line on every note",
        true,
        PresetShape::Curve(Curve::Exp(1.3)),
    ),
];

pub(super) struct SvtUi {
    pub params: SvtParams,
    /// Typed over what the chart says at the ends of the segment.
    pub from_sv: Option<f64>,
    pub to_sv: Option<f64>,
    pub from_vol: Option<i32>,
    pub to_vol: Option<i32>,
    pub exponent: f64,
    pub value: f64,
    pub brush: bool,
    pub volume_view: bool,
    pub tab: u8,
    /// The open dropdown (see `SAct::Drop`) and the button it hangs from.
    pub drop: Option<(u8, [f32; 4])>,
    pub advanced: bool,
    /// The curve picked on the graph, by the source times of its first and last line.
    pub picked: Option<(f64, f64)>,
    pub handles: Vec<([f32; 4], bool)>,
    pub handle: Option<HandleDrag>,
    /// The ghost of what Apply would write is on screen (the panel is open).
    pub live: bool,
    runs: Option<RunCache>,
    pub(super) ghost: Option<Ghost>,
}

impl SvtUi {
    pub fn new() -> SvtUi {
        SvtUi {
            params: SvtParams::default(),
            from_sv: None,
            to_sv: None,
            from_vol: None,
            to_vol: None,
            exponent: 0.5,
            value: 1.1,
            brush: false,
            volume_view: false,
            tab: 0,
            drop: None,
            advanced: false,
            picked: None,
            handles: Vec::new(),
            handle: None,
            live: false,
            runs: None,
            ghost: None,
        }
    }
}

impl View {
    // ---- what the graph shows ----

    /// What a line is drawn at: its volume in the volume view, its velocity otherwise.
    pub(super) fn graph_val(&self, p: &TimingPoint) -> f64 {
        if self.svt.volume_view {
            f64::from(p.volume)
        } else {
            p.sv()
        }
    }

    /// The same read at a time: what is in force there.
    pub(super) fn val_at(&self, t: f64) -> f64 {
        let pts = &self.ed.chart.timing;
        if self.svt.volume_view {
            timing::active_at(pts, t).map_or(100.0, |p| f64::from(p.volume))
        } else {
            timing::sv_at(pts, t)
        }
    }

    pub(super) fn val_y(&self, v: f64, range: (f64, f64)) -> f32 {
        if self.svt.volume_view {
            let (bottom, span) = self.plot();
            bottom - ((v / 100.0).clamp(0.0, 1.0) as f32) * span
        } else {
            self.sv_y(v, range)
        }
    }

    pub(super) fn y_val(&self, py: f32, range: (f64, f64)) -> f64 {
        if self.svt.volume_view {
            let (bottom, span) = self.plot();
            (f64::from((bottom - py) / span).clamp(0.0, 1.0) * 100.0).round()
        } else {
            (self.y_sv(py, range) * 100.0).round() / 100.0
        }
    }

    // ---- the curves already in the chart ----

    fn note_times(&self) -> Vec<f64> {
        self.ed.chart.objects.iter().map(|o| o.time).collect()
    }

    fn refresh_runs(&mut self) {
        let rev = self.ed.history.rev();
        let (lo, hi) = (
            self.view_start.floor() as i64,
            self.view_end().ceil() as i64,
        );
        if let Some(c) = &self.svt.runs
            && c.rev == rev
            && c.lo == lo
            && c.hi == hi
        {
            return;
        }
        let anchors = sv_curves::anchors(
            &self.ed.chart.timing,
            &self.note_times(),
            lo as f64,
            hi as f64,
        );
        let runs = sv_curves::detect_runs(&anchors, false);
        self.svt.runs = Some(RunCache {
            rev,
            lo,
            hi,
            anchors,
            runs,
        });
    }

    /// The picked curve, with each of its lines and the note time the line is for.
    fn picked_run(&mut self) -> Option<(Run, Vec<(u64, f64)>)> {
        let (a, b) = self.svt.picked?;
        self.refresh_runs();
        let cache = self.svt.runs.as_ref()?;
        let run = cache
            .runs
            .iter()
            .find(|r| r.start.source_time == a && r.end.source_time == b)?
            .clone();
        let lines = self
            .ed
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red() && p.time >= a && p.time <= b)
            .map(|p| {
                let at = cache
                    .anchors
                    .iter()
                    .find(|n| n.source_time == p.time)
                    .map_or(p.time, |n| n.time);
                (p.id, at)
            })
            .collect();
        Some((run, lines))
    }

    /// Drops the pick when the selection is no longer that curve's lines.
    fn check_picked(&mut self) {
        let Some((a, b)) = self.svt.picked else {
            return;
        };
        let mut want: Vec<u64> = self
            .ed
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red() && p.time >= a && p.time <= b)
            .map(|p| p.id)
            .collect();
        let mut have = self.selected_lines();
        want.sort_unstable();
        have.sort_unstable();
        if want != have || want.len() < 3 {
            self.svt.picked = None;
        }
    }

    fn pick_run(&mut self, i: usize) {
        self.refresh_runs();
        let Some(run) = self.svt.runs.as_ref().and_then(|c| c.runs.get(i)).cloned() else {
            return;
        };
        let (a, b) = (run.start.source_time, run.end.source_time);
        let ids: Vec<u64> = self
            .ed
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red() && p.time >= a && p.time <= b)
            .map(|p| p.id)
            .collect();
        self.sel_line = ids.first().copied();
        self.sel_extra = ids.into_iter().collect();
        self.svt.picked = Some((a, b));
        self.ed.select_none();
        // Picking a curve opens its editor, so the click always has a visible result.
        if self.mode == super::Mode::Timing && !self.windowed() {
            self.svt.tab = 1;
        }
        if let RunKind::Curve(Curve::Exp(n)) = &run.kind {
            self.svt.exponent = *n;
        }
        self.status =
            "curve picked: drag its round ends on the graph, or change it in the SV tool".into();
    }

    /// The curve whose line passes within a few units of (`x`, `y`).
    pub(super) fn curve_near(&mut self, x: f32, y: f32) -> Option<usize> {
        if self.svt.volume_view || self.mode != super::Mode::Timing {
            return None;
        }
        self.refresh_runs();
        let range = self.graph_range();
        let t = self.t_at(x);
        let cache = self.svt.runs.as_ref()?;
        cache.runs.iter().position(|run| {
            matches!(run.kind, RunKind::Curve(_))
                && t >= run.start.time
                && t <= run.end.time
                && (self.sv_y(run.curve_at(t), range) - y).abs() <= 5.0
        })
    }

    /// Writes `run`'s curve onto its lines.
    fn rewrite_run(&mut self, run: &Run, lines: &[(u64, f64)], drag: bool) {
        let ids: Vec<u64> = lines.iter().map(|l| l.0).collect();
        let r = self.ed.edit_timings(
            &ids,
            |p| {
                if let Some(&(_, at)) = lines.iter().find(|l| l.0 == p.id) {
                    let v = (run.curve_at(at) * 10_000.0).round() / 10_000.0;
                    let _ = p.set_sv(v.clamp(0.01, 10.0));
                }
            },
            drag,
        );
        match r {
            Ok(()) => self.mark_stale(),
            Err(e) => self.status = e,
        }
    }

    /// Draws the curves over the graph, with the chips that pick them and the handles of the
    /// picked one.
    pub(super) fn draw_curve_layer(&mut self, c: &mut Canvas, range: (f64, f64)) {
        self.svt.handles.clear();
        if self.svt.volume_view {
            return;
        }
        self.check_picked();
        self.refresh_runs();
        let Some(cache) = self.svt.runs.take() else {
            return;
        };
        let [gx, gy, gw, gh] = self.layout.graph;
        let editing = self.mode == super::Mode::Timing;
        let idle = matches!(self.drag, Drag::None);
        let (mx, my) = self.mouse;
        let near = if idle && editing && super::inside(self.layout.graph, mx, my) {
            let (t, range) = (self.t_at(mx), range);
            cache.runs.iter().position(|run| {
                matches!(run.kind, RunKind::Curve(_))
                    && t >= run.start.time
                    && t <= run.end.time
                    && (self.sv_y(run.curve_at(t), range) - my).abs() <= 5.0
            })
        } else {
            None
        };
        for (i, run) in cache.runs.iter().enumerate() {
            let RunKind::Curve(kind) = &run.kind else {
                continue;
            };
            let picked = self.svt.picked == Some((run.start.source_time, run.end.source_time));
            let (x0, x1) = (self.x_of(run.start.time), self.x_of(run.end.time));
            if x1 < gx || x0 > gx + gw {
                continue;
            }
            let col = if picked { accent() } else { CURVE_COL };
            let near = idle && editing && near == Some(i);
            let th = if picked || near { 2.5 } else { 1.5 };
            let mut px = x0.max(gx);
            while px <= x1.min(gx + gw) {
                let t = self.t_at(px).clamp(run.start.time, run.end.time);
                let y = self.sv_y(run.curve_at(t), range);
                c.rect(px - th / 2.0, y - th / 2.0, th, th, rgba(col, 0.95));
                px += 2.0;
            }
            if !editing {
                continue;
            }
            if picked {
                for (end, a) in [(false, &run.start), (true, &run.end)] {
                    let (hx, hy) = (self.x_of(a.time), self.sv_y(a.value, range));
                    c.circle(hx, hy, 6.0, rgba(WHITE, 1.0), rgba(accent(), 1.0), 2.0);
                    let tag = format!("x{:.2}", a.value);
                    let tx = if end {
                        hx + 9.0
                    } else {
                        hx - 9.0 - Canvas::text_width(6.0, &tag)
                    };
                    c.text(tx, hy - 3.0, 6.0, rgba(WHITE, 1.0), &tag);
                    self.svt
                        .handles
                        .push(([hx - 8.0, hy - 8.0, 16.0, 16.0], end));
                }
            }
            if idle {
                let label = format!(
                    "{} x{:.2} > x{:.2}",
                    curve_name(kind),
                    run.start.value,
                    run.end.value
                );
                let w = Canvas::text_width(5.5, &label) + 8.0;
                let mid = (run.start.time + run.end.time) / 2.0;
                let cx = self
                    .x_of(mid)
                    .clamp(gx + w / 2.0 + 2.0, gx + gw - w / 2.0 - 2.0);
                let cy =
                    (self.sv_y(run.curve_at(mid), range) - 15.0).clamp(gy + 2.0, gy + gh - 12.0);
                let hover = near || c.hovered(cx - w / 2.0, cy, w, 10.0);
                c.round_rect(
                    [cx - w / 2.0, cy],
                    [w, 10.0],
                    3.0,
                    rgba(if picked || hover { 0x4a221b } else { 0x1d1d28 }, 0.97),
                    rgba(col, 1.0),
                    1.0,
                );
                c.text_centered(cx, cy + 2.0, 5.5, rgba(WHITE, 1.0), &label);
                self.buttons
                    .push(([cx - w / 2.0, cy, w, 10.0], Btn::S(SAct::Pick(i))));
            }
        }
        self.svt.runs = Some(cache);
    }

    // ---- dragging a curve's end ----

    /// A press on a handle of the picked curve. Returns whether it was one.
    pub(super) fn handle_down(&mut self, x: f32, y: f32) -> bool {
        let Some(&(_, end)) = self
            .svt
            .handles
            .iter()
            .find(|(r, _)| super::inside(*r, x, y))
        else {
            return false;
        };
        let Some((run, lines)) = self.picked_run() else {
            return false;
        };
        self.g_frozen = Some(self.graph_range());
        self.svt.handle = Some(HandleDrag { run, lines });
        self.drag = Drag::Handle { end };
        true
    }

    pub(super) fn handle_drag(&mut self, end: bool, y: f32) {
        let range = self.graph_range();
        let v = self.y_val(y, range).clamp(0.01, 10.0);
        let Some(h) = &mut self.svt.handle else {
            return;
        };
        let mut run = h.run.clone();
        if end {
            run.end.value = v;
        } else {
            run.start.value = v;
        }
        let lines = h.lines.clone();
        self.rewrite_run(&run, &lines, true);
    }

    pub(super) fn handle_up(&mut self) {
        self.svt.handle = None;
        self.g_frozen = None;
        self.ed.end_drag();
        self.mark_stale();
    }

    // ---- the brush ----

    /// A press with the brush on: the first end of the curve.
    pub(super) fn brush_down(&mut self, x: f32, y: f32, keys: Keys) {
        let range = self.graph_range();
        self.g_frozen = Some(range);
        let (t, v) = self.brush_end(x, y, keys);
        self.drag = Drag::Brush {
            t0: t,
            v0: v,
            t1: t,
            v1: v,
        };
    }

    pub(super) fn brush_move(&mut self, x: f32, y: f32, keys: Keys) {
        let Drag::Brush { t0, v0, .. } = self.drag else {
            return;
        };
        let (t1, v1) = self.brush_end(x, y, keys);
        self.drag = Drag::Brush { t0, v0, t1, v1 };
    }

    /// Where a brush end at (`x`, `y`) lands: on a line drawn close by, so strokes join the
    /// lines already there (Alt: never), else on the grid at the pointer's height.
    fn brush_end(&self, x: f32, y: f32, keys: Keys) -> (f64, f64) {
        if let Some(p) = self.brush_magnet(x, y, keys) {
            return p;
        }
        let range = self.graph_range();
        (self.grid_or_free(self.t_at(x), keys), self.y_val(y, range))
    }

    /// The nearest line in the graph (its time and value) within reach of (`x`, `y`). Only a
    /// pointer right by its dot is taken for wanting it: moving off lets the end go free again.
    fn brush_magnet(&self, x: f32, y: f32, keys: Keys) -> Option<(f64, f64)> {
        const REACH: f32 = 10.0;
        if keys.alt {
            return None;
        }
        let range = self.graph_range();
        self.ed
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red() || self.svt.volume_view)
            .map(|p| {
                let v = self.graph_val(p);
                let d = (self.x_of(p.time) - x).hypot(self.val_y(v, range) - y);
                (d, p.time, v)
            })
            .filter(|(d, ..)| *d <= REACH)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, t, v)| (t, v))
    }

    /// Whether a brush end sits on a line (the magnet caught it).
    fn brush_on_line(&self, t: f64, v: f64) -> bool {
        self.ed
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red() || self.svt.volume_view)
            .any(|p| p.time == t && self.graph_val(p) == v)
    }

    /// What a brush stroke from `(t0, v0)` to `(t1, v1)` would write.
    fn brush_plan(&self, t0: f64, v0: f64, t1: f64, v1: f64) -> Result<Plan, String> {
        let (segment, params) = self.brush_segment(t0, v0, t1, v1)?;
        svt::plan(
            &self.ed.chart.timing,
            &self.note_times(),
            &[segment],
            &params,
        )
    }

    fn brush_segment(
        &self,
        t0: f64,
        v0: f64,
        t1: f64,
        v1: f64,
    ) -> Result<(Segment, SvtParams), String> {
        let (ta, va, tb, vb) = if t0 <= t1 {
            (t0, v0, t1, v1)
        } else {
            (t1, v1, t0, v0)
        };
        if tb - ta < 2.0 {
            return Err("drag across the notes to draw a curve".into());
        }
        let mut params = self.svt.params.clone();
        if params.shape.is_flat() {
            return Err("the brush draws a curve: pick Linear, Sine... or Fan".into());
        }
        let mut seg = svt::segment_between(&self.ed.chart.timing, ta, tb);
        if self.svt.volume_view {
            params.write_sv = false;
            params.write_volume = true;
            seg.start.volume = va.round() as i32;
            seg.end.volume = vb.round() as i32;
        } else {
            seg.start.sv = va;
            seg.end.sv = vb;
        }
        Ok((seg, params))
    }

    pub(super) fn brush_up(&mut self, t0: f64, v0: f64, t1: f64, v1: f64) {
        self.g_frozen = None;
        let r = self
            .brush_segment(t0, v0, t1, v1)
            .and_then(|(seg, params)| self.svt_commit(&[seg], &params));
        self.status = match r {
            Ok(said) => said,
            Err(e) => e,
        };
    }

    /// Writes what `params` makes of `segments` as one undo step and picks what it wrote.
    pub(super) fn svt_commit(
        &mut self,
        segments: &[Segment],
        params: &SvtParams,
    ) -> Result<String, String> {
        let plan = svt::plan(&self.ed.chart.timing, &self.note_times(), segments, params)?;
        let (n, gone) = (plan.timing.len(), plan.remove.len());
        let times: Vec<f64> = plan.timing.iter().map(|p| p.time).collect();
        self.ed.commit_plan(plan);
        if let Err(why) = self.ed.chart.validate() {
            self.ed.undo();
            return Err(why);
        }
        let ids: Vec<u64> = self
            .ed
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red() && times.contains(&p.time))
            .map(|p| p.id)
            .collect();
        self.sel_line = ids.first().copied();
        self.sel_extra = ids.iter().copied().collect();
        self.svt.picked = match (times.first(), times.last()) {
            (Some(a), Some(b)) if ids.len() >= 3 && matches!(params.shape, Shape::Between(_)) => {
                Some((*a, *b))
            }
            _ => None,
        };
        self.ed.select_none();
        self.mark_stale();
        Ok(format!(
            "wrote {n} line{}{}",
            if n == 1 { "" } else { "s" },
            if gone > 0 {
                format!(", replaced {gone}")
            } else {
                String::new()
            }
        ))
    }

    // ---- the segment and the panel ----

    /// What SVT would interpolate over: pairs of selected green lines, else the selected notes'
    /// span, else the playhead to the end of its bar. And a few words for it.
    pub(super) fn svt_segments(&self) -> (Vec<Segment>, String) {
        let pts = &self.ed.chart.timing;
        let picked: std::collections::HashSet<u64> = self.selected_lines().into_iter().collect();
        let greens: Vec<&TimingPoint> = pts
            .iter()
            .filter(|p| !p.is_red() && picked.contains(&p.id))
            .collect();
        if greens.len() >= 2 {
            let segs = svt::segments_from_lines(&greens);
            let said = format!(
                "{} segment{} between {} selected green lines",
                segs.len(),
                if segs.len() == 1 { "" } else { "s" },
                greens.len()
            );
            return (segs, said);
        }
        let sel = self.ed.selected();
        let (a, b, what) = if sel.len() >= 2 {
            let a = sel.iter().map(|o| o.time).fold(f64::INFINITY, f64::min);
            let b = sel.iter().map(|o| o.time).fold(f64::NEG_INFINITY, f64::max);
            (a, b, format!("{} selected notes", sel.len()))
        } else {
            let beat = timing::beat_length_at(pts, self.ed.time);
            let start = self.ed.snapped(self.ed.time);
            (
                start,
                (start + 4.0 * beat).floor(),
                "playhead to the end of the bar".to_string(),
            )
        };
        let mut seg = svt::segment_between(pts, a, b);
        if let Some(v) = self.svt.from_sv {
            seg.start.sv = v;
        }
        if let Some(v) = self.svt.to_sv {
            seg.end.sv = v;
        }
        if let Some(v) = self.svt.from_vol {
            seg.start.volume = v;
        }
        if let Some(v) = self.svt.to_vol {
            seg.end.volume = v;
        }
        (
            vec![seg],
            format!("{what}, {} - {}", fmt_time(a), fmt_time(b)),
        )
    }

    fn refresh_ghost(&mut self) {
        let (segments, _) = self.svt_segments();
        let rev = self.ed.history.rev();
        let params = self.svt.params.clone();
        if let Some(g) = &self.svt.ghost
            && g.rev == rev
            && g.params == params
            && g.segments == segments
        {
            return;
        }
        let result = svt::plan(
            &self.ed.chart.timing,
            &self.note_times(),
            &segments,
            &params,
        );
        self.svt.ghost = Some(Ghost {
            rev,
            params,
            segments,
            result,
        });
    }

    /// The ghost of what Apply (or a brush stroke) would write, over the graph.
    pub(super) fn draw_svt_ghost(&mut self, c: &mut Canvas, range: (f64, f64)) {
        let plan = if let Drag::Brush { t0, v0, t1, v1 } = self.drag {
            // The stroke itself: its two ends and what it would write.
            for (t, v) in [(t0, v0), (t1, v1)] {
                let (px, py) = (self.x_of(t), self.val_y(v, range));
                c.circle(px, py, 4.5, rgba(WHITE, 1.0), rgba(accent(), 1.0), 1.5);
                // Joined to a line already there.
                if self.brush_on_line(t, v) {
                    c.ring(px, py, 8.0, rgba(accent(), 0.9), 1.5);
                }
            }
            let unit = if self.svt.volume_view { "" } else { "x" };
            c.text(
                self.x_of(t1) + 8.0,
                self.val_y(v1, range) - 9.0,
                6.0,
                rgba(WHITE, 1.0),
                &format!("{unit}{v0} > {unit}{v1}"),
            );
            self.brush_plan(t0, v0, t1, v1).ok()
        } else if self.svt.live && self.mode == super::Mode::Timing {
            self.svt
                .ghost
                .as_ref()
                .and_then(|g| g.result.as_ref().ok())
                .cloned()
        } else {
            None
        };
        let Some(plan) = plan else { return };
        for g in &plan.remove {
            let (px, py) = (self.x_of(g.time), self.val_y(self.graph_val(g), range));
            c.ring(px, py, 5.0, rgba(0xe2463c, 0.9), 1.0);
        }
        for p in &plan.timing {
            let (px, py) = (self.x_of(p.time), self.val_y(self.graph_val(p), range));
            c.circle(px, py, 3.0, rgba(accent(), 0.35), rgba(accent(), 1.0), 1.0);
        }
    }

    /// The buttons in a corner of the graph: the brush and the SV / volume switch.
    pub(super) fn draw_graph_corner(&mut self, c: &mut Canvas) {
        if self.mode != super::Mode::Timing {
            return;
        }
        let [gx, gy, gw, _] = self.layout.graph;
        let items = [
            ("Brush", SAct::Brush, self.svt.brush),
            ("Volume", SAct::VolumeView(true), self.svt.volume_view),
            ("SV", SAct::VolumeView(false), !self.svt.volume_view),
        ];
        let mut x = gx + gw - 4.0;
        for (label, act, on) in items {
            let w = Canvas::text_width(6.5, label) + 12.0;
            x -= w;
            self.button(c, [x, gy + 3.0, w, 12.0], label, Btn::S(act), on);
            x -= 3.0;
        }
        self.tips.push((
            [x + 3.0, gy + 3.0, gx + gw - x, 12.0],
            "Brush: drag across the graph to draw a curve from one speed to another over the notes, grid or lines under it (set what it places on in the SV tool). An end dropped by a line clings to it, so strokes join up; Alt keeps it free. Volume shows and edits each line's volume instead of its speed.".to_string(),
        ));
    }

    // ---- the panel ----

    /// The picked curve as the last frame cached it.
    fn picked_peek(&self) -> Option<&Run> {
        let (a, b) = self.svt.picked?;
        self.svt
            .runs
            .as_ref()?
            .runs
            .iter()
            .find(|r| r.start.source_time == a && r.end.source_time == b)
    }

    fn svt_value(&self, f: SF) -> f64 {
        if let Some(run) = self.picked_peek() {
            match f {
                SF::FromSv => return run.start.value,
                SF::ToSv => return run.end.value,
                _ => {}
            }
        }
        let edge = self.svt_segments().0.first().copied();
        match f {
            SF::FromSv => edge.map_or(1.0, |s| s.start.sv),
            SF::ToSv => edge.map_or(1.0, |s| s.end.sv),
            SF::FromVol => edge.map_or(100.0, |s| f64::from(s.start.volume)),
            SF::ToVol => edge.map_or(100.0, |s| f64::from(s.end.volume)),
            SF::Exponent => self.svt.exponent,
            SF::Value => self.svt.value,
            SF::Offset => f64::from(self.svt.params.offset),
            SF::Buffer => f64::from(self.svt.params.buffer),
            SF::Spacing => f64::from(self.svt.params.min_spacing),
            SF::SnapNumer => self.svt.params.snap_numer,
            SF::SnapDenom => self.svt.params.snap_denom,
        }
    }

    fn svt_label(f: SF) -> &'static str {
        match f {
            SF::FromSv => "Start SV",
            SF::ToSv => "End SV",
            SF::FromVol => "Start vol",
            SF::ToVol => "End vol",
            SF::Exponent => "Exponent",
            SF::Value => "Amount",
            SF::Offset => "Offset",
            SF::Buffer => "Buffer",
            SF::Spacing => "Min spacing",
            SF::SnapNumer => "Grid n",
            SF::SnapDenom => "Grid /",
        }
    }

    fn svt_shown(&self, f: SF) -> String {
        let v = self.svt_value(f);
        match f {
            SF::FromSv | SF::ToSv | SF::Value => format!("{v:.2}"),
            SF::Exponent => format!("{v:.1}"),
            _ => format!("{}", v.round() as i64),
        }
    }

    pub(super) fn svt_set(&mut self, f: SF, v: f64) {
        if !v.is_finite() {
            return;
        }
        // With a curve picked, its end fields edit the curve itself.
        if matches!(f, SF::FromSv | SF::ToSv | SF::Exponent)
            && let Some((mut run, lines)) = self.picked_run()
        {
            let v = v.clamp(0.01, 10.0);
            match f {
                SF::FromSv => run.start.value = v,
                SF::ToSv => run.end.value = v,
                _ => {
                    let n = v.clamp(MIN_EXPONENT, MAX_EXPONENT);
                    self.svt.exponent = n;
                    if matches!(run.kind, RunKind::Curve(Curve::Exp(_))) {
                        run.kind = RunKind::Curve(Curve::Exp(n));
                    } else {
                        return;
                    }
                }
            }
            return self.rewrite_run(&run, &lines, false);
        }
        match f {
            SF::FromSv => self.svt.from_sv = Some(v.clamp(0.01, 10.0)),
            SF::ToSv => self.svt.to_sv = Some(v.clamp(0.01, 10.0)),
            SF::FromVol => self.svt.from_vol = Some(v.round().clamp(0.0, 100.0) as i32),
            SF::ToVol => self.svt.to_vol = Some(v.round().clamp(0.0, 100.0) as i32),
            SF::Exponent => {
                self.svt.exponent = v.clamp(MIN_EXPONENT, MAX_EXPONENT);
                if matches!(self.svt.params.shape, Shape::Between(Curve::Exp(_))) {
                    self.svt.params.shape = Shape::Between(Curve::Exp(self.svt.exponent));
                }
            }
            SF::Value => {
                self.svt.value = v.clamp(-10.0, 10.0);
                match self.svt.params.shape {
                    Shape::Add(_) => self.svt.params.shape = Shape::Add(self.svt.value),
                    Shape::Scale(_) => {
                        self.svt.value = v.clamp(0.01, 10.0);
                        self.svt.params.shape = Shape::Scale(self.svt.value);
                    }
                    _ => {}
                }
            }
            SF::Offset => self.svt.params.offset = v.round().clamp(-1000.0, 1000.0) as i32,
            SF::Buffer => self.svt.params.buffer = v.round().clamp(0.0, 1000.0) as i32,
            SF::Spacing => self.svt.params.min_spacing = v.round().clamp(0.0, 1000.0) as i32,
            SF::SnapNumer => self.svt.params.snap_numer = v.round().clamp(1.0, 64.0),
            SF::SnapDenom => self.svt.params.snap_denom = v.round().clamp(1.0, 64.0),
        }
    }

    pub(super) fn svt_step(&mut self, f: SF, dir: f64, keys: Keys) {
        let base = match f {
            SF::FromSv | SF::ToSv | SF::Value => 0.05,
            SF::FromVol | SF::ToVol => 5.0,
            SF::Exponent => 0.1,
            _ => 1.0,
        };
        let k = if keys.shift {
            10.0
        } else if keys.alt && base < 1.0 {
            0.2
        } else {
            1.0
        };
        let now = self.svt_value(f);
        self.svt_set(f, ((now + dir * base * k) * 1000.0).round() / 1000.0);
    }

    /// A number with its label, and - and + beside it.
    fn svt_field(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, f: SF) {
        let typing = matches!(&self.typing, Some((Field::S(tf), _)) if *tf == f);
        let text = match &self.typing {
            Some((Field::S(tf), t)) if *tf == f => format!("{}: {t}_", Self::svt_label(f)),
            _ => format!("{} {}", Self::svt_label(f), self.svt_shown(f)),
        };
        let lw = w - 32.0;
        self.button(c, [x, y, lw, 14.0], &text, Btn::Edit(Field::S(f)), typing);
        self.button(
            c,
            [x + lw + 2.0, y, 14.0, 14.0],
            "-",
            Btn::Dec(Field::S(f)),
            false,
        );
        self.button(
            c,
            [x + lw + 17.0, y, 14.0, 14.0],
            "+",
            Btn::Inc(Field::S(f)),
            false,
        );
    }

    /// A button that opens the dropdown of a setting.
    fn drop_button(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, kind: u8, label: &str) {
        let open = self.svt.drop.is_some_and(|d| d.0 == kind);
        self.button(c, [x, y, w, 14.0], "", Btn::S(SAct::Drop(kind)), open);
        c.text(
            x + 6.0,
            y + 3.5,
            7.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(7.0, w - 22.0, label),
        );
        c.text_right(x + w - 6.0, y + 3.5, 7.0, rgba(DIM, 1.0), "v");
    }

    /// The options of the open dropdown, under (or over) the button it hangs from.
    pub(super) fn draw_svt_drop(&mut self, c: &mut Canvas) {
        let Some((kind, at)) = self.svt.drop else {
            return;
        };
        if !self.svt.live && !self.windowed() {
            self.svt.drop = None;
            return;
        }
        let p = self.svt.params.clone();
        let flat = p.shape.is_flat();
        let items: Vec<(String, Btn, bool)> = match kind {
            0 => SHAPES
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let cur = match self.picked_peek() {
                        Some(Run {
                            kind: RunKind::Curve(k),
                            ..
                        }) => curve_kind(k),
                        _ => shape_kind(&p.shape),
                    };
                    (n.to_string(), Btn::S(SAct::Shape(i as u8)), i as u8 == cur)
                })
                .collect(),
            1 => ["Speed (SV)", "Volume", "Speed + volume"]
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let cur = match (p.write_sv, p.write_volume) {
                        (true, false) => 0,
                        (false, true) => 1,
                        _ => 2,
                    };
                    (n.to_string(), Btn::S(SAct::Write(i as u8)), i == cur)
                })
                .collect(),
            2 => vec![
                ("Notes".into(), Btn::S(SAct::Hits), p.hits && !flat),
                (
                    format!("Beat grid {}/{}", p.snap_numer as i64, p.snap_denom as i64),
                    Btn::S(SAct::Snaps),
                    p.snaps && !flat,
                ),
                (
                    "Green lines already there".into(),
                    Btn::S(SAct::Lines),
                    p.lines || flat,
                ),
            ],
            _ => PRESETS
                .iter()
                .enumerate()
                .map(|(i, p)| (p.name.to_string(), Btn::S(SAct::Preset(i as u8)), false))
                .collect(),
        };
        let w = items
            .iter()
            .map(|(l, _, _)| Canvas::text_width(7.0, l) + 26.0)
            .fold(at[2], f32::max);
        let h = items.len() as f32 * 15.0 + 4.0;
        let x = at[0].min(c.width - w - 4.0);
        let y = if at[1] + at[3] + h < 476.0 {
            at[1] + at[3] + 1.0
        } else {
            at[1] - h - 1.0
        };
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x15151d, 1.0),
            rgba(0x5a5a78, 1.0),
            1.0,
        );
        for (i, (label, b, on)) in items.into_iter().enumerate() {
            let r = [x + 2.0, y + 2.0 + i as f32 * 15.0, w - 4.0, 14.0];
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
                r[1] + 3.5,
                7.0,
                rgba(if on { 0xffffff } else { 0xd8d8e8 }, 1.0),
                &label,
            );
            if on {
                c.text_right(r[0] + r[2] - 6.0, r[1] + 3.5, 7.0, rgba(accent(), 1.0), "*");
            }
            self.buttons.push((r, b));
        }
    }

    /// The SV tool: edits the picked curve, or draws a new one (Apply).
    pub(super) fn draw_svt_panel(&mut self, c: &mut Canvas, x: f32, mut y: f32, w: f32) {
        self.svt.live = true;
        self.check_picked();
        self.refresh_ghost();
        let (segments, said) = self.svt_segments();
        let p = self.svt.params.clone();
        let flat = p.shape.is_flat();
        let picked = self.picked_run();
        let fw = ((w - 12.0) / 4.0).clamp(100.0, 150.0).min(w);
        let per_row = (((w + 4.0) / (fw + 4.0)) as usize).max(1);
        let grid = |i: usize| {
            (
                x + (i % per_row) as f32 * (fw + 4.0),
                (i / per_row) as f32 * 18.0,
            )
        };

        // ---- a curve picked on the graph: its editor ----
        if let Some((run, lines)) = &picked {
            let RunKind::Curve(k) = &run.kind else {
                self.svt.picked = None;
                return;
            };
            c.text(
                x,
                y,
                7.5,
                rgba(accent(), 1.0),
                &Canvas::fit(
                    7.5,
                    w - 60.0,
                    &format!("Editing a {} curve, {} lines", curve_name(k), lines.len()),
                ),
            );
            self.button(
                c,
                [x + w - 54.0, y - 3.0, 54.0, 14.0],
                "Done",
                Btn::S(SAct::Done),
                false,
            );
            y += 15.0;
            self.drop_button(
                c,
                x,
                y,
                fw + 24.0,
                0,
                &format!("Shape: {}", SHAPES[curve_kind(k) as usize]),
            );
            if curve_kind(k) == 2 {
                self.svt_field(c, x + fw + 28.0, y, fw, SF::Exponent);
            }
            y += 19.0;
            self.svt_field(c, x, y, fw, SF::FromSv);
            self.svt_field(c, x + fw + 4.0, y, fw, SF::ToSv);
            y += 22.0;
            self.paragraph(
                c,
                x,
                y,
                w,
                6.5,
                DIM,
                "Drag the round ends on the graph, or type the speeds here. Changing the shape redraws the curve over the same lines. Ctrl+Z takes it back.",
            );
            return;
        }

        // ---- a new curve ----
        c.text(x, y, 6.5, rgba(DIM, 1.0), &Canvas::fit(6.5, w, &said));
        y += 12.0;
        let shape_label = format!("Shape: {}", SHAPES[shape_kind(&p.shape) as usize]);
        self.drop_button(c, x, y, fw + 24.0, 0, &shape_label);
        let kind = shape_kind(&p.shape);
        let ex = x + fw + 28.0;
        if kind == 2 {
            self.svt_field(c, ex, y, fw, SF::Exponent);
        } else if flat {
            self.svt_field(c, ex, y, fw, SF::Value);
        } else if kind == 8 {
            let per_pair = matches!(p.shape, Shape::Fan { per_pair: true, .. });
            self.button(
                c,
                [ex, y, 70.0, 14.0],
                "Per pair",
                Btn::S(SAct::PerPair),
                per_pair,
            );
        }
        self.tips.push((
            [x, y, w, 14.0],
            "How the speed changes from the start to the end.\nLinear: evenly. Geometric: by the same factor each step (reads as an even acceleration). Poly: t^n, below 1 starts fast and settles. Sine: eases in and out.\nFlat + / x: add to or multiply the green lines already in range.\nFan: alternates around the start."
                .to_string(),
        ));
        y += 19.0;
        if !flat {
            let mut cells: Vec<SF> = Vec::new();
            if p.write_sv {
                cells.extend([SF::FromSv, SF::ToSv]);
            }
            if p.write_volume {
                cells.extend([SF::FromVol, SF::ToVol]);
            }
            for (i, f) in cells.iter().enumerate() {
                let (cx, cy) = grid(i);
                self.svt_field(c, cx, y + cy, fw, *f);
            }
            y += 18.0 * cells.len().div_ceil(per_row) as f32;
        }
        // What is written and where.
        let write = match (p.write_sv, p.write_volume) {
            (true, false) => "Write: speed",
            (false, true) => "Write: volume",
            _ => "Write: speed + volume",
        };
        self.drop_button(c, x, y, 128.0, 1, write);
        let mut on: Vec<String> = Vec::new();
        if flat || p.lines {
            on.push("green lines".into());
        }
        if !flat && p.hits {
            on.push("notes".into());
        }
        if !flat && p.snaps {
            on.push(format!(
                "grid {}/{}",
                p.snap_numer as i64, p.snap_denom as i64
            ));
        }
        let place = format!("On: {}", on.join(" + "));
        self.drop_button(c, x + 132.0, y, 170.0, 2, &place);
        if p.snaps && !flat {
            self.svt_field(c, x + 306.0, y, 70.0, SF::SnapNumer);
            self.svt_field(c, x + 380.0, y, 70.0, SF::SnapDenom);
        }
        self.tips.push((
            [x, y, w, 14.0],
            "Write: what the new lines carry. On: where they go: on each note, on the beat grid (n/d of a beat), or on the green lines already there (flat changes always use those).".to_string(),
        ));
        y += 19.0;
        // Rarely touched settings.
        let open = self.svt.advanced;
        self.button(
            c,
            [x, y, 80.0, 14.0],
            if open { "Advanced -" } else { "Advanced +" },
            Btn::S(SAct::Advanced),
            open,
        );
        if open {
            let mut tx = x + 84.0;
            for (label, a, on) in [
                ("Ignore BPM", SAct::IgnoreBpm, p.ignore_bpm),
                ("Relative", SAct::Relative, p.relative),
            ] {
                let bw = Canvas::text_width(7.0, label) + 12.0;
                self.button(c, [tx, y, bw, 14.0], label, Btn::S(a), on);
                tx += bw + 4.0;
            }
            y += 19.0;
            for (i, f) in [SF::Offset, SF::Buffer, SF::Spacing].iter().enumerate() {
                let (cx, cy) = grid(i);
                self.svt_field(c, cx, y + cy, fw, *f);
            }
            self.tips.push((
                [x, y, w, 18.0 * 3usize.div_ceil(per_row) as f32],
                "Offset: ms from the object a line goes (negative: before it).\nBuffer: ms around the segment that count as in it, for notes that aren't perfectly snapped.\nMin spacing: lines closer than this collapse, and old green lines this close to a new one are replaced."
                    .to_string(),
            ));
            y += 18.0 * 3usize.div_ceil(per_row) as f32;
        } else {
            y += 19.0;
        }
        // The result and Apply.
        let (note, ok) = match self.svt.ghost.as_ref().map(|g| &g.result) {
            Some(Ok(plan)) => (
                format!(
                    "Would write {} line{}{}",
                    plan.timing.len(),
                    if plan.timing.len() == 1 { "" } else { "s" },
                    if plan.remove.is_empty() {
                        String::new()
                    } else {
                        format!(" and replace {}", plan.remove.len())
                    }
                ),
                true,
            ),
            Some(Err(e)) => (e.clone(), false),
            None => (String::new(), false),
        };
        self.drop_button(c, x, y, 110.0, 3, "Presets");
        let bx = x + w - 112.0;
        c.text(
            x + 116.0,
            y + 4.0,
            6.5,
            rgba(if ok { GREEN_LINE } else { 0xfcb806 }, 1.0),
            &Canvas::fit(6.5, bx - x - 200.0, &note),
        );
        self.button(
            c,
            [bx - 74.0, y, 70.0, 14.0],
            "From chart",
            Btn::S(SAct::TakeChart),
            false,
        );
        self.button(
            c,
            [bx, y, 112.0, 14.0],
            "Apply",
            Btn::S(SAct::Apply),
            ok && !segments.is_empty(),
        );
    }

    /// Sets the shape; with a curve picked, redraws that curve in it.
    fn svt_shape(&mut self, kind: u8) {
        let s = &mut self.svt;
        s.params.shape = match kind {
            0 => Shape::Between(Curve::Linear),
            1 => Shape::Between(Curve::TrueExp),
            2 => Shape::Between(Curve::Exp(s.exponent)),
            3 => Shape::Between(Curve::Sin),
            4 => Shape::Between(Curve::SinIn),
            5 => Shape::Between(Curve::SinOut),
            6 => Shape::Add(s.value),
            7 => {
                s.value = s.value.clamp(0.01, 10.0);
                Shape::Scale(s.value)
            }
            _ => Shape::Fan {
                curve: Curve::Linear,
                per_pair: false,
            },
        };
        if kind <= 5
            && let Some((mut run, lines)) = self.picked_run()
        {
            let curve = match &self.svt.params.shape {
                Shape::Between(c) => c.clone(),
                _ => return,
            };
            run.kind = RunKind::Curve(curve);
            self.rewrite_run(&run, &lines, false);
        }
    }

    pub(super) fn press_svt(&mut self, a: SAct) {
        match a {
            SAct::Tab(t) => self.svt.tab = t,
            SAct::Brush => {
                self.svt.brush = !self.svt.brush;
                self.status = if self.svt.brush {
                    "brush on: drag across the graph to draw a curve".into()
                } else {
                    "brush off".into()
                };
            }
            SAct::VolumeView(v) => self.svt.volume_view = v,
            SAct::Shape(k) => self.svt_shape(k),
            SAct::Hits => self.svt.params.hits = !self.svt.params.hits,
            SAct::Snaps => self.svt.params.snaps = !self.svt.params.snaps,
            SAct::Lines => self.svt.params.lines = !self.svt.params.lines,
            SAct::Write(m) => {
                let p = &mut self.svt.params;
                (p.write_sv, p.write_volume) = (m != 1, m != 0);
            }
            SAct::Drop(k) => {
                let at = self
                    .buttons
                    .iter()
                    .rev()
                    .find(|(_, b)| *b == Btn::S(SAct::Drop(k)))
                    .map(|(r, _)| *r);
                self.svt.drop = at.map(|r| (k, r));
            }
            SAct::Advanced => self.svt.advanced = !self.svt.advanced,
            SAct::Done => {
                self.svt.picked = None;
                self.select_line(None);
            }
            SAct::IgnoreBpm => self.svt.params.ignore_bpm = !self.svt.params.ignore_bpm,
            SAct::Relative => self.svt.params.relative = !self.svt.params.relative,
            SAct::PerPair => {
                if let Shape::Fan { per_pair, .. } = &mut self.svt.params.shape {
                    *per_pair = !*per_pair;
                }
            }
            SAct::TakeChart => {
                let s = &mut self.svt;
                (s.from_sv, s.to_sv, s.from_vol, s.to_vol) = (None, None, None, None);
            }
            SAct::Pick(i) => self.pick_run(i),
            SAct::Preset(i) => {
                let Some(preset) = PRESETS.get(i as usize) else {
                    return;
                };
                let value = self.svt.value.clamp(0.01, 10.0);
                let p = &mut self.svt.params;
                (p.lines, p.hits, p.snaps) = (!preset.on_notes, preset.on_notes, false);
                p.write_sv = true;
                p.shape = match &preset.shape {
                    PresetShape::Curve(c) => Shape::Between(c.clone()),
                    PresetShape::Scale => Shape::Scale(value),
                };
            }
            SAct::Apply => {
                let (segments, _) = self.svt_segments();
                let params = self.svt.params.clone();
                self.status = match self.svt_commit(&segments, &params) {
                    Ok(said) => said,
                    Err(e) => e,
                };
            }
        }
    }
}
