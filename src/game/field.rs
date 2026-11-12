//! Drawing the playfield: lane, notes, drum, judgements and the mods' looks.

use super::*;
use crate::hit::Canvas;
use crate::hud::{self, Part};
use crate::medals::{SPLIT_CYAN, SPLIT_RED, TEAR_CYAN, TEAR_RED};
use crate::net::ItemText;
use crate::render::{Font, Shape, Sprite, VIRTUAL_HEIGHT, rgba};

/// What only drawing a play needs, apart from the play itself (`Game`): the HUD's layout and
/// what it hides, the playfield's boxes and path as last drawn, the notes on screen, and what the
/// visual mods worked out from the map. A headless play (a replay simulated for its results)
/// never reads it.
pub struct View {
    pub(super) min_velocity: f64,
    pub(super) note_blocks: Vec<NoteBlock>,
    pub(super) visible: Vec<usize>,
    /// Draw the HP bar (not in calibration).
    pub show_hp: bool,
    /// Show the live offset suggestion under the hit error meter.
    pub show_offset_hint: bool,
    /// The key that applies it, if it has one.
    pub offset_key: Option<String>,
    /// Kiai time tints the lane and pulses the hit target.
    pub kiai_effects: bool,
    /// Interface parts hidden (the interface toggle).
    pub hidden: crate::hud::Parts,
    /// Where the interface's parts go, and the playfield area they may be attached to.
    pub hud: crate::hud::Layout,
    pub hud_area: crate::layout::Area,
    /// A stand-in play for the interface editor: every part shows, with made-up numbers.
    pub(super) hud_sample: bool,
    /// Where mods have the lane this frame, for the parts attached to it.
    pub(super) field_boxes: Option<hud::FieldBoxes>,
    /// The lane's middle on the screen, in the playfield's units, while mods bend or move it, and
    /// how far its edges are from that.
    pub(super) lane_path: Vec<(f32, f32)>,
    pub(super) lane_half: f32,
    /// When `field_boxes` last moved, to ease it.
    pub(super) field_at: f64,
    /// Parts mods have moved off their places: their top and when it was worked out.
    pub(super) settled: std::cell::Cell<[Option<(f32, f64)>; Part::ALL.len()]>,
    /// Display latency to make up for (real ms): the lane is drawn as it will be this much later,
    /// when the frame reaches the screen. Judging stays on the audio clock.
    pub visual_offset: f64,
    /// Someone else's play being watched: no pause menu (it holds while they pause) and no
    /// skip prompt (it follows their skips).
    pub watched: bool,
    /// Text shown at the top ("REPLAY - x", "SPECTATING - x").
    pub banner: Option<String>,
    /// A test play from the map editor: leaving goes back to it.
    pub test_play: bool,
    /// Roulette: where each 4-bar section starts (map ms), its speed, and the velocity its
    /// marker scrolls at.
    pub(super) sections: Vec<(f64, f64, f64)>,
    /// Decoy: the fake notes (never judged).
    pub(super) decoys: Vec<beatmap::Note>,
    /// Decoy: the rhythm each fake note pretends to be part of (the note before it's).
    pub(super) decoy_flows: Vec<u8>,
    /// Liar: the map time each note is drawn at far off (its own, or a 1/4's for a triplet).
    pub(super) lies: Vec<f64>,
    /// Multiverse: where the focus moves to another playfield.
    pub(super) focus_switches: Vec<FocusSwitch>,
    /// Where the playfield in play was drawn last frame (x, y, w, h on the canvas): the lane,
    /// wherever the mods took it; with Multiverse, the playfield in focus.
    pub(super) zone: Option<[f32; 4]>,
}

impl View {
    /// A play's drawing state at its start: the default HUD, and what the visual mods worked out
    /// from the map (`Game::with_half`).
    pub(super) fn new(
        min_velocity: f64,
        note_blocks: Vec<NoteBlock>,
        sections: Vec<(f64, f64, f64)>,
        decoys: Vec<beatmap::Note>,
        decoy_flows: Vec<u8>,
        lies: Vec<f64>,
        focus_switches: Vec<FocusSwitch>,
    ) -> View {
        View {
            min_velocity,
            note_blocks,
            visible: Vec::new(),
            show_hp: true,
            show_offset_hint: false,
            offset_key: None,
            kiai_effects: true,
            hidden: hud::Layout::default().off,
            hud: hud::Layout::default(),
            hud_area: crate::layout::Area::FULL,
            hud_sample: false,
            field_boxes: None,
            lane_path: Vec::new(),
            lane_half: 0.0,
            field_at: 0.0,
            settled: std::cell::Cell::new([None; Part::ALL.len()]),
            visual_offset: 0.0,
            watched: false,
            banner: None,
            test_play: false,
            sections,
            decoys,
            decoy_flows,
            lies,
            focus_switches,
            zone: None,
        }
    }
}

impl View {
    /// A play's drawing state at its start: the default HUD, and what the visual mods work out
    /// from its map (decoys, lies, Roulette's sections, Multiverse's focus switches).
    pub(super) fn for_play(g: &Game) -> View {
        let (map, mods) = (g.map(), g.mods());
        // Floored only against 0: SV 0.02 at 120 BPM is already under 0.01, and a fake slider
        // in it is on screen ~40 s ahead, and nothing caps the lookahead (the whole map can be).
        let min_velocity = map
            .notes
            .iter()
            .map(|n| n.velocity)
            .chain(map.barlines.iter().map(|b| b.velocity))
            .fold(f64::INFINITY, f64::min)
            .max(1e-6);
        let sections = if mods.has(Mod::Roulette) {
            roulette_sections(map)
                .into_iter()
                .zip(map.barlines.iter().step_by(4))
                .map(|((t, k), b)| (t, k, b.velocity))
                .collect()
        } else {
            Vec::new()
        };
        let decoys = if mods.has(Mod::Decoy) {
            decoys(map)
        } else {
            Vec::new()
        };
        let decoy_flows = decoys
            .iter()
            .map(|d| {
                map.notes
                    .partition_point(|n| n.time < d.time)
                    .checked_sub(1)
                    .map_or(1, |i| g.flows()[i])
            })
            .collect();
        let lies = if mods.has(Mod::Liar) {
            lies(map)
        } else {
            Vec::new()
        };
        let min_velocity = decoys
            .iter()
            .map(|n| n.velocity)
            .fold(min_velocity, f64::min);
        let focus_switches = if mods.has(Mod::Multiverse) {
            focus_switches(map)
        } else {
            Vec::new()
        };
        let note_blocks = NoteBlock::index(&map.notes);
        View::new(
            min_velocity,
            note_blocks,
            sections,
            decoys,
            decoy_flows,
            lies,
            focus_switches,
        )
    }
}

/// A play as the client shows it: the play itself and what drawing it keeps. Reads and changes of
/// the play go straight through (`Deref`).
pub struct Play {
    pub game: Game,
    pub view: View,
}

impl Play {
    /// Where the playfield in play was drawn last frame (x, y, w, h on the canvas it was drawn
    /// on), for views that show only that (spectating everyone at once).
    pub fn play_zone(&self) -> Option<[f32; 4]> {
        self.view.zone
    }

    pub fn new(game: Game) -> Play {
        let view = View::for_play(&game);
        Play { game, view }
    }
}

impl std::ops::Deref for Play {
    type Target = Game;

    fn deref(&self) -> &Game {
        &self.game
    }
}

impl std::ops::DerefMut for Play {
    fn deref_mut(&mut self) -> &mut Game {
        &mut self.game
    }
}

/// A rect `len` long and `thick` across, centred on (`x`, `y`) and turned to heading `a`.
pub(super) fn lane_rect(
    c: &mut Canvas,
    (x, y, a): (f32, f32, f32),
    len: f32,
    thick: f32,
    fill: [f32; 4],
) {
    c.rect(x - len / 2.0, y - thick / 2.0, len, thick, fill);
    if let Some(sh) = c.shapes.last_mut() {
        sh.angle = -a;
    }
}
/// A capsule of radius `r` from `p` to `q`.
pub(super) fn segment(c: &mut Canvas, p: (f32, f32), q: (f32, f32), r: f32, fill: [f32; 4]) {
    let len = (q.0 - p.0).hypot(q.1 - p.1);
    let (x, y) = ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0);
    c.shapes.push(Shape {
        center: [x, y],
        half: [len / 2.0 + r, r],
        fill,
        border: fill,
        params: [r, 0.0, 0.0, 0.0],
        angle: (q.1 - p.1).atan2(q.0 - p.0),
    });
}
/// `base` with `over` laid on top at opacity `a`.
pub(super) fn tint(base: [f32; 4], over: u32, a: f32) -> [f32; 4] {
    let o = rgba(over, 1.0);
    [
        base[0] + (o[0] - base[0]) * a,
        base[1] + (o[1] - base[1]) * a,
        base[2] + (o[2] - base[2]) * a,
        base[3],
    ]
}
/// 0 at 0 and 1 at 1, overshooting by about 10% on the way and settling back.
pub(super) fn overshoot(x: f32) -> f32 {
    const C: f32 = 1.70158;
    let y = x - 1.0;
    1.0 + (C + 1.0) * y * y * y + C * y * y
}
/// How long a fake slider has been standing still at `vt`: it scrolls on after its cap reaches
/// the drum, for FAKE_FREEZE_LENGTH more of slider length, then stops (see `draw_playfield`).
pub(super) fn fake_freeze(n: &beatmap::Note, vt: f64) -> f64 {
    let moving = n.fake_len / n.fake_length.max(1e-9) * FAKE_FREEZE_LENGTH;
    (vt - (n.time - n.fake_len) - moving).max(0.0)
}
/// A pop-up fake slider at map time `vt`: (lane distance of its head, half its body, alpha), or
/// None for any other note. A long fake slider on a gimmick red line (a 1 ms beat) is too fast to
/// ever be on screen; stable shows it standing still, farther out the longer it is, for
/// |length| / (100 × SliderMultiplier × SV) of the map's main beats before its time (the mapping
/// doc: "-100 at x1.00 disappears in exactly 1 beat"), fading in from then over POPUP_FADE_MS and
/// gone at its time, no fade out (owner, checked in stable; senpai, notice me! [w], 2026-10-28).
/// `sm` is the map's SliderMultiplier, `sv` the note's.
// ponytail: "too fast" is crossing the lane within a 60 Hz frame; refine if a map shows a middle
// case.
pub(super) fn popup(
    n: &beatmap::Note,
    vt: f64,
    lane_len: f32,
    speed: f64,
    main_beat: f64,
    sm: f64,
    sv: f64,
) -> Option<(f32, f32, f32)> {
    let fast = (lane_len as f64) < 16.0 * n.velocity * speed;
    let until = n.time - vt;
    (n.kind == Kind::Roll && n.fake_len > 0.0 && !n.fake_fades && fast && until > 0.0).then(|| {
        let per = POPUP_PER_LENGTH * lane_len;
        let len = n.fake_length as f32;
        let lead = n.fake_length / (100.0 * sm * sv) * main_beat;
        (
            (len - POPUP_FROM_LENGTH).max(0.0) * per,
            len * per,
            fade_from(lead, POPUP_FADE_MS, until),
        )
    })
}
/// Where a pop-up head stands, fitted to the owner's stable screenshot (1080p): heads 95 px apart
/// per 30 of length, the -300 one 270 px past the target, on a 1560 px lane. So lane fractions per
/// unit of length, counted from length 215: -510, the farthest, at 60%.
// ponytail: a fit to one screenshot; re-measure if another map disagrees.
pub(super) const POPUP_PER_LENGTH: f32 = 95.0 / 30.0 / 1560.0;
pub(super) const POPUP_FROM_LENGTH: f32 = 215.0;
/// A pop-up head's fade in: lazer's swell 666 ms (owner: a half beat was too fast, 2026-10-28).
// ponytail: guessed from "too fast"; measure stable's if it still looks off.
pub(super) const POPUP_FADE_MS: f64 = 666.0;
/// A kiai flash's colour: the object's own, each channel up by max(25, 300 − R − G − B) (stable).
pub(super) fn kiai_brighten(col: u32) -> u32 {
    let ch = |k: u32| (col >> k) & 0xff;
    let add = 300u32.saturating_sub(ch(16) + ch(8) + ch(0)).max(25);
    (ch(16) + add).min(255) << 16 | (ch(8) + add).min(255) << 8 | (ch(0) + add).min(255)
}
/// Stable's taiko PreEmpt (ms): how long 1000 osu!px take at the map's *first* timing point
/// (`first_velocity`, osu!px per ms), once for the whole map, at most 5 s. Read from osu!.exe
/// (`min(5000, 600 / VelocityAt(first timing point) / 0.6 * 1000)`; KatsuDonEditor's
/// `stable_preempt`, 2026-10-30).
pub(super) fn stable_preempt(first_velocity: f64) -> f64 {
    if first_velocity <= 0.0 {
        return 5000.0;
    }
    (1000.0 / first_velocity).min(5000.0).floor()
}
/// A swell's alpha `until` ms before it starts: stable's SpinnerTaiko fades it in linearly from
/// PreEmpt before to 0.6 PreEmpt before. The PreEmpt is the map's, from its first SV, not the
/// swell's: a map starting at 10x shows its swells only just before they're hit, however slow
/// they scroll, and one starting slow shows a fast swell as soon as it's on the lane (owner,
/// 2026-10-30). The swell still scrolls at its own SV.
pub(super) fn swell_fade(preempt: f64, until: f64) -> f32 {
    let end = (preempt * 0.6).floor();
    if preempt <= end {
        return 1.0;
    }
    ((preempt - until) / (preempt - end)).clamp(0.0, 1.0) as f32
}
/// Hidden until `lead` ms before, then fading in (easing in) over `fade` ms.
pub(super) fn fade_from(lead: f64, fade: f64, until: f64) -> f32 {
    let p = ((lead - until) / fade).clamp(0.0, 1.0) as f32;
    p * p
}
/// Flip Flop's horizontal scale at map time `t`: 1, or -1 on every other 8-bar section, turning
/// over (through 0) in the first 350 ms of each.
pub(super) fn flip_flop(bars: &[beatmap::Barline], t: f64) -> f32 {
    let Some((i, ..)) = bar_at(bars, t, 1.0) else {
        return 1.0;
    };
    let section = i / 8;
    if section == 0 {
        return 1.0;
    }
    let side = if section % 2 == 1 { -1.0 } else { 1.0 };
    let turn = ((t - bars[section * 8].time) / 350.0).clamp(0.0, 1.0);
    (-side * (std::f64::consts::PI * turn).cos()) as f32
}
/// A fully saturated colour of hue `h` (0-1), for Rave.
pub(super) fn hue(h: f64) -> u32 {
    let h = h.rem_euclid(1.0) * 6.0;
    let x = ((1.0 - (h % 2.0 - 1.0).abs()) * 255.0) as u32;
    let (r, g, b) = match h as u32 {
        0 => (255, x, 0),
        1 => (x, 255, 0),
        2 => (0, 255, x),
        3 => (0, x, 255),
        4 => (x, 0, 255),
        _ => (255, 0, x),
    };
    r << 16 | g << 8 | b
}
impl Play {
    /// Lane, barlines, notes, drum, combo and judgements, laid out as if `c` were the window (the
    /// caller maps it into the playfield area). The scroll and visual mods apply here; Multiverse
    /// draws it three times over.
    pub fn draw_playfield(&mut self, c: &mut Canvas, scroll_speed: f64) {
        // A .tko map's dark backdrop goes under everything of the playfield, its pictures too.
        let vt = self.visual_time();
        if let Some(r) = self.game.tko_mut() {
            r.prepare(vt);
            crate::tko::screen::backdrop(c, &r.field_with(vt, crate::tko::calm()));
        }
        self.draw_layers(c, true);
        if self.game.mods().has(Mod::Multiverse) {
            self.draw_multiverse(c, scroll_speed);
        } else {
            let uni = Universe {
                colours: crate::game::note_colours(),
                ..Universe::CLASSIC
            };
            self.draw_field(c, scroll_speed, Focus::FULL, &uni, self.game.mods());
        }
        self.draw_layers(c, false);
    }

    /// A `.tko` map's text and pictures: those under the lane (`behind`) or those over it.
    pub(super) fn draw_layers(&self, c: &mut Canvas, behind: bool) {
        use crate::tko::clips::Show;
        let Some(r) = self.game.tko() else {
            return;
        };
        for (l, at) in r.layers(self.visual_time()) {
            let p = at.pose;
            if l.behind != behind || p.alpha < 0.002 {
                continue;
            }
            // `x` is from the middle of the editor's 16:9 frame: in a play area of another shape
            // it keeps its place on the width. Sizes and `y` go by the height, as the area's does.
            let sideways = c.width / crate::video::FRAME_VW;
            let (x, y) = (c.width / 2.0 + p.x as f32 * sideways, p.y as f32);
            let h = (p.size * at.scale) as f32;
            let angle = (p.turn as f32).to_radians();
            let tint = rgba(l.colour, p.alpha as f32);
            match &l.show {
                Show::Picture { file } => {
                    c.picture(file, x, y, h, angle, tint);
                }
                Show::Text { text } => {
                    let start = c.shapes.len();
                    let w = Canvas::text_width(h, text);
                    c.text(x - w / 2.0, y - Canvas::text_height(h) / 2.0, h, tint, text);
                    if angle != 0.0 {
                        c.rotate_since(start, [x, y], angle, 1.0, [x, y]);
                    }
                }
            }
        }
    }

    /// Multiverse at map time `t`, for each playfield (see `Focus`). The playfield the focus goes
    /// to next is called over the two beats before its bar: it parks, lights up and pings on each
    /// beat. Over the last three quarters of a beat the focus slides across.
    pub(super) fn multiverse_at(&self, t: f64) -> [Focus; FIELD_COUNT] {
        let mut out = [Focus::default(); FIELD_COUNT];
        let sw = &self.view.focus_switches;
        let i = sw.partition_point(|s| s.at <= t);
        let now = i.checked_sub(1).map_or(FIRST_FOCUS, |i| sw[i].field);
        out[now].focus = 1.0;
        out[now].colour = 1.0;
        out[now].grow = 1.0;
        out[now].settle = 1.0;
        if let Some(s) = i
            .checked_sub(1)
            .map(|i| &sw[i])
            .filter(|s| t - s.at < FOCUS_BURST_MS)
        {
            out[s.field].burst = Some(((t - s.at) / FOCUS_BURST_MS) as f32);
        }
        if let Some(&FocusSwitch {
            at,
            field: next,
            beat,
            ..
        }) = sw.get(i)
        {
            let beat = beat.max(100.0);
            let fade = (0.75 * beat).clamp(150.0, 450.0);
            let x = ((t - (at - fade)) / fade).clamp(0.0, 1.0);
            let p = smooth(0.0, 1.0, x) as f32;
            out[now].focus = 1.0 - p;
            out[now].colour = 1.0 - p;
            out[now].grow = 1.0 - p;
            out[now].settle = 1.0 - p;
            out[next].focus = p;
            out[next].grow = overshoot(x as f32);
            let called = at - 2.0 * beat;
            out[next].settle = smooth(called, at - fade, t) as f32;
            // Its notes bloom into colour over the first beat of the call, a beat before the
            // focus lands; the one in focus keeps its colours till then.
            out[next].colour = smooth(called, called + beat, t) as f32;
            out[next].aim = (smooth(called, called + 0.3 * beat, t)
                * (1.0 - smooth(at - 0.5 * fade, at, t))) as f32;
            // Called, it's already a little bigger than the rest.
            out[next].grow = out[next].grow.max(0.3 * out[next].aim);
            if t >= called {
                out[next].call = (1.0 - ((t - called) / beat).fract() as f32).powi(2);
            }
        }
        out
    }

