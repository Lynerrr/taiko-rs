//! Reading and writing the `.tko` file: an `.osu` with `TkoVersion` in `[Metadata]` and a trailing
//! `[Gimmick]` section of one JSON line.
//!
//! Versions: a file newer than this build is refused with a message saying to update (never
//! guessed at), an older one is brought up to date by the chain in `MIGRATIONS`, and one without a
//! `TkoVersion` is not a `.tko`.

use serde_json::Value;

use super::TKO_VERSION;
use super::gimmick::Gimmick;

/// The most JSON a file may carry.
pub const MAX_JSON: usize = 4 << 20;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Made by a newer game than this one.
    Newer {
        found: u32,
    },
    /// A `.tko` without `TkoVersion` in `[Metadata]`.
    MissingVersion,
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Newer { found } => write!(
                f,
                "This map needs a newer version of the game (map format v{found}, this game reads up to v{TKO_VERSION}). Update the game to play it."
            ),
            Error::MissingVersion => {
                write!(f, "Not a valid .tko map: [Metadata] has no TkoVersion.")
            }
            Error::Invalid(why) => write!(f, "Invalid .tko map: {why}"),
        }
    }
}

/// One step of the upgrade from version `n` (index `n - 1`) to `n + 1`, on the `[Gimmick]` JSON.
pub type Migration = fn(&mut Value);

/// Every version bump adds one entry here and a fixture of the old format to the tests below.
pub const MIGRATIONS: &[Migration] = &[];

const _: () = assert!(MIGRATIONS.len() as u32 == TKO_VERSION - 1);

/// Brings `json` from `from` up to `to` through `table`.
pub fn migrate_with(table: &[Migration], json: &mut Value, from: u32, to: u32) {
    for step in table
        .iter()
        .take((to - 1) as usize)
        .skip((from - 1) as usize)
    {
        step(json);
    }
}

pub struct Parsed {
    pub gimmick: Gimmick,
}

fn section_of(line: &str) -> Option<&str> {
    let t = line.trim();
    (t.len() >= 2 && t.starts_with('[') && t.ends_with(']')).then(|| &t[1..t.len() - 1])
}

/// `TkoVersion` from `[Metadata]`, if the text has one.
pub fn version_of(text: &str) -> Option<u32> {
    let mut in_meta = false;
    for line in text.lines() {
        if let Some(s) = section_of(line) {
            in_meta = s == "Metadata";
        } else if in_meta && let Some(v) = line.trim().strip_prefix("TkoVersion:") {
            return Some(v.trim().parse().unwrap_or(0));
        }
    }
    None
}

/// The text with its `[Gimmick]` section taken out, and what the section held (one JSON line).
pub fn split(text: &str) -> (String, Option<String>) {
    let mut kept = String::with_capacity(text.len());
    let mut json = None;
    let mut in_gimmick = false;
    for line in text.split_inclusive('\n') {
        if let Some(s) = section_of(line) {
            in_gimmick = s == "Gimmick";
            if in_gimmick {
                continue;
            }
        }
        if in_gimmick {
            let t = line.trim();
            if !t.is_empty() && json.is_none() {
                json = Some(t.to_string());
            }
        } else {
            kept.push_str(line);
        }
    }
    (kept, json)
}

/// Reads the gimmick of a `.tko` text.
pub fn read(text: &str) -> Result<Parsed, Error> {
    let version = version_of(text).ok_or(Error::MissingVersion)?;
    if version == 0 {
        return Err(Error::Invalid("TkoVersion isn't a number above 0".into()));
    }
    if version > TKO_VERSION {
        return Err(Error::Newer { found: version });
    }
    let (_, json) = split(text);
    let mut gimmick = match json {
        None => Gimmick::default(),
        Some(json) => {
            if json.len() > MAX_JSON {
                return Err(Error::Invalid("the [Gimmick] section is too large".into()));
            }
            let mut value: Value = serde_json::from_str(&json)
                .map_err(|e| Error::Invalid(format!("[Gimmick]: {e}")))?;
            migrate_with(MIGRATIONS, &mut value, version, TKO_VERSION);
            serde_json::from_value(value).map_err(|e| Error::Invalid(format!("[Gimmick]: {e}")))?
        }
    };
    gimmick.sanitize();
    Ok(Parsed { gimmick })
}

