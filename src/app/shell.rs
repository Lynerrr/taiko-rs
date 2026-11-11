//! Starting the game: the command line (desktop), `android_main`, and `run`.

#[allow(unused_imports)]
use crate::*;

use super::crash::log_crashes;
use super::run_loop::App;

/// Starts loading update notes in the background, and notes this build as started.
/// The caller keeps the view pending until a nonempty list of changes is available.
pub(super) fn whats_new(cfg: &mut Config) -> Option<changelog::View> {
    let build = update::BUILD?;
    if cfg.last_build == build {
        return None;
    }
    let old = std::mem::replace(&mut cfg.last_build, build.to_string());
    cfg.save();
    if !old.is_empty() {
        Some(changelog::View::since(old))
    } else if cfg.existed {
        Some(changelog::View::recent(true))
    } else {
        None
    }
}

/// Desktop entry point: the command line, then the game.
/// `--recalc-pp`: see `taiko_recalc`.
pub fn recalc_pp(args: &[String]) -> Result<(), String> {
    taiko_recalc::run(args)
}

pub fn desktop_main() {
    let cfg = Config::load();
    config::set_data_folder(cfg.data_folder.clone());
    // Only where a crash ends the game (release builds), as with panics.
    #[cfg(windows)]
    if cfg!(panic = "abort") {
        super::crash::native_crashes::install();
    }
    log_crashes();
    let mut args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let auto = args.iter().any(|a| a == "--auto");
    args.retain(|a| a != "--auto");
    // A `taiko-rs://` link (a score's "Watch replay" button): the running game opens it, else
    // this one does.
    let link = args.iter().find_map(|a| link::parse(&a.to_string_lossy()));
    args.retain(|a| link::parse(&a.to_string_lossy()).is_none());
    if link.is_some_and(link::forward) {
        return;
    }
    // A `.rsz` opened from the file manager (some pass it as a file:// URL): imported, by the
    // running game if there is one.
    for a in &mut args {
        if let Some(path) = link::file_url(&a.to_string_lossy()) {
            *a = path;
        }
    }
    let mut opens: Vec<PathBuf> = args
        .iter()
        .filter(|a| library::is_rsz(a) && a.is_file())
        .cloned()
        .collect();
    args.retain(|a| !opens.contains(a));
    let had_opens = !opens.is_empty();
    opens.retain(|p| !link::forward_open(p));
    if had_opens && opens.is_empty() && args.is_empty() && link.is_none() {
        return;
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!(
            "usage: taiko-rs [--auto] [Songs folder | map folder | file.osz | file.osu | taiko-rs://score/<id>]...\n       taiko-rs file.rsz...           import taiko.rs map archives into the Songs folder"
        );
        println!("       taiko-rs --info file.osu...   print converted map stats");
        println!(
            "With no arguments, scans osu! stable install locations and `songs =` lines in the config."
        );
        return;
    }
    if args.first().is_some_and(|a| a == "--info") {
        for path in &args[1..] {
            info(path);
        }
        return;
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("event loop");
    // Scripted runs (tests) neither take links nor become their handler.
    if std::env::var_os("TAIKO_SHOTS").is_none() && std::env::var_os("TAIKO_INPUT").is_none() {
        let proxy = event_loop.create_proxy();
        link::listen(move |arrival| {
            let _ = proxy.send_event(match arrival {
                link::Arrival::Link(l) => UserEvent::Link(l),
                link::Arrival::Open(path) => UserEvent::Open(path),
            });
        });
        link::register();
    }
    run(cfg, args, auto, link, opens, event_loop);
}

/// Android entry point (the native activity's `android_main`): no command line, the app's own
/// storage for settings and songs.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub(crate) fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android::setup(&app);
    let cfg = Config::load();
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .with_android_app(app)
        .build()
        .expect("event loop");
    android::set_proxy(event_loop.create_proxy());
    run(cfg, Vec::new(), false, None, Vec::new(), event_loop);
}

