//! Shapeshift's setup panel: which lane shapes, how often and how fast they morph.

use crate::hit::{Canvas, Hit};
use crate::render::{accent, rgba};
use crate::shapeshift::*;
use winit::keyboard::KeyCode;

/// Shape `i`'s name, by the index `game::SHAPESHIFT` uses.
fn shape_name(i: usize) -> &'static str {
    match i {
        0 => tr!("Zigzag", "ジグザグ", "Зигзаг"),
        1 => tr!("U-Turn", "Uターン", "Разворот"),
        2 => tr!("Spiral", "らせん", "Спираль"),
        3 => tr!("Loop", "ループ", "Петля"),
        4 => tr!("Elbow", "L字", "Колено"),
        6 => tr!("Meadow", "草原", "Луг"),
        _ => tr!("Straight", "まっすぐ", "Прямая"),
    }
}

/// Morph `i`'s name, by its index in `MORPH`.
fn morph_name(i: usize) -> &'static str {
    match i {
        0 => tr!("1/2 bar", "1/2小節", "1/2 такта"),
        1 => tr!("1 bar", "1小節", "1 такт"),
        2 => tr!("2 bars", "2小節", "2 такта"),
        _ => tr!("4 bars", "4小節", "4 такта"),
    }
}

/// The rows of the setup.
pub(crate) const ROWS: usize = 4;

pub(crate) const SHAPES_ROW: usize = 3;

pub(crate) fn row_len(row: usize) -> usize {
    match row {
        0 => EVERY.len(),
        1 => MORPH.len(),
        2 => 2,
        _ => KEYS.len(),
    }
}

/// The Shapeshift setup (a tab of the mod customization): a row per setting, a chip per choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub row: usize,
    pub col: usize,
}

impl Setup {
    /// Picks the choice at `row`, `col`; on the shapes row it switches the shape on or off.
    pub fn pick(c: &mut Config, row: usize, col: usize) {
        match row {
            0 => c.every = col.min(EVERY.len() - 1),
            1 => c.morph = col.min(MORPH.len() - 1),
            2 => c.shuffle = col == 0,
            _ => {
                let s = &mut c.shapes[col.min(KEYS.len() - 1)];
                s.1 = !s.1;
            }
        }
    }

    /// Moves the selected shape `dir` places along the order. Returns whether it moved.
    pub fn move_shape(&mut self, c: &mut Config, dir: i32) -> bool {
        let to = self.col as i32 + dir;
        if self.row != SHAPES_ROW || !(0..KEYS.len() as i32).contains(&to) {
            return false;
        }
        c.shapes.swap(self.col, to as usize);
        self.col = to as usize;
        true
    }

