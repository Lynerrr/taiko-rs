//! The secret medals' side of the game: what it tells the server it found or saw, for the
//! medals of the URA tab (`Request::ClientEvent`). The server decides what any of it earns; a
//! medal comes back as `Down::Medals`, like every other.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::config::Config;

/// Something only the game sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// The logo tapped until it fell apart (`menu`).
    Logo,
    /// The stray note among the menu's background notes, caught (`menu_notes`).
    StrayNote,
    /// The menu turned upside down with both rims at once (`menu`).
    Flip,
    /// The unknown medal's question mark tapped open (`medals`).
    Curious,
    /// A play passed with the music muted.
    Deaf,
    /// One map started again and again in a row.
    Again,
    /// A map saved in the editor; counted each time.
    MapSave,
}

impl Event {
    /// Its name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Self::Logo => "ura_logo",
            Self::StrayNote => "ura_stray_note",
            Self::Flip => "ura_flip",
            Self::Curious => "ura_curious",
            Self::Deaf => "ura_deaf",
            Self::Again => "ura_again",
            Self::MapSave => "ura_map_save",
        }
    }

    /// Whether every time counts, not just the first.
    fn counts_each_time(self) -> bool {
        self == Self::MapSave
    }
}

/// Starts of one map in a row that make `Event::Again`.
pub const AGAIN: u32 = 50;
/// The least accuracy of a muted play (`Event::Deaf`).
const DEAF_ACCURACY: f64 = 0.95;

/// One map started again and again: which, and how many times in a row (restarts too).
#[derive(Default)]
pub struct Again {
    map: Option<std::path::PathBuf>,
    starts: u32,
}

impl Again {
    /// A start of `map`; true on the start that makes `AGAIN` in a row.
    pub fn start(&mut self, map: &std::path::Path) -> bool {
        if self.map.as_deref() == Some(map) {
            self.starts += 1;
        } else {
            self.map = Some(map.to_path_buf());
            self.starts = 1;
        }
        self.starts == AGAIN
    }
}

/// A whole play passed (from the start, not Autoplay), played muted. A clean play on a custom
/// map the player uploaded is the server's to see from the submitted play: a downloaded custom
/// set keeps the same number in its folder (`custom::SIDECAR`) as an uploaded one.
pub fn whole_play(cfg: &Config, accuracy: f64) {
    if cfg.music_volume * cfg.master_volume <= 0.0 && accuracy >= DEAF_ACCURACY {
        report(cfg, Event::Deaf);
    }
}

thread_local! {
    /// The one-off events already sent: finding an egg twice earns nothing more, so it isn't
    /// sent again. The game reports from its main thread only, so per thread is per run (and
    /// keeps parallel tests apart).
    static SENT: RefCell<HashSet<Event>> = RefCell::new(HashSet::new());
    /// What a test reported, instead of sending it.
    #[cfg(test)]
    static REPORTED: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
}

/// What this test thread reported so far, emptied, and forgets what was sent.
#[cfg(test)]
pub fn take_reported() -> Vec<Event> {
    SENT.with(|s| s.borrow_mut().clear());
    REPORTED.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// Tells the server, in the background, when logged in. Tests only note it.
pub fn report(cfg: &Config, event: Event) {
    if cfg.online_token.is_empty() {
        return;
    }
    if !event.counts_each_time() && !SENT.with(|s| s.borrow_mut().insert(event)) {
        return;
    }
    #[cfg(test)]
    REPORTED.with(|r| r.borrow_mut().push(event));
    #[cfg(not(test))]
    {
        use taiko_proto::online;
        let request = online::Request::ClientEvent {
            token: cfg.online_token.clone(),
            event: event.name().into(),
            key: crate::score_outbox::new_key(),
        };
        let server = cfg.online_server.clone();
        std::thread::spawn(move || {
            if let Err(e) = online::call(&server, request) {
                eprintln!("telling the server about {}: {e}", event.name());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logged_in() -> Config {
        Config {
            online_token: "t".into(),
            music_volume: 0.6,
            master_volume: 0.8,
            ..Config::default()
        }
    }

    #[test]
    fn names_are_the_servers_and_one_offs_go_once() {
        take_reported();
        let names = [
            Event::Logo,
            Event::StrayNote,
            Event::Flip,
            Event::Curious,
            Event::Deaf,
            Event::Again,
            Event::MapSave,
        ]
        .map(Event::name);
        // The server's `ClientEvent` list, word for word.
        assert_eq!(
            names,
            [
                "ura_logo",
                "ura_stray_note",
                "ura_flip",
                "ura_curious",
                "ura_deaf",
                "ura_again",
                "ura_map_save"
            ]
        );
        let cfg = logged_in();
        report(&cfg, Event::Logo);
        report(&cfg, Event::Logo);
        report(&cfg, Event::MapSave);
        report(&cfg, Event::MapSave);
        report(&Config::default(), Event::Flip);
        assert_eq!(
            take_reported(),
            vec![Event::Logo, Event::MapSave, Event::MapSave]
        );
    }

    #[test]
    fn fifty_starts_of_one_map_in_a_row() {
        let (a, b) = (std::path::Path::new("a.osu"), std::path::Path::new("b.osu"));
        let mut again = Again::default();
        for _ in 0..AGAIN - 1 {
            assert!(!again.start(a));
        }
        // Another map in between starts the count over.
        assert!(!again.start(b));
        for _ in 0..AGAIN - 1 {
            assert!(!again.start(a));
        }
        assert!(again.start(a));
        assert!(!again.start(a));
    }

    #[test]
    fn whole_plays_report_only_what_the_game_alone_sees() {
        take_reported();
        let mut cfg = logged_in();
        whole_play(&cfg, 0.99);
        assert_eq!(take_reported(), Vec::<Event>::new());
        cfg.music_volume = 0.0;
        whole_play(&cfg, 0.94);
        assert_eq!(take_reported(), Vec::<Event>::new());
        whole_play(&cfg, 0.95);
        assert_eq!(take_reported(), vec![Event::Deaf]);
        // Everything muted counts too.
        cfg.music_volume = 0.6;
        cfg.master_volume = 0.0;
        whole_play(&cfg, 0.97);
        assert_eq!(take_reported(), vec![Event::Deaf]);
    }
}
