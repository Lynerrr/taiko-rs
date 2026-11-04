//! The gimmick tools as the editor runs them: pick a structure, aim it at the playhead or at the
//! selected notes, and it lands as one undo step. Also Generate SV.
//!
//! The builders in `gimmicks` and `sv_curves` do the arithmetic; this is where they meet the chart:
//! the base timing snapshot they read, the kiai and volume carried onto what they write, the SV
//! lines a wall replaces, and the checks before anything is committed.

use super::chart::{HitObject, Taiko, TimingPoint};
use super::gimmicks::{self, FakeKind, GimmickConfig, NoteKind};
use super::history::Command;
use super::state::Editor;
use super::sv_curves::{self, Curve, StepSv, SvModel, SvSource};
use super::timing;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GimmickTool {
    BarlineNote,
    FakeSlider,
    FakeBig,
    Shiny,
    FakeNote,
    RedLine,
    AntiBarline,
    HiddenAnti,
}

impl GimmickTool {
    pub const ALL: [GimmickTool; 8] = [
        GimmickTool::BarlineNote,
        GimmickTool::FakeSlider,
        GimmickTool::FakeBig,
        GimmickTool::Shiny,
        GimmickTool::FakeNote,
        GimmickTool::RedLine,
        GimmickTool::AntiBarline,
        GimmickTool::HiddenAnti,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GimmickTool::BarlineNote => "Barline note",
            GimmickTool::FakeSlider => "Fake slider",
            GimmickTool::FakeBig => "Big fake",
            GimmickTool::Shiny => "Shiny",
            GimmickTool::FakeNote => "Hidden note",
            GimmickTool::RedLine => "Red line",
            GimmickTool::AntiBarline => "Anti-barline",
            GimmickTool::HiddenAnti => "Hidden anti",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            GimmickTool::BarlineNote => "a note drawn out of barlines (don: one bar, kat: three)",
            GimmickTool::FakeSlider => "a slider of negative length: draws, can't be hit",
            GimmickTool::FakeBig => "the same, as the big (finisher) body",
            GimmickTool::Shiny => "a white glow of fake sliders beside a note that stays",
            GimmickTool::FakeNote => "hides the note and leaves only a fake slider (kat: big)",
            GimmickTool::RedLine => "one red line: a standalone speed change",
            GimmickTool::AntiBarline => "a solid wall of bars with each note a slit in it",
            GimmickTool::HiddenAnti => "the same wall drawn by osu!: four lines per note",
        }
    }

    /// Builds from a range of notes rather than at single times.
    pub fn is_wall(self) -> bool {
        matches!(self, GimmickTool::AntiBarline | GimmickTool::HiddenAnti)
    }
}

/// Where a tool lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// At the playhead, a don.
    Playhead(bool),
    /// On each selected note (a wall: on the selection's range, or the whole chart).
    Selection,
}

/// What a tool would write.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub remove: Vec<TimingPoint>,
    pub timing: Vec<TimingPoint>,
    pub objects: Vec<HitObject>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.timing.is_empty() && self.objects.is_empty()
    }

    pub fn summary(&self) -> String {
        let (r, n) = (self.timing.len(), self.objects.len());
        let gone = if self.remove.is_empty() {
            String::new()
        } else {
            format!(", {} SV lines replaced", self.remove.len())
        };
        format!("{r} lines, {n} objects{gone}")
    }
}

/// How Generate SV fills the range.
#[derive(Clone, Debug, PartialEq)]
pub struct SvParams {
    pub from: f64,
    pub to: f64,
    pub curve: Curve,
    /// One line per 1/`divisor` of a beat.
    pub divisor: u32,
    /// Fan out from `from` by `to - from` alternating side instead of sweeping.
    pub oscillate: bool,
    pub per_pair: bool,
    /// Multiply the speed already there instead of replacing it.
    pub relative: bool,
}

impl Default for SvParams {
    fn default() -> Self {
        SvParams {
            from: 1.0,
            to: 2.0,
            curve: Curve::Linear,
            divisor: 4,
            oscillate: false,
            per_pair: false,
            relative: false,
        }
    }
}

const MAX_SV_LINES: usize = 4000;

impl Editor {
    /// The timing the tools read: the snapshot, taken from the chart at first use.
    pub fn base_timing(&mut self) -> Vec<TimingPoint> {
        if self.base.is_none() {
            self.base = Some(self.chart.timing.clone());
        }
        self.base.clone().unwrap_or_default()
    }