/// Scans the library, opens audio and runs the game until it quits. `args`: songs folders, map
/// folders, .osz or a single .osu to play; empty = the usual song folders. `link`: a score to open.
pub(crate) fn run(
    cfg: Config,
    args: Vec<PathBuf>,
    auto: bool,
    link: Option<link::Link>,
    opens: Vec<PathBuf>,
    event_loop: EventLoop<UserEvent>,
) {
    config::set_data_folder(cfg.data_folder.clone());
    library::set_songs_folder(cfg.songs_folder.clone());
    multiverse::set(cfg.universes);
    shapeshift::set(cfg.shapeshift);
    tact::set(cfg.tact);
    od_override::set(cfg.od_override);
    multiverse::set_note_colours(cfg.note_colours);
    let inputs = library::startup_inputs(&args, &cfg);
    let songs_dir = download_dir(&inputs);
    let songs_dirs_for_db: Vec<PathBuf> = inputs.iter().filter(|p| p.is_dir()).cloned().collect();
    let scan = start_scan(inputs);

    let audio = match Engine::new(cfg.audio_buffer) {
        Ok(a) => {
            eprintln!("audio: {}", a.description);
            let (music, fx) = cfg.volumes();
            a.set_volume(music, fx);
            Some(a)
        }
        Err(e) => {
            eprintln!("audio unavailable ({e}); running silent");
            None
        }
    };
    cfg.save();

    // A single .osu on the command line goes straight into gameplay.
    let play_when_scanned = args.len() == 1
        && args[0]
            .extension()
            .is_some_and(|e| e == "osu" || e == "tko");
    // No splash or warning before a map given on the command line, or in scripted runs
    // (TAIKO_SPLASH=1 and TAIKO_WARNING=1 force them).
    let scripted =
        std::env::var_os("TAIKO_SHOTS").is_some() || std::env::var_os("TAIKO_INPUT").is_some();
    let want_splash = std::env::var_os("TAIKO_SPLASH").is_some()
        || (cfg.splash && !play_when_scanned && !scripted);
    let want_warning = std::env::var_os("TAIKO_WARNING").is_some()
        || (cfg.epilepsy_warning && !play_when_scanned && !scripted);
    let splash = splash::Splash::new(audio.as_ref().map(|a| a.rate), want_splash, want_warning);
    update::cleanup();
    let main_menu = cfg.main_menu && !play_when_scanned && !scripted;
    let guide = !cfg.guide_seen && !play_when_scanned && !scripted;
    let mut state = State::new(
        cfg,
        audio,
        splash,
        songs_dir,
        songs_dirs_for_db,
        Some(scan),
        play_when_scanned,
        auto,
    );
    state.main_menu = main_menu.then(menu::Menu::new);
    state.guide.page = guide.then_some(0);
    state.load_skin();
    if let Some(l) = link {
        state.open_link(l);
    }
    for path in opens {
        state.open_archive(path);
    }
    let mut app = App {
        state: Arc::new(Mutex::new(state)),
        cursor: CursorIcon::Default,
        cursors: None,
        cursor_accent: render::DEFAULT_ACCENT,
        cursor_hidden: false,
        window: None,
        ime: false,
        instance: None,
        render_thread: None,
        proxy: event_loop.create_proxy(),
    };
    event_loop.run_app(&mut app).expect("run");
}

/// `taiko-rs --info map.osu...`: prints what the converter produced, for checking maps headlessly.
pub(crate) fn info(path: &std::path::Path) {
    let map = match beatmap::load(path) {
        Ok(m) => m,
        Err(e) => return eprintln!("{}: {e}", path.display()),
    };
    let count = |k: beatmap::Kind| map.notes.iter().filter(|n| n.kind == k).count();
    let big = map.notes.iter().filter(|n| n.big).count();
    let m = &map.meta;
    println!(
        "{} - {} [{}] mode {} v{} OD {}",
        m.artist, m.title, m.version, m.mode, m.format_version, m.od
    );
    println!(
        "  don {}  kat {}  big {}  roll {}  swell {}  barlines {}  control points {}",
        count(beatmap::Kind::Don),
        count(beatmap::Kind::Kat),
        big,
        count(beatmap::Kind::Roll),
        count(beatmap::Kind::Swell),
        map.barlines.len(),
        map.control_points.len()
    );
    let (lo, hi) = map.notes.iter().fold((f64::MAX, 0.0f64), |(lo, hi), n| {
        (lo.min(n.velocity), hi.max(n.velocity))
    });
    println!(
        "  first {:.0} ms  last {:.0} ms  velocity {lo:.3}..{hi:.3} px/ms",
        map.notes[0].time,
        map.notes.last().unwrap().end_time
    );
    let (sr, sr_dt) = (stars::star_rating(path, 1.0), stars::star_rating(path, 1.5));
    println!(
        "  stars {:.2} (1.5x: {:.2})",
        sr.unwrap_or(f64::NAN),
        sr_dt.unwrap_or(f64::NAN)
    );
    let audio_path = path.parent().unwrap_or(path).join(&m.audio_file);
    let t = Instant::now();
    match audio::decode(&audio_path, 48000) {
        Ok(track) => println!(
            "  audio {:.1} s decoded in {:.0} ms",
            track.frames() as f64 / 48000.0,
            t.elapsed().as_secs_f64() * 1000.0
        ),
        Err(e) => println!("  audio error: {e}"),
    }
}
