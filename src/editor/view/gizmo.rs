//! Moving the playfield by hand in the preview (F4 Effects, .tko maps): a drag moves it, with
//! Shift it turns, with Ctrl it zooms, and each writes keys at the playhead, the way an
//! animation program records what is done to a picture. The keys go on the playfield's own
//! tracks (the Keyframes page edits them too) and show as diamonds over the clip rows, where a
//! click on one goes there and a right click takes that moment's keys away.

use super::clips::pose_mut;
use super::{Drag, Keys, Mode, View, inside};
use crate::editor::keyframes::Subject;
use crate::game::{HIT_X, LANE_Y};
use crate::render::{Canvas, accent, rgba};
use crate::tko::clips::{Pose, Show};
use crate::tko::gimmick::FieldProp;
use crate::tko::track::Key;
use crate::video::FRAME_VW;

/// What a drag in the preview does.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Grip {
    Move,
    Turn,
    Zoom,
}

/// The properties the preview's hand moves.
const MOTION: [FieldProp; 4] = [
    FieldProp::X,
    FieldProp::Y,
    FieldProp::Rotation,
    FieldProp::Zoom,
];

/// How close (units) to a diamond a click takes it.
const NEAR: f32 = 4.0;

impl View {
    /// Whether the preview can be moved by hand.
    pub(super) fn gizmo_on(&self) -> bool {
        self.mode == Mode::Effects
            && self.ed.is_tko()
            && !self.windowed()
            && self.g_page != super::motion::PAGE
    }

    /// Preview units per playfield unit.
    fn preview_scale(&self) -> f32 {
        self.layout.preview[2] / FRAME_VW
    }

    fn motion_at(&self, p: FieldProp, t: f64) -> f64 {
        self.ed.value_of(Subject::Field(p), t)
    }

    /// The text or picture block picked, with the pose a drag places (its end pose when that
    /// is being edited).
    fn picked_layer(&self) -> Option<(u32, Pose)> {
        let c = self.picked()?;
        let l = c.layer.as_deref()?;
        let p = match l.to {
            Some(to) if self.fx_end => to,
            _ => l.from,
        };
        Some((c.id, p))
    }

    /// Where the hit target is drawn in the preview, by the playfield's keys at the playhead; or
    /// the block picked, if it is text or a picture.
    pub(super) fn pivot_in_preview(&self) -> (f32, f32) {
        let [fx, fy, _, _] = self.layout.preview;
        let k = self.preview_scale();
        if let Some((_, p)) = self.picked_layer() {
            return (fx + (FRAME_VW / 2.0 + p.x as f32) * k, fy + p.y as f32 * k);
        }
        let t = self.ed.time;
        (
            fx + (HIT_X + self.motion_at(FieldProp::X, t) as f32) * k,
            fy + (LANE_Y + self.motion_at(FieldProp::Y, t) as f32) * k,
        )
    }

    /// The handle over the preview: a ring on the hit target, and what a drag does.
    pub(super) fn draw_gizmo(&mut self, c: &mut Canvas) {
        if !self.gizmo_on() || self.game.is_none() {
            return;
        }
        let [fx, fy, fw, fh] = self.layout.preview;
        self.draw_pose_frames(c);
        let (px, py) = self.pivot_in_preview();
        let live = matches!(self.drag, Drag::Gizmo { .. });
        let hover = c.hovered(fx, fy, fw, fh);
        if hover || live {
            let a = if live { 1.0 } else { 0.7 };
            c.ring(px, py, 9.0, rgba(accent(), a), 1.5);
            c.rect(px - 13.0, py - 0.5, 26.0, 1.0, rgba(accent(), a * 0.8));
            c.rect(px - 0.5, py - 13.0, 1.0, 26.0, rgba(accent(), a * 0.8));
            let hint = if self.picked_layer().is_some() {
                "drag: place it   Shift: turn   Ctrl: size"
            } else {
                "drag: move   Shift: turn   Ctrl: zoom   (keys at the playhead)"
            };
            c.text(
                fx + 4.0,
                fy + fh - 9.0,
                5.5,
                rgba(0xffffff, 0.8),
                &Canvas::fit(5.5, fw - 8.0, hint),
            );
        }
    }

