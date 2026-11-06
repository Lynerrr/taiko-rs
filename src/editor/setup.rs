//! Song setup, like osu!stable's: the map's metadata and its difficulty settings (HP drain,
//! overall difficulty, the base slider velocity), each change one undo step.

use super::chart::fmt_num;
use super::history::Command;
use super::state::Editor;

/// One setting of the map, a `Key:Value` line of `[Metadata]` or `[Difficulty]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// Romanised (plain Latin letters), as osu! lists it.
    Artist,
    /// In its own script.
    ArtistUnicode,
    Title,
    TitleUnicode,
    Creator,
    /// The difficulty's name.
    Version,
    Source,
    Tags,
    Hp,
    Od,
    /// `SliderMultiplier`: how fast notes scroll at 1x, and what drumroll lengths are read by.
    BaseSv,
}

/// Rolls written shorter than this are fake sliders (`stars::TINY_SLIDER_LENGTH`): their lengths
/// are a gimmick's, never a duration to keep.
const TINY: f64 = 0.1;

impl Key {
    pub fn section(self) -> &'static str {
        if self.is_text() {
            "Metadata"
        } else {
            "Difficulty"
        }
    }

    /// The key as the file writes it.
    pub fn name(self) -> &'static str {
        match self {
            Key::Artist => "Artist",
            Key::ArtistUnicode => "ArtistUnicode",
            Key::Title => "Title",
            Key::TitleUnicode => "TitleUnicode",
            Key::Creator => "Creator",
            Key::Version => "Version",
            Key::Source => "Source",
            Key::Tags => "Tags",
            Key::Hp => "HPDrainRate",
            Key::Od => "OverallDifficulty",
            Key::BaseSv => "SliderMultiplier",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Key::Artist => "Artist (romanised)",
            Key::ArtistUnicode => "Artist",
            Key::Title => "Title (romanised)",
            Key::TitleUnicode => "Title",
            Key::Creator => "Mapper",
            Key::Version => "Difficulty name",
            Key::Source => "Source",
            Key::Tags => "Tags",
            Key::Hp => "HP drain",
            Key::Od => "Accuracy (OD)",
            Key::BaseSv => "Base SV",
        }
    }

    pub fn is_text(self) -> bool {
        !matches!(self, Key::Hp | Key::Od | Key::BaseSv)
    }

    /// The most characters a text can have.
    pub fn max_len(self) -> usize {
        if self == Key::Tags { 400 } else { 120 }
    }

    /// Lowest, highest and the value osu! reads when the line is missing.
    fn range(self) -> (f64, f64, f64) {
        match self {
            Key::BaseSv => (0.4, 3.6, 1.4),
            _ => (0.0, 10.0, 5.0),
        }
    }

    /// One press of - or +.
    pub fn step(self) -> f64 {
        if self == Key::BaseSv { 0.1 } else { 0.5 }
    }
}

/// `v` kept in range and rounded to what the file is worth writing.
fn tidy(k: Key, v: f64) -> f64 {
    let (lo, hi, _) = k.range();
    (v.clamp(lo, hi) * 100.0).round() / 100.0
}

impl Editor {
    /// The setting as it is now ("" for an empty text; a number missing from the file reads as
    /// osu!'s default).
    pub fn setting(&self, k: Key) -> String {
        if k.is_text() {
            return self
                .chart
                .get_key(k.section(), k.name())
                .unwrap_or_default();
        }
        fmt_num(self.setting_number(k))
    }

    pub fn setting_number(&self, k: Key) -> f64 {
        let (lo, hi, default) = k.range();
        let v = self.chart.number(k.section(), k.name(), default);
        if v.is_finite() {
            v.clamp(lo, hi)
        } else {
            default
        }
    }

    /// Sets `k` to `value` (typed): one undo step. False when nothing changed.
    pub fn set_setting(&mut self, k: Key, value: &str) -> Result<bool, String> {
        if k.is_text() {
            let v: String = value
                .replace(['\r', '\n'], " ")
                .trim()
                .chars()
                .take(k.max_len())
                .collect();
            return Ok(self.set_text(k, v));
        }
        let v: f64 = value.trim().parse().map_err(|_| "not a number")?;
        if !v.is_finite() {
            return Err("not a number".into());
        }
        Ok(self.set_number(k, v))
    }

