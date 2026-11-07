//! What the editor does to a chart: placing, selecting, moving and copying notes, and editing
//! timing lines. Every change is one `Command` through the history, so it can be undone, and
//! every time it writes is a whole millisecond the way stable's editor writes it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::chart::{Chart, HitObject, Taiko, TimingPoint};
use super::gimmicks::GimmickConfig;
use super::history::{Command, History};
use super::timing;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    /// A don or a kat (see `Editor::kat`).
    Note,
    Roll,
    /// A swell, which stable calls a spinner or denden.
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
    /// The next note placed is a kat (a don otherwise).
    pub kat: bool,
    /// Bookmarked moments (map ms, sorted), kept in the file's `[Editor]` section like stable's.
    pub bookmarks: Vec<f64>,
    bookmarks_changed: bool,
    clipboard: Vec<HitObject>,
    /// Timing lines copied, with their times relative to the first.
    line_clipboard: Vec<TimingPoint>,
    /// The gimmick tools' settings.
    pub gimmick: GimmickConfig,
    /// The timing the gimmick tools read BPM and SV from: the chart as it was before any gimmick
    /// (taken at first use; see `Editor::reset_base`).
    pub base: Option<Vec<TimingPoint>>,
    /// Read the chart's SV as the curves it was drawn with (ramps between notes) rather than as
    /// osu! steps, for the anti-barline wall.
    pub sv_curves: bool,
    /// The file name (without extension) the metadata gave when the file was opened or last
    /// renamed: a save moves the file once the names in it make a different one.
    pub(super) named: String,
}

/// The file name (without extension) `chart`'s metadata gives a difficulty, as a new map is
/// named: the artist and title in their own script, the romanised ones when those are empty.
pub(super) fn named_by(chart: &Chart) -> String {
    let get = |key: &str| {
        chart
            .get_key("Metadata", key)
            .filter(|v| !v.trim().is_empty())
    };
    let either =
        |own: &str, romanised: &str| get(own).or_else(|| get(romanised)).unwrap_or_default();
    super::new_map::file_stem(
        &either("ArtistUnicode", "Artist"),
        &either("TitleUnicode", "Title"),
        &get("Creator").unwrap_or_default(),
        &get("Version").unwrap_or_default(),
    )
}

/// Whether `a` and `b` are one file (a name differing only in case, on a system that ignores it).
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// `path` with `.bak` added, where a save keeps the file's first version.
fn backup_of(path: &Path) -> PathBuf {
    let mut bak = path.as_os_str().to_owned();
    bak.push(".bak");
    PathBuf::from(bak)
}

impl Editor {
    pub fn new(chart: Chart, path: PathBuf) -> Editor {
        let named = named_by(&chart);
        let time = chart
            .objects
            .first()
            .map_or(0.0, |o| (o.time - 2000.0).max(0.0));
        let mut bookmarks: Vec<f64> = chart
            .get_key("Editor", "Bookmarks")
            .map(|v| {
                v.split(',')
                    .filter_map(|n| n.trim().parse::<f64>().ok())
                    .filter(|t| t.is_finite() && *t >= 0.0)
                    .collect()
            })
            .unwrap_or_default();
        bookmarks.sort_by(f64::total_cmp);
        bookmarks.dedup();
        Editor {
            chart,
            history: History::new(),
            path,
            selection: BTreeSet::new(),
            time,
            snap: 4,
            tool: Tool::Note,
            big: false,
            kat: false,
            bookmarks,
            bookmarks_changed: false,
            clipboard: Vec::new(),
            line_clipboard: Vec::new(),
            gimmick: GimmickConfig::default(),
            base: None,
            sv_curves: false,
            named,
        }
    }

    pub fn open(path: PathBuf) -> Result<Editor, String> {
        Ok(Editor::new(Chart::read(&path)?, path))
    }

    pub fn modified(&self) -> bool {
        self.history.modified() || self.bookmarks_changed
    }

