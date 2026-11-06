//! What a `.tko` map does to the picture itself: trails and kaleidoscopes of copies of the
//! playfield, colour fringes, tears, washes, vignettes and flashes.
//!
//! The renderer has no post-process pass, so all of it is made from the shapes the playfield just
//! drew: copy them, move and tint the copies, push them back. That is cheap (a frame only holds the
//! notes on screen) and exact, and it can't touch judging: it only reads what was drawn.
//!
//! Two steps, because some things must move with the playfield and some must not: `local` runs
//! before the playfield is rotated and zoomed (so a trail turns and zooms with it), `screen` after
//! (a glitch's slices and the washes are in screen space).

use super::runtime::{FieldFx, hash01};
use crate::render::{Canvas, Shape, VIRTUAL_HEIGHT, rgba, rotate_shapes};

/// Trail copies at most.
pub const MAX_ECHO: usize = 8;
/// Rotated copies at most, the original included.
pub const MAX_KALEIDO: usize = 8;
/// Slices a glitch tears the playfield into at most.
pub const MAX_BANDS: usize = 12;
/// How long a glitch holds one tear, in ms.
const GLITCH_STEP_MS: f64 = 70.0;
/// Shapes all the copies of a frame may add (a frame holds a few hundred).
const MAX_EXTRA_SHAPES: usize = 20_000;
/// `Shape::params[2]` this negative draws the shape as light, added to what is under it.
const ADDITIVE: f32 = -1.0e6;

/// Whether an effect has anything to do before the playfield moves.
pub fn has_local(fx: &FieldFx) -> bool {
    fx.stretch_x != 1.0
        || fx.stretch_y != 1.0
        || fx.echo >= 0.05
        || fx.kaleido >= 1.5
        || fx.chroma >= 0.5
}

/// Whether an effect has anything to do once it has.
pub fn has_screen(fx: &FieldFx) -> bool {
    fx.glitch >= 0.01
        || fx.wash >= 0.01
        || fx.vignette >= 0.01
        || fx.flash >= 0.01
        || fx.fade >= 0.01
        || fx.lane_fade >= 0.01
        || fx.blackout >= 0.01
}

/// Black behind the playfield, over what was drawn before it (the background): call before the
/// playfield draws.
pub fn backdrop(c: &mut Canvas, fx: &FieldFx) {
    let a = fx.backdrop.clamp(0.0, 1.0);
    if a >= 0.01 {
        c.rect(
            -200.0,
            -200.0,
            c.width + 400.0,
            VIRTUAL_HEIGHT + 400.0,
            [0.0, 0.0, 0.0, a],
        );
    }
}

/// The lane and its like: shapes as wide or tall as the screen. Copies leave them out, or each one
/// would lay its own dark lane over the notes.
fn is_panel(s: &Shape, w: f32) -> bool {
    s.half[0] >= 0.3 * w || s.half[1] >= 0.3 * VIRTUAL_HEIGHT
}

fn fade(shapes: &mut [Shape], alpha: f32) {
    for s in shapes {
        s.fill[3] *= alpha;
        s.border[3] *= alpha;
    }
}

/// `shapes` as coloured light of `strength`, added to what is under them.
fn light(shapes: &mut [Shape], rgb: [f32; 3], strength: f32) {
    for s in shapes {
        let a = s.fill[3].max(s.border[3]) * strength;
        let on = |alpha: f32| if alpha > 0.0 { a } else { 0.0 };
        s.fill = [rgb[0], rgb[1], rgb[2], on(s.fill[3])];
        s.border = [rgb[0], rgb[1], rgb[2], on(s.border[3])];
        s.params[2] = ADDITIVE;
    }
}

