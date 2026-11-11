//! The player's data in `taiko.db` (`store.rs`): each kind written and read back the same,
//! through a real file that is closed and opened again, as the game will between launches.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::scores::ScoreRow;
use crate::store::{self, Store};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("taiko-rs-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn row(replays: &Path, n: u64, pp: Option<f64>) -> ScoreRow {
    ScoreRow {
        // The top bit set: hashes are u64s, SQLite's integers i64s.
        map_hash: 0xF00D_0000_0000_0000 | n,
        date: 1_700_000_000 + n,
        score: 1_000_000 - n * 1000,
        accuracy: 0.987_654_321,
        max_combo: 400 + n as u32,
        miss: n as u32,
        rate: 1.5,
        player: format!("Player {n} ✓"),
        file: replays.join(format!("{}-{:016x}.json", 1_700_000_000 + n, n)),
        mods: if n.is_multiple_of(2) {
            "HDHR".into()
        } else {
            String::new()
        },
        pp,
    }
}

#[test]
fn everything_written_reads_back_the_same_after_reopening() {
    let dir = temp_dir("roundtrip");
    let path = dir.join(store::FILE);
    let replays = dir.join("replays");

    let rows = vec![
        row(&replays, 3, Some(312.34)),
        row(&replays, 1, None),
        row(&replays, 2, Some(0.0)),
    ];
    let plays: HashMap<PathBuf, u64> = [
        (PathBuf::from("/songs/1 a/a [x].osu"), 1_700_000_100),
        (PathBuf::from("C:\\Songs\\2 б\\б [y].osu"), 1_700_000_200),
    ]
    .into();
    let songs: HashSet<PathBuf> = [PathBuf::from("/songs/1 a")].into();
    let maps: HashSet<PathBuf> = [
        PathBuf::from("/songs/2 b/b [hard].osu"),
        PathBuf::from("/songs/3 c/c [日本].osu"),
    ]
    .into();
    let offsets: HashMap<u64, f64> = [(u64::MAX, -12.5), (7, 3.0), (0, 0.25)].into();
    let skins_off: HashSet<String> = ["1 a".into(), "4 d (tab\tin name)".into()].into();
    let favourites: HashSet<String> = ["1 a/audio.mp3".into(), "2 б/song.ogg".into()].into();

    {
        let mut s = Store::open(&path).unwrap();
        assert_eq!(s.version().unwrap(), store::VERSION);
        s.add_score(&rows[0]).unwrap();
        s.add_score(&rows[1]).unwrap();
        s.add_score(&rows[2]).unwrap();
        for (p, &at) in &plays {
            s.add_play(p, at).unwrap();
            // An earlier play of the same map doesn't make it older.
            s.add_play(p, at - 50).unwrap();
        }
        s.replace_hidden(&songs, &maps).unwrap();
        s.replace_map_offsets(&offsets).unwrap();
        s.replace_map_skins_off(&skins_off).unwrap();
        s.replace_favourites(&favourites).unwrap();
    }

    let mut s = Store::open(&path).unwrap();
    assert_eq!(
        s.scores(&replays).unwrap(),
        rows,
        "scores, in the order they were added"
    );
    assert_eq!(s.plays().unwrap(), plays);
    assert_eq!(s.hidden().unwrap(), (songs, maps));
    assert_eq!(s.map_offsets().unwrap(), offsets);
    assert_eq!(s.map_skins_off().unwrap(), skins_off);
    assert_eq!(s.favourites().unwrap(), favourites);

    // Written whole: what's left out is gone, nothing else changes.
    let fewer = vec![rows[2].clone(), rows[0].clone()];
    s.replace_scores(&fewer).unwrap();
    s.replace_map_offsets(&[(7, -1.0)].into()).unwrap();
    s.replace_favourites(&HashSet::new()).unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(s.scores(&replays).unwrap(), fewer);
    assert_eq!(s.map_offsets().unwrap(), [(7, -1.0)].into());
    assert!(s.favourites().unwrap().is_empty());
    assert_eq!(s.map_skins_off().unwrap(), skins_off);
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scores_read_from_another_folder_point_into_it() {
    // The data folder can move (settings): replays are found by name in the new one.
    let s = Store::in_memory().unwrap();
    s.add_score(&row(Path::new("/old/replays"), 1, None))
        .unwrap();
    let moved = s.scores(Path::new("/new/replays")).unwrap();
    assert_eq!(
        moved[0].file,
        Path::new("/new/replays").join(row(Path::new(""), 1, None).file)
    );
}
