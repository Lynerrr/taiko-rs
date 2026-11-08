//! What the game reads and writes: `.osu` charts, `config.txt`, replays and mod lists. A change
//! here can make old files unreadable or read differently, so each is kept golden.

use super::maps;
use super::snapshot::{assert_snapshot, num};
use crate::beatmap::{self, Beatmap};
use crate::config::Config;
use crate::mods::{self, Mod, Mods};
use crate::scores::{self, Replay};
use std::fmt::Write as _;

/// Everything judging and drawing take from a parsed chart, one line per object.
pub fn describe(map: &Beatmap) -> String {
    let m = &map.meta;
    let mut out = format!(
        "meta: {} - {} [{}] by {} | mode {} od {} hp {} sm {} tick {} v{} | audio {:?} preview {} \
         lead-in {} | bg {:?} | set {:?} id {:?} | playable {}\n",
        m.artist,
        m.title,
        m.version,
        m.creator,
        m.mode,
        m.od,
        m.hp,
        m.slider_multiplier,
        m.slider_tick_rate,
        m.format_version,
        m.audio_file,
        m.preview_time,
        m.audio_lead_in,
        m.background,
        m.set_id,
        m.beatmap_id,
        m.is_playable(),
    );
    let _ = writeln!(
        out,
        "first velocity {} | {} notes, {} barlines, {} control points, {} fakes",
        num(map.first_velocity, 4),
        map.notes.len(),
        map.barlines.len(),
        map.control_points.len(),
        map.fakes.len()
    );
    for n in &map.notes {
        let _ = write!(
            out,
            "  {} {:?}{} v {}",
            num(n.time, 2),
            n.kind,
            if n.big { " big" } else { "" },
            num(n.velocity, 4)
        );
        if n.end_time != n.time {
            let _ = write!(out, " end {}", num(n.end_time, 2));
        }
        if n.kind == beatmap::Kind::Roll {
            let _ = write!(out, " ticks every {}", num(n.tick_spacing, 3));
        }
        if n.kind == beatmap::Kind::Swell {
            let _ = write!(out, " hits {}", n.required_hits);
        }
        if n.fake_len != 0.0 || n.tiny || n.unhittable {
            let _ = write!(
                out,
                " fake_len {} fades {} tiny {} unhittable {}",
                num(n.fake_len, 3),
                n.fake_fades,
                n.tiny,
                n.unhittable
            );
        }
        out.push('\n');
    }
    out.push_str("  barlines:");
    for b in &map.barlines {
        let _ = write!(out, " {}", num(b.time, 1));
    }
    out.push('\n');
    for c in &map.control_points {
        let _ = writeln!(
            out,
            "  cp {} origin {} beat {} sv {} kiai {} vol {}",
            num(c.time, 2),
            num(c.beat_origin, 2),
            num(c.beat_length, 3),
            num(c.sv, 4),
            c.kiai,
            num(c.volume as f64, 2)
        );
    }
    out
}

#[test]
fn every_test_chart_parses_the_same() {
    let mut out = String::new();
    for chart in maps::all() {
        let _ = writeln!(out, "== {}", chart.name);
        out += &describe(&chart.beatmap());
    }
    assert_snapshot("formats-charts", &out);
}

/// Charts people really have: odd versions, converts, gimmicks, broken lines.
fn odd_charts() -> Vec<(&'static str, String)> {
    let head = |mode: u8, extra: &str| {
        format!(
            "osu file format v14\n\n[General]\nMode: {mode}\n\n[Metadata]\nTitle:t\nArtist:a\n\
             Creator:c\nVersion:v\n\n[Difficulty]\nHPDrainRate:5\nOverallDifficulty:5\n\
             SliderMultiplier:1.4\nSliderTickRate:1\n\n[Events]\n0,0,\"bg.jpg\",0,0\n\n\
             [TimingPoints]\n0,500,4,1,0,100,1,0\n{extra}\n[HitObjects]\n"
        )
    };
    vec![
        (
            "osu!standard convert (circles, a slider with repeats, a spinner)",
            head(0, "")
                + "100,100,1000,1,0,0:0:0:0:\n200,100,1500,1,8,0:0:0:0:\n\
                   100,100,2000,2,0,B|200:100|300:200,2,140\n256,192,4000,12,0,5000,0:0:0:0:\n",
        ),
        (
            "osu!mania chart (not playable)",
            head(3, "") + "64,192,1000,1,0,0:0:0:0:\n",
        ),
        (
            "fake sliders: negative, tiny and NaN lengths",
            head(1, "")
                + "256,192,1000,1,0,0:0:0:0:\n256,192,1500,2,0,L|300:192,1,-50\n\
                   256,192,2000,2,0,L|300:192,1,0.05\n256,192,2500,1,2,0:0:0:0:\n\
                   256,192,3000,2,0,L|300:192,1,nan\n256,192,3500,1,0,0:0:0:0:\n",
        ),
        (
            "no timing points",
            "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nOverallDifficulty:5\n\n\
             [HitObjects]\n256,192,1000,1,0,0:0:0:0:\n"
                .into(),
        ),
        (
            "old format v5, extreme SV, kiai, omitted barline",
            head(1, "1000,-10,4,1,0,100,0,1\n2000,-1000,4,1,0,100,0,8\n").replace("v14", "v5")
                + "256,192,500,1,0,0:0:0:0:\n256,192,1200,1,4,0:0:0:0:\n\
                   256,192,2200,1,6,0:0:0:0:\n",
        ),
        (
            "garbage lines are skipped",
            head(1, "nonsense\n")
                + "256,192,1000,1,0,0:0:0:0:\nnot,a,note\n256,192,x,1,0\n\
                   256,192,1300,1,2,0:0:0:0:\n",
        ),
        (
            "CRLF line endings and a BOM",
            format!("\u{feff}{}", head(1, "") + "256,192,1000,1,0,0:0:0:0:\n")
                .replace('\n', "\r\n"),
        ),
    ]
}

