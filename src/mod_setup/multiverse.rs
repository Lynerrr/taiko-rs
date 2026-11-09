//! Multiverse's setup panel: each universe's look, shape, speed and mods, and the presets.

use crate::hit::{Canvas, Hit};
use crate::multiverse::*;
use crate::render::{accent, rgba};
use winit::keyboard::KeyCode;

/// The columns of the setup: colours, shape, speed, then one per mod.
pub(crate) const COLUMNS: usize = 3 + MODS.len();

/// Palette `i`'s name as shown (`CUSTOM`: the player's own colours).
pub(crate) fn palette_label(i: usize) -> &'static str {
    match i {
        0 => tr!("classic", "クラシック", "классика"),
        1 => tr!("neon", "ネオン", "неон"),
        2 => tr!("sunset", "夕焼け", "закат"),
        3 => tr!("ice", "氷", "лёд"),
        4 => tr!("forest", "森", "лес"),
        5 => tr!("candy", "キャンディ", "конфеты"),
        6 => tr!("ember", "残り火", "угли"),
        7 => tr!("mono", "モノクロ", "моно"),
        8 => tr!("toxic", "毒々しい", "токсик"),
        9 => tr!("swapped", "入れ替え", "наоборот"),
        _ => tr!("custom", "自分の色", "свои"),
    }
}

/// A look's name as shown, in the player's language (its `name` is the config's): the words are
/// the client's, taiko-logic keeps the looks in no language.
pub(crate) trait LookText {
    fn label(self) -> &'static str;
}

impl LookText for Colours {
    fn label(self) -> &'static str {
        palette_label(self.palette)
    }
}

impl LookText for Shape {
    fn label(self) -> &'static str {
        match self {
            Shape::Round => tr!("round", "丸", "круг"),
            Shape::Square => tr!("square", "四角", "квадрат"),
            Shape::Diamond => tr!("diamond", "ひし形", "ромб"),
            Shape::Ring => tr!("ring", "輪", "кольцо"),
            Shape::Pill => tr!("pill", "カプセル", "капсула"),
        }
    }
}

/// Every preset in the order the setup lists them: the built-in ones, then the player's.
pub(crate) fn presets(own: &[Preset]) -> Vec<(String, [Universe; COUNT])> {
    crate::multiverse::presets(
        own,
        [
            tr!("defaults", "初期設定", "исходные"),
            tr!("classic", "クラシック", "классика"),
        ],
    )
}

/// The Multiverse setup (M in the mod select): a row per universe, a column per setting, and
/// the presets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub row: usize,
    pub col: usize,
    pub typing: Option<Typing>,
    /// What was typed didn't work (the prompt shows red until the next key).
    pub bad: bool,
}

impl Setup {
    pub fn new(row: usize, col: usize) -> Setup {
        Setup {
            row,
            col,
            ..Default::default()
        }
    }

    /// Changes the setting at `row`, `col`: the next (`dir` 1) or previous (-1) colours, shape or
    /// speed; a mod switches on or off.
    pub fn change(all: &mut [Universe; COUNT], row: usize, col: usize, dir: i32) {
        let u = &mut all[row];
        let step = |i: usize, n: usize| (i as i32 + dir).rem_euclid(n as i32) as usize;
        match col {
            0 => u.colours = u.colours.step(dir),
            1 => {
                u.shape = Shape::ALL[step(
                    Shape::ALL.iter().position(|&s| s == u.shape).unwrap_or(0),
                    Shape::ALL.len(),
                )]
            }
            2 => u.speed = step(u.speed, SPEEDS.len()),
            c => u.mods.toggle(MODS[(c - 3).min(MODS.len() - 1)]),
        }
    }

    /// The preset (index in the setup's list) the universes are now, if any.
    pub fn current(all: &[Universe; COUNT], own: &[Preset]) -> Option<usize> {
        presets(own).iter().position(|p| p.1 == *all)
    }

    /// Loads preset `i` of the setup's list.
    pub fn load(all: &mut [Universe; COUNT], own: &[Preset], i: usize) -> Action {
        match presets(own).get(i) {
            Some(p) => {
                *all = p.1;
                Action::Changed
            }
            None => Action::None,
        }
    }

    /// Deletes preset `i` of the setup's list (only the player's own).
    pub fn delete(own: &mut Vec<Preset>, i: usize) -> Action {
        match i.checked_sub(BUILTIN_PRESETS).filter(|&k| k < own.len()) {
            Some(k) => {
                own.remove(k);
                Action::Changed
            }
            None => Action::None,
        }
    }

