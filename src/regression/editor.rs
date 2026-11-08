//! The map editor: charts read and written back, every kind of edit with undo and redo, saving,
//! timing lines, `.tko` maps with their gimmick and fake notes, and the editor driven from the
//! game (its modes, keys, test play and leaving it).

use super::formats::{describe, odd_charts};
use super::gameplay::{play, summary};
use super::harness::{self, *};
use super::maps::{self, Player};
use super::snapshot::{assert_snapshot, num};
use crate::editor::chart::Chart;
use crate::editor::state::Editor;
use crate::editor::view::{Keys, View};
use crate::*;
use std::fmt::Write as _;

/// A folder of its own for one test's files.
fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "taiko-rs-regression-editor-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The hit objects and timing lines of a chart, the part edits change.
fn body(c: &Chart) -> String {
    let mut out = String::new();
    for p in &c.timing {
        let _ = writeln!(
            out,
            "  line {} beat {} meter {} vol {} red {} fx {}",
            num(p.time, 2),
            num(p.beat_length, 4),
            p.meter,
            p.volume,
            p.uninherited,
            p.effects
        );
    }
    for o in &c.objects {
        let _ = writeln!(
            out,
            "  obj {} kind {} sound {} {}{}",
            num(o.time, 2),
            o.kind,
            o.sound,
            o.rest,
            if o.fake { " fake" } else { "" }
        );
    }
    out
}

#[test]
fn an_unedited_chart_is_written_back_exactly_and_plays_the_same() {
    let mut texts: Vec<(String, String)> = maps::all()
        .iter()
        .map(|c| (c.name.to_string(), c.text()))
        .collect();
    texts.extend(odd_charts().into_iter().map(|(n, t)| (n.to_string(), t)));
    for (name, text) in texts {
        let chart = Chart::parse(&text);
        let back = chart.to_text();
        assert_eq!(
            back, text,
            "{name}: an unedited chart gives back what was read"
        );
        // And reading it again changes nothing.
        assert_eq!(Chart::parse(&back).to_text(), back, "{name}");
        match (beatmap::from_text(&text), beatmap::from_text(&back)) {
            (Ok(a), Ok(b)) => assert_eq!(describe(&a), describe(&b), "{name}: same gameplay"),
            (Err(a), Err(b)) => assert_eq!(a, b, "{name}"),
            _ => panic!("{name}: one reads and the other doesn't"),
        }
    }
}