    /// Takes the chart as it is now as the base (the mapper says this is the clean timing).
    pub fn reset_base(&mut self) {
        self.base = Some(self.chart.timing.clone());
    }

    pub fn base_looks_gimmicked(&self) -> bool {
        gimmicks::looks_gimmicked(self.base.as_deref().unwrap_or(&self.chart.timing))
    }

    fn notes_for(&self, target: Target) -> Vec<HitObject> {
        match target {
            Target::Selection => self.selected(),
            Target::Playhead(_) => Vec::new(),
        }
    }

    /// What `tool` would write at `target`, without touching the chart.
    pub fn plan_gimmick(&mut self, tool: GimmickTool, target: Target) -> Result<Plan, String> {
        let cfg: GimmickConfig = self.gimmick.clone();
        cfg.validate()?;
        let base = self.base_timing();
        let mut plan = Plan::default();

        if tool.is_wall() {
            let mut notes = self.notes_for(target);
            if notes.is_empty() {
                notes = self.chart.objects.clone();
            }
            notes.retain(|o| o.is_circle());
            if notes.is_empty() {
                return Err("no notes to build a wall around".into());
            }
            let first = notes.iter().map(|o| o.time).fold(f64::INFINITY, f64::min);
            let last = notes
                .iter()
                .map(|o| o.time)
                .fold(f64::NEG_INFINITY, f64::max);
            plan.timing = if tool == GimmickTool::AntiBarline {
                let times: Vec<f64> = self.chart.objects.iter().map(|o| o.time).collect();
                let model = self
                    .sv_curves
                    .then(|| SvModel::build(&base, &times, first, last, true));
                gimmicks::anti_barline(
                    &notes,
                    first,
                    last,
                    &base,
                    &cfg,
                    model.as_ref().map(|m| m as &dyn SvSource),
                )
            } else {
                gimmicks::hidden_anti_barline(&notes, first, last, &base, &cfg)
            };
            // The wall writes the chart's SV itself, so the lines it covers are replaced.
            if let (Some(a), Some(b)) = (
                plan.timing.iter().map(|p| p.time).reduce(f64::min),
                plan.timing.iter().map(|p| p.time).reduce(f64::max),
            ) {
                plan.remove = self
                    .chart
                    .timing
                    .iter()
                    .filter(|p| !p.is_red() && p.time >= a && p.time <= b)
                    .cloned()
                    .collect();
            }
        } else {
            // One structure per time: the playhead's, or each selected plain note's.
            let spots: Vec<(i64, bool, f64, f64)> = match target {
                Target::Playhead(kat) => {
                    vec![(self.snapped(self.time) as i64, kat, 256.0, 192.0)]
                }
                Target::Selection => self
                    .selected()
                    .iter()
                    .filter(|o| o.is_circle())
                    .map(|o| (o.time as i64, o.taiko() == Taiko::Kat, o.x, o.y))
                    .collect(),
            };
            if spots.is_empty() {
                return Err("select a don or kat first".into());
            }
            let on_notes = target == Target::Selection;
            for (t, kat, x, y) in spots {
                let mut built = match tool {
                    GimmickTool::BarlineNote => {
                        let mut c = cfg.clone();
                        // On an existing note the note is already there.
                        c.place_notes = c.place_notes && !on_notes;
                        gimmicks::barline_note(
                            t,
                            &base,
                            &c,
                            if kat { NoteKind::Kat } else { NoteKind::Don },
                        )
                    }
                    GimmickTool::FakeSlider | GimmickTool::FakeBig => gimmicks::fake_slider(
                        t,
                        &base,
                        &cfg,
                        FakeKind::Regular,
                        x,
                        y,
                        false,
                        None,
                        tool == GimmickTool::FakeBig,
                    ),
                    GimmickTool::Shiny => gimmicks::fake_slider(
                        t,
                        &base,
                        &cfg,
                        if kat { FakeKind::Kat } else { FakeKind::Don },
                        x,
                        y,
                        true,
                        None,
                        false,
                    ),
                    GimmickTool::FakeNote => gimmicks::fake_slider(
                        t,
                        &base,
                        &cfg,
                        if kat { FakeKind::Kat } else { FakeKind::Don },
                        x,
                        y,
                        false,
                        None,
                        false,
                    ),
                    GimmickTool::RedLine => {
                        gimmicks::red_line(t, gimmicks::base_bpm_at(&base, t as f64), &cfg)
                    }
                    _ => unreachable!("walls are built above"),
                };
                if on_notes {
                    // The note under it is the chart's own.
                    built.objects.retain(|o| !o.is_circle());
                }
                plan.timing.extend(built.timing);
                plan.objects.extend(built.objects);
            }
        }
        // A circle on a millisecond that already holds one would sit invisible under it.
        let taken: std::collections::HashSet<i64> = self
            .chart
            .objects
            .iter()
            .filter(|o| o.is_circle())
            .map(|o| o.time as i64)
            .collect();
        plan.objects
            .retain(|o| !(o.is_circle() && taken.contains(&(o.time as i64))));
        gimmicks::carry_active_state(&mut plan.timing, &self.chart.timing);
        for p in &plan.timing {
            if let Some(why) = p.problem() {
                return Err(why);
            }
        }
        Ok(plan)
    }

