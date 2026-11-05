//! Editing a `.tko` chart's gimmick: keys on the playfield's and on groups of notes' tracks, and
//! turning a plain `.osu` chart into a `.tko`. Every change is one undo step.

use std::path::PathBuf;

use super::history::Command;
use super::state::Editor;
use crate::tko::gimmick::{FieldProp, Gimmick, Group, MAX_GROUPS, NoteProp};
use crate::tko::track::{Ease, Key};

/// What a keyframe track belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Field(FieldProp),
    Group(usize, NoteProp),
}

impl Subject {
    pub fn default_value(self) -> f64 {
        match self {
            Subject::Field(p) => p.default_value(),
            Subject::Group(_, p) => p.default_value(),
        }
    }

    pub fn range(self) -> (f64, f64) {
        match self {
            Subject::Field(p) => p.range(),
            Subject::Group(_, p) => p.range(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Subject::Field(p) => p.name(),
            Subject::Group(_, p) => p.name(),
        }
    }

    /// How far a value is worth showing apart on the graph at the least.
    pub fn min_span(self) -> f64 {
        match self {
            Subject::Field(FieldProp::X | FieldProp::Y)
            | Subject::Group(_, NoteProp::Dx | NoteProp::Dy) => 100.0,
            Subject::Field(FieldProp::Rotation) => 90.0,
            _ => 1.0,
        }
    }
}

impl Editor {
    pub fn is_tko(&self) -> bool {
        self.chart.gimmick.is_some()
    }

    pub fn gimmick(&self) -> Option<&Gimmick> {
        self.chart.gimmick.as_ref()
    }

    /// Changes the gimmick as one undo step (`drag`: folded into the drag in progress). Returns
    /// whether anything changed. A plain `.osu` chart has none to change.
    pub fn edit_gimmick(&mut self, drag: bool, f: impl FnOnce(&mut Gimmick)) -> bool {
        let Some(before) = self.chart.gimmick.clone() else {
            return false;
        };
        let mut after = before.clone();
        f(&mut after);
        after.sanitize();
        if after == before {
            return false;
        }
        let cmd = Command::Gimmick {
            before: Box::new(before),
            after: Box::new(after),
        };
        if drag {
            self.history.run_drag(&mut self.chart, cmd);
        } else {
            self.history.run(&mut self.chart, cmd);
        }
        true
    }

    /// The track of `s`, as it is now.
    pub fn track_of(&self, s: Subject) -> Option<&crate::tko::track::Track> {
        let g = self.gimmick()?;
        match s {
            Subject::Field(p) => g.field_track(p),
            Subject::Group(i, p) => g.groups.get(i)?.track(p),
        }
    }

    /// The value `s` has at `t` (its default with no keys).
    pub fn value_of(&self, s: Subject, t: f64) -> f64 {
        self.track_of(s)
            .map_or(s.default_value(), |tr| tr.value(t, s.default_value()))
    }

    /// Sets a key, replacing the one at its time.
    pub fn key(&mut self, s: Subject, key: Key, drag: bool) -> bool {
        self.edit_gimmick(drag, |g| match s {
            Subject::Field(p) => g.key_field(p, key),
            Subject::Group(i, p) => g.key_note(i, p, key),
        })
    }

    /// Moves key `index` of `s` to `key` (its time, value and easing as given).
    pub fn move_key(&mut self, s: Subject, index: usize, key: Key, drag: bool) -> bool {
        self.edit_gimmick(drag, |g| {
            let track = match s {
                Subject::Field(p) => g.field_track_mut(p),
                Subject::Group(i, p) => match g.groups.get_mut(i) {
                    Some(group) => group.track_mut(p),
                    None => return,
                },
            };
            track.remove(index);
            let (lo, hi) = s.range();
            track.set(Key {
                v: key.v.clamp(lo, hi),
                ..key
            });
        })
    }

