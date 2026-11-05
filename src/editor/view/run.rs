//! Running a command by name: what the keys, the menus and the buttons all end up calling.

use super::cmd::{Cmd, Mode};
use super::keys::KAct;
use super::lines::TAct;
use super::{Btn, Drag, Keys, Modal, RATES, View};
use crate::audio::Engine;
use crate::editor::chart::{SOUND_CLAP, SOUND_FINISH, SOUND_WHISTLE};
use crate::editor::state::Editor;
use crate::editor::timing;

impl View {
    /// Set when the editor should close; the screen asks.
    pub fn take_exit(&mut self) -> bool {
        std::mem::take(&mut self.exit)
    }

    pub(super) fn set_mode(&mut self, m: Mode) {
        self.mode = m;
        match m {
            Mode::Gimmicks if self.g_page > 1 => self.g_page = 0,
            Mode::Effects if self.g_page < 2 => self.g_page = 2,
            _ => {}
        }
        self.roll_start = None;
        if matches!(
            self.drag,
            Drag::Key { .. } | Drag::Node { .. } | Drag::Wire { .. }
        ) {
            self.drag = Drag::None;
        }
    }

    fn set_rate(&mut self, r: u32, audio: Option<&Engine>) {
        self.stop(audio);
        self.rate = r;
        self.want_rate = Some(r as f64 / 100.0);
    }

    fn step_rate(&mut self, dir: i32, fine: bool, audio: Option<&Engine>) {
        let to = if fine {
            (self.rate as i32 + dir * 5).clamp(25, 100) as u32
        } else {
            let at = RATES.iter().position(|r| *r >= self.rate).unwrap_or(3) as i32;
            RATES[(at + dir).clamp(0, RATES.len() as i32 - 1) as usize]
        };
        if to != self.rate {
            self.set_rate(to, audio);
        }
    }

    fn leave(&mut self, audio: Option<&Engine>) {
        self.stop(audio);
        if self.ed.modified() {
            self.modal = Some(Modal::Leave);
        } else {
            self.exit = true;
        }
    }

    fn revert(&mut self) {
        if !self.ed.modified() {
            self.status = "nothing to revert: no changes since the last save".into();
            return;
        }
        if !self.revert_armed {
            self.revert_armed = true;
            self.status = "Revert again to throw away every change since the last save".into();
            return;
        }
        self.revert_armed = false;
        match Editor::open(self.ed.path.clone()) {
            Ok(mut ed) => {
                let (snap, tool, time) = (self.ed.snap, self.ed.tool, self.ed.time);
                ed.snap = snap;
                ed.tool = tool;
                ed.time = time;
                self.ed = ed;
                self.select_line(None);
                self.n_sel = None;
                self.k_sel = None;
                self.g_pending = false;
                self.mark_stale();
                self.status = "reverted to the last save".into();
            }
            Err(e) => self.status = format!("can't revert: {e}"),
        }
    }

    fn seek_line(&mut self, forward: bool, audio: Option<&Engine>) {
        let t = self.ed.time;
        let pts = &self.ed.chart.timing;
        let to = if forward {
            pts.iter().map(|p| p.time).find(|pt| *pt > t + 1.0)
        } else {
            pts.iter().rev().map(|p| p.time).find(|pt| *pt < t - 1.0)
        };
        match to {
            Some(to) => {
                self.stop(audio);
                self.set_time(to);
            }
            None => self.status = "no more timing lines that way".into(),
        }
    }

    fn sound(&mut self, bit: i32) {
        let had = !self.ed.selection.is_empty();
        let on = self.ed.toggle_sound(bit);
        let what = match bit {
            SOUND_FINISH => "big",
            SOUND_WHISTLE | SOUND_CLAP => "kat",
            _ => "",
        };
        self.status = match (had, on) {
            (true, true) => format!("selected notes: {what}"),
            (true, false) => format!("selected notes: not {what}"),
            (false, true) => format!("next notes you place are {what}"),
            (false, false) => format!("next notes you place are not {what}"),
        };
        self.mark_stale();
    }

