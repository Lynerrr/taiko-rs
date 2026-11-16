//! Moving taiko.rs's maps out of osu!'s Songs folder into its own: offered once over song select
//! when some are there (a website set, a `.tko` set), never osu!'s sets; Not now leaves
//! everything where it was and isn't asked again; Move takes the sets across whole, their plays
//! following them, and lists them from the new place without leaving song select; Settings >
//! Library moves them back.

use super::harness::*;
use super::maps;
use super::snapshot::assert_snapshot;
use crate::*;
use std::path::{Path, PathBuf};

/// osu!'s Songs folder with a website set, a `.tko` set and an osu! set, and taiko.rs's own
/// custom maps folder beside it, empty.
struct Folders {
    root: PathBuf,
    songs: PathBuf,
    custom: PathBuf,
    /// The website set's chart, the `.tko` set's, the osu! set's.
    site: PathBuf,
    tko: PathBuf,
    osu: PathBuf,
}

impl Folders {
    fn new(tag: &str) -> Folders {
        let root = std::env::temp_dir().join(format!(
            "taiko-rs-regression-custom-migration-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (songs, custom) = (root.join("osu!/Songs"), root.join("taiko-rs/Custom"));
        let put = |set: &str, n: usize, ext: &str| {
            let chart = &maps::all()[n];
            let path = songs.join(set).join(format!("{}.{ext}", chart.name));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, chart.text()).unwrap();
            path
        };
        let site = put("c12 Regression - Site", 0, "osu");
        std::fs::write(
            site.parent().unwrap().join(custom::SIDECAR),
            "{\"id\": 12}\n",
        )
        .unwrap();
        let tko = put("Regression - Mine", 1, "tko");
        let osu = put("622136 Regression - osu", 2, "osu");
        Folders {
            root,
            songs,
            custom,
            site,
            tko,
            osu,
        }
    }

    /// Where a chart of osu!'s Songs folder is once moved.
    fn moved(&self, chart: &Path) -> PathBuf {
        self.custom.join(chart.strip_prefix(&self.songs).unwrap())
    }
}

impl Drop for Folders {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Song select over `f`'s Songs folder (as if picked in settings) and custom folder, the move
/// not offered yet.
fn client(f: &Folders) -> (State, Instant) {
    let mut s = state_with(
        Config {
            custom_move_asked: false,
            ..config()
        },
        Vec::new(),
    );
    s.songs_dir = f.songs.clone();
    s.song_dirs = vec![f.songs.clone()];
    s.set_custom_dir(f.custom.clone());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.refresh_library(false);
    assert!(
        wait_for(&mut s, &mut now, |s| s.scan.is_none()),
        "the scan ends in time"
    );
    settle(&mut s, &mut now);
    (s, now)
}

fn listed(s: &State, path: &Path) -> bool {
    s.library
        .iter()
        .any(|e| e.path == path && !s.select.gone.contains(&e.path))
}

fn toast(s: &mut State) -> String {
    s.toast.take().map(|t| t.text).unwrap_or_default()
}

/// Waits for the offer.
fn offered(s: &mut State, now: &mut Instant) {
    assert!(
        wait_for(s, now, |s| s.select.moving.is_some()),
        "the move is offered"
    );
}

/// Waits for a move to end and the list to be read again.
fn moved(s: &mut State, now: &mut Instant) {
    assert!(
        wait_for(s, now, |s| s.custom_move.is_idle() && s.scan.is_none()),
        "the move ends in time"
    );
    settle(s, now);
}

#[test]
fn the_move_is_offered_once_for_taiko_rss_sets_only() {
    let f = Folders::new("offer");
    let (mut s, mut now) = client(&f);
    offered(&mut s, &mut now);
    let ask = s.select.moving.clone().unwrap();
    let mut sets: Vec<PathBuf> = ask.found.iter().map(|x| x.folder.clone()).collect();
    sets.sort();
    assert_eq!(
        sets,
        [f.tko.parent().unwrap(), f.site.parent().unwrap()],
        "the website set and the .tko set, not osu!'s"
    );
    assert!(ask.on_move, "Move is picked at first");
    frame(&mut s, now);
    frame(&mut s, now);
    let taking: Vec<String> = s.hits.iter().map(|h| format!("{:?}", h.hit)).collect();
    assert_eq!(
        taking,
        ["Key(Escape)", "MoveCustomMaps", "Changelog", "Account"],
        "nothing under the offer takes a click"
    );
    let mut c = crate::hit::Canvas {
        audit: true,
        width: WIDTH,
        ..Default::default()
    };
    crate::screens::select::move_custom::draw_move_ask(&mut c, &ask);
    assert_snapshot("custom-move-ask", &screen(&c, &[]));

    // Not now: nothing moves, and it isn't offered again.
    press(&mut s, KeyCode::Escape, now);
    assert!(s.select.moving.is_none());
    assert!(s.cfg.custom_move_asked);
    assert!(toast(&mut s).contains("Settings > Library"));
    assert!(f.site.is_file() && f.tko.is_file());
    // The next start (which hasn't looked yet) doesn't look again.
    s.custom_move_looked = false;
    for _ in 0..10 {
        frame(&mut s, now);
        now += Duration::from_millis(16);
    }
    assert!(
        s.custom_move.is_idle() && s.select.moving.is_none(),
        "not looked for again"
    );
}

#[test]
fn move_takes_the_sets_across_with_their_plays_and_settings_move_them_back() {
    let f = Folders::new("move");
    let (mut s, mut now) = client(&f);
    assert!(listed(&s, &f.site) && listed(&s, &f.tko) && listed(&s, &f.osu));
    let played_at = 1_763_251_200;
    s.select.played.insert(f.site.clone(), played_at);
    s.cfg.editor_recent = vec![f.tko.to_string_lossy().into_owned()];
    offered(&mut s, &mut now);
    press(&mut s, KeyCode::Enter, now);
    assert!(s.select.moving.is_none() && s.cfg.custom_move_asked);
    assert!(toast(&mut s).contains("moving 2 taiko.rs sets"));
    moved(&mut s, &mut now);

    let (site, tko) = (f.moved(&f.site), f.moved(&f.tko));
    assert!(site.is_file() && tko.is_file(), "moved whole");
    assert!(!f.site.parent().unwrap().exists() && !f.tko.parent().unwrap().exists());
    assert!(f.osu.is_file(), "osu!'s set stays");
    assert!(
        listed(&s, &site) && listed(&s, &tko) && listed(&s, &f.osu),
        "listed from the new place without leaving song select"
    );
    assert!(!listed(&s, &f.site) && !listed(&s, &f.tko));
    assert!(
        s.library.iter().any(|e| e.path == site && e.is_own()),
        "a map of taiko.rs's own folder now"
    );
    assert_eq!(
        s.select.played.get(&site),
        Some(&played_at),
        "its last play follows it"
    );
    assert_eq!(s.cfg.editor_recent, [tko.to_string_lossy().into_owned()]);
    assert_eq!(
        custom::Manifest::read(site.parent().unwrap()).and_then(|m| m.id),
        Some(12)
    );
    assert_eq!(
        custom::Manifest::read(tko.parent().unwrap()).and_then(|m| m.origin),
        Some(custom::Origin::Import)
    );

    // Settings > Library: back where they were.
    s.move_custom_maps_back();
    moved(&mut s, &mut now);
    let said = toast(&mut s);
    assert!(said.contains("moved 2 sets back"), "{said}");
    assert!(f.site.is_file() && f.tko.is_file() && !site.exists() && !tko.exists());
    assert!(listed(&s, &f.site) && listed(&s, &f.tko));
    assert_eq!(s.select.played.get(&f.site), Some(&played_at));
    // Nothing left to move back.
    s.move_custom_maps_back();
    assert!(toast(&mut s).contains("nothing was moved"));
}

#[test]
fn settings_move_them_at_once_and_say_when_theres_nothing() {
    let f = Folders::new("settings");
    let (mut s, mut now) = client(&f);
    s.cfg.custom_move_asked = true;
    s.select.moving = None;
    s.look_for_custom_maps(true);
    assert!(
        wait_for(&mut s, &mut now, |s| s
            .toast
            .as_ref()
            .is_some_and(|t| t.text.contains("moving 2"))),
        "moved without asking again: the player asked"
    );
    moved(&mut s, &mut now);
    assert!(f.moved(&f.site).is_file());
    s.look_for_custom_maps(true);
    assert!(
        wait_for(&mut s, &mut now, |s| s
            .toast
            .as_ref()
            .is_some_and(|t| t.text.contains("none of taiko.rs's maps"))),
        "nothing left to move"
    );
}