    /// Runs a plan as one undo step.
    pub fn commit_plan(&mut self, plan: Plan) {
        let mut steps = Vec::new();
        if !plan.remove.is_empty() {
            steps.push(Command::RemoveTiming(plan.remove));
        }
        if !plan.timing.is_empty() {
            steps.push(Command::InsertTiming(plan.timing));
        }
        if !plan.objects.is_empty() {
            steps.push(Command::InsertObjects(plan.objects));
        }
        if !steps.is_empty() {
            self.history.run(&mut self.chart, Command::Composite(steps));
        }
    }

    /// Plans and commits. Returns what was written.
    pub fn apply_gimmick(&mut self, tool: GimmickTool, target: Target) -> Result<String, String> {
        let plan = self.plan_gimmick(tool, target)?;
        if plan.is_empty() {
            return Err("nothing to write".into());
        }
        let said = plan.summary();
        self.commit_plan(plan);
        if let Err(why) = self.chart.validate() {
            self.history.undo(&mut self.chart);
            return Err(why);
        }
        Ok(said)
    }

    /// The range Generate SV fills: the selection's, else the playhead to the end of its bar.
    pub fn sv_range(&self) -> (f64, f64) {
        let sel = self.selected();
        if sel.len() >= 2 {
            let a = sel.iter().map(|o| o.time).fold(f64::INFINITY, f64::min);
            let b = sel.iter().map(|o| o.time).fold(f64::NEG_INFINITY, f64::max);
            return (a, b);
        }
        let beat = timing::beat_length_at(&self.chart.timing, self.time);
        let start = self.snapped(self.time);
        (start, (start + 4.0 * beat).floor())
    }

    /// The green lines Generate SV would write for `p` over the range.
    pub fn plan_sv(&self, p: &SvParams) -> Result<Plan, String> {
        let (start, end) = self.sv_range();
        if end <= start {
            return Err("the range is empty".into());
        }
        let beat = timing::beat_length_at(&self.chart.timing, start);
        let step = beat / p.divisor.max(1) as f64;
        if !(step.is_finite() && step >= 1.0) {
            return Err("the snap is too fine for this section".into());
        }
        let count = (((end - start) / step).floor() as usize + 1).min(MAX_SV_LINES);
        if count < 2 {
            return Err("the range holds fewer than two lines".into());
        }
        let values = if p.oscillate {
            sv_curves::oscillating_series(p.from, p.to - p.from, count, &p.curve, p.per_pair)
        } else {
            sv_curves::sweep(p.from, p.to, count, &p.curve)
        };
        let existing = StepSv(&self.chart.timing);
        let mut plan = Plan::default();
        let mut last = None;
        for (i, v) in values.into_iter().enumerate() {
            let at = timing::osu_snap_ms(start + i as f64 * step) as f64;
            if last == Some(at) {
                continue;
            }
            last = Some(at);
            let v = if p.relative {
                v * existing.value_at(at)
            } else {
                v
            };
            let template = timing::active_at(&self.chart.timing, at);
            let sv = (v * 100.0).round() / 100.0;
            let sv = sv.clamp(0.01, 10.0);
            plan.timing.push(match template {
                Some(t) => TimingPoint::green_like(at, sv, t),
                None => TimingPoint::green(at, sv),
            });
        }
        // Greens on the same ms as a red must follow it: they are inserted after any line there,
        // which `Chart::insert_timing` guarantees. The ones this replaces:
        plan.remove = self
            .chart
            .timing
            .iter()
            .filter(|q| !q.is_red() && q.time >= start && q.time <= end)
            .cloned()
            .collect();
        Ok(plan)
    }

