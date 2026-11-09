//! Tact's setup panel: the step the notes move in.

use crate::hit::{Canvas, Hit};
use crate::render::{accent, rgba};
use crate::tact::*;
use winit::keyboard::KeyCode;

/// The Tact setup (a tab of the mod customization): one row of steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub col: usize,
}

impl Setup {
    pub fn new(current: usize) -> Setup {
        Setup {
            col: current.min(STEPS.len() - 1),
        }
    }

    /// Handles a key. Returns the step picked, if one was (save it).
    pub fn key(&mut self, code: KeyCode) -> Option<usize> {
        let n = STEPS.len();
        match code {
            KeyCode::ArrowLeft => self.col = (self.col + n - 1) % n,
            KeyCode::ArrowRight => self.col = (self.col + 1) % n,
            KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter => return Some(self.col),
            KeyCode::KeyD => {
                self.col = DEFAULT;
                return Some(DEFAULT);
            }
            _ => {}
        }
        None
    }

    /// Draws the setup over the mod select, like Shapeshift's. `current`: the step played with.
    pub fn draw_panel(&self, c: &mut Canvas, current: usize) {
        const COLOUR: u32 = 0xFFD34D;
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
            rgba(COLOUR, 0.9),
            1.5,
        );
        let title_w = c.text(px + 16.0, py + 12.0, 12.0, rgba(0xffffff, 1.0), "TACT");
        let hint = tr!(
            "Left/Right choose   Space pick   D default   Tab next tab   Esc back",
            "左右 選択   Space 決定   D 初期設定   Tab 次のタブ   Esc 戻る",
            "влево/вправо выбор   Space отметить   D сброс   Tab вкладка   Esc назад"
        );
        c.text(
            px + 26.0 + title_w,
            py + 16.0,
            6.0,
            rgba(0x6f6f86, 1.0),
            &crate::mobile::key_hint(6.0, pw - title_w - 42.0, hint),
        );

        let phone = crate::mobile::active();
        let (x0, y, h) = if phone {
            (px + 16.0, py + 44.0, 48.0)
        } else {
            (px + 16.0, py + 40.0, 24.0)
        };
        let label_w = crate::mobile::pick(70.0, 130.0);
        c.text(
            x0,
            y + h / 2.0 - 4.0,
            7.0,
            rgba(0x8a8aa0, 1.0),
            tr!("MOVE EVERY", "移動", "ШАГ"),
        );
        let n = STEPS.len();
        let room = px + pw - 16.0 - (x0 + label_w);
        let cw = ((room - 6.0 * (n as f32 - 1.0)) / n as f32).min(crate::mobile::pick(90.0, 150.0));
        for col in 0..n {
            let cx = x0 + label_w + col as f32 * (cw + 6.0);
            let (selected, on) = (self.col == col, current == col);
            let fill = if on {
                0x4a3f18
            } else if selected {
                0x34344a
            } else {
                0x1d1d27
            };
            let border = if selected {
                accent()
            } else if on {
                COLOUR
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
            let text_col = if on { 0xffffff } else { 0x8a8aa0 };
            c.text_centered(
                cx + cw / 2.0,
                y + h / 2.0 - 3.5,
                7.0,
                rgba(text_col, 1.0),
                &Canvas::fit(7.0, cw - 6.0, step_name(col)),
            );
            c.hit(cx, y, cw, h, Hit::TactCell(col));
        }
        let about = tr!(
            "Far off, the notes crawl through each step, then rush the rest of the way at its end. Closer in they move as usual, so each one is in the hit circle on time. A shorter step rushes more often.",
            "遠くのノーツは各ステップでゆっくり進み、終わりに一気に残りを進みます。近づくといつも通りに動くので、どのノーツも時間通りに判定円に来ます。短いほど頻繁に加速します。",
            "Вдали ноты медленно ползут на каждом шаге, а в конце рывком проходят остаток. Ближе они движутся как обычно, так что каждая нота вовремя в круге удара. Чем короче шаг, тем чаще рывки."
        );
        let mut ty = y + h + 18.0;
        let chars = ((pw - 32.0) / Canvas::char_width(7.0)).max(10.0) as usize;
        for line in crate::social::wrap(about, chars) {
            c.text(x0, ty, 7.0, rgba(0x9a9ab0, 1.0), &line);
            ty += crate::mobile::pick(12.0, 20.0);
        }
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
            tr!("Default", "初期設定", "Сброс"),
            size,
            Hit::Key(KeyCode::KeyD),
            false,
        );
    }
}
