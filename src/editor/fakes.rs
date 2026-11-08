//! Fake notes in the editor (`.tko` maps): notes that are only drawn in the game, never judged.
//! They are edited as notes with a flag (see `HitObject::fake`), so moving, copying and deleting
//! them works as for any note, and they go into the gimmick when the map is saved.

use super::chart::{HitObject, Taiko};
use super::history::Command;
use super::state::Editor;

impl Editor {
    /// The selected dons and kats.
    fn picked_hits(&self) -> Vec<HitObject> {
        self.selected()
            .into_iter()
            .filter(|o| matches!(o.taiko(), Taiko::Don | Taiko::Kat))
            .collect()
    }

    /// Makes the selected dons and kats fakes, or real again when they all are. Returns how many
    /// changed and whether they are fakes now.
    pub fn toggle_fake(&mut self) -> Result<(usize, bool), String> {
        if !self.is_tko() {
            return Err("fake notes need a .tko map".into());
        }
        let notes = self.picked_hits();
        if notes.is_empty() {
            return Err("select dons or kats first".into());
        }
        let on = !notes.iter().all(|o| o.fake);
        // A fake becomes a note only where no note is (one note to a millisecond; of fakes
        // stacked on one, the first).
        let mut made_real: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let before: Vec<HitObject> = notes
            .into_iter()
            .filter(|o| o.fake != on)
            .filter(|o| {
                on || (!self.occupied(o.time, false, |_| false) && made_real.insert(o.time as i64))
            })
            .collect();
        if before.is_empty() {
            return Err("there are notes where those fakes are already".into());
        }
        let after: Vec<HitObject> = before
            .iter()
            .map(|o| {
                let mut o = o.clone();
                o.fake = on;
                o
            })
            .collect();
        let n = after.len();
        self.history
            .run(&mut self.chart, Command::EditObjects { before, after });
        Ok((n, on))
    }

    /// How many of the selected notes are drawn over every effect, and how many are selected.
    pub fn above_picked(&self) -> (usize, usize) {
        let picked = self.selected();
        let above = self
            .gimmick()
            .map_or(0, |g| picked.iter().filter(|o| g.is_above(o.time)).count());
        (above, picked.len())
    }

    /// Draws the selected notes over every effect (a blackout, a flash, effects on all the
    /// notes), or not again when they all are; one undo step. Returns how many changed and
    /// whether they are drawn over now.
    pub fn toggle_above(&mut self) -> Result<(usize, bool), String> {
        if !self.is_tko() {
            return Err("notes over the effects need a .tko map".into());
        }
        let times: Vec<f64> = self.selected().iter().map(|o| o.time).collect();
        if times.is_empty() {
            return Err("select notes first".into());
        }
        let (above, all) = self.above_picked();
        let on = above < all;
        let n = if on { all - above } else { all };
        self.edit_gimmick(false, |g| {
            if on {
                g.above.extend(times.iter().copied());
            } else {
                g.above
                    .retain(|&a| !times.iter().any(|&t| (t - a).abs() <= 0.5));
            }
        });
        Ok((n, on))
    }

