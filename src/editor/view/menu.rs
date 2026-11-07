//! The menu bar: every command in a menu with its key beside it, so nothing has to be found by
//! guessing.

use super::chrome::{DIM, EDGE};
use super::cmd::{Cmd, Mode};
use super::{Btn, View};
use crate::editor::state::Tool;
use crate::render::{Canvas, rgba};

pub enum Item {
    Do(Cmd),
    Sep,
}

pub struct Menu {
    pub name: &'static str,
    pub items: &'static [Item],
}

use Item::{Do, Sep};

pub const MENUS: &[Menu] = &[
    Menu {
        name: "File",
        items: &[
            Do(Cmd::Save),
            Do(Cmd::Revert),
            Sep,
            Do(Cmd::SongSetup),
            Do(Cmd::Upload),
            Do(Cmd::Export),
            Do(Cmd::OpenFolder),
            Sep,
            Do(Cmd::Leave),
        ],
    },
    Menu {
        name: "Edit",
        items: &[
            Do(Cmd::Undo),
            Do(Cmd::Redo),
            Sep,
            Do(Cmd::Cut),
            Do(Cmd::CopyNotes),
            Do(Cmd::Paste),
            Do(Cmd::SelectAll),
            Do(Cmd::SelectNone),
            Do(Cmd::Delete),
            Sep,
            Do(Cmd::Duplicate),
            Do(Cmd::Reverse),
            Do(Cmd::Flip),
            Do(Cmd::Nudge(false)),
            Do(Cmd::Nudge(true)),
            Sep,
            Do(Cmd::Tool(Tool::Select)),
            Do(Cmd::Tool(Tool::Note)),
            Do(Cmd::Tool(Tool::Roll)),
            Do(Cmd::Tool(Tool::Swell)),
            Sep,
            Do(Cmd::Kat),
            Do(Cmd::Finish),
        ],
    },
    Menu {
        name: "View",
        items: &[
            Do(Cmd::Mode(Mode::Compose)),
            Do(Cmd::Mode(Mode::Gimmicks)),
            Do(Cmd::Mode(Mode::Timing)),
            Do(Cmd::Mode(Mode::Effects)),
            Sep,
            Do(Cmd::ZoomIn),
            Do(Cmd::ZoomOut),
            Do(Cmd::ZoomFit),
            Do(Cmd::Waveform),
            Do(Cmd::LeftPan),
            Sep,
            Do(Cmd::SnapStep(1)),
            Do(Cmd::SnapStep(-1)),
            Do(Cmd::RateStep(-1)),
            Do(Cmd::RateStep(1)),
            Sep,
            Do(Cmd::Play),
            Do(Cmd::PlayFromStart),
            Do(Cmd::FirstNote),
            Do(Cmd::LastNote),
            Sep,
            Do(Cmd::TestPlay(false)),
            Do(Cmd::TestPlay(true)),
        ],
    },
    Menu {
        name: "Timing",
        items: &[
            Do(Cmd::AddRed),
            Do(Cmd::AddGreen),
            Do(Cmd::DeleteLine),
            Do(Cmd::TimingList),
            Do(Cmd::Metronome),
            Do(Cmd::PrevLine),
            Do(Cmd::NextLine),
            Sep,
            Do(Cmd::Bookmark),
            Do(Cmd::Unbookmark),
            Do(Cmd::PrevBookmark),
            Do(Cmd::NextBookmark),
            Sep,
            Do(Cmd::EffectMark),
            Do(Cmd::EffectUnmark),
        ],
    },
    Menu {
        name: "Help",
        items: &[Do(Cmd::Manual), Do(Cmd::Tour), Do(Cmd::Shortcuts)],
    },
];

const ROW: f32 = 13.0;

impl View {
    /// The open menu, drawn over everything else.
    pub(super) fn draw_menu_drop(&mut self, c: &mut Canvas) {
        let Some(open) = self.menu else { return };
        let Some(menu) = MENUS.get(open) else { return };
        let mut x = 4.0;
        for m in &MENUS[..open] {
            x += Canvas::text_width(7.0, m.name) + 14.0;
        }
        let width = menu
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Do(cmd) => Some(
                    Canvas::text_width(7.0, cmd.name())
                        + cmd
                            .hotkey()
                            .map_or(0.0, |k| Canvas::text_width(6.0, k) + 20.0),
                ),
                Item::Sep => None,
            })
            .fold(120.0, f32::max)
            + 20.0;
        let height: f32 = menu
            .items
            .iter()
            .map(|i| if matches!(i, Item::Sep) { 5.0 } else { ROW })
            .sum::<f32>()
            + 6.0;
        c.round_rect(
            [x, 14.0],
            [width, height],
            3.0,
            rgba(0x1b1b26, 1.0),
            rgba(0x4a4a66, 1.0),
            1.0,
        );
        let mut y = 17.0;
        for item in menu.items {
            match item {
                Item::Sep => {
                    c.rect(x + 6.0, y + 2.0, width - 12.0, 1.0, rgba(EDGE, 1.0));
                    y += 5.0;
                }
                Item::Do(cmd) => {
                    let r = [x + 2.0, y, width - 4.0, ROW];
                    if c.hovered(r[0], r[1], r[2], r[3]) {
                        c.rect(r[0], r[1], r[2], r[3], rgba(0x3a3a52, 1.0));
                    }
                    c.text(x + 10.0, y + 3.0, 7.0, rgba(0xffffff, 1.0), cmd.name());
                    if let Some(k) = cmd.hotkey() {
                        c.text_right(x + width - 8.0, y + 3.5, 6.0, rgba(DIM, 1.0), k);
                    }
                    self.buttons.push((r, Btn::C(*cmd)));
                    y += ROW;
                }
            }
        }
    }
}