/// Puts `shapes` into the canvas at `at`, after what is there before it; what lay at or beyond it
/// keeps its place in the order and its clips, hit areas and opaque mark follow.
fn insert_at(c: &mut Canvas, at: usize, shapes: Vec<Shape>) {
    let n = shapes.len();
    if n == 0 {
        return;
    }
    c.shapes.splice(at..at, shapes);
    for (range, _) in &mut c.clips {
        if range.start >= at {
            range.start += n;
        }
        if range.end > at {
            range.end += n;
        }
    }
    for h in &mut c.hits {
        if h.shape >= at {
            h.shape += n;
        }
    }
    if let Some(o) = &mut c.opaque
        && *o >= at
    {
        *o += n;
    }
}

/// Stretching, trails, kaleidoscope and colour fringes of the shapes drawn since `start`, before
/// the playfield is rotated and zoomed about `pivot` (the hit target).
pub fn local(c: &mut Canvas, start: usize, fx: &FieldFx, pivot: [f32; 2]) {
    if start >= c.shapes.len() || !has_local(fx) {
        return;
    }
    let w = c.width;
    if fx.stretch_x != 1.0 || fx.stretch_y != 1.0 {
        c.stretch_since(start, pivot, fx.stretch_x, fx.stretch_y);
    }
    let echo = fx.echo.clamp(0.0, MAX_ECHO as f32);
    let kaleido = fx.kaleido.clamp(0.0, MAX_KALEIDO as f32).round() as usize;
    let chroma = fx.chroma.clamp(0.0, 80.0);
    if echo < 0.05 && kaleido < 2 && chroma < 0.5 {
        return;
    }
    let body: Vec<Shape> = c.shapes[start..]
        .iter()
        .filter(|s| !is_panel(s, w))
        .copied()
        .collect();
    if body.is_empty() {
        return;
    }
    let mut budget = MAX_EXTRA_SHAPES;
    // Behind the notes but over the lane: after the shapes the lane is made of.
    let at = start
        + c.shapes[start..]
            .iter()
            .take_while(|s| is_panel(s, w))
            .count();
    let mut behind: Vec<Shape> = Vec::new();
    // Trails: the furthest back first, so each nearer one lies over it.
    let count = echo.ceil() as usize;
    for i in (1..=count).rev() {
        if body.len() > budget {
            break;
        }
        let step = i as f32;
        let mut part = body.clone();
        rotate_shapes(
            &mut part,
            pivot,
            (fx.echo_rot * step).to_radians(),
            fx.echo_zoom.clamp(0.5, 1.5).powf(step),
            [pivot[0] + fx.echo_x * step, pivot[1] + fx.echo_y * step],
        );
        let mut alpha = fx.echo_fade.clamp(0.0, 1.0).powf(step);
        if i == count {
            alpha *= echo - (count - 1) as f32;
        }
        fade(&mut part, alpha);
        budget -= part.len();
        behind.extend(part);
    }
    // Kaleidoscope: the same notes turned about the middle of the screen.
    for k in 1..kaleido {
        if body.len() > budget {
            break;
        }
        let mut part = body.clone();
        let mid = [w / 2.0, VIRTUAL_HEIGHT / 2.0];
        rotate_shapes(
            &mut part,
            mid,
            std::f32::consts::TAU * k as f32 / kaleido as f32,
            1.0,
            mid,
        );
        fade(&mut part, 0.85);
        budget -= part.len();
        behind.extend(part);
    }
    insert_at(c, at, behind);
    // A red and a blue copy either side, as light.
    if chroma >= 0.5 && body.len() * 2 <= budget {
        for (sign, rgb) in [(-1.0, [1.0, 0.12, 0.12]), (1.0, [0.1, 0.55, 1.0])] {
            let mut part = body.clone();
            for s in &mut part {
                s.center[0] += sign * chroma;
            }
            light(&mut part, rgb, 0.6);
            c.shapes.extend(part);
        }
    }
}

/// An RGB of full strength for a hue in degrees.
pub fn hue_rgb(hue: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as u32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    }
}