    /// Adds a fake copy of each selected don and kat `beats` beats later (earlier below 0), where
    /// no note is yet. The copies become the selection. Returns how many.
    pub fn add_fake_copies(&mut self, beats: f64) -> Result<usize, String> {
        if !self.is_tko() {
            return Err("fake notes need a .tko map".into());
        }
        let notes = self.picked_hits();
        if notes.is_empty() {
            return Err("select dons or kats first".into());
        }
        let mut made: Vec<HitObject> = Vec::new();
        // A copy flies with its note's group (its path, its moves), as a copy should.
        let mut gimmick = self.chart.gimmick.clone();
        for o in &notes {
            let t = (o.time + beats * self.beat_at(o.time)).round();
            // A copy goes on no note (it would only hide behind it); fakes stack freely.
            if t < 0.0 || self.occupied(t, false, |_| false) {
                continue;
            }
            let mut f = HitObject::hit(t, o.taiko() == Taiko::Kat, o.big());
            f.fake = true;
            made.push(f);
            if let Some(gr) = gimmick.as_mut().and_then(|g| {
                g.groups
                    .iter_mut()
                    .find(|gr| gr.notes.iter().any(|&n| (n - o.time).abs() <= 0.5))
            }) {
                gr.notes.push(t);
                gr.notes.sort_by(f64::total_cmp);
            }
        }
        if made.is_empty() {
            return Err("there are notes in all those places already".into());
        }
        let n = made.len();
        let mut steps = vec![Command::InsertObjects(made)];
        if let (Some(before), Some(mut after)) = (self.chart.gimmick.clone(), gimmick) {
            after.sanitize();
            if after != before {
                steps.push(Command::Gimmick {
                    before: Box::new(before),
                    after: Box::new(after),
                });
            }
        }
        self.history.run(&mut self.chart, Command::Composite(steps));
        let mut ids: Vec<u64> = self.chart.objects.iter().map(|o| o.id).collect();
        ids.sort_unstable();
        self.selection = ids[ids.len() - n..].iter().copied().collect();
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::Chart;
    use crate::editor::chart::tests::MAP;

    fn tko() -> Editor {
        let mut e = Editor::new(Chart::parse(MAP), std::path::PathBuf::from("a.tko"));
        e.chart.gimmick = Some(Default::default());
        e
    }

    fn pick(e: &mut Editor, n: usize) {
        let ids: Vec<u64> = e
            .chart
            .objects
            .iter()
            .filter(|o| matches!(o.taiko(), Taiko::Don | Taiko::Kat))
            .take(n)
            .map(|o| o.id)
            .collect();
        e.selection = ids.into_iter().collect();
    }

    #[test]
    fn notes_go_over_the_effects_and_back_and_stay_so_when_moved() {
        let mut e = tko();
        assert!(e.toggle_above().is_err(), "nothing picked");
        pick(&mut e, 2);
        assert_eq!(e.above_picked(), (0, 2));
        assert_eq!(e.toggle_above().unwrap(), (2, true));
        assert_eq!(e.above_picked(), (2, 2));
        let first = e.selected()[0].time;
        let g = e.gimmick().unwrap();
        assert!(g.is_above(first));
        // Saved with the map and read back.
        let text = e.chart.to_text();
        let back = Chart::parse_tko(&text).unwrap();
        assert!(back.gimmick.as_ref().unwrap().is_above(first));
        // Moved, they stay over the effects where they went.
        assert!(e.move_selected(10.0, false));
        let g = e.gimmick().unwrap();
        assert!(g.is_above(first + 10.0) && !g.is_above(first));
        // Toggled again, they are under the effects; undo puts them back over.
        assert_eq!(e.toggle_above().unwrap(), (2, false));
        assert!(e.gimmick().unwrap().above.is_empty());
        assert!(e.undo());
        assert_eq!(e.above_picked(), (2, 2));
        let mut plain = Editor::new(Chart::parse(MAP), std::path::PathBuf::from("a.osu"));
        plain.selection = e.selection.clone();
        assert!(plain.toggle_above().is_err(), "a plain .osu has no effects");
    }

    #[test]
    fn fakes_leave_the_chart_for_the_gimmick_and_come_back() {
        let mut e = tko();
        let hits = |text: &str| crate::beatmap::from_text(text).unwrap().notes.len();
        let real = hits(&e.chart.to_text());
        pick(&mut e, 2);
        assert_eq!(e.toggle_fake().unwrap(), (2, true));
        let text = e.chart.to_text();
        // Out of the hit objects, into the gimmick, and drawn by the game as fakes.
        let map = crate::beatmap::from_text(&text).unwrap();
        assert_eq!(map.notes.len(), real - 2);
        assert_eq!(map.fakes.len(), 2);
        // At the speed a real note there has, on the game's clock like the notes.
        assert_eq!(map.fakes[0].velocity, map.notes[0].velocity);
        assert_eq!(map.fakes[0].time, 1000.0 + crate::beatmap::OSU_TIMING_MS);
        // Read back, they are notes flagged fake again.
        let back = Chart::parse_tko(&text).unwrap();
        assert_eq!(back.objects.iter().filter(|o| o.fake).count(), 2);
        assert!(back.gimmick.as_ref().unwrap().fakes.is_empty());
        assert_eq!(back.to_text(), text, "read and written unchanged");
        // Toggled again they are real, and undo brings the fakes back.
        assert_eq!(e.toggle_fake().unwrap(), (2, false));
        assert_eq!(hits(&e.chart.to_text()), real);
        assert!(e.undo());
        assert_eq!(hits(&e.chart.to_text()), real - 2);
    }

    #[test]
    fn fake_copies_go_where_no_note_is() {
        let mut e = tko();
        pick(&mut e, 2);
        let n = e.add_fake_copies(0.5).unwrap();
        assert!(n >= 1);
        assert!(e.selected().iter().all(|o| o.fake));
        let map = crate::beatmap::from_text(&e.chart.to_text()).unwrap();
        assert_eq!(map.fakes.len(), n);
        // A plain .osu has nowhere to keep them.
        let mut plain = Editor::new(Chart::parse(MAP), std::path::PathBuf::from("a.osu"));
        pick(&mut plain, 1);
        assert!(plain.toggle_fake().is_err());
    }

    #[test]
    fn the_motion_example_lays_all_four_parts_on_a_busy_stretch() {
        let mut text = crate::editor::chart::tests::MAP
            .split("[HitObjects]")
            .next()
            .unwrap()
            .to_string()
            + "[HitObjects]\r\n";
        // A note every half beat for 16 beats (a 500 ms beat), dons and kats in turn.
        for k in 0..32 {
            text += &format!(
                "256,192,{},1,{},0:0:0:0:\r\n",
                k * 250,
                if k % 2 == 1 { 2 } else { 0 }
            );
        }
        let mut e = Editor::new(Chart::parse(&text), std::path::PathBuf::from("a.tko"));
        e.chart.gimmick = Some(Default::default());
        assert_eq!(e.motion_example(0.0).map(|p| p.len()), Ok(4));
        let g = e.gimmick().unwrap();
        let names: Vec<&str> = g.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            ["Example: path", "Example: pop in", "Example: swapped"]
        );
        assert!(g.groups[0].path.is_some());
        assert!(!g.groups[1].before.is_empty());
        assert!(!g.groups[2].show.is_true());
        assert!(!g.scroll.is_empty());
        assert!(
            g.field_track(crate::tko::gimmick::FieldProp::Drum)
                .is_some()
        );
        assert!(e.selection.is_empty());
        // Half a beat after each note is another note here, so no fake fits.
        let map = crate::beatmap::from_text(&e.chart.to_text()).unwrap();
        assert_eq!(map.notes.len(), 32);
        // Without notes there is nothing to show it on.
        let mut empty = tko();
        assert!(empty.motion_example(20_000.0).is_err());
    }