    pub fn remove_key(&mut self, s: Subject, index: usize) -> bool {
        self.edit_gimmick(false, |g| {
            match s {
                Subject::Field(p) => g.field_track_mut(p).remove(index),
                Subject::Group(i, p) => g
                    .groups
                    .get_mut(i)
                    .and_then(|gr| gr.track_mut(p).remove(index)),
            };
        })
    }

    /// A group of the selected notes. Returns its index.
    pub fn add_group_from_selection(&mut self) -> Result<usize, String> {
        let times: Vec<f64> = self.selected().iter().map(|o| o.time).collect();
        if times.is_empty() {
            return Err("select some notes first".into());
        }
        let Some(g) = self.gimmick() else {
            return Err("this chart isn't a .tko yet".into());
        };
        if g.groups.len() >= MAX_GROUPS {
            return Err("too many groups".into());
        }
        let at = g.groups.len();
        self.edit_gimmick(false, |g| {
            g.groups.push(Group {
                name: format!("Group {}", at + 1),
                notes: times,
                ..Group::default()
            });
        });
        Ok(at)
    }

    pub fn delete_group(&mut self, i: usize) -> bool {
        self.edit_gimmick(false, |g| {
            if i < g.groups.len() {
                g.groups.remove(i);
            }
        })
    }

    /// Selects the notes of group `i`.
    pub fn select_group(&mut self, i: usize) {
        let Some(g) = self.gimmick().and_then(|g| g.groups.get(i)) else {
            return;
        };
        let times = g.notes.clone();
        self.selection = self
            .chart
            .objects
            .iter()
            .filter(|o| {
                let at = times.partition_point(|&t| t < o.time - 0.5);
                times.get(at).is_some_and(|&t| (t - o.time).abs() <= 0.5)
            })
            .map(|o| o.id)
            .collect();
    }

    /// Makes this chart a `.tko`: written next to the `.osu` (which stays as it is on disk) as
    /// `<name>.tko`, with the difficulty named "<version> (tko)". Editing goes on in the new file.
    pub fn make_tko(&mut self) -> Result<PathBuf, String> {
        if self.is_tko() {
            return Err("already a .tko".into());
        }
        let path = self.path.with_extension("tko");
        if path.exists() {
            return Err(format!("{} already exists", path.display()));
        }
        let version = self
            .chart
            .get_key("Metadata", "Version")
            .unwrap_or_else(|| "Gimmick".into());
        self.chart
            .set_key("Metadata", "Version", &format!("{version} (tko)"));
        self.chart.gimmick = Some(Gimmick::default());
        self.chart.dirty = true;
        if let Err(e) = self.chart.save(&path, false) {
            self.chart.gimmick = None;
            self.chart.set_key("Metadata", "Version", &version);
            return Err(e);
        }
        self.path = path.clone();
        self.history.mark_saved();
        Ok(path)
    }