/// `osu` (a chart as `.osu` text) with `TkoVersion` set in `[Metadata]` and the gimmick written as
/// the last section. An existing `[Gimmick]` section is replaced.
pub fn write(osu: &str, gimmick: &Gimmick) -> Result<String, Error> {
    let (base, _) = split(osu);
    let eol = if base.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(base.len() + 256);
    let (mut in_meta, mut seen_meta, mut wrote) = (false, false, false);
    let mut pending_blank = String::new();
    let version_line = format!("TkoVersion: {TKO_VERSION}{eol}");
    let finish_meta = |out: &mut String, pending: &mut String| {
        out.push_str(&version_line);
        out.push_str(pending);
        pending.clear();
    };
    for line in base.split_inclusive('\n') {
        if let Some(s) = section_of(line) {
            if in_meta && !wrote {
                finish_meta(&mut out, &mut pending_blank);
                wrote = true;
            }
            out.push_str(&pending_blank);
            pending_blank.clear();
            in_meta = s == "Metadata";
            seen_meta |= in_meta;
            out.push_str(line);
            continue;
        }
        if in_meta {
            let t = line.trim();
            if t.starts_with("TkoVersion:") {
                if !wrote {
                    finish_meta(&mut out, &mut pending_blank);
                    wrote = true;
                }
                continue;
            }
            if t.is_empty() {
                pending_blank.push_str(line);
                continue;
            }
        }
        out.push_str(&pending_blank);
        pending_blank.clear();
        out.push_str(line);
    }
    if in_meta && !wrote {
        if !out.ends_with('\n') {
            out.push_str(eol);
        }
        finish_meta(&mut out, &mut pending_blank);
        wrote = true;
    }
    out.push_str(&pending_blank);
    if !seen_meta || !wrote {
        return Err(Error::Invalid("the chart has no [Metadata] section".into()));
    }
    let mut g = gimmick.clone();
    g.sanitize();
    let json = serde_json::to_string(&g).map_err(|e| Error::Invalid(e.to_string()))?;
    if json.len() > MAX_JSON {
        return Err(Error::Invalid("the gimmick is too large to save".into()));
    }
    if !out.ends_with('\n') {
        out.push_str(eol);
    }
    out.push_str(eol);
    out.push_str("[Gimmick]");
    out.push_str(eol);
    out.push_str(&json);
    out.push_str(eol);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tko::gimmick::{FieldProp, Group, NoteProp};
    use crate::tko::track::{Ease, Key};

    fn chart() -> &'static str {
        crate::editor::chart::tests::MAP
    }

    fn gimmick() -> Gimmick {
        let mut g = Gimmick::default();
        g.key_field(
            FieldProp::Rotation,
            Key {
                t: 1000.0,
                v: 45.0,
                ease: Ease::CubicOut,
            },
        );
        g.groups.push(Group {
            name: "chorus".into(),
            notes: vec![1000.0, 1500.0],
            ..Group::default()
        });
        g.key_note(
            0,
            NoteProp::Dy,
            Key {
                t: 1000.0,
                v: 30.0,
                ease: Ease::Linear,
            },
        );
        g
    }

    #[test]
    fn a_gimmick_survives_write_and_read_and_the_chart_is_untouched() {
        let text = write(chart(), &gimmick()).unwrap();
        assert_eq!(version_of(&text), Some(TKO_VERSION));
        let parsed = read(&text).unwrap();
        assert_eq!(parsed.gimmick, gimmick());
        // Everything but the version line and the section is the chart as it was.
        let (back, _) = split(&text);
        let back: String = back
            .lines()
            .filter(|l| !l.starts_with("TkoVersion:"))
            .collect::<Vec<_>>()
            .join("\n");
        let orig: String = chart().lines().collect::<Vec<_>>().join("\n");
        assert_eq!(back.trim_end(), orig.trim_end());
    }

    #[test]
    fn writing_again_replaces_rather_than_stacks() {
        let once = write(chart(), &gimmick()).unwrap();
        let twice = write(&once, &Gimmick::default()).unwrap();
        assert_eq!(twice.matches("[Gimmick]").count(), 1);
        assert_eq!(twice.matches("TkoVersion:").count(), 1);
        assert!(read(&twice).unwrap().gimmick.is_empty());
    }

    #[test]
    fn line_endings_are_kept() {
        let crlf = chart().replace('\n', "\r\n");
        let text = write(&crlf, &gimmick()).unwrap();
        assert!(
            !text.replace("\r\n", "").contains('\n'),
            "a bare LF in a CRLF file"
        );
        assert!(read(&text).is_ok());
    }

    #[test]
    fn the_version_line_sits_in_metadata() {
        let text = write(chart(), &Gimmick::default()).unwrap();
        let at = text.find("TkoVersion:").unwrap();
        assert!(text[..at].rfind("[Metadata]").unwrap() > text[..at].rfind("[General]").unwrap());
        assert!(text[at..].find("[Difficulty]").unwrap() > 0);
    }

    #[test]
    fn a_newer_file_is_refused_and_says_to_update() {
        let text = write(chart(), &gimmick())
            .unwrap()
            .replace("TkoVersion: 1", "TkoVersion: 99");
        let e = read(&text).err().unwrap();
        assert_eq!(e, Error::Newer { found: 99 });
        assert!(e.to_string().contains("Update the game"));
    }

    #[test]
    fn a_file_without_a_version_is_not_a_tko() {
        assert_eq!(read(chart()).err(), Some(Error::MissingVersion));
        let bad = write(chart(), &gimmick())
            .unwrap()
            .replace("TkoVersion: 1", "TkoVersion: x");
        assert!(matches!(read(&bad), Err(Error::Invalid(_))));
    }

    #[test]
    fn broken_json_is_an_error_not_a_panic() {
        let text = write(chart(), &gimmick()).unwrap();
        let broken = text.replace("\"groups\"", "\"groups");
        assert!(matches!(read(&broken), Err(Error::Invalid(_))));
        let huge = format!(
            "{}\n[Gimmick]\n{{\"x\":\"{}\"}}\n",
            write(chart(), &Gimmick::default())
                .unwrap()
                .split("[Gimmick]")
                .next()
                .unwrap(),
            "a".repeat(MAX_JSON)
        );
        assert!(matches!(read(&huge), Err(Error::Invalid(_))));
    }

    #[test]
    fn a_file_with_a_version_and_no_section_has_an_empty_gimmick() {
        let text = write(chart(), &Gimmick::default()).unwrap();
        let (osu, _) = split(&text);
        assert!(read(&osu).unwrap().gimmick.is_empty());
    }

    #[test]
    fn the_migration_chain_runs_each_step_in_order_from_the_files_version() {
        fn one_to_two(v: &mut Value) {
            v["renamed"] = v["old"].take();
        }
        fn two_to_three(v: &mut Value) {
            v["renamed"] = Value::from(v["renamed"].as_i64().unwrap_or(0) + 1);
        }
        let table: &[Migration] = &[one_to_two, two_to_three];
        let mut v: Value = serde_json::json!({"old": 5});
        migrate_with(table, &mut v, 1, 3);
        assert_eq!(v["renamed"], 6);
        // A file already at v2 only gets the second step.
        let mut v: Value = serde_json::json!({"renamed": 5});
        migrate_with(table, &mut v, 2, 3);
        assert_eq!(v["renamed"], 6);
        // A current file is left alone.
        let mut v: Value = serde_json::json!({"renamed": 5});
        migrate_with(table, &mut v, 3, 3);
        assert_eq!(v["renamed"], 5);
    }

    #[test]
    fn a_v1_fixture_keeps_reading() {
        // The format as it was first released: pinned so a later change can't break old files.
        let fixture = r#"{"field":{"rotation":{"keys":[{"t":1000.0,"v":0.0},{"t":2000.0,"v":90.0,"ease":"back_out"}]}},"groups":[{"name":"a","notes":[1000.0],"tracks":{"opacity":{"keys":[{"t":0.0,"v":0.5}]}}}]}"#;
        let text = format!(
            "{}\n[Gimmick]\n{fixture}\n",
            write(chart(), &Gimmick::default())
                .unwrap()
                .split("[Gimmick]")
                .next()
                .unwrap()
                .trim_end()
        );
        let g = read(&text).unwrap().gimmick;
        assert_eq!(
            g.field_track(FieldProp::Rotation).unwrap().keys[1].ease,
            Ease::BackOut
        );
        assert_eq!(g.groups[0].notes, [1000.0]);
        assert_eq!(g.groups[0].track(NoteProp::Opacity).unwrap().keys[0].v, 0.5);
    }
}
