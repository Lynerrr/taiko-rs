//! Opt-in, headless measurements of real client paths. No performance assertions depend on
//! wall-clock thresholds; correctness assertions check the compared offset estimates.

use super::*;
use std::hint::black_box;

fn report(name: &str, mut samples: Vec<f64>) {
    samples.sort_by(f64::total_cmp);
    println!("AUDIT {name}: p50_us={:.3} p95_us={:.3}", samples[samples.len() / 2], samples[samples.len() * 95 / 100]);
}

fn measure(name: &str, batch: usize, mut f: impl FnMut()) {
    for _ in 0..3 {
        f();
    }
    let samples = (0..21).map(|_| {
        let at = Instant::now();
        for _ in 0..batch {
            f();
        }
        at.elapsed().as_secs_f64() * 1e6 / batch as f64
    }).collect();
    report(name, samples);
}

fn map(notes: usize, slow: bool) -> beatmap::Beatmap {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let template = map.notes[0].clone();
    map.notes = (0..notes).map(|i| {
        let time = 2000.0 + i as f64 * 100.0;
        beatmap::Note { time, end_time: time, velocity: if slow && i == notes - 1 { 1e-5 } else { 0.4 }, ..template.clone() }
    }).collect();
    map
}

fn game(notes: usize, slow: bool, mods: &str) -> Game {
    Game::with_mods(map(notes, slow), Arc::new(audio::Track { samples: Vec::new(), rate: 1.0 }), None, Mods::parse(mods))
}

#[test]
#[ignore = "opt-in performance audit; run with the perf-audit profile"]
fn offset_hint_history() {
    for n in [200usize, 10_000, 100_000] {
        let mut full = game(64, false, "");
        full.stats.errors = (0..n).map(|i| 8.0 + (i % 41) as f64 - 20.0).collect();
        full.stats.error_offsets = (0..n).map(|i| (i % 5) as f64).collect();
        let mut tail = game(64, false, "");
        tail.stats.errors = full.stats.errors[n.saturating_sub(200)..].to_vec();
        tail.stats.error_offsets = full.stats.error_offsets[n.saturating_sub(200)..].to_vec();
        for scaled in [false, true] {
            full.ur_scaled_hint = scaled;
            tail.ur_scaled_hint = scaled;
            let a = full.offset_suggestion(3.0).unwrap();
            let b = tail.offset_suggestion(3.0).unwrap();
            assert_eq!((a.needed, a.significant, a.estimate.hits), (b.needed, b.significant, b.estimate.hits));
            assert_eq!((a.estimate.error_ms, a.estimate.ur, a.estimate.margin_ms, a.estimate.suggested_offset), (b.estimate.error_ms, b.estimate.ur, b.estimate.margin_ms, b.estimate.suggested_offset));
            measure(&format!("offset/full/{n}/scaled={scaled}"), 100, || { black_box(full.offset_suggestion(black_box(3.0))); });
            measure(&format!("offset/tail200/{n}/scaled={scaled}"), 100, || { black_box(tail.offset_suggestion(black_box(3.0))); });
        }
    }
}

#[test]
#[ignore = "opt-in performance audit; run with the perf-audit profile"]
fn rare_slow_note() {
    render::set_animations(false);
    for n in [2_000, 10_000] {
        for (slow, mods) in [(false, ""), (true, ""), (false, "MV"), (true, "MV")] {
            let mut game = game(n, slow, mods);
            game.time = 3000.0;
            let mut canvas = Canvas::default();
            measure(&format!("draw/{n}/slow={slow}/mods={mods}"), 10, || {
                canvas.clear(853.3);
                game.draw_playfield(&mut canvas, 1.0);
                black_box(canvas.shapes.len());
            });
            println!("AUDIT shapes/{n}/slow={slow}/mods={mods}: {}", canvas.shapes.len());
        }
    }
}

fn library(size: usize) -> Vec<Entry> {
    (0..size).map(|i| Entry::new(PathBuf::from(format!("/nonexistent/{i} set/map.osu")), beatmap::Meta {
        title: format!("Song {i}"), artist: "Artist".into(), version: "Oni".into(), mode: 1,
        ..Default::default()
    })).collect()
}

