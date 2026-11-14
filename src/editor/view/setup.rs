//! File > Song setup: the map's names, tags and difficulty settings in one window, typed in or
//! stepped with - and +, each change one undo step (as osu!stable's Song setup).

use super::chrome::{DIM, WARN, wrap};
use super::{Btn, Cmd, Field, Modal, View};
use crate::editor::setup::Key;
use crate::hit::Canvas;
use crate::render::{accent, rgba};

/// The text fields, two to a row (`None`: a gap).
const ROWS: [[Option<Key>; 2]; 4] = [
    [Some(Key::ArtistUnicode), Some(Key::Artist)],
    [Some(Key::TitleUnicode), Some(Key::Title)],
    [Some(Key::Creator), Some(Key::Version)],
    [Some(Key::Source), Some(Key::BasedOn)],
];

const NUMBERS: [Key; 3] = [Key::Hp, Key::Od, Key::BaseSv];

/// Where Tab goes from `k` (Shift+Tab the other way): the text fields in the order they show.
/// The original mapper's box is there only for an `edit` of an osu! map.
pub fn next_text(k: Key, back: bool, edit: bool) -> Key {
    let order: Vec<Key> = ROWS
        .iter()
        .flatten()
        .flatten()
        .copied()
        .filter(|k| edit || *k != Key::BasedOn)
        .chain([Key::Tags])
        .collect();
    let at = order.iter().position(|o| *o == k).unwrap_or(0);
    let n = order.len();
    order[if back { (at + n - 1) % n } else { (at + 1) % n }]
}

/// `s` cut from the front until it fits `w` (the end of what is being typed stays in sight).
fn tail(size: f32, w: f32, s: &str) -> String {
    if Canvas::text_width(size, s) <= w {
        return s.to_string();
    }
    let mut cut: Vec<char> = s.chars().collect();
    while !cut.is_empty() && Canvas::text_width(size, &format!("..{}", String::from_iter(&cut))) > w
    {
        cut.remove(0);
    }
    format!("..{}", String::from_iter(&cut))
}

// Only the regression suite (Linux only) opens it this way.
#[cfg(all(test, target_os = "linux"))]
impl View {
    /// File > Song setup, as the menu opens it.
    pub(crate) fn song_setup_for_tests(&mut self) {
        self.open_setup();
    }
}

impl View {
    /// File > Song setup.
    pub(super) fn open_setup(&mut self) {
        self.commit_typing();
        self.modal = Some(Modal::Setup);
    }

    /// Whether Song setup has the original mapper's box: the map came from osu!. Not in a
    /// mapping session, whose ops carry song setup's other keys only (`mapping::KEYS`).
    pub(super) fn names_original(&self) -> bool {
        self.ed.osu_set().is_some() && self.collab.is_none()
    }

    /// The star rating of the map as it is now, worked out again after each change.
    fn setup_stars(&mut self) -> Option<f64> {
        let rev = self.ed.history.rev();
        if self.setup_stars.is_none_or(|(at, _)| at != rev) {
            let stars = crate::stars::star_rating_text(&self.ed.chart.to_text());
            self.setup_stars = Some((rev, stars));
        }
        self.setup_stars.and_then(|(_, s)| s)
    }

    /// Starts typing into `k` (a text shows what it holds, to be replaced or added to).
    pub(super) fn type_setting(&mut self, k: Key) {
        let now = if k.is_text() {
            self.ed.setting(k)
        } else {
            String::new()
        };
        self.typing_fresh = k.is_text() && !now.is_empty();
        self.typing = Some((Field::Meta(k), now));
    }

    pub(super) fn apply_setting(&mut self, k: Key, text: &str) {
        match self.ed.set_setting(k, text) {
            Ok(true) => {
                self.mark_stale();
                self.status = format!("{}: {}", k.label(), self.ed.setting(k));
            }
            Ok(false) => {}
            Err(e) => self.status = e,
        }
    }

    pub(super) fn step_setting(&mut self, k: Key, dir: f64, keys: super::Keys) {
        let k_steps = if keys.shift {
            2.0
        } else if keys.alt {
            // 0.1 for HP and OD, 0.01 for the base SV.
            if k == Key::BaseSv { 0.1 } else { 0.2 }
        } else {
            1.0
        };
        if self.ed.step_setting(k, dir * k_steps) {
            self.mark_stale();
            self.status = format!("{}: {}", k.label(), self.ed.setting(k));
        }
    }

