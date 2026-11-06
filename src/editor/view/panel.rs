//! The Gimmicks panel: pick a structure, set its numbers, aim it at the playhead or the selected
//! notes, preview it on the playfield (it is really applied; Cancel undoes it) and keep it.
//! Slider velocity curves are the Timing mode's SV tool.

use super::keys::KAct;
use super::{Btn, Mode, View};
use crate::editor::gimmicks::GimmickConfig;
use crate::editor::tools::{GimmickTool, Target};
use crate::render::{Canvas, rgba};

/// A number a gimmick tool takes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GParam {
    GimmickBpm,
    Spacing,
    KatSpacing,
    Offset,
    ShinyOffset,
    ShinyCount,
    FakeSv,
    RedBpm,
    RedOffset,
    Meter,
    Lines,
    DonTicks,
    KatTicks,
    Window,
}

/// A switch a gimmick tool has.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GFlag {
    Hide,
    Omit,
    PlaceNote,
    Mirror,
    RestoreAfter,
    ShinyRed,
    SvCurves,
}

impl GParam {
    fn label(self) -> &'static str {
        match self {
            GParam::GimmickBpm => "Line BPM",
            GParam::Spacing => "Spacing ms",
            GParam::KatSpacing => "Kat spacing ms",
            GParam::Offset => "Offset ms",
            GParam::ShinyOffset => "Glow offset ms",
            GParam::ShinyCount => "Glow count",
            GParam::FakeSv => "Hide SV",
            GParam::RedBpm => "BPM",
            GParam::RedOffset => "Offset ms",
            GParam::Meter => "Meter",
            GParam::Lines => "Bars/beat",
            GParam::DonTicks => "Don slit",
            GParam::KatTicks => "Kat slit",
            GParam::Window => "Slit window 1/",
        }
    }
}

impl GParam {
    /// One line on what the number does ("" where the name says it).
    fn about(self) -> &'static str {
        match self {
            GParam::GimmickBpm => "The BPM of the gimmick lines this tool writes.",
            GParam::Spacing => "A don's mirrored restore pair sits at plus and minus this many ms.",
            GParam::KatSpacing => {
                "The gap in ms between a kat's lines; the later ones follow at +2 and +4."
            }
            GParam::Offset => "How many ms from the note the fake slider is put.",
            GParam::ShinyOffset => "How many ms from the note the glow lines are put.",
            GParam::ShinyCount => "How many glow lines are written.",
            GParam::FakeSv => "The slider velocity the fake slider is written with.",
            GParam::RedBpm => "The BPM of the red line. \"base\" keeps the map's own.",
            GParam::RedOffset => "Moves the red line this many ms from where it is applied.",
            GParam::Meter => "Beats per bar of the red line.",
            GParam::Lines => "How many barlines per beat the wall is made of.",
            GParam::DonTicks => "How many slits in the wall a don gets.",
            GParam::KatTicks => "How many slits in the wall a kat gets.",
            GParam::Window => "The hidden window is 1/this of a beat.",
        }
    }
}

impl GFlag {
    /// One line on what the switch does ("" where the name says it).
    fn about(self) -> &'static str {
        match self {
            GFlag::Hide => "Squash the real note out of sight: the structure takes its place.",
            GFlag::Omit => "Write the lines without a barline.",
            GFlag::RestoreAfter => "Put the speed back after the structure.",
            GFlag::ShinyRed => "Write a red line for the glow.",
            GFlag::SvCurves => {
                "Read the chart's SV as the curves it was drawn with (ramps between notes) instead of steps."
            }
            GFlag::PlaceNote | GFlag::Mirror => "",
        }
    }

    fn label(self) -> &'static str {
        match self {
            GFlag::Hide => "Hide the note",
            GFlag::Omit => "Omit bar",
            GFlag::PlaceNote => "Place note",
            GFlag::Mirror => "Mirror kat",
            GFlag::RestoreAfter => "Restore after",
            GFlag::ShinyRed => "Glow line",
            GFlag::SvCurves => "SV curves",
        }
    }
}