#[test]
#[ignore = "opt-in performance audit; run with the perf-audit profile"]
fn library_draw_and_filter() {
    render::set_animations(false);
    for size in [100, 10_000, 50_000] {
        let library = library(size);
        let mut select = Select::new(&library);
        let mut canvas = Canvas::default();
        measure(&format!("select/draw/{size}"), 5, || {
            canvas.clear(853.3);
            select.draw(&mut canvas, &library, 8.33, "", &|_| None, &|_| None);
            black_box(canvas.shapes.len());
        });
        println!("AUDIT select/shapes/{size}: {}", canvas.shapes.len());
        measure(&format!("select/refilter/{size}"), 1, || { select.refilter(black_box(&library)); });
        select.filter = "artist".into();
        measure(&format!("select/search/{size}"), 1, || { select.refilter(black_box(&library)); });
        select.filter.clear();
        select.sort = ui::SortMode::Title;
        measure(&format!("select/title-sort/{size}"), 1, || { select.refilter(black_box(&library)); });
    }
}

#[test]
#[ignore = "opt-in performance audit; run with the perf-audit profile"]
fn skin_and_builtin_sounds() {
    measure("sounds/rebuild/48000", 1, || { black_box(audio::hitsound_set(48_000, audio::Custom::default())); });
    measure("sounds/medals-only/48000", 1, || { black_box(crate::medal_sound::sounds(48_000)); });
    let dir = std::env::temp_dir().join(format!("taiko-perf-skin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for size in [512, 2048] {
        image::RgbaImage::from_fn(size, size, |x, y| image::Rgba([(x % 251) as u8, (y % 251) as u8, ((x ^ y) % 251) as u8, 255]))
            .save(dir.join("taikohitcircle.png")).unwrap();
        measure(&format!("skin/load/{size}px"), 1, || {
            black_box(skin::load("", &skin::Sources::default(), None, Some(&dir)));
        });
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "opt-in performance audit; run with the perf-audit profile"]
fn async_skin_handoff() {
    let dir = std::env::temp_dir().join(format!("taiko-perf-async-skin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::from_fn(2048, 2048, |x, y| image::Rgba([(x % 251) as u8, (y % 251) as u8, ((x ^ y) % 251) as u8, 255]))
        .save(dir.join("taikohitcircle.png")).unwrap();
    let loader = skin::Loader::new();
    let key = skin::Key { main: String::new(), parts: skin::Sources::default(), rate: Some(48_000), map_dir: Some(dir.clone()) };
    let mut requests = Vec::new();
    let mut completions = Vec::new();
    let mut handoffs = Vec::new();
    for _ in 0..21 {
        let at = Instant::now();
        loader.request(key.clone(), true);
        requests.push(at.elapsed().as_secs_f64() * 1e6);
        let deadline = at + Duration::from_secs(10);
        let ready = loop {
            if let Some(ready) = loader.poll() { break ready; }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        completions.push(at.elapsed().as_secs_f64() * 1e6);
        let at = Instant::now();
        black_box(ready.assets.images.clone());
        black_box(ready.assets.sounds.clone());
        handoffs.push(at.elapsed().as_secs_f64() * 1e6);
        assert!(ready.assets.images.sprites[render::Sprite::HitCircle as usize].is_some());
    }
    report("skin/async/request/2048px", requests);
    report("skin/async/worker-completion/2048px", completions);
    report("skin/async/arc-handoff/2048px", handoffs);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "opt-in performance audit; run with the perf-audit profile"]
fn input_mutex_contention() {
    // A contention probe, not an end-to-end input measurement: run the exact sounds rebuild
    // while holding the same State mutex used by rendering and window event handling.
    let state = Arc::new(Mutex::new(layout_tests::state()));
    let mut samples = Vec::new();
    for _ in 0..21 {
        let shared = state.clone();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let guard = lock(&shared);
            tx.send(()).unwrap();
            black_box(audio::hitsound_set(48_000, audio::Custom::default()));
            drop(guard);
        });
        rx.recv().unwrap();
        let at = Instant::now();
        drop(lock(&state));
        samples.push(at.elapsed().as_secs_f64() * 1e6);
        worker.join().unwrap();
    }
    report("mutex/sounds-rebuild/wait", samples);
}

#[test]
#[ignore = "opt-in performance audit; run with the perf-audit profile"]
fn result_stars() {
    let dir = std::env::temp_dir().join(format!("taiko-perf-stars-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    for size in [2_000, 10_000] {
        let mut text = "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:6\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n".to_string();
        for i in 0..size {
            text.push_str(&format!("256,192,{},1,{},0:0:0:0:\n", 2000 + i * 100, if i % 3 == 0 { 2 } else { 0 }));
        }
        std::fs::write(&path, text).unwrap();
        measure(&format!("results/star-rating/{size}"), 1, || {
            let stars = stars::star_rating_played(&path, 1.0, Mods::NONE, 1.0, 0, Default::default());
            assert!(stars.is_some());
            black_box(stars);
        });
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "opt-in performance audit; run with the perf-audit profile"]
fn game_preparation_and_expiry() {
    for n in [2_000, 10_000] {
        let template = map(n, false);
        let track = Arc::new(audio::Track { samples: Vec::new(), rate: 1.0 });
        let mut samples = Vec::new();
        for _ in 0..21 {
            let map = template.clone();
            let at = Instant::now();
            let game = Game::with_mods(map, track.clone(), None, Mods::NONE);
            samples.push(at.elapsed().as_secs_f64() * 1e6);
            black_box(game);
        }
        report(&format!("game/prepare/{n}"), samples);
        for long_roll in [false, true] {
            let mut map = template.clone();
            if long_roll {
                let first = map.notes[0].clone();
                map.notes.insert(0, beatmap::Note { time: 1999.0, end_time: 2000.0 + n as f64 * 100.0, kind: beatmap::Kind::Roll, tick_spacing: 100.0, ..first });
            }
            let mut game = Game::with_mods(map, track.clone(), None, Mods::parse("NF"));
            let time = 2000.0 + n as f64 * 50.0;
            game.set_time(time);
            let missed = game.stats.miss;
            measure(&format!("game/expiry/{n}/long-roll={long_roll}"), 100, || { game.set_time(black_box(time)); });
            assert_eq!(game.stats.miss, missed);
        }
    }
}

#[test]
#[ignore = "opt-in performance audit; run with the perf-audit profile"]
fn lazy_map_hash() {
    let dir = std::env::temp_dir().join(format!("taiko-perf-hash-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    let mut state = layout_tests::state();
    state.scene = Scene::Select;
    state.map_offsets.clear();
    state.library = vec![Entry::new(path.clone(), Default::default())];
    state.select = Select::new(&state.library);
    for size in [65_536, 1_048_576, 8_388_608] {
        std::fs::write(&path, vec![b'x'; size]).unwrap();
        black_box(std::fs::read(&path).unwrap()); // Warm filesystem cache, not cold disk claims.
        let mut times = Vec::new();
        for _ in 0..21 {
            let entry = Entry::new(path.clone(), Default::default());
            let at = Instant::now();
            black_box(entry.hash());
            times.push(at.elapsed().as_secs_f64() * 1e6);
        }
        report(&format!("hash/first/{size}"), times);
        let mut times = Vec::new();
        for _ in 0..21 {
            state.library[0] = Entry::new(path.clone(), beatmap::Meta { mode: 1, ..Default::default() });
            let at = Instant::now();
            state.refresh_play_offset();
            times.push(at.elapsed().as_secs_f64() * 1e6);
            assert_eq!(state.play_offset, state.cfg.offset_ms);
        }
        report(&format!("offset-lookup/empty-cache/first/{size}"), times);
        let entry = Entry::new(path.clone(), Default::default());
        entry.hash();
        measure(&format!("hash/cached/{size}"), 1000, || { black_box(entry.hash()); });
    }
    std::fs::remove_dir_all(dir).unwrap();
}
