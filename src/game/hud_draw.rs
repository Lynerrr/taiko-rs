//! Drawing the interface over a play: score, combo, HP, accuracy, the hit error bar and the
//! pause menu, each part where the player put it (`crate::hud`).

use super::*;
use crate::hit::{Canvas, Hit};
use crate::hud::{self, Part};
use crate::render::{Font, Sprite, VIRTUAL_HEIGHT, accent, progress, rgba};

impl Play {
    /// The interface's parts where the player put them (see `hud`), then the skip hint and the
    /// pause menu (with the map's name), which stay at fixed screen positions.
    pub fn draw_hud(&self, c: &mut Canvas) {
        let t = self.game.time;
        let w = c.width;
        if let Some(text) = &self.view.banner {
            let tw = Canvas::text_width(9.0, text);
            // Defense has its lives panel up there: the banner goes under it.
            let by = if self.game.defense.is_some() {
                90.0
            } else {
                14.0
            };
            c.round_rect(
                [w / 2.0 - tw / 2.0 - 12.0, by],
                [tw + 24.0, 20.0],
                10.0,
                rgba(0x173049, 0.9),
                rgba(0x4fb4ff, 1.0),
                1.0,
            );
            c.text_centered(w / 2.0, by + 5.5, 9.0, rgba(WHITE, 1.0), text);
        }

        self.draw_defense(c, t, w);

        self.draw_parts(c);
        // Overtime: he's here, over everything but the pause menu.
        if let Some(&at) = self
            .scares
            .iter()
            .find(|&&at| (at..at + SCARE_MS).contains(&t))
        {
            draw_scare(c, ((t - at) / SCARE_MS) as f32, t);
        }

        if self.game.can_skip()
            && self.game.paused.is_none()
            && !self.game.mute_hits
            && !self.view.watched
        {
            c.text_centered(
                w / 2.0,
                330.0,
                12.0,
                rgba(WHITE, 0.7),
                tr!(
                    "SPACE to skip intro",
                    "SPACEでイントロを飛ばす",
                    "ПРОБЕЛ: пропустить вступление"
                ),
            );
        }
        if let (Some(_), Some((start, end))) = (self.game.paused, self.game.resuming) {
            // Resume countdown: the playfield stays visible, lightly dimmed.
            let now = Instant::now();
            let left = end.saturating_duration_since(now).as_secs_f32();
            let total = (end - start).as_secs_f32().max(0.001);
            c.rect(
                0.0,
                0.0,
                w,
                480.0,
                rgba(0x0b0b10, 0.35 * (left / total).min(1.0)),
            );
            let (cx, cy) = (w / 2.0, 330.0);
            c.ring(cx, cy, 26.0, rgba(0x2a2a38, 0.9), 4.0);
            // Shrinking arc, drawn as dots around the ring.
            let n = 48;
            for k in 0..((left / total * n as f32).ceil() as usize).min(n) {
                let a = -std::f32::consts::FRAC_PI_2 + k as f32 / n as f32 * std::f32::consts::TAU;
                c.circle(
                    cx + a.cos() * 26.0,
                    cy + a.sin() * 26.0,
                    2.2,
                    rgba(accent(), 1.0),
                    rgba(accent(), 1.0),
                    0.0,
                );
            }
            c.text_centered(
                cx,
                cy - 7.0,
                14.0,
                rgba(WHITE, 1.0),
                &format!("{:.1}", left),
            );
            c.text_centered(
                cx,
                cy + 36.0,
                8.0,
                rgba(0x9a9ab0, 1.0),
                tr!(
                    "get ready   (Esc: back to menu)",
                    "準備して   (Esc: メニューに戻る)",
                    "приготовьтесь   (Esc: назад в меню)"
                ),
            );
        } else if let Some(cursor) = self.game.paused.filter(|_| !self.view.watched) {
            // The main menu's look: one panel, the options as notes on it (don and kat in turn),
            // the chosen one bigger and lit, the selector sliding between them.
            let t = progress(self.game.paused_at, 180.0);
            c.rect(0.0, 0.0, w, 480.0, rgba(0x0b0b10, 0.78 * t));
            let rise = (1.0 - t) * 12.0;
            let phone = crate::mobile::active();
            let row_h: f32 = if phone { 62.0 } else { 36.0 };
            let label: f32 = if phone { 18.0 } else { 11.0 };
            let pw = (w - 40.0).min(if phone { 480.0 } else { 340.0 });
            let ph = 92.0 + PAUSE_ITEMS as f32 * row_h + 26.0;
            let (px, py) = (w / 2.0 - pw / 2.0, (480.0 - ph) / 2.0 - 6.0 - rise);
            let start = c.shapes.len();
            crate::style::panel(c, px, py, pw, ph);
            let title = tr!("PAUSED", "ポーズ", "ПАУЗА");
            crate::style::title(
                c,
                w / 2.0 - Canvas::text_width(26.0, title) / 2.0,
                py + 14.0,
                26.0,
                WHITE,
                title,
            );
            // The map's name lives here rather than under the lane, where moved lanes ran into it.
            let m = &self.game.map.meta;
            let rate = if (self.game.rate - 1.0).abs() > 1e-6 {
                format!("   {:.2}x", self.game.rate)
            } else {
                String::new()
            };
            let fit = pw - 28.0;
            c.text_centered(
                w / 2.0,
                py + if phone { 44.0 } else { 50.0 },
                8.0,
                rgba(0x9a9ab0, 1.0),
                &Canvas::fit(8.0, fit, &format!("{} - {}", m.artist, m.title)),
            );
            c.text_centered(
                w / 2.0,
                py + if phone { 60.0 } else { 62.0 },
                8.0,
                rgba(0x6f6f86, 1.0),
                &Canvas::fit(
                    8.0,
                    fit,
                    &format!(
                        "[{}]  OD {:.1}{rate}",
                        m.version,
                        // As it plays: with EZ / HR / PR and at this speed.
                        crate::beatmap::od_at_rate(self.game.mods.od(m.od), self.game.rate)
                    ),
                ),
            );
            c.rect(px + 14.0, py + 78.0, pw - 28.0, 1.0, rgba(0x2a2a38, 1.0));
            c.fade_since(start, t, 1.0);
            let colours = crate::multiverse::note_colours();
            let (rx, rw) = (px + 12.0, pw - 24.0);
            let test = self.view.test_play;
            let items = (0..PAUSE_ITEMS).map(|i| match i {
                4 if test => tr!("Back to the editor", "エディターに戻る", "Назад в редактор"),
                _ => pause_item(i),
            });
            for (i, item) in items.enumerate() {
                // Options slide up one after another.
                let ti = progress(
                    self.game.paused_at + std::time::Duration::from_millis(25 * i as u64),
                    180.0,
                );
                let y = py + 88.0 + i as f32 * row_h + (1.0 - ti) * 18.0;
                let selected = i == cursor;
                let row = c.shapes.len();
                if selected {
                    c.round_rect(
                        [rx, y],
                        [rw, row_h - 6.0],
                        7.0,
                        rgba(
                            crate::style::selector_colour(),
                            0.12 + 0.05 * crate::style::pulse(),
                        ),
                        rgba(0, 0.0),
                        0.0,
                    );
                    crate::style::selector(c, "pause", [rx - 3.0, y - 3.0, rw + 6.0, row_h]);
                } else if c.hovered(rx, y, rw, row_h - 6.0) {
                    c.round_rect(
                        [rx, y],
                        [rw, row_h - 6.0],
                        7.0,
                        rgba(0x262634, 0.6),
                        rgba(0, 0.0),
                        0.0,
                    );
                }
                c.hit(rx, y, rw, row_h - 6.0, Hit::PauseItem(i));
                let (nx, ny) = (rx + 18.0, y + (row_h - 6.0) / 2.0);
                let col = if i % 2 == 0 {
                    colours.don()
                } else {
                    colours.kat()
                };
                let r = if selected { 9.0 } else { 7.0 };
                crate::style::note(c, nx, ny, r, col, selected, 0.6);
                c.text(
                    rx + 36.0,
                    y + (row_h - 6.0 - label) / 2.0,
                    label,
                    rgba(if selected { WHITE } else { 0xa8a8bc }, 1.0),
                    &Canvas::fit(label, rw - 48.0, item),
                );
                c.fade_since(row, ti, 1.0);
            }
            // Fades in once the options have slid past it.
            let th = progress(
                self.game.paused_at + std::time::Duration::from_millis(220),
                150.0,
            );
            let hint = tr!(
                "Up/Down + Enter    Esc: continue",
                "上下 + Enter    Esc: 続ける",
                "Вверх/вниз + Enter    Esc: продолжить"
            );
            // Nothing to say to a finger.
            if !phone {
                c.text_centered(
                    w / 2.0,
                    py + ph - 18.0,
                    7.0,
                    rgba(0x8a8aa0, th),
                    &Canvas::fit_hints(7.0, pw - 28.0, hint),
                );
            }
        }
        if let Some((_, at)) = self.game.failed {
            let p = progress(at, 400.0);
            c.rect(0.0, 0.0, w, 480.0, rgba(0x0b0b10, 0.6 * p));
            c.text_centered(
                w / 2.0,
                200.0,
                32.0,
                rgba(0xEB452C, p),
                tr!("FAILED", "失敗", "ПРОВАЛ"),
            );
        }
    }