/// The numbers and switches a tool shows.
fn controls(tool: GimmickTool) -> (&'static [GParam], &'static [GFlag]) {
    match tool {
        GimmickTool::BarlineNote => (
            &[GParam::GimmickBpm, GParam::Spacing, GParam::KatSpacing],
            &[GFlag::Hide, GFlag::PlaceNote, GFlag::Mirror],
        ),
        GimmickTool::FakeSlider | GimmickTool::FakeBig => {
            (&[GParam::Offset], &[GFlag::Omit, GFlag::RestoreAfter])
        }
        GimmickTool::Shiny => (
            &[GParam::ShinyCount, GParam::ShinyOffset],
            &[GFlag::Omit, GFlag::ShinyRed],
        ),
        GimmickTool::FakeNote => (
            &[GParam::Offset, GParam::FakeSv, GParam::GimmickBpm],
            &[GFlag::Hide, GFlag::Omit],
        ),
        GimmickTool::RedLine => (&[GParam::RedBpm, GParam::RedOffset, GParam::Meter], &[]),
        GimmickTool::AntiBarline => (
            &[
                GParam::Lines,
                GParam::DonTicks,
                GParam::KatTicks,
                GParam::GimmickBpm,
            ],
            &[GFlag::Hide, GFlag::SvCurves],
        ),
        GimmickTool::HiddenAnti => (&[GParam::Window], &[GFlag::Hide]),
    }
}

fn tool_offset_is_shiny(tool: GimmickTool) -> bool {
    tool == GimmickTool::Shiny
}

impl View {
    fn param_text(&self, p: GParam) -> String {
        let c = &self.ed.gimmick;
        let v = match p {
            GParam::GimmickBpm => format!("{}", c.gimmick_bpm),
            GParam::Spacing => c.spacing_ms.to_string(),
            GParam::KatSpacing => c.kat_spacing1_ms.to_string(),
            GParam::Offset => c.fake_slider_offset_ms.to_string(),
            GParam::ShinyOffset => c.shiny_offset_ms.to_string(),
            GParam::ShinyCount => c.shiny_count.to_string(),
            GParam::FakeSv => format!("x{}", c.fake_slider_sv),
            GParam::RedBpm => c
                .red_line_bpm
                .map_or("base".to_string(), |b| format!("{b}")),
            GParam::RedOffset => c.red_line_offset_ms.to_string(),
            GParam::Meter => c.red_line_meter.to_string(),
            GParam::Lines => c.anti_lines_per_beat.to_string(),
            GParam::DonTicks => c.anti_don_ticks.to_string(),
            GParam::KatTicks => c.anti_kat_ticks.to_string(),
            GParam::Window => c.hidden_window_divisor.to_string(),
        };
        format!("{}: {v}", p.label())
    }

    fn flag_on(&self, f: GFlag) -> bool {
        let c = &self.ed.gimmick;
        match f {
            GFlag::Hide => c.hide_note,
            GFlag::Omit => c.omit_barline,
            GFlag::PlaceNote => c.place_notes,
            GFlag::Mirror => c.mirror_kat_lines,
            GFlag::RestoreAfter => c.fake_slider_restore_after,
            GFlag::ShinyRed => c.shiny_red_line,
            GFlag::SvCurves => self.ed.sv_curves,
        }
    }

    pub(super) fn toggle_flag(&mut self, f: GFlag) {
        let on = !self.flag_on(f);
        let c = &mut self.ed.gimmick;
        match f {
            GFlag::Hide => c.hide_note = on,
            GFlag::Omit => c.omit_barline = on,
            GFlag::PlaceNote => c.place_notes = on,
            GFlag::Mirror => c.mirror_kat_lines = on,
            GFlag::RestoreAfter => c.fake_slider_restore_after = on,
            GFlag::ShinyRed => c.shiny_red_line = on,
            GFlag::SvCurves => self.ed.sv_curves = on,
        }
    }