#[test]
fn every_kind_of_edit_undoes_and_redoes() {
    let dir = folder("edits");
    let path = dir.join("basics.osu");
    let original = maps::basics().text();
    std::fs::write(&path, &original).unwrap();
    let mut ed = Editor::open(path.clone()).expect("the editor opens a chart");
    let mut log = String::new();
    let mut step = |ed: &mut Editor, what: &str, result: String| {
        let _ = writeln!(
            log,
            "{what}: {result} | {} objects, {} lines, modified {}",
            ed.chart.objects.len(),
            ed.chart.timing.len(),
            ed.modified()
        );
    };
    step(&mut ed, "opened", String::new());
    let r = ed.place_hit(11_000.0, false);
    step(
        &mut ed,
        "place a don after the end",
        format!("{}", r.is_some()),
    );
    let r = ed.place_hit(11_150.0, true);
    step(&mut ed, "place a kat", format!("{}", r.is_some()));
    let r = ed.place_hit(1000.0, false);
    step(
        &mut ed,
        "place on an existing note",
        format!("{}", r.is_some()),
    );
    let r = ed.place_roll(11_500.0, 12_400.0);
    step(&mut ed, "place a drumroll", format!("{}", r.is_some()));
    let r = ed.place_swell(12_700.0, 14_000.0);
    step(&mut ed, "place a swell", format!("{}", r.is_some()));
    ed.select_range(1000.0, 1450.0, false);
    let n = ed.selection.len();
    step(&mut ed, "select 1000-1450", format!("{n} selected"));
    ed.retype_selected(None, Some(true));
    step(&mut ed, "make them big", String::new());
    ed.flip_selected();
    step(&mut ed, "flip their colours", String::new());
    let r = ed.move_selected(300.0, false);
    step(&mut ed, "move them 300 ms later", format!("{r}"));
    let r = ed.copy();
    step(&mut ed, "copy", format!("{r}"));
    let r = ed.paste(15_000.0);
    step(&mut ed, "paste at 15000", format!("{r}"));
    let r = ed.clone_selected();
    step(&mut ed, "clone the selection", format!("{r}"));
    ed.select_range(5000.0, 6000.0, false);
    let r = ed.delete_selected();
    step(&mut ed, "delete 5000-6000", format!("{r}"));
    let r = ed.toggle_sound(4);
    step(&mut ed, "toggle finish on the selection", format!("{r}"));
    let r = ed.add_red(16_000.0, 240.0).map(|_| ());
    step(&mut ed, "add a 240 BPM red line", format!("{r:?}"));
    let r = ed.add_green(17_000.0, 1.5).map(|_| ());
    step(&mut ed, "add a 1.5x green line", format!("{r:?}"));
    let id = ed.chart.timing[0].id;
    let r = ed.edit_timing(id, |p| p.volume = 60, false);
    step(&mut ed, "first line to volume 60", format!("{r:?}"));
    let r = ed.add_bookmark(2000.0);
    step(&mut ed, "bookmark 2000", format!("{r}"));
    let edited = ed.chart.to_text();
    let _ = writeln!(log, "edited chart:\n{}", body(&ed.chart));

    // Every edit undoes, back to the file as it was read.
    let mut undos = 0;
    while ed.undo() {
        undos += 1;
    }
    let _ = writeln!(log, "undid {undos} edits, modified {}", ed.modified());
    assert_eq!(
        ed.chart.to_text(),
        original,
        "undo all the way gives back the file"
    );
    let mut redos = 0;
    while ed.redo() {
        redos += 1;
    }
    let _ = writeln!(log, "redid {redos} edits");
    assert_eq!(undos, redos);
    assert_eq!(
        ed.chart.to_text(),
        edited,
        "redo all the way gives back the edits"
    );

    // Saving writes what the editor has, keeps the first version beside it and plays.
    let saved = ed.save();
    let _ = writeln!(log, "save: {saved:?}, modified after {}", ed.modified());
    let on_disk = std::fs::read_to_string(&ed.path).unwrap();
    let bookmarks = on_disk.lines().find(|l| l.starts_with("Bookmarks"));
    let _ = writeln!(log, "saved bookmarks line: {bookmarks:?}");
    assert!(
        bookmarks.is_some_and(|l| l.contains("2000")),
        "bookmarks are saved"
    );
    let mut bak = ed.path.clone().into_os_string();
    bak.push(".bak");
    assert_eq!(
        std::fs::read_to_string(PathBuf::from(bak)).unwrap(),
        original,
        "the first version is kept as .bak"
    );
    let map = beatmap::load(&ed.path).expect("the saved chart plays");
    let _ = writeln!(log, "saved map: {} notes", map.notes.len());
    let _ = writeln!(
        log,
        "played perfectly: {}",
        summary(&play(&map, Player::Perfect, &[]))
    );
    let _ = writeln!(
        log,
        "played with jitter: {}",
        summary(&play(&map, Player::Jitter(40.0), &[]))
    );
    assert_snapshot("editor-edits", &log);
}

