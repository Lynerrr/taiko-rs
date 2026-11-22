//! Song select's list as 1.0 drew it: rows rather than cards, for its look under
//! Settings > System > Legacy menu (`Config::legacy_menu`). The list's logic is the same
//! `Select`'s; only the drawing is 1.0's.

use super::*;

impl Select {
    /// `Select::draw` as it was in 1.0.
    pub(crate) fn draw_classic(
        &mut self,
        c: &mut Canvas,
        library: &[Entry],
        dt_ms: f64,
        status: &str,
        stars: &dyn Fn(usize) -> Option<f32>,
        grade: &dyn Fn(&Entry) -> Option<&'static str>,
    ) {
        self.open_cursor_section();
        // In the screen's units, as ever.
        self.height = c.height;
        let w = c.width;
        let mode = if self.show_converts {
            "taiko + converts"
        } else {
            "taiko only"
        };
        let groups = &self.groups;
        let summary = format!(
            "{} songs, {} maps  |  {mode}{}",
            self.song_count,
            self.visible.len(),
            self.rate_label
        );
        let lw = Self::list_width(w, self.list_share);
        let legacy = self.legacy;
        // The row with the search: under the tabs, or at the top beside the way back.
        let row = if legacy { 54.0 } else { 20.0 };
        let mut left = 24.0;
        if legacy {
            crate::settings::tab_bar(c, 0, &summary);
        } else {
            let back = if self.to_menu {
                tr!("< MENU", "< メニュー", "< МЕНЮ")
            } else {
                tr!("< BACK", "< 戻る", "< НАЗАД")
            };
            if self.covered {
                left += Canvas::text_width(8.0, back) + 26.0;
            } else {
                left += crate::settings::back_button(c, row + 2.0, back, Hit::ToMenu) + 6.0;
            }
            // Over the panel beside the list, where there is one.
            let (x, right) = (24.0 + lw + 12.0, w - 24.0);
            if right - x > 60.0 {
                c.text_right(
                    right,
                    row + 8.0,
                    7.0,
                    rgba(0x9a9ab0, 1.0),
                    &Canvas::fit(7.0, right - x, &summary),
                );
            }
        }
        // Mods, sort and collection buttons share the search row, right to left. On a narrow
        // screen the widest of them switches to a shorter label until the search box gets at
        // least SEARCH_MIN; past that the widest give up room (their labels get cut).
        const SEARCH_MIN: f32 = 140.0;
        let mods = if self.mods_label.is_empty() {
            vec!["Mods".to_string()]
        } else {
            vec![
                format!("Mods: {}", self.mods_label),
                format!("Mods ({})", self.mods_label.len() / 2),
            ]
        };
        let sort = self.sort.name();
        let then = match self.tabs.order {
            Order::Sort => String::new(),
            o => format!(", {}", o.label()),
        };
        let filters = self.tabs.active(self.shown_source()).tokens().len();
        let more = if filters > 0 {
            format!(" +{filters}")
        } else {
            String::new()
        };
        let mut buttons = vec![(
            vec![format!("Sort: {sort}{then}{more}"), format!("{sort}{more}")],
            Hit::SortList,
            filters > 0 || self.lens_ui.is_some(),
        )];
        if legacy {
            buttons.insert(
                0,
                (mods.clone(), Hit::ModsButton, !self.mods_label.is_empty()),
            );
        }
        if !self.collections.is_empty() {
            let name = self
                .collection
                .and_then(|c| self.collections.get(c))
                .map_or("all", |c| c.name.as_str());
            buttons.push((
                vec![format!("Collection: {name}"), name.to_string()],
                Hit::CollectionPick,
                self.collection.is_some(),
            ));
        }
        let width = |label: &str| Canvas::text_width(7.0, label) + 20.0;
        let budget = lw - (left - 24.0) - SEARCH_MIN - 6.0 * buttons.len() as f32;
        let mut pick = vec![0; buttons.len()];
        loop {
            let widths: Vec<f32> = buttons
                .iter()
                .zip(&pick)
                .map(|((labels, ..), &k)| width(&labels[k]))
                .collect();
            if widths.iter().sum::<f32>() <= budget {
                break;
            }
            let shorter = (0..buttons.len())
                .filter(|&i| pick[i] + 1 < buttons[i].0.len())
                .max_by(|&a, &b| widths[a].total_cmp(&widths[b]));
            let Some(i) = shorter else { break };
            pick[i] += 1;
        }
        let labels: Vec<&str> = buttons
            .iter()
            .zip(&pick)
            .map(|((labels, ..), &k)| labels[k].as_str())
            .collect();
        let natural: Vec<f32> = labels.iter().map(|l| width(l)).collect();
        let cap = shared_cap(&natural, budget);
        let mut bx = 24.0 + lw;
        for ((label, (_, hit, accent)), nw) in labels.iter().zip(&buttons).zip(natural) {
            let bw = nw.min(cap);
            bx -= bw;
            c.button(bx, row + 2.0, bw, 20.0, label, 7.0, *hit, *accent);
            bx -= 6.0;
        }
        let search_w = bx - left;
        let edge = if self.searching {
            rgba(crate::render::accent(), 1.0)
        } else {
            rgba(0x3a3a4e, 1.0)
        };
        c.round_rect(
            [left, row],
            [search_w, 24.0],
            6.0,
            rgba(0x23232f, 0.9),
            edge,
            1.0,
        );
        // The filters on, as tokens at the start of the box: the text goes after them.
        let token_room = search_w * 0.6;
        let mut tokens_w = 0.0;
        for (_, text, _) in self.tabs.active(self.shown_source()).tokens() {
            let tw = Canvas::text_width(7.0, &text) + 18.0;
            if tokens_w + tw > token_room {
                break;
            }
            tokens_w += tw + 4.0;
        }
        let text_x = left + 10.0 + tokens_w;
        let room = search_w - 20.0 - tokens_w;
        let caret = "";
        let (search, search_col) = if self.filter.is_empty() {
            let (long, short) = if self.searching {
                ("type to search...", "search...")
            } else {
                ("click to search (ctrl+f)...", "search...")
            };
            let hint = if Canvas::text_width(10.0, long) <= room {
                long
            } else {
                short
            };
            (Canvas::fit(10.0, room, &format!("{hint}{caret}")), 0x6f6f86)
        } else {
            let full = format!("search: {}{caret}", self.filter);
            let text = if Canvas::text_width(10.0, &full) <= room {
                full
            } else {
                Canvas::fit_end(10.0, room, &format!("{}{caret}", self.filter))
            };
            (text, 0xffffff)
        };
        if self.searching {
            c.input(
                crate::text_input::Field::SongSearch,
                [text_x, row + 7.0, room, 12.0],
                10.0,
                &self.filter,
                "type to search...",
                rgba(0xffffff, 1.0),
                false,
                true,
            );
        } else {
            c.text(text_x, row + 7.0, 10.0, rgba(search_col, 1.0), &search);
        }
        c.hit(left, row, search_w, 24.0, Hit::TextField);
        self.draw_tokens(c, left + 6.0, row + 4.0, token_room);

        if let Some(n) = self.scanning {
            c.text_centered(
                24.0 + lw / 2.0,
                200.0,
                12.0,
                rgba(0xc8c8d8, 1.0),
                "Scanning your Songs folder...",
            );
            c.text_centered(
                24.0 + lw / 2.0,
                222.0,
                10.0,
                rgba(0x8a8aa0, 1.0),
                &format!("{n} files read"),
            );
        } else if library.is_empty() && !cfg!(target_os = "android") {
            // Most likely osu! lives somewhere it wasn't looked for.
            let cx = 24.0 + lw / 2.0;
            c.text_centered(cx, 190.0, 12.0, rgba(0xc8c8d8, 1.0), "No maps found.");
            let hint = "Show taiko.rs where your osu! Songs folder is, or get maps in DOWNLOAD.";
            let size = Canvas::fit_size(10.0, 7.0, lw, hint);
            c.text_centered(
                cx,
                212.0,
                size,
                rgba(0x8a8aa0, 1.0),
                &Canvas::fit(size, lw, hint),
            );
            let label = if self.picking_folder {
                "Choosing in the folder dialog..."
            } else {
                "Choose Songs folder"
            };
            let bw = (Canvas::text_width(9.0, label) + 32.0).min(lw);
            c.button(
                cx - bw / 2.0,
                234.0,
                bw,
                26.0,
                label,
                9.0,
                Hit::SongsFolder,
                true,
            );
            let later = "or press Enter. Change it later in Settings > Library.";
            c.text_centered(
                cx,
                268.0,
                7.0,
                rgba(0x6f6f86, 1.0),
                &Canvas::fit(7.0, lw, later),
            );
        } else if self.visible.is_empty() {
            // Maps there are, but what's picked leaves none: say how to get them back.
            let narrowed = !self.filter.is_empty()
                || !self.tabs.active(self.shown_source()).is_empty()
                || self.collection.is_some();
            c.text_centered(
                24.0 + lw / 2.0,
                200.0,
                12.0,
                rgba(0xc8c8d8, 1.0),
                if narrowed {
                    "No maps match."
                } else {
                    "No maps found."
                },
            );
            let hint = if narrowed && self.collection.is_some() {
                "Ctrl+Backspace clears the search and filters, Ctrl+L: collections"
            } else if narrowed {
                "Ctrl+Backspace clears the search and filters"
            } else if cfg!(target_os = "android") {
                "Open .osz files with taiko.rs, or get maps in DOWNLOAD"
            } else {
                "Run: taiko-rs <Songs folder | map.osz | map.osu>"
            };
            let size = Canvas::fit_size(10.0, 7.0, lw, hint);
            c.text_centered(
                24.0 + lw / 2.0,
                222.0,
                size,
                rgba(0x8a8aa0, 1.0),
                &Canvas::fit(size, lw, hint),
            );
        }

        // Rows: one per song, plus the selected song's difficulties under it.
        const SONG_H: f32 = SONG_ROW_H;
        const DIFF_H: f32 = 22.0;
        // A heading above each section's songs (what the sort groups by).
        const SECTION_H: f32 = 20.0;
        enum Row {
            Section(usize),
            Song(std::ops::Range<usize>),
            Diff(usize),
        }
        let _ = dt_ms;
        let (top, bottom) = (self.list_top(), self.list_bottom());
        let view = bottom - top;
        // Where everything ends up: the selected song open, the others closed.
        let open_start = self.group_of.get(self.cursor).map(|&g| groups[g].start);
        let section_open: Vec<bool> = (0..self.sections.len())
            .map(|s| self.section_open(s))
            .collect();
        let cursor_section = self.cursor_section();
        let cursor_hidden = cursor_section.is_some_and(|si| !section_open[si]);
        let cursor_group = self.group_of.get(self.cursor).copied();
        let final_layout = ListLayout::new(
            &self.sections,
            &section_open,
            self.headings,
            (SONG_H, SECTION_H),
            cursor_group
                .filter(|_| !cursor_hidden)
                .map(|g| (g, groups[g].len() as f32 * DIFF_H))
                .into_iter()
                .collect(),
        );
        let final_y = final_layout.total;
        let final_cursor = match (cursor_section, cursor_group) {
            // A closed heading holding the selection is the one to keep in view.
            (Some(si), _) if !section_open[si] => final_layout.tops[si],
            (Some(si), Some(g)) => {
                final_layout.song_y(si, g)
                    + SONG_H
                    + (self.cursor - groups[g].start) as f32 * DIFF_H
            }
            _ => 0.0,
        };

        let max_scroll = (final_y - view).max(0.0);
        self.scroll_max = max_scroll;
        let final_scroll = (final_cursor - view * 0.4).clamp(0.0, max_scroll);
        let final_outline = top + final_cursor - final_scroll;
        // Another song selected: the list goes to it.
        if self
            .free
            .as_ref()
            .is_some_and(|f| f.song.as_deref() != self.selected_folder(library))
        {
            self.free = None;
        }

        // A new selection starts a transition from the current state.
        let a = &mut self.anim;
        if a.cursor != Some(self.cursor) || a.open != open_start {
            if a.cursor.is_none() {
                // First frame: no motion.
                *a = ListAnim {
                    open: open_start,
                    open_from: 1.0,
                    scroll_from: final_scroll,
                    outline_from: final_outline,
                    ..Default::default()
                };
            } else {
                if open_start != a.open {
                    let reopen = open_start.is_some() && open_start == a.closing;
                    let from = if reopen { a.closing_now } else { 0.0 };
                    a.closing = a.open;
                    a.closing_from = a.open_now;
                    a.open = open_start;
                    a.open_from = from;
                } else {
                    a.open_from = a.open_now;
                    a.closing_from = a.closing_now;
                }
                a.scroll_from = a.scroll;
                a.outline_from = a.outline;
                a.start = Some(std::time::Instant::now());
                // A release's longer glide applies to the transition it starts, then resets.
                if a.cursor != Some(usize::MAX) {
                    a.ms = 0.0;
                }
            }
            a.cursor = Some(self.cursor);
        }
        let ms = if a.ms > 0.0 { a.ms } else { LIST_ANIM_MS };
        let p = a.start.map(|t| progress(t, ms)).unwrap_or(1.0);
        let open_f = a.open_from + (1.0 - a.open_from) * p;
        let closing_f = a.closing_from * (1.0 - p);
        let mut scroll = a.scroll_from + (final_scroll - a.scroll_from) * p;
        let mut outline = a.outline_from + (final_outline - a.outline_from) * p;
        if let Some(f) = &mut self.free {
            f.glide.settle(max_scroll, c.dt_ms, f.tau);
            scroll = f.glide.now;
            outline = top + final_cursor - scroll;
        }
        if let Some(d) = &self.drag {
            // Held: exactly where the pointer has it (a little past the ends at most).
            let max = max_scroll;
            scroll = if d.now < 0.0 {
                d.now * 0.3
            } else if d.now > max {
                max + (d.now - max) * 0.3
            } else {
                d.now
            };
            outline = top + final_cursor - scroll;
        }
        (a.scroll, a.outline, a.open_now, a.closing_now) = (scroll, outline, open_f, closing_f);
        let (open_g, closing_g) = (a.open, a.closing);
        self.scroll = scroll;

        // (row, y, height, alpha): rows of opening / closing songs are squeezed and faded.
        // Only the rows in view are made: a big library has thousands of songs to scroll past.
        let group_at = |start: Option<usize>| {
            start
                .and_then(|s| self.group_of.get(s).copied())
                .filter(|&g| Some(groups[g].start) == start)
        };
        let anim_extra = |g: Option<usize>, f: f32| {
            g.map(|g| {
                (
                    g,
                    if f > 0.001 {
                        groups[g].len() as f32 * DIFF_H * f
                    } else {
                        0.0
                    },
                )
            })
        };
        let open_gi = group_at(open_g);
        let closing_gi = group_at(closing_g).filter(|g| Some(*g) != open_gi);
        let mut extras: Vec<(usize, f32)> = [
            anim_extra(open_gi, open_f),
            anim_extra(closing_gi, closing_f),
        ]
        .into_iter()
        .flatten()
        .collect();
        extras.sort_by_key(|(g, _)| *g);
        let layout = ListLayout::new(
            &self.sections,
            &section_open,
            self.headings,
            (SONG_H, SECTION_H),
            extras,
        );
        let mut rows: Vec<(Row, f32, f32, f32)> = Vec::new();
        let (from, to) = (scroll - 2.0, scroll + view + 2.0);
        for (si, sec) in self.sections.iter().enumerate() {
            let end = layout.tops.get(si + 1).copied().unwrap_or(layout.total);
            if end < from || layout.tops[si] > to {
                continue;
            }
            if self.headings {
                rows.push((Row::Section(si), layout.tops[si], SECTION_H, 1.0));
            }
            if !section_open[si] {
                continue;
            }
            let shown = layout.visible_songs(si, from, to);
            let mut y = if shown.is_empty() {
                0.0
            } else {
                layout.song_y(si, shown.start)
            };
            for gi in shown {
                let g = &groups[gi];
                rows.push((Row::Song(g.clone()), y, SONG_H, 1.0));
                y += SONG_H;
                let f = if Some(gi) == open_gi {
                    open_f
                } else if Some(gi) == closing_gi {
                    closing_f
                } else {
                    0.0
                };
                if f > 0.001 {
                    for i in g.clone() {
                        rows.push((Row::Diff(i), y, DIFF_H, f * f));
                        y += DIFF_H * f;
                    }
                }
            }
            let _ = sec;
        }
        // The main menu's look: one dark panel the list runs down, rows as lines on it.
        let frame_top = self.frame_top();
        crate::style::panel(c, 20.0, frame_top - 4.0, lw + 8.0, bottom - frame_top + 8.0);
        let tabs = self.source_tabs();
        if !tabs.is_empty() {
            self.draw_source_tabs_classic(c, 24.0, frame_top, lw, &tabs);
        }
        // A new source: its rows come in from the right, one after another, through a flicker.
        let switch_ms = self
            .source_switched
            .map(|t| t.elapsed().as_secs_f32() * 1000.0)
            .filter(|_| crate::render::animations());
        if switch_ms.is_some_and(|ms| ms > 1000.0) {
            self.source_switched = None;
        }
        let mut drawn = 0usize;
        let list_start = c.shapes.len();
        let hits_start = c.hits.len();
        let fit = |s: String, size: f32, room: f32| {
            Canvas::fit(size, room.max(Canvas::char_width(size) * 4.0), &s)
        };
        for (row, ry, rh, alpha) in &rows {
            let y = top + ry - self.scroll;
            if y + rh < top || y > bottom || *alpha < 0.02 {
                continue;
            }
            let row_start = c.shapes.len();
            drawn += 1;
            match row {
                Row::Section(si) => {
                    let sec = &self.sections[*si];
                    let col = sec.colour.unwrap_or_else(accent);
                    let open = section_open[*si];
                    let hover = c.hovered(24.0, y, lw, rh - 2.0);
                    let holds_cursor = cursor_section == Some(*si) && !open;
                    // A heading on the panel: its name on a pill of its colour, a line of it on.
                    if hover {
                        c.round_rect(
                            [24.0, y],
                            [lw, rh - 3.0],
                            6.0,
                            rgba(0x24242f, 0.6),
                            rgba(0, 0.0),
                            0.0,
                        );
                    }
                    if holds_cursor {
                        crate::style::classic::brackets(
                            c,
                            [21.0, y - 3.0, lw + 6.0, rh + 3.0],
                            1.0,
                        );
                    }
                    c.hit(24.0, y, lw, *rh, Hit::SectionHeader(*si));
                    let n = sec.groups.len();
                    c.text(
                        32.0,
                        y + 6.0,
                        7.0,
                        rgba(col, 1.0),
                        if open { "v" } else { ">" },
                    );
                    let label = fit(sec.label.clone(), 7.0, lw - 140.0);
                    let pw = Canvas::text_width(7.0, &label) + 14.0;
                    crate::style::pill(c, 44.0, y + 1.5, pw, rh - 6.0, col);
                    c.text(51.0, y + 6.0, 7.0, rgba(0xffffff, 1.0), &label);
                    let count = format!("{n} song{}", if n == 1 { "" } else { "s" });
                    let after = c.text(
                        44.0 + pw + 8.0,
                        y + 6.0,
                        7.0,
                        rgba(0x8a8aa0, 1.0),
                        &fit(count, 7.0, lw - pw - 40.0),
                    );
                    let line_x = 44.0 + pw + 8.0 + after + 8.0;
                    if line_x < 16.0 + lw {
                        c.rect(
                            line_x,
                            y + rh / 2.0 - 1.5,
                            16.0 + lw - line_x,
                            1.0,
                            rgba(col, 0.3),
                        );
                    }
                }
                Row::Song(g) => {
                    let e = &library[self.visible[g.start]];
                    let open = g.contains(&self.cursor);
                    let hover = !open && c.hovered(24.0, y, lw, rh - 4.0);
                    if open {
                        c.round_rect(
                            [24.0, y],
                            [lw, rh - 4.0],
                            7.0,
                            rgba(crate::style::classic::selector_colour(), 0.08),
                            rgba(0, 0.0),
                            0.0,
                        );
                    } else if hover {
                        c.round_rect(
                            [24.0, y],
                            [lw, rh - 4.0],
                            7.0,
                            rgba(0x262634, 0.6),
                            rgba(0, 0.0),
                            0.0,
                        );
                    }
                    c.rect(32.0, y + rh - 3.0, lw - 16.0, 1.0, rgba(0x23232f, 1.0));
                    c.hit(24.0, y, lw, *rh, Hit::SongHeader(g.start));
                    // A note for the song, as on the main menu: a don for taiko maps, a kat for
                    // converts; the open song's bigger and lit.
                    let any_taiko = g.clone().any(|i| library[self.visible[i]].meta.mode == 1);
                    let colours = crate::game::note_colours();
                    let tag = if any_taiko {
                        colours.don()
                    } else {
                        colours.kat()
                    };
                    let (nr, ny) = (if open { 6.5 } else { 5.0 }, y + (rh - 4.0) / 2.0);
                    crate::style::note(c, 33.0, ny, nr, tag, open, 0.7);
                    // A song with a map marked EPIC: a prism stripe down its left edge, its title
                    // in the shimmer, and a spark round the open one.
                    let epic = g.clone().any(|i| self.is_epic(&library[self.visible[i]]));
                    // A TJA chart's courses, as the arcade's ladder at the right: each in its
                    // colour with its level. The title stops short of it.
                    let one = self.split_difficulties();
                    let ladder: Vec<(u32, String, f32)> = if e.is_tja() && !one {
                        g.clone()
                            .map(|i| {
                                let course = &library[self.visible[i]];
                                let kind = crate::tja_maps::split(&course.path).map(|(_, k)| k);
                                let text = course
                                    .meta
                                    .level
                                    .map_or("?".to_string(), crate::style::level_text);
                                let bw = (Canvas::text_width(6.0, &text) + 6.0).max(12.0);
                                (kind.map_or(0x8a8aa0, course_colour), text, bw)
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let ladder_w: f32 = ladder.iter().map(|l| l.2 + 2.0).sum();
                    let title_room = (lw - 110.0).min(lw - 12.0 - ladder_w - 28.0);
                    let title = fit(e.meta.title.clone(), 10.0, title_room);
                    if epic {
                        crate::epic::stripe(c, 24.0, y + 3.0, rh - 10.0, 1.0);
                        crate::epic::text(c, 44.0, y + 6.0, 10.0, &title, 1.0);
                        if open {
                            crate::epic::spark_round(c, [24.0, y, lw, rh - 4.0], 1.0);
                        }
                    } else {
                        c.text(44.0, y + 6.0, 10.0, rgba(0xffffff, 1.0), &title);
                    }
                    // A row per difficulty: which one it is, rather than who mapped the set.
                    let sub = if one {
                        format!("{}  |  {}", e.meta.artist, e.meta.version)
                    } else {
                        format!("{}  |  mapped by {}", e.meta.artist, e.meta.creator)
                    };
                    // Centred at y + 23.5, in what's left between the title (taller on a text
                    // grid) and the bottom of the box, so the two never touch.
                    let title_bottom = 11.0 + Canvas::text_height(10.0) / 2.0;
                    let room = 2.0 * (23.5 - title_bottom - 1.5).min(rh - 4.0 - 1.5 - 23.5);
                    let sub_size = Canvas::fit_height(7.0, room);
                    c.text(
                        44.0,
                        y + 20.0 + (7.0 - sub_size) / 2.0,
                        sub_size,
                        rgba(0x9a9ab0, 1.0),
                        &fit(sub, sub_size, lw - 110.0),
                    );
                    // A newer version on osu!: click (or Ctrl+Shift+U) to get it. Under the
                    // difficulty count (moved up for it), inside the row (not on one cut off at
                    // the list's end).
                    let update = self.update_of(e).filter(|u| {
                        matches!(u, SetUpdate::Available | SetUpdate::Updating)
                            && y + rh - 4.0 <= bottom
                    });
                    let n = g.len();
                    let diffs_y = if update.is_some() { y + 6.0 } else { y + 11.0 };
                    if one {
                        // What the list is ordered by (no mods, 1.0x); the open row has the mods'.
                        let sorted_by = self
                            .row_value
                            .get(g.start)
                            .copied()
                            .filter(|v| v.is_finite())
                            .map(|v| v as f32);
                        // A TJA course is ordered by its level, and shows it as its charter wrote it.
                        let level = e.meta.level.filter(|_| self.sort == SortMode::Difficulty);
                        crate::style::classic::map_pill(
                            c,
                            24.0 + lw - 12.0,
                            diffs_y - 1.0,
                            8.0,
                            level,
                            sorted_by,
                        );
                    } else if !ladder.is_empty() {
                        let mut right = 24.0 + lw - 12.0;
                        for (colour, text, bw) in ladder.iter().rev() {
                            let (colour, bw) = (*colour, *bw);
                            right -= bw;
                            c.rect(right, diffs_y - 2.0, bw, 11.0, rgba(colour, 1.0));
                            c.rect(right, diffs_y + 8.0, bw, 1.0, rgba(0x000000, 0.35));
                            c.text_centered(
                                right + bw / 2.0,
                                diffs_y,
                                6.0,
                                rgba(0x15151d, 1.0),
                                text,
                            );
                            right -= 2.0;
                        }
                    } else {
                        c.text_right(
                            24.0 + lw - 12.0,
                            diffs_y,
                            8.0,
                            rgba(0x8a8aa0, 1.0),
                            &format!("{n} diff{}", if n == 1 { "" } else { "s" }),
                        );
                    }
                    let (by, bh) = (y + 19.0, 9.0);
                    let ty = by + (bh - Canvas::text_height(6.0)) / 2.0;
                    match update {
                        Some(SetUpdate::Available) => {
                            let label = "update";
                            let bw = Canvas::text_width(6.0, label) + 10.0;
                            let bx = 24.0 + lw - 12.0 - bw;
                            let hover = c.hovered(bx, by, bw, bh);
                            c.round_rect(
                                [bx, by],
                                [bw, bh],
                                3.0,
                                rgba(if hover { 0x5a4a1a } else { 0x3a3218 }, 1.0),
                                rgba(0xFFD34D, 1.0),
                                1.0,
                            );
                            c.text_centered(bx + bw / 2.0, ty, 6.0, rgba(0xFFD34D, 1.0), label);
                            c.hit(bx, by, bw, bh, Hit::UpdateSet(self.visible[g.start]));
                        }
                        Some(SetUpdate::Updating) => c.text_right(
                            24.0 + lw - 12.0,
                            ty,
                            6.0,
                            rgba(0x4fb4ff, 1.0),
                            "updating...",
                        ),
                        _ => {}
                    }
                }
                Row::Diff(i) => {
                    let a = *alpha;
                    let e = &library[self.visible[*i]];
                    let selected = *i == self.cursor;
                    if selected {
                        c.round_rect(
                            [44.0, y],
                            [lw - 20.0, rh - 3.0],
                            5.0,
                            rgba(
                                crate::style::classic::selector_colour(),
                                (0.12 + 0.05 * crate::style::pulse()) * a,
                            ),
                            rgba(0, 0.0),
                            0.0,
                        );
                    } else if c.hovered(44.0, y, lw - 20.0, rh - 3.0) {
                        c.round_rect(
                            [44.0, y],
                            [lw - 20.0, rh - 3.0],
                            5.0,
                            rgba(0x24242f, 0.6 * a),
                            rgba(0, 0.0),
                            0.0,
                        );
                    }
                    c.rect(52.0, y + rh - 2.0, lw - 36.0, 1.0, rgba(0x1d1d27, a));
                    c.hit(44.0, y, lw - 20.0, *rh, Hit::SongRow(*i));
                    let col = if selected { 0xffffff } else { 0xc8c8d8 };
                    let convert = if e.meta.mode == 0 { "  (convert)" } else { "" };
                    let pill = if a > 0.6 {
                        crate::style::classic::map_pill(
                            c,
                            24.0 + lw - 10.0,
                            y + 6.0,
                            7.0,
                            e.meta.level,
                            stars(self.visible[*i]),
                        )
                    } else {
                        40.0
                    };
                    if let Some(g) = grade(e) {
                        let right = 24.0 + lw - 18.0 - pill;
                        // SS is split into don and kat colours, like on the website.
                        if g == "SS" {
                            c.text_right(right - 1.0, y + 5.0, 9.0, rgba(0xEB452C, a), g);
                            c.text_right(right + 1.0, y + 5.0, 9.0, rgba(0x4fb4ff, a), g);
                        }
                        c.text_right(right, y + 5.0, 9.0, rgba(grade_colour(g), a), g);
                    }
                    c.text(
                        56.0,
                        y + 6.0,
                        8.0,
                        rgba(col, a),
                        &fit(format!("{}{convert}", e.meta.version), 8.0, lw - 170.0),
                    );
                }
            }
            if let Some(ms) = switch_ms {
                let t = ((ms - drawn as f32 * 18.0) / lens::SWITCH_MS).clamp(0.0, 1.0);
                if t < 1.0 {
                    let ease = 1.0 - (1.0 - t).powi(3);
                    c.transform_since(row_start, [0.0, 0.0], 1.0, (1.0 - ease) * 28.0, 0.0);
                    c.fade_since(row_start, ease, 1.0);
                }
            }
        }
        if let Some(ms) = switch_ms.filter(|&ms| ms < 160.0) {
            crate::style::glitch_bars(c, [24.0, top, lw, bottom - top], ms / 160.0);
        }

        // The selection outline moves with the same transition.
        // Not with its heading closed: the heading shows where the selection is instead.
        let oy = outline;
        if !self.visible.is_empty() && !cursor_hidden && oy > top - DIFF_H && oy < bottom {
            crate::style::classic::brackets(c, [41.0, oy - 3.0, lw - 14.0, DIFF_H + 3.0], 1.0);
        }
        // Rows scrolling past the edges slide under them instead of over the search row.
        c.clip_since(list_start, 20.0, top - 2.0, lw + 8.0, bottom - top + 4.0);
        for h in &mut c.hits[hits_start..] {
            let [_, y, _, hh] = &mut h.rect;
            let (y0, y1) = (y.max(top - 2.0), (*y + *hh).min(bottom + 2.0));
            (*y, *hh) = (y0, (y1 - y0).max(0.0));
        }
        // The scroll bar (after the list's clip, outside it), in the margin left of the list: where
        // the list is, clicked or dragged to go somewhere (`scroll_to`, like right-dragging).
        if groups.len() > 1 {
            let (track_x, track_w) = (8.0, 8.0);
            let track = bottom - top;
            let hover = c.hovered(track_x - 4.0, top, track_w + 12.0, track);
            c.round_rect(
                [track_x, top],
                [track_w, track],
                4.0,
                rgba(0x15151d, if hover { 0.9 } else { 0.6 }),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
            let total = final_y.max(view);
            // Where each heading starts, as a mark on the track.
            if self.headings && self.sections.len() > 1 {
                for (si, sec) in self.sections.iter().enumerate() {
                    let ty = top + final_layout.tops[si] / total * track;
                    let col = sec.colour.unwrap_or(0x6f6f86);
                    c.rect(track_x - 3.0, ty, 3.0, 1.0, rgba(col, 0.9));
                }
            }
            let thumb = (track * view / total).clamp(18.0, track);
            self.thumb = thumb;
            let pos = (scroll / (total - view).max(1.0)).clamp(0.0, 1.0);
            c.round_rect(
                [track_x + 1.0, top + pos * (track - thumb)],
                [track_w - 2.0, thumb],
                3.0,
                rgba(if hover { accent() } else { 0x8a8aa8 }, 0.95),
                rgba(0, 0.0),
                0.0,
            );
            c.hit(
                track_x - 4.0,
                top,
                track_w + 12.0,
                track,
                Hit::SongScrollbar,
            );
        }

        // Flying through the list: which heading it's passing.
        if let Some(py) = self.scrubbing {
            let line = scroll + view / 2.0;
            let heading = self
                .headings
                .then(|| layout.tops.partition_point(|t| *t <= line).checked_sub(1))
                .flatten();
            if let Some(si) = heading {
                let label = Canvas::fit(10.0, lw - 40.0, &self.sections[si].label);
                let bw = Canvas::text_width(10.0, &label) + 24.0;
                let by = (py - 13.0).clamp(top, bottom - 26.0);
                c.round_rect(
                    [30.0, by],
                    [bw, 26.0],
                    6.0,
                    rgba(0x15151d, 0.96),
                    rgba(accent(), 1.0),
                    1.5,
                );
                c.text(42.0, by + 8.0, 10.0, rgba(0xffffff, 1.0), &label);
            }
        }
        // Scrolled away from the selection: the way back.
        let cursor_y = top + final_cursor - scroll;
        if self.free.is_some()
            && self.scrubbing.is_none()
            && !self.visible.is_empty()
            && (cursor_y < top - DIFF_H || cursor_y > bottom)
        {
            let label = tr!(
                "back to the selected song",
                "選択中の曲へ戻る",
                "к выбранной песне"
            );
            let bw = Canvas::text_width(7.0, label) + 20.0;
            c.button(
                24.0 + (lw - bw) / 2.0,
                bottom - 24.0,
                bw,
                20.0,
                label,
                7.0,
                Hit::ListBack,
                true,
            );
        }

        let hints = tr!(
            "Enter play  Up/Down difficulty  Left/Right song  Wheel or right drag scroll  F1 mods  F2 random  [ ] rate  Ctrl+S sort and filter  Ctrl+L collection  Tab settings  Esc quit",
            "Enter プレイ  上下 難易度  左右 曲  ホイールか右ドラッグ スクロール  F1 Mod  F2 ランダム  [ ] レート  Ctrl+S 並び替えと絞り込み  Ctrl+L コレクション  Tab 設定  Esc 終了",
            "Enter играть  Вверх/вниз сложность  Влево/вправо песня  Колесо или правый драг: прокрутка  F1 моды  F2 случайная  [ ] рейт  Ctrl+S сортировка и фильтры  Ctrl+L коллекция  Tab настройки  Esc выход"
        );
        if legacy {
            c.text(
                24.0,
                452.0,
                7.0,
                rgba(0x6f6f86, 1.0),
                &Canvas::fit_hints(7.0, w - 48.0, hints),
            );
        }
        // The status line gives way (smaller) when the hints above come out taller.
        let status_size = Canvas::fit_height(
            7.0,
            2.0 * (469.5 - (455.5 + Canvas::text_height(7.0) / 2.0) - 3.0),
        );
        c.text(
            24.0,
            466.0 + (7.0 - status_size) / 2.0,
            status_size,
            rgba(0x55556a, 1.0),
            &Canvas::fit(status_size, w - 48.0, status),
        );
    }
}