    /// A box that shows a setting and takes a click to type into it.
    fn setting_box(&mut self, c: &mut Canvas, [x, y, w, h]: [f32; 4], k: Key) {
        let typing = match &self.typing {
            Some((Field::Meta(t), text)) if *t == k => Some(text.clone()),
            _ => None,
        };
        let hover = c.hovered(x, y, w, h);
        let border = if typing.is_some() {
            accent()
        } else if hover {
            0x6a6a88
        } else {
            0x3a3a4e
        };
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x101018, 1.0),
            rgba(border, 1.0),
            1.0,
        );
        let size = 7.0;
        let (tx, ty, room) = (x + 5.0, y + (h - size) / 2.0, w - 10.0);
        match typing {
            Some(text) if self.typing_fresh => {
                let shown = tail(size, room, &text);
                let sw = Canvas::text_width(size, &shown);
                c.rect(
                    tx - 1.0,
                    ty - 1.5,
                    sw + 2.0,
                    size + 3.0,
                    rgba(accent(), 0.45),
                );
                c.text(tx, ty, size, rgba(0xffffff, 1.0), &shown);
            }
            Some(text) => {
                let shown = tail(size, room, &format!("{text}_"));
                c.text(tx, ty, size, rgba(0xffffff, 1.0), &shown);
            }
            None => {
                let v = self.ed.setting(k);
                if v.is_empty() {
                    c.text(tx, ty, size, rgba(0x60607a, 1.0), "empty");
                } else {
                    c.text(
                        tx,
                        ty,
                        size,
                        rgba(0xe0e0ec, 1.0),
                        &Canvas::fit(size, room, &v),
                    );
                }
            }
        }
        self.buttons.push(([x, y, w, h], Btn::Edit(Field::Meta(k))));
    }

    pub(super) fn draw_setup(&mut self, c: &mut Canvas) {
        c.rect(0.0, 0.0, c.width, 480.0, rgba(0x000000, 0.62));
        let w = 500.0_f32.min(c.width - 20.0);
        let (inner, gap) = (w - 28.0, 10.0);
        let small = 6.5;
        let about = wrap(
            small,
            inner,
            "This difficulty's names and settings. Ctrl+S saves them with the map; Ctrl+Z takes a change back.",
        );
        // The romanised names are what osu! sorts and searches by: plain Latin letters only.
        let latin = [Key::Artist, Key::Title]
            .iter()
            .all(|k| self.ed.setting(*k).is_ascii());
        let note = wrap(
            small,
            inner,
            if latin {
                "Romanised: the names in plain Latin letters (filled in for you while both are the same)."
            } else {
                "A romanised name has letters that aren't plain Latin: osu! lists maps by these."
            },
        );
        let od = self.ed.setting_number(Key::Od);
        let windows = crate::game::Windows::new(od);
        let (great, ok) = (windows.great, windows.ok);
        let stars = match self.setup_stars() {
            Some(s) => format!("{s:.2} stars"),
            None => "no star rating".to_string(),
        };
        let info = wrap(
            small,
            inner,
            &format!(
                "Hit windows: great ±{great:.1} ms, ok ±{ok:.1} ms. {stars}. Base SV is how fast notes scroll at 1.0x SV; drumrolls keep their length when it changes."
            ),
        );
        let label = "Done";
        let bw = Canvas::text_width(7.0, label) + 18.0;
        let hint = wrap(
            small,
            inner - bw - 10.0,
            "Enter keeps what you typed, Tab goes to the next box, Esc closes.",
        );
        let line = 9.5;
        let lines = |l: &Vec<String>| l.len() as f32 * line;
        let h = 30.0
            + lines(&about)
            + 6.0
            + 14.0
            + 4.0 * 30.0
            + 30.0
            + lines(&note)
            + 6.0
            + 14.0
            + 32.0
            + lines(&info)
            + 10.0
            + 20.0_f32.max(lines(&hint))
            + 12.0;
        let (x, y) = ((c.width - w) / 2.0, ((480.0 - h) / 2.0 - 6.0).max(4.0));
        self.overlay_rect = [x, y, w, h];
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x1b1b26, 1.0),
            rgba(0x6a6a88, 1.0),
            1.0,
        );
        let x0 = x + 14.0;
        c.text(x0, y + 12.0, 9.0, rgba(0xffffff, 1.0), "Song setup");
        let mut ty = y + 27.0;
        for l in &about {
            c.text(x0, ty, small, rgba(DIM, 1.0), l);
            ty += line;
        }
        ty += 6.0;
        let col = (inner - gap) / 2.0;
        c.text(x0, ty, 7.5, rgba(accent(), 1.0), "Metadata");
        ty += 14.0;
        let edit = self.names_original();
        for row in ROWS {
            for (i, k) in row.into_iter().enumerate() {
                let Some(k) = k.filter(|k| edit || *k != Key::BasedOn) else {
                    continue;
                };
                let fx = x0 + i as f32 * (col + gap);
                c.text(fx, ty, small, rgba(DIM, 1.0), k.label());
                self.setting_box(c, [fx, ty + 9.0, col, 16.0], k);
            }
            ty += 30.0;
        }
        c.text(
            x0,
            ty,
            small,
            rgba(DIM, 1.0),
            "Tags (words to find the map by)",
        );
        self.setting_box(c, [x0, ty + 9.0, inner, 16.0], Key::Tags);
        ty += 30.0;
        for l in &note {
            c.text(x0, ty, small, rgba(if latin { DIM } else { WARN }, 1.0), l);
            ty += line;
        }
        ty += 6.0;
        c.text(x0, ty, 7.5, rgba(accent(), 1.0), "Difficulty");
        ty += 14.0;
        let third = (inner - 2.0 * gap) / 3.0;
        for (i, k) in NUMBERS.into_iter().enumerate() {
            let fx = x0 + i as f32 * (third + gap);
            c.text(fx, ty, small, rgba(DIM, 1.0), k.label());
            let by = ty + 9.0;
            self.button(
                c,
                [fx, by, 16.0, 16.0],
                "-",
                Btn::Dec(Field::Meta(k)),
                false,
            );
            self.setting_box(c, [fx + 19.0, by, third - 38.0, 16.0], k);
            self.button(
                c,
                [fx + third - 16.0, by, 16.0, 16.0],
                "+",
                Btn::Inc(Field::Meta(k)),
                false,
            );
        }
        ty += 32.0;
        for l in &info {
            c.text(x0, ty, small, rgba(DIM, 1.0), l);
            ty += line;
        }
        ty += 10.0;
        self.button(c, [x0, ty, bw, 20.0], label, Btn::C(Cmd::Stay), true);
        let mut hy = ty + 10.0 - lines(&hint) / 2.0 + 1.0;
        for l in &hint {
            c.text(x0 + bw + 10.0, hy, small, rgba(DIM, 1.0), l);
            hy += line;
        }
    }
}
