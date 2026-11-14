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
        mp_name: "tester".into(),
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
    // The Random seed is drawn from the clock for every new config, and the multiplayer name is
    // the system's user name.
    let text: String = Config::default()
        .to_text()
        .lines()
        .filter(|l| !l.starts_with("random_seed") && !l.starts_with("mp_name"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_snapshot("formats-default-config", &text);
    assert_snapshot("formats-changed-config", &changed_config().to_text());
}

/// Every key of `config.txt` away from its default, the way a player's file could have it.
const EVERY_KEY_CHANGED: &str = "\
keys = A, S, K, L
preset_keys = F7, F8
touch_layout = DKKD
screenshot_to = both
language = ru
show_playfield_size = false
saved_presets = 1600x900, 1024x768
preset_spots = 1600x900@0.1000/0.2000
offset_scale = sound
offset = 10
offset_ms = -50
offset_latency_ms = 17
visual_offset_ms = 8
scroll_speed = 1.35
scroll_speed_osu = true
master_volume = 0.35
music_volume = 0.45
unfocused_volume = 0.10
fx_volume = 0.55
present_mode = vsync
menu_vsync = false
half_resolution = true
graphics_api = gl
fps_cap = 240
calibration_bpm = 150
playfield_area = 0.100,0.050,0.800,0.900
bg_dim = 0.25
rate = 1.25
rate_keep_pitch = false
audio_buffer = 512
show_fps = false
fullscreen = false
animations = false
splash = false
epilepsy_warning = false
main_menu = false
menu_notes = false
menu_notes_dim = 0.40
guide_seen = true
check_updates = false
resume_delay_ms = 1500
sort = title
kiai_effects = false
hide_great = false
fail_restart = true
black_lane = true
true_notes = true
calm_maps = true
drum_menus = true
alt_f4_leaves_map = true
hit_flash = false
show_offset_hint = false
offset_hint_ur_scaled = false
show_map_info = false
mods = HR,HD
mp_scoreboard = false
autoplay_style = semi
autoplay_binding = 4231
autoplay_left_handed = true
skin = Rafis
skin_part = notes = <built-in>
map_skins = false
menu_glitch = false
accent = #123456
discord_presence = false
discord_app_id = 1234567890
idle_minutes = 5
send_crash_reports = false
editor_tour_done = true
editor_left_pan = true
editor_follow_timing = true
editor_snap_preset = 3,6,12|Triplets
editor_snap_active = Triplets
editor_recent = /maps/a.osu
editor_recent = /maps/b.osu
chat_filter = false
message_sound = false
notify_playing = true
notify_online = false
notify_messages = false
notify_mentions = false
notify_requests = false
mp_name = tester
mp_port = 7788
mp_address = 10.0.0.2:7777
online_server = example.org:7800
online_user = someone
online_token = abc123
terms_version = 3
vsync_compensation = false
vsync_tune_ms = -12
letterbox = true
letterbox_size = 1600x900
letterbox_x = 10
letterbox_y = -20
window_size = 1600x900
data_folder = /games/taiko-data
wheel_resizes_playfield = true
show_converts = true
song_headings = true
bg_dim_black = false
interface_hidden = true
interface_hides = score,combo,!pp,!scoreboard,!spectators,!ur,!counts
interface_key = F9
song_list_share = 0.600
song_preview_size = 0.500
hotkey.scroll_slower = Ctrl+F1
hotkey.quit = none
last_build = abc1234
video_size = 1920x1080
video_fps = 30
video_quality = best
video_target_mb = 25
video_speed = slow
video_blur = light
video_hud = false
video_hitsounds = false
multiverse = classic round 1x -; neon diamond 1x -; ice ring 0.9x WV; forest pill 1x SK; classic round 1x -; candy round 1x PY; ember diamond 1.25x AC; mono square 1x BL
hud_layout = off=counts
hud_preset = Mine: off=ur
multiverse_preset = Calm: classic round 1x -; neon diamond 1x -; ice ring 0.9x WV; forest pill 1x SK; classic round 1x -; candy round 1x PY; ember diamond 1.25x AC; mono square 1x BL
random_seed = 77
shapeshift = every=8 morph=1 order=fixed shapes=zigzag,uturn,spiral,loop,elbow,straight,meadow
tact = 1beat
od_override = 7.5
favourite_mods = DT,HD
mod_preset = 1.5|HR,HD|Speedy
note_colours = neon
songs_folder = /games/osu/Songs
songs = /games/more-songs
songs = /games/one.osz
";

/// The keys of a config file's settings lines, in order.
fn config_keys(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim()))
        .collect()
}

#[test]
fn every_key_of_the_config_file_reads_back_what_was_saved() {
    let cfg = Config::parse_text(EVERY_KEY_CHANGED);
    let text = cfg.to_text();
    assert_eq!(
        Config::parse_text(&text).to_text(),
        text,
        "config.txt round trip"
    );
    // Each key the game writes is in the file above, and away from its default there, so a key
    // that stopped being read or written shows up here.
    let line = |text: &str, key: &str| {
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .filter(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == key))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let default = Config::default().to_text();
    let written = config_keys(&text);
    let mut wrong = Vec::new();
    for key in config_keys(&default) {
        if !written.contains(&key) {
            wrong.push(format!("{key} isn't written"));
        } else if key != "scroll_speed_osu" && line(&default, key) == line(&text, key) {
            wrong.push(format!("{key} is at its default"));
        }
    }
    // And what the file says is what is saved again, key for key.
    for key in config_keys(EVERY_KEY_CHANGED) {
        let (given, saved) = (line(EVERY_KEY_CHANGED, key), line(&text, key));
        if given != saved {
            wrong.push(format!(
                "{key} reads back differently: {given:?} -> {saved:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}\n\n{text}", wrong.join("\n"));
    // Like the default file's golden, kept on Linux only.
    if cfg!(target_os = "linux") {
        assert_snapshot("formats-every-key-config", &text);
    }
}