    /// The part's place: where the player put it, or else where it goes to begin with.
    pub(super) fn part_place(&self, c: &Canvas, part: Part) -> hud::Place {
        use hud::{Place, Target, axis, on_screen};
        self.view.hud.place(part).unwrap_or_else(|| match part {
            Part::Score => Place::new(on_screen(1.0, -18.0), on_screen(0.0, 18.0)),
            Part::Accuracy => Place::new(on_screen(1.0, -18.0), on_screen(0.0, 50.0)),
            // On the accuracy's line, left of it.
            Part::Pp => {
                let accuracy_here = !self.view.hidden.has(Part::Accuracy)
                    && self.view.hud.place(Part::Accuracy).is_none();
                let acc_w = if accuracy_here {
                    score_width(c, 14.0, &self.accuracy_text()) + 14.0
                } else {
                    0.0
                };
                Place::new(on_screen(1.0, -18.0 - acc_w), on_screen(0.0, 50.0))
            }
            Part::Progress => Place::new(on_screen(0.0, 0.0), on_screen(0.0, 0.0)),
            // The skin's bar where osu! has it, in the top left at its own size (some are a frame
            // round the whole screen). Ours is under the score, accuracy and mods (the top-left
            // corner is the FPS overlay's).
            Part::Health if skin_health(c).is_some() => {
                Place::new(on_screen(0.0, 0.0), on_screen(0.0, 0.0))
            }
            Part::Health => Place::new(on_screen(1.0, -18.0), on_screen(0.0, HP_BAR_Y)),
            Part::Mods => Place::new(on_screen(1.0, -18.0), on_screen(0.0, 72.0)),
            Part::HitError => Place::new(on_screen(0.5, 0.0), on_screen(0.0, 408.0)),
            Part::OffsetHint => Place::new(on_screen(0.5, 0.0), on_screen(0.0, 450.0)),
            Part::Combo => Place::new(
                axis(Target::Drum, 0.5, 0.5, 0.0),
                axis(Target::Drum, 0.5, 0.5, 0.0),
            ),
            Part::Judgements => Place::new(
                axis(Target::HitPoint, 0.5, 0.5, 0.0),
                axis(Target::Lane, 0.0, 1.0, -8.0),
            ),
            Part::Ur => Place::new(
                axis(Target::Lane, 1.0, 1.0, -8.0),
                axis(Target::Lane, 1.0, 0.0, 8.0),
            ),
            Part::Counts => Place::new(
                axis(Target::Lane, 0.0, 0.0, 8.0),
                axis(Target::Lane, 1.0, 0.0, 8.0),
            ),
            Part::Scoreboard | Part::Spectators | Part::HitAnimation => {
                Place::new(on_screen(0.0, 12.0), on_screen(0.0, 276.0))
            }
        })
    }