    /// Multiverse: the playfield eight times over, each a window circling about its home, over
    /// each other. The one in focus is parked, big, bright, in colour and steady; the others
    /// small, dim and colourless. Sparks run from it to the one the focus goes to next.
    pub(super) fn draw_multiverse(&mut self, c: &mut Canvas, scroll_speed: f64) {
        use std::f32::consts::{PI, TAU};
        // On the beat as it'll be seen, like the notes (see `draw_field`).
        let t = self.visual_time();
        let w = c.width;
        let inner = FIELD_W * w / FIELD_ZOOM.1;
        let looks = self.multiverse_at(t);
        let cp = beatmap::control_point_at(&self.game.map().control_points, t);
        let pulse = breath(cp.beat_phase(t), cp.beat_length);
        // Out of focus first, then the one it's heading for, then the one in focus on top.
        let mut order: [usize; FIELD_COUNT] = std::array::from_fn(|f| f);
        let rank = |f: usize| 2.0 * looks[f].focus + looks[f].aim;
        order.sort_by(|&a, &b| rank(a).total_cmp(&rank(b)));
        // Where each drum ends up on screen, for the sparks.
        let drum_x = if self.game.mods().has(Mod::Mirror) {
            inner - HIT_X
        } else {
            HIT_X
        };
        let mut drums = [(0.0f32, 0.0f32); FIELD_COUNT];
        let universes = self.game.universes().unwrap_or_else(multiverse::get);
        let base = self.game.mods();
        // The zone in play, summed weighted by focus, and the weights.
        let mut zone = ([0.0f32; 4], 0.0f32);
        for i in order {
            let l = looks[i];
            let (left, mid, lean) = HOMES[i];
            let start = c.shapes.len();
            // Drawn at its size in focus, so it reaches as far ahead in time whatever its zoom,
            // with its universe's mods on top of the play's for just as long.
            c.width = inner;
            self.draw_field(
                c,
                scroll_speed,
                l,
                &universes[i],
                base.plus(universes[i].mods),
            );
            c.width = w;
            let field_zone = self.view.zone.take();
            // Out of focus it circles its home, each its own way round and at its own pace,
            // leaning and rocking. Called or in focus it parks where it fits on screen, holds
            // still and breathes with the beat.
            let loose = 1.0 - l.settle;
            let dir = if i % 2 == 0 { 1.0 } else { -1.0 };
            let turn = dir * TAU * (t / (7000.0 + 1300.0 * i as f64)).rem_euclid(1.0) as f32
                + i as f32 * 1.7;
            let parked = (parked(i).0 * w, parked(i).1);
            let circling = (left * w + ORBIT.0 * turn.cos(), mid + ORBIT.1 * turn.sin());
            let (x, y) = (
                parked.0 + (circling.0 - parked.0) * loose,
                parked.1 + (circling.1 - parked.1) * loose,
            );
            let angle = loose * (lean + 0.08 * (1.5 * turn).sin());
            let size = FIELD_ZOOM.0 + (FIELD_ZOOM.1 - FIELD_ZOOM.0) * l.grow;
            let swell = 0.015 * pulse * l.focus;
            let zoom = size * (1.0 + swell);
            // It breathes about its drum, not the lane's left end, so its notes keep their pace.
            let k = size * swell * drum_x;
            let (x, y) = (x - k * angle.cos(), y - k * angle.sin());
            let lit = l.aim * (1.0 - l.focus);
            c.fade_since(
                start,
                0.45 + 0.55 * l.focus + 0.4 * lit,
                0.75 + 0.25 * l.focus + 0.2 * lit,
            );
            c.rotate_since(start, [0.0, LANE_Y], angle, zoom, [x, y]);
            drums[i] = (
                x + drum_x * zoom * angle.cos(),
                y + drum_x * zoom * angle.sin(),
            );
            // The zone in play: the one in focus, sliding to the next as the focus does.
            if let Some(z) = field_zone.filter(|_| l.focus > 0.0) {
                let z = turned_box(z, [0.0, LANE_Y], angle, zoom, [x, y]);
                for (sum, v) in zone.0.iter_mut().zip(z) {
                    *sum += v * l.focus;
                }
                zone.1 += l.focus;
            }
        }
        self.view.zone = (zone.1 > 0.0).then(|| zone.0.map(|v| v / zone.1));
        // Multiverse: on the playfield in focus, as big as it is.
        self.view.lane_path.clear();
        let focus = self.view.zone.map(|z| {
            let (_, band_h) = lane_band();
            let k = z[3] / band_h;
            let mid = z[1] + z[3] / 2.0;
            let (lane_y, lane_h) = (mid - LANE_H * k / 2.0, LANE_H * k);
            hud::FieldBoxes {
                lane: [z[0], lane_y, z[2], lane_h],
                drum: [z[0], lane_y, PANEL_W * k, lane_h],
                hit: [
                    z[0] + (HIT_X - NOTE_R) * k,
                    mid - NOTE_R * k,
                    2.0 * NOTE_R * k,
                    2.0 * NOTE_R * k,
                ],
            }
        });
        if let Some(boxes) = focus {
            self.ease_field_boxes(boxes, self.game.time());
        }
        // The sparks: from the drum in focus to the drum of the one it's heading for.
        let to = (0..FIELD_COUNT).find(|&f| looks[f].aim > 0.0);
        let from = (0..FIELD_COUNT)
            .filter(|&f| looks[f].aim == 0.0)
            .max_by(|&a, &b| looks[a].focus.total_cmp(&looks[b].focus));
        if let (Some(from), Some(to)) = (from, to) {
            let (a, b) = (drums[from], drums[to]);
            let n = ((b.0 - a.0).hypot(b.1 - a.1) / 18.0).ceil().max(1.0) as usize;
            let run = (t / 350.0).rem_euclid(1.0) as f32;
            for k in 0..n {
                let s = (k as f32 + run) / n as f32;
                let (x, y) = (a.0 + (b.0 - a.0) * s, a.1 + (b.1 - a.1) * s);
                let alpha = looks[to].aim * (PI * s).sin() * (0.4 + 0.6 * s);
                c.glow(x, y, 2.5, 7.0, rgba(FOCUS_COL, 0.6 * alpha));
                c.circle(x, y, 2.0, rgba(WHITE, alpha), rgba(WHITE, 0.0), 0.0);
            }
        }
    }

