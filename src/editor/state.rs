//! What the editor does to a chart: placing, selecting, moving and copying notes, and editing
//! timing lines. Every change is one `Command` through the history, so it can be undone, and
//! every time it writes is a whole millisecond the way stable's editor writes it.

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::chart::{Chart, HitObject, Taiko, TimingPoint};
use super::history::{Command, History};
use super::timing;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Don,
    Kat,
    Roll,
    Swell,
}

pub struct Editor {
    pub chart: Chart,
    pub history: History,
    /// Where the chart saves to.
    pub path: PathBuf,
    /// Selected hit objects, by id.
    pub selection: BTreeSet<u64>,
    /// The playhead (map ms).
    pub time: f64,
    /// Beat division the grid and placement snap to.
    pub snap: u32,
    pub tool: Tool,
    /// Place the finisher version of a note.
    pub big: bool,
    clipboard: Vec<HitObject>,
}

impl Editor {
    pub fn new(chart: Chart, path: PathBuf) -> Editor {
        let time = chart
            .objects
            .first()
            .map_or(0.0, |o| (o.time - 2000.0).max(0.0));
        Editor {
            chart,
            history: History::new(),
            path,
            selection: BTreeSet::new(),
            time,
            snap: 4,
            tool: Tool::Select,
            big: false,
            clipboard: Vec::new(),
        }
    }

    pub fn open(path: PathBuf) -> Result<Editor, String> {
        Ok(Editor::new(Chart::read(&path)?, path))
    }

    pub fn modified(&self) -> bool {
        self.history.modified()
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.chart.save(&self.path, true)?;
        self.history.mark_saved();
        Ok(())
    }

    // ---- the grid ----

    /// The millisecond a click at `t` lands on with the current snap.
    pub fn snapped(&self, t: f64) -> f64 {
        timing::snapped_ms(&self.chart.timing, t, self.snap) as f64
    }

    pub fn sm(&self) -> f64 {
        self.chart.slider_multiplier()
    }

    /// Where an object ends, by the timing in force at its start.
    pub fn end_of(&self, o: &HitObject) -> f64 {
        o.end_time(
            self.sm(),
            timing::beat_length_at(&self.chart.timing, o.time),
            timing::sv_at(&self.chart.timing, o.time),
        )
    }

    /// The object whose body holds `t` (the nearest start for a note), within `slack` ms.
    pub fn object_at(&self, t: f64, slack: f64) -> Option<u64> {
        self.chart
            .objects
            .iter()
            .filter(|o| t >= o.time - slack && t <= self.end_of(o).max(o.time) + slack)
            .min_by(|a, b| (a.time - t).abs().total_cmp(&(b.time - t).abs()))
            .map(|o| o.id)
    }

    // ---- placing ----

    /// Puts a don or kat at `t` (made whole by the caller's snap). Nothing is put where a note
    /// already stands.
    pub fn place_hit(&mut self, t: f64, kat: bool) -> Option<u64> {
        let t = t.round();
        if t < 0.0 || self.chart.objects.iter().any(|o| o.time == t) {
            return None;
        }
        let o = HitObject::hit(t, kat, self.big);
        Some(self.run_insert(o))
    }

    /// Puts a drumroll from `start` to `end` (made whole), at the velocity in force at its start.
    pub fn place_roll(&mut self, start: f64, end: f64) -> Option<u64> {
        let (start, end) = (start.round(), end.round());
        if start < 0.0 || end <= start || self.chart.objects.iter().any(|o| o.time == start) {
            return None;
        }
        let length = HitObject::length_for(
            end - start,
            1,
            self.sm(),
            timing::beat_length_at(&self.chart.timing, start),
            timing::sv_at(&self.chart.timing, start),
        );
        Some(self.run_insert(HitObject::roll(start, length, self.big)))
    }

    pub fn place_swell(&mut self, start: f64, end: f64) -> Option<u64> {
        let (start, end) = (start.round(), end.round());
        if start < 0.0 || end <= start || self.chart.objects.iter().any(|o| o.time == start) {
            return None;
        }
        Some(self.run_insert(HitObject::swell(start, end)))
    }

    fn run_insert(&mut self, o: HitObject) -> u64 {
        self.history
            .run(&mut self.chart, Command::InsertObjects(vec![o]));
        // The one just made is the one at its time (ids are handed out in order).
        let id = self.chart.objects.iter().map(|o| o.id).max().unwrap_or(0);
        self.selection = BTreeSet::from([id]);
        id
    }

    // ---- selecting ----

    pub fn select_all(&mut self) {
        self.selection = self.chart.objects.iter().map(|o| o.id).collect();
    }