    #[test]
    fn any_number_of_fakes_stack_on_one_millisecond_and_on_a_note() {
        let mut e = tko();
        let note = e.chart.objects[0].time;
        // Five fakes on the first note's millisecond, all kept; a second note there is refused.
        for k in 0..5 {
            assert!(e.place_note(note, k % 2 == 1, true).is_some());
        }
        assert!(e.place_note(note, false, false).is_none());
        let on_it = |e: &Editor| e.chart.objects.iter().filter(|o| o.time == note).count();
        assert_eq!(on_it(&e), 6);
        // Saved and read back, every one of them is still there.
        let back = Chart::parse_tko(&e.chart.to_text()).unwrap();
        assert_eq!(back.objects.iter().filter(|o| o.time == note).count(), 6);
        // Fakes dragged elsewhere and back onto the stack go; the note never onto another note.
        let fakes: Vec<u64> = e
            .chart
            .objects
            .iter()
            .filter(|o| o.fake)
            .map(|o| o.id)
            .collect();
        e.selection = fakes[..2].iter().copied().collect();
        assert!(e.move_selected(30.0, false));
        assert!(e.move_selected(-30.0, false));
        assert_eq!(on_it(&e), 6);
        let second = e
            .chart
            .objects
            .iter()
            .find(|o| !o.fake && o.time > note)
            .unwrap()
            .clone();
        e.selection = [second.id].into();
        assert!(!e.move_selected(note - second.time, false));
        // Fakes made notes: only one may stand where no note is.
        assert!(e.place_note(note + 7.0, false, true).is_some());
        assert!(e.place_note(note + 7.0, true, true).is_some());
        e.selection = e
            .chart
            .objects
            .iter()
            .filter(|o| o.time == note + 7.0)
            .map(|o| o.id)
            .collect();
        assert_eq!(e.toggle_fake().unwrap(), (1, false));
    }

    #[test]
    fn a_fake_leaving_a_notes_millisecond_leaves_the_note_in_its_group() {
        let mut e = tko();
        let note = e.chart.objects[0].time;
        e.edit_gimmick(false, |g| {
            g.groups.push(crate::tko::gimmick::Group {
                notes: vec![note],
                ..Default::default()
            })
        });
        let fake = e.place_note(note, false, true).unwrap();
        e.selection = [fake].into();
        assert!(e.move_selected(40.0, false));
        let g = &e.gimmick().unwrap().groups[0];
        // The note keeps its group, and the fake that was in it by its time goes on in it.
        assert_eq!(g.notes, vec![note, note + 40.0]);
    }

    #[test]
    fn a_map_with_fakes_saves_and_reads_back() {
        let mut e = tko();
        pick(&mut e, 2);
        e.toggle_fake().unwrap();
        let dir = std::env::temp_dir().join(format!("taiko-rs-fakes-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.tko");
        let saved = e.chart.save(&path, false);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(saved, Ok(()));
        let back = Chart::parse_tko(&text).unwrap();
        assert_eq!(back.objects.iter().filter(|o| o.fake).count(), 2);
    }

    #[test]
    fn fake_copies_fly_with_their_notes_group() {
        let mut e = tko();
        pick(&mut e, 1);
        let g = e.motion_group().unwrap();
        e.set_path_preset(g, 2);
        let groups = |e: &Editor| e.gimmick().unwrap().groups[g].notes.len();
        assert_eq!(groups(&e), 1);
        assert_eq!(e.add_fake_copies(0.5), Ok(1));
        assert_eq!(groups(&e), 2, "the copy joins the group");
        // The copy and its place in the group are one undo step.
        assert!(e.undo());
        assert_eq!(groups(&e), 1);
        assert!(e.chart.objects.iter().all(|o| !o.fake));
    }
}