    /// Moves a number by `steps` presses.
    pub fn step_setting(&mut self, k: Key, steps: f64) -> bool {
        if k.is_text() {
            return false;
        }
        let now = self.setting_number(k);
        // Off the grid of this step (typed in), the first press lands on it.
        let step = k.step() * steps.abs();
        let next = if steps > 0.0 {
            ((now + 1e-9) / step).floor() * step + step
        } else {
            ((now - 1e-9) / step).ceil() * step - step
        };
        self.set_number(k, next)
    }

    fn set_text(&mut self, k: Key, v: String) -> bool {
        let mut steps = Vec::new();
        let mut put = |ed: &Editor, k: Key, v: &str| {
            let before = ed.chart.get_key(k.section(), k.name());
            if before.as_deref().unwrap_or("") != v {
                steps.push(Command::SetKey {
                    section: k.section(),
                    key: k.name(),
                    before,
                    after: Some(v.to_string()),
                });
            }
        };
        put(self, k, &v);
        // The romanised name and the one in its own script go together while they are the same
        // (or one is empty), as osu!stable fills them: typing one fills the other too. Only
        // plain Latin letters go in a romanised one.
        let pair = match k {
            Key::Artist => Some(Key::ArtistUnicode),
            Key::ArtistUnicode => Some(Key::Artist),
            Key::Title => Some(Key::TitleUnicode),
            Key::TitleUnicode => Some(Key::Title),
            _ => None,
        };
        if let Some(other) = pair {
            let was = self.setting(k);
            let theirs = self.setting(other);
            let romanised = matches!(other, Key::Artist | Key::Title);
            if (theirs.is_empty() || theirs == was) && (!romanised || v.is_ascii()) {
                put(self, other, &v);
            }
        }
        if steps.is_empty() {
            return false;
        }
        let cmd = if steps.len() == 1 {
            steps.remove(0)
        } else {
            Command::Composite(steps)
        };
        self.history.run(&mut self.chart, cmd);
        true
    }