    /// Handles a key. Returns whether the setup changed (save it).
    pub fn key(&mut self, code: KeyCode, shift: bool, c: &mut Config) -> bool {
        match code {
            KeyCode::ArrowUp => self.row = (self.row + ROWS - 1) % ROWS,
            KeyCode::ArrowDown => self.row = (self.row + 1) % ROWS,
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let dir = if code == KeyCode::ArrowLeft { -1 } else { 1 };
                if shift {
                    return self.move_shape(c, dir);
                }
                let n = row_len(self.row) as i32;
                self.col = (self.col.min(n as usize - 1) as i32 + dir).rem_euclid(n) as usize;
                return false;
            }
            KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter => {
                Self::pick(c, self.row, self.col.min(row_len(self.row) - 1));
                return true;
            }
            KeyCode::KeyD => {
                *c = Config::default();
                return true;
            }
            _ => {}
        }
        self.col = self.col.min(row_len(self.row) - 1);
        false
    }

    /// Draws the setup over the mod select, like the Multiverse one.
    pub fn draw_panel(
        &self,
        c: &mut Canvas,
        cfg: &Config,
        outline: impl Fn(usize) -> Vec<(f32, f32)>,
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
            rgba(0x5ee6c8, 0.9),
            1.5,
        );
        let title_w = c.text(
            px + 16.0,
            py + 12.0,
            12.0,
            rgba(0xffffff, 1.0),
            "SHAPESHIFT",
        );
        let hint = tr!(
            "arrows choose   Space pick   Shift + Left/Right move a shape   D defaults   Tab next tab   Esc back",
            "矢印 選択   Space 決定   Shift + 左右 形を移動   D 初期設定   Tab 次のタブ   Esc 戻る",
            "стрелки выбор   Space отметить   Shift + влево/вправо двигать   D сброс   Tab вкладка   Esc назад"
        );
        c.text(
            px + 26.0 + title_w,
            py + 16.0,
            6.0,
            rgba(0x6f6f86, 1.0),
            &crate::mobile::key_hint(6.0, pw - title_w - 42.0, hint),
        );
        self.draw(c, cfg, px, py + 40.0, pw, outline);
        let (by, bw, bh, size) = crate::mobile::setup_button(py + ph);
        c.button(
            px + pw - bw - 12.0,
            by,
            bw,
            bh,
            tr!("Back", "戻る", "Назад"),
            size,
            Hit::Key(KeyCode::Escape),
            true,
        );
        c.button(
            px + pw - 2.0 * bw - 18.0,
            by,
            bw,
            bh,
            tr!("Defaults", "初期設定", "Сброс"),
            size,
            Hit::Key(KeyCode::KeyD),
            false,
        );
    }

    /// Draws the setup's rows in the panel `px, py, pw` wide (below its title).
    /// `outline(shape)` gives a shape's lane, as points to draw a thumbnail with.
    pub fn draw(
        &self,
        c: &mut Canvas,
        cfg: &Config,
        px: f32,
        py: f32,
        pw: f32,
        outline: impl Fn(usize) -> Vec<(f32, f32)>,
    ) {
        let x0 = px + 16.0;
        let label_w = crate::mobile::pick(70.0, 100.0);
        let mut hovered = None;
        let mut y = py;
        for row in 0..ROWS {
            let h = if row == SHAPES_ROW {
                76.0
            } else {
                crate::mobile::pick(24.0, 42.0)
            };
            let name = [
                tr!("EVERY", "間隔", "КАЖДЫЕ"),
                tr!("MORPH", "変形", "ПЕРЕХОД"),
                tr!("ORDER", "順番", "ПОРЯДОК"),
                tr!("SHAPES", "形", "ФОРМЫ"),
            ][row];
            c.text(x0, y + h / 2.0 - 4.0, 7.0, rgba(0x8a8aa0, 1.0), name);
            let n = row_len(row);
            let room = px + pw - 16.0 - (x0 + label_w);
            let cw = ((room - 6.0 * (n as f32 - 1.0)) / n as f32).min(if row == SHAPES_ROW {
                110.0
            } else {
                crate::mobile::pick(90.0, 130.0)
            });
            for col in 0..n {
                let cx = x0 + label_w + col as f32 * (cw + 6.0);
                let selected = self.row == row && self.col == col;
                let on = match row {
                    0 => cfg.every == col,
                    1 => cfg.morph == col,
                    2 => cfg.shuffle == (col == 0),
                    _ => cfg.shapes[col].1,
                };
                if c.hovered(cx, y, cw, h) {
                    hovered = Some((row, col));
                }
                let fill = if on {
                    0x1f4a44
                } else if selected {
                    0x34344a
                } else {
                    0x1d1d27
                };
                let border = if selected {
                    accent()
                } else if on {
                    0x5ee6c8
                } else {
                    0x2a2a38
                };
                c.round_rect(
                    [cx, y],
                    [cw, h],
                    4.0,
                    rgba(fill, 1.0),
                    rgba(border, 1.0),
                    if selected { 1.5 } else { 1.0 },
                );
                let text = match row {
                    0 => {
                        let n = EVERY[col];
                        let bars_ru = crate::lang::ru_plural(n, ["такт", "такта", "тактов"]);
                        tr!("{n} bars", "{n}小節", "{n} {bars_ru}";)
                    }
                    1 => morph_name(col).to_string(),
                    2 => [
                        tr!("Shuffled", "シャッフル", "Перемешать"),
                        tr!("In order", "順番どおり", "По порядку"),
                    ][col]
                        .to_string(),
                    _ => format!("{}. {}", col + 1, shape_name(cfg.shapes[col].0 as usize)),
                };
                let text_col = if on { 0xffffff } else { 0x8a8aa0 };
                if row == SHAPES_ROW {
                    c.text_centered(
                        cx + cw / 2.0,
                        y + h - 13.0,
                        7.0,
                        rgba(text_col, 1.0),
                        &Canvas::fit(7.0, cw - 6.0, &text),
                    );
                    thumbnail(
                        c,
                        &outline(cfg.shapes[col].0 as usize),
                        cx + 4.0,
                        y + 4.0,
                        cw - 8.0,
                        h - 22.0,
                        if on { 0x5ee6c8 } else { 0x4a4a5e },
                    );
                } else {
                    c.text_centered(
                        cx + cw / 2.0,
                        y + h / 2.0 - 3.5,
                        7.0,
                        rgba(text_col, 1.0),
                        &Canvas::fit(7.0, cw - 6.0, &text),
                    );
                }
                c.hit(cx, y, cw, h, Hit::ShapeshiftCell(row, col));
            }
            y += h + 10.0;
        }
        // Moving the selected shape along the order, by mouse.
        if self.row == SHAPES_ROW {
            let bx = x0 + label_w;
            c.button(
                bx,
                y,
                crate::mobile::pick(70.0, 120.0),
                crate::mobile::pick(16.0, 36.0),
                tr!("< earlier", "< 前へ", "< раньше"),
                7.0,
                Hit::ShapeshiftMove(-1),
                false,
            );
            c.button(
                bx + crate::mobile::pick(76.0, 128.0),
                y,
                crate::mobile::pick(70.0, 120.0),
                crate::mobile::pick(16.0, 36.0),
                tr!("later >", "後へ >", "позже >"),
                7.0,
                Hit::ShapeshiftMove(1),
                false,
            );
        }
        let bars = cfg.bars();
        let morph_note = if cfg.morph_bars() < MORPH[cfg.morph] {
            tr!(" (at most half of {bars} bars)", " (最大で{bars}小節の半分)", " (не больше половины из {bars})";)
        } else {
            String::new()
        };
        let about = match hovered.unwrap_or((self.row, self.col)).0 {
            0 => tr!("Every: how many bars the lane keeps one shape.", "間隔: レーンが一つの形を保つ小節数。", "Каждые: сколько тактов дорожка держит одну форму.").to_string(),
            1 => tr!(
                "Morph: how long the lane takes to bend into the next shape{morph_note}.",
                "変形: レーンが次の形に曲がるまでの長さ{morph_note}。",
                "Переход: сколько дорожка изгибается в следующую форму{morph_note}.";
            ),
            2 => tr!(
                "Order: Shuffled gives each map its own order (the same every time you play it); In order goes along the shapes below.",
                "順番: シャッフルは譜面ごとに決まった順番 (毎回同じ)。順番どおりは下の形の順に進みます。",
                "Порядок: Перемешать даёт каждой карте свой порядок (всегда один и тот же); По порядку идёт по формам ниже."
            )
            .to_string(),
            _ => tr!(
                "Shapes: click or Space to leave one out; Shift + Left/Right moves the selected one earlier or later.",
                "形: クリックかSpaceで外す。Shift + 左右で選んだ形を前後に動かします。",
                "Формы: клик или Space убирает форму; Shift + влево/вправо двигает выбранную раньше или позже."
            )
            .to_string(),
        };
        c.text(
            x0,
            y + 26.0,
            7.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit_end(7.0, pw - 32.0, &about),
        );
    }
}