    /// One playfield (see `draw_playfield`). Multiverse's are windows, framed and lit by `look`,
    /// each in its universe's colours, note shape and speed; out of colour, dons and kats look
    /// alike.
    pub(super) fn draw_field(
        &mut self,
        c: &mut Canvas,
        scroll_speed: f64,
        look: Focus,
        uni: &Universe,
        mods: Mods,
    ) {
        use std::f32::consts::{PI, TAU};
        // Notes, barlines and the beat pulse are drawn at `vt`, where they'll be once the frame is
        // on screen; animations of what already happened (hits, presses) run on `t`.
        let t = self.game.time();
        let vt = self.visual_time();
        let w = c.width;
        let start_shape = c.shapes.len();
        let party = mods.has(Mod::Party);
        // Crossfire: the drum in the middle, kats coming from the left (laid out here the other
        // way along the lane from it), dons from the right.
        let crossfire = mods.has(Mod::Crossfire);
        let side_of = |kind: Kind, s: f32| {
            if crossfire && kind == Kind::Kat {
                -s
            } else {
                s
            }
        };
        // Multiverse: a window, not the whole screen. Nothing may stick out of it.
        let boxed = mods.has(Mod::Multiverse);
        let lane_top = LANE_Y - LANE_H / 2.0;
        let cp = beatmap::control_point_at(&self.game.map().control_points, vt);
        let beat_phase = cp.beat_phase(vt) as f32;
        // Beats since the red line, for the mods that move with the beat.
        let beats = ((vt - cp.beat_origin) / cp.beat_length) as f32;
        let pulse = (1.0 - beat_phase).powi(3);
        let kiai = cp.kiai && self.view.kiai_effects;
        let in_kiai = cp.kiai;
        // Radar: where the beam is (see `radar_sweep`), and how long it takes to cross the lane.
        let radar = mods.has(Mod::Radar);
        let sweep = radar_sweep(beats, in_kiai);
        let sweep_ms = cp.beat_length
            * if in_kiai {
                RADAR_BEATS.1
            } else {
                RADAR_BEATS.0
            } as f64;

        // Tact: far off, the notes crawl through each step, then rush to where they'd be at its
        // end. Like Pulse's and Stutter's, the shift fades out on the way in (`Scroll::x`), so
        // every note is in the hit circle on time.
        let tact = if mods.has(Mod::Tact) {
            self.tact_time(vt) - vt
        } else {
            0.0
        };
        let warp = beat_warp(mods, cp, vt) + self.extra_warp(vt, cp, mods) + tact;
        // A .tko map's playfield motion: what is drawn, never when a note is hit.
        let tko_fx = self
            .tko_mut()
            .map(|r| {
                r.prepare(vt);
                r.field_with(vt, crate::tko::calm())
            })
            .unwrap_or_default();
        // The lane's shape, and how fast notes go along it (see `lane_stretch`).
        let lane = (tko_fx.lane_shape > 0 && tko_fx.lane_bend > 0.001)
            .then(|| (tko_fx.lane_shape as usize - 1, tko_fx.lane_bend));
        let (rail, lane_len, rail_drop, rail_zoom) = self.rail_at(vt, w, lane, mods);
        // A .tko map's moving drum: the hit target slides along a straight lane and everything
        // on the lane goes with it, so notes still meet it on time.
        let drum = if rail.is_none() {
            tko_fx.drum.clamp(0.0, (lane_len - 2.0 * BIG_R).max(0.0))
        } else {
            0.0
        };
        // Where it's played before the whole-playfield moves (x0, y0, x1, y1): from the screen's
        // left edge (the drum) along the lane, with room for the judgements.
        let (band_top, band_h) = lane_band();
        let local_zone = match &rail {
            Some(r) => r.pts.iter().fold(
                [0.0, band_top, HIT_X, band_top + band_h],
                |z, &(x, y, _)| {
                    [
                        z[0],
                        z[1].min(y - LANE_H / 2.0),
                        z[2].max(x + LANE_H / 2.0),
                        z[3].max(y + LANE_H / 2.0),
                    ]
                },
            ),
            None => [
                0.0,
                band_top,
                (HIT_X + lane_len).max(HIT_X + BIG_R),
                band_top + band_h,
            ],
        };
        let portal = self.portal_at(vt, lane_len, mods);
        let scroll = Scroll {
            t: vt,
            speed: osu_speed_scale(w)
                * scroll_speed
                * uni.speed()
                * mods.scroll()
                * lane_stretch(rail.is_some(), lane_len, w),
            lane: lane_len as f64,
            mods,
            warp,
            live: self.miss_factor(t, mods) * self.slow_factor(t) * tko_fx.scroll,
            shared: self
                .game
                .tko()
                .and_then(|rt| rt.shared_scroll())
                .map(|(table, shift)| {
                    let now = table.x(vt - shift);
                    (table, shift, now)
                }),
        };
        // Distance along the lane of something at map time `time` scrolling at `vel`.
        let s_of = |time: f64, vel: f64| scroll.x(time, vel) - HIT_X;
        // Uncapped: extremely slow notes are common, so the whole map may be on screen at once,
        // its last object included (owner, 2026-10-28).
        // Keep the global bound for special objects and barlines; ordinary-note blocks also
        // have conservative local bounds so one distant slow note does not widen every scan.
        let lookahead = if scroll.shared.is_some() {
            // Frozen or running backwards, notes far off in time may be near: look well ahead
            // and let the lane's ends hide the rest.
            SHARED_LOOKAHEAD_MS
        } else {
            (lane_len + BIG_R) as f64
                / (self.view.min_velocity * scroll.speed * scroll.min_factor())
                + warp.abs()
        };

        // Notes on screen, nearest first.
        let start = self
            .map()
            .notes
            .partition_point(|n| n.time < vt.min(t) - lookahead.max(20_000.0));
        self.view.visible.clear();
        let first_block = self
            .view
            .note_blocks
            .partition_point(|b| b.range.end <= start);
        for block in &self.view.note_blocks[first_block..] {
            if self.game.map().notes[block.range.start].time - vt > lookahead {
                break;
            }
            if block.regular && !mods.has(Mod::Liar) && scroll.shared.is_none() {
                let reach = (lane_len + BIG_R) as f64
                    / (block.min_velocity * scroll.speed * scroll.min_factor())
                    + warp.abs();
                if self.game.map().notes[block.range.start].time - vt.max(t) > reach
                    || block.last_end < vt.min(t) - 3000.0
                {
                    continue;
                }
            }
            for i in start.max(block.range.start)..block.range.end {
                let n = &self.game.map().notes[i];
                if n.time - vt > lookahead {
                    break;
                }
                if n.end_time + n.fake_len < vt.min(t) - 3000.0 && n.kind != Kind::Swell {
                    continue;
                }
                self.view.visible.push(i);
            }
        }
        // Distance along the lane of note `i`. Liar: far off, a triplet sits where a 1/4 would,
        // sliding to its real place on the way in.
        let note_s = |i: usize| -> f32 {
            let n = &self.game.map().notes[i];
            let s = s_of(n.time, n.velocity);
            match self.view.lies.get(i) {
                Some(&lie) if lie != n.time => {
                    let fake = s_of(lie, n.velocity);
                    s + (fake - s) * smooth(0.3, 0.55, (s / lane_len) as f64) as f32
                }
                _ => s,
            }
        };
        // Shatter: pieces of the lane with streams on them slide further.
        let mut shatter = self.shatter_at(vt, mods);
        if mods.has(Mod::Shatter) {
            let mut on = [[0u32; 2]; 4];
            for &i in &self.view.visible {
                let s = note_s(i);
                if matches!(self.game.map().notes[i].kind, Kind::Don | Kind::Kat)
                    && s > 0.0
                    && s < lane_len
                {
                    let piece = &mut on[((s / lane_len * 4.0) as usize).min(3)];
                    piece[0] += 1;
                    piece[1] += (self.game.flows()[i] >= 3) as u32;
                }
            }
            for (d, [all, streams]) in shatter.iter_mut().zip(on) {
                *d *= 0.6 + 0.8 * streams as f32 / all.max(1) as f32;
            }
        }
        let geo = LaneGeo {
            rail,
            len: lane_len,
            shatter,
            cracks: self.game.cracks_at(t, lane_len),
        };
        let bent = geo.bent() || portal.is_some();
        // The stretches of a bent lane a solid roll body lies on already.
        let mut body_laid = vec![false; if bent { body_cells(lane_len) } else { 0 }];
        // Where on screen something `s` along the lane is drawn, `side` off its middle: through
        // the portal, onto the (bent, broken) lane. Also its heading and size.
        let place = |s: f32, side: f32| -> (f32, f32, f32, f32) {
            let (s, size) = portal.map_or((s, 1.0), |p| through_portal(s, p));
            let (x, y, a) = geo.at(s, side);
            (x, y, a, size)
        };
        // A window's notes fade in over the end of its lane rather than poke out past it.
        let fade_in = |s: f32| -> f32 {
            if boxed {
                smooth(lane_len as f64, (lane_len - 60.0) as f64, s as f64) as f32
            } else {
                1.0
            }
        };
        // How visible an approaching note `s` along the lane is under the visual mods.
        let memory_over = mods.has(Mod::Memory) && vt > self.game.first_note() + 15_000.0;
        let note_alpha = |s: f32| -> f32 {
            let u = s / lane_len;
            let mut a = fade_in(s);
            if mods.has(Mod::Hidden) {
                // osu!lazer: fades out over the first 37.5% of its way in.
                a *= ((u - 0.625) / 0.375).clamp(0.0, 1.0);
            }
            if mods.has(Mod::Sudden) {
                a *= ((0.45 - u) / 0.12).clamp(0.0, 1.0);
            }
            if mods.has(Mod::Blink) && u > 0.15 && (0.5..0.95).contains(&beat_phase) {
                a = 0.0;
            }
            if memory_over {
                a = 0.0;
            }
            a
        };
        // Radar: how far a note at map time `time` scrolling at `vel` comes in a sweep, and how
        // lit it is `s` along the lane.
        let radar_u = |time: f64, vel: f64| s_of(time + sweep_ms, vel) - s_of(time, vel);
        let radar_lit = |s: f32, time: f64, vel: f64| {
            if radar {
                radar_light(s, radar_u(time, vel), lane_len, sweep)
            } else {
                1.0
            }
        };
        // A don or kat `s` along the lane (`k`th drawn, in a 1/`flow` stream): Snake swings it
        // sideways, Party pumps and squishes it. Nothing changes at the drum.
        let colorless = mods.has(Mod::Colorless) || mods.has(Mod::Echo);
        // The kiai flash keeps stable's colours, except where notes look alike: a coloured flash
        // would tell them apart on every beat.
        let (flash_don, flash_kat) = if colorless { (GREY, GREY) } else { (DON, KAT) };
        let (don, kat) = if colorless {
            (GREY, GREY)
        } else {
            (
                mix(uni.idle(), uni.don(), look.colour),
                mix(uni.idle(), uni.kat(), look.colour),
            )
        };
        let true_notes = playfield_look().true_notes;
        let draw_hit = |c: &mut Canvas,
                        s: f32,
                        n: &beatmap::Note,
                        k: usize,
                        flow: u8,
                        col: u32,
                        a: f32,
                        fx: NoteFx| {
            let mut side = 0.0;
            if mods.has(Mod::Snake) {
                // Streams swing the most, triplets the other way, notes on the beat not at all.
                let (amp, flip) = match flow {
                    1 => (0.0, 0.0),
                    2 => (0.45, 0.0),
                    0 => (0.6, 0.0),
                    d if d % 3 == 0 => (1.0, PI),
                    _ => (1.0, 0.0),
                };
                // The wave jolts forward on each beat.
                let u = (s / lane_len).max(0.0);
                let f = smooth(0.0, 0.3, u as f64) as f32;
                let jolt = beats.floor() + 1.0 - (1.0 - beat_phase).powi(3);
                side +=
                    32.0 * amp * f * (0.8 + 0.4 * pulse) * (TAU * 1.3 * u - PI * jolt + flip).sin();
            }
            let (x, y, _, mut size) = place(side_of(n.kind, s), side);
            let (hx, hy, _, _) = place(0.0, 0.0);
            let (x, y) = tko_moved(&fx, x, y, (hx, hy));
            let a = a * fx.opacity;
            size *= fx.scale;
            // A .tko lie: drawn as another colour or size until its truth shows (unless the
            // player asked for the truth).
            let lie = !true_notes && fx.lies();
            let truth = if lie { fx.truth } else { 1.0 };
            let col = match fx.lie_kat.filter(|_| lie) {
                Some(k) => mix(if k { kat } else { don }, col, truth),
                None => col,
            };
            let real_r = if n.big { BIG_R } else { NOTE_R };
            let (big, base_r) = match fx.lie_big.filter(|_| lie) {
                Some(b) => {
                    let shown = if b { BIG_R } else { NOTE_R };
                    (
                        if truth < 0.5 { b } else { n.big },
                        shown + (real_r - shown) * truth,
                    )
                }
                None => (n.big, real_r),
            };
            // A .tko tint: the note's colour towards a hue.
            let col = if fx.tint >= 0.01 {
                let [r, g, b] = crate::tko::screen::hue_rgb(fx.tint_hue);
                let hue =
                    ((r * 255.0) as u32) << 16 | ((g * 255.0) as u32) << 8 | (b * 255.0) as u32;
                mix(col, hue, fx.tint.min(1.0))
            } else {
                col
            };
            let mut squish = 1.0;
            if party {
                // Each note pulses on its own grid: a 1/4 four times a beat.
                let own = (1.0 - (beats * flow.clamp(1, 8) as f32).rem_euclid(1.0)).powi(3);
                size *= 1.0 + 0.22 * own;
                squish = 1.0 + 0.3 * own * if k.is_multiple_of(2) { 1.0 } else { -1.0 };
            }
            let r = base_r * size;
            let first = c.shapes.len();
            if uni.shape != NoteShape::Round || !skin_note(c, x, y, r, big, col, a) {
                shaped_note(c, uni.shape, x, y, r, col, a);
            }
            let (sx, sy) = (squish * fx.stretch_x, fx.stretch_y / squish);
            if sx != 1.0 || sy != 1.0 {
                c.stretch_since(first, [x, y], sx, sy);
            }
        };
        // Text is drawn last, after Mirror flips the rest (letters must stay readable), and so
        // are the skin's numbers and judgements (centre x, top, height).
        let mut texts: Vec<(f32, f32, f32, [f32; 4], String)> = Vec::new();
        let mut upright: Vec<(f32, f32, f32, [f32; 4], Upright)> = Vec::new();

        // Multiverse: a halo round the window, for the focus (breathing with the beat) and for
        // the call (glowing, flaring on each beat).
        let halo = (look.focus * (0.6 + 0.4 * pulse)).max(look.aim * (0.75 + 0.25 * look.call));
        if boxed && halo > 0.01 {
            for (k, a) in [0.3, 0.16, 0.08, 0.03].into_iter().enumerate() {
                let d = 3.0 + 6.0 * k as f32;
                c.round_rect(
                    [-d, lane_top - 3.0 - d],
                    [w + 2.0 * d, LANE_H + 6.0 + 2.0 * d],
                    4.0 + d,
                    rgba(0, 0.0),
                    rgba(FOCUS_COL, a * halo),
                    6.0,
                );
            }
        }

        // Lane. When it turns, it reaches past the screen so its ends never show.
        let turns = mods.has(Mod::Tilt)
            || mods.has(Mod::Spin)
            || mods.has(Mod::Mania)
            || mods.has(Mod::BarrelRoll)
            || crossfire;
        let mut lane_col = if kiai {
            rgba(0x2b2433, 1.0)
        } else {
            rgba(0x20202b, 1.0)
        };
        let mut edge_col = rgba(0x3a3a4e, 1.0);
        let mut skin_lane_tint = rgba(WHITE, 1.0);
        if radar {
            lane_col = if in_kiai {
                rgba(0x17150d, 1.0)
            } else {
                rgba(0x0b110f, 1.0)
            };
        }
        if kiai {
            // Radar's lane stays dark, a little warmer.
            lane_col = tint(
                lane_col,
                0xff8a3d,
                (0.05 + 0.06 * pulse) * if radar { 0.35 } else { 1.0 },
            );
        }
        if party {
            // The lane flashes a new colour on every beat.
            let col = hue(hash01(beats.floor() as f64 * 2.7 + 0.3));
            lane_col = tint(
                lane_col,
                col,
                (0.07 + 0.16 * pulse) * if in_kiai { 1.6 } else { 1.0 },
            );
            edge_col = rgba(col, 0.9);
            skin_lane_tint = tint(skin_lane_tint, col, 0.15 + 0.25 * pulse);
        }
        let black = black_lane();
        if black {
            lane_col = rgba(0x000000, 1.0);
        }
        if geo.bent() {
            // In short pieces along the way: edges first, so the lane's inside covers their joins.
            const PIECE: f32 = 8.0;
            let from = if turns { -w } else { -HIT_X };
            let to = lane_len
                + if boxed {
                    0.0
                } else if geo.rail.is_some() {
                    60.0
                } else {
                    HIT_X
                };
            let n = ((to - from) / PIECE).ceil() as usize;
            for (thick, col) in [(LANE_H + 6.0, edge_col), (LANE_H, lane_col)] {
                for k in 0..n {
                    let s = from + (k as f32 + 0.5) * PIECE;
                    // Longer by how much the outside of a bend is longer than the middle, by the
                    // sharper turn to either neighbour: where a corner starts or ends (Zigzag's
                    // are tight) the turn within the piece is less than to the next one, and the
                    // outer edge tore there.
                    let a = |s: f32| geo.at(s, 0.0).2;
                    let turn = (a(s + PIECE) - a(s)).abs().max((a(s) - a(s - PIECE)).abs());
                    lane_rect(
                        c,
                        geo.at(s, 0.0),
                        PIECE + 1.0 + turn * thick / 2.0,
                        thick,
                        col,
                    );
                }
            }
            // Cracks: dark seams where the knocked-out pieces start and end.
            for &(a, l, _, seam) in &geo.cracks {
                for s in [a, a + l] {
                    lane_rect(c, geo.at(s, 0.0), 3.0, LANE_H + 6.0, rgba(0x08080b, seam));
                }
            }
        } else {
            let (lane_x, lane_w) = if turns { (-w, 3.0 * w) } else { (0.0, w) };
            let glow = if kiai { 0.75 + 0.25 * pulse } else { 0.0 };
            // Radar's lane is its own dark one.
            if black || radar || !skin_lane(c, lane_x, lane_w, skin_lane_tint, glow) {
                draw_lane(c, lane_x, lane_w, lane_col, edge_col);
            }
        }
        // Crumble: crumbs fly off the edge each piece moves out to and fall away.
        for cr in self.game.crumbs() {
            let age = ((t - cr.at()) / 1000.0) as f32;
            if !(0.0..0.7).contains(&age) {
                continue;
            }
            for k in 0..1 + (4.0 * cr.size()).round() as usize {
                let h = |j: f64| hash01(cr.at() + k as f64 * 5.3 + j) as f32;
                let s = cr.start() * lane_len + cr.length() * h(0.1);
                let (x, y, a) = geo.at(s, cr.shift().signum() * (LANE_H / 2.0 + 3.0));
                let (vx, vy) = (
                    (h(0.2) * 2.0 - 1.0) * 70.0,
                    cr.shift().signum() * (30.0 + 60.0 * h(0.3)),
                );
                let p = (
                    x + vx * age,
                    y + vy * age + 700.0 * age * age,
                    a + (h(0.4) * 2.0 - 1.0) * 8.0 * age,
                );
                let side = 3.0 + 6.0 * cr.size() * h(0.5);
                let col = tint(edge_col, WHITE, 0.3);
                lane_rect(
                    c,
                    p,
                    side,
                    side,
                    [col[0], col[1], col[2], col[3] * (1.0 - age / 0.7)],
                );
            }
        }
        // Portal: the entry (orange) and the exit (blue), across the lane.
        if let Some((entry, exit)) = portal {
            for (s, col) in [(entry, 0xff8a3d), (exit, 0x4fb4ff)] {
                let p = geo.at(s, 0.0);
                c.glow(p.0, p.1, 22.0, 20.0, rgba(col, 0.3 + 0.2 * pulse));
                lane_rect(c, p, 7.0, LANE_H * 0.9, rgba(col, 0.9));
            }
        }

        let lane_shapes = c.shapes.len();
        // Barlines.
        // As far back as ahead: a slow bar can still be on screen long after it passed.
        let first_bar = self
            .map()
            .barlines
            .partition_point(|b| b.time < vt - lookahead);
        let bars = &self.game.map().barlines[first_bar..];
        for b in bars {
            if b.time - vt > lookahead {
                break;
            }
            let s = s_of(b.time, b.velocity);
            let shown = if crossfire {
                s > 0.0
            } else {
                s > PANEL_W - HIT_X
            };
            if shown && s < lane_len + 2.0 {
                for s in if crossfire { vec![s, -s] } else { vec![s] } {
                    let (x, y, a, _) = place(s, 0.0);
                    if let Some([bw, bh]) = c.sprite_size(Sprite::Barline) {
                        c.sprite_box(
                            Sprite::Barline,
                            x,
                            y,
                            (bw * LANE_PX / 2.0).max(0.6),
                            bh * LANE_PX / 2.0,
                            rgba(WHITE, 1.0),
                        );
                        if let Some(sh) = c.shapes.last_mut() {
                            sh.angle = -a;
                        }
                    } else {
                        lane_rect(c, (x, y, a), 1.6, LANE_H, rgba(WHITE, 0.28));
                    }
                }
            }
        }
        // Roulette: where the speed changes, and to what.
        let first_section = self.view.sections.partition_point(|s| s.0 < vt - 3000.0);
        for &(time, k, vel) in &self.view.sections[first_section..] {
            if time - vt > lookahead {
                break;
            }
            let s = s_of(time, vel);
            if s > PANEL_W - HIT_X && s < lane_len + 30.0 {
                let col = if k > 1.0 {
                    0xff5a3d
                } else if k < 1.0 {
                    0x4fb4ff
                } else {
                    WHITE
                };
                let (x, y, a, _) = place(s, 0.0);
                lane_rect(c, (x, y, a), 4.0, LANE_H, rgba(col, 0.8));
                let (lx, ly, ..) = place(s + 18.0, -LANE_H / 2.0 + 10.0);
                texts.push((lx, ly - 4.0, 8.0, rgba(col, 1.0), format!("x{k}")));
            }
        }

        // Radar: the beam, a bright line with a trail fading out behind it.
        if radar {
            let b = sweep * lane_len;
            for d in if crossfire {
                vec![1.0, -1.0]
            } else {
                vec![1.0]
            } {
                for k in 0..14 {
                    let back = b - 6.0 * k as f32 - 3.0;
                    if back > 0.0 {
                        let (x, y, a, _) = place(d * back, 0.0);
                        lane_rect(
                            c,
                            (x, y, a),
                            6.5,
                            LANE_H,
                            rgba(RADAR_COL, 0.16 * (1.0 - k as f32 / 14.0).powi(2)),
                        );
                    }
                }
                let (x, y, a, _) = place(d * b, 0.0);
                lane_rect(c, (x, y, a), 9.0, LANE_H, rgba(RADAR_COL, 0.2));
                lane_rect(c, (x, y, a), 2.5, LANE_H, rgba(0xc8ffe0, 0.95));
            }
        }

        // Hit target.
        draw_hit_target(c);
        if kiai {
            c.glow(HIT_X, LANE_Y, NOTE_R, 30.0, rgba(0xffb347, 0.25 * pulse));
        }
        if let Some((j, at)) = self
            .last_judge()
            .filter(|_| !self.view.hidden.has(Part::Judgements))
        {
            let a = (1.0 - (t - at) / 220.0).clamp(0.0, 1.0) as f32;
            if a > 0.0 && j != Judge::Miss && splashes(j) && playfield_look().hit_flash {
                c.glow(HIT_X, LANE_Y, NOTE_R + 2.0, 26.0, rgba(j.color(), 0.55 * a));
            }
        }

        // Decoys, under the real notes: gone before they'd have to be hit.
        let first = self.view.decoys.partition_point(|n| n.time < vt);
        for (k, n) in self.view.decoys.iter().enumerate().skip(first) {
            if n.time - vt > lookahead {
                break;
            }
            let s = s_of(n.time, n.velocity);
            // A little see-through, so a careful eye can tell them apart.
            let a = DECOY_ALPHA
                * note_alpha(s)
                * radar_lit(s, n.time, n.velocity)
                * smooth(0.22, 0.38, (s / lane_len) as f64) as f32;
            if a > 0.01 && s < lane_len + BIG_R {
                draw_hit(
                    c,
                    s,
                    n,
                    k,
                    self.view.decoy_flows[k],
                    if n.kind == Kind::Don { don } else { kat },
                    a,
                    NoteFx::default(),
                );
            }
        }

        // Defense: the towers beside the maze, glitched pixel blocks in their builder's colour:
        // red and cyan copies slip out from under them, the core flickers, and a shot tears
        // them apart for a moment. Dim once their notes are gone by.
        if let Some(d) = self.game.defense() {
            let bucket = (vt / 40.0).floor() as u32;
            for (k, tw) in d.towers.iter().enumerate() {
                let (x, y, _, size) = place(tw.s(), tw.side());
                // Built: it glitches in over a few steps; done: it crumbles out over a second.
                let since = (vt - (tw.from() - TOWER_LEAD_MS)) as f32;
                let left = (1.0 - ((vt - tw.to()) / 1000.0) as f32).clamp(0.0, 1.0);
                if since < 0.0 || left <= 0.0 {
                    continue;
                }
                let col = team_colour(tw.who());
                let shoots = if tw.kat() { KAT } else { DON };
                let busy = vt < tw.to();
                let a = (since / 200.0).min(1.0) * if busy { 1.0 } else { 0.5 * left };
                // How hard it's firing: one of its notes just went past it.
                let fire = tw
                    .notes()
                    .iter()
                    .map(|&i| note_s(i))
                    .filter(|&s| s <= tw.s() && s > tw.s() - 45.0)
                    .map(|s| 1.0 - (tw.s() - s) / 45.0)
                    .fold(0.0, f32::max);
                let h = |j: u32| glitch_hash(k as u32, bucket, j);
                let jit = if h(1) < 0.2 + 0.7 * fire {
                    (h(2) - 0.5) * (3.0 + 8.0 * fire) * size
                } else {
                    0.0
                };
                let (r, slip) = (
                    20.0 * size,
                    (2.0 + 7.0 * fire + if h(3) < 0.1 { 3.0 } else { 0.0 }) * size,
                );
                let (bx, by, bw) = (x - r + jit, y - r, 2.0 * r);
                c.rect(bx - slip, by, bw, bw, rgba(SPLIT_RED, 0.5 * a));
                c.rect(bx + slip, by, bw, bw, rgba(SPLIT_CYAN, 0.5 * a));
                c.rect(bx, by, bw, bw, rgba(0x0b0b10, 0.95 * a));
                for (ex, ey, ew, eh) in [
                    (bx, by, bw, 1.5),
                    (bx, by + bw - 1.5, bw, 1.5),
                    (bx, by, 1.5, bw),
                    (bx + bw - 1.5, by, 1.5, bw),
                ] {
                    c.rect(ex, ey, ew, eh, rgba(col, a));
                }
                // A 2x2 core of pixels, some of them out each step (all on while it fires).
                let px = 8.0 * size;
                for j in 0..4u32 {
                    if h(10 + j) > 0.25 - 0.25 * fire {
                        let (ox, oy) = ((j % 2) as f32, (j / 2) as f32);
                        c.rect(
                            x - px + ox * px + jit,
                            y - px + oy * px,
                            px - 0.6,
                            px - 0.6,
                            rgba(shoots, a),
                        );
                    }
                }
                // Firing: torn slices sticking out of it.
                if fire > 0.0 {
                    for j in 0..3u32 {
                        let ty = by + h(20 + j) * bw;
                        let reach = (6.0 + 14.0 * h(24 + j)) * fire * size;
                        c.rect(
                            bx - reach,
                            ty,
                            bw + 2.0 * reach,
                            1.5 * size,
                            rgba(if j % 2 == 0 { TEAR_RED } else { TEAR_CYAN }, fire),
                        );
                    }
                }
            }
            // The shield: a broken cyan ring round the drum, a segment out here and there.
            if let Some(st) = d.state.filter(|st| st.shield > 0) {
                let (x, y, _, size) = place(0.0, 0.0);
                let ring = (BIG_R + 9.0) * size;
                let bright = (0.45 + 0.15 * st.shield as f32).min(1.0);
                for j in 0..16u32 {
                    if glitch_hash(j, bucket, 40) < 0.12 {
                        continue;
                    }
                    let ang = j as f32 * std::f32::consts::TAU / 16.0;
                    let off = if glitch_hash(j, bucket, 41) < 0.1 {
                        3.0 * size
                    } else {
                        0.0
                    };
                    let (sx, sy) = (x + ang.cos() * (ring + off), y + ang.sin() * (ring + off));
                    c.rect(
                        sx - 2.5 * size,
                        sy - 2.5 * size,
                        5.0 * size,
                        5.0 * size,
                        rgba(SPLIT_CYAN, bright),
                    );
                }
            }
        }

        // Notes, back to front so earlier notes sit on top. A window's leave it under the drum.
        // Hit ones fly out of the drum over its panel, so they're drawn after it.
        let mut flying: Vec<(f32, bool, u32)> = Vec::new();
        let behind = |r: f32| if boxed { r - HIT_X } else { -HIT_X - r };
        let on_lane = |s: f32, r: f32| s > behind(r) && s < lane_len + r;
        // On one ms, a fake slider sits over the don/kat it's stacked on (and, with the map's
        // own spinner-warning, a swell head too).
        let map_spinner = crate::skin::map_has_spinner_warning();
        let over =
            |n: &beatmap::Note| n.kind == Kind::Roll || (map_spinner && n.kind == Kind::Swell);
        let main_beat = self.game.main_beat();
        let sm = self.game.map().meta.slider_multiplier;
        let popup = |n: &beatmap::Note| {
            popup(
                n,
                vt,
                lane_len,
                scroll.speed,
                main_beat,
                sm,
                beatmap::control_point_at(&self.game.map().control_points, n.time).sv,
            )
        };
        // .tko approach paths the map shows the player: faint dots from where notes come in to
        // the drum.
        if let Some(rt) = self.game.tko() {
            let (hx, hy, _, _) = place(0.0, 0.0);
            for (table, w) in rt.shown_paths(vt) {
                let mut d = 0.0;
                while d < table.len() {
                    let [px, py] = table.at(d);
                    c.circle(hx + px, hy + py, 1.6, rgba(WHITE, 0.22 * w), [0.0; 4], 0.0);
                    d += 9.0;
                }
            }
        }
        // Notes flying in on a .tko approach path, drawn over the drum panel after it: a path
        // from behind the drum starts under the panel, where they'd be hidden most of the way.
        // (lane distance, fake, index, flow, colour, alpha, motion)
        let mut on_path: Vec<(f32, bool, usize, u8, u32, f32, NoteFx)> = Vec::new();
        let over_panel = |fx: &NoteFx| !crossfire && fx.path.is_some_and(|p| p[2] > 0.0);
        // The shapes of notes a .tko map draws over every effect: lifted over them at the end.
        let mut above_shapes: Vec<std::ops::Range<usize>> = Vec::new();
        // A .tko map's fake notes, under the real ones: drawn like them, never judged, gone when
        // they reach the drum.
        let fakes_on: Vec<usize> = {
            let f = &self.game.map().fakes;
            let from = f.partition_point(|n| n.time <= vt);
            let to = f.partition_point(|n| n.time <= vt + lookahead);
            (from..to).collect()
        };
        for &k in fakes_on.iter().rev() {
            let n = &self.game.map().fakes[k];
            let r = if n.big { BIG_R } else { NOTE_R };
            let s = s_of(n.time, n.velocity);
            if !on_lane(s, r) {
                continue;
            }
            let a = note_alpha(s) * radar_lit(s, n.time, n.velocity);
            let mut nfx = self.tko().and_then(|rt| rt.fake(k, vt)).unwrap_or_default();
            nfx.path = self.tko().and_then(|rt| rt.fake_path(k, vt, s, lane_len));
            if a > 0.01 && !nfx.hidden {
                let col = if n.kind == Kind::Kat { kat } else { don };
                if over_panel(&nfx) {
                    on_path.push((s, true, k, 1, col, a, nfx));
                } else {
                    let from = c.shapes.len();
                    draw_hit(c, s, n, k, 1, col, a, nfx);
                    if nfx.above {
                        above_shapes.push(from..c.shapes.len());
                    }
                }
            }
        }
        let mut order = self.view.visible.clone();
        order.sort_by(|&a, &b| {
            let (na, nb) = (&self.game.map().notes[a], &self.game.map().notes[b]);
            nb.time
                .total_cmp(&na.time)
                .then(over(na).cmp(&over(nb)))
                .then(b.cmp(&a))
        });
        for i in order {
            let n = &self.game.map().notes[i];
            let st = &self.game.state()[i];
            let r = if n.big { BIG_R } else { NOTE_R };
            match n.kind {
                Kind::Don | Kind::Kat => {
                    let col = if n.kind == Kind::Don { don } else { kat };
                    match st.judged() {
                        // Hit: the note flies up out of the drum (unless the interface toggle hides it).
                        Some(Judge::Great | Judge::Ok)
                            if self.view.hidden.has(Part::HitAnimation) => {}
                        Some(Judge::Great)
                            if self.game.defense().is_some_and(|d| d.shot[i].is_some()) => {}
                        Some(Judge::Great | Judge::Ok) => {
                            let p = ((t - st.judged_at()) / HIT_ANIM_MS).clamp(0.0, 1.0) as f32;
                            if p < 1.0 {
                                flying.push((p, n.big, col));
                            }
                        }
                        judged => {
                            let s = note_s(i);
                            // Defense: a teammate's note, faint with a ring in their colour, gone
                            // once it's past the drum.
                            let theirs = self
                                .defense()
                                .filter(|d| d.owner[i] as usize != d.me)
                                .map(|d| team_colour(d.owner[i] as usize));
                            if theirs.is_some() && s < 0.0 {
                                continue;
                            }
                            if let Some(d) = self.game.defense() {
                                // Taken by a tower: gone past it, with a shot and a burst just after.
                                let bucket = (vt / 40.0).floor() as u32;
                                if let Some(tw) = d.shot[i].and_then(|k| d.towers.get(k as usize))
                                    && s <= tw.s()
                                {
                                    // Shot: a stepped pixel beam from the tower, and the note
                                    // breaks into pixels flying apart.
                                    let k = (tw.s() - s) / 45.0;
                                    if k < 1.0 {
                                        let (x, y, _, size) = place(s, 0.0);
                                        let (tx, ty, _, _) = place(tw.s(), tw.side());
                                        let fade = 1.0 - k;
                                        for j in 0..7u32 {
                                            let u = j as f32 / 6.0;
                                            let wob = (glitch_hash(i as u32, bucket, j) - 0.5)
                                                * 6.0
                                                * size;
                                            let pc = [SPLIT_RED, SPLIT_CYAN, team_colour(tw.who())]
                                                [j as usize % 3];
                                            c.rect(
                                                tx + (x - tx) * u - 1.5 * size,
                                                ty + (y - ty) * u - 1.5 * size + wob,
                                                3.0 * size,
                                                3.0 * size,
                                                rgba(pc, fade),
                                            );
                                        }
                                        for j in 0..10u32 {
                                            let ang = glitch_hash(i as u32, j, 50)
                                                * std::f32::consts::TAU;
                                            let dist = (r * 0.4
                                                + (8.0 + 26.0 * glitch_hash(i as u32, j, 51)) * k)
                                                * size;
                                            let pc =
                                                [col, SPLIT_RED, SPLIT_CYAN, WHITE][j as usize % 4];
                                            let px = (2.0 + 3.0 * glitch_hash(i as u32, j, 52))
                                                * fade
                                                * size;
                                            c.rect(
                                                x + ang.cos() * dist - px / 2.0,
                                                y + ang.sin() * dist - px / 2.0,
                                                px,
                                                px,
                                                rgba(pc, fade),
                                            );
                                        }
                                    }
                                    continue;
                                }
                            }
                            if on_lane(s, r) {
                                let a = if judged.is_some() {
                                    0.4
                                } else {
                                    note_alpha(s) * radar_lit(s, n.time, n.velocity)
                                };
                                let mut nfx =
                                    self.tko().and_then(|r| r.note(i, vt)).unwrap_or_default();
                                nfx.path = self.tko().and_then(|r| r.note_path(i, vt, s, lane_len));
                                if a > 0.01 && !nfx.hidden {
                                    let a = if theirs.is_some() { 0.3 * a } else { a };
                                    if over_panel(&nfx) {
                                        on_path.push((
                                            s,
                                            false,
                                            i,
                                            self.game.flows()[i],
                                            col,
                                            a,
                                            nfx,
                                        ));
                                    } else {
                                        let from = c.shapes.len();
                                        draw_hit(c, s, n, i, self.game.flows()[i], col, a, nfx);
                                        if nfx.above {
                                            above_shapes.push(from..c.shapes.len());
                                        }
                                    }
                                    if let Some(ring) = theirs {
                                        let (x, y, _, size) = place(side_of(n.kind, s), 0.0);
                                        c.ring(x, y, (r + 2.0) * size, rgba(ring, 0.8 * a), 2.0);
                                    }
                                }
                                // Radar: a note the beam just passed flashes and pings.
                                let ping =
                                    radar_age(s, radar_u(n.time, n.velocity), lane_len, sweep)
                                        .filter(|&age| {
                                            radar
                                                && judged.is_none()
                                                && s > RADAR_NEAR
                                                && age < 0.35
                                        });
                                if let Some(age) = ping {
                                    let (x, y, _, size) = place(side_of(n.kind, s), 0.0);
                                    let k = age / 0.35;
                                    let fade = note_alpha(s) * (1.0 - k);
                                    let sprite = if n.big {
                                        Sprite::BigCircle
                                    } else {
                                        Sprite::HitCircle
                                    };
                                    c.light(
                                        sprite,
                                        x,
                                        y,
                                        r * size,
                                        rgba(WHITE, 0.25 * fade * (1.0 - k)),
                                    );
                                    c.ring(
                                        x,
                                        y,
                                        (r + 3.0 + 16.0 * k) * size,
                                        rgba(RADAR_COL, 0.85 * fade),
                                        1.0 + 2.5 * (1.0 - k),
                                    );
                                }
                            }
                        }
                    }
                }
                Kind::Roll => {
                    let pop = popup(n);
                    // A fake slider, as stable draws it (owner's test map, 2026-10-28): body right
                    // of the head by its length, cap *left* of it by the same. When the cap reaches
                    // the drum the head is gone, and body and cap scroll on, fading out over
                    // FAKE_FADE_MS, until they've gone FAKE_FREEZE_LENGTH of slider length further;
                    // there they stop (a -200 at 3x stops with its body just right of the drum).
                    // A -0.0001 one's cap lands with its head. Nothing shows before the head is on
                    // screen, so a long one is only ever body. Still drawn at the head's speed, and
                    // gimmick sections run past the BPM / SV limits this game clamps to, so long
                    // ones aren't exact.
                    // A pop-up one (see `popup`) stands still, fading in, and is gone at its time.
                    let fake = n.fake_len > 0.0;
                    // A +0.0001 is gone at the drum, not passing through like a real roll
                    // (Lunatic Rough Party!!'s first object; owner, 2026-10-28, stable).
                    if n.tiny && !fake && vt >= n.time {
                        continue;
                    }
                    let live = fake && pop.is_none();
                    let lands = n.time - n.fake_len;
                    let shift = if live { fake_freeze(n, vt) } else { 0.0 };
                    let at = |time: f64| s_of(time + shift, n.velocity);
                    let s0 = pop.map_or_else(|| at(n.time), |p| p.0);
                    let mut fake_a = pop.map_or(1.0, |p| p.2);
                    let landed = live && vt >= lands;
                    if landed {
                        let p = ((vt - lands) / FAKE_FADE_MS) as f32;
                        if p >= 1.0 {
                            continue;
                        }
                        fake_a = 1.0 - p;
                    }
                    if fake_a < 0.01 {
                        continue;
                    }
                    let (s_cap, s1) = match pop {
                        Some((s, d, _)) => (s - d, s + d),
                        None => (at(n.time - n.fake_len), at(n.end_time + n.fake_len)),
                    };
                    if s1 < -HIT_X - r || s0 > lane_len + r {
                        continue;
                    }
                    let (s0, s1) = if boxed {
                        (s0.max(behind(r)), s1.min(lane_len))
                    } else {
                        (s0, s1)
                    };
                    let a = if memory_over {
                        0.0
                    } else {
                        fade_in(s0) * fake_a
                    };
                    let body_r = r * 0.86;
                    if bent {
                        // The body follows the lane in short pieces (none across a portal). Where
                        // a body already lies, another adds nothing to see: gimmick maps stack
                        // hundreds along a long rail, millions of pieces a frame, more than a
                        // graphics card can hold.
                        let (from, to) = (s0.max(-HIT_X - r), s1.min(lane_len + r));
                        let steps = ((to - from) / 6.0).ceil().max(1.0) as usize;
                        let along = |k: usize| from + (to - from) * k as f32 / steps as f32;
                        let at = |s: f32| {
                            let p = place(s, 0.0);
                            (p.0, p.1)
                        };
                        let cell = |s: f32| ((s + HIT_X + BIG_R) / BODY_CELL).max(0.0) as usize;
                        let pieces: Vec<_> = (0..steps)
                            .filter(|&k| {
                                let mid = (along(k) + along(k + 1)) / 2.0;
                                !body_laid.get(cell(mid)).is_some_and(|&laid| laid)
                            })
                            .map(|k| (at(along(k)), at(along(k + 1))))
                            .collect();
                        for (rr, col) in [(body_r + 2.5, rgba(WHITE, a)), (body_r, rgba(ROLL, a))] {
                            for (p0, p1) in &pieces {
                                if (p1.0 - p0.0).hypot(p1.1 - p0.1) < 20.0 {
                                    segment(c, *p0, *p1, rr, col);
                                }
                            }
                        }
                        // Only a solid body hides what's under it.
                        if a >= 0.99 {
                            for k in 0..steps {
                                let mid = (along(k) + along(k + 1)) / 2.0;
                                if let Some(laid) = body_laid.get_mut(cell(mid)) {
                                    *laid = true;
                                }
                            }
                        }
                    } else if let Some([_, sy]) = c.sprite_size(Sprite::RollMiddle) {
                        // The skin's body stretched from the head to the end, its cap just past that.
                        let (x0, x1) = (HIT_X + s0, HIT_X + s1);
                        c.sprite_box(
                            Sprite::RollMiddle,
                            (x0 + x1) / 2.0,
                            LANE_Y,
                            (x1 - x0) / 2.0,
                            r * sy,
                            rgba(ROLL, a),
                        );
                        if let Some([ex, ey]) = c.sprite_size(Sprite::RollEnd) {
                            let xe = if fake { HIT_X + s_cap } else { x1 };
                            c.sprite_box(
                                Sprite::RollEnd,
                                xe + r * ex,
                                LANE_Y,
                                r * ex,
                                r * ey,
                                rgba(ROLL, a),
                            );
                        }
                    } else {
                        c.capsule(
                            [HIT_X + s0, HIT_X + s1],
                            LANE_Y,
                            body_r,
                            rgba(ROLL, a),
                            rgba(WHITE, a),
                            2.5,
                        );
                        if fake {
                            c.circle(
                                HIT_X + s_cap,
                                LANE_Y,
                                body_r,
                                rgba(ROLL, a),
                                rgba(WHITE, a),
                                2.5,
                            );
                        }
                    }
                    if landed {
                        continue;
                    }
                    // Ticks in sliderscorepoint's own colour, as stable draws them, and only on
                    // the body: a last one rounded past the end isn't drawn beyond it.
                    // Ticks too close to tell apart are one: a gimmick roll can have hundreds of
                    // thousands on screen, more than a frame can hold.
                    let mut drawn_at = None;
                    for (k, hit) in st.ticks().iter().enumerate() {
                        let ts = s_of(n.time + k as f64 * n.tick_spacing, n.velocity);
                        let apart = drawn_at.is_none_or(|at| ts - at >= TICK_GAP);
                        if !hit && ts > s0 && ts <= s1 && ts < lane_len && apart {
                            drawn_at = Some(ts);
                            let (x, y, ..) = place(ts, 0.0);
                            if !c.sprite(Sprite::RollTick, x, y, NOTE_R, rgba(WHITE, a)) {
                                c.circle(x, y, 4.0, rgba(WHITE, 0.85 * a), rgba(WHITE, 0.0), 0.0);
                            }
                        }
                    }
                    let (x, y, _, size) = place(s0, 0.0);
                    if !skin_note(c, x, y, r * size, n.big, ROLL, a) {
                        c.circle(
                            x,
                            y,
                            r * size,
                            rgba(ROLL, a),
                            rgba(WHITE, a),
                            r * size * 0.1,
                        );
                    }
                }
                Kind::Swell => {
                    if st.finished() {
                        if st.judged() == Some(Judge::Great) {
                            let p = ((t - st.judged_at()) / 300.0).clamp(0.0, 1.0) as f32;
                            if p < 1.0 {
                                c.ring(
                                    HIT_X,
                                    LANE_Y,
                                    52.0 + 40.0 * p,
                                    rgba(SWELL, 1.0 - p),
                                    6.0 * (1.0 - p) + 1.0,
                                );
                            }
                        }
                        continue;
                    }
                    if vt < n.time {
                        let s = s_of(n.time, n.velocity);
                        let preempt =
                            stable_preempt(self.game.map().first_velocity * mods.scroll());
                        let a = note_alpha(s)
                            * radar_lit(s, n.time, n.velocity)
                            * swell_fade(preempt, n.time - vt);
                        if s < lane_len + r && a > 0.01 {
                            let (x, y, _, size) = place(s, 0.0);
                            let r = r * size;
                            // A map shipping its own spinner-warning draws its swells as small notes.
                            let drawn =
                                if map_spinner && c.sprite_size(Sprite::SpinnerWarning).is_some() {
                                    c.sprite_box(
                                        Sprite::SpinnerWarning,
                                        x,
                                        y,
                                        NOTE_R * size,
                                        NOTE_R * size,
                                        rgba(WHITE, a),
                                    );
                                    true
                                } else {
                                    c.sprite(Sprite::SpinnerWarning, x, y, r, rgba(WHITE, a))
                                };
                            if !drawn {
                                c.circle(x, y, r, rgba(SWELL, a), rgba(WHITE, a), r * 0.1);
                                c.ring(x, y, r * 0.55, rgba(WHITE, 0.8 * a), 3.0);
                            }
                        }
                    } else if c.sprite(
                        Sprite::SpinnerCircle,
                        HIT_X,
                        LANE_Y,
                        SPIN_R,
                        rgba(WHITE, 1.0),
                    ) {
                        // The skin's ring closes in on its circle as the time to hit it runs out.
                        let left = 1.0
                            - ((t - n.time) / (n.end_time - n.time).max(1.0)).clamp(0.0, 1.0)
                                as f32;
                        c.sprite(
                            Sprite::SpinnerApproach,
                            HIT_X,
                            LANE_Y,
                            SPIN_R * (1.0 + 1.5 * left),
                            rgba(WHITE, 1.0),
                        );
                        let left = n.required_hits.saturating_sub(st.hits());
                        texts.push((
                            HIT_X,
                            LANE_Y - 12.0,
                            24.0,
                            rgba(WHITE, 1.0),
                            left.to_string(),
                        ));
                    } else {
                        let progress = st.hits() as f32 / n.required_hits.max(1) as f32;
                        c.circle(
                            HIT_X,
                            LANE_Y,
                            SPIN_R,
                            rgba(SWELL, 0.95),
                            rgba(WHITE, 1.0),
                            4.0,
                        );
                        c.ring(
                            HIT_X,
                            LANE_Y,
                            SPIN_R + 4.0 + 14.0 * progress,
                            rgba(SWELL, 0.6),
                            3.0,
                        );
                        let left = n.required_hits.saturating_sub(st.hits());
                        texts.push((
                            HIT_X,
                            LANE_Y - 12.0,
                            24.0,
                            rgba(WHITE, 1.0),
                            left.to_string(),
                        ));
                    }
                }
            }
        }

        // Kiai: every head on screen takes its own flash on the beat, in its own colour
        // brightened, added over everything drawn, so a pile of fake sliders under one note (a
        // "shiny") lights it brighter the deeper it is. Stable's `UpdateKiai`; see `kiai_flash`.
        if self.view.kiai_effects {
            for &i in &self.view.visible {
                let n = &self.game.map().notes[i];
                let r = if n.big { BIG_R } else { NOTE_R };
                let sprite = if n.big {
                    Sprite::BigCircle
                } else {
                    Sprite::HitCircle
                };
                // A .tko note's motion moves (or hides) its flash with it.
                let mut fx = NoteFx::default();
                let (s, col, a) = match n.kind {
                    Kind::Don | Kind::Kat if self.game.state()[i].judged().is_none() => {
                        let s = note_s(i);
                        if let Some(rt) = self.game.tko() {
                            fx = rt.note(i, vt).unwrap_or_default();
                            fx.path = rt.note_path(i, vt, s, lane_len);
                        }
                        if fx.hidden {
                            continue;
                        }
                        (
                            s,
                            if n.kind == Kind::Kat {
                                flash_kat
                            } else {
                                flash_don
                            },
                            note_alpha(s) * radar_lit(s, n.time, n.velocity) * fx.opacity,
                        )
                    }
                    Kind::Roll if popup(n).is_some() => {
                        let (s, _, pa) = popup(n).unwrap();
                        (s, ROLL, if memory_over { 0.0 } else { pa })
                    }
                    Kind::Roll if !(n.fake_len > 0.0 && vt >= n.time - n.fake_len) => {
                        let s = s_of(n.time, n.velocity);
                        (s, ROLL, if memory_over { 0.0 } else { fade_in(s) })
                    }
                    _ => continue,
                };
                // A fake slider ends at its head (its end_time is its time).
                let flash = self.kiai_flash(vt, n.time, n.end_time) * KIAI_FLASH_ALPHA;
                let col = kiai_brighten(col);
                if flash > 0.0 && a > 0.01 && on_lane(s, r) {
                    let (x, y, _, size) = place(side_of(n.kind, s), 0.0);
                    let (hx, hy, _, _) = place(0.0, 0.0);
                    let (x, y) = tko_moved(&fx, x, y, (hx, hy));
                    c.light(sprite, x, y, r * size * fx.scale, rgba(col, flash * a));
                }
            }
            // Fakes flash like the notes they pass for.
            for &k in &fakes_on {
                let n = &self.game.map().fakes[k];
                let r = if n.big { BIG_R } else { NOTE_R };
                let s = s_of(n.time, n.velocity);
                let mut fx = NoteFx::default();
                if let Some(rt) = self.game.tko() {
                    fx = rt.fake(k, vt).unwrap_or_default();
                    fx.path = rt.fake_path(k, vt, s, lane_len);
                }
                if fx.hidden {
                    continue;
                }
                let a = note_alpha(s) * radar_lit(s, n.time, n.velocity) * fx.opacity;
                let flash = self.kiai_flash(vt, n.time, n.end_time) * KIAI_FLASH_ALPHA;
                let col = kiai_brighten(if n.kind == Kind::Kat {
                    flash_kat
                } else {
                    flash_don
                });
                if flash > 0.0 && a > 0.01 && on_lane(s, r) {
                    let sprite = if n.big {
                        Sprite::BigCircle
                    } else {
                        Sprite::HitCircle
                    };
                    let (x, y, _, size) = place(side_of(n.kind, s), 0.0);
                    let (hx, hy, _, _) = place(0.0, 0.0);
                    let (x, y) = tko_moved(&fx, x, y, (hx, hy));
                    c.light(sprite, x, y, r * size * fx.scale, rgba(col, flash * a));
                }
            }
        }

        if drum > 0.0 {
            c.transform_since(lane_shapes, [0.0, 0.0], 1.0, drum, 0.0);
        }

        // Flashlight / Spotlight / Blackout: dark everywhere but a circle around the drum.
        let fl_combo = match self.game.stats().combo {
            0..100 => 1.0,
            100..200 => 0.9,
            _ => 0.8,
        };
        if mods.has(Mod::Flashlight) {
            darken_outside(c, HIT_X, LANE_Y, 190.0 * fl_combo, w);
        } else if mods.has(Mod::Spotlight) {
            darken_outside(c, HIT_X, LANE_Y, 190.0 * fl_combo * (0.85 + 0.3 * pulse), w);
        }
        if mods.has(Mod::Blackout) && in_kiai {
            darken_outside(c, HIT_X, LANE_Y, 70.0, w);
        }

        // Drum panel on top so passed notes slide underneath it.
        let flash = |at: f64| ((1.0 - (t - at) / 130.0).clamp(0.0, 1.0) * 0.95) as f32;
        let drum_x = if crossfire {
            for (k, f) in self.game.presses().map(flash).into_iter().enumerate() {
                if f > 0.0 {
                    let col = if k == 0 || k == 3 {
                        uni.kat()
                    } else {
                        uni.don()
                    };
                    c.glow(HIT_X, LANE_Y, NOTE_R, 18.0, rgba(col, 0.5 * f));
                }
            }
            HIT_X
        } else {
            draw_drum(c, self.game.presses().map(flash), uni.don(), uni.kat())
        };
        // Back to front, as they were met.
        for (s, fake, k, flow, col, a, nfx) in on_path {
            let n = if fake {
                &self.game.map().fakes[k]
            } else {
                &self.game.map().notes[k]
            };
            let from = c.shapes.len();
            draw_hit(c, s, n, k, flow, col, a, nfx);
            if nfx.above {
                above_shapes.push(from..c.shapes.len());
            }
        }
        // The combo: on the drum, or with Crossfire under the hit target.
        let combo_y = |h: f32| {
            if crossfire {
                lane_top + LANE_H + 8.0
            } else {
                LANE_Y - h / 2.0
            }
        };
        for (p, big, col) in flying {
            let y = LANE_Y - 80.0 * p * (2.0 - p);
            let r = if big { BIG_R } else { NOTE_R } * (1.0 + 0.15 * p);
            if uni.shape != NoteShape::Round {
                shaped_note(c, uni.shape, HIT_X, y, r, col, 1.0 - p);
            } else if !skin_note(c, HIT_X, y, r, big, col, 1.0 - p) {
                c.circle(HIT_X, y, r, rgba(col, 1.0 - p), rgba(WHITE, 1.0 - p), 3.0);
            }
        }
        // Moved off the drum, it's drawn with the rest of the interface.
        if self.game.stats().combo >= 2
            && !self.view.hidden.has(Part::Combo)
            && self.view.hud.place(Part::Combo).is_none()
        {
            let s = self.game.stats().combo.to_string();
            let skin_size = if s.len() > 3 { 17.0 } else { 22.0 };
            if c.number_width(Font::Combo, skin_size, &s).is_some() {
                upright.push((
                    drum_x,
                    combo_y(skin_size),
                    skin_size,
                    rgba(self.view.hud.colour(Part::Combo).unwrap_or(WHITE), 1.0),
                    Upright::Number(Font::Combo, s),
                ));
            } else {
                let size = if s.len() > 3 { 12.0 } else { 16.0 };
                let col = if crossfire { 0xe8e8f0 } else { 0x1d1d26 };
                let col = self.view.hud.colour(Part::Combo).unwrap_or(col);
                texts.push((drum_x, combo_y(size), size, rgba(col, 1.0), s));
            }
        }

        // Judgement text.
        if let Some((j, at)) = self.game.last_judge().filter(|&(j, _)| {
            splashes(j)
                && !self.view.hidden.has(Part::Judgements)
                && self.view.hud.place(Part::Judgements).is_none()
        }) {
            let age = (t - at) as f32;
            if age < 450.0 {
                let a = (1.0 - age / 450.0).min(1.0);
                let sprite = match j {
                    Judge::Great => Sprite::JudgeGreat,
                    Judge::Ok => Sprite::JudgeOk,
                    Judge::Miss => Sprite::JudgeMiss,
                };
                // The skin's sits in the hit target, popping a little as it shows. At its own size
                // next to the notes as osu! draws it: lazer fits the 128 px hitcircle into a 95
                // unit note but draws judgements pixel for pixel, so they're 128/95 of ours.
                if let Some([_, sh]) = c.sprite_size(sprite) {
                    const JUDGE_SCALE: f32 = 128.0 / 95.0;
                    let h = 2.0
                        * NOTE_R
                        * sh
                        * JUDGE_SCALE
                        * (1.0 + 0.12 * (1.0 - age / 90.0).max(0.0));
                    upright.push((
                        HIT_X + drum,
                        LANE_Y - h / 2.0 - age * 0.02,
                        h,
                        rgba(self.view.hud.colour(Part::Judgements).unwrap_or(WHITE), a),
                        Upright::Image(sprite),
                    ));
                } else {
                    texts.push((
                        HIT_X + drum,
                        lane_top - 30.0 - age * 0.02,
                        15.0,
                        rgba(
                            self.view.hud.colour(Part::Judgements).unwrap_or(j.color()),
                            a,
                        ),
                        j.label().to_string(),
                    ));
                }
            }
        }

        // Multiverse: the window's frame (unless the lane is broken), and the burst as it takes
        // the focus: a flash and a ring off the drum.
        if boxed && !geo.bent() {
            let frame = tint(rgba(0x3a3a4e, 1.0), FOCUS_COL, look.focus.max(look.aim));
            c.round_rect(
                [0.0, lane_top - 3.0],
                [w, LANE_H + 6.0],
                4.0,
                rgba(0, 0.0),
                frame,
                2.0,
            );
        }
        // Called: a ping off the drum on each beat.
        if boxed && look.aim > 0.0 {
            let k = 1.0 - look.call.sqrt();
            c.ring(
                HIT_X,
                LANE_Y,
                NOTE_R + 140.0 * k,
                rgba(FOCUS_COL, look.aim * (1.0 - k)),
                3.0 + 6.0 * (1.0 - k),
            );
        }
        if let Some(b) = look.burst.filter(|_| boxed) {
            c.rect(
                0.0,
                lane_top,
                w,
                LANE_H,
                rgba(WHITE, 0.22 * (1.0 - b).powi(4)),
            );
            c.ring(
                HIT_X,
                LANE_Y,
                NOTE_R + 130.0 * b.sqrt(),
                rgba(FOCUS_COL, 0.9 * (1.0 - b).powi(2)),
                2.0 + 8.0 * (1.0 - b),
            );
        }

        // Whole-playfield moves: Mirror and Flip Flop turn it over, the rest move it about the
        // hit target (`pivot`), which lands on `dest`.
        let mirror = mods.has(Mod::Mirror);
        if mirror {
            c.mirror_since(start_shape, w);
        }
        let fx = if mods.has(Mod::FlipFlop) {
            flip_flop(&self.game.map().barlines, vt)
        } else {
            1.0
        } * (std::f32::consts::PI * tko_fx.flip.clamp(0.0, 1.0)).cos();
        if fx != 1.0 {
            c.squash_x_since(start_shape, w / 2.0, fx);
        }
        let flip_x = |x: f32| w / 2.0 + ((if mirror { w - x } else { x }) - w / 2.0) * fx;
        let pivot = [flip_x(HIT_X), LANE_Y];
        let (mut angle, mut zoom, mut dx, mut dy) = (0.0f32, rail_zoom, 0.0f32, rail_drop);
        let beat_no = beats.floor() as f64;
        let mut glitch = false;
        if party {
            // Sways, nods on the beat, shakes after big hits, pulses and glitches.
            dy += 14.0 * (t / 650.0).sin() as f32 + 10.0 * pulse;
            dx += 6.0 * (t / 1100.0).cos() as f32;
            let age = (t - self.game.quake_at()) as f32;
            if (0.0..260.0).contains(&age) {
                let amp = 7.0 * (1.0 - age / 260.0);
                dx += amp * (age * 0.9).sin();
                dy += amp * (age * 1.3).cos();
            }
            zoom *= 1.0 + 0.05 * pulse;
            glitch = beat_phase < 0.18 && hash01(beat_no * 3.1 + 0.7) < 0.35;
            if glitch {
                dx += (hash01(beat_no * 5.3 + 0.1) as f32 - 0.5) * 44.0;
                dy += (hash01(beat_no * 6.1 + 0.2) as f32 - 0.5) * 12.0;
            }
        }
        if mods.has(Mod::Tilt) {
            angle += 0.11 * (PI * beats).cos();
        }
        if mods.has(Mod::Spin) {
            angle += TAU * (vt / 24_000.0).rem_euclid(1.0) as f32;
        }
        if mods.has(Mod::BarrelRoll) {
            let i = self.game.kiai_starts().partition_point(|&(k, _)| k <= vt);
            if let Some(&(k, b)) = i
                .checked_sub(1)
                .and_then(|i| self.game.kiai_starts().get(i))
                && vt - k < 2.0 * b
            {
                angle += TAU * smooth(0.0, 2.0 * b, vt - k) as f32;
            }
        }
        angle += tko_fx.rotation.to_radians();
        zoom *= tko_fx.zoom;
        dx += tko_fx.x;
        dy += tko_fx.y;
        let mut dest = pivot;
        if crossfire {
            dest = [w / 2.0, LANE_Y];
        }
        if mods.has(Mod::Spin) {
            dest = [w / 2.0, 240.0];
        }
        if mods.has(Mod::Mania) {
            // Upright, the drum at the bottom (clear of the hit error meter).
            angle -= PI / 2.0;
            zoom *= 0.85;
            dest = [w / 2.0, 290.0];
        }
        // What's attached to the lane goes where it is, but only by what holds still: not swaying,
        // tilting or spinning with it, which would have the numbers shake about or fly round.
        let mania = mods.has(Mod::Mania);
        let (boxes, path, half) = field_boxes(&FieldView {
            w,
            rail: geo.rail.as_ref(),
            lane_len,
            flip: &flip_x,
            pivot,
            turned: if mania { -PI / 2.0 } else { 0.0 },
            zoom: rail_zoom * if mania { 0.85 } else { 1.0 },
            dest: [dest[0], dest[1] + rail_drop],
            whole_width: crossfire,
            spinning: mods.has(Mod::Spin),
        });
        self.ease_field_boxes(boxes, t);
        // A lane that's where it always is has nothing to go by.
        let plain = geo.rail.is_none() && !mania && !crossfire;
        self.view.lane_path = if plain { Vec::new() } else { path };
        self.view.lane_half = half;
        let dest = [dest[0] + dx, dest[1] + dy];
        // Notes drawn over every effect leave a hole where they were (nothing moves in the
        // list, which clips count by), and come back on top once the effects are on.
        let lifted = lift_shapes(c, &above_shapes);
        // A .tko map's trails, stretches and fringes turn and zoom with the playfield.
        // What the playfield is sized by from here on, so the lane is still told from the notes
        // once it is turned over or zoomed out.
        let size = [fx, 1.0];
        crate::tko::screen::local(c, start_shape, &tko_fx, pivot, size);
        let size = [
            size[0] * tko_fx.stretch_x * zoom,
            size[1] * tko_fx.stretch_y * zoom,
        ];
        if angle != 0.0 || zoom != 1.0 || dest != pivot {
            c.rotate_since(start_shape, pivot, angle, zoom, dest);
        }
        // Its copies go behind it, each where its own clips take it from the playfield's usual
        // place: the map moving, turning or zooming the playfield leaves them be (mods that sway
        // or spin it still take them along).
        let home = [dest[0] - tko_fx.x, dest[1] - tko_fx.y];
        let unmove = crate::tko::screen::Unmove {
            from: dest,
            angle: -tko_fx.rotation.to_radians(),
            zoom: if tko_fx.zoom.abs() > 1e-3 {
                1.0 / tko_fx.zoom
            } else {
                1.0
            },
            to: home,
        };
        crate::tko::screen::copies(c, start_shape, &tko_fx, [w / 2.0, home[1]], unmove, size);
        // Its tears, washes and flashes lie over the screen as it ends up.
        crate::tko::screen::screen(c, start_shape, &tko_fx, vt, size);
        // Over them all, where the playfield took them, the notes drawn over every effect.
        if !lifted.is_empty() {
            let from = c.shapes.len();
            c.shapes.extend(lifted);
            if angle != 0.0 || zoom != 1.0 || dest != pivot {
                c.rotate_since(from, pivot, angle, zoom, dest);
            }
        }
        let [x0, y0, x1, y1] = local_zone;
        let (x0, x1) = (flip_x(x0).min(flip_x(x1)), flip_x(x0).max(flip_x(x1)));
        self.view.zone = Some(turned_box(
            [x0, y0, x1 - x0, y1 - y0],
            pivot,
            angle,
            zoom,
            dest,
        ));
        // Texts go where the playfield took them, but stay upright and readable: their centre x,
        // top and height there.
        let (sin, cos) = angle.sin_cos();
        let moved = |x: f32, y: f32, size: f32| {
            let (rx, ry) = (
                (flip_x(x) - pivot[0]) * zoom,
                (y + size / 2.0 - pivot[1]) * zoom,
            );
            (
                dest[0] + rx * cos - ry * sin,
                dest[1] + rx * sin + ry * cos - size * zoom / 2.0,
                size * zoom,
            )
        };
        for (x, y, size, color, what) in upright {
            let (x, y, size) = moved(x, y, size);
            match what {
                Upright::Number(font, s) => {
                    if let Some(nw) = c.number_width(font, size, &s) {
                        c.number(font, x - nw / 2.0, y, size, color, &s);
                    }
                }
                Upright::Image(sprite) => {
                    if let Some([sw, sh]) = c.sprite_size(sprite) {
                        c.sprite_box(
                            sprite,
                            x,
                            y + size / 2.0,
                            size / 2.0 * sw / sh,
                            size / 2.0,
                            color,
                        );
                    }
                }
            }
        }
        for (x, y, size, color, text) in texts {
            let (x, y, size) = moved(x, y, size);
            c.text_centered(x, y, size, color, &text);
        }
        // Glitch: colour-split ghosts of everything but the big backdrops.
        if glitch {
            let end = c.shapes.len();
            for (tint, off) in [([1.0, 0.15, 0.3], -5.0), ([0.1, 0.9, 1.0], 5.0)] {
                for i in start_shape..end {
                    let mut sh = c.shapes[i];
                    if sh.half[0] > 150.0 {
                        continue;
                    }
                    sh.center[0] += off;
                    sh.fill = [tint[0], tint[1], tint[2], sh.fill[3] * 0.3];
                    sh.border = [tint[0], tint[1], tint[2], sh.border[3] * 0.3];
                    c.shapes.push(sh);
                }
            }
        }
        // Echo's bar stays where it is, whatever the playfield does.
        if mods.has(Mod::Echo) {
            self.draw_echo(c, vt, w, uni);
        }
    }