    pub fn generate_sv(&mut self, p: &SvParams) -> Result<String, String> {
        let plan = self.plan_sv(p)?;
        let said = plan.summary();
        self.commit_plan(plan);
        if let Err(why) = self.chart.validate() {
            self.history.undo(&mut self.chart);
            return Err(why);
        }
        Ok(said)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::{Chart, tests::MAP};
    use std::path::PathBuf;

    fn editor() -> Editor {
        Editor::new(Chart::parse(MAP), PathBuf::from("t.osu"))
    }

    #[test]
    fn a_tool_at_the_playhead_is_one_undo_step() {
        let mut e = editor();
        let (lines, objects) = (e.chart.timing.len(), e.chart.objects.len());
        e.time = 4000.0;
        let said = e
            .apply_gimmick(GimmickTool::BarlineNote, Target::Playhead(false))
            .unwrap();
        assert!(said.contains("2 lines"), "{said}");
        assert_eq!(e.chart.timing.len(), lines + 2);
        assert_eq!(e.chart.objects.len(), objects + 1);
        assert!(e.undo());
        assert_eq!(
            (e.chart.timing.len(), e.chart.objects.len()),
            (lines, objects)
        );
    }

    #[test]
    fn the_base_is_taken_once_and_not_from_the_gimmicked_chart() {
        let mut e = editor();
        e.time = 4000.0;
        e.apply_gimmick(GimmickTool::BarlineNote, Target::Playhead(false))
            .unwrap();
        let base_len = e.base.as_ref().unwrap().len();
        e.time = 8000.0;
        e.apply_gimmick(GimmickTool::BarlineNote, Target::Playhead(false))
            .unwrap();
        assert_eq!(e.base.as_ref().unwrap().len(), base_len);
        assert!(!e.base_looks_gimmicked());
        e.reset_base();
        assert!(
            e.base_looks_gimmicked(),
            "now the base holds 60000 BPM lines"
        );
    }

    #[test]
    fn a_tool_on_the_selection_writes_no_second_note() {
        let mut e = editor();
        let id = e.chart.objects.iter().find(|o| o.is_circle()).unwrap().id;
        e.selection = [id].into();
        let n = e.chart.objects.len();
        e.apply_gimmick(GimmickTool::BarlineNote, Target::Selection)
            .unwrap();
        assert_eq!(e.chart.objects.len(), n);
        e.selection.clear();
        assert!(
            e.apply_gimmick(GimmickTool::FakeSlider, Target::Selection)
                .is_err()
        );
    }

    #[test]
    fn a_wall_replaces_the_green_lines_it_covers_and_undoes_whole() {
        let mut e = editor();
        let before: Vec<(f64, f64, i32)> = e
            .chart
            .timing
            .iter()
            .map(|p| (p.time, p.beat_length, p.uninherited))
            .collect();
        let objects = e.chart.objects.len();
        e.apply_gimmick(GimmickTool::AntiBarline, Target::Selection)
            .unwrap();
        assert!(e.chart.timing.len() > before.len());
        assert_eq!(e.chart.objects.len(), objects, "a wall writes timing only");
        assert!(e.undo());
        let after: Vec<(f64, f64, i32)> = e
            .chart
            .timing
            .iter()
            .map(|p| (p.time, p.beat_length, p.uninherited))
            .collect();
        assert_eq!(after, before);
    }

    #[test]
    fn a_bad_configuration_is_refused_before_anything_is_written() {
        let mut e = editor();
        e.gimmick.gimmick_bpm = -1.0;
        let lines = e.chart.timing.len();
        assert!(
            e.apply_gimmick(GimmickTool::BarlineNote, Target::Playhead(true))
                .is_err()
        );
        assert_eq!(e.chart.timing.len(), lines);
    }

    #[test]
    fn generate_sv_writes_a_ramp_on_the_grid_and_replaces_what_it_covers() {
        let mut e = editor();
        e.time = 2000.0;
        let before = e.chart.timing.len();
        let p = SvParams {
            from: 1.0,
            to: 2.0,
            divisor: 2,
            ..SvParams::default()
        };
        let said = e.generate_sv(&p).unwrap();
        assert!(said.contains("lines"));
        let greens: Vec<(f64, f64)> = e
            .chart
            .timing
            .iter()
            .filter(|q| !q.is_red() && q.time >= 2000.0)
            .map(|q| (q.time, q.sv()))
            .collect();
        assert!(greens.len() >= 4, "{greens:?}");
        assert!((greens[0].1 - 1.0).abs() < 1e-9);
        assert!((greens.last().unwrap().1 - 2.0).abs() < 1e-9);
        assert!(
            greens
                .windows(2)
                .all(|w| w[1].1 >= w[0].1 && w[1].0 > w[0].0)
        );
        assert!(e.undo());
        assert_eq!(e.chart.timing.len(), before);
    }

    #[test]
    fn relative_sv_multiplies_what_is_there() {
        let mut e = editor();
        e.add_green(1900.0, 2.0).unwrap();
        e.time = 2000.0;
        let p = SvParams {
            from: 1.0,
            to: 1.0,
            relative: true,
            divisor: 1,
            ..SvParams::default()
        };
        e.generate_sv(&p).unwrap();
        let g = e
            .chart
            .timing
            .iter()
            .find(|q| q.time == 2000.0 && !q.is_red())
            .unwrap();
        assert!((g.sv() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn the_oscillating_mode_fans_out_around_the_base() {
        let mut e = editor();
        e.time = 2000.0;
        let p = SvParams {
            from: 1.0,
            to: 1.4,
            oscillate: true,
            divisor: 2,
            ..SvParams::default()
        };
        let plan = e.plan_sv(&p).unwrap();
        let v: Vec<f64> = plan.timing.iter().map(|q| q.sv()).collect();
        assert!((v[0] - 1.0).abs() < 1e-9);
        assert!(v[1] > 1.0 && v[2] < 1.0);
    }

    /// Every tool, on a real map, in the middle of it: the chart still validates and the game
    /// still reads what was written, and one undo takes it all back. Run with `--ignored`.
    #[test]
    #[ignore = "needs a folder of .osu files in TAIKO_CORPUS"]
    fn corpus_tools_leave_a_chart_the_game_reads() {
        let Some(dir) = std::env::var_os("TAIKO_CORPUS") else {
            panic!("set TAIKO_CORPUS to a folder of .osu files");
        };
        let (mut maps, mut ran) = (0, 0);
        for entry in std::fs::read_dir(dir).unwrap().flatten().take(120) {
            let path = entry.path();
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if path.extension().is_none_or(|e| e != "osu") {
                continue;
            }
            let chart = Chart::parse(&text);
            if chart.objects.len() < 8 || chart.timing.is_empty() {
                continue;
            }
            maps += 1;
            let mid = chart.objects[chart.objects.len() / 2].time;
            for tool in GimmickTool::ALL {
                let mut e = Editor::new(chart.clone(), path.clone());
                e.time = mid;
                // A few notes around the middle selected, the rest left alone.
                let ids: Vec<u64> = e
                    .chart
                    .objects
                    .iter()
                    .filter(|o| o.is_circle() && (o.time - mid).abs() < 4000.0)
                    .map(|o| o.id)
                    .collect();
                e.selection = ids.into_iter().collect();
                let target = if tool.is_wall() || tool == GimmickTool::RedLine {
                    Target::Selection
                } else {
                    Target::Playhead(false)
                };
                let before = e.chart.to_text();
                let Ok(said) = e.apply_gimmick(tool, target) else {
                    continue;
                };
                ran += 1;
                let name = path.display();
                assert!(e.chart.validate().is_ok(), "{name} {tool:?} {said}");
                let out = e.chart.to_text();
                assert!(
                    crate::beatmap::from_text(&out).is_ok(),
                    "{name} {tool:?}: the game can't read it"
                );
                assert!(e.undo());
                assert_eq!(e.chart.to_text(), before, "{name} {tool:?}: undo");
            }
            let mut e = Editor::new(chart.clone(), path.clone());
            e.time = mid;
            let before = e.chart.to_text();
            if e.generate_sv(&SvParams::default()).is_ok() {
                ran += 1;
                assert!(crate::beatmap::from_text(&e.chart.to_text()).is_ok());
                assert!(e.undo());
                assert_eq!(e.chart.to_text(), before);
            }
        }
        println!("{maps} maps, {ran} tool runs");
        assert!(maps > 0 && ran > 0);
    }
}