    /// Writes the chart to its file. When the artist, title, mapper or difficulty name changed
    /// since the file was opened, the file then takes the name they give (`path` follows).
    /// `Ok(Some(why))`: saved, but the file kept its old name.
    pub fn save(&mut self) -> Result<Option<String>, String> {
        if self.bookmarks_changed {
            let list: Vec<String> = self
                .bookmarks
                .iter()
                .map(|t| (*t as i64).to_string())
                .collect();
            self.chart.set_key("Editor", "Bookmarks", &list.join(","));
        }
        self.chart.save(&self.path, true)?;
        self.history.mark_saved();
        self.bookmarks_changed = false;
        Ok(self.follow_name().err())
    }

    /// Moves the saved file to the name its metadata gives, once that changed.
    fn follow_name(&mut self) -> Result<(), String> {
        let named = named_by(&self.chart);
        if named == self.named {
            return Ok(());
        }
        let ext = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("osu");
        let dir = self.path.parent().unwrap_or(Path::new("."));
        let target = dir.join(format!("{named}.{ext}"));
        if target == self.path {
            self.named = named;
            return Ok(());
        }
        let file = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if target.exists() && !same_file(&target, &self.path) {
            return Err(format!("{file} already exists, so the file keeps its name"));
        }
        std::fs::rename(&self.path, &target)
            .map_err(|e| format!("can't rename the file to {file}: {e}"))?;
        // The first version kept beside it goes along.
        let (bak, new_bak) = (backup_of(&self.path), backup_of(&target));
        if bak.exists() && !new_bak.exists() {
            let _ = std::fs::rename(&bak, &new_bak);
        }
        self.path = target;
        self.named = named;
        Ok(())
    }

    // ---- bookmarks ----

    /// Bookmarks the moment `t`; false when it already is.
    pub fn add_bookmark(&mut self, t: f64) -> bool {
        let t = t.round();
        if self.bookmarks.contains(&t) {
            return false;
        }
        self.bookmarks.push(t);
        self.bookmarks.sort_by(f64::total_cmp);
        self.bookmarks_changed = true;
        true
    }