    pub fn select_none(&mut self) {
        self.selection.clear();
    }

    /// Selects what starts in `from..=to`; `add` keeps what was selected.
    pub fn select_range(&mut self, from: f64, to: f64, add: bool) {
        let (from, to) = (from.min(to), from.max(to));
        if !add {
            self.selection.clear();
        }
        let first = self.chart.objects.partition_point(|o| o.time < from);
        for o in self.chart.objects[first..]
            .iter()
            .take_while(|o| o.time <= to)
        {
            self.selection.insert(o.id);
        }
    }

    pub fn toggle(&mut self, id: u64) {
        if !self.selection.remove(&id) {
            self.selection.insert(id);
        }
    }

    fn selected(&self) -> Vec<HitObject> {
        self.chart
            .objects
            .iter()
            .filter(|o| self.selection.contains(&o.id))
            .cloned()
            .collect()
    }

    // ---- changing what's selected ----

    pub fn delete_selected(&mut self) -> usize {
        let gone = self.selected();
        let n = gone.len();
        if n > 0 {
            self.history
                .run(&mut self.chart, Command::RemoveObjects(gone));
            self.selection.clear();
        }
        n
    }

    /// Moves the selection `delta` ms (made whole), refusing to push anything before 0.
    /// `drag`: the step of a drag still going on, folded into one undo.
    pub fn move_selected(&mut self, delta: f64, drag: bool) -> bool {
        let before = self.selected();
        let delta = delta.round();
        let first = before.iter().map(|o| o.time).fold(f64::INFINITY, f64::min);
        if before.is_empty() || delta == 0.0 || first + delta < 0.0 {
            return false;
        }
        let after = before.iter().map(|o| o.shifted(delta)).collect();
        self.edit(before, after, drag);
        true
    }

    /// Makes the selected notes kat (`Some(true)`) or don, and big or small.
    pub fn retype_selected(&mut self, kat: Option<bool>, big: Option<bool>) {
        let before: Vec<HitObject> = self
            .selected()
            .into_iter()
            .filter(|o| o.taiko() != Taiko::Swell)
            .collect();
        let after: Vec<HitObject> = before
            .iter()
            .map(|o| {
                let mut o = o.clone();
                // Kat is a whistle or a clap, don neither (the finisher bit is kept apart).
                if let Some(kat) = kat {
                    o.sound &= !(super::chart::SOUND_WHISTLE | super::chart::SOUND_CLAP);
                    if kat {
                        o.sound |= super::chart::SOUND_WHISTLE;
                    }
                }
                if let Some(big) = big {
                    o.sound &= !super::chart::SOUND_FINISH;
                    if big {
                        o.sound |= super::chart::SOUND_FINISH;
                    }
                }
                o
            })
            .collect();
        if !before.is_empty() {
            self.edit(before, after, false);
        }
    }

    /// Flips each selected note between don and kat.
    pub fn flip_selected(&mut self) {
        let before: Vec<HitObject> = self
            .selected()
            .into_iter()
            .filter(|o| matches!(o.taiko(), Taiko::Don | Taiko::Kat))
            .collect();
        let after: Vec<HitObject> = before
            .iter()
            .map(|o| {
                let mut o = o.clone();
                let kat = o.taiko() == Taiko::Kat;
                o.sound &= !(super::chart::SOUND_WHISTLE | super::chart::SOUND_CLAP);
                if !kat {
                    o.sound |= super::chart::SOUND_WHISTLE;
                }
                o
            })
            .collect();
        if !before.is_empty() {
            self.edit(before, after, false);
        }
    }

    /// Gives a roll or swell a new end (made whole).
    pub fn set_end(&mut self, id: u64, end: f64, drag: bool) -> bool {
        let Some(before) = self.chart.object(id).cloned() else {
            return false;
        };
        let end = end.round();
        if end <= before.time {
            return false;
        }
        let mut after = before.clone();
        match before.taiko() {
            Taiko::Swell => after.set_spinner_end(end),
            Taiko::Roll => after.set_length(HitObject::length_for(
                end - before.time,
                before.slides(),
                self.sm(),
                timing::beat_length_at(&self.chart.timing, before.time),
                timing::sv_at(&self.chart.timing, before.time),
            )),
            _ => return false,
        }
        self.edit(vec![before], vec![after], drag);
        true
    }

    fn edit(&mut self, before: Vec<HitObject>, after: Vec<HitObject>, drag: bool) {
        let cmd = Command::EditObjects { before, after };
        if drag {
            self.history.run_drag(&mut self.chart, cmd);
        } else {
            self.history.run(&mut self.chart, cmd);
        }
    }

