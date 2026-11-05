//! SVT on the velocity graph: the curves the chart's green lines already follow, drawn over the
//! graph with handles to reshape them, a brush that draws a new one across the notes, a volume
//! view, and the panel with SVT's settings.
//!
//! Everything it writes goes through [`svt::plan`], so a curve drawn here, one typed into the panel
//! and one read back from the chart are the same thing (`sv_curves::detect_runs` finds the same
//! curve again). Nothing touches the chart until a drag ends or Apply is pressed, and each is one
//! undo step.

use super::chrome::{DIM, FAINT};
use super::{Btn, Drag, Field, GREEN_LINE, Keys, View, fmt_time};
use crate::editor::chart::TimingPoint;
use crate::editor::sv_curves::{self, Anchor, Curve, MAX_EXPONENT, MIN_EXPONENT, Run, RunKind};
use crate::editor::svt::{self, Segment, Shape, SvtParams};
use crate::editor::timing;
use crate::editor::tools::Plan;
use crate::render::{Canvas, accent, rgba};

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
    WriteSv,
    WriteVolume,
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
            let mut px = x0.max(gx);
            while px <= x1.min(gx + gw) {
                let t = self.t_at(px).clamp(run.start.time, run.end.time);
                let y = self.sv_y(run.curve_at(t), range);
                c.rect(px - 0.75, y - 0.75, 1.5, 1.5, rgba(col, 0.95));
                px += 2.0;
            }
            if !editing {
                continue;
            }
            if picked {
                for (end, a) in [(false, &run.start), (true, &run.end)] {
                    let (hx, hy) = (self.x_of(a.time), self.sv_y(a.value, range));
                    c.circle(hx, hy, 5.0, rgba(WHITE, 1.0), rgba(accent(), 1.0), 1.5);
                    self.svt
                        .handles
                        .push(([hx - 7.0, hy - 7.0, 14.0, 14.0], end));
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
                let hover = c.hovered(cx - w / 2.0, cy, w, 10.0);
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
        let t = self.grid_or_free(self.t_at(x), keys);
        let v = self.y_val(y, range);
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
        let range = self.graph_range();
        self.drag = Drag::Brush {
            t0,
            v0,
            t1: self.grid_or_free(self.t_at(x), keys),
            v1: self.y_val(y, range),
        };
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
        let greens: Vec<&TimingPoint> = self
            .selected_lines()
            .iter()
            .filter_map(|id| self.ed.chart.timing_point(*id))
            .filter(|p| !p.is_red())
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
            "Brush: drag across the graph to draw a curve from one speed to another over the notes, grid or lines under it (set what it places on in the SV tool). Volume shows and edits each line's volume instead of its speed.".to_string(),
        ));
    }

    // ---- the panel ----

    fn svt_value(&self, f: SF) -> f64 {
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

    /// SVT's settings, the segment they work on and Apply.
    pub(super) fn draw_svt_panel(&mut self, c: &mut Canvas, x: f32, mut y: f32, w: f32) {
        self.svt.live = true;
        self.check_picked();
        self.refresh_ghost();
        let (segments, said) = self.svt_segments();
        let p = self.svt.params.clone();
        let kind = shape_kind(&p.shape);
        let flat = p.shape.is_flat();
        let picked = self.picked_run();
        let head = match &picked {
            Some((run, lines)) => match &run.kind {
                RunKind::Curve(k) => format!(
                    "Curve picked: {} x{:.2} > x{:.2}, {} lines. Drag its ends on the graph, or pick another shape below.",
                    curve_name(k),
                    run.start.value,
                    run.end.value,
                    lines.len()
                ),
                RunKind::Step => said.clone(),
            },
            None => said.clone(),
        };
        c.text(x, y, 6.5, rgba(DIM, 1.0), &Canvas::fit(6.5, w, &head));
        y += 11.0;
        let items: Vec<(String, Btn, bool)> = SHAPES
            .iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    name.to_string(),
                    Btn::S(SAct::Shape(i as u8)),
                    i as u8 == kind,
                )
            })
            .collect();
        let shapes_y = y;
        y = self.wrap_row(c, x, y, w, &items);
        self.tips.push((
            [x, shapes_y, w, y - shapes_y],
            "How the speed changes from the start to the end.\nLinear: evenly. Geometric: by the same factor each step (reads as an even acceleration). Poly: t^n, below 1 starts fast and settles. Sine: eases in and out.\nFlat + / x: add to or multiply the green lines already in range.\nFan: alternates around the start."
                .to_string(),
        ));
        let fw = ((w - 12.0) / 4.0).min(150.0);
        let mut cells: Vec<SF> = Vec::new();
        if flat {
            cells.push(SF::Value);
        } else {
            if p.write_sv {
                cells.extend([SF::FromSv, SF::ToSv]);
            }
            if p.write_volume {
                cells.extend([SF::FromVol, SF::ToVol]);
            }
            if kind == 2 {
                cells.push(SF::Exponent);
            }
        }
        for (i, f) in cells.iter().enumerate() {
            self.svt_field(c, x + i as f32 * (fw + 4.0), y, fw, *f);
        }
        y += 18.0;
        let snap = format!("Snaps {}/{}", p.snap_numer as i64, p.snap_denom as i64);
        let mut ax = self.row(
            c,
            x,
            y,
            &[
                ("SV".into(), Btn::S(SAct::WriteSv), p.write_sv),
                ("Volume".into(), Btn::S(SAct::WriteVolume), p.write_volume),
            ],
        ) + 8.0;
        c.text(ax, y + 4.0, 6.0, rgba(FAINT, 1.0), "on");
        ax += 12.0;
        let end = self.row(
            c,
            ax,
            y,
            &[
                ("Notes".into(), Btn::S(SAct::Hits), p.hits && !flat),
                (snap, Btn::S(SAct::Snaps), p.snaps && !flat),
                ("Green lines".into(), Btn::S(SAct::Lines), p.lines || flat),
            ],
        );
        if p.snaps && !flat {
            self.svt_field(c, end + 4.0, y + 1.0, 70.0, SF::SnapNumer);
            self.svt_field(c, end + 78.0, y + 1.0, 70.0, SF::SnapDenom);
        }
        self.tips.push((
            [x, y, w, 16.0],
            "Where the new lines go: on each note, on the beat grid (n/d of a beat), or on the green lines already there. Flat changes always use the lines already there.".to_string(),
        ));
        y += 19.0;
        for (i, f) in [SF::Offset, SF::Buffer, SF::Spacing].iter().enumerate() {
            self.svt_field(c, x + i as f32 * (fw + 4.0), y, fw, *f);
        }
        self.tips.push((
            [x, y, 3.0 * (fw + 4.0), 14.0],
            "Offset: ms from the object a line goes (negative: before it).\nBuffer: ms around the segment that count as in it, for notes that aren't perfectly snapped.\nMin spacing: lines closer than this collapse, and old green lines this close to a new one are replaced."
                .to_string(),
        ));
        let mut toggles = vec![
            (
                "Ignore BPM".to_string(),
                Btn::S(SAct::IgnoreBpm),
                p.ignore_bpm,
            ),
            ("Relative".to_string(), Btn::S(SAct::Relative), p.relative),
        ];
        if kind == 8 {
            let per_pair = matches!(p.shape, Shape::Fan { per_pair: true, .. });
            toggles.push(("Per pair".to_string(), Btn::S(SAct::PerPair), per_pair));
        }
        self.row(c, x + 3.0 * (fw + 4.0) + 4.0, y - 1.0, &toggles);
        y += 19.0;
        let presets: Vec<(String, Btn, bool)> = [
            "Ramp selected lines",
            "Geometric ramp lines",
            "Multiply selected",
            "Every note",
        ]
        .iter()
        .enumerate()
        .map(|(i, n)| (n.to_string(), Btn::S(SAct::Preset(i as u8)), false))
        .collect();
        let end = self.row(c, x, y, &presets);
        self.tips.push((
            [x, y, end - x, 16.0],
            "Setups for the usual jobs. They only change the settings above; Apply writes."
                .to_string(),
        ));
        y += 19.0;
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
        let bx = x + w - 112.0;
        c.text(
            x,
            y + 4.0,
            6.5,
            rgba(if ok { GREEN_LINE } else { 0xfcb806 }, 1.0),
            &Canvas::fit(6.5, bx - x - 6.0, &note),
        );
        self.button(
            c,
            [bx - 74.0, y, 70.0, 16.0],
            "From chart",
            Btn::S(SAct::TakeChart),
            false,
        );
        self.button(
            c,
            [bx, y, 112.0, 16.0],
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
            SAct::WriteSv => {
                let p = &mut self.svt.params;
                p.write_sv = !p.write_sv || !p.write_volume;
            }
            SAct::WriteVolume => {
                let p = &mut self.svt.params;
                p.write_volume = !p.write_volume || !p.write_sv;
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
                let value = self.svt.value.clamp(0.01, 10.0);
                let p = &mut self.svt.params;
                (p.lines, p.hits, p.snaps) = (i < 3, i >= 3, false);
                p.write_sv = true;
                p.shape = match i {
                    1 => Shape::Between(Curve::TrueExp),
                    2 => Shape::Scale(value),
                    _ => Shape::Between(Curve::Linear),
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