    fn set_number(&mut self, k: Key, v: f64) -> bool {
        let v = tidy(k, v);
        let now = self.setting_number(k);
        let before = self.chart.get_key(k.section(), k.name());
        if (v - now).abs() < 1e-9 && before.is_some() {
            return false;
        }
        let mut steps = vec![Command::SetKey {
            section: k.section(),
            key: k.name(),
            before,
            after: Some(fmt_num(v)),
        }];
        // Drumrolls are written in osu!pixels, which the base SV turns into time: they are
        // rewritten to last as long as they did.
        if k == Key::BaseSv && now > 0.0 {
            let scale = v / now;
            let (mut before, mut after) = (Vec::new(), Vec::new());
            for o in &self.chart.objects {
                let Some(len) = o.length().filter(|l| l.is_finite() && *l > TINY) else {
                    continue;
                };
                let mut a = o.clone();
                a.set_length(((len * scale * 1000.0).round() / 1000.0).max(TINY * 2.0));
                before.push(o.clone());
                after.push(a);
            }
            if !before.is_empty() {
                steps.push(Command::EditObjects { before, after });
            }
        }
        self.history.run(&mut self.chart, Command::Composite(steps));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::chart::Chart;
    use super::super::chart::tests::MAP;
    use super::*;

    fn editor() -> Editor {
        Editor::new(Chart::parse(MAP), "x.osu".into())
    }

    #[test]
    fn metadata_and_difficulty_are_set_and_undone_in_one_step_each() {
        let mut ed = editor();
        let text = ed.chart.to_text();
        assert!(ed.set_setting(Key::Version, "  Inner\n Oni  ").unwrap());
        assert_eq!(ed.setting(Key::Version), "Inner  Oni");
        assert!(ed.set_setting(Key::Od, "6.25").unwrap());
        assert_eq!(ed.setting(Key::Od), "6.25");
        assert!(ed.set_setting(Key::Hp, "12").unwrap(), "kept in range");
        assert_eq!(ed.setting(Key::Hp), "10");
        assert!(ed.set_setting(Key::Od, "x").is_err());
        assert!(
            !ed.set_setting(Key::Version, "Inner  Oni").unwrap(),
            "no change"
        );
        assert!(ed.modified());
        for _ in 0..3 {
            assert!(ed.history.undo(&mut ed.chart));
        }
        assert_eq!(ed.chart.to_text(), text, "undone to the byte");
        assert!(!ed.modified());
    }

    #[test]
    fn steps_land_on_the_grid_and_stop_at_the_ends() {
        let mut ed = editor();
        ed.set_setting(Key::Od, "5.2").unwrap();
        ed.step_setting(Key::Od, 1.0);
        assert_eq!(ed.setting(Key::Od), "5.5");
        ed.step_setting(Key::Od, -1.0);
        assert_eq!(ed.setting(Key::Od), "5");
        for _ in 0..30 {
            ed.step_setting(Key::Od, 1.0);
        }
        assert_eq!(ed.setting(Key::Od), "10");
        ed.set_setting(Key::BaseSv, "1.4").unwrap();
        ed.step_setting(Key::BaseSv, 1.0);
        assert_eq!(ed.setting(Key::BaseSv), "1.5");
        // Fine steps keep going (a fifth of a step for OD, a tenth for SV).
        ed.set_setting(Key::Od, "5").unwrap();
        ed.step_setting(Key::Od, 0.2);
        ed.step_setting(Key::Od, 0.2);
        assert_eq!(ed.setting(Key::Od), "5.2");
        ed.step_setting(Key::BaseSv, -0.1);
        ed.step_setting(Key::BaseSv, -0.1);
        assert_eq!(ed.setting(Key::BaseSv), "1.48");
    }

    #[test]
    fn a_key_the_file_lacks_is_added_and_undo_takes_it_out() {
        let mut ed = editor();
        ed.chart.remove_key("Metadata", "Source");
        let text = ed.chart.to_text();
        assert_eq!(ed.setting(Key::Source), "");
        ed.set_setting(Key::Source, "Original").unwrap();
        assert_eq!(
            ed.chart.get_key("Metadata", "Source").as_deref(),
            Some("Original")
        );
        ed.history.undo(&mut ed.chart);
        assert_eq!(ed.chart.to_text(), text);
    }

    #[test]
    fn the_romanised_name_follows_while_they_match() {
        let mut ed = editor();
        ed.set_setting(Key::Artist, "").unwrap();
        ed.set_setting(Key::ArtistUnicode, "").unwrap();
        ed.set_setting(Key::Artist, "Camellia").unwrap();
        assert_eq!(ed.setting(Key::ArtistUnicode), "Camellia");
        // Not plain Latin: the romanised one stays.
        ed.set_setting(Key::ArtistUnicode, "かめりあ").unwrap();
        assert_eq!(ed.setting(Key::Artist), "Camellia");
        // Now they differ: each goes on its own.
        ed.set_setting(Key::Artist, "Kamelia").unwrap();
        assert_eq!(ed.setting(Key::ArtistUnicode), "かめりあ");
        ed.history.undo(&mut ed.chart);
        assert_eq!(ed.setting(Key::Artist), "Camellia");
    }

    #[test]
    fn a_new_base_sv_keeps_every_drumroll_as_long_as_it_was() {
        let mut ed = editor();
        let ends = |ed: &Editor| -> Vec<f64> {
            ed.chart
                .objects
                .iter()
                .filter(|o| o.length().is_some())
                .map(|o| ed.end_of(o))
                .collect()
        };
        let before = ends(&ed);
        assert!(!before.is_empty(), "the test map has a roll");
        ed.set_setting(Key::BaseSv, "2.1").unwrap();
        for (a, b) in before.iter().zip(ends(&ed)) {
            assert!((a - b).abs() < 1.0, "{a} vs {b}");
        }
        ed.history.undo(&mut ed.chart);
        assert_eq!(ends(&ed), before);
    }
}