    pub(super) fn run(&mut self, cmd: Cmd, audio: Option<&Engine>) {
        self.menu = None;
        if cmd != Cmd::Revert {
            self.revert_armed = false;
        }
        let t = self.ed.time;
        match cmd {
            Cmd::Save => self.save(),
            Cmd::Revert => self.revert(),
            Cmd::Leave => self.leave(audio),
            Cmd::LeaveSave => {
                self.modal = None;
                self.save();
                if !self.ed.modified() {
                    self.exit = true;
                }
            }
            Cmd::LeaveDiscard => {
                self.modal = None;
                self.exit = true;
            }
            Cmd::Stay => self.modal = None,
            Cmd::Undo => {
                if self.ed.undo() {
                    self.mark_stale();
                } else {
                    self.status = "nothing to undo".into();
                }
            }
            Cmd::Redo => {
                if self.ed.redo() {
                    self.mark_stale();
                } else {
                    self.status = "nothing to redo".into();
                }
            }
            Cmd::Cut if self.mode == Mode::Timing => self.run(Cmd::CopyNotes, audio),
            Cmd::Cut => {
                let n = self.ed.cut();
                self.status = format!("cut {n}");
                self.mark_stale();
            }
            Cmd::CopyNotes if self.mode == Mode::Timing => {
                self.press_line_action(TAct::Copy);
            }
            Cmd::CopyNotes => {
                let n = self.ed.copy();
                self.status = format!("copied {n}");
            }
            Cmd::Paste if self.mode == Mode::Timing => self.press_line_action(TAct::Paste),
            Cmd::Paste => {
                let at = self.ed.snapped(t);
                let n = self.ed.paste(at);
                self.status = format!("pasted {n}");
                self.mark_stale();
            }
            Cmd::SelectAll if self.mode == Mode::Timing => {
                let all: Vec<u64> = self.ed.chart.timing.iter().map(|p| p.id).collect();
                self.sel_line = all.last().copied();
                self.sel_extra = all.into_iter().collect();
            }
            Cmd::SelectAll => self.ed.select_all(),
            Cmd::Delete if self.mode == Mode::Timing && !self.selected_lines().is_empty() => {
                self.press_line_action(TAct::Delete);
            }
            Cmd::Delete => {
                if self.node_mode() {
                    self.delete_selected_node();
                } else if self.keyframe_mode() && self.k_sel.is_some() {
                    self.press_key_action(KAct::DeleteKey, false, false);
                } else {
                    let n = self.ed.delete_selected();
                    if n > 0 {
                        self.status = format!("deleted {n}");
                        self.mark_stale();
                    }
                }
            }
            Cmd::Duplicate => {
                let n = self.ed.clone_selected();
                self.status = if n == 0 {
                    "nothing cloned: select notes first (and there must be room)".into()
                } else {
                    format!("cloned {n}")
                };
                self.mark_stale();
            }
            Cmd::Reverse => {
                match self.ed.reverse_selected() {
                    Ok(n) => self.status = format!("reversed {n}"),
                    Err(e) => self.status = e,
                }
                self.mark_stale();
            }
            Cmd::Flip => {
                self.ed.flip_selected();
                self.mark_stale();
            }
            Cmd::Nudge(fwd) => {
                if self.ed.nudge_selected(fwd) {
                    self.mark_stale();
                }
            }
            Cmd::Tool(tool) => {
                self.ed.tool = tool;
                self.roll_start = None;
                if self.mode != Mode::Compose {
                    self.set_mode(Mode::Compose);
                }
            }
            Cmd::Whistle => self.sound(SOUND_WHISTLE),
            Cmd::Clap => self.sound(SOUND_CLAP),
            Cmd::Finish => self.sound(SOUND_FINISH),
            Cmd::Play => self.toggle_play(audio),
            Cmd::PlayFromStart => {
                self.stop(audio);
                self.set_time(0.0);
                self.start(audio);
            }
            Cmd::FirstNote => {
                let to = self.ed.chart.objects.first().map_or(0.0, |o| o.time);
                self.stop(audio);
                self.set_time(to);
            }
            Cmd::LastNote => {
                let to = self.ed.chart.objects.last().map_or(0.0, |o| o.time);
                self.stop(audio);
                self.set_time(to);
            }
            Cmd::Step(n) => {
                let mut to = t;
                for _ in 0..n.unsigned_abs() {
                    to = timing::step(&self.ed.chart.timing, to, n > 0, self.ed.snap);
                }
                self.stop(audio);
                self.set_time(to);
            }
            Cmd::PrevLine => self.seek_line(false, audio),
            Cmd::NextLine => self.seek_line(true, audio),
            Cmd::ZoomIn | Cmd::ZoomOut => {
                let [x, _, w, _] = self.layout.timeline;
                let at = if self.playing.is_some() {
                    self.x_of(self.ed.time)
                } else {
                    x + w / 2.0
                };
                self.zoom(if cmd == Cmd::ZoomIn { 0.8 } else { 1.25 }, at);
            }
            Cmd::ZoomFit => {
                let w = self.timeline_w() as f64;
                self.ms_per_px = (self.end_ms() / w.max(1.0)).clamp(0.25, 80.0);
                self.view_start = 0.0;
            }
            Cmd::SnapStep(d) => self.step_snap(d),
            Cmd::SnapTo(n) => self.ed.snap = n,
            Cmd::RateStep(d) => self.step_rate(d, false, audio),
            Cmd::RateFine(d) => self.step_rate(d, true, audio),
            Cmd::Bookmark => {
                self.status = if self.ed.add_bookmark(t) {
                    format!("bookmarked {}", super::fmt_time(t))
                } else {
                    "already bookmarked".into()
                };
            }
            Cmd::Unbookmark => {
                self.status = if self.ed.remove_bookmark(t, 500.0) {
                    "bookmark removed".into()
                } else {
                    "no bookmark within half a second of the playhead".into()
                };
            }
            Cmd::PrevBookmark | Cmd::NextBookmark => {
                let to = if cmd == Cmd::NextBookmark {
                    self.ed.bookmark_after(t)
                } else {
                    self.ed.bookmark_before(t)
                };
                match to {
                    Some(to) => {
                        self.stop(audio);
                        self.set_time(to);
                    }
                    None => self.status = "no more bookmarks that way".into(),
                }
            }
            Cmd::AddRed => self.press(Btn::AddRed, Keys::default(), audio),
            Cmd::AddGreen => self.press(Btn::AddGreen, Keys::default(), audio),
            Cmd::DeleteLine => match self.ed.delete_timing_at(t) {
                Ok(()) => {
                    self.select_line(None);
                    self.mark_stale();
                }
                Err(e) => match self.sel_line {
                    Some(id) => self.delete_line(id),
                    None => self.status = e,
                },
            },
            Cmd::TimingList => {
                self.list = !self.list;
                self.list_scroll = 0;
                self.sheet = false;
            }
            Cmd::Mode(m) => self.set_mode(m),
            Cmd::Tour => self.start_tour(),
            Cmd::Shortcuts => {
                self.sheet = !self.sheet;
                self.list = false;
            }
        }
    }
}
