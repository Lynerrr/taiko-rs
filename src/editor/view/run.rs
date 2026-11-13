//! Running a command by name: what the keys, the menus and the buttons all end up calling.

use super::cmd::{Cmd, Mode};
use super::keys::KAct;
use super::lines::TAct;
use super::{Btn, Drag, Keys, Modal, RATES, View};
use crate::audio::Engine;
use crate::editor::chart::SOUND_FINISH;
use crate::editor::state::{Editor, Tool};
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
            Mode::Effects if self.g_page < 2 => self.g_page = 4,
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

    pub(super) fn set_rate(&mut self, r: u32, audio: Option<&Engine>) {
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

    /// The File menu's new difficulty: this one is saved first (the new one is made from the
    /// file), then the screen makes it and opens it. Not in a mapping session: its difficulties
    /// are the session's, and leaving here would leave it.
    fn new_difficulty(&mut self, keep_notes: bool, audio: Option<&Engine>) {
        if self.collab.is_some() {
            self.status = "a new difficulty can't be made during a mapping session: the host's set is shared as it was opened".into();
            return;
        }
        self.stop(audio);
        if let Err(e) = self.save_file() {
            self.status = format!("can't save: {e}");
            return;
        }
        self.want_new_diff = Some(keep_notes);
    }

    fn leave(&mut self, audio: Option<&Engine>) {
        self.stop(audio);
        // A value still being typed is a change to ask about (and to save), not one to lose.
        self.commit_typing();
        // In a session our edits went out already, and the others' are only here: the whole
        // set is saved as the session has it, and the game offers to go back in.
        if self.collab.is_some() {
            match self.save_file() {
                Ok(_) => {
                    self.left_session = self.session_to_return();
                    self.exit = true;
                }
                Err(e) => self.status = format!("can't save: {e}"),
            }
            return;
        }
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

    /// Kat (`None`) or big (the finish bit), on the selection or on the note in hand.
    fn sound(&mut self, bit: Option<i32>) {
        // With the Note tool the keys are for the note about to be placed, even with a note
        // selected (the one just placed is): recolouring a placed note is a click on it.
        let had = !self.ed.selection.is_empty() && self.ed.tool != Tool::Note;
        let on = match (bit, had) {
            (Some(bit), true) => self.ed.toggle_sound(bit),
            (None, true) => self.ed.toggle_kat(),
            (bit, false) => self
                .ed
                .toggle_next_sound(bit.unwrap_or(crate::editor::chart::SOUND_WHISTLE)),
        };
        let what = if bit.is_some() { "big" } else { "kat" };
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
            Cmd::SongSetup => self.open_setup(),
            Cmd::Upload => self.open_upload(),
            Cmd::Export => self.export(),
            Cmd::NewDifficulty(keep) => self.new_difficulty(keep, audio),
            Cmd::OpenFolder => self.show = Some(self.ed.path.clone()),
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
                // A key is picked by its place in the track, which undo may change; a drag
                // going on would go on editing what was undone.
                self.drag = Drag::None;
                self.k_sel = None;
                self.m_key = None;
                if self.ed.undo() {
                    self.mark_stale();
                } else {
                    self.status = "nothing to undo".into();
                }
            }
            Cmd::Redo => {
                self.drag = Drag::None;
                self.k_sel = None;
                self.m_key = None;
                if self.ed.redo() {
                    self.mark_stale();
                } else {
                    self.status = "nothing to redo".into();
                }
            }
            Cmd::Cut if self.line_keys(false) => self.run(Cmd::CopyNotes, audio),
            Cmd::Cut if self.clip_mode() && self.picked().is_some() => self.copy_clips(true),
            Cmd::Cut => {
                self.k_copied = None;
                let n = self.ed.cut();
                self.status = format!("cut {n}");
                self.mark_stale();
            }
            Cmd::CopyNotes if self.line_keys(false) => {
                self.press_line_action(TAct::Copy);
            }
            Cmd::CopyNotes if self.clip_mode() && self.picked().is_some() => self.copy_clips(false),
            // On the key graph: the key picked, or every key of the track when no notes are
            // picked either.
            Cmd::CopyNotes
                if self.keyframe_mode()
                    && (self.k_sel.is_some() || self.ed.selection.is_empty())
                    && self.copy_keys() => {}
            Cmd::CopyNotes => {
                self.k_copied = None;
                let n = self.ed.copy();
                self.status = format!("copied {n}");
            }
            Cmd::Paste if self.line_keys(true) => self.press_line_action(TAct::Paste),
            Cmd::Paste if self.clip_mode() && View::has_copied_clips() => self.paste_copied_clips(),
            Cmd::Paste if self.keyframe_mode() && self.k_copied.is_some() => self.paste_keys(),
            Cmd::Paste => {
                let at = self.ed.snapped(t);
                let n = self.ed.paste(at);
                self.status = format!("pasted {n}");
                self.mark_stale();
            }
            Cmd::SelectAll if self.mode == Mode::Timing || self.tw => {
                let all: Vec<u64> = self.ed.chart.timing.iter().map(|p| p.id).collect();
                self.sel_line = all.last().copied();
                self.sel_extra = all.into_iter().collect();
            }
            Cmd::SelectAll if self.clip_mode() => self.pick_all_clips(),
            Cmd::SelectAll => self.ed.select_all(),
            Cmd::SelectNone if self.mode == Mode::Timing || self.tw => self.select_line(None),
            Cmd::SelectNone => self.ed.select_none(),
            Cmd::Delete
                if (self.mode == Mode::Timing || self.tw) && !self.selected_lines().is_empty() =>
            {
                self.press_line_action(TAct::Delete);
            }
            Cmd::Delete if self.clip_mode() && self.picked().is_some() => {
                self.press_fx(super::clips::FxAct::Delete);
            }
            Cmd::Duplicate if self.clip_mode() && self.picked().is_some() => {
                self.press_fx(super::clips::FxAct::Duplicate);
            }
            Cmd::Delete => {
                if self.node_mode() {
                    self.delete_selected_node();
                } else if self.keyframe_mode() && self.k_sel.is_some() {
                    self.press_key_action(KAct::DeleteKey);
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
                // Picking notes is wanted on every page (Motion, groups, clips over the notes);
                // only the tools that place notes take the editor to Compose.
                if tool != Tool::Select && self.mode != Mode::Compose {
                    self.set_mode(Mode::Compose);
                }
            }
            Cmd::Kat => self.sound(None),
            Cmd::Finish => self.sound(Some(SOUND_FINISH)),
            Cmd::Play => self.toggle_play(audio),
            Cmd::PlayFromStart => {
                self.stop(audio);
                self.set_time(0.0);
                self.start(audio);
            }
            Cmd::TestPlay(from_start) => {
                self.stop(audio);
                self.want_test = Some((!from_start).then_some(self.ed.time));
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
            Cmd::Waveform => {
                self.wave_on = !self.wave_on;
                self.status = if self.wave_on {
                    "waveform on".into()
                } else {
                    "waveform off".into()
                };
            }
            Cmd::Metronome => {
                self.metronome = !self.metronome;
                if self.playing.is_some() {
                    // The clicks are in the music: cut it out again from here.
                    self.reseek = true;
                }
                self.status = if !self.metronome {
                    "metronome off".into()
                } else if self.track.as_ref().is_some_and(|t| t.samples.is_empty()) {
                    "metronome on, but this song is streamed and can't carry the clicks".into()
                } else {
                    "metronome on: a click on every beat while the song plays".into()
                };
            }
            Cmd::FollowTiming => {
                self.ed.follow_timing = !self.ed.follow_timing;
                self.status = if self.ed.follow_timing {
                    "notes follow: a red line's new time or BPM moves its notes along".into()
                } else {
                    "notes stay put when a red line changes".into()
                };
            }
            Cmd::SnapToGrid => self.open_resnap(),
            Cmd::LeftPan => {
                self.left_pan = !self.left_pan;
                self.status = if self.left_pan {
                    "a left drag on empty space pans the timeline (Ctrl+drag selects a box)".into()
                } else {
                    "a left drag on empty space selects a box".into()
                };
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
            Cmd::EffectMark => {
                self.status = if !self.ed.is_tko() {
                    "Effect marks live in a .tko: make one on the Effects page first".into()
                } else if self.ed.add_mark(t) {
                    format!("mark added at {}", super::fmt_time(t))
                } else {
                    "there is a mark here already".into()
                };
            }
            Cmd::EffectUnmark => {
                self.status = if self.ed.remove_mark(t, 500.0) {
                    "mark removed".into()
                } else {
                    "no effect mark within half a second of the playhead".into()
                };
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
                self.tw = !self.tw;
                self.tw_seen = None;
                self.sheet = false;
                if self.tw && self.node_mode() {
                    self.set_mode(Mode::Timing);
                }
            }
            Cmd::Mode(m) => self.set_mode(m),
            Cmd::Tour => self.start_tour(),
            Cmd::Manual => self.open_manual(),
            Cmd::Shortcuts => {
                self.sheet = !self.sheet;
            }
        }
    }
}