    /// A moving text or picture block picked: frames where it is at its start and at its end,
    /// with a dotted path from one to the other, the pose being edited lit. The preview itself
    /// shows the block where it is at the playhead, which is neither while it travels.
    fn draw_pose_frames(&mut self, c: &mut Canvas) {
        let Some(clip) = self.picked() else { return };
        let Some(l) = clip.layer.as_deref() else {
            return;
        };
        let Some(to) = l.to else { return };
        let (from, end, show) = (l.from, self.fx_end, l.show.clone());
        let [fx, fy, _, _] = self.layout.preview;
        let k = self.preview_scale();
        let at = |p: &Pose| (fx + (FRAME_VW / 2.0 + p.x as f32) * k, fy + p.y as f32 * k);
        let size = |p: &Pose| -> (f32, f32) {
            let h = p.size as f32;
            let w = match &show {
                Show::Text { text } => Canvas::text_width(h, text),
                Show::Picture { .. } => h,
            };
            ((w * k).max(6.0), (h * k).max(6.0))
        };
        let (a, b) = (at(&from), at(&to));
        let steps = ((b.0 - a.0).hypot(b.1 - a.1) / 5.0) as usize;
        for i in 1..steps {
            let f = i as f32 / steps as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            c.rect(x - 0.6, y - 0.6, 1.2, 1.2, rgba(0xffffff, 0.45));
        }
        for (p, label, lit) in [(&from, "start", !end), (&to, "end", end)] {
            let (x, y) = at(p);
            let (w, h) = size(p);
            let colour = if lit { accent() } else { 0xffffff };
            let alpha = if lit { 0.95 } else { 0.4 };
            c.round_rect(
                [x - w / 2.0 - 2.0, y - h / 2.0 - 2.0],
                [w + 4.0, h + 4.0],
                2.0,
                rgba(0, 0.0),
                rgba(colour, alpha),
                if lit { 1.5 } else { 1.0 },
            );
            c.text_centered(x, y - h / 2.0 - 9.0, 5.5, rgba(colour, alpha), label);
        }
    }

    /// A press in the preview: picks up the playfield. False when it isn't for the gizmo.
    pub(super) fn gizmo_down(&mut self, x: f32, y: f32, keys: Keys) -> bool {
        if !self.gizmo_on() || !inside(self.layout.preview, x, y) {
            return false;
        }
        let grip = if keys.shift {
            Grip::Turn
        } else if keys.ctrl {
            Grip::Zoom
        } else {
            Grip::Move
        };
        if let Some((id, p)) = self.picked_layer() {
            self.drag = Drag::Gizmo {
                grip,
                from: (x, y),
                at: (p.x, p.y, p.turn, p.size),
                t: self.ed.time,
                layer: Some(id),
            };
            return true;
        }
        let t = self.ed.snapped(self.ed.time);
        let at = MOTION.map(|p| self.motion_at(p, t));
        self.drag = Drag::Gizmo {
            grip,
            from: (x, y),
            at: (at[0], at[1], at[2], at[3]),
            t,
            layer: None,
        };
        true
    }