    /// Enter while typing: the universe gets its own colours, or the universes are saved under
    /// the name (replacing a preset of that name). Something unusable keeps the typing open.
    fn finish_typing(&mut self, all: &mut [Universe; COUNT], own: &mut Vec<Preset>) -> Action {
        match &self.typing {
            Some(Typing::Colours(s)) => match parse_pair(s) {
                Some([don, kat]) => all[self.row].colours = Colours::own(don, kat),
                None => {
                    self.bad = true;
                    return Action::None;
                }
            },
            Some(Typing::Name(s)) => {
                let name = clean_name(s);
                if name.is_empty() {
                    self.bad = true;
                    return Action::None;
                }
                let preset = Preset {
                    name,
                    universes: *all,
                };
                match own
                    .iter_mut()
                    .find(|p| p.name.eq_ignore_ascii_case(&preset.name))
                {
                    Some(p) => *p = preset,
                    None => own.push(preset),
                }
            }
            None => return Action::None,
        }
        self.typing = None;
        Action::Changed
    }

    pub fn key(
        &mut self,
        code: KeyCode,
        text: Option<&str>,
        shift: bool,
        all: &mut [Universe; COUNT],
        own: &mut Vec<Preset>,
        first_focus: usize,
    ) -> Action {
        self.bad = false;
        if let Some(Typing::Colours(s) | Typing::Name(s)) = &mut self.typing {
            match code {
                KeyCode::Escape => self.typing = None,
                KeyCode::Enter | KeyCode::NumpadEnter => return self.finish_typing(all, own),
                KeyCode::Backspace => {
                    s.pop();
                }
                _ => {
                    for ch in text.unwrap_or("").chars().filter(|c| !c.is_control()) {
                        if s.chars().count() < 40 {
                            s.push(ch);
                        }
                    }
                }
            }
            return Action::None;
        }
        let dir = if shift { -1 } else { 1 };
        match code {
            KeyCode::ArrowUp => self.row = (self.row + COUNT - 1) % COUNT,
            KeyCode::ArrowDown => self.row = (self.row + 1) % COUNT,
            KeyCode::ArrowLeft => self.col = (self.col + COLUMNS - 1) % COLUMNS,
            KeyCode::ArrowRight => self.col = (self.col + 1) % COLUMNS,
            KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter => {
                Self::change(all, self.row, self.col, dir);
                return Action::Changed;
            }
            KeyCode::KeyE => {
                let c = all[self.row].colours;
                self.col = 0;
                self.typing = Some(Typing::Colours(format!(
                    "#{:06x} #{:06x}",
                    c.don(),
                    c.kat()
                )));
            }
            KeyCode::KeyS => {
                let name = Self::current(all, own)
                    .filter(|&i| i >= BUILTIN_PRESETS)
                    .map(|i| own[i - BUILTIN_PRESETS].name.clone());
                self.typing = Some(Typing::Name(name.unwrap_or_default()));
            }
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                let n = presets(own).len() as i32;
                let step = if code == KeyCode::BracketRight { 1 } else { -1 };
                let at =
                    Self::current(all, own).map_or(if step > 0 { -1 } else { 0 }, |i| i as i32);
                return Self::load(all, own, (at + step).rem_euclid(n) as usize);
            }
            KeyCode::Delete => {
                if let Some(i) = Self::current(all, own) {
                    return Self::delete(own, i);
                }
            }
            KeyCode::KeyR => {
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64);
                *all = shuffle(seed, first_focus);
                return Action::Changed;
            }
            KeyCode::KeyC => {
                *all = classic();
                return Action::Changed;
            }
            KeyCode::KeyD => {
                *all = defaults();
                return Action::Changed;
            }
            KeyCode::Escape | KeyCode::KeyM | KeyCode::F1 | KeyCode::Backspace | KeyCode::Tab => {
                return Action::Close;
            }
            _ => {}
        }
        Action::None
    }

    /// Draws the setup over the mod select. `first_focus`: the universe the focus starts on.
    pub fn draw(
        &self,
        c: &mut Canvas,
        all: &[Universe; COUNT],
        own: &[Preset],
        first_focus: usize,
    ) {
        let w = c.width;
        c.rect(
            0.0,
            0.0,
            w,
            480.0,
            rgba(0x000000, crate::mobile::pick(0.6, 0.97)),
        );
        let (px, py, pw, ph) = crate::mobile::setup_rect(w);
        c.round_rect(
            [px, py],
            [pw, ph],
            10.0,
            rgba(0x15151d, 0.99),
            rgba(0xff5fa2, 0.9),
            1.5,
        );
        let title_w = c.text(
            px + 16.0,
            py + 12.0,
            12.0,
            rgba(0xffffff, 1.0),
            "MULTIVERSE",
        );
        let hint = tr!(
            "arrows choose   Space change (Shift back)   E own colours   S save preset   [ ] presets   R shuffle   Esc back",
            "矢印 選択   Space 変更 (Shiftで戻す)   E 自分の色   S プリセット保存   [ ] プリセット   R シャッフル   Esc 戻る",
            "стрелки выбор   Space менять (Shift назад)   E свои цвета   S сохранить   [ ] пресеты   R перемешать   Esc назад"
        );
        c.text(
            px + 26.0 + title_w,
            py + 16.0,
            6.0,
            rgba(0x6f6f86, 1.0),
            &crate::mobile::key_hint(6.0, pw - title_w - 42.0, hint),
        );

        // Columns: number and its notes, colours, shape, speed, the mod chips.
        let (num_w, look_w, shape_w, speed_w) = if crate::mobile::active() {
            (72.0, 128.0, 108.0, 70.0)
        } else {
            (70.0, 72.0, 62.0, 46.0)
        };
        let x0 = px + 12.0;
        let chips_x = x0 + num_w + look_w + shape_w + speed_w;
        let chip_w =
            ((px + pw - 12.0 - chips_x) / MODS.len() as f32).min(crate::mobile::pick(34.0, 60.0));
        let col_x = |col: usize| match col {
            0 => (x0 + num_w, look_w),
            1 => (x0 + num_w + look_w, shape_w),
            2 => (x0 + num_w + look_w + shape_w, speed_w),
            c => (chips_x + (c - 3) as f32 * chip_w, chip_w),
        };
        let head_y = py + 36.0;
        for (col, name) in [
            (0, tr!("COLOURS", "色", "ЦВЕТА")),
            (1, tr!("SHAPE", "形", "ФОРМА")),
            (2, tr!("SPEED", "速度", "СКОРОСТЬ")),
            (3, tr!("MODS", "MOD", "МОДЫ")),
        ] {
            c.text(col_x(col).0 + 4.0, head_y, 6.0, rgba(0x8a8aa0, 1.0), name);
        }
        const ROW: f32 = 36.0;
        let mut hovered = None;
        for (i, u) in all.iter().enumerate() {
            let y = head_y + 12.0 + i as f32 * ROW;
            let (h, mid) = (ROW - 5.0, y + (ROW - 5.0) / 2.0);
            c.round_rect(
                [x0, y],
                [px + pw - 12.0 - x0, h],
                6.0,
                rgba(if i == self.row { 0x23232f } else { 0x1b1b24 }, 1.0),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
            let label = if i == first_focus {
                format!("{}*", i + 1)
            } else {
                (i + 1).to_string()
            };
            c.text(x0 + 8.0, mid - 4.0, 8.0, rgba(0xc8c8d8, 1.0), &label);
            crate::game::shaped_note(c, u.shape, x0 + 36.0, mid, 9.5, u.don(), 1.0);
            crate::game::shaped_note(c, u.shape, x0 + 58.0, mid, 9.5, u.kat(), 1.0);
            for col in 0..COLUMNS {
                let (cx, cw) = col_x(col);
                let (cx, cw) = (cx + 2.0, cw - 4.0);
                let selected = i == self.row && col == self.col;
                if c.hovered(cx, y + 3.0, cw, h - 6.0) {
                    hovered = Some(col);
                }
                let on = col >= 3 && u.mods.has(MODS[col - 3]);
                let fill = if on {
                    0x6b2a4c
                } else if selected {
                    0x34344a
                } else {
                    0x1d1d27
                };
                let border = if selected {
                    accent()
                } else if on {
                    0xff5fa2
                } else {
                    0x2a2a38
                };
                c.round_rect(
                    [cx, y + 3.0],
                    [cw, h - 6.0],
                    4.0,
                    rgba(fill, 1.0),
                    rgba(border, 1.0),
                    if selected { 1.5 } else { 1.0 },
                );
                let text = match col {
                    0 => u.colours.label().to_string(),
                    1 => u.shape.label().to_string(),
                    2 => format!("{}x", SPEEDS[u.speed]),
                    c => MODS[c - 3].acronym().to_string(),
                };
                let col_text = if col >= 3 && !on { 0x6f6f86 } else { 0xffffff };
                c.text_centered(
                    cx + cw / 2.0,
                    mid - 3.5,
                    7.0,
                    rgba(col_text, 1.0),
                    &Canvas::fit(7.0, cw - 4.0, &text),
                );
                c.hit(cx, y + 3.0, cw, h - 6.0, Hit::UniverseCell(i, col));
            }
        }

        // What's being typed, else what the pointed-at (or the selected) setting is.
        let line_y = py + ph - crate::mobile::pick(48.0, 68.0);
        let (line, col) = match &self.typing {
            Some(Typing::Colours(s)) => {
                let n = self.row + 1;
                let line = tr!(
                    "Universe {n}'s own colours, don then kat: {s}_   Enter set   Esc cancel",
                    "宇宙{n}の自分の色 (ドン、カッの順): {s}_   Enter 決定   Esc 取消",
                    "Свои цвета вселенной {n}, дон и кат: {s}_   Enter задать   Esc отмена";
                );
                (line, 0xFFD34D)
            }
            Some(Typing::Name(s)) => {
                let line = tr!(
                    "Save these universes as: {s}_   Enter save (the same name replaces it)   Esc cancel",
                    "この宇宙たちの保存名: {s}_   Enter 保存 (同じ名前は上書き)   Esc 取消",
                    "Сохранить вселенные как: {s}_   Enter сохранить (то же имя заменяет)   Esc отмена";
                );
                (line, 0xFFD34D)
            }
            None => {
                let about = match hovered.unwrap_or(self.col) {
                    0 => tr!(
                        "Colours: dons' and kats' (out of focus they stay alike, faintly tinted). E: your own, like #d03020 #40a0ff.",
                        "色: ドンとカッ (焦点外では同じ色にうっすら色付き)。E: 自分の色 (#d03020 #40a0ffのように)。",
                        "Цвета: донов и катов (вне фокуса одинаковые, слегка подкрашены). E: свои, вроде #d03020 #40a0ff."
                    )
                    .to_string(),
                    1 => tr!("Shape: how its notes look. Round ones use your skin.", "形: ノーツの見た目。丸はスキンを使います。", "Форма: как выглядят ноты. Круглые берутся из скина.").to_string(),
                    2 => tr!(
                        "Speed: how fast its notes scroll, times your scroll speed. The timing is the map's either way.",
                        "速度: ノーツが流れる速さ (スクロール速度に掛けます)。タイミングはどちらでも譜面のまま。",
                        "Скорость: как быстро летят ноты, умноженная на вашу. Тайминг в любом случае как в карте."
                    )
                    .to_string(),
                    c => format!("{}: {}", MODS[c - 3].name(), MODS[c - 3].description()),
                };
                (about, 0x9a9ab0)
            }
        };
        let col = if self.bad { 0xFF4B4B } else { col };
        c.text(
            px + 16.0,
            line_y,
            7.0,
            rgba(col, 1.0),
            &Canvas::fit_end(7.0, pw - 32.0, &line),
        );

        // Presets: the built-in ones and the player's, the one the universes are now lit; the
        // player's have an x to delete them.
        let (by, bh, bw) = if crate::mobile::active() {
            (py + ph - 46.0, 38.0, 96.0)
        } else {
            (py + ph - 26.0, 16.0, 56.0)
        };
        c.button(
            px + pw - bw - 12.0,
            by,
            bw,
            bh,
            tr!("Back", "戻る", "Назад"),
            7.0,
            Hit::Key(KeyCode::Escape),
            true,
        );
        c.button(
            px + pw - 2.0 * bw - 18.0,
            by,
            bw,
            bh,
            tr!("Shuffle", "シャッフル", "Смешать"),
            7.0,
            Hit::Key(KeyCode::KeyR),
            false,
        );
        c.button(
            px + pw - 3.0 * bw - 24.0,
            by,
            bw,
            bh,
            tr!("+ Save", "+ 保存", "+ Сохр."),
            7.0,
            Hit::Key(KeyCode::KeyS),
            false,
        );
        // Own colours are a key on a monitor.
        let own_w = crate::mobile::pick(0.0, 124.0);
        if crate::mobile::active() {
            let own_label = tr!("Own colours", "自分の色", "Свои цвета");
            let own_x = px + pw - 3.0 * bw - 30.0 - own_w;
            c.button(
                own_x,
                by,
                own_w,
                bh,
                own_label,
                7.0,
                Hit::Key(KeyCode::KeyE),
                false,
            );
        }
        let room_end = px + pw - 3.0 * bw - 32.0 - own_w;
        let mut x = px
            + 16.0
            + c.text(
                px + 16.0,
                by + (bh - 6.0) / 2.0,
                6.0,
                rgba(0x8a8aa0, 1.0),
                tr!("PRESETS", "プリセット", "ПРЕСЕТЫ"),
            )
            + 8.0;
        let current = Self::current(all, own);
        let list = presets(own);
        for (i, (name, _)) in list.iter().enumerate() {
            let mine = i >= BUILTIN_PRESETS;
            let name_w = Canvas::text_width(7.0, name).min(crate::mobile::pick(90.0, 130.0));
            let cw = name_w + 14.0 + if mine { 12.0 } else { 0.0 };
            if x + cw > room_end {
                c.text(
                    x,
                    by + (bh - 7.0) / 2.0,
                    7.0,
                    rgba(0x8a8aa0, 1.0),
                    &format!("+{}", list.len() - i),
                );
                break;
            }
            let on = current == Some(i);
            c.round_rect(
                [x, by],
                [cw, bh],
                4.0,
                rgba(if on { 0x6b2a4c } else { 0x1d1d27 }, 1.0),
                rgba(if on { 0xff5fa2 } else { 0x3a3a4e }, 1.0),
                1.0,
            );
            c.text(
                x + 7.0,
                by + (bh - 7.0) / 2.0,
                7.0,
                rgba(0xffffff, 1.0),
                &Canvas::fit(7.0, name_w, name),
            );
            c.hit(
                x,
                by,
                cw - if mine { 12.0 } else { 0.0 },
                bh,
                Hit::UniversePreset(i),
            );
            if mine {
                c.text(
                    x + cw - 11.0,
                    by + (bh - 7.0) / 2.0,
                    7.0,
                    rgba(0x8a8aa0, 1.0),
                    "x",
                );
                c.hit(x + cw - 13.0, by, 13.0, bh, Hit::UniversePresetDelete(i));
            }
            x += cw + 5.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    use crate::mods::Mod;

    #[test]
    fn a_universe_mod_switches_off_what_it_conflicts_with() {
        let mut all = classic();
        Setup::change(&mut all, 0, 3, 1);
        assert!(all[0].mods.has(Mod::Hidden));
        // Sudden: Hidden goes (together they'd hide every note).
        Setup::change(&mut all, 0, 4, 1);
        assert!(all[0].mods.has(Mod::Sudden) && !all[0].mods.has(Mod::Hidden));
        // Back from classic: the player's own colours, then the last palette.
        Setup::change(&mut all, 0, 0, -1);
        assert_eq!(all[0].colours.name(), "custom");
        Setup::change(&mut all, 0, 0, -1);
        assert_eq!(all[0].colours.name(), "swapped");
    }

    #[test]
    fn presets_save_load_cycle_and_delete() {
        let (mut all, mut own, mut s) = (defaults(), Vec::new(), Setup::new(0, 0));
        let typed =
            |s: &mut Setup, all: &mut [Universe; COUNT], own: &mut Vec<Preset>, text: &str| {
                s.key(KeyCode::KeyA, Some(text), false, all, own, 4);
                s.key(KeyCode::Enter, None, false, all, own, 4)
            };
        // Own colours for universe 1, then saved as a preset.
        s.key(KeyCode::KeyE, None, false, &mut all, &mut own, 4);
        assert_eq!(s.typing, Some(Typing::Colours("#ff3fb4 #2fe0ff".into())));
        s.typing = Some(Typing::Colours(String::new()));
        assert_eq!(
            typed(&mut s, &mut all, &mut own, "#102030 #a0b0c0"),
            Action::Changed
        );
        assert_eq!(all[0].colours, Colours::own(0x102030, 0xa0b0c0));
        s.key(KeyCode::KeyS, None, false, &mut all, &mut own, 4);
        assert_eq!(typed(&mut s, &mut all, &mut own, "mine"), Action::Changed);
        assert_eq!(own.len(), 1);
        assert_eq!(Setup::current(&all, &own), Some(BUILTIN_PRESETS));
        // ] goes round: defaults, classic, mine.
        s.key(KeyCode::BracketRight, None, false, &mut all, &mut own, 4);
        assert_eq!(all, defaults());
        s.key(KeyCode::BracketRight, None, false, &mut all, &mut own, 4);
        assert_eq!(all, classic());
        s.key(KeyCode::BracketRight, None, false, &mut all, &mut own, 4);
        assert_eq!(all, own[0].universes);
        // The built-in ones stay; the player's go.
        assert_eq!(Setup::delete(&mut own, 0), Action::None);
        s.key(KeyCode::Delete, None, false, &mut all, &mut own, 4);
        assert!(own.is_empty());
        // A bad colour or an empty name is refused, and typing goes on.
        s.key(KeyCode::KeyE, None, false, &mut all, &mut own, 4);
        s.typing = Some(Typing::Colours(String::new()));
        assert_eq!(typed(&mut s, &mut all, &mut own, "red"), Action::None);
        assert!(s.bad && s.typing.is_some());
    }
}