    /// Echo: the bar map time `t` is in and the one after it (start, length each). Before the
    /// first barline, the bar leading up to it.
    pub(super) fn echo_bars(&self, t: f64) -> Option<[(f64, f64); 2]> {
        let bars = &self.game.map().barlines;
        let fallback =
            4.0 * beatmap::control_point_at(&self.game.map().control_points, t).beat_length;
        let len_of = |i: usize| {
            bars.get(i + 1)
                .map_or(fallback, |b| b.time - bars[i].time)
                .max(1.0)
        };
        match bar_at(bars, t, fallback) {
            Some((i, start, len)) => Some([
                (start, len),
                (
                    start + len,
                    if i + 1 < bars.len() {
                        len_of(i + 1)
                    } else {
                        fallback
                    },
                ),
            ]),
            None => {
                let first = bars.first()?.time;
                let len = len_of(0);
                Some([(first - len, len), (first, len)])
            }
        }
    }

    /// Echo: the next bar's notes in colour on a line under the lane, and a cursor running along
    /// it with the bar being played. Each note rings as the cursor reaches it, a bar before it's
    /// hit; the line fades over to the next bar on the barline.
    pub(super) fn draw_echo(&self, c: &mut Canvas, vt: f64, w: f32, uni: &Universe) {
        let Some([(start, len), (next, next_len)]) = self.echo_bars(vt) else {
            return;
        };
        let p = ((vt - start) / len).clamp(0.0, 1.0);
        let fade = (smooth(0.0, 0.05, p) * (1.0 - smooth(0.94, 1.0, p))) as f32;
        let p = p as f32;
        let (x0, x1) = (HIT_X, w - 120.0);
        let x_at = |f: f32| x0 + (x1 - x0) * f;
        let line = rgba(0x4a4a60, fade);
        c.rect(x0, ECHO_Y - 0.75, x1 - x0, 1.5, line);
        for x in [x0, x1] {
            c.rect(x - 0.75, ECHO_Y - 5.0, 1.5, 10.0, line);
        }
        let notes = &self.game.map().notes;
        let from = notes.partition_point(|n| n.time < next - 1.0);
        for n in notes[from..]
            .iter()
            .take_while(|n| n.time < next + next_len - 1.0)
        {
            let col = match n.kind {
                Kind::Don => uni.don(),
                Kind::Kat => uni.kat(),
                _ => continue,
            };
            let f = ((n.time - next) / next_len) as f32;
            let (x, r) = (x_at(f), if n.big { 8.5 } else { 5.5 });
            // Ms since the cursor got to it.
            let age = (p - f) * len as f32;
            if age >= 0.0 {
                c.circle(x, ECHO_Y, r + 0.5, rgba(col, fade), rgba(WHITE, fade), 1.5);
                if age < ECHO_RING_MS {
                    let k = age / ECHO_RING_MS;
                    c.ring(
                        x,
                        ECHO_Y,
                        r + 3.0 + 10.0 * k,
                        rgba(col, 0.8 * (1.0 - k) * fade),
                        1.0 + 2.0 * (1.0 - k),
                    );
                }
            } else {
                c.circle(x, ECHO_Y, r, rgba(col, 0.6 * fade), rgba(col, 0.0), 0.0);
            }
        }
        c.rect(
            x_at(p) - 1.0,
            ECHO_Y - 11.0,
            2.0,
            22.0,
            rgba(WHITE, 0.85 * fade),
        );
    }

