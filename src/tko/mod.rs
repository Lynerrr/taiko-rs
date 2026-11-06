//! `.tko`: maps that use the client's own logic on top of an osu!-compatible chart.
//!
//! A `.tko` file is an `.osu` file (same sections, same timing and hit objects, so stars, pp and
//! the editor's stable guarantees all carry over) with two additions: `TkoVersion: n` in
//! `[Metadata]`, and a trailing `[Gimmick]` section holding one line of JSON.
//!
//! In version 1 a gimmick is **visual only**: keyframed motion of the playfield and of groups of
//! notes. It never changes when a note is hit or how it is judged, so scores, pp and replays of a
//! `.tko` map are as fair as those of its `.osu`.

pub mod clips;
pub mod format;
pub mod gimmick;
pub mod graph;
pub mod recipes;
pub mod runtime;
pub mod screen;
pub mod track;

/// The newest format this build reads and writes. Raising it needs a migration in
/// `format::MIGRATIONS` and a fixture of the old format in its tests.
pub const TKO_VERSION: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::tests::MAP;
    use crate::tko::gimmick::{FieldProp, Gimmick};
    use crate::tko::track::{Ease, Key};

    fn gimmick() -> Gimmick {
        let mut g = Gimmick::default();
        g.key_field(
            FieldProp::Rotation,
            Key {
                t: 1000.0,
                v: 90.0,
                ease: Ease::QuadOut,
            },
        );
        g
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("taiko-rs-tko-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// What every other part of the client does with a map file works the same on the `.tko`: its
    /// header is read for song select, rosu-pp rates it, and the chart is the `.osu`'s.
    #[test]
    fn a_tko_is_rated_and_listed_like_its_osu() {
        let d = dir("rated");
        let osu = d.join("a.osu");
        let tko = d.join("a.tko");
        std::fs::write(&osu, MAP).unwrap();
        std::fs::write(&tko, format::write(MAP, &gimmick()).unwrap()).unwrap();
        let (m_osu, m_tko) = (
            crate::beatmap::read_meta_file(&osu).unwrap(),
            crate::beatmap::read_meta_file(&tko).unwrap(),
        );
        assert_eq!((m_osu.tko_version, m_tko.tko_version), (0, TKO_VERSION));
        assert_eq!(
            (&m_osu.title, &m_osu.version, m_osu.od),
            (&m_tko.title, &m_tko.version, m_tko.od)
        );
        let (a, b) = (
            crate::stars::star_rating(&osu, 1.0),
            crate::stars::star_rating(&tko, 1.0),
        );
        assert!(a.is_some(), "the .osu is rated");
        assert_eq!(a, b, "the .tko is rated the same");
        let (lo, lt) = (
            crate::beatmap::load(&osu).unwrap(),
            crate::beatmap::load(&tko).unwrap(),
        );
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(lo.notes.len(), lt.notes.len());
        assert!(
            lo.notes
                .iter()
                .zip(&lt.notes)
                .all(|(x, y)| x.time == y.time),
            "the same notes at the same times"
        );
        assert!(lo.gimmick.is_none() && lt.gimmick.is_some());
    }

    #[test]
    fn a_tko_without_its_version_or_from_the_future_is_refused_with_a_reason() {
        let d = dir("refused");
        let bare = d.join("bare.tko");
        std::fs::write(&bare, MAP).unwrap();
        let future = d.join("future.tko");
        std::fs::write(
            &future,
            format::write(MAP, &gimmick())
                .unwrap()
                .replace(&format!("TkoVersion: {TKO_VERSION}"), "TkoVersion: 7"),
        )
        .unwrap();
        let (a, b) = (crate::beatmap::load(&bare), crate::beatmap::load(&future));
        let _ = std::fs::remove_dir_all(&d);
        assert!(a.err().unwrap().contains("TkoVersion"));
        let why = b.err().unwrap();
        assert!(
            why.contains("newer version") && why.contains("Update"),
            "{why}"
        );
    }

    #[test]
    fn the_library_finds_tko_files_next_to_osu_files() {
        let d = dir("library");
        std::fs::create_dir_all(d.join("set")).unwrap();
        std::fs::write(d.join("set/a.osu"), MAP).unwrap();
        std::fs::write(d.join("set/a.tko"), format::write(MAP, &gimmick()).unwrap()).unwrap();
        std::fs::write(d.join("set/notes.txt"), "x").unwrap();
        let mut found = Vec::new();
        crate::library::collect_map_files(&d.join("set"), 0, &mut found);
        let _ = std::fs::remove_dir_all(&d);
        let mut names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["a.osu", "a.tko"]);
    }

    #[test]
    fn this_build_reads_exactly_the_versions_it_has_migrations_for() {
        assert_eq!(format::MIGRATIONS.len() as u32 + 1, TKO_VERSION);
    }
}