/// Tears, washes, vignette and flash over the shapes drawn since `start`, once the playfield is
/// where it ends up. `t` is the game's clock, which picks what a glitch looks like (with the
/// graph's seed, so a replay shows the same).
pub fn screen(c: &mut Canvas, start: usize, fx: &FieldFx, t: f64) {
    if !has_screen(fx) {
        return;
    }
    let w = c.width;
    let h = VIRTUAL_HEIGHT;
    // See-through: the whole playfield, else only the lane.
    let fade_all = 1.0 - fx.fade.clamp(0.0, 1.0);
    let fade_lane = 1.0 - fx.lane_fade.clamp(0.0, 1.0);
    if (fade_all < 0.99 || fade_lane < 0.99) && start < c.shapes.len() {
        for s in &mut c.shapes[start..] {
            let a = if is_panel(s, w) {
                fade_all * fade_lane
            } else {
                fade_all
            };
            s.fill[3] *= a;
            s.border[3] *= a;
        }
    }
    let glitch = fx.glitch.clamp(0.0, 1.0);
    if glitch >= 0.01 && start < c.shapes.len() {
        let bands = 4 + (glitch * 8.0).round() as usize;
        let bands = bands.min(MAX_BANDS);
        let row = h / bands as f32;
        let slot = (t / GLITCH_STEP_MS).floor() as i64 as u64;
        let end = c.shapes.len();
        let mut budget = MAX_EXTRA_SHAPES;
        for k in 0..bands {
            if hash01(fx.seed, k as u64 * 7 + 1, slot) >= f64::from(glitch) * 0.7 {
                continue;
            }
            let (y0, y1) = (k as f32 * row, (k + 1) as f32 * row);
            let shift = (hash01(fx.seed, k as u64 * 7 + 2, slot) as f32 - 0.5)
                * 2.0
                * (8.0 + 70.0 * glitch);
            // Only what reaches into the slice is worth copying.
            let part: Vec<Shape> = c.shapes[start..end]
                .iter()
                .filter(|s| {
                    let r = s.half[0].hypot(s.half[1]);
                    !is_panel(s, w) && s.center[1] + r > y0 && s.center[1] - r < y1
                })
                .map(|s| Shape {
                    center: [s.center[0] + shift, s.center[1]],
                    ..*s
                })
                .collect();
            if part.is_empty() || part.len() > budget {
                continue;
            }
            budget -= part.len();
            let first = c.shapes.len();
            c.shapes.extend(part);
            c.clip_since(first, 0.0, y0, w, row);
        }
    }
    let wash = fx.wash.clamp(0.0, 1.0);
    if wash >= 0.01 {
        let [r, g, b] = hue_rgb(fx.wash_hue);
        c.rect(0.0, 0.0, w, h, [r, g, b, wash * 0.55]);
    }
    let vignette = fx.vignette.clamp(0.0, 1.0);
    if vignette >= 0.01 {
        let far = (w / 2.0).hypot(h / 2.0) * 1.05;
        let near = far * (0.3 + 0.5 * (1.0 - vignette));
        let rings = 12;
        let step = (far - near) / rings as f32;
        for i in 0..rings {
            let u = (i + 1) as f32 / rings as f32;
            let outer = near + step * (i + 1) as f32;
            // The last ring reaches past the corners.
            let width = if i + 1 == rings {
                step + 200.0
            } else {
                step + 1.0
            };
            c.ring(
                w / 2.0,
                h / 2.0,
                outer + if i + 1 == rings { 200.0 } else { 0.0 },
                rgba(0x000000, vignette * 0.95 * u * u),
                width,
            );
        }
    }
    let blackout = fx.blackout.clamp(0.0, 1.0);
    if blackout >= 0.01 {
        c.rect(
            -200.0,
            -200.0,
            w + 400.0,
            h + 400.0,
            [0.0, 0.0, 0.0, blackout],
        );
    }
    let flash = fx.flash.clamp(0.0, 1.0);
    if flash >= 0.01 {
        c.rect(0.0, 0.0, w, h, [1.0, 1.0, 1.0, flash * 0.9]);
        if let Some(s) = c.shapes.last_mut() {
            s.params[2] = ADDITIVE;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canvas 640 wide holding a lane and `notes` notes, as the playfield draws them.
    fn field(notes: usize) -> Canvas {
        let mut c = Canvas {
            width: 640.0,
            ..Canvas::default()
        };
        c.rect(0.0, 190.0, 640.0, 100.0, rgba(0x202020, 1.0));
        for i in 0..notes {
            c.circle(
                150.0 + 40.0 * i as f32,
                240.0,
                18.0,
                rgba(0xee4422, 1.0),
                rgba(0, 1.0),
                1.0,
            );
        }
        c
    }

    const PIVOT: [f32; 2] = [100.0, 240.0];

    #[test]
    fn a_plain_playfield_is_left_exactly_as_it_is() {
        let mut c = field(5);
        let before: Vec<[f32; 2]> = c.shapes.iter().map(|s| s.center).collect();
        let fx = FieldFx::default();
        assert!(!has_local(&fx) && !has_screen(&fx));
        local(&mut c, 0, &fx, PIVOT);
        screen(&mut c, 0, &fx, 1234.0);
        assert_eq!(c.shapes.len(), 6);
        assert_eq!(
            c.shapes.iter().map(|s| s.center).collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn a_trail_adds_a_copy_of_the_notes_per_step_and_leaves_the_lane_alone() {
        let mut c = field(5);
        let fx = FieldFx {
            echo: 3.0,
            ..FieldFx::default()
        };
        local(&mut c, 0, &fx, PIVOT);
        assert_eq!(
            c.shapes.len(),
            1 + 5 + 3 * 5,
            "one lane, the notes and 3 trails"
        );
        // The lane stays first and the original notes last, over their trails.
        assert!(c.shapes[0].half[0] >= 300.0);
        let last = &c.shapes[c.shapes.len() - 5..];
        assert!(
            last.iter()
                .all(|s| s.fill[3] == 1.0 && s.center[1] == 240.0)
        );
        // Further back means fainter and further along the step.
        let faint: Vec<f32> = c.shapes[1..16].iter().map(|s| s.fill[3]).collect();
        assert!(faint.iter().all(|a| *a < 1.0));
    }

    #[test]
    fn the_last_trail_copy_only_has_the_fraction_of_its_strength() {
        let mut c = field(1);
        let fx = FieldFx {
            echo: 1.5,
            echo_fade: 1.0,
            ..FieldFx::default()
        };
        local(&mut c, 0, &fx, PIVOT);
        let alphas: Vec<f32> = c.shapes[1..3].iter().map(|s| s.fill[3]).collect();
        assert_eq!(alphas, [0.5, 1.0]);
    }

    #[test]
    fn a_kaleidoscope_turns_copies_about_the_middle() {
        let mut c = field(2);
        let fx = FieldFx {
            kaleido: 4.0,
            ..FieldFx::default()
        };
        local(&mut c, 0, &fx, PIVOT);
        assert_eq!(c.shapes.len(), 1 + 2 + 3 * 2);
        // Half a turn puts a note at the mirror point through the centre (320, 240).
        let want = [640.0 - 190.0, 240.0];
        assert!(c.shapes.iter().any(|s| {
            (s.center[0] - want[0]).abs() < 0.01 && (s.center[1] - want[1]).abs() < 0.01
        }));
    }

    #[test]
    fn chroma_adds_a_red_and_a_blue_copy_as_light_on_top() {
        let mut c = field(3);
        let fx = FieldFx {
            chroma: 10.0,
            ..FieldFx::default()
        };
        local(&mut c, 0, &fx, PIVOT);
        assert_eq!(c.shapes.len(), 1 + 3 + 6);
        let top = &c.shapes[4..];
        assert!(top.iter().all(|s| s.params[2] < -0.5), "added as light");
        assert!(top[0].fill[0] > top[0].fill[2] && top[3].fill[2] > top[3].fill[0]);
        assert_eq!(top[0].center[0], 150.0 - 10.0);
        assert_eq!(top[3].center[0], 150.0 + 10.0);
    }

    #[test]
    fn stretching_makes_ovals_of_the_notes() {
        let mut c = field(1);
        let fx = FieldFx {
            stretch_x: 2.0,
            stretch_y: 0.5,
            ..FieldFx::default()
        };
        local(&mut c, 0, &fx, PIVOT);
        let n = &c.shapes[1];
        assert_eq!((n.half[0], n.half[1]), (36.0, 9.0));
    }

    #[test]
    fn copies_are_capped() {
        let mut c = field(2000);
        let fx = FieldFx {
            echo: 8.0,
            kaleido: 8.0,
            chroma: 30.0,
            ..FieldFx::default()
        };
        local(&mut c, 0, &fx, PIVOT);
        assert!(c.shapes.len() <= 1 + 2000 + MAX_EXTRA_SHAPES + 2000 * 2);
    }

    #[test]
    fn a_glitch_tears_slices_each_clipped_to_its_row_and_replays_the_same() {
        let fx = FieldFx {
            glitch: 1.0,
            seed: 9,
            ..FieldFx::default()
        };
        let run = |t: f64| {
            let mut c = field(8);
            screen(&mut c, 0, &fx, t);
            c
        };
        let (a, b) = (run(5000.0), run(5000.0));
        assert!(a.shapes.len() > 9, "some slice was torn");
        assert_eq!(a.clips.len(), b.clips.len());
        assert_eq!(
            a.shapes.iter().map(|s| s.center).collect::<Vec<_>>(),
            b.shapes.iter().map(|s| s.center).collect::<Vec<_>>()
        );
        for (range, [x, y, w, h]) in &a.clips {
            assert!(range.start >= 9 && range.end <= a.shapes.len());
            assert_eq!((*x, *w), (0.0, 640.0));
            assert!(*y >= 0.0 && y + h <= VIRTUAL_HEIGHT + 0.01);
        }
        // Another moment tears differently.
        let other = run(5000.0 + 10.0 * GLITCH_STEP_MS);
        assert_ne!(
            a.shapes
                .iter()
                .map(|s| s.center[0].to_bits())
                .collect::<Vec<_>>(),
            other
                .shapes
                .iter()
                .map(|s| s.center[0].to_bits())
                .collect::<Vec<_>>()
        );
    }

    /// A tear copies the notes and the drum, never the lane they run on: a copy of that wide dark
    /// panel, laid over the original in its slice, hides every note in it.
    #[test]
    fn a_glitch_never_lays_a_copy_of_the_lane_over_the_notes() {
        let fx = FieldFx {
            glitch: 1.0,
            seed: 3,
            ..FieldFx::default()
        };
        for step in 0..40 {
            let mut c = field(8);
            let before = c.shapes.clone();
            screen(&mut c, 0, &fx, 5000.0 + step as f64 * GLITCH_STEP_MS);
            assert_eq!(
                c.shapes[..before.len()]
                    .iter()
                    .map(|s| (s.center, s.half))
                    .collect::<Vec<_>>(),
                before
                    .iter()
                    .map(|s| (s.center, s.half))
                    .collect::<Vec<_>>(),
                "the playfield itself is left as it was"
            );
            assert!(
                c.shapes[before.len()..]
                    .iter()
                    .all(|s| !is_panel(s, c.width)),
                "step {step}: a panel-sized copy lies over the notes"
            );
        }
    }

    #[test]
    fn washes_vignette_and_flash_lie_over_the_picture() {
        let mut c = field(1);
        let fx = FieldFx {
            wash: 1.0,
            wash_hue: 120.0,
            vignette: 1.0,
            flash: 1.0,
            ..FieldFx::default()
        };
        screen(&mut c, 0, &fx, 0.0);
        let wash = &c.shapes[2];
        assert!(
            wash.fill[1] > wash.fill[0] && wash.fill[1] > wash.fill[2],
            "green"
        );
        assert!(c.shapes.len() > 2 + 10, "the vignette's rings");
        let flash = c.shapes.last().unwrap();
        assert_eq!(flash.fill, [1.0, 1.0, 1.0, 0.9]);
        assert!(flash.params[2] < -0.5, "added as light");
    }

    #[test]
    fn the_playfield_or_its_lane_alone_turns_see_through() {
        let mut c = field(2);
        let fx = FieldFx {
            lane_fade: 1.0,
            ..FieldFx::default()
        };
        screen(&mut c, 0, &fx, 0.0);
        assert_eq!(c.shapes[0].fill[3], 0.0, "the lane is gone");
        assert_eq!(c.shapes[1].fill[3], 1.0, "the notes are kept");
        let mut c = field(2);
        let fx = FieldFx {
            fade: 0.75,
            ..FieldFx::default()
        };
        screen(&mut c, 1, &fx, 0.0);
        assert_eq!(
            c.shapes[0].fill[3], 1.0,
            "what was drawn before is left alone"
        );
        assert!(c.shapes[1..].iter().all(|s| s.fill[3] == 0.25));
        assert_eq!(c.shapes.len(), 3, "nothing added");
    }

    #[test]
    fn a_blackout_covers_the_screen_and_a_backdrop_goes_under_the_playfield() {
        let mut c = field(1);
        let fx = FieldFx {
            blackout: 1.0,
            flash: 0.5,
            ..FieldFx::default()
        };
        screen(&mut c, 0, &fx, 0.0);
        let n = c.shapes.len();
        assert_eq!(
            c.shapes[n - 2].fill,
            [0.0, 0.0, 0.0, 1.0],
            "black over it all"
        );
        assert!(
            c.shapes[n - 1].params[2] < -0.5,
            "a flash still shows on black"
        );
        let mut c = Canvas {
            width: 640.0,
            ..Canvas::default()
        };
        let fx = FieldFx {
            backdrop: 0.5,
            ..FieldFx::default()
        };
        backdrop(&mut c, &fx);
        assert_eq!(c.shapes.len(), 1);
        assert_eq!(c.shapes[0].fill, [0.0, 0.0, 0.0, 0.5]);
        backdrop(&mut c, &FieldFx::default());
        assert_eq!(c.shapes.len(), 1, "none without one");
    }

    #[test]
    fn hues_walk_round_the_colour_wheel() {
        assert_eq!(hue_rgb(0.0), [1.0, 0.0, 0.0]);
        assert_eq!(hue_rgb(120.0), [0.0, 1.0, 0.0]);
        assert_eq!(hue_rgb(240.0), [0.0, 0.0, 1.0]);
        assert_eq!(hue_rgb(360.0 + 60.0), hue_rgb(60.0));
        assert_eq!(hue_rgb(-120.0), hue_rgb(240.0));
    }

    #[test]
    fn inserting_keeps_clips_and_marks_on_what_they_belonged_to() {
        let mut c = field(2);
        c.mark_opaque();
        c.clip_since(1, 0.0, 0.0, 10.0, 10.0);
        let marked = c.opaque;
        let extra = vec![c.shapes[1]; 4];
        insert_at(&mut c, 1, extra);
        assert_eq!(c.clips[0].0, 5..7);
        assert_eq!(c.opaque, marked.map(|o| o + 4));
        let mut d = field(2);
        d.mark_opaque();
        let extra = vec![d.shapes[1]; 4];
        insert_at(&mut d, 3, extra);
        assert_eq!(d.opaque, Some(2), "a mark before the insert stays");
    }
}