    pub(super) fn gizmo_drag(&mut self, x: f32, y: f32) {
        let Drag::Gizmo {
            grip,
            from,
            at: (bx, by, rot, zoom),
            t,
            layer,
        } = self.drag
        else {
            return;
        };
        let k = self.preview_scale().max(1e-3);
        let round = |v: f64, to: f64| (v / to).round() * to;
        let keys: Vec<(FieldProp, f64)> = match grip {
            Grip::Move => vec![
                (FieldProp::X, round(bx + ((x - from.0) / k) as f64, 1.0)),
                (FieldProp::Y, round(by + ((y - from.1) / k) as f64, 1.0)),
            ],
            Grip::Turn => {
                let (px, py) = self.pivot_in_preview();
                let angle = |(ax, ay): (f32, f32)| (ay - py).atan2(ax - px).to_degrees() as f64;
                let mut d = angle((x, y)) - angle(from);
                d = (d + 180.0).rem_euclid(360.0) - 180.0;
                vec![(FieldProp::Rotation, round(rot + d, 1.0))]
            }
            Grip::Zoom => {
                // Up grows it, down shrinks it: doubled every 60 units.
                let f = 2f64.powf(((from.1 - y) / 60.0) as f64);
                vec![(FieldProp::Zoom, round(zoom * f, 0.01))]
            }
        };
        if let Some(id) = layer {
            let end = self.fx_end;
            let changed = self.ed.set_clip(id, true, |c| {
                if let Some(l) = c.layer.as_deref_mut() {
                    let pose = pose_mut(l, end);
                    for (p, v) in &keys {
                        match p {
                            FieldProp::X => pose.x = *v,
                            FieldProp::Y => pose.y = *v,
                            FieldProp::Rotation => pose.turn = *v,
                            // For a block, the zoom grip carries its size (units).
                            _ => pose.size = v.round().max(1.0),
                        }
                    }
                }
            });
            if changed {
                self.mark_stale();
            }
            return;
        }
        let ease = self.k_ease;
        let mut changed = false;
        for (p, v) in keys {
            changed |= self.ed.key(Subject::Field(p), Key { t, v, ease }, true);
        }
        if changed {
            self.mark_stale();
        }
    }

    /// The moments the preview's hand left keys at (any of the motion tracks), ascending.
    pub(super) fn motion_keys(&self) -> Vec<f64> {
        let mut out: Vec<f64> = MOTION
            .iter()
            .filter_map(|p| self.ed.track_of(Subject::Field(*p)))
            .flat_map(|tr| tr.keys.iter().map(|k| k.t))
            .collect();
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }

    /// Diamonds for the motion keys in the strip from `y` to `y + h` of the clip rows.
    pub(super) fn draw_motion_strip(&mut self, c: &mut Canvas, y: f32, h: f32) {
        let [x, _, w, _] = self.layout.graph;
        c.rect(x + 1.0, y, w - 2.0, h, rgba(0x000000, 0.25));
        c.text(
            x + 4.0,
            y + (h - 5.5) / 2.0,
            5.5,
            rgba(0x6a6a80, 1.0),
            "MOTION",
        );
        let cy = y + h / 2.0;
        for t in self.motion_keys() {
            let kx = self.x_of(t);
            if kx < x + 30.0 || kx > x + w {
                continue;
            }
            let on = (t - self.ed.time).abs() < 0.5;
            c.round_rect(
                [kx - 3.0, cy - 3.0],
                [6.0, 6.0],
                1.0,
                rgba(if on { 0xffffff } else { 0xFCB806 }, 1.0),
                rgba(0x000000, 0.8),
                1.0,
            );
        }
    }

    /// The motion key under `x` in the strip, if any.
    fn motion_key_near(&self, x: f32) -> Option<f64> {
        self.motion_keys()
            .into_iter()
            .filter(|&t| (self.x_of(t) - x).abs() <= NEAR)
            .min_by(|a, b| {
                (self.x_of(*a) - x)
                    .abs()
                    .total_cmp(&(self.x_of(*b) - x).abs())
            })
    }

    /// A click in the strip: to the key there. True when it was on one.
    pub(super) fn motion_strip_down(&mut self, x: f32) -> bool {
        match self.motion_key_near(x) {
            Some(t) => {
                self.set_time(t);
                true
            }
            None => false,
        }
    }

    /// A right click in the strip: that moment's motion keys go (one undo step).
    pub(super) fn motion_strip_right(&mut self, x: f32) -> bool {
        let Some(t) = self.motion_key_near(x) else {
            return false;
        };
        let changed = self.ed.edit_gimmick(false, |g| {
            for p in MOTION {
                let track = g.field_track_mut(p);
                if let Some(i) = track.near(t, 0.5) {
                    track.remove(i);
                }
            }
        });
        if changed {
            self.mark_stale();
        }
        true
    }
}