    /// The kiai flash at map time `t` on an object from `start` to `end`, stable's `UpdateKiai`
    /// (ppy/osu#26990): on each beat, if kiai is on then, 1 fading linearly to 0 over int(¾ beat)
    /// ms, or until the object's end if that's sooner. So a kiai with no beat in it never
    /// flashes, and the last flash finishes after the kiai ends.
    ///
    /// The beat is the first red line's, all map long, whatever line is in force (owner's
    /// source, 2026-10-28), and only objects starting within `KIAI_FLASH_REACH` of its beats
    /// ahead take it: a 2222 BPM first line lights only what's at the drum.
    pub(super) fn kiai_flash(&self, t: f64, start: f64, end: f64) -> f32 {
        let Some(first) = self.game.map().control_points.first() else {
            return 0.0;
        };
        let beat = first.beat_origin
            + ((t - first.beat_origin) / first.beat_length).floor() * first.beat_length;
        if !beatmap::control_point_at(&self.game.map().control_points, beat).kiai
            || start - beat > KIAI_FLASH_REACH * first.beat_length
        {
            return 0.0;
        }
        let len = (first.beat_length * 0.75).floor().min(end - beat);
        if len <= 0.0 || t - beat >= len {
            return 0.0;
        }
        1.0 - ((t - beat) / len) as f32
    }