/// A shape's lane in a `w` x `h` box: its points scaled to fit, as dots.
pub(crate) fn thumbnail(
    c: &mut Canvas,
    pts: &[(f32, f32)],
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    col: u32,
) {
    if pts.is_empty() {
        return;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(px, py) in pts {
        (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
    }
    let k = (w / (x1 - x0).max(1.0)).min(h / (y1 - y0).max(1.0));
    let (ox, oy) = (x + (w - (x1 - x0) * k) / 2.0, y + (h - (y1 - y0) * k) / 2.0);
    for &(px, py) in pts {
        c.circle(
            ox + (px - x0) * k,
            oy + (py - y0) * k,
            1.2,
            rgba(col, 1.0),
            rgba(0, 0.0),
            0.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    #[test]
    fn keys_pick_toggle_and_move_shapes() {
        let mut c = Config::default();
        let mut s = Setup::default();
        assert!(!s.key(KeyCode::ArrowRight, false, &mut c));
        assert!(s.key(KeyCode::Space, false, &mut c));
        assert_eq!(c.bars(), 8);
        s = Setup {
            row: SHAPES_ROW,
            col: 0,
        };
        assert!(s.key(KeyCode::ArrowRight, true, &mut c));
        assert_eq!((c.shapes[1].0, s.col), (0, 1));
        assert!(s.key(KeyCode::Space, false, &mut c));
        assert!(!c.shapes[1].1);
        // Not past the ends.
        s.col = 0;
        assert!(!s.key(KeyCode::ArrowLeft, true, &mut c));
    }
}
