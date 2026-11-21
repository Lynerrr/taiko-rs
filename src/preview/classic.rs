//! Song select's details panel as 1.0 drew it, for its look under Settings > System >
//! Legacy menu (`Config::legacy_menu`).

use super::*;

impl Preview {
    /// `Preview::draw_panel` as it was in 1.0.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_panel_classic(
        &mut self,
        c: &mut Canvas,
        library: &[Entry],
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        scroll_speed: f64,
        stars: Option<f32>,
        button: Option<(&str, &str)>,
        status: Option<(&str, u32)>,
        liked: Option<bool>,
    ) -> Option<(std::ops::Range<usize>, crate::layout::Area)> {
        // The preview shows ~640 units of lane (several notes ahead), so its height follows the
        // panel width; the panel shrinks to fit instead of leaving empty space.
        // The lane alone fills the box, laid out as on a 4:3 screen (640 units wide): bigger notes,
        // fewer of them, whatever the panel's shape.
        let (band_top, band_h) = game::lane_only();
        const GRAPH_H: f32 = 40.0;
        // Under the preview box: the timeline and its button.
        let after_box = 8.0 + GRAPH_H + 44.0;
        // The preview's height per unit of its size. Smaller, it shows more of the lane ahead in
        // a shorter box: as small as dragged, and smaller still where the panel would otherwise
        // be taller than `h` (the details above it being `head` tall).
        let (k_full, lane_w) = lane_fit(
            w,
            crate::ui::Select::panel_width(c.width, crate::ui::LIST_SHARE.2),
        );
        let unit = band_h * k_full;
        self.size_unit = unit;
        let wanted = if self.size > 0.0 {
            self.size.clamp(PREVIEW_SIZE.0, PREVIEW_SIZE.1)
        } else {
            PREVIEW_SIZE.1
        };
        let fitted = |head: f32| {
            ((h - head - after_box - 12.0) / unit).clamp(PREVIEW_SIZE.0, PREVIEW_SIZE.1)
        };
        self.graph_rect = None;
        self.play_rect = None;
        self.box_rect = None;
        let Some(i) = self.seen.filter(|&i| i < library.len()) else {
            (self.size_cap, self.size_shown) = (fitted(110.0), wanted.min(fitted(110.0)));
            let below = unit * self.size_shown + 12.0 + after_box;
            self.panel_bottom = y + h.min(110.0 + below);
            c.round_rect(
                [x, y],
                [w, h.min(110.0 + below)],
                8.0,
                rgba(0x15151d, 0.82),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
            return None;
        };
        let e = &library[i];
        // Map details and the timeline appear with the map; the autoplay runs on the map's music,
        // so it joins them when that plays.
        let ready = self.map.as_ref().is_some_and(|(mi, ..)| *mi == i);
        let playing = self.ready(library, i) || (self.ready_for == Some(i) && ready);
        if playing {
            self.ready_for = Some(i);
        }
        if ready && self.shown_for != Some(i) {
            self.shown_for = Some(i);
            self.ready_at = Some(Instant::now());
        }
        let fade = self
            .ready_at
            .filter(|_| ready)
            .map(|t| crate::render::progress(t, 180.0))
            .unwrap_or(0.0);
        let loaded = self.map.as_ref().filter(|(mi, _, _, _)| *mi == i && ready);

        // Text sits on a 14 unit inset; long names wrap onto a second line before they are cut.
        let (lx, iw) = (x + 14.0, w - 28.0);
        let (th_title, th_body, th_small) = (
            Canvas::text_height(12.0),
            Canvas::text_height(9.0),
            Canvas::text_height(8.0),
        );
        let title = wrap(12.0, iw - self.title_room, &e.meta.title, 2);
        let artist = wrap(9.0, iw, &e.meta.artist, 2);
        // The star pill is about this wide; the difficulty name wraps beside it.
        let pill_w = Canvas::text_width(9.0, "0.00") + 9.0 * 1.6 + 6.0;
        let version = wrap(9.0, iw - pill_w - 8.0, &format!("[{}]", e.meta.version), 2);
        // Stats as label + value, flowed left to right; the first group is the map's, the second its notes.
        let mut stats: [Vec<StatCell>; 4] = [
            // As it plays: EZ / HR / Precise, and the speed, which shrinks or grows the windows.
            vec![(
                "OD",
                format!(
                    "{:.1}",
                    crate::beatmap::od_at_rate(
                        self.mods.od_with(e.meta.od, crate::od_override::get()),
                        self.requested_rate.0,
                    )
                ),
                1.0,
            )],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ];
        // What a full combo would give: SS, 99%, 98%.
        if let Some(pp) = self.potential {
            for (label, pp) in ["SS", "99%", "98%"].into_iter().zip(pp) {
                stats[2].push((label, format!("{:.0}pp", pp), 1.0));
            }
        }
        if let Some((_, _, d, _)) = loaded {
            // At the speed played (DT, HT, the rate): as fast and as long as it'll be.
            let speed = self.requested_rate.0.max(0.01);
            let bpm = if (d.bpm.1 - d.bpm.0).abs() < 0.5 {
                format!("{:.0}", d.bpm.0 * speed)
            } else {
                format!("{:.0}-{:.0}", d.bpm.0 * speed, d.bpm.1 * speed)
            };
            stats[0].push(("BPM", bpm, fade));
            stats[0].push(("Length", fmt_time(d.length_ms / speed), fade));
            if self.map_offset != 0.0 {
                stats[0].push(("Offset", format!("{:+.0} ms", self.map_offset), fade));
            }
            stats[1].push(("Notes", (d.dons + d.kats).to_string(), fade));
            stats[1].push(("Don", d.dons.to_string(), fade));
            stats[1].push(("Kat", d.kats.to_string(), fade));
            if d.rolls > 0 {
                stats[1].push(("Rolls", d.rolls.to_string(), fade));
            }
            if d.swells > 0 {
                stats[1].push(("Swells", d.swells.to_string(), fade));
            }
            // What a TJA course uses that isn't supported yet: a row of its own.
            if let Some((_, what)) = self.left_out.as_ref().filter(|(li, _)| *li == i) {
                let label = left_out_label();
                let room = iw - Canvas::text_width(8.0, &format!("{label} "));
                stats[3].push((label, Canvas::fit(8.0, room, what), fade));
            }
        }
        let cell_w =
            |(label, value, _): &StatCell| Canvas::text_width(8.0, &format!("{label} {value}"));
        let rows: Vec<Vec<StatRow<'_>>> = stats
            .iter()
            // The first three always hold a row; what a TJA course leaves out only when it has some.
            .take(if stats[3].is_empty() { 3 } else { 4 })
            .map(|group| {
                let mut rows: Vec<StatRow<'_>> = vec![Vec::new()];
                let mut used = 0.0;
                for cell in group {
                    let cw = cell_w(cell);
                    if used > 0.0 && used + STAT_GAP + cw > iw {
                        rows.push(Vec::new());
                        used = 0.0;
                    }
                    used += if used > 0.0 { STAT_GAP } else { 0.0 } + cw;
                    rows.last_mut().unwrap().push(cell);
                }
                rows
            })
            .collect();
        // Three rows are always kept (the pp one too), so the panel doesn't jump when the details
        // arrive.
        let stat_rows = rows.iter().map(Vec::len).sum::<usize>().max(3);
        let (title_step, body_step, row_h) = (th_title + 4.0, th_body + 3.0, th_small + 4.0);
        let head = 12.0
            + title.len() as f32 * title_step
            + 3.0
            + artist.len() as f32 * body_step
            + 4.0
            + 13.0
            + 6.0
            + (version.len() as f32 * body_step).max(15.0)
            + 6.0
            + stat_rows as f32 * row_h
            + 6.0;
        (self.size_cap, self.size_shown) = (fitted(head), wanted.min(fitted(head)));
        // The lane fills the panel's width (at the size chosen), however little room the details
        // leave: the panel grows down, into the leaderboard's room, rather than the lane shrink.
        let k = k_full * wanted;
        // A .tko map that moves things off the lane gets a taller box, as far as the room left
        // under the panel goes (both ends drawn in alike when it doesn't all fit).
        let [top, bottom] = self.shown_reach();
        let extra = (bottom - top - band_h) * k;
        let room = (h - head - band_h * k - after_box).max(0.0);
        let fit = if extra > room { room / extra } else { 1.0 };
        let band_top = band_top - (band_top - top) * fit;
        let band_h = band_h + (bottom - top - band_h) * fit;
        let preview_h = band_h * k;
        let h = head + preview_h + after_box;
        self.panel_bottom = y + h;
        c.round_rect(
            [x, y],
            [w, h],
            8.0,
            rgba(0x15151d, 0.82),
            rgba(0x2a2a38, 1.0),
            1.0,
        );

        let mut cy = y + 12.0;
        for line in &title {
            c.text(lx, cy, 12.0, rgba(0xffffff, 1.0), line);
            cy += title_step;
        }
        cy += 3.0;
        for line in &artist {
            c.text(lx, cy, 9.0, rgba(0xc8c8d8, 1.0), line);
            cy += body_step;
        }
        cy += 4.0;
        // The EPIC mark at the very right of the mapper's line, bumping on the beat. It stands in
        // the status's place, as on the website: the map's osu! status isn't shown with it.
        self.epic_at = None;
        let epic_w = if self.epic {
            let pw = crate::epic::pill_width();
            let ex = x + w - 14.0 - pw;
            crate::epic::pill(c, ex, cy, 1.0, crate::style::pulse());
            self.epic_at = Some([ex, cy]);
            pw + 8.0
        } else {
            0.0
        };
        // Otherwise the status on osu!.
        let status_w = epic_w
            + match status.filter(|s| !s.0.is_empty() && !self.epic) {
                Some((label, col)) => {
                    let lw = Canvas::text_width(7.0, label) + 12.0;
                    let sx = x + w - 14.0 - epic_w - lw;
                    c.round_rect(
                        [sx, cy],
                        [lw, 13.0],
                        6.5,
                        rgba(col, 0.15),
                        rgba(col, 0.9),
                        1.0,
                    );
                    c.text_centered(sx + lw / 2.0, cy + 3.0, 7.0, rgba(col, 1.0), label);
                    lw + 8.0
                }
                None => 0.0,
            };
        // The heart, left of the status: whether the song's liked, and a click changes it.
        let heart_w = match liked {
            Some(liked) => {
                const HEART_W: f32 = 16.0;
                let hx = x + w - 14.0 - status_w - HEART_W;
                let hover = c.hovered(hx, cy, HEART_W, 13.0);
                let colour = match (liked, hover) {
                    (true, _) => 0xff5fa2,
                    (false, true) => 0xc8c8d8,
                    (false, false) => 0x5a5a70,
                };
                crate::player::draw_heart(c, hx + 3.5, cy + 2.5, 1.0, colour);
                c.hit(hx, cy, HEART_W, 13.0, Hit::SelectLike);
                HEART_W + 6.0
            }
            None => 0.0,
        };
        let mapper = Canvas::fit(
            8.0,
            iw - status_w - heart_w,
            &format!("mapped by {}", e.meta.creator),
        );
        c.text(
            lx,
            cy + (13.0 - th_small) / 2.0,
            8.0,
            rgba(0x8a8aa0, 1.0),
            &mapper,
        );
        cy += 13.0 + 6.0;
        crate::style::classic::map_pill(c, x + w - 14.0, cy + 3.0, 9.0, e.meta.level, stars);
        for line in &version {
            c.text(lx, cy + 3.0, 9.0, rgba(accent(), 1.0), line);
            cy += body_step;
        }
        cy += (15.0 - version.len() as f32 * body_step).max(0.0) + 6.0;
        // Muted labels, bright values.
        // The OD under the pointer: where its hit windows go, once all else is drawn.
        let mut od_hovered = None;
        for group in &rows {
            for row in group {
                let mut sx = lx;
                for cell @ (label, value, alpha) in row.iter().copied() {
                    if *label == "OD" && c.hovered(sx, cy - 2.0, cell_w(cell), row_h) {
                        od_hovered = Some((sx, cy + row_h));
                    }
                    sx += c.text(sx, cy, 8.0, rgba(0x8a8aa0, *alpha), &format!("{label} "));
                    sx += c.text(sx, cy, 8.0, rgba(0xe8e8f4, *alpha), value) + STAT_GAP;
                }
                cy += row_h;
            }
        }
        cy = cy.max(y + 12.0)
            + (stat_rows.saturating_sub(rows.iter().map(Vec::len).sum()) as f32) * row_h
            + 6.0;

        // Autoplay preview: just the lane band of the playfield, fitted to the box.
        // As wide as the lane at that scale (all of the panel at full size), centred.
        let bw = lane_w * k;
        let (bx, by, bh) = (x + 10.0 + (w - 20.0 - bw) / 2.0, cy, preview_h);
        c.round_rect(
            [bx, by],
            [bw, bh],
            6.0,
            rgba(0x0c0c12, 1.0),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        self.box_rect = Some([bx, by, bw, bh]);
        self.drawn_scale = VIRTUAL_HEIGHT / c.height.max(1.0);
        c.hit(bx, by, bw, bh, Hit::Preview);
        let loaded = loaded.is_some();
        if loaded {
            self.draw_timeline_classic(c, e, [bx, by + bh + 8.0, bw, GRAPH_H], button);
            // Seeking goes by the music: until this map's plays, a click would move the last one.
            if !playing {
                self.graph_rect = None;
                self.play_rect = None;
            }
            if fade < 1.0 {
                c.rect(
                    bx - 1.0,
                    by + bh + 4.0,
                    bw + 2.0,
                    GRAPH_H + 40.0,
                    rgba(0x15151d, 1.0 - fade),
                );
            }
        } else {
            self.graph_rect = None;
            self.play_rect = None;
        }
        // What's over the panel goes last, on the way out with a lane or without.
        let tip = od_hovered.map(|at| {
            let windows = crate::game::Windows::for_map(
                &e.meta,
                self.mods,
                crate::od_override::get(),
                crate::game::WindowRule::Osu,
            );
            (at, windows.at_rate(self.requested_rate.0))
        });
        let Some(game) = self.game.as_mut().filter(|_| loaded && playing) else {
            if let Some((at, windows)) = tip {
                draw_hit_windows(c, at, windows);
            }
            return None;
        };
        let full_w = c.width;
        let start = c.shapes.len();
        c.width = lane_w;
        game.view.kiai_effects = self.kiai_effects;
        game.draw_playfield(c, scroll_speed);
        let oy = by - band_top * k;
        c.transform_since(start, [0.0, 0.0], k, bx, oy);
        c.width = full_w;
        if button.is_some() {
            // Song select only (download previews have no fullscreen view).
            c.text_right(
                bx + bw - 6.0,
                by + bh - 10.0,
                6.0,
                rgba(0x6f6f86, 1.0),
                if crate::mobile::active() {
                    "tap: fullscreen"
                } else {
                    "click: fullscreen (Ctrl+Enter)"
                },
            );
        }
        if fade < 1.0 {
            c.round_rect(
                [bx, by],
                [bw, bh],
                6.0,
                rgba(0x0c0c12, 1.0 - fade),
                rgba(0, 0.0),
                0.0,
            );
        }
        let clip = crate::layout::Area {
            x: bx / full_w,
            y: by / VIRTUAL_HEIGHT,
            w: bw / full_w,
            h: bh / VIRTUAL_HEIGHT,
        };
        // Only the lane is clipped to the box.
        let end = c.shapes.len();
        if let Some((at, windows)) = tip {
            draw_hit_windows(c, at, windows);
        }
        Some((start..end, clip))
    }

    fn draw_timeline_classic(
        &mut self,
        c: &mut Canvas,
        e: &Entry,
        rect: [f32; 4],
        button: Option<(&str, &str)>,
    ) {
        self.graph_rect = None;
        self.play_rect = None;
        let loaded = self.map.as_ref().filter(|(mi, ..)| Some(*mi) == self.seen);
        let (Some((_, _, _, graph)), Some(game)) = (loaded, self.game.as_ref()) else {
            return;
        };
        let marker = self.marker(e);
        // The playhead goes where it was sent at once, not when the music gets there.
        self.timeline = draw_graph_classic(
            c,
            graph,
            rect,
            self.pending_seek.unwrap_or(game.time()),
            e.meta.preview_time,
            marker,
        );
        self.graph_rect = Some(rect);
        self.drawn_in(c);
        self.play_rect = draw_play_button_classic(
            c,
            rect[0] + rect[2] / 2.0,
            rect[1] + rect[3] + 17.0,
            marker,
            button,
        );
        // The same slack as `time_at`.
        c.hit(
            rect[0] - 2.0,
            rect[1] - 2.0,
            rect[2] + 4.0,
            rect[3] + 4.0,
            Hit::Preview,
        );
        if let Some([x, y, w, h]) = self.play_rect {
            c.hit(x, y, w, h, Hit::Preview);
        }
    }
}

/// `draw_graph` as it was in 1.0.
fn draw_graph_classic(
    c: &mut Canvas,
    g: &Graph,
    [x, y, w, h]: [f32; 4],
    now: f64,
    preview_at: f64,
    marker: Option<f64>,
) -> (f64, f64) {
    let (t0, t1) = (g.start.min(0.0).max(g.start - 2000.0), g.end);
    let span = (t1 - t0).max(1.0);
    let x_of = |t: f64| x + (((t - t0) / span).clamp(0.0, 1.0) as f32) * w;
    c.round_rect(
        [x, y],
        [w, h],
        5.0,
        rgba(0x0c0c12, 1.0),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    for &(s, e) in &g.kiai {
        c.rect(
            x_of(s),
            y + 1.0,
            (x_of(e) - x_of(s)).max(1.0),
            h - 2.0,
            rgba(0xff8a3d, 0.12),
        );
    }
    // Bars cover the strain sections; the axis starts a little earlier for the intro.
    let (bx0, bx1) = (
        x_of(g.peaks_start),
        x_of(g.peaks_start + g.peaks.len() as f64 * g.section),
    );
    let n = (((bx1 - bx0) / 2.5) as usize).max(1);
    let bar_w = (bx1 - bx0) / n as f32;
    for (i, v) in g.bins(n).into_iter().enumerate() {
        let bh = 2.0 + v * (h - 8.0);
        let col = strain_colour(v);
        let played = (bx0 + (i as f32 + 0.5) * bar_w) <= x_of(now);
        c.rect(
            bx0 + i as f32 * bar_w,
            y + h - 3.0 - bh,
            (bar_w - 0.5).max(0.5),
            bh,
            rgba(col, if played { 1.0 } else { 0.55 }),
        );
    }
    if preview_at >= 0.0 {
        let px = x_of(preview_at);
        c.rect(px - 0.5, y + 1.0, 1.0, 5.0, rgba(0x86E05A, 1.0));
    }
    if let Some(m) = marker {
        // The picked start point: a blue line with a little flag.
        let mx = x_of(m);
        c.rect(mx - 0.75, y - 3.0, 1.5, h + 6.0, rgba(MARKER, 1.0));
        c.rect(mx + 0.75, y - 3.0, 6.0, 4.5, rgba(MARKER, 1.0));
    }
    let hx = x_of(now);
    c.rect(hx - 0.75, y - 2.0, 1.5, h + 4.0, rgba(0xffffff, 0.95));
    c.text(
        x,
        y + h + 5.0,
        7.0,
        rgba(0xc8c8d8, 1.0),
        &format!("{} / {}", fmt_time(now - t0.min(0.0)), fmt_time(t1)),
    );
    let peak = format!(
        "peak {:.1} notes/s at {}",
        g.peak_nps,
        fmt_time(g.peak_nps_at)
    );
    c.text_right(x + w, y + h + 5.0, 7.0, rgba(0x8a8aa0, 1.0), &peak);
    (t0, t1)
}

fn draw_play_button_classic(
    c: &mut Canvas,
    cx: f32,
    y: f32,
    marker: Option<f64>,
    button: Option<(&str, &str)>,
) -> Option<[f32; 4]> {
    let (Some(t), Some((verb, hint))) = (marker, button) else {
        let text = if crate::mobile::active() {
            "tap or drag to scroll"
        } else if button.is_some() {
            "click / drag / wheel to scroll - play from where you stop"
        } else {
            "click / drag / wheel to scroll"
        };
        c.text_centered(cx, y + 4.0, 6.0, rgba(0x6f6f86, 1.0), text);
        return None;
    };
    let label = format!("{verb} {}", fmt_time(t));
    let (lw, hw) = (
        Canvas::text_width(8.0, &label),
        Canvas::text_width(6.0, hint),
    );
    let bw = 14.0 + 10.0 + lw + 10.0 + hw + 10.0;
    let (bx, bh) = (cx - bw / 2.0, 17.0);
    c.round_rect(
        [bx, y],
        [bw, bh],
        5.0,
        rgba(0x173049, 1.0),
        rgba(MARKER, 1.0),
        1.0,
    );
    // A play triangle out of thin slices (the font is plain ASCII).
    for i in 0..7 {
        let f = i as f32;
        c.rect(
            bx + 9.0 + f,
            y + 4.5 + f * 0.5,
            1.0,
            8.0 - f,
            rgba(0xffffff, 1.0),
        );
    }
    c.text(bx + 24.0, y + 5.0, 8.0, rgba(0xffffff, 1.0), &label);
    c.text(
        bx + 24.0 + lw + 10.0,
        y + 6.0,
        6.0,
        rgba(0x9ac8f0, 1.0),
        hint,
    );
    Some([bx, y, bw, bh])
}