    /// Sets `visual_offset`, except while paused: the vsync lead in it moves a little every frame,
    /// and on a frozen clock that shook the stopped lane.
    pub fn set_visual_offset(&mut self, ms: f64) {
        if self.game.paused().is_none() {
            self.view.visual_offset = ms;
        }
    }

    /// Map time the lane is drawn at: `time`, `visual_offset` real ms ahead.
    pub(super) fn visual_time(&self) -> f64 {
        if self.view.visual_offset == 0.0 {
            self.game.time()
        } else {
            self.game
                .speed()
                .map(self.game.raw_time() - self.game.offset() + self.view.visual_offset)
        }
    }

    /// Defense: the team's lives (a bar that empties) and shield, whose turn it is and when ours
    /// is; under the drum the gold, the shop (keys 1-4), tag-in and spotting, and what the team
    /// did lately.
    pub(super) fn draw_defense(&self, c: &mut Canvas, t: f64, w: f32) {
        let (Some(d), Some(left), Some((turn, wait))) = (
            self.game.defense(),
            self.game.lives_left(),
            self.game.defense_turn_at(t),
        ) else {
            return;
        };
        let st = d.state.unwrap_or_default();
        let bucket = (t / 40.0).floor() as u32;
        // Slowed: CRT scanlines over the screen, rolling in steps.
        let slow = (1.0 - self.slow_factor(t)) as f32 * 2.0;
        if slow > 0.0 {
            let roll = (bucket % 4) as f32;
            let mut ly = roll;
            while ly < VIRTUAL_HEIGHT {
                c.rect(0.0, ly, w, 1.0, rgba(SPLIT_CYAN, 0.07 * slow));
                ly += 4.0;
            }
        }
        // The team just did something: tears run across the screen in the logo's colours.
        if let Some(&(at, who, _)) = d
            .feed
            .last()
            .filter(|e| (0.0..GLITCH_MS).contains(&(t - e.0)))
        {
            let k = 1.0 - ((t - at) / GLITCH_MS) as f32;
            for j in 0..5u32 {
                let ty = glitch_hash(j, bucket, 70) * VIRTUAL_HEIGHT;
                let th = 1.0 + 5.0 * glitch_hash(j, bucket, 71);
                let tx = (glitch_hash(j, bucket, 72) - 0.5) * 60.0;
                let col = [TEAR_RED, TEAR_CYAN, team_colour(who)][j as usize % 3];
                c.rect(tx, ty, w, th, rgba(col, 0.55 * k));
            }
        }
        // A life lost: the panel shakes and slips red and cyan for a moment.
        let hurt = (1.0 - ((t - d.lost_at) / GLITCH_MS) as f32).clamp(0.0, 1.0);
        let shake = if hurt > 0.0 {
            (glitch_hash(1, bucket, 80) - 0.5) * 10.0 * hurt
        } else {
            0.0
        };
        let (pw, ph, x0, y0) = (260.0, 46.0, w / 2.0 - 130.0 + shake, 8.0);
        if hurt > 0.0 {
            let slip = 3.0 + 4.0 * hurt;
            c.round_rect(
                [x0 - slip, y0],
                [pw, ph],
                8.0,
                rgba(SPLIT_RED, 0.45 * hurt),
                rgba(0, 0.0),
                0.0,
            );
            c.round_rect(
                [x0 + slip, y0],
                [pw, ph],
                8.0,
                rgba(SPLIT_CYAN, 0.45 * hurt),
                rgba(0, 0.0),
                0.0,
            );
        }
        c.round_rect(
            [x0, y0],
            [pw, ph],
            8.0,
            rgba(0x14141c, 0.85),
            rgba(team_colour(d.me), 0.9),
            1.0,
        );
        let k = left as f32 / d.lives.max(1) as f32;
        let fill = if k > 0.5 {
            0x86E05A
        } else if k > 0.2 {
            0xFFD34D
        } else {
            0xEB452C
        };
        c.round_rect(
            [x0 + 10.0, y0 + 7.0],
            [pw - 20.0, 6.0],
            3.0,
            rgba(0x2a2a38, 1.0),
            rgba(0, 0.0),
            0.0,
        );
        if left > 0 {
            c.round_rect(
                [x0 + 10.0, y0 + 7.0],
                [(pw - 20.0) * k.min(1.0), 6.0],
                3.0,
                rgba(fill, 1.0),
                rgba(0, 0.0),
                0.0,
            );
        }
        // Row 1: the lives left, the shield on the right. Row 2: whose turn it is.
        let lives = d.lives;
        c.text(
            x0 + 10.0,
            y0 + 18.0,
            7.0,
            rgba(fill, 1.0),
            &tr!("LIVES {left}/{lives}", "ライフ {left}/{lives}", "ЖИЗНИ {left}/{lives}"; [] [left, lives]),
        );
        if st.shield > 0 {
            let shield = st.shield;
            c.text_right(
                x0 + pw - 10.0,
                y0 + 18.0,
                7.0,
                rgba(SPLIT_CYAN, 1.0),
                &tr!("+{shield} shield", "+{shield}シールド", "+{shield} щит"; [] [shield]),
            );
        }
        let (turn_text, col) = if wait == 0 {
            (
                tr!("YOUR TURN", "あなたの番", "ТВОЙ ХОД").to_string(),
                team_colour(d.me),
            )
        } else {
            let name = d.team.get(turn).map_or("?", |p| p.1.as_str());
            let name = Canvas::fit(7.0, 110.0, name);
            (
                tr!("{name}  (you in {wait})", "{name}  (あと{wait}小節)", "{name}  (ты через {wait})"; [] [name, wait]),
                team_colour(turn),
            )
        };
        c.text_centered(
            x0 + pw / 2.0,
            y0 + 31.0,
            7.0,
            rgba(col, 1.0),
            &Canvas::fit(7.0, pw - 20.0, &turn_text),
        );

        // What the team just did, and what it does, in a banner under the lives.
        let callout = d
            .feed
            .last()
            .filter(|e| (0.0..CALLOUT_MS).contains(&(t - e.0)))
            .map(|&(at, who, done)| {
                let name = d.team.get(who).map_or("?", |p| p.1.as_str());
                let (what, about) = said(done);
                (at, format!("{name}: {what} - {about}"), team_colour(who))
            });
        if let Some((at, text, col)) = callout {
            let k = (1.0 - ((t - at) / CALLOUT_MS) as f32 * 0.6)
                .clamp(0.0, 1.0)
                .min(((t - at) / 80.0) as f32);
            let text = Canvas::fit(7.5, w - 60.0, &text);
            let tw = Canvas::text_width(7.5, &text) + 24.0;
            let jit = if glitch_hash(2, bucket, 90) < 0.15 {
                (glitch_hash(3, bucket, 91) - 0.5) * 6.0
            } else {
                0.0
            };
            let (bx, by) = (w / 2.0 - tw / 2.0 + jit, 60.0);
            c.rect(bx - 2.0, by, tw, 18.0, rgba(SPLIT_RED, 0.35 * k));
            c.rect(bx + 2.0, by, tw, 18.0, rgba(SPLIT_CYAN, 0.35 * k));
            c.rect(bx, by, tw, 18.0, rgba(0x0b0b10, 0.92 * k));
            c.rect(bx, by + 16.5, tw, 1.5, rgba(col, k));
            c.text_centered(w / 2.0 + jit, by + 5.0, 7.5, rgba(WHITE, k), &text);
        }

        // The team panel, right under the drum (the scoreboard has the left).
        let (x, mut y) = (w - 198.0, 296.0);
        c.round_rect(
            [x - 4.0, y - 4.0],
            [192.0, 96.0],
            6.0,
            rgba(0x14141c, 0.8),
            rgba(0, 0.0),
            0.0,
        );
        let gold = st.gold;
        c.text(
            x,
            y,
            7.0,
            rgba(0xFFD34D, 1.0),
            &tr!("GOLD {gold}", "ゴールド {gold}", "ЗОЛОТО {gold}"; [] [gold]),
        );
        c.text_right(
            x + 184.0,
            y,
            6.5,
            rgba(0x9a9ab0, 1.0),
            tr!("a GREAT = 1 gold", "GREAT = 1ゴールド", "GREAT = 1 золото"),
        );
        y += 12.0;
        if !d.rules.shop {
            c.text(
                x,
                y,
                6.5,
                rgba(0x55556a, 1.0),
                tr!(
                    "the host turned the shop off",
                    "ホストがショップをオフにした",
                    "хост выключил магазин"
                ),
            );
        }
        for (k, item) in DefenseItem::ALL
            .into_iter()
            .enumerate()
            .filter(|_| d.rules.shop)
        {
            let (cx, cy) = (x + (k % 2) as f32 * 94.0, y + (k / 2) as f32 * 11.0);
            let price = item.price(&st);
            let tower = matches!(item, DefenseItem::DonTower | DefenseItem::KatTower);
            let can = st.gold >= price && !(tower && self.game.tower_active(t));
            c.text(
                cx,
                cy,
                6.5,
                rgba(if can { WHITE } else { 0x55556a }, 1.0),
                &Canvas::fit(6.5, 90.0, &format!("{} {} {price}", k + 1, item.name())),
            );
        }
        y += 34.0;
        let tag = if self.game.can_tag_in(t) {
            WHITE
        } else {
            0x55556a
        };
        let left = self.game.tag_ins_left();
        c.text(
            x,
            y,
            6.5,
            rgba(tag, 1.0),
            &tr!("T tag in ({left} left)", "T 交代 (残り{left})", "T перехват (осталось {left})"; [] [left]),
        );
        y += 12.0;
        for &(at, who, done) in d.feed.iter().rev().filter(|e| t - e.0 < 5000.0).take(2) {
            let name = d.team.get(who).map_or("?", |p| p.1.as_str());
            let what = said(done).0;
            let fade = (1.0 - ((t - at) / 5000.0) as f32).clamp(0.0, 1.0);
            c.text(
                x,
                y,
                6.5,
                rgba(team_colour(who), fade),
                &Canvas::fit(6.5, 184.0, &format!("{name}: {what}")),
            );
            y += 10.0;
        }
    }
}
/// Defense: what a player did, in words, and what it does.
pub(super) fn said(done: Done) -> (&'static str, &'static str) {
    match done {
        Done::Bought(item) => (item.name(), item.about()),
        Done::TaggedIn => (
            tr!("tagged in", "交代", "перехватил ход"),
            tr!(
                "took over the rest of this turn",
                "このターンの残りを引き継いだ",
                "забрал остаток этого хода"
            ),
        ),
    }
}