#[test]
fn odd_charts_parse_the_same() {
    let mut out = String::new();
    for (what, text) in odd_charts() {
        let _ = writeln!(out, "== {what}");
        match beatmap::from_text(&text) {
            Ok(map) => out += &describe(&map),
            Err(e) => {
                let _ = writeln!(out, "error: {e}");
            }
        }
        let meta = beatmap::read_meta(&text);
        let _ = writeln!(
            out,
            "read_meta: {} - {} [{}] mode {} od {} bg {:?}",
            meta.artist, meta.title, meta.version, meta.mode, meta.od, meta.background
        );
    }
    assert_snapshot("formats-odd-charts", &out);
}

#[test]
fn chart_helpers() {
    let mut out = String::new();
    for od in [0.0, 5.0, 10.0] {
        let _ = writeln!(
            out,
            "od {od}: range {} window {} judged {}",
            num(beatmap::difficulty_range(od, 50.0, 35.0, 20.0), 3),
            num(beatmap::window_ms(od, 50.0, 35.0, 20.0), 3),
            num(beatmap::judged_window_ms(od, 50.0, 35.0, 20.0), 3)
        );
    }
    for (first, lead) in [(500.0, 0.0), (5000.0, 0.0), (1000.0, 3000.0)] {
        let _ = writeln!(
            out,
            "play_start({first}, {lead}) = {}",
            beatmap::play_start(first, lead)
        );
    }
    let cal = beatmap::calibration_map(500.0, 4, 8);
    let _ = writeln!(out, "calibration map:\n{}", describe(&cal));
    assert_snapshot("formats-chart-helpers", &out);
}

#[test]
fn every_mod_keeps_its_acronym_name_and_multiplier() {
    let mut out = String::new();
    for m in mods::ALL {
        let alone = Mods::default().flip(m);
        let incompatible: Vec<&str> = mods::ALL
            .iter()
            .filter(|o| **o != m && alone.incompatible(**o))
            .map(|o| o.acronym())
            .collect();
        let _ = writeln!(
            out,
            "{} {:?} \"{}\" x{} unranked {} relax {} hp(5) {} scroll {} od(5) {} rate {} | \
             not with {}",
            m.acronym(),
            m,
            m.name(),
            num(m.multiplier(), 4),
            alone.unranked(),
            alone.relax(),
            num(alone.hp(5.0), 3),
            num(alone.scroll(), 3),
            num(alone.od(5.0), 3),
            num(alone.speed(1.0, true).0, 3),
            incompatible.join(",")
        );
        assert_eq!(Mod::from_acronym(m.acronym()), Some(m));
        assert_eq!(Mods::parse(m.acronym()), alone, "{m:?}");
    }
    assert_snapshot("formats-mods", &out);
}

#[test]
fn mod_lists_round_trip() {
    let many = Mods::parse("HD,HR,DT,FL,PR,AC,WV,PY,SU,MR");
    for set in [
        Mods::default(),
        Mods::parse("HD,HR,DT"),
        Mods::parse("EZ,NF,HT"),
        Mods::parse("hd, hr ,fl"),
        many,
    ] {
        assert_eq!(Mods::parse(&set.to_config()), set);
        assert_eq!(Mods::from_list(&set.list()), set);
    }
    // Unknown acronyms are dropped, not an error.
    assert_eq!(Mods::parse("HD,??,,"), Mods::parse("HD"));
    // Turning a mod on takes its incompatible ones off.
    let mut m = Mods::parse("HR");
    m.toggle(Mod::Easy);
    assert!(m.has(Mod::Easy) && !m.has(Mod::HardRock));
    let mut m = Mods::parse("DT");
    m.toggle(Mod::HalfTime);
    assert!(m.has(Mod::HalfTime) && !m.has(Mod::DoubleTime));
    let mut out = String::new();
    for seed in 1..=6 {
        let _ = writeln!(out, "shuffle({seed}) = {}", Mods::shuffle(seed).label());
    }
    let _ = writeln!(
        out,
        "label HD,HR,DT,FL = {}",
        Mods::parse("HD,HR,DT,FL").label()
    );
    let _ = writeln!(
        out,
        "config HD,HR,DT,FL = {}",
        Mods::parse("HD,HR,DT,FL").to_config()
    );
    assert_snapshot("formats-mod-lists", &out);
}