    pub fn end_drag(&mut self) {
        self.history.end_drag();
    }

    // ---- clipboard ----

    pub fn copy(&mut self) -> usize {
        let mut list = self.selected();
        let Some(first) = list.first().map(|o| o.time) else {
            return 0;
        };
        // Kept relative to the first, so it can land anywhere.
        list = list.iter().map(|o| o.shifted(-first)).collect();
        self.clipboard = list;
        self.clipboard.len()
    }

    pub fn cut(&mut self) -> usize {
        let n = self.copy();
        self.delete_selected();
        n
    }

    /// Pastes the clipboard with its first note at `t` (made whole), skipping any note that would
    /// land on one already there. The pasted notes become the selection.
    pub fn paste(&mut self, t: f64) -> usize {
        let t = t.round();
        let mut made: Vec<HitObject> = Vec::new();
        for o in &self.clipboard {
            let mut n = o.shifted(t);
            n.id = 0;
            if n.time >= 0.0
                && !self.chart.objects.iter().any(|q| q.time == n.time)
                && !made.iter().any(|q| q.time == n.time)
            {
                made.push(n);
            }
        }
        if made.is_empty() {
            return 0;
        }
        let n = made.len();
        self.history
            .run(&mut self.chart, Command::InsertObjects(made));
        // The newest ids are the pasted ones.
        let mut ids: Vec<u64> = self.chart.objects.iter().map(|o| o.id).collect();
        ids.sort_unstable();
        self.selection = ids[ids.len() - n..].iter().copied().collect();
        n
    }

    // ---- timing lines ----

    pub fn add_red(&mut self, t: f64, bpm: f64) -> Result<u64, String> {
        if !bpm.is_finite() || bpm <= 0.0 {
            return Err("BPM must be positive".into());
        }
        let mut p = TimingPoint::red(t.round(), bpm);
        if let Some(prev) = timing::active_at(&self.chart.timing, t) {
            p.sample_set = prev.sample_set;
            p.sample_index = prev.sample_index;
            p.volume = prev.volume;
            p.meter = prev.meter;
        }
        self.run_timing(p)
    }

    /// A green line at `t`, taking its hitsound settings and kiai from the line in force.
    pub fn add_green(&mut self, t: f64, sv: f64) -> Result<u64, String> {
        if !sv.is_finite() || sv <= 0.0 {
            return Err("slider velocity must be positive".into());
        }
        let mut p = TimingPoint::green(t.round(), sv);
        if let Some(prev) = timing::active_at(&self.chart.timing, t) {
            p.sample_set = prev.sample_set;
            p.sample_index = prev.sample_index;
            p.volume = prev.volume;
            p.meter = prev.meter;
            p.effects = prev.effects & super::chart::EFFECT_KIAI;
        }
        self.run_timing(p)
    }

    fn run_timing(&mut self, p: TimingPoint) -> Result<u64, String> {
        self.history
            .run(&mut self.chart, Command::InsertTiming(vec![p]));
        // Checked now so a bad line is undone rather than discovered at save.
        if let Err(why) = self.chart.validate() {
            self.history.undo(&mut self.chart);
            return Err(why);
        }
        // The new line sorts after any at its time, so it's the last one there.
        Ok(self.chart.timing.iter().map(|p| p.id).max().unwrap_or(0))
    }

    pub fn edit_timing(
        &mut self,
        id: u64,
        change: impl FnOnce(&mut TimingPoint),
        drag: bool,
    ) -> Result<(), String> {
        let before = self.chart.timing_point(id).cloned().ok_or("no such line")?;
        let mut after = before.clone();
        change(&mut after);
        if let Some(why) = after.problem() {
            return Err(why);
        }
        let cmd = Command::EditTiming {
            before: vec![before],
            after: vec![after],
        };
        if drag {
            self.history.run_drag(&mut self.chart, cmd);
        } else {
            self.history.run(&mut self.chart, cmd);
        }
        Ok(())
    }

    /// Removes a line, keeping at least one red one.
    pub fn delete_timing(&mut self, id: u64) -> Result<(), String> {
        let p = self.chart.timing_point(id).cloned().ok_or("no such line")?;
        if p.is_red() && self.chart.timing.iter().filter(|q| q.is_red()).count() == 1 {
            return Err("a chart needs at least one red line".into());
        }
        self.history
            .run(&mut self.chart, Command::RemoveTiming(vec![p]));
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        let done = self.history.undo(&mut self.chart);
        self.selection
            .retain(|id| self.chart.objects.iter().any(|o| o.id == *id));
        done
    }