pub(super) fn draw_scare(c: &mut Canvas, p: f32, t: f64) {
    let w = c.width;
    let start = c.shapes.len();
    // Lunges in over the first 12%, then holds, shaking, and flickers out at the end.
    let k = 0.55 + 0.75 * (p / 0.12).min(1.0).powf(0.5);
    let shake = 14.0 * (1.0 - p * 0.5);
    let (sx, sy) = (
        shake * (t as f32 * 0.37).sin(),
        shake * (t as f32 * 0.53).cos(),
    );
    let flicker = if p > 0.85 && ((t / 40.0) as i64) % 2 == 0 {
        0.4
    } else {
        1.0
    };
    c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x050000, 0.92 * flicker));
    // A skin's own scare (its picture, or its animation's frame now) lunges in the same way,
    // filling the screen's height once it's in.
    let layout = c.skin_layout;
    if layout.overtime_frames > 0 {
        let frame = if layout.overtime_frame_ms > 0.0 {
            (p * SCARE_MS as f32 / layout.overtime_frame_ms) as usize % layout.overtime_frames
        } else {
            0
        };
        let sprite = Sprite::ALL[Sprite::Overtime0 as usize + frame];
        if let Some([aw, ah]) = c.sprite_size(sprite) {
            let hh = VIRTUAL_HEIGHT / 2.0 * k / 1.3;
            let hw = hh * aw / ah.max(1e-3);
            c.sprite_box(
                sprite,
                w / 2.0 + sx,
                VIRTUAL_HEIGHT / 2.0 + sy,
                hw,
                hh,
                rgba(WHITE, flicker),
            );
        }
        return;
    }
    let (fur, dark, snout) = (0x5a3418, 0x3a2010, 0xb98552);
    // Ears, head, hat.
    for side in [-1.0, 1.0] {
        c.circle(
            side * 125.0,
            -130.0,
            52.0,
            rgba(fur, 1.0),
            rgba(dark, 1.0),
            6.0,
        );
        c.circle(
            side * 125.0,
            -130.0,
            26.0,
            rgba(dark, 1.0),
            rgba(dark, 1.0),
            0.0,
        );
    }
    c.circle(0.0, 0.0, 165.0, rgba(fur, 1.0), rgba(dark, 1.0), 8.0);
    c.rect(-95.0, -200.0, 190.0, 16.0, rgba(0x0b0b0b, 1.0));
    c.rect(-60.0, -268.0, 120.0, 72.0, rgba(0x0b0b0b, 1.0));
    // Eye sockets, black, with tiny white pupils.
    for side in [-1.0, 1.0] {
        c.circle(
            side * 62.0,
            -52.0,
            40.0,
            rgba(0x000000, 1.0),
            rgba(dark, 1.0),
            4.0,
        );
        c.glow(side * 62.0, -50.0, 5.0, 10.0, rgba(0xffffff, 0.9));
        c.circle(
            side * 62.0,
            -50.0,
            5.0,
            rgba(0xffffff, 1.0),
            rgba(0xffffff, 1.0),
            0.0,
        );
    }
    // Snout, nose, and the jaw hanging open, teeth top and bottom.
    c.round_rect(
        [-95.0, -5.0],
        [190.0, 95.0],
        45.0,
        rgba(snout, 1.0),
        rgba(dark, 1.0),
        4.0,
    );
    c.round_rect(
        [-26.0, -8.0],
        [52.0, 30.0],
        12.0,
        rgba(0x0b0b0b, 1.0),
        rgba(0x0b0b0b, 1.0),
        0.0,
    );
    c.round_rect(
        [-85.0, 55.0],
        [170.0, 110.0],
        30.0,
        rgba(0x1a0404, 1.0),
        rgba(dark, 1.0),
        5.0,
    );
    for i in 0..7 {
        let x = -72.0 + i as f32 * 21.0;
        c.rect(x, 58.0, 15.0, 22.0, rgba(0xe8e0c8, 1.0));
        c.rect(x, 140.0, 15.0, 22.0, rgba(0xe8e0c8, 1.0));
    }
    // Bow tie under the chin.
    for side in [-1.0, 1.0] {
        c.round_rect(
            [side * 34.0 - 30.0, 168.0],
            [60.0, 34.0],
            8.0,
            rgba(0x0b0b0b, 1.0),
            rgba(0x0b0b0b, 1.0),
            0.0,
        );
    }
    c.circle(
        0.0,
        185.0,
        12.0,
        rgba(0x0b0b0b, 1.0),
        rgba(0x0b0b0b, 1.0),
        0.0,
    );
    // Everything but the backdrop was drawn about (0, 0): scale and shake it into place.
    c.transform_since(start + 1, [0.0, 0.0], k, w / 2.0 + sx, 230.0 + sy);
    for sh in &mut c.shapes[start..] {
        sh.fill[3] *= flicker;
        sh.border[3] *= flicker;
    }
}
/// Flashlight-style darkness everywhere outside a soft-edged circle of radius `r` at (`x`, `y`)
/// on a playfield `w` wide.
pub(super) fn darken_outside(c: &mut Canvas, x: f32, y: f32, r: f32, w: f32) {
    let far = w + 480.0;
    // Solid beyond the soft edge, then a few fainter rings fading into the lit circle.
    c.ring(x, y, far, rgba(0x08080b, 0.97), far - (r + 18.0));
    for k in 0..6 {
        let inner = r + k as f32 * 3.0;
        c.ring(
            x,
            y,
            inner + 3.0,
            rgba(0x08080b, 0.97 * (k as f32 + 0.5) / 6.0),
            3.0,
        );
    }
}
/// Copies of the shapes in `ranges`, which are left in place but drawn as nothing: shapes are
/// counted by their place in the list (clips, effects since a shape), so none may move.
pub(super) fn lift_shapes(
    c: &mut Canvas,
    ranges: &[std::ops::Range<usize>],
) -> Vec<crate::render::Shape> {
    let mut out = Vec::new();
    for r in ranges {
        let Some(shapes) = c.shapes.get_mut(r.clone()) else {
            continue;
        };
        out.extend_from_slice(shapes);
        for sh in shapes {
            sh.fill = [0.0; 4];
            sh.border = [0.0; 4];
        }
    }
    out
}
/// The box (x, y, w, h) around `rect` turned by `angle` and scaled by `scale` about `pivot`,
/// which lands on `dest` (as `Canvas::rotate_since` moves shapes).
pub(super) fn turned_box(
    rect: [f32; 4],
    pivot: [f32; 2],
    angle: f32,
    scale: f32,
    dest: [f32; 2],
) -> [f32; 4] {
    let (sin, cos) = angle.sin_cos();
    let [x, y, w, h] = rect;
    let corners = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)].map(|(px, py)| {
        let (dx, dy) = ((px - pivot[0]) * scale, (py - pivot[1]) * scale);
        (dest[0] + dx * cos - dy * sin, dest[1] + dx * sin + dy * cos)
    });
    let (x0, x1) = corners
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = corners
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    [x0, y0, x1 - x0, y1 - y0]
}
/// How the playfield stands this frame, for `field_boxes`.
pub(super) struct FieldView<'a> {
    pub(super) w: f32,
    pub(super) rail: Option<&'a Rail>,
    pub(super) lane_len: f32,
    /// Where mirroring takes an x.
    pub(super) flip: &'a dyn Fn(f32) -> f32,
    pub(super) pivot: [f32; 2],
    pub(super) turned: f32,
    pub(super) zoom: f32,
    pub(super) dest: [f32; 2],
    /// The lane runs both ways from the drum.
    pub(super) whole_width: bool,
    /// It goes all the way round `dest`.
    pub(super) spinning: bool,
}
/// Where the lane, the drum and the hit point are, the lane going by what of it is on the screen;
/// the points along it there, and how far its edges are from them.
pub(super) fn field_boxes(v: &FieldView) -> (hud::FieldBoxes, Vec<(f32, f32)>, f32) {
    let (sin, cos) = v.turned.sin_cos();
    let place = |x: f32, y: f32| {
        let (dx, dy) = (
            ((v.flip)(x) - v.pivot[0]) * v.zoom,
            (y - v.pivot[1]) * v.zoom,
        );
        (
            v.dest[0] + dx * cos - dy * sin,
            v.dest[1] + dx * sin + dy * cos,
        )
    };
    let half = LANE_H / 2.0 * v.zoom;
    let (w, h) = (v.w, VIRTUAL_HEIGHT);
    let mut lane = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut path = Vec::new();
    let mut take = |(x, y): (f32, f32)| {
        if x > -half && x < w + half && y > -half && y < h + half {
            path.push((x, y));
            lane = [
                lane[0].min(x - half),
                lane[1].min(y - half),
                lane[2].max(x + half),
                lane[3].max(y + half),
            ];
        }
    };
    take(place(0.0, LANE_Y));
    take(place(HIT_X, LANE_Y));
    match v.rail {
        Some(r) => r
            .pts
            .iter()
            .step_by(4)
            .for_each(|&(x, y, _)| take(place(x, y))),
        None => {
            let steps = (v.lane_len / 16.0).ceil() as usize;
            for i in 0..=steps {
                take(place(HIT_X + (i as f32 * 16.0).min(v.lane_len), LANE_Y));
            }
        }
    }
    if v.whole_width {
        (lane[0], lane[2]) = (0.0, w);
    }
    if v.spinning {
        let r = v.lane_len * v.zoom;
        lane = [v.dest[0] - r, v.dest[1] - r, v.dest[0] + r, v.dest[1] + r];
    }
    let [x0, y0, x1, y1] = lane;
    let (x0, y0, x1, y1) = (x0.max(0.0), y0.max(0.0), x1.min(w), y1.min(h));
    let boxed = |x: f32, y: f32, hw: f32, hh: f32| {
        let (a, b) = (place(x - hw, y - hh), place(x + hw, y + hh));
        [
            a.0.min(b.0),
            a.1.min(b.1),
            (a.0 - b.0).abs(),
            (a.1 - b.1).abs(),
        ]
    };
    let boxes = hud::FieldBoxes {
        lane: [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)],
        drum: boxed(PANEL_W / 2.0, LANE_Y, PANEL_W / 2.0, LANE_H / 2.0),
        hit: boxed(HIT_X, LANE_Y, NOTE_R, NOTE_R),
    };
    (boxes, if v.spinning { Vec::new() } else { path }, half)
}
/// The lane alone (top, height), for a preview it fills.
pub fn lane_only() -> (f32, f32) {
    (LANE_Y - LANE_H / 2.0, LANE_H)
}
/// Vertical band (top, height) of the lane with a little room for judgements, for previews that
/// show only the lane.
pub fn lane_band() -> (f32, f32) {
    (LANE_Y - LANE_H / 2.0 - 34.0, LANE_H + 42.0)
}
/// A don or kat drawn with the skin's images: the circle tinted `col`, then its overlay. False
/// (nothing drawn) when the skin has no circle for it.
pub fn skin_note(c: &mut Canvas, x: f32, y: f32, r: f32, big: bool, col: u32, alpha: f32) -> bool {
    let (circle, overlay) = if big {
        (Sprite::BigCircle, Sprite::BigCircleOverlay)
    } else {
        (Sprite::HitCircle, Sprite::HitCircleOverlay)
    };
    if !c.sprite(circle, x, y, r, rgba(col, alpha)) {
        return false;
    }
    c.sprite(overlay, x, y, r, rgba(WHITE, alpha));
    true
}
/// Units per sprite size unit (128 SD px) of osu!'s playfield images: its bar is 200 px tall.
pub(super) const LANE_PX: f32 = LANE_H / 200.0 * 128.0;
/// Drawn upright after the playfield's moves, like text.
pub(super) enum Upright {
    /// A number in the skin's font.
    Number(Font, String),
    /// A skin image.
    Image(Sprite),
}
/// The skin's lane over `x`..`x + w` (stretched across, like osu!), its kiai glow over it at
/// `glow`. False (nothing drawn) when the skin has none.
pub(super) fn skin_lane(c: &mut Canvas, x: f32, w: f32, tint: [f32; 4], glow: f32) -> bool {
    if c.sprite_size(Sprite::LaneBar).is_none() {
        return false;
    }
    c.sprite_box(
        Sprite::LaneBar,
        x + w / 2.0,
        LANE_Y,
        w / 2.0,
        LANE_H / 2.0,
        tint,
    );
    if glow > 0.0 && c.sprite_size(Sprite::LaneKiai).is_some() {
        c.sprite_box(
            Sprite::LaneKiai,
            x + w / 2.0,
            LANE_Y,
            w / 2.0,
            LANE_H / 2.0,
            rgba(WHITE, glow),
        );
    }
    true
}
/// The hit target: the skin's (osu!'s approach circle, and its big circle faintly round it, as
/// see-through as lazer has them) or the built-in rings.
pub(super) fn draw_hit_target(c: &mut Canvas) {
    let big = c.sprite(
        Sprite::HitTargetBig,
        HIT_X,
        LANE_Y,
        BIG_R,
        rgba(WHITE, 0.22),
    );
    let ring = c.sprite(Sprite::HitTarget, HIT_X, LANE_Y, NOTE_R, rgba(WHITE, 0.47));
    if !big && !ring {
        c.circle(
            HIT_X,
            LANE_Y,
            BIG_R,
            rgba(WHITE, 0.03),
            rgba(WHITE, 0.12),
            2.0,
        );
        c.circle(
            HIT_X,
            LANE_Y,
            NOTE_R + 1.0,
            rgba(WHITE, 0.06),
            rgba(WHITE, 0.4),
            2.5,
        );
    }
}
/// The drum panel and the drum, its halves lit by `flash` (left rim, left centre, right centre,
/// right rim: 0-1) in `don` / `kat` (the built-in drum; a skin's lights are its own). Returns
/// the drum's centre x.
pub(super) fn draw_drum(c: &mut Canvas, flash: [f32; 4], don: u32, kat: u32) -> f32 {
    let lane_top = LANE_Y - LANE_H / 2.0;
    // The skin's panel at its own size (osu!'s is 181 x 200 SD px, but some reach out round the
    // hit target), as tall as the lane.
    let k = LANE_PX / 128.0;
    let panel = c.sprite_size(Sprite::DrumPanel);
    let panel = panel.filter(|_| !black_lane());
    let skinned_panel = panel.is_some();
    if let Some([pw, _]) = panel {
        let hw = pw * LANE_PX / 2.0;
        c.sprite_box(
            Sprite::DrumPanel,
            hw,
            LANE_Y,
            hw,
            LANE_H / 2.0,
            rgba(WHITE, 1.0),
        );
    } else {
        let fill = if black_lane() { 0x000000 } else { 0x191921 };
        c.rect(0.0, lane_top, PANEL_W, LANE_H, rgba(fill, 1.0));
    }
    if c.sprite_size(Sprite::DrumInner).is_some() || c.sprite_size(Sprite::DrumOuter).is_some() {
        // Where each half's top-left goes (SD px into the panel): skins from before version 2.1
        // have smaller halves round a drum a little further left.
        let legacy = c.skin_layout.legacy_drum;
        let (centre, place) = if legacy {
            (
                85.5,
                [
                    (Sprite::DrumOuterFlipped, 13.0, 37.0),
                    (Sprite::DrumInner, 29.0, 50.0),
                    (Sprite::DrumInnerFlipped, 86.0, 50.0),
                    (Sprite::DrumOuter, 85.0, 37.0),
                ],
            )
        } else {
            (
                90.0,
                [
                    (Sprite::DrumOuterFlipped, 0.0, 0.0),
                    (Sprite::DrumInner, 0.0, 0.0),
                    (Sprite::DrumInnerFlipped, 90.0, 0.0),
                    (Sprite::DrumOuter, 90.0, 0.0),
                ],
            )
        };
        for ((sprite, x, y), a) in place.into_iter().zip(flash) {
            if let Some([sw, sh]) = c.sprite_size(sprite).filter(|_| a > 0.0) {
                let (hw, hh) = (sw * LANE_PX / 2.0, sh * LANE_PX / 2.0);
                c.sprite_box(
                    sprite,
                    x * k + hw,
                    lane_top + y * k + hh,
                    hw,
                    hh,
                    rgba(WHITE, a),
                );
            }
        }
        return centre * k;
    }
    let drum_x = PANEL_W / 2.0;
    if skinned_panel {
        // The panel has its drum drawn on: light it the built-in way.
        for (side, r, col, a) in [
            (1, 46.0, kat, flash[0]),
            (2, 46.0, kat, flash[3]),
            (1, 32.0, don, flash[1]),
            (2, 32.0, don, flash[2]),
        ] {
            c.half_circle(
                [drum_x, LANE_Y],
                r,
                side,
                rgba(col, a * 0.7),
                rgba(col, 0.0),
                0.0,
            );
        }
        return drum_x;
    }
    c.circle(
        drum_x,
        LANE_Y,
        46.0,
        rgba(0x2f2f3e, 1.0),
        rgba(0x4a4a60, 1.0),
        2.0,
    );
    c.half_circle(
        [drum_x, LANE_Y],
        46.0,
        1,
        rgba(kat, flash[0]),
        rgba(kat, 0.0),
        0.0,
    );
    c.half_circle(
        [drum_x, LANE_Y],
        46.0,
        2,
        rgba(kat, flash[3]),
        rgba(kat, 0.0),
        0.0,
    );
    c.circle(
        drum_x,
        LANE_Y,
        32.0,
        rgba(0xe9dcc3, 1.0),
        rgba(0x8a7b63, 1.0),
        2.0,
    );
    c.half_circle(
        [drum_x, LANE_Y],
        32.0,
        1,
        rgba(don, flash[1]),
        rgba(don, 0.0),
        0.0,
    );
    c.half_circle(
        [drum_x, LANE_Y],
        32.0,
        2,
        rgba(don, flash[2]),
        rgba(don, 0.0),
        0.0,
    );
    drum_x
}
/// The lane and its border. The border is two strips rather than a rect under the whole lane
/// (they reach a unit under the lane, so its edge blends into border colour just the same), and
/// the background isn't drawn under the lane at all (`mark_opaque`).
pub(super) fn draw_lane(c: &mut Canvas, x: f32, w: f32, color: [f32; 4], border: [f32; 4]) {
    let lane_top = LANE_Y - LANE_H / 2.0;
    c.rect(x, lane_top - 3.0, w, 4.0, border);
    c.rect(x, lane_top + LANE_H - 1.0, w, 4.0, border);
    c.rect(x, lane_top, w, LANE_H, color);
    c.mark_opaque();
}
/// A still playfield (lane, drum, a few notes) for the playfield area editor.
pub fn draw_preview(c: &mut Canvas) {
    let w = c.width;
    if !skin_lane(c, 0.0, w, rgba(WHITE, 1.0), 0.0) {
        draw_lane(c, 0.0, w, rgba(0x20202b, 1.0), rgba(0x3a3a4e, 1.0));
    }
    draw_hit_target(c);
    // Back to front, like gameplay.
    c.capsule(
        [HIT_X + 560.0, HIT_X + 760.0],
        LANE_Y,
        NOTE_R * 0.86,
        rgba(ROLL, 1.0),
        rgba(WHITE, 1.0),
        2.5,
    );
    c.circle(
        HIT_X + 560.0,
        LANE_Y,
        NOTE_R,
        rgba(ROLL, 1.0),
        rgba(WHITE, 1.0),
        NOTE_R * 0.1,
    );
    let colours = crate::game::note_colours();
    for (dx, col, big) in [
        (400.0, colours.don(), true),
        (260.0, colours.kat(), false),
        (150.0, colours.don(), false),
    ] {
        let r = if big { BIG_R } else { NOTE_R };
        if !skin_note(c, HIT_X + dx, LANE_Y, r, big, col, 1.0) {
            c.circle(
                HIT_X + dx,
                LANE_Y,
                r,
                rgba(col, 1.0),
                rgba(WHITE, 1.0),
                r * 0.1,
            );
        }
    }
    draw_drum(c, [0.0; 4], colours.don(), colours.kat());
    c.text_centered(
        w / 2.0,
        300.0,
        14.0,
        rgba(WHITE, 0.85),
        tr!(
            "drag to move, edges to resize",
            "ドラッグで移動、端でサイズ変更",
            "тяните — двигать, края — размер"
        ),
    );
}
/// Where a note drawn at `x`, `y` on the lane goes under its .tko motion: onto its group's
/// approach path (from the drum at `drum`, as far as the path's weight says), shifted by the
/// group's dx and dy, then turned about the drum by its swirl, the further out the wider.
pub(super) fn tko_moved(fx: &NoteFx, x: f32, y: f32, (hx, hy): (f32, f32)) -> (f32, f32) {
    let (x, y) = match fx.path {
        Some([px, py, w]) => (x + (hx + px - x) * w, y + (hy + py - y) * w),
        None => (x, y),
    };
    let (mut x, mut y) = (x + fx.dx, y + fx.dy);
    if fx.swirl != 0.0 {
        let (s, co) = fx.swirl.to_radians().sin_cos();
        let (ox, oy) = (x - HIT_X, y - LANE_Y);
        (x, y) = (HIT_X + ox * co - oy * s, LANE_Y + ox * s + oy * co);
    }
    (x, y)
}

/// Whether `j` gets its flash and label.
pub(super) fn splashes(j: Judge) -> bool {
    !(j == Judge::Great && playfield_look().hide_great)
}

/// The drum on the left: lazer's input drum (180 of a 768-tall screen, in our 480 units).
pub const PANEL_W: f32 = 112.5;

/// A spinner being hit: its circle over the drum (a skin's 128 px spinner image at this radius).
pub(super) const SPIN_R: f32 = 50.0;

pub(super) const HIT_ANIM_MS: f64 = 160.0;

/// A fake slider fades out over this once its cap reaches the drum, and stops scrolling after this
/// much more of slider length (owner's stable test map, 2026-10-28: -200 at 3x stops 238 ms on).
/// ponytail: the fade is only known to be 238..714 ms; measure it if a map looks off.
pub(super) const FAKE_FADE_MS: f64 = 500.0;

pub(super) const FAKE_FREEZE_LENGTH: f64 = 200.0;

/// The kiai beat flash on each note at its start: stable's `UpdateKiai` (ppy/osu#26990).
pub(super) const KIAI_FLASH_ALPHA: f32 = 0.3;

/// How far ahead of the beat, in first-red-line beats, objects take its flash: stable lit ~22
/// notes 250 ms apart at 30 BPM, short of the screen (owner, 2026-10-28).
/// ponytail: one measurement; stable's spawn window may also scale with SliderMultiplier (1.4
/// there). Re-measure at another BPM or multiplier if a map looks off.
pub(super) const KIAI_FLASH_REACH: f64 = 2.625;

pub(super) const ROLL: u32 = 0xFCB806;

pub(super) const SWELL: u32 = 0xF39A2B;

pub(super) const WHITE: u32 = 0xFFFFFF;

/// Colorless: dons and kats alike.
pub(super) const GREY: u32 = 0x8c8ca6;

/// Decoy: how opaque the fake notes are next to real ones.
pub(super) const DECOY_ALPHA: f32 = 0.6;

/// Multiverse: how far (virtual units) a playfield out of focus circles from its home, across
/// and up and down.
pub(super) const ORBIT: (f32, f32) = (46.0, 28.0);

/// Multiverse: a playfield's size out of focus and in it.
pub(super) const FIELD_ZOOM: (f32, f32) = (0.36, 0.6);

/// Multiverse: the colour of the focus (its halo, frame and burst).
pub(super) const FOCUS_COL: u32 = 0xbfe9ff;

/// Multiverse: how long (ms) the burst lasts as a playfield takes the focus.
pub(super) const FOCUS_BURST_MS: f64 = 600.0;