fn sample_replay() -> Replay {
    let mut r = super::gameplay::replay(
        vec![(1023.0, 1), (1173.5, 0), (1324.25, 2), (1473.0, 3)],
        &["HD", "DT"],
        scores::new_version(),
    );
    r.map_hash = 0x1234_5678_9abc_def0;
    r.map_path = "/songs/set/map.osu".into();
    r.date = 1_700_000_000;
    r.score = 987_654;
    r.accuracy = 0.987;
    r.max_combo = 4;
    r.great = 3;
    r.ok = 1;
    r.ur = 85.25;
    r
}

#[test]
fn replays_keep_their_file_format() {
    let json = serde_json::to_string_pretty(&sample_replay()).unwrap();
    assert_snapshot("formats-replay-json", &json);
    let dir =
        std::env::temp_dir().join(format!("taiko-rs-regression-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Version 1, from before mods, scroll speed and seeds: still loads, with their defaults.
    let old = dir.join("v1.json");
    std::fs::write(
        &old,
        r#"{"version":1,"map_hash":1,"map_path":"","map_label":"m","player":"p","date":0,"rate":1.0,"anti_mash":false,"score":0,"accuracy":1.0,"max_combo":0,"great":0,"ok":0,"miss":0,"ur":0.0,"inputs":[[1000.0,1]]}"#,
    )
    .unwrap();
    let r = scores::load_replay(&old).expect("an old replay loads");
    assert!(r.mods.is_empty() && r.seed == 0 && !r.keep_pitch);
    assert!((r.scroll_speed - 1.0).abs() < 1e-9);
    assert_eq!(r.mash(), crate::game::Mash::legacy(false));
    // From a newer build: refused instead of misread.
    let newer = dir.join("newer.json");
    let mut future = sample_replay();
    future.version = scores::new_version() + 1;
    std::fs::write(&newer, serde_json::to_vec(&future).unwrap()).unwrap();
    assert!(scores::load_replay(&newer).is_err());
    // Not a replay at all.
    let junk = dir.join("junk.json");
    std::fs::write(&junk, b"{ nope").unwrap();
    assert!(scores::load_replay(&junk).is_err());
    let mut rules = String::new();
    for v in 1..=scores::new_version() {
        let mut r = sample_replay();
        r.version = v;
        let _ = writeln!(
            rules,
            "v{v}: mash {:?} half {:?} windows {:?} scoring {:?}",
            r.mash(),
            r.half_rule(),
            r.window_rule(),
            r.scoring()
        );
    }
    assert_snapshot("formats-replay-rules", &rules);
}

/// A config with every kind of value away from its default.
fn changed_config() -> Config {
    Config {
        mods: Mods::parse("HD,HR"),
        master_volume: 0.35,
        animations: false,
        song_headings: true,
        check_updates: false,
        online_server: "example.org:7800".into(),
        idle_minutes: 5,
        random_seed: 5,
        ..Config::default()
    }
}

#[test]
fn the_config_file_reads_back_what_was_saved() {
    for cfg in [Config::default(), changed_config()] {
        let text = cfg.to_text();
        let back = Config::parse_text(&text);
        assert_eq!(back.to_text(), text, "config.txt round trip");
    }
    // Unknown lines and bad values don't lose the rest.
    let text = changed_config().to_text();
    let noisy = format!("some_future_setting = 42\n{text}\nmaster_volume = loud\n= nothing\n");
    let back = Config::parse_text(&noisy);
    assert_eq!(back.mods, Mods::parse("HD,HR"));
    assert_eq!(back.online_server, "example.org:7800");
    assert_eq!(back.idle_minutes, 5);
}

#[cfg(target_os = "linux")]
#[test]
fn the_default_config_file_stays_the_same() {
    // The Random seed is drawn from the clock for every new config.
    let text: String = Config::default()
        .to_text()
        .lines()
        .filter(|l| !l.starts_with("random_seed"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_snapshot("formats-default-config", &text);
    assert_snapshot("formats-changed-config", &changed_config().to_text());
}