    pub fn redo(&mut self) -> bool {
        let done = self.history.redo(&mut self.chart);
        self.selection
            .retain(|id| self.chart.objects.iter().any(|o| o.id == *id));
        done
    }
}

#[cfg(test)]
mod tests {
    use super::super::chart::tests::MAP;
    use super::*;

    fn editor() -> Editor {
        Editor::new(Chart::parse(MAP), PathBuf::from("map.osu"))
    }

    fn times(e: &Editor) -> Vec<f64> {
        e.chart.objects.iter().map(|o| o.time).collect()
    }

    #[test]
    fn a_click_lands_on_the_millisecond_stable_would_write() {
        let mut e = editor();
        // 1/4 of a 500 ms beat is 125 ms; a click near 1126 snaps to 1125.
        e.snap = 4;
        assert_eq!(e.snapped(1126.0), 1125.0);
        // On a beat length that isn't a whole number of ms, the grid position truncates down.
        let mut e = Editor::new(
            Chart::parse(
                "osu file format v14\n[TimingPoints]\n0,315.789,4,1,0,100,1,0\n[HitObjects]\n256,192,500,1,0\n",
            ),
            PathBuf::new(),
        );
        e.snap = 1;
        assert_eq!(e.snapped(640.0), 631.0);
    }

    #[test]
    fn placing_makes_the_note_and_refuses_a_second_on_the_same_ms() {
        let mut e = editor();
        let id = e.place_hit(3000.0, true).unwrap();
        assert_eq!(e.chart.object(id).unwrap().taiko(), Taiko::Kat);
        assert_eq!(e.selection, BTreeSet::from([id]));
        assert!(e.place_hit(3000.0, false).is_none());
        e.big = true;
        let big = e.place_hit(3100.0, false).unwrap();
        assert!(e.chart.object(big).unwrap().big());
        assert!(e.modified());
        e.undo();
        e.undo();
        assert!(!e.modified());
    }

    #[test]
    fn a_placed_roll_lasts_exactly_as_long_as_it_was_dragged() {
        let mut e = editor();
        // From 5000, under the 2x SV in force from 4000.
        for end in [5100.0, 5250.0, 5777.0, 7000.0] {
            let id = e.place_roll(5000.0, end).unwrap();
            let o = e.chart.object(id).unwrap().clone();
            assert_eq!(e.end_of(&o), end, "length {:?}", o.length());
            e.undo();
        }
        // And the game agrees: it reads the saved chart back with the same end.
        let id = e.place_roll(5000.0, 6000.0).unwrap();
        let map = crate::beatmap::tests::parse(&e.chart.to_text());
        let roll = map.notes.iter().find(|n| n.time == 5000.0).unwrap();
        assert_eq!(roll.end_time, 6000.0);
        assert!(e.set_end(id, 6400.0, false));
        let map = crate::beatmap::tests::parse(&e.chart.to_text());
        assert_eq!(
            map.notes
                .iter()
                .find(|n| n.time == 5000.0)
                .unwrap()
                .end_time,
            6400.0
        );
    }

    #[test]
    fn a_swell_keeps_its_length_when_moved_and_its_start_when_stretched() {
        let mut e = editor();
        let id = e.place_swell(12000.0, 13000.0).unwrap();
        assert!(e.move_selected(500.0, false));
        let o = e.chart.object(id).unwrap();
        assert_eq!((o.time, o.spinner_end()), (12500.0, Some(13500.0)));
        assert!(e.set_end(id, 14000.0, false));
        let o = e.chart.object(id).unwrap();
        assert_eq!((o.time, o.spinner_end()), (12500.0, Some(14000.0)));
    }

    #[test]
    fn moving_cant_push_a_note_before_the_start_and_a_drag_is_one_undo() {
        let mut e = editor();
        e.select_range(1000.0, 1000.0, false);
        assert!(!e.move_selected(-1001.0, false));
        assert!(e.move_selected(-1000.0, false), "all the way to 0 is fine");
        e.undo();
        for d in [10.0, 10.0, 15.0] {
            assert!(e.move_selected(d, true));
        }
        e.end_drag();
        assert_eq!(times(&e)[0], 1035.0);
        e.undo();
        assert_eq!(times(&e)[0], 1000.0);
    }

    #[test]
    fn selecting_a_range_deleting_and_undoing() {
        let mut e = editor();
        e.select_range(1000.0, 2000.0, false);
        assert_eq!(e.selection.len(), 3);
        e.select_range(2500.0, 2500.0, true);
        assert_eq!(e.selection.len(), 4);
        assert_eq!(e.delete_selected(), 4);
        assert_eq!(times(&e), [9000.0]);
        e.undo();
        assert_eq!(e.chart.objects.len(), 5);
        e.select_all();
        assert_eq!(e.selection.len(), 5);
    }