/// osu!px per ms -> virtual units per ms at scroll speed 1.0, so that a note stays on screen
/// exactly as long as in osu!. Stable scrolls notes at their velocity across a 480-tall gamefield
/// (the same units as ours) from its right edge to the hit target at x = 160; lazer copies that as
/// a time range, with the aspect ratio clamped to 5:4..16:9. Our hit target is osu!'s too, so this
/// is 1.0 except outside that aspect range, where the lane is longer or shorter than osu!'s.
pub(super) fn osu_speed_scale(w: f32) -> f64 {
    let aspect = (w / VIRTUAL_HEIGHT).clamp(5.0 / 4.0, 16.0 / 9.0) as f64;
    (w - HIT_X).max(1.0) as f64 / (aspect * VIRTUAL_HEIGHT as f64 - 160.0)
}

/// How much faster than on a plain lane notes go along a lane `len` long on a canvas `w` wide.
/// Rails only bend the lane: notes go along it as they would along a plain one, as many to a
/// stretch of lane, so a long rail (a spiral, a loop) shows more of the map ahead rather than
/// rushing it in. A playfield shrunk to fit the shape is the same picture, smaller: speeding
/// notes up to make up for it spread them out and rushed them along the rail. Mania's and Spin's
/// shorter lanes keep notes on them for the usual time.
pub(super) fn lane_stretch(rail: bool, len: f32, w: f32) -> f64 {
    if rail {
        1.0
    } else {
        (len / (w - HIT_X)) as f64
    }
}

/// How far ahead (ms) notes are looked for under a shared scroll, which may hold far notes near.
pub(super) const SHARED_LOOKAHEAD_MS: f64 = 30_000.0;

/// Pulse / Stutter's time shift of far notes at map time `t` (see `Scroll::warp`): Pulse makes
/// them jump forward on each beat, Stutter hold, then catch up.
pub(super) fn beat_warp(mods: Mods, cp: &beatmap::ControlPoint, t: f64) -> f64 {
    let p = cp.beat_phase(t);
    if mods.has(Mod::Pulse) {
        cp.beat_length * ((1.0 - (1.0 - p).powi(3)) - p)
    } else if mods.has(Mod::Stutter) {
        let x = ((p - 0.5) / 0.5).clamp(0.0, 1.0);
        cp.beat_length * (x * x * (3.0 - 2.0 * x) - p)
    } else {
        0.0
    }
}

/// Multiverse's breath, 0 to 1 over a beat `beat_ms` long at `phase`: it swells over the first
/// 90 ms (less of a fast beat) and eases out by the next, so no frame jumps on the beat.
pub(super) fn breath(phase: f64, beat_ms: f64) -> f32 {
    let rise = (90.0 / beat_ms.max(1.0)).min(0.4);
    if phase < rise {
        smooth(0.0, rise, phase) as f32
    } else {
        (1.0 - smooth(rise, 1.0, phase)) as f32
    }
}

/// Radar's beam.
pub(super) const RADAR_COL: u32 = 0x5ef0a0;

/// Radar: where the beam is, 0 at the drum to 1 at the lane's end, `beats` since the red line.
/// It leaves the drum every `RADAR_BEATS` beats.
pub(super) fn radar_sweep(beats: f32, kiai: bool) -> f32 {
    (beats / if kiai { RADAR_BEATS.1 } else { RADAR_BEATS.0 }).rem_euclid(1.0)
}

/// Radar: how lit that note is: fully as the beam passes, gone by the next one (and unseen till
/// it first passes them); always fully by the drum.
pub(super) fn radar_light(s: f32, u: f32, len: f32, sweep: f32) -> f32 {
    let near = smooth((RADAR_NEAR + 50.0) as f64, RADAR_NEAR as f64, s as f64) as f32;
    let lit = radar_age(s, u, len, sweep).map_or(0.0, |a| (1.0 - a).max(0.0).powf(1.5));
    lit.max(near)
}

/// Echo: where its bar sits, under the lane.
pub(super) const ECHO_Y: f32 = LANE_Y + LANE_H / 2.0 + 50.0;

/// Echo: how long a note rings once the cursor reaches it.
pub(super) const ECHO_RING_MS: f32 = 300.0;

/// Defense's glitches: a steady pseudo-random 0-1 for `a`, `b`, `k` (the medal screens' way:
/// time goes in 40 ms steps, so things jump rather than slide).
pub(super) fn glitch_hash(a: u32, b: u32, k: u32) -> f32 {
    let mut h =
        a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ k.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

/// How long a glitch from something the team did (or a life lost) lasts.
pub(super) const GLITCH_MS: f64 = 260.0;

/// Defense: how long a banner saying what the team just did stays up.
pub(super) const CALLOUT_MS: f64 = 3200.0;

pub fn team_colour(k: usize) -> u32 {
    TEAM_COLOURS[k % TEAM_COLOURS.len()]
}

/// How many of those there are on a lane `lane_len` long, past both ends.
pub(super) fn body_cells(lane_len: f32) -> usize {
    ((lane_len + HIT_X + 2.0 * BIG_R) / BODY_CELL).max(0.0) as usize + 2
}

/// The closest two roll ticks are drawn (lane units): they're 8 across.
pub(super) const TICK_GAP: f32 = 2.0;

/// Portal: notes reaching `entry` (distance along the lane) come out at `exit` and cover the
/// rest of the way in the time left. Returns where a note `s` along is drawn and how big
/// (shrinking into the entry, growing out of the exit).
pub(super) fn through_portal(s: f32, (entry, exit): (f32, f32)) -> (f32, f32) {
    if s <= 0.0 {
        (s, 1.0)
    } else if s >= entry {
        (s, ((s - entry) / 25.0).clamp(0.15, 1.0))
    } else {
        let s = s * exit / entry;
        (s, ((exit - s) / 25.0).clamp(0.15, 1.0))
    }
}

/// Mania: how much of the lane stands above the drum (virtual units).
pub(super) const MANIA_LANE: f32 = 340.0;

/// Spin: how long the lane is as it turns about the middle of the screen.
pub(super) const SPIN_LANE: f32 = 300.0;

/// Radar: notes this close to the drum are always fully lit.
pub(super) const RADAR_NEAR: f32 = 100.0;

/// Radar: how many beats the beam takes to cross the lane, and in kiai.
pub(super) const RADAR_BEATS: (f32, f32) = (2.0, 1.0);

/// Radar: how far a note `s` along a lane `len` long that comes `u` closer every sweep is from
/// the beam's last pass (0) to its next (1), the beam at `sweep`. None if the note wasn't on
/// screen yet when the beam last went by.
pub(super) fn radar_age(s: f32, u: f32, len: f32, sweep: f32) -> Option<f32> {
    let b = sweep * len;
    // How far the beam has gone since; the note came closer meanwhile, as the beam and the note
    // close in on each other at `len + u` a sweep.
    let back = if s <= b { b - s } else { b + len - s };
    let u = u.max(0.0);
    (s + u * back / (len + u) <= len).then_some(back / len)
}

/// Defense: the player on turn sees the drum end of the maze this big (the overview's
/// `RailShape::zoom` the rest of the time), with the drum this far down.
pub(super) const FOCUS_ZOOM: f32 = 0.9;

pub(super) const FOCUS_DROP: f32 = 50.0;

/// Defense: how long a slow lasts (the lane fades to half speed and back over `SLOW_FADE_MS`
/// at each end).
pub(super) const SLOW_MS: f64 = 6000.0;

pub(super) const SLOW_FADE_MS: f64 = 400.0;

/// Defense: the team's colours, in turn order.
pub const TEAM_COLOURS: [u32; 8] = [
    0x4fb4ff, 0xFFD34D, 0xc77dff, 0x86E05A, 0xff5fa2, 0xFF8A3D, 0x5ee6c8, 0xEB452C,
];

/// How much of a bent lane (units) counts as one stretch of it for roll bodies stacked there.
pub(super) const BODY_CELL: f32 = 6.0;

impl Play {
    /// Defense: the slows' speed factor at map time `t` (1, or down to a half).
    pub(super) fn slow_factor(&self, t: f64) -> f64 {
        let Some(d) = self.game.defense() else {
            return 1.0;
        };
        let k = d
            .slows
            .iter()
            .map(|&at| ((t - at).min(at + SLOW_MS - t) / SLOW_FADE_MS).clamp(0.0, 1.0))
            .fold(0.0, f64::max);
        1.0 - 0.5 * k
    }

    /// Panic / Mercy's speed factor at map time `t`: each recent miss fades in over 250 ms and
    /// out over its span.
    pub(super) fn miss_factor(&self, t: f64, mods: Mods) -> f64 {
        let weight = |span: f64| -> f64 {
            self.game
                .misses()
                .iter()
                .map(|&at| t - at)
                .filter(|age| (0.0..span).contains(age))
                .map(|age| (age / 250.0).min(1.0) * (1.0 - age / span))
                .sum()
        };
        if mods.has(Mod::Panic) {
            (1.0 + 0.15 * weight(4000.0)).min(2.0)
        } else if mods.has(Mod::Mercy) {
            1.0 / (1.0 + 0.25 * weight(3000.0)).min(2.0)
        } else {
            1.0
        }
    }

    /// Tact: the map time notes are drawn at for map time `t`: a crawl through its step, then a
    /// rush to the step's end (`tact::shown_time`).
    pub(super) fn tact_time(&self, t: f64) -> f64 {
        let bars = &self.game.map().barlines;
        crate::tact::shown_time(t, |x| {
            let cp = beatmap::control_point_at(&self.game.map().control_points, x);
            let bar = bar_at(bars, x, cp.beat_length * 4.0);
            crate::tact::step_start(
                self.game.tact(),
                x,
                bar,
                |i| bars.get(i).map_or(x, |b| b.time),
                (cp.beat_origin, cp.beat_length),
            )
        })
    }

    /// The warp (see `Scroll::warp`) of Drop, Lag Spike and Moonwalk at map time `t`.
    pub(super) fn extra_warp(&self, t: f64, cp: &beatmap::ControlPoint, mods: Mods) -> f64 {
        let mut warp = 0.0;
        if mods.has(Mod::Drop) {
            // Two beats before a kiai: held still for one and a half, then a rush to catch up.
            let i = self.game.kiai_starts().partition_point(|&(k, _)| k <= t);
            if let Some(&(k, b)) = self.game.kiai_starts().get(i) {
                let s = t - (k - 2.0 * b);
                if s >= 0.0 {
                    warp += if s < 1.5 * b {
                        s
                    } else {
                        1.5 * b * (1.0 - smooth(0.0, 1.0, (s - 1.5 * b) / (0.5 * b)))
                    };
                }
            }
        }
        if mods.has(Mod::LagSpike) {
            // In 40% of 1.5 s slots, one freeze of 150-370 ms, then a jump to where notes should be.
            let slot = (t / 1500.0).floor();
            if hash01(slot * 1.37 + 0.31) < 0.4 {
                let at = slot * 1500.0 + 200.0 + 900.0 * hash01(slot * 2.11 + 0.17);
                let len = 150.0 + 220.0 * hash01(slot * 3.7 + 0.53);
                if (at..at + len).contains(&t) {
                    warp += t - at;
                }
            }
        }
        if mods.has(Mod::Moonwalk) {
            // The first eighth of a bar the notes back away, the rest of it they make up for it.
            if let Some((_, start, len)) =
                bar_at(&self.game.map().barlines, t, cp.beat_length * 4.0)
            {
                let back = 0.125 * len;
                let s = t - start;
                warp -= if s < back {
                    2.0 * s
                } else {
                    2.0 * back * (1.0 - (s - back) / (len - back)).max(0.0)
                };
            }
        }
        warp
    }

    /// The lane's shape at map time `t` on a canvas `w` wide: rail (rails mods, Shapeshift
    /// morphing from one to the next, or a .tko map's lane clip, `lane`: a `SHAPESHIFT` shape and
    /// how far into it, which goes before the player's mods), length, and how far the playfield
    /// moves down and shrinks for it.
    pub(super) fn rail_at(
        &self,
        t: f64,
        w: f32,
        lane: Option<(usize, f32)>,
        mods: Mods,
    ) -> (Option<Rail>, f32, f32, f32) {
        if let Some((k, u)) = lane {
            let shape = LANE_SHAPES_OF_CLIPS[k % LANE_SHAPES_OF_CLIPS.len()];
            return morph_rail(RailShape::Straight, shape, u.clamp(0.0, 1.0), w);
        }
        if let Some(shape) = RailShape::of(mods) {
            let (mut drop, mut zoom) = (shape.drop(), shape.zoom(w));
            // Defense: the player on turn sees the drum end of the maze up close.
            let f = self.defense_focus(t);
            if f > 0.0 {
                drop += (FOCUS_DROP - drop) * f;
                zoom += (FOCUS_ZOOM.max(zoom) - zoom) * f;
            }
            return (
                Some(Rail::build(|s| shape.bend(s), shape.length(w))),
                shape.length(w),
                drop,
                zoom,
            );
        }
        if !mods.has(Mod::Shapeshift) {
            let len = if mods.has(Mod::Mania) {
                MANIA_LANE
            } else if mods.has(Mod::Crossfire) {
                // From the drum in the middle to either edge.
                w / 2.0
            } else if mods.has(Mod::Spin) {
                SPIN_LANE
            } else {
                w - HIT_X
            };
            return (None, len, 0.0, 1.0);
        }
        // A shape every so many bars (the player's setup), in the map's order; the first bars of
        // each morph into it from the last.
        let bars = &self.game.map().barlines;
        let (every, morph) = (
            self.game.shapeshift().bars(),
            self.game.shapeshift().morph_bars(),
        );
        let shape = |section: usize| {
            self.game
                .shape_order()
                .get(section)
                .map_or(RailShape::Straight, |&k| {
                    SHAPESHIFT[k as usize % SHAPESHIFT.len()]
                })
        };
        let (from, to, u) = match bar_at(bars, t, 2000.0) {
            None => (RailShape::Straight, RailShape::Straight, 1.0),
            Some((i, ..)) => {
                let section = i / every;
                let prev = if section == 0 {
                    RailShape::Straight
                } else {
                    shape(section - 1)
                };
                let first = section * every;
                let start = bars[first].time;
                // The morph ends part way into a bar (half a bar), or on a barline.
                let whole = morph.floor() as usize;
                let at = |k: usize| bars.get(first + k).map(|b| b.time);
                let base = at(whole).unwrap_or(start + 2000.0 * whole as f64);
                let bar_len = at(whole + 1).map_or(2000.0, |next| next - base);
                let end = base + (morph - whole as f64) * bar_len;
                (
                    prev,
                    shape(section),
                    ((t - start) / (end - start).max(1.0)).clamp(0.0, 1.0) as f32,
                )
            }
        };
        morph_rail(from, to, u, w)
    }

    /// Shatter's sideways shift of each quarter of the lane at map time `t`: apart on even bars
    /// (each piece its own way), back in line on odd ones, sliding over the first quarter-bar.
    pub(super) fn shatter_at(&self, t: f64, mods: Mods) -> [f32; 4] {
        let mut out = [0.0; 4];
        if !mods.has(Mod::Shatter) {
            return out;
        }
        let Some((i, start, len)) = bar_at(&self.game.map().barlines, t, 2000.0) else {
            return out;
        };
        let target = |bar: usize, k: usize| -> f32 {
            if bar % 2 == 1 {
                return 0.0;
            }
            let h = hash01(bar as f64 * 7.1 + k as f64 * 1.7) as f32 * 2.0 - 1.0;
            h.signum() * (20.0 + 30.0 * h.abs())
        };
        let m = smooth(0.0, 0.25, (t - start) / len) as f32;
        for (k, d) in out.iter_mut().enumerate().skip(1) {
            let prev = if i == 0 { 0.0 } else { target(i - 1, k) };
            *d = prev + (target(i, k) - prev) * m;
        }
        out
    }

    /// Portal's entry and exit (distances along a lane `len` long) at map time `t`: a new pair
    /// every 8 bars.
    pub(super) fn portal_at(&self, t: f64, len: f32, mods: Mods) -> Option<(f32, f32)> {
        if !mods.has(Mod::Portal) {
            return None;
        }
        let section =
            bar_at(&self.game.map().barlines, t, 2000.0).map_or(0, |(i, ..)| i / 8) as f64;
        let entry = len * (0.5 + 0.3 * hash01(section * 1.3 + 0.2) as f32);
        let mut exit = len * (0.15 + 0.5 * hash01(section * 2.9 + 0.6) as f32);
        if (entry - exit).abs() < 0.15 * len {
            exit = entry * 0.45;
        }
        Some((entry, exit))
    }

    /// Defense: how far the camera is zoomed in on the drum at map time `t`: all the way on our
    /// turn, easing in over the second half of the bar before it and out over the first half
    /// of the bar after.
    pub(super) fn defense_focus(&self, t: f64) -> f32 {
        let Some(d) = self.game.defense() else {
            return 0.0;
        };
        let bar = self.game.bar_at(t);
        let mine = |b: usize| self.game.turn_of_bar(b) == d.me;
        if mine(bar) {
            return 1.0;
        }
        let bars = &self.game.map().barlines;
        let (Some(start), Some(next)) = (bars.get(bar), bars.get(bar + 1)) else {
            return 0.0;
        };
        let u = ((t - start.time) / (next.time - start.time).max(1.0)).clamp(0.0, 1.0);
        let out = if bar > 0 && mine(bar - 1) {
            1.0 - smooth(0.0, 0.5, u)
        } else {
            0.0
        };
        let into = if mine(bar + 1) {
            smooth(0.5, 1.0, u)
        } else {
            0.0
        };
        out.max(into) as f32
    }
}

/// A don or kat `r` across in `shape`. Round is the usual note (skins draw their own).
pub fn shaped_note(c: &mut Canvas, shape: NoteShape, x: f32, y: f32, r: f32, col: u32, a: f32) {
    let (fill, white) = (rgba(col, a), rgba(0xffffff, a));
    match shape {
        NoteShape::Round => {
            c.circle(
                x,
                y,
                r + 2.0,
                rgba(0x000000, 0.35 * a),
                rgba(0x000000, 0.0),
                0.0,
            );
            c.circle(x, y, r, fill, white, r * 0.1);
        }
        NoteShape::Square => {
            let h = r * 0.9;
            c.round_rect(
                [x - h, y - h],
                [2.0 * h, 2.0 * h],
                r * 0.28,
                fill,
                white,
                r * 0.1,
            );
        }
        NoteShape::Diamond => {
            let first = c.shapes.len();
            let h = r * 0.82;
            c.round_rect(
                [x - h, y - h],
                [2.0 * h, 2.0 * h],
                r * 0.18,
                fill,
                white,
                r * 0.1,
            );
            c.rotate_since(first, [x, y], std::f32::consts::FRAC_PI_4, 1.0, [x, y]);
        }
        NoteShape::Ring => {
            c.circle(x, y, r, rgba(col, 0.22 * a), fill, r * 0.32);
            c.ring(x, y, r * 0.62, rgba(0xffffff, 0.8 * a), r * 0.07);
        }
        NoteShape::Pill => {
            c.capsule(
                [x - r * 0.45, x + r * 0.45],
                y,
                r * 0.78,
                fill,
                white,
                r * 0.1,
            );
        }
    }
}