    /// `boxes` at map time `t`, eased from where they were.
    pub(super) fn ease_field_boxes(&mut self, boxes: hud::FieldBoxes, t: f64) {
        let k = ease_step(t - self.view.field_at);
        let mix = |a: [f32; 4], b: [f32; 4]| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * k);
        self.view.field_boxes = Some(self.view.field_boxes.map_or(boxes, |o| hud::FieldBoxes {
            lane: mix(o.lane, boxes.lane),
            drum: mix(o.drum, boxes.drum),
            hit: mix(o.hit, boxes.hit),
        }));
        self.view.field_at = t;
    }

    /// The lane's top and bottom on the screen in the columns from `x0` to `x1`, if it's there.
    pub(super) fn lane_column(&self, x0: f32, x1: f32, c: &Canvas) -> Option<(f32, f32)> {
        let area = self.view.hud_area;
        let (k, ox, oy) = (area.h, area.x * c.width, area.y * VIRTUAL_HEIGHT);
        let half = self.view.lane_half * k;
        let (mut top, mut bottom) = (f32::MAX, f32::MIN);
        for &(x, y) in &self.view.lane_path {
            let (x, y) = (ox + x * k, oy + y * k);
            if x + half >= x0 && x - half <= x1 {
                (top, bottom) = (top.min(y - half), bottom.max(y + half));
            }
        }
        (top <= bottom).then_some((top.max(0.0), bottom.min(VIRTUAL_HEIGHT)))
    }

    /// While mods bend or move the lane: what's attached to it goes by the lane right there, and
    /// what that puts on top of another part gives way, towards the middle of the screen.
    pub(super) fn settle_parts(&self, c: &Canvas) {
        let mut settled = [None; Part::ALL.len()];
        let now = self.game.time;
        if !self.view.lane_path.is_empty() {
            let drawn = |p: Part| {
                !self.view.hidden.has(p)
                    && (self.view.hud.place(p).is_some()
                        || !matches!(
                            p,
                            Part::Combo | Part::Judgements | Part::Scoreboard | Part::Spectators
                        ))
            };
            let frames = self.frames(c);
            // Each with where it'd be, for what stands close together there on purpose.
            let mut boxes: Vec<(Part, hud::Placed, bool, hud::Placed)> = Vec::new();
            for part in Part::PLACED.into_iter().filter(|&p| drawn(p)) {
                let (place, _, mut b) = self.part_box(c, part);
                let home = b;
                let a = place.y;
                let column = (a.target == hud::Target::Lane)
                    .then(|| self.lane_column(b.x, b.x + b.w, c))
                    .flatten();
                if let Some((top, bottom)) = column {
                    let line = top + a.at * (bottom - top);
                    let y = line + a.off * frames.follow(&place) - a.pivot * b.h;
                    b.y = y.clamp(0.0, (VIRTUAL_HEIGHT - b.h).max(0.0));
                }
                boxes.push((part, b, a.target == hud::Target::Lane, home));
            }
            boxes.sort_by(|a, b| a.1.y.total_cmp(&b.1.y));
            let touching = |a: &hud::Placed, b: &hud::Placed| {
                a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
            };
            let middle = VIRTUAL_HEIGHT / 2.0;
            // The top half downwards, then the bottom half upwards.
            for i in 0..boxes.len() {
                if !boxes[i].2 || boxes[i].1.y + boxes[i].1.h / 2.0 >= middle {
                    continue;
                }
                for j in 0..i {
                    let other = boxes[j].1;
                    if touching(&boxes[i].1, &other) && !touching(&boxes[i].3, &boxes[j].3) {
                        boxes[i].1.y = other.y + other.h + 2.0;
                    }
                }
            }
            for i in (0..boxes.len()).rev() {
                if !boxes[i].2 || boxes[i].1.y + boxes[i].1.h / 2.0 < middle {
                    continue;
                }
                for j in (i + 1..boxes.len()).rev() {
                    let other = boxes[j].1;
                    if touching(&boxes[i].1, &other) && !touching(&boxes[i].3, &boxes[j].3) {
                        boxes[i].1.y = other.y - boxes[i].1.h - 2.0;
                    }
                }
            }
            let before = self.view.settled.get();
            for (part, b, ..) in boxes.into_iter().filter(|b| b.2) {
                let i = hud::Layout::index(part);
                let y = before[i].map_or(b.y, |(y, at)| y + (b.y - y) * ease_step(now - at));
                settled[i] = Some((y, now));
            }
        }
        self.view.settled.set(settled);
    }

    /// Where the parts can be attached to now.
    pub(super) fn frames(&self, c: &Canvas) -> hud::Frames {
        hud::Frames::new(c.width, self.view.hud_area)
            .with_field(self.view.hud_area, self.view.field_boxes)
    }

    /// How big a part at `place` is in its own units.
    pub(super) fn part_size(&self, c: &Canvas, part: Part, place: &hud::Place) -> [f32; 2] {
        match part {
            Part::Score => [score_width(c, 24.0, "00000000"), 28.0],
            Part::Accuracy => [score_width(c, 14.0, "100.00%"), 16.0],
            Part::Pp => [score_width(c, 14.0, "9999pp"), 16.0],
            // As long as what it's on.
            Part::Progress => {
                let f = self.frames(c);
                [f.rect(place.x.target)[2] / f.follow(place), 3.0]
            }
            Part::Health => {
                skin_health(c).map_or([(c.width * 0.25).min(180.0), 8.0], |[bg, _]| [bg[0], bg[1]])
            }
            Part::Mods => [Canvas::text_width(9.0, &self.mods_text()).max(40.0), 10.0],
            Part::HitError => [300.0, 44.0],
            Part::OffsetHint => [420.0, 10.0],
            Part::Combo => [56.0, 24.0],
            Part::Judgements => [80.0, 22.0],
            Part::Ur => [Canvas::text_width(UR_SIZE, "UR 999.99"), 12.0],
            Part::Counts => {
                let on = self.view.hud.counter;
                let n = [on.great, on.ok, on.miss].iter().filter(|&&c| c).count() as f32;
                let cells = n * count_cell(on.titles) + (n - 1.0).max(0.0) * COUNT_GAP;
                [cells.max(COUNT_GAP * 2.0), 10.0]
            }
            Part::Scoreboard => [300.0, 138.0],
            Part::Spectators => [140.0, 92.0],
            Part::HitAnimation => [0.0, 0.0],
        }
    }

    /// A part's place, size in its own units and box on the screen.
    pub fn part_box(&self, c: &Canvas, part: Part) -> (hud::Place, [f32; 2], hud::Placed) {
        let place = self.part_place(c, part);
        let size = self.part_size(c, part, &place);
        let frames = self.frames(c);
        let moved = self.view.hud.place(part).is_some();
        (place, size, frames.resolve(&place, size, moved))
    }

    /// Draws a part with `f` in its own box from (0, 0), which also gets the box's size and how
    /// text lines up in it (0 left, 1 right), and puts it in its place.
    pub(super) fn in_place(
        &self,
        c: &mut Canvas,
        part: Part,
        f: impl FnOnce(&mut Canvas, [f32; 2], f32),
    ) {
        let (place, size, b) = self.part_box(c, part);
        let y = self.settled_top(part).unwrap_or(b.y);
        let start = c.shapes.len();
        f(c, size, place.x.pivot);
        c.transform_since(start, [0.0, 0.0], b.k, b.x, y);
    }

    /// Puts what `f` draws from `from`, a part drawn somewhere of its own, in its place.
    pub fn placed_from(
        &self,
        c: &mut Canvas,
        part: Part,
        from: [f32; 2],
        f: impl FnOnce(&mut Canvas),
    ) {
        let (_, _, b) = self.part_box(c, part);
        let y = self.settled_top(part).unwrap_or(b.y);
        let start = c.shapes.len();
        f(c);
        c.transform_since(start, from, b.k, b.x - from[0], y - from[1]);
    }

    pub(super) fn settled_top(&self, part: Part) -> Option<f32> {
        self.view.settled.get()[hud::Layout::index(part)].map(|(y, _)| y)
    }

    pub(super) fn accuracy_text(&self) -> String {
        format!("{:.2}%", self.game.stats.accuracy() * 100.0)
    }

    pub(super) fn mods_text(&self) -> String {
        self.game
            .mods
            .iter()
            .map(|m| m.acronym())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Everything the player can place.
    pub fn draw_parts(&self, c: &mut Canvas) {
        self.settle_parts(c);
        let t = self.game.time;
        let shown = |p: Part| !self.view.hidden.has(p);
        // Over the lane unless moved: the playfield draws those itself.
        let off_the_lane = |p: Part| self.view.hud_sample || self.view.hud.place(p).is_some();
        let chosen = |p: Part| self.view.hud.colour(p);

        // Defense is a team's: one player's score says nothing, the team panel does.
        if shown(Part::Score) && self.game.defense.is_none() {
            let s = format!("{:08}", self.game.stats.score);
            self.in_place(c, Part::Score, |c, [w, _], align| {
                score_number(c, w, align, 24.0, (WHITE, chosen(Part::Score)), &s);
            });
        }
        if shown(Part::Accuracy) {
            let s = self.accuracy_text();
            self.in_place(c, Part::Accuracy, |c, [w, _], align| {
                score_number(c, w, align, 14.0, (0xc8c8d8, chosen(Part::Accuracy)), &s);
            });
        }
        if let Some(pp) = self
            .live_pp_value
            .filter(|_| shown(Part::Pp) && self.game.defense.is_none())
        {
            let s = format!("{pp:.0}pp");
            let pink = chosen(Part::Pp).unwrap_or(0xff8ad8);
            self.in_place(c, Part::Pp, |c, [w, _], align| {
                let nw = c.number_width(Font::Score, 14.0, &s);
                let x = (w - nw.unwrap_or_else(|| Canvas::text_width(14.0, &s))) * align;
                if nw.is_some() {
                    c.number(Font::Score, x, 0.0, 14.0, rgba(pink, 1.0), &s);
                } else {
                    c.text(x, 0.0, 14.0, rgba(pink, 1.0), &s);
                }
            });
        }
        if shown(Part::Progress) {
            let done = if self.view.hud_sample {
                0.4
            } else {
                self.game.progress() as f32
            };
            self.in_place(c, Part::Progress, |c, [w, h], _| {
                c.rect(0.0, 0.0, w * done, h, rgba(WHITE, 0.45));
            });
        }
        // HP: has to be at least half full at the end (the mark), like osu!taiko. Defense goes
        // by the team's lives instead (its panel).
        if self.view.show_hp && shown(Part::Health) && self.game.defense.is_none() {
            self.in_place(c, Part::Health, |c, [hw, _], _| self.draw_health(c, hw));
        }
        if !self.game.mods.is_empty() && shown(Part::Mods) {
            let label = self.mods_text();
            self.in_place(c, Part::Mods, |c, [w, _], align| {
                let x = (w - Canvas::text_width(9.0, &label)) * align;
                let col = chosen(Part::Mods).unwrap_or(0xc8c8d8);
                c.text(x, 0.0, 9.0, rgba(col, 0.9), &label);
            });
        }
        if shown(Part::HitError) {
            self.in_place(c, Part::HitError, |c, [w, h], _| {
                self.draw_hit_error(c, t, w / 2.0, h / 2.0);
            });
        }
        if shown(Part::OffsetHint) {
            let hint = if self.view.hud_sample {
                Some((
                    tr!(
                        "offset looks good: +1.2 +-3.4 ms (UR 95, 120 hits)",
                        "オフセットは良好: +1.2 ±3.4 ms (UR 95、120回)",
                        "смещение в порядке: +1.2 +-3.4 мс (UR 95, 120 ударов)"
                    )
                    .to_string(),
                    rgba(0x86E05A, 0.9),
                ))
            } else {
                self.offset_hint_text()
            };
            if let Some((text, colour)) = hint {
                self.in_place(c, Part::OffsetHint, |c, [w, _], _| {
                    let colour = chosen(Part::OffsetHint).map_or(colour, |col| rgba(col, 1.0));
                    c.text_centered(w / 2.0, 0.0, 8.0, colour, &text);
                });
            }
        }
        if shown(Part::Ur) {
            let s = format!("UR {:.2}", self.game.stats.unstable_rate());
            self.in_place(c, Part::Ur, |c, [w, _], align| {
                let x = (w - Canvas::text_width(UR_SIZE, &s)) * align;
                let col = chosen(Part::Ur).unwrap_or(0xc8c8d8);
                c.text(x, 0.0, UR_SIZE, rgba(col, 1.0), &s);
            });
        }
        if shown(Part::Counts) {
            let st = &self.game.stats;
            let picked = chosen(Part::Counts);
            let on = self.view.hud.counter;
            let titles = on.titles;
            let cells = [
                (on.great, "300", st.great, Judge::Great),
                (on.ok, "100", st.ok, Judge::Ok),
                (on.miss, "MISS", st.miss, Judge::Miss),
            ];
            self.in_place(c, Part::Counts, |c, _, _| {
                let shown = cells.into_iter().filter(|cell| cell.0);
                for (i, (_, label, n, j)) in shown.enumerate() {
                    let x = i as f32 * (count_cell(titles) + COUNT_GAP);
                    let col = picked.unwrap_or(j.color());
                    let n = n.to_string();
                    if titles {
                        c.text(x, 0.0, COUNT_SIZE, rgba(col, 1.0), label);
                        let at = x + Canvas::text_width(COUNT_SIZE, label) + COUNT_SPACE;
                        let count = rgba(picked.unwrap_or(WHITE), 1.0);
                        c.text(at, 0.0, COUNT_SIZE, count, &n);
                    } else {
                        // The colour is all that says which it is.
                        c.text(x, 0.0, COUNT_SIZE, rgba(col, 1.0), &n);
                    }
                }
            });
        }
        if shown(Part::Combo) && off_the_lane(Part::Combo) && self.game.stats.combo >= 2 {
            let s = self.game.stats.combo.to_string();
            self.in_place(c, Part::Combo, |c, [w, h], _| {
                let skin_size = if s.len() > 3 { 17.0 } else { 22.0 };
                match c.number_width(Font::Combo, skin_size, &s) {
                    Some(nw) => {
                        let y = (h - skin_size) / 2.0;
                        let x = (w - nw) / 2.0;
                        let tint = chosen(Part::Combo).unwrap_or(WHITE);
                        c.number(Font::Combo, x, y, skin_size, rgba(tint, 1.0), &s);
                    }
                    None => {
                        let size = if s.len() > 3 { 12.0 } else { 16.0 };
                        let y = (h - size) / 2.0;
                        // Dark on the drum, where the stand-in play shows it.
                        let on_drum = self.view.hud.place(Part::Combo).is_none();
                        let col = chosen(Part::Combo).unwrap_or(if on_drum {
                            0x1d1d26
                        } else {
                            0xe8e8f0
                        });
                        c.text_centered(w / 2.0, y, size, rgba(col, 1.0), &s);
                    }
                }
            });
        }
        if shown(Part::Judgements) && off_the_lane(Part::Judgements) {
            let judged = if self.view.hud_sample {
                Some((Judge::Great, 1.0))
            } else {
                self.game
                    .last_judge
                    .filter(|&(j, _)| splashes(j))
                    .and_then(|(j, at)| {
                        let age = (t - at) as f32;
                        (age < 450.0).then(|| (j, (1.0 - age / 450.0).min(1.0)))
                    })
            };
            if let Some((j, a)) = judged {
                self.in_place(c, Part::Judgements, |c, [w, h], _| {
                    let sprite = match j {
                        Judge::Great => Sprite::JudgeGreat,
                        Judge::Ok => Sprite::JudgeOk,
                        Judge::Miss => Sprite::JudgeMiss,
                    };
                    if let Some([sw, sh]) = c.sprite_size(sprite) {
                        // Fitted to the box, as tall as it can be.
                        let hh = h / 2.0;
                        let hw = hh * sw / sh.max(0.001);
                        let tint = chosen(Part::Judgements).unwrap_or(WHITE);
                        c.sprite_box(sprite, w / 2.0, hh, hw, hh, rgba(tint, a));
                    } else {
                        let col = chosen(Part::Judgements).unwrap_or(j.color());
                        c.text_centered(w / 2.0, 3.0, 15.0, rgba(col, a), j.label());
                    }
                });
            }
        }
    }

    /// Stand-ins for the parts only a match or friends watching show. They start out in the same
    /// spot, and never appear together in a play: their names are kept apart here.
    pub fn draw_stand_ins(&self, c: &mut Canvas) {
        for (part, y) in [(Part::Scoreboard, 124.0), (Part::Spectators, 6.0)] {
            if !self.view.hidden.has(part) {
                self.in_place(c, part, |c, [w, h], _| {
                    c.round_rect(
                        [0.0, 0.0],
                        [w, h],
                        6.0,
                        rgba(0x000000, 0.45),
                        rgba(0, 0.0),
                        0.0,
                    );
                    let label = Canvas::fit(8.0, w - 12.0, part.label());
                    c.text(6.0, y, 8.0, rgba(0xc8c8d8, 1.0), &label);
                });
            }
        }
    }

    /// The health bar in its own box, `hw` wide.
    pub(super) fn draw_health(&self, c: &mut Canvas, hw: f32) {
        let hp = self.game.hp as f32;
        if let Some([[bg_w, bg_h], [fill_w, fill_h]]) = skin_health(c) {
            // The fill on it 5, 16 px in, cut off at the HP.
            c.sprite_box(
                Sprite::HealthBg,
                bg_w / 2.0,
                bg_h / 2.0,
                bg_w / 2.0,
                bg_h / 2.0,
                rgba(WHITE, 1.0),
            );
            let (fx, fy) = (5.0 * HUD_PX, 16.0 * HUD_PX);
            let start = c.shapes.len();
            c.sprite_box(
                Sprite::HealthFill,
                fx + fill_w / 2.0,
                fy + fill_h / 2.0,
                fill_w / 2.0,
                fill_h / 2.0,
                rgba(WHITE, 1.0),
            );
            c.clip_since(start, fx, fy, fill_w * hp, fill_h);
            c.rect(
                fx + fill_w / 2.0 - 0.75,
                fy - 2.0,
                1.5,
                fill_h + 4.0,
                rgba(WHITE, 0.8),
            );
        } else {
            c.round_rect(
                [0.0, 0.0],
                [hw, 8.0],
                3.0,
                rgba(0x15151d, 0.85),
                rgba(0x3a3a4e, 1.0),
                1.0,
            );
            let fill = if hp >= 0.5 { 0xFFD34D } else { 0xEB452C };
            if hp > 0.0 {
                c.round_rect(
                    [1.0, 1.0],
                    [(hw - 2.0) * hp, 6.0],
                    2.5,
                    rgba(fill, 0.95),
                    rgba(0, 0.0),
                    0.0,
                );
            }
            c.rect(hw / 2.0 - 0.75, -2.0, 1.5, 12.0, rgba(WHITE, 0.8));
        }
    }

    /// The hit error meter around (`cx`, `my`).
    pub(super) fn draw_hit_error(&self, c: &mut Canvas, t: f64, cx: f32, my: f32) {
        let scale = 150.0 / self.game.windows.miss as f32;
        let wnd = |ms: f64| ms as f32 * scale;
        c.round_rect(
            [cx - wnd(self.game.windows.miss), my - 3.0],
            [wnd(self.game.windows.miss) * 2.0, 6.0],
            3.0,
            rgba(Judge::Miss.color(), 0.25),
            rgba(0, 0.0),
            0.0,
        );
        c.round_rect(
            [cx - wnd(self.game.windows.ok), my - 3.0],
            [wnd(self.game.windows.ok) * 2.0, 6.0],
            3.0,
            rgba(Judge::Ok.color(), 0.4),
            rgba(0, 0.0),
            0.0,
        );
        c.round_rect(
            [cx - wnd(self.game.windows.great), my - 3.0],
            [wnd(self.game.windows.great) * 2.0, 6.0],
            3.0,
            rgba(Judge::Great.color(), 0.6),
            rgba(0, 0.0),
            0.0,
        );
        for &(err, at, j) in &self.game.recent_errors {
            let age = (t - at) as f32;
            if age < 4000.0 {
                let a = 1.0 - age / 4000.0;
                c.rect(
                    cx + err as f32 * scale - 1.0,
                    my - 11.0,
                    2.0,
                    22.0,
                    rgba(j.color(), a),
                );
            }
        }
        if !self.game.recent_errors.is_empty() {
            let n = self.game.recent_errors.len().min(20);
            let mean = self
                .recent_errors
                .iter()
                .rev()
                .take(n)
                .map(|e| e.0)
                .sum::<f64>()
                / n as f64;
            c.circle(
                cx + mean as f32 * scale,
                my - 18.0,
                3.5,
                rgba(WHITE, 0.9),
                rgba(WHITE, 0.0),
                0.0,
            );
        }
        c.rect(cx - 1.0, my - 14.0, 2.0, 28.0, rgba(WHITE, 0.9));
    }

    /// The offset advice's line and colour, once it has something to say.
    pub(super) fn offset_hint_text(&self) -> Option<(String, [f32; 4])> {
        if !self.view.show_offset_hint {
            return None;
        }
        let h = self.game.offset_suggestion(self.game.offset)?;
        let e = &h.estimate;
        let (hits, needed) = (e.hits, h.needed);
        let building = if hits < needed {
            tr!("  confidence {hits}/{needed}", "  信頼度 {hits}/{needed}", "  надёжность {hits}/{needed}";)
        } else {
            String::new()
        };
        let ur = e.ur;
        let hits_ru = crate::lang::ru_plural(hits, ["удар", "удара", "ударов"]);
        let stats =
            tr!("UR {ur:.0}, {hits} hits", "UR {ur:.0}、{hits}回", "UR {ur:.0}, {hits} {hits_ru}";);
        let suggested = self.game.suggested_shown_offset(e.suggested_offset);
        let (error, margin) = (e.error_ms.abs(), e.margin_ms);
        if h.significant {
            let dir = if e.error_ms > 0.0 {
                tr!("late", "遅い", "поздно")
            } else {
                tr!("early", "早い", "рано")
            };
            let apply = self.view.offset_key.as_ref().map_or(String::new(), |k| tr!("  |  {k}: this map  Shift+{k}: all", "  |  {k}: この譜面  Shift+{k}: 全体", "  |  {k}: эта карта  Shift+{k}: все";));
            let text = tr!(
                "{error:.1} +-{margin:.1} ms {dir} ({stats})  |  suggested offset {suggested:+.0} ms{apply}{building}",
                "{error:.1} ±{margin:.1} ms {dir} ({stats})  |  提案オフセット {suggested:+.0} ms{apply}{building}",
                "{error:.1} +-{margin:.1} мс {dir} ({stats})  |  смещение {suggested:+.0} мс{apply}{building}";
            );
            Some((text, rgba(0xFFD34D, 0.95)))
        } else {
            let error = e.error_ms;
            let text = tr!(
                "offset looks good: {error:+.1} +-{margin:.1} ms ({stats}){building}",
                "オフセットは良好: {error:+.1} ±{margin:.1} ms ({stats}){building}",
                "смещение в порядке: {error:+.1} +-{margin:.1} мс ({stats}){building}";
            );
            Some((text, rgba(0x86E05A, 0.9)))
        }
    }

    /// A made-up play for the interface editor to show the parts with.
    pub fn hud_sample() -> Play {
        let track = Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        });
        let map = beatmap::calibration_map(500.0, 0, 64);
        let mut g = Game::with_mods(map, track, None, Mods::parse("HD"));
        g.stats.score = 1_234_560;
        (g.stats.great, g.stats.ok, g.stats.miss) = (812, 37, 4);
        g.stats.combo = 321;
        g.stats.errors = (0..20).map(|i| (i % 7) as f64 * 5.0 - 15.0).collect();
        g.live_pp_value = Some(312.0);
        g.hp = 0.8;
        g.recent_errors = g
            .stats
            .errors
            .iter()
            .map(|&e| (e, g.time, Judge::Great))
            .collect();
        let mut p = Play::new(g);
        p.view.hud_sample = true;
        p.view.show_hp = true;
        p
    }
}
/// Overtime's animatronic bear lunging at the screen, `p` (0-1) of the way through, shaking
/// with map time `t`. Drawn from shapes, centred on a canvas `c.width` wide.
/// The HP bar's top.
pub(super) const HP_BAR_Y: f32 = 88.0;
/// The UR counter's text size.
pub(super) const UR_SIZE: f32 = 11.0;
/// The hit counter's text size and the room between its cells.
pub(super) const COUNT_SIZE: f32 = 9.0;
pub(super) const COUNT_GAP: f32 = 10.0;
pub(super) const COUNT_SPACE: f32 = 4.0;
/// One cell of the hit counter: its count, after its title if it has one.
pub(super) fn count_cell(titles: bool) -> f32 {
    let title = Canvas::text_width(COUNT_SIZE, "MISS") + COUNT_SPACE;
    Canvas::text_width(COUNT_SIZE, "9999") + if titles { title } else { 0.0 }
}
/// How wide `s` comes out in the score's font.
pub(super) fn score_width(c: &Canvas, size: f32, s: &str) -> f32 {
    c.number_width(Font::Score, size, s)
        .unwrap_or_else(|| Canvas::text_width(size, s))
}
/// `s` in the skin's score font if it has one, lined up by `align` in a box `w` wide.
/// `chosen`: the player's colour, which tints the skin's digits too.
pub(super) fn score_number(
    c: &mut Canvas,
    w: f32,
    align: f32,
    size: f32,
    (col, chosen): (u32, Option<u32>),
    s: &str,
) {
    let x = (w - score_width(c, size, s)) * align;
    if !c.number(
        Font::Score,
        x,
        0.0,
        size,
        rgba(chosen.unwrap_or(WHITE), 1.0),
        s,
    ) {
        c.text(x, 0.0, size, rgba(chosen.unwrap_or(col), 1.0), s);
    }
}
/// The sizes of the skin's health bar and its fill, if it has them.
pub(super) fn skin_health(c: &Canvas) -> Option<[[f32; 2]; 2]> {
    let k = HUD_PX * 128.0;
    let bg = c.sprite_size(Sprite::HealthBg)?;
    let fill = c.sprite_size(Sprite::HealthFill)?;
    Some([[bg[0] * k, bg[1] * k], [fill[0] * k, fill[1] * k]])
}
/// How long (ms) parts take to catch up with a lane that's moved: bits of a bending lane come on
/// and off the screen all at once, which they shouldn't jump with.
pub(super) const HUD_EASE_MS: f64 = 140.0;
/// How far along to where it's going something eased gets in `dt` ms. A step back in time or a long
/// way forward is a seek: it goes straight there.
pub(super) fn ease_step(dt: f64) -> f32 {
    if dt <= 0.0 || dt > 1000.0 {
        1.0
    } else {
        (1.0 - (-dt / HUD_EASE_MS).exp()) as f32
    }
}
/// Units per SD px of osu!'s interface images: its screen is 768 px tall.
pub(super) const HUD_PX: f32 = VIRTUAL_HEIGHT / 768.0;

/// Pause menu item `i`'s name.
pub(super) fn pause_item(i: usize) -> &'static str {
    match i {
        0 => tr!("Continue", "続ける", "Продолжить"),
        1 => tr!("Retry", "やり直す", "Заново"),
        2 => tr!(
            "Playfield size",
            "プレイフィールドのサイズ",
            "Размер игрового поля"
        ),
        3 => tr!("Settings", "設定", "Настройки"),
        _ => tr!("Quit to song select", "選曲に戻る", "Выйти к выбору песни"),
    }
}
