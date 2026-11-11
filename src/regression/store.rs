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

// Bringing the old files in (`store::import`).

use crate::store::import::{self, Files};

/// The old files of a player who has used the game for a while, the way they look on disk:
/// a line from before pp, a line from before mods, garbage, and a last line cut short by a
/// crash while it was being written.
fn old_files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let replays = dir.join("replays");
    std::fs::create_dir_all(&replays).unwrap();
    // Two of the three plays still have their replay.
    std::fs::write(replays.join("1700000001-00000000000000aa.json"), "{}").unwrap();
    std::fs::write(replays.join("1700000003-f00d0000000000cc.json"), "{}").unwrap();
    let files: Vec<(PathBuf, &[u8])> = vec![
        (
            replays.join("scores.tsv"),
            b"00000000000000aa\t1700000001\t912345\t0.981234\t512\t3\t1.00\tPlayer\t1700000001-00000000000000aa.json\tHDHR\t312.45\n\
              00000000000000bb\t1700000002\t500000\t0.900000\t100\t20\t1.50\tPlayer\t1700000002-00000000000000bb.json\n\
              not a score line\n\
              f00d0000000000cc\t1700000003\t1000000\t1.000000\t800\t0\t0.75\t\xd0\x98\xd0\xb3\xd1\x80\xd0\xbe\xd0\xba\t1700000003-f00d0000000000cc.json\t\t\n\
              00000000000000dd\t17000",
        ),
        (
            dir.join("plays.tsv"),
            b"1700000100\t/songs/1 a/a [x].osu\n1700000300\t/songs/2 b/b [y].osu\n\
              1700000200\t/songs/1 a/a [x].osu\nbroken\n",
        ),
        (
            dir.join("hidden.tsv"),
            b"song\t/songs/3 c\nmap\t/songs/2 b/b [hard].osu\nsomething else\n",
        ),
        (
            dir.join("map_offsets_osu.tsv"),
            b"00000000000000aa\t-12.5\nffffffffffffffff\t4\nzz\t1\n",
        ),
        (dir.join("map_skins_off.txt"), b"1 a\n\n3 c\n"),
        (
            dir.join("favourite-songs.txt"),
            b"1 a/audio.mp3\n2 b/song.ogg\n",
        ),
    ];
    for (path, bytes) in &files {
        std::fs::write(path, bytes).unwrap();
    }
    files.into_iter().map(|(p, b)| (p, b.to_vec())).collect()
}

/// What the game read from `old_files`.
fn expected(dir: &Path) -> Files {
    let replays = dir.join("replays");
    let score =
        |hash: u64, n: u64, score, accuracy, combo, miss, rate, player: &str, mods: &str, pp| {
            ScoreRow {
                map_hash: hash,
                date: 1_700_000_000 + n,
                score,
                accuracy,
                max_combo: combo,
                miss,
                rate,
                player: player.into(),
                file: replays.join(format!("{}-{hash:016x}.json", 1_700_000_000 + n)),
                mods: mods.into(),
                pp,
            }
        };
    Files {
        scores: vec![
            score(
                0xaa,
                1,
                912_345,
                0.981_234,
                512,
                3,
                1.0,
                "Player",
                "HDHR",
                Some(312.45),
            ),
            score(0xbb, 2, 500_000, 0.9, 100, 20, 1.5, "Player", "", None),
            score(
                0xf00d_0000_0000_00cc,
                3,
                1_000_000,
                1.0,
                800,
                0,
                0.75,
                "Игрок",
                "",
                None,
            ),
        ],
        plays: [
            (PathBuf::from("/songs/1 a/a [x].osu"), 1_700_000_200),
            (PathBuf::from("/songs/2 b/b [y].osu"), 1_700_000_300),
        ]
        .into(),
        hidden_songs: [PathBuf::from("/songs/3 c")].into(),
        hidden_maps: [PathBuf::from("/songs/2 b/b [hard].osu")].into(),
        map_offsets: [(0xaa, -12.5), (u64::MAX, 4.0)].into(),
        map_skins_off: ["1 a".into(), "3 c".into()].into(),
        favourites: ["1 a/audio.mp3".into(), "2 b/song.ogg".into()].into(),
    }
}