    /// A key at `t` for `s` holding the value there now, with `ease`.
    pub fn key_here(&mut self, s: Subject, t: f64, ease: Ease) -> bool {
        let v = self.value_of(s, t);
        self.key(s, Key { t, v, ease }, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::{Chart, tests::MAP};
    use std::path::PathBuf;

    fn tko_editor() -> Editor {
        let mut e = Editor::new(Chart::parse(MAP), PathBuf::from("t.osu"));
        e.chart.gimmick = Some(Gimmick::default());
        e
    }

    fn key(t: f64, v: f64) -> Key {
        Key {
            t,
            v,
            ease: Ease::Linear,
        }
    }

    #[test]
    fn a_plain_osu_has_no_gimmick_to_edit() {
        let mut e = Editor::new(Chart::parse(MAP), PathBuf::from("t.osu"));
        assert!(!e.is_tko());
        assert!(!e.key(Subject::Field(FieldProp::Zoom), key(0.0, 2.0), false));
        assert!(e.add_group_from_selection().is_err());
    }

    #[test]
    fn keys_are_added_moved_removed_and_undone() {
        let mut e = tko_editor();
        let zoom = Subject::Field(FieldProp::Zoom);
        assert!(e.key(zoom, key(1000.0, 1.0), false));
        assert!(e.key(zoom, key(2000.0, 2.0), false));
        assert_eq!(e.value_of(zoom, 1500.0), 1.5);
        assert!(e.move_key(zoom, 1, key(3000.0, 3.0), false));
        assert_eq!(e.value_of(zoom, 2000.0), 2.0);
        assert!(e.remove_key(zoom, 0));
        assert_eq!(e.track_of(zoom).unwrap().keys.len(), 1);
        assert!(e.undo() && e.undo() && e.undo());
        assert_eq!(e.track_of(zoom).unwrap().keys.len(), 1);
        assert!(e.undo());
        assert!(e.track_of(zoom).is_none());
    }

    #[test]
    fn a_drag_of_a_key_is_one_undo_step() {
        let mut e = tko_editor();
        let rot = Subject::Field(FieldProp::Rotation);
        e.key(rot, key(1000.0, 0.0), false);
        for i in 1..=5 {
            assert!(e.move_key(rot, 0, key(1000.0 + 10.0 * i as f64, 5.0 * i as f64), true));
        }
        e.end_drag();
        assert_eq!(e.track_of(rot).unwrap().keys[0].t, 1050.0);
        assert!(e.undo());
        assert_eq!(e.track_of(rot).unwrap().keys[0].t, 1000.0);
    }

    #[test]
    fn a_key_is_held_inside_what_its_property_allows() {
        let mut e = tko_editor();
        e.key(Subject::Field(FieldProp::Zoom), key(0.0, 50.0), false);
        assert_eq!(e.value_of(Subject::Field(FieldProp::Zoom), 0.0), 8.0);
    }

    #[test]
    fn a_group_takes_the_selected_notes_and_selects_them_back() {
        let mut e = tko_editor();
        assert!(e.add_group_from_selection().is_err());
        let ids: Vec<u64> = e.chart.objects.iter().take(3).map(|o| o.id).collect();
        e.selection = ids.iter().copied().collect();
        let i = e.add_group_from_selection().unwrap();
        assert_eq!(e.gimmick().unwrap().groups[i].notes.len(), 3);
        e.selection.clear();
        e.select_group(i);
        assert_eq!(e.selection.len(), 3);
        let sub = Subject::Group(i, NoteProp::Opacity);
        e.key(sub, key(0.0, 1.0), false);
        e.key(sub, key(1000.0, 0.0), false);
        assert_eq!(e.value_of(sub, 500.0), 0.5);
        assert!(e.delete_group(i));
        assert!(e.gimmick().unwrap().groups.is_empty());
        assert!(e.undo());
        assert_eq!(e.gimmick().unwrap().groups.len(), 1);
    }

    #[test]
    fn making_a_tko_writes_a_new_file_and_leaves_the_osu_alone() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-tko-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let osu = dir.join("Artist - Title (Mapper) [Oni].osu");
        std::fs::write(&osu, MAP).unwrap();
        let mut e = Editor::open(osu.clone()).unwrap();
        let path = e.make_tko().unwrap();
        assert_eq!(path.extension().unwrap(), "tko");
        assert!(e.make_tko().is_err());
        e.key(
            Subject::Field(FieldProp::Rotation),
            key(1000.0, 90.0),
            false,
        );
        e.save().unwrap();
        let (osu_now, tko) = (
            std::fs::read_to_string(&osu).unwrap(),
            std::fs::read_to_string(&path).unwrap(),
        );
        let reopened = Editor::open(path.clone()).unwrap();
        let game_reads = crate::beatmap::load(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(osu_now, MAP, "the .osu is untouched");
        assert!(tko.contains("TkoVersion: 1") && tko.contains("[Gimmick]"));
        assert!(tko.contains("[Oni (tko)]") || tko.contains("Version:Oni (tko)"));
        assert_eq!(
            reopened.value_of(Subject::Field(FieldProp::Rotation), 1000.0),
            90.0
        );
        assert!(!reopened.modified());
        let map = game_reads.unwrap();
        assert_eq!(map.meta.tko_version, 1);
        assert!(map.gimmick.is_some());
    }
}