    pub(super) fn step_param(&mut self, p: GParam, dir: f64, big: bool) {
        let k = if big { 10.0 } else { 1.0 };
        let tool = self.g_tool;
        let c: &mut GimmickConfig = &mut self.ed.gimmick;
        let whole = |v: i64, step: f64, min: i64, max: i64| -> i64 {
            (v + (dir * step * k) as i64).clamp(min, max)
        };
        match p {
            GParam::GimmickBpm => c.gimmick_bpm = (c.gimmick_bpm + dir * 1000.0 * k).max(100.0),
            GParam::Spacing => c.spacing_ms = whole(c.spacing_ms, 1.0, 1, 1000),
            GParam::KatSpacing => {
                let n = whole(c.kat_spacing1_ms, 1.0, 1, 1000);
                c.kat_spacing1_ms = n;
                c.kat_spacing2_ms = n + 2;
                c.kat_spacing3_ms = n + 4;
            }
            GParam::Offset => {
                if tool_offset_is_shiny(tool) {
                    c.shiny_offset_ms = whole(c.shiny_offset_ms, 1.0, 1, 1000);
                } else {
                    c.fake_slider_offset_ms = whole(c.fake_slider_offset_ms, 1.0, 1, 1000);
                }
            }
            GParam::ShinyOffset => c.shiny_offset_ms = whole(c.shiny_offset_ms, 1.0, 1, 1000),
            GParam::ShinyCount => {
                c.shiny_count = whole(c.shiny_count as i64, 1.0, 1, 16) as usize;
            }
            GParam::FakeSv => c.fake_slider_sv = (c.fake_slider_sv + dir * 0.5 * k).max(0.1),
            GParam::RedBpm => {
                let now = c.red_line_bpm.unwrap_or(0.0);
                let next = if now == 0.0 {
                    if dir > 0.0 { 180.0 } else { 0.0 }
                } else {
                    now + dir * 10.0 * k
                };
                c.red_line_bpm = (next >= 1.0).then_some(next);
            }
            GParam::RedOffset => {
                c.red_line_offset_ms = whole(c.red_line_offset_ms, 1.0, -1000, 1000)
            }
            GParam::Meter => c.red_line_meter = whole(c.red_line_meter as i64, 1.0, 1, 64) as i32,
            GParam::Lines => {
                c.anti_lines_per_beat = whole(c.anti_lines_per_beat as i64, 6.0, 1, 512) as i32;
            }
            GParam::DonTicks => {
                c.anti_don_ticks = whole(c.anti_don_ticks as i64, 1.0, 1, 32) as usize
            }
            GParam::KatTicks => {
                c.anti_kat_ticks = whole(c.anti_kat_ticks as i64, 1.0, 1, 32) as usize
            }
            GParam::Window => {
                c.hidden_window_divisor = whole(c.hidden_window_divisor as i64, 1.0, 1, 64) as i32;
            }
        }
    }

    /// One row of "-  Label: value  +" controls. Returns the next y.
    fn param_row(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, params: &[GParam]) -> f32 {
        if params.is_empty() {
            return y;
        }
        let cell = (w / params.len() as f32).min(180.0);
        for (i, p) in params.iter().enumerate() {
            let cx = x + cell * i as f32;
            let text = self.param_text(*p);
            let tw = cell - 40.0;
            self.button(c, [cx, y, 14.0, 14.0], "-", Btn::GStep(*p, -1), false);
            c.text(
                cx + 18.0,
                y + 4.0,
                6.5,
                rgba(0xffffff, 1.0),
                &Canvas::fit(6.5, tw, &text),
            );
            self.button(
                c,
                [cx + cell - 20.0, y, 14.0, 14.0],
                "+",
                Btn::GStep(*p, 1),
                false,
            );
            if !p.about().is_empty() {
                self.tips
                    .push(([cx + 16.0, y, cell - 38.0, 14.0], p.about().to_string()));
            }
        }
        y + 18.0
    }

    fn flag_row(&mut self, c: &mut Canvas, x: f32, y: f32, flags: &[GFlag]) -> f32 {
        let items: Vec<(String, Btn, bool)> = flags
            .iter()
            .map(|f| (f.label().to_string(), Btn::GFlag(*f), self.flag_on(*f)))
            .collect();
        if items.is_empty() {
            return y;
        }
        self.row(c, x, y, &items);
        let first = self.buttons.len() - items.len();
        for (i, f) in flags.iter().enumerate() {
            if !f.about().is_empty() {
                let r = self.buttons[first + i].0;
                self.tips.push((r, format!("{}\n{}", f.label(), f.about())));
            }
        }
        y + 20.0
    }

    /// What a plain .osu sees on the Effects pages: what a .tko is, and the button that makes one.
    pub(super) fn draw_tko_explainer(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32) {
        c.text(x, y, 8.0, rgba(0xffffff, 1.0), "This map is a plain .osu");
        let y = self.paragraph(
            c,
            x,
            y + 14.0,
            w,
            6.5,
            0x9a9ab0,
            "Effects animate the playfield and the notes: keyframes move things over time, nodes react to the beat and to how the player plays.",
        );
        let y = self.paragraph(
            c,
            x,
            y + 3.0,
            w,
            6.5,
            0x9a9ab0,
            "They only exist in this client. Making a .tko saves a copy next to your .osu and leaves the .osu exactly as it is, so it still plays the same in osu!stable.",
        );
        self.button(
            c,
            [x, y + 6.0, 120.0, 18.0],
            "Make a .tko copy",
            Btn::K(KAct::MakeTko),
            true,
        );
    }