/// Every file in `dir`, with its contents.
fn snapshot_dir(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_old_files_come_in_once_and_read_back_as_the_game_read_them() {
    let data = temp_dir("import");
    let files = old_files(&data);
    let before = snapshot_dir(&data);
    let db = temp_dir("import-db").join(store::FILE);

    let mut s = Store::open(&db).unwrap();
    assert!(!import::imported(&s).unwrap());
    assert_eq!(import::read_files(&data).unwrap(), expected(&data));
    assert!(import::import(&mut s, &data, 1_700_001_000).unwrap());
    drop(s);

    // The next launch: the database alone holds the same as the files did.
    let s = Store::open(&db).unwrap();
    assert!(import::imported(&s).unwrap());
    let back = import::read_store(&s, &data.join("replays")).unwrap();
    assert_eq!(back, expected(&data));
    // Song select's leaderboard: the plays whose replay is there, as `scores::load_index` gave.
    let shown: Vec<u64> = back
        .scores
        .iter()
        .filter(|r| r.file.is_file())
        .map(|r| r.map_hash)
        .collect();
    assert_eq!(shown, [0xaa, 0xf00d_0000_0000_00cc]);
    assert_eq!(
        s.meta(import::IMPORTED).unwrap().as_deref(),
        Some("1700001000")
    );

    // The files weren't touched, and nothing was added next to them.
    assert_eq!(snapshot_dir(&data), before);
    assert_eq!(files.len(), 6);
    drop(s);
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

#[test]
fn a_second_launch_doesnt_bring_the_files_in_again() {
    let data = temp_dir("import-twice");
    old_files(&data);
    let db = temp_dir("import-twice-db").join(store::FILE);
    let mut s = Store::open(&db).unwrap();
    assert!(import::import(&mut s, &data, 1).unwrap());
    // Played on since: in the database.
    s.add_play(Path::new("/songs/9 new/n.osu"), 1_700_009_000)
        .unwrap();
    drop(s);
    // Meanwhile an older build wrote to the old files.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(data.join("plays.tsv"))
        .unwrap();
    std::io::Write::write_all(&mut f, b"1700005000\t/songs/8 old/o.osu\n").unwrap();
    std::fs::write(data.join("favourite-songs.txt"), "").unwrap();
    drop(f);

    let mut s = Store::open(&db).unwrap();
    assert!(!import::import(&mut s, &data, 2).unwrap(), "imported again");
    let plays = s.plays().unwrap();
    assert!(plays.contains_key(Path::new("/songs/9 new/n.osu")));
    assert!(!plays.contains_key(Path::new("/songs/8 old/o.osu")));
    assert_eq!(s.favourites().unwrap(), expected(&data).favourites);
    assert_eq!(s.meta(import::IMPORTED).unwrap().as_deref(), Some("1"));
    drop(s);
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

#[test]
fn a_file_that_cant_be_read_stops_the_import_and_keeps_nothing() {
    let data = temp_dir("import-bad");
    old_files(&data);
    // Not text: the game took it for no hidden maps; the import doesn't guess.
    std::fs::write(data.join("hidden.tsv"), b"song\t/songs/\xff\xfe\n").unwrap();
    let before = snapshot_dir(&data);
    let db = temp_dir("import-bad-db").join(store::FILE);

    let mut s = Store::open(&db).unwrap();
    let e = import::import(&mut s, &data, 1).unwrap_err();
    assert!(e.contains("hidden.tsv"), "{e}");
    assert!(!import::imported(&s).unwrap());
    assert_eq!(
        import::read_store(&s, &data.join("replays")).unwrap(),
        Files::default()
    );
    assert_eq!(snapshot_dir(&data), before, "the files were touched");
    drop(s);

    // Mended: the next launch brings everything in.
    std::fs::write(data.join("hidden.tsv"), "song\t/songs/3 c\n").unwrap();
    let mut s = Store::open(&db).unwrap();
    assert!(import::import(&mut s, &data, 2).unwrap());
    let back = import::read_store(&s, &data.join("replays")).unwrap();
    assert_eq!(back.scores, expected(&data).scores);
    assert_eq!(back.hidden_songs, [PathBuf::from("/songs/3 c")].into());
    assert!(back.hidden_maps.is_empty());
    drop(s);
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

#[test]
fn a_player_without_old_files_starts_with_an_empty_database() {
    let data = temp_dir("import-none");
    let mut s = Store::in_memory().unwrap();
    assert!(import::import(&mut s, &data, 5).unwrap());
    assert!(import::imported(&s).unwrap());
    assert_eq!(
        import::read_store(&s, &data.join("replays")).unwrap(),
        Files::default()
    );
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn offsets_from_before_osus_sign_come_in_turned_round() {
    let data = temp_dir("import-old-sign");
    std::fs::write(data.join("map_offsets.tsv"), "00000000000000aa\t10\n").unwrap();
    let mut s = Store::in_memory().unwrap();
    assert!(import::import(&mut s, &data, 1).unwrap());
    assert_eq!(s.map_offsets().unwrap(), [(0xaa, -10.0)].into());
    // As read; the game's own reader renames that file, the import leaves it.
    assert!(data.join("map_offsets.tsv").is_file());
    let _ = std::fs::remove_dir_all(&data);
}