#[test]
fn timing_lines_keep_the_notes_on_their_beats() {
    let dir = folder("timing");
    let path = dir.join("sv.osu");
    std::fs::write(&path, maps::sv_and_kiai().text()).unwrap();
    let mut ed = Editor::open(path).unwrap();
    let mut log = String::new();
    let _ = writeln!(log, "snapped(1013) = {}", num(ed.snapped(1013.0), 2));
    let red = ed
        .chart
        .timing
        .iter()
        .find(|p| p.uninherited == 1)
        .unwrap()
        .id;
    let r = ed.edit_timing(red, |p| p.beat_length = 300.0, false);
    let _ = writeln!(log, "first red line to 200 BPM: {r:?}");
    let r = ed.delete_timing(red);
    let _ = writeln!(log, "delete it: {r:?}");
    let ids: Vec<u64> = ed.chart.timing.iter().map(|p| p.id).collect();
    let r = ed.copy_lines(&ids[..2]);
    let _ = writeln!(log, "copy two lines: {r}");
    let r = ed.paste_lines(20_000.0);
    let _ = writeln!(log, "paste them at 20000: {r:?}");
    ed.select_all();
    let r = ed.resnap_selected();
    let _ = writeln!(log, "resnap everything: {r} moved");
    // Removing every red line is refused: a chart needs one.
    let reds: Vec<u64> = ed
        .chart
        .timing
        .iter()
        .filter(|p| p.uninherited == 1)
        .map(|p| p.id)
        .collect();
    let r = ed.delete_timings(&reds);
    let _ = writeln!(log, "delete every red line: {r:?}");
    let _ = writeln!(log, "chart:\n{}", body(&ed.chart));
    match beatmap::from_text(&ed.chart.to_text()) {
        Ok(map) => {
            let _ = writeln!(log, "plays: {}", summary(&play(&map, Player::Perfect, &[])));
        }
        Err(e) => {
            let _ = writeln!(log, "doesn't read: {e}");
        }
    }
    assert_snapshot("editor-timing", &log);
}

#[test]
fn tko_maps_keep_their_gimmick_and_fake_notes() {
    let dir = folder("tko");
    let osu = maps::basics().text();
    let tko = crate::tko::format::write(&osu, &crate::tko::gimmick::Gimmick::default())
        .expect("a .tko with an empty gimmick");
    let path = dir.join("basics.tko");
    std::fs::write(&path, &tko).unwrap();
    let mut ed = Editor::open(path.clone()).expect("the editor opens a .tko");
    let mut log = String::new();
    let _ = writeln!(
        log,
        "opened: tko {} objects {}",
        ed.is_tko(),
        ed.chart.objects.len()
    );
    // Fake notes only go into a .tko; they stack with real ones.
    for (t, kat) in [(1000.0, false), (1075.0, true), (11_000.0, false)] {
        let r = ed.place_note(t, kat, true);
        let _ = writeln!(
            log,
            "fake {} at {t}: {}",
            if kat { "kat" } else { "don" },
            r.is_some()
        );
    }
    let saved = ed.save();
    let _ = writeln!(log, "save: {saved:?}");
    let text = std::fs::read_to_string(&ed.path).unwrap();
    let parsed = crate::tko::format::read(&text).expect("the saved .tko reads");
    let _ = writeln!(log, "fakes in the gimmick: {}", parsed.gimmick.fakes.len());
    let map = beatmap::load(&ed.path).expect("the saved .tko plays");
    let _ = writeln!(
        log,
        "plays: {} notes, {} fakes, tko version {}",
        map.notes.len(),
        map.fakes.len(),
        map.meta.tko_version
    );
    let _ = writeln!(
        log,
        "perfect: {}",
        summary(&play(&map, Player::Perfect, &[]))
    );
    let (_, gimmick) = crate::tko::format::split(&text);
    let _ = writeln!(log, "[Gimmick]: {}", gimmick.unwrap_or_default().trim());
    // Fakes are refused in a plain .osu.
    let plain = dir.join("plain.osu");
    std::fs::write(&plain, &osu).unwrap();
    let mut ed = Editor::open(plain).unwrap();
    let _ = writeln!(
        log,
        "fake in a plain .osu: {}",
        ed.place_note(11_000.0, false, true).is_some()
    );
    // A .tko from a newer build is refused instead of misread.
    let newer = tko.replace(
        &format!("TkoVersion:{}", crate::tko::TKO_VERSION),
        &format!("TkoVersion:{}", crate::tko::TKO_VERSION + 1),
    );
    let _ = writeln!(
        log,
        "newer .tko: {:?}",
        crate::tko::format::read(&newer).err()
    );
    assert_snapshot("editor-tko", &log);
}

/// A client on song select with the editor open on `chart`, its tour seen or not.
fn editing(tag: &str, tour_done: bool) -> (State, Songs, Instant) {
    let songs = Songs::new(tag, &[maps::basics()]);
    let mut cfg = harness::config();
    cfg.editor_tour_done = tour_done;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.open_editor(0);
    assert_eq!(scene_name(&s), "Editor", "the editor opens");
    settle(&mut s, &mut now);
    (s, songs, now)
}