    pub(super) fn draw_gimmicks(&mut self, c: &mut Canvas) {
        let effects = self.mode == Mode::Effects;
        let title = if effects {
            "EFFECTS (.tko)"
        } else {
            "GIMMICKS"
        };
        let (x, mut y) = self.panel_frame(c, self.layout.inspector, title);
        let w = self.layout.inspector[2];
        let inner = w - 16.0;
        let dim = rgba(0x9a9ab0, 1.0);
        if effects {
            // The pages sit in the title row, so the content gets the whole panel. Keyframes and
            // nodes are the advanced ways: shown on asking.
            let mut tabs = vec![("Clips".to_string(), Btn::GPage(4), self.g_page == 4)];
            if self.fx_advanced || self.g_page < 4 {
                tabs.push(("Keyframes".to_string(), Btn::GPage(2), self.g_page == 2));
                tabs.push(("Nodes".to_string(), Btn::GPage(3), self.g_page == 3));
            } else {
                tabs.push((
                    "Advanced...".to_string(),
                    Btn::Fx(super::clips::FxAct::Advanced),
                    false,
                ));
            }
            let ty = self.layout.inspector[1] + 0.5;
            self.row(c, x + 100.0, ty, &tabs);
        }
        if self.g_page == 4 {
            self.draw_clips_page(c, x, y, inner);
        } else if self.g_page == 3 {
            self.draw_nodes_page(c, x, y, inner);
        } else if self.g_page == 2 {
            self.draw_keys_page(c, x, y, inner);
        } else {
            let tools: Vec<(String, Btn, bool)> = GimmickTool::ALL
                .iter()
                .map(|t| (t.label().to_string(), Btn::GTool(*t), self.g_tool == *t))
                .collect();
            let (a, b) = tools.split_at(4);
            let first = self.buttons.len();
            self.row(c, x, y, a);
            self.row(c, x, y + 19.0, b);
            for (i, t) in GimmickTool::ALL.iter().enumerate() {
                let r = self.buttons[first + i].0;
                self.tips.push((r, format!("{}\n{}", t.label(), t.help())));
            }
            y += 40.0;
            c.text(x, y, 6.0, dim, &Canvas::fit(6.0, inner, self.g_tool.help()));
            y += 10.0;
            let (params, flags) = controls(self.g_tool);
            y = self.param_row(c, x, y, inner, params);
            y = self.flag_row(c, x, y, flags);
            let wall = self.g_tool.is_wall();
            let mut items = Vec::new();
            if wall {
                items.push((
                    "Apply to selection".to_string(),
                    Btn::GApply(Target::Selection),
                    false,
                ));
            } else {
                items.push((
                    "At playhead: don".to_string(),
                    Btn::GApply(Target::Playhead(false)),
                    false,
                ));
                items.push((
                    "kat".to_string(),
                    Btn::GApply(Target::Playhead(true)),
                    false,
                ));
                items.push((
                    "On selection".to_string(),
                    Btn::GApply(Target::Selection),
                    false,
                ));
            }
            let ex = self.row(c, x, y, &items);
            if self.g_pending {
                self.row(
                    c,
                    ex + 6.0,
                    y,
                    &[
                        ("Keep".to_string(), Btn::GKeep, true),
                        ("Cancel".to_string(), Btn::GCancel, false),
                    ],
                );
            } else {
                self.row(
                    c,
                    ex + 6.0,
                    y,
                    &[("Preview".to_string(), Btn::GPreview, false)],
                );
            }
            y += 20.0;
            let base = if self.ed.base_looks_gimmicked() {
                "Base timing already has gimmick lines. "
            } else {
                ""
            };
            c.text(
                x,
                y,
                6.0,
                if base.is_empty() {
                    dim
                } else {
                    rgba(0xfcb806, 1.0)
                },
                &Canvas::fit(
                    6.0,
                    inner - 126.0,
                    &format!("{base}BPM and SV come from the base timing."),
                ),
            );
            self.button(
                c,
                [x + inner - 120.0, y - 3.0, 120.0, 14.0],
                "Use current as base",
                Btn::GBase,
                false,
            );
        }
    }

    // ---- actions ----

    pub(super) fn gimmick_apply(&mut self, target: Target, preview: bool) {
        self.settle_preview();
        match self.ed.apply_gimmick(self.g_tool, target) {
            Ok(said) => {
                self.status = format!("{}: {said}", self.g_tool.label());
                self.g_pending = preview;
                self.mark_stale();
            }
            Err(e) => self.status = e,
        }
    }

    /// An earlier preview that wasn't cancelled is kept.
    pub(super) fn settle_preview(&mut self) {
        self.g_pending = false;
    }

    pub(super) fn gimmick_preview(&mut self) {
        let target = if self.g_tool.is_wall() {
            Target::Selection
        } else {
            Target::Playhead(false)
        };
        self.gimmick_apply(target, true);
    }

    pub(super) fn gimmick_cancel(&mut self) {
        if std::mem::take(&mut self.g_pending) && self.ed.undo() {
            self.mark_stale();
            self.status = "preview cancelled".into();
        }
    }
}