    /// Removes the bookmark nearest `t` within `reach` ms.
    pub fn remove_bookmark(&mut self, t: f64, reach: f64) -> bool {
        let near = self
            .bookmarks
            .iter()
            .enumerate()
            .filter(|(_, b)| (**b - t).abs() <= reach)
            .min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs()))
            .map(|(i, _)| i);
        let Some(i) = near else { return false };
        self.bookmarks.remove(i);
        self.bookmarks_changed = true;
        true
    }

    pub fn bookmark_before(&self, t: f64) -> Option<f64> {
        self.bookmarks.iter().rev().find(|b| **b < t - 1.0).copied()
    }

    pub fn bookmark_after(&self, t: f64) -> Option<f64> {
        self.bookmarks.iter().find(|b| **b > t + 1.0).copied()
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
        // Only rolls and swells have a length; skipping the timing lookups for the rest keeps this
        // cheap enough to call for every note on every frame.
        if !matches!(o.taiko(), Taiko::Roll | Taiko::Swell) {
            return o.time;
        }
        o.end_time(
            self.sm(),
            timing::beat_length_at(&self.chart.timing, o.time),
            timing::sv_at(&self.chart.timing, o.time),
        )
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

    // ---- sounds ----

    /// Toggles a hitsound bit (whistle, finish, clap) on the selected notes: all of them get it
    /// unless they all have it already. With nothing selected it flips what the next note placed
    /// gets instead. Returns whether the bit ended up on.
    pub fn toggle_sound(&mut self, bit: i32) -> bool {
        let before: Vec<HitObject> = self
            .selected()
            .into_iter()
            .filter(|o| o.taiko() != Taiko::Swell)
            .collect();
        if before.is_empty() {
            return self.toggle_next_sound(bit);
        }
        let on = !before.iter().all(|o| o.sound & bit != 0);
        let after: Vec<HitObject> = before
            .iter()
            .map(|o| {
                let mut o = o.clone();
                if on {
                    o.sound |= bit;
                } else {
                    o.sound &= !bit;
                }
                o
            })
            .collect();
        self.edit(before, after, false);
        on
    }

    /// Flips what the next note placed gets: big for the finish bit, kat for the others. Returns
    /// whether it ended up on.
    pub fn toggle_next_sound(&mut self, bit: i32) -> bool {
        if bit == super::chart::SOUND_FINISH {
            self.big = !self.big;
            self.big
        } else {
            self.kat = !self.kat;
            self.kat
        }
    }

    /// Makes the selected notes kats, or dons when they all are kats already. In the file a kat
    /// is a whistle or a clap (they look and play the same): a kat keeps the one it has, a new
    /// one gets a whistle, and a don loses both. Drumrolls and dendens are left alone. With no
    /// note selected it flips what the next note placed is. Returns whether they ended up kats.
    pub fn toggle_kat(&mut self) -> bool {
        use super::chart::{SOUND_CLAP, SOUND_WHISTLE};
        let notes: Vec<HitObject> = self
            .selected()
            .into_iter()
            .filter(|o| matches!(o.taiko(), Taiko::Don | Taiko::Kat))
            .collect();
        if notes.is_empty() {
            return self.toggle_next_sound(SOUND_WHISTLE);
        }
        let on = !notes.iter().all(|o| o.taiko() == Taiko::Kat);
        let before: Vec<HitObject> = notes
            .into_iter()
            .filter(|o| (o.taiko() == Taiko::Kat) != on)
            .collect();
        let after: Vec<HitObject> = before
            .iter()
            .map(|o| {
                let mut o = o.clone();
                o.sound &= !(SOUND_WHISTLE | SOUND_CLAP);
                if on {
                    o.sound |= SOUND_WHISTLE;
                }
                o
            })
            .collect();
        self.edit(before, after, false);
        on
    }

    /// Gives the note `id` the look of what the next note placed gets (kat or don, big or not).
    /// Returns whether it changed: drumrolls and dendens are left alone, and so is a note that
    /// already looks like that (a clap kat stays a clap kat).
    pub fn recolour(&mut self, id: u64) -> bool {
        use super::chart::{SOUND_CLAP, SOUND_FINISH, SOUND_WHISTLE};
        let Some(o) = self.chart.object(id).cloned() else {
            return false;
        };
        let is_kat = match o.taiko() {
            Taiko::Don => false,
            Taiko::Kat => true,
            Taiko::Roll | Taiko::Swell => return false,
        };
        if is_kat == self.kat && o.big() == self.big {
            return false;
        }
        let mut after = o.clone();
        if is_kat != self.kat {
            after.sound &= !(SOUND_WHISTLE | SOUND_CLAP);
            if self.kat {
                after.sound |= SOUND_WHISTLE;
            }
        }
        if o.big() != self.big {
            after.sound ^= SOUND_FINISH;
        }
        self.edit(vec![o], vec![after], false);
        true
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

    pub(super) fn selected(&self) -> Vec<HitObject> {
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
        let after: Vec<HitObject> = before.iter().map(|o| o.shifted(delta)).collect();
        // Never onto another note's millisecond, as placing and pasting refuse it too.
        let taken = |t: f64| {
            self.chart
                .objects
                .iter()
                .any(|q| (q.time - t).abs() < 1.0 && !self.selection.contains(&q.id))
        };
        if after.iter().any(|o| taken(o.time)) {
            return false;
        }
        self.edit(before, after, drag);
        true
    }

    /// Puts each selected object on the nearest line of the grid it is already on (the coarsest
    /// division that fits), the way stable's editor writes it. An object on no grid within a
    /// couple of milliseconds was put there on purpose and stays; so does one that would land on
    /// another's millisecond. Returns how many moved.
    pub fn resnap_selected(&mut self) -> usize {
        let before = self.selected();
        let mut taken: std::collections::HashSet<i64> = self
            .chart
            .objects
            .iter()
            .filter(|o| !self.selection.contains(&o.id))
            .map(|o| o.time as i64)
            .collect();
        let (mut b, mut a) = (Vec::new(), Vec::new());
        for o in before {
            let Some(to) = timing::resnap(&self.chart.timing, o.time) else {
                taken.insert(o.time as i64);
                continue;
            };
            if to as f64 == o.time || taken.contains(&to) {
                taken.insert(o.time as i64);
                continue;
            }
            taken.insert(to);
            a.push(o.shifted(to as f64 - o.time));
            b.push(o);
        }
        let n = a.len();
        if n > 0 {
            self.edit(b, a, false);
        }
        n
    }

    /// Makes the selected notes kat (`Some(true)`) or don, and big or small.
    #[cfg(test)]
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

    /// Moves the selection one snap division along the grid (`J`/`K` in stable).
    pub fn nudge_selected(&mut self, forward: bool) -> bool {
        let Some(first) = self.selected().first().map(|o| o.time) else {
            return false;
        };
        let to = timing::step(&self.chart.timing, first, forward, self.snap);
        self.move_selected(to - first, false)
    }

    /// Copies the selection to start one measure after its last object (`Ctrl+D`). The copies
    /// become the selection. Returns how many were made.
    pub fn clone_selected(&mut self) -> usize {
        let list = self.selected();
        let (Some(first), Some(last)) = (list.first(), list.last()) else {
            return 0;
        };
        let end = list
            .iter()
            .map(|o| self.end_of(o))
            .fold(last.time, f64::max);
        let measure = timing::beat_length_at(&self.chart.timing, last.time)
            * timing::active_red(&self.chart.timing, last.time).map_or(4, |p| p.meter.max(1))
                as f64;
        let at = self.snapped(end + measure);
        let keep = std::mem::take(&mut self.clipboard);
        let first_time = first.time;
        self.copy();
        let n = self.paste(at.max(first_time + 1.0));
        self.clipboard = keep;
        n
    }

    /// Mirrors the selection in time inside the span it covers (`Ctrl+G`).
    pub fn reverse_selected(&mut self) -> Result<usize, String> {
        let before = self.selected();
        if before.len() < 2 {
            return Err("select at least two notes to reverse".into());
        }
        let first = before.iter().map(|o| o.time).fold(f64::INFINITY, f64::min);
        let last = before
            .iter()
            .map(|o| self.end_of(o))
            .fold(f64::NEG_INFINITY, f64::max);
        let after: Vec<HitObject> = before
            .iter()
            .map(|o| {
                let to = (first + last - self.end_of(o)).round();
                o.shifted(to - o.time)
            })
            .collect();
        let taken = |t: f64| {
            self.chart
                .objects
                .iter()
                .any(|q| q.time == t && !self.selection.contains(&q.id))
        };
        if after.iter().any(|o| taken(o.time)) {
            return Err("reversed, a note would land on another".into());
        }
        let n = after.len();
        self.edit(before, after, false);
        Ok(n)
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
            // A plain .osu has nowhere to keep fakes: they come in as notes.
            n.fake &= self.chart.gimmick.is_some();
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
        // Checked now so a bad line is taken back rather than discovered at save.
        self.history.run_checked(
            &mut self.chart,
            Command::InsertTiming(vec![p]),
            Chart::validate,
        )?;
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

    /// Edits several timing lines as one undo step (`drag`: a step of a drag still going on).
    /// `change` is told each line before it is changed. A line it makes invalid refuses the lot.
    pub fn edit_timings(
        &mut self,
        ids: &[u64],
        mut change: impl FnMut(&mut TimingPoint),
        drag: bool,
    ) -> Result<(), String> {
        let (mut before, mut after) = (Vec::new(), Vec::new());
        for id in ids {
            let Some(b) = self.chart.timing_point(*id).cloned() else {
                continue;
            };
            let mut a = b.clone();
            change(&mut a);
            if let Some(why) = a.problem() {
                return Err(why);
            }
            if a.differs_from(&b) {
                before.push(b);
                after.push(a);
            }
        }
        if before.is_empty() {
            return Ok(());
        }
        let cmd = Command::EditTiming { before, after };
        if drag {
            self.history.run_drag(&mut self.chart, cmd);
        } else {
            self.history.run(&mut self.chart, cmd);
        }
        Ok(())
    }

    /// Removes timing lines as one step, never the last red one.
    pub fn delete_timings(&mut self, ids: &[u64]) -> Result<usize, String> {
        let gone: Vec<TimingPoint> = self
            .chart
            .timing
            .iter()
            .filter(|p| ids.contains(&p.id))
            .cloned()
            .collect();
        if gone.is_empty() {
            return Ok(0);
        }
        let reds = self.chart.timing.iter().filter(|p| p.is_red()).count();
        if gone.iter().filter(|p| p.is_red()).count() >= reds {
            return Err("a chart needs at least one red line".into());
        }
        let n = gone.len();
        self.history
            .run(&mut self.chart, Command::RemoveTiming(gone));
        Ok(n)
    }

    pub fn copy_lines(&mut self, ids: &[u64]) -> usize {
        let mut list: Vec<TimingPoint> = self
            .chart
            .timing
            .iter()
            .filter(|p| ids.contains(&p.id))
            .cloned()
            .collect();
        let Some(first) = list.first().map(|p| p.time) else {
            return 0;
        };
        for p in &mut list {
            p.time -= first;
        }
        self.line_clipboard = list;
        self.line_clipboard.len()
    }

    /// Whether lines were copied to paste.
    pub fn has_copied_lines(&self) -> bool {
        !self.line_clipboard.is_empty()
    }

    /// Puts the copied lines down with the first at `t`; a line that would share a time with one
    /// already there is skipped. Returns how many went in.
    pub fn paste_lines(&mut self, t: f64) -> Result<usize, String> {
        let t = t.round();
        let mut made: Vec<TimingPoint> = Vec::new();
        for p in &self.line_clipboard {
            let mut n = p.clone();
            n.time = (t + p.time).round();
            n.id = 0;
            if !self
                .chart
                .timing
                .iter()
                .any(|q| q.time == n.time && q.is_red() == n.is_red())
            {
                made.push(n);
            }
        }
        if made.is_empty() {
            return Ok(0);
        }
        let n = made.len();
        self.history.run_checked(
            &mut self.chart,
            Command::InsertTiming(made),
            Chart::validate,
        )?;
        Ok(n)
    }

    /// Removes the line starting at `t` (within a millisecond); the last one there when two share it.
    pub fn delete_timing_at(&mut self, t: f64) -> Result<(), String> {
        let id = self
            .chart
            .timing
            .iter()
            .rev()
            .find(|p| (p.time - t).abs() < 1.0)
            .map(|p| p.id)
            .ok_or("no timing line at the playhead")?;
        self.delete_timing(id)
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
    fn a_line_turned_down_leaves_undo_and_redo_as_they_were() {
        let mut e = editor();
        let lines = e.chart.timing.len();
        e.add_green(5000.0, 1.5).unwrap();
        assert!(e.undo());
        assert!(e.add_red(f64::NAN, 120.0).is_err());
        assert_eq!(e.chart.timing.len(), lines);
        assert!(e.redo(), "the green line can still come back");
        assert_eq!(e.chart.timing.len(), lines + 1);
        assert!(!e.redo(), "and the line turned down never does");
    }

    #[test]
    fn notes_never_move_onto_another_notes_millisecond() {
        let mut e = editor();
        let (a, b) = (e.chart.objects[0].clone(), e.chart.objects[1].clone());
        e.selection = [a.id].into();
        assert!(!e.move_selected(b.time - a.time, false));
        assert_eq!(e.chart.object(a.id).unwrap().time, a.time);
        assert!(e.move_selected(1.0, false));
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
    fn resnap_moves_what_is_a_hair_off_its_grid_and_leaves_what_is_on_purpose() {
        let mut e = Editor::new(
            Chart::parse(super::super::chart::tests::MAP),
            PathBuf::from("t"),
        );
        e.chart.objects.clear();
        // 120 BPM from 1000: a gridline every 125 ms at 1/4. 1251 is a ms off, 1380 is nowhere.
        e.chart.insert_object(HitObject::hit(1251.0, false, false));
        e.chart.insert_object(HitObject::hit(1380.0, true, false));
        e.chart.insert_object(HitObject::hit(1500.0, false, false));
        e.select_all();
        assert_eq!(e.resnap_selected(), 1);
        let times: Vec<f64> = e.chart.objects.iter().map(|o| o.time).collect();
        assert_eq!(times, [1250.0, 1380.0, 1500.0]);
        assert!(e.undo());
        assert_eq!(e.chart.objects[0].time, 1251.0);
        assert_eq!(e.resnap_selected(), 1, "the selection is still there");
        assert_eq!(e.resnap_selected(), 0, "and now it is on the grid");
    }

    #[test]
    fn a_sound_toggles_on_the_selection_or_on_the_next_note() {
        use super::super::chart::{SOUND_FINISH, SOUND_WHISTLE};
        let mut e = editor();
        // Nothing selected: the next note.
        assert!(e.toggle_sound(SOUND_WHISTLE) && e.kat);
        assert!(e.toggle_sound(SOUND_FINISH) && e.big);
        assert!(!e.toggle_sound(SOUND_WHISTLE) && !e.kat);
        // A selection where only some have it: all get it, then none.
        e.selection = e.chart.objects.iter().take(2).map(|o| o.id).collect();
        let had = e.chart.objects[1].sound & SOUND_WHISTLE != 0;
        assert!(e.toggle_sound(SOUND_WHISTLE));
        assert!(
            e.chart.objects[..2]
                .iter()
                .all(|o| o.sound & SOUND_WHISTLE != 0)
        );
        assert!(!e.toggle_sound(SOUND_WHISTLE));
        assert!(
            e.chart.objects[..2]
                .iter()
                .all(|o| o.sound & SOUND_WHISTLE == 0)
        );
        let _ = had;
        assert!(e.undo() && e.undo());
    }

    #[test]
    fn one_kat_key_turns_whistles_and_claps_alike() {
        use super::super::chart::{SOUND_CLAP, SOUND_WHISTLE};
        let mut e = editor();
        let circles: Vec<u64> = e
            .chart
            .objects
            .iter()
            .filter(|o| matches!(o.taiko(), Taiko::Don | Taiko::Kat))
            .take(3)
            .map(|o| o.id)
            .collect();
        assert_eq!(circles.len(), 3);
        // A clap kat, a whistle kat and a don.
        for (id, sound) in circles.iter().zip([SOUND_CLAP, SOUND_WHISTLE, 0]) {
            let o = e.chart.objects.iter_mut().find(|o| o.id == *id).unwrap();
            o.sound = sound;
        }
        let look = |e: &Editor| -> Vec<i32> {
            circles
                .iter()
                .map(|id| e.chart.object(*id).unwrap().sound)
                .collect()
        };
        e.selection = circles.iter().copied().collect();
        // Not all are kats: the don becomes one, and the kats keep the sound they had.
        assert!(e.toggle_kat());
        assert_eq!(look(&e), [SOUND_CLAP, SOUND_WHISTLE, SOUND_WHISTLE]);
        // All are kats: all become dons, the clap one too.
        assert!(!e.toggle_kat());
        assert_eq!(look(&e), [0, 0, 0]);
        assert!(e.undo());
        assert_eq!(look(&e), [SOUND_CLAP, SOUND_WHISTLE, SOUND_WHISTLE]);
        // With nothing selected it is the note in hand.
        e.selection.clear();
        assert!(e.toggle_kat() && e.kat);
    }

    #[test]
    fn bookmarks_are_kept_sorted_and_found_by_neighbour() {
        let mut e = editor();
        assert!(e.add_bookmark(3000.0) && e.add_bookmark(1000.0));
        assert!(!e.add_bookmark(1000.0), "no second one at the same time");
        assert_eq!(e.bookmarks, [1000.0, 3000.0]);
        assert_eq!(e.bookmark_after(1000.0), Some(3000.0));
        assert_eq!(e.bookmark_before(3000.0), Some(1000.0));
        assert_eq!(e.bookmark_before(1000.0), None);
        assert!(e.remove_bookmark(3200.0, 500.0));
        assert!(!e.remove_bookmark(9000.0, 500.0));
        assert!(e.modified());
    }

    #[test]
    fn reverse_mirrors_the_selection_in_its_span_and_undoes() {
        let mut e = editor();
        e.select_all();
        let before = times(&e);
        let kinds: Vec<_> = e.chart.objects.iter().map(|o| o.taiko()).collect();
        assert_eq!(e.reverse_selected().unwrap(), before.len());
        let after = times(&e);
        assert_eq!(after.len(), before.len());
        // The first note is now where the last one ended, and the kinds are in the opposite order.
        let rev: Vec<_> = e.chart.objects.iter().map(|o| o.taiko()).collect();
        assert_eq!(rev, kinds.iter().rev().copied().collect::<Vec<_>>());
        assert!(e.undo());
        assert_eq!(times(&e), before);
        e.selection = e.chart.objects.iter().take(1).map(|o| o.id).collect();
        assert!(
            e.reverse_selected().is_err(),
            "one note has nothing to reverse"
        );
    }

    #[test]
    fn clone_lands_a_measure_after_the_last_note_and_leaves_the_clipboard() {
        let mut e = editor();
        e.selection = e.chart.objects.iter().take(2).map(|o| o.id).collect();
        e.copy();
        let n = e.chart.objects.len();
        e.selection = e.chart.objects.iter().take(2).map(|o| o.id).collect();
        assert_eq!(e.clone_selected(), 2);
        assert_eq!(e.chart.objects.len(), n + 2);
        // 1500 + a 4/4 measure of 500 ms beats = 3500, a first at 1000 + that gap.
        let t = times(&e);
        assert!(t.contains(&3500.0), "{t:?}");
        assert_eq!(e.selection.len(), 2, "the copies are selected");
        assert_eq!(e.paste(20000.0), 2, "the clipboard was kept");
        assert_eq!(e.clone_selected(), 2);
    }

    #[test]
    fn nudging_moves_one_snap_step_and_a_line_is_removed_at_the_playhead() {
        let mut e = editor();
        e.snap = 4;
        e.selection = e.chart.objects.iter().take(1).map(|o| o.id).collect();
        assert!(e.nudge_selected(true));
        assert_eq!(e.chart.objects[0].time, 1125.0);
        assert!(e.nudge_selected(false));
        assert_eq!(e.chart.objects[0].time, 1000.0);
        // The green line at 4000.
        assert!(e.delete_timing_at(4000.0).is_ok());
        assert!(e.delete_timing_at(4000.0).is_err());
        assert_eq!(e.chart.timing.len(), 1);
        assert!(e.delete_timing_at(0.0).is_err(), "the only red line stays");
    }

    #[test]
    fn lines_are_deleted_copied_and_pasted_with_their_spacing() {
        let mut e = editor();
        let a = e.add_green(1000.0, 1.2).unwrap();
        let b = e.add_green(1600.0, 0.8).unwrap();
        assert_eq!(e.copy_lines(&[a, b]), 2);
        assert_eq!(e.paste_lines(9000.0).unwrap(), 2);
        let times: Vec<f64> = e.chart.timing.iter().map(|p| p.time).collect();
        assert!(
            times.contains(&9000.0) && times.contains(&9600.0),
            "{times:?}"
        );
        assert_eq!(e.paste_lines(9000.0).unwrap(), 0, "nothing doubles up");
        assert!(e.undo());
        assert_eq!(e.delete_timings(&[a, b]).unwrap(), 2);
        let red = e.chart.timing[0].id;
        assert!(e.delete_timings(&[red]).is_err(), "the only red line stays");
    }
}