fn view(s: &mut State) -> &mut View {
    let name = scene_name(s);
    match &mut s.scene {
        Scene::Editor(v) => v,
        _ => panic!("not in the editor but {name}"),
    }
}

fn ctrl() -> Keys {
    Keys {
        ctrl: true,
        ..Keys::default()
    }
}

#[cfg(target_os = "linux")]
#[test]
fn editor_screens() {
    let (mut s, _songs, mut now) = editing("editor-screens", true);
    for (key, name) in [
        (KeyCode::F1, "compose"),
        (KeyCode::F2, "gimmicks"),
        (KeyCode::F3, "timing"),
        (KeyCode::F4, "effects"),
    ] {
        view(&mut s).key(key, false, Keys::default(), None);
        settle(&mut s, &mut now);
        assert_snapshot(
            &format!("editor-screen-{name}"),
            &screen(&frame(&mut s, now), &[" fps  "]),
        );
    }
    let (mut s, _songs, mut now) = editing("editor-tour", false);
    settle(&mut s, &mut now);
    assert_snapshot(
        "editor-screen-first-run-tour",
        &screen(&frame(&mut s, now), &[" fps  "]),
    );
}

#[test]
fn the_editor_from_the_game_edits_saves_test_plays_and_closes() {
    let (mut s, songs, mut now) = editing("editor-flow", true);
    let mut log = String::new();
    let path = songs.paths[0].clone();
    let before = std::fs::read_to_string(&path).unwrap();
    // An edit, undone and redone with the keys.
    let v = view(&mut s);
    let n = v.ed.chart.objects.len();
    v.ed.place_hit(11_000.0, false);
    v.key(KeyCode::KeyZ, false, ctrl(), None);
    let undone = view(&mut s).ed.chart.objects.len();
    view(&mut s).key(KeyCode::KeyY, false, ctrl(), None);
    let redone = view(&mut s).ed.chart.objects.len();
    let _ = writeln!(log, "objects: {n} -> undo {undone} -> redo {redone}");
    assert_eq!((undone, redone), (n, n + 1));
    // Ctrl+S saves it to the file.
    view(&mut s).key(KeyCode::KeyS, false, ctrl(), None);
    frame(&mut s, now);
    let after = std::fs::read_to_string(&path).unwrap();
    let _ = writeln!(
        log,
        "saved: changed {} modified {}",
        after != before,
        view(&mut s).ed.modified()
    );
    assert_ne!(after, before, "Ctrl+S wrote the file");
    // F5 plays the map as it is from the playhead, and quitting comes back to the editor.
    view(&mut s).set_time(9000.0);
    view(&mut s).key(KeyCode::F5, false, Keys::default(), None);
    assert!(
        wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"),
        "the test play starts"
    );
    let g = harness::game(&s).unwrap();
    let _ = writeln!(
        log,
        "test play: {} notes, no fail {}, test {}",
        g.map.notes.len(),
        g.mods.has(mods::Mod::NoFail),
        g.test_play
    );
    press(&mut s, KeyCode::Escape, now);
    for _ in 1..game::PAUSE_ITEMS {
        press(&mut s, KeyCode::ArrowDown, now);
    }
    press(&mut s, KeyCode::Enter, now);
    run_until(&mut s, &mut now, Duration::from_millis(16), 50, |s| {
        scene_name(s) == "Editor"
    });
    let _ = writeln!(log, "after quitting the test play: {}", scene_name(&s));
    assert_eq!(scene_name(&s), "Editor");
    assert_eq!(
        view(&mut s).ed.chart.objects.len(),
        n + 1,
        "the editor as it was"
    );
    // Escape with nothing unsaved leaves the editor.
    view(&mut s).key(KeyCode::Escape, false, Keys::default(), None);
    frame(&mut s, now);
    let _ = writeln!(log, "after Escape: {}", scene_name(&s));
    assert_snapshot("editor-flow", &log);
}