    #[test]
    fn retyping_changes_colour_and_size_independently() {
        let mut e = editor();
        e.select_range(1000.0, 2000.0, false);
        e.retype_selected(Some(true), None);
        assert!(e.chart.objects[..3].iter().all(|o| o.taiko() == Taiko::Kat));
        // The finisher bit on the third stays.
        assert!(e.chart.objects[2].big() && !e.chart.objects[0].big());
        e.retype_selected(None, Some(true));
        assert!(e.chart.objects[..3].iter().all(|o| o.big()));
        e.retype_selected(Some(false), Some(false));
        assert!(e.chart.objects[..3].iter().all(|o| o.sound == 0));
        e.flip_selected();
        assert!(e.chart.objects[..3].iter().all(|o| o.taiko() == Taiko::Kat));
    }

    #[test]
    fn a_paste_keeps_the_spacing_and_skips_what_would_collide() {
        let mut e = editor();
        e.select_range(1000.0, 2000.0, false);
        assert_eq!(e.copy(), 3);
        assert_eq!(e.paste(3000.0), 3);
        assert_eq!(e.selection.len(), 3);
        assert_eq!(times(&e)[3..6], [2500.0, 3000.0, 3500.0].map(|t| t));
        // 4000 is where the first paste's last note already stands: that one is skipped.
        assert_eq!(e.paste(4000.0), 2);
        assert_eq!(e.paste(4000.0), 0);
        let kinds: Vec<Taiko> = e
            .chart
            .objects
            .iter()
            .filter(|o| o.time >= 3000.0 && o.time <= 3500.0)
            .map(|o| o.taiko())
            .collect();
        assert_eq!(kinds, [Taiko::Don, Taiko::Kat]);
        e.undo();
        assert_eq!(e.chart.objects.len(), 8);
        e.undo();
        assert_eq!(e.chart.objects.len(), 5);
    }

    #[test]
    fn a_pasted_swell_moves_its_end_with_it() {
        let mut e = editor();
        e.select_range(9000.0, 9000.0, false);
        e.copy();
        e.paste(20000.0);
        let o = e.chart.objects.last().unwrap();
        assert_eq!((o.time, o.spinner_end()), (20000.0, Some(22000.0)));
    }

    #[test]
    fn timing_lines_are_added_edited_and_removed_with_checks() {
        let mut e = editor();
        let green = e.add_green(5000.0, 1.5).unwrap();
        let p = e.chart.timing_point(green).unwrap();
        assert!(
            (p.sv() - 1.5).abs() < 1e-9 && p.kiai(),
            "takes the kiai in force"
        );
        assert!(e.add_green(6000.0, 0.0).is_err());
        assert!(
            e.edit_timing(green, |p| p.beat_length = 10.0, false)
                .is_err()
        );
        e.edit_timing(green, |p| p.set_sv(2.0).unwrap(), false)
            .unwrap();
        assert_eq!(e.chart.timing_point(green).unwrap().sv(), 2.0);
        let red = e.add_red(6000.0, 180.0).unwrap();
        assert_eq!(
            e.chart.timing_point(red).unwrap().bpm(),
            Some(180.0 + 0.0).map(|b: f64| b)
        );
        e.delete_timing(red).unwrap();
        assert!(
            e.delete_timing(e.chart.timing[0].id).is_err(),
            "the only red line stays"
        );
        assert!(e.chart.to_text().contains("5000,-50,4,1,0,100,0,1"));
    }

    #[test]
    fn moving_a_line_in_time_keeps_the_chart_ordered() {
        let mut e = editor();
        let green = e.chart.timing[1].id;
        e.edit_timing(green, |p| p.time = 100.0, false).unwrap();
        assert_eq!(e.chart.timing[1].id, green);
        let t: Vec<f64> = e.chart.timing.iter().map(|p| p.time).collect();
        assert_eq!(t, [0.0, 100.0]);
        e.undo();
        assert_eq!(e.chart.timing[1].time, 4000.0);
    }

    #[test]
    fn objects_are_found_by_their_body_not_just_their_start() {
        let mut e = editor();
        let roll = e.place_roll(5000.0, 6000.0).unwrap();
        assert_eq!(e.object_at(5500.0, 0.0), Some(roll));
        assert_eq!(e.object_at(1003.0, 5.0), Some(e.chart.objects[0].id));
        assert_eq!(e.object_at(1200.0, 5.0), None);
    }
}
