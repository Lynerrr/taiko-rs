//! The music player and the system: media keys, and the song in the desktop's media controls
//! (MPRIS on Linux, the media overlay on Windows). On a thread of its own, which holds them.

use std::sync::mpsc::{self, Receiver, Sender};

/// A media key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Toggle,
    Play,
    Pause,
    Next,
    Prev,
}

enum Message {
    Song { title: String, artist: String },
    Playing(bool),
    Stopped,
}

pub struct Media {
    tx: Sender<Message>,
    pub keys: Receiver<Key>,
}

impl Media {
    /// `window`: the window's handle (Windows needs one).
    pub fn start(window: Option<usize>) -> Media {
        let (tx, rx) = mpsc::channel();
        let (key_tx, keys) = mpsc::channel();
        std::thread::spawn(move || run(window, rx, key_tx));
        Media { tx, keys }
    }

    pub fn song(&self, title: String, artist: String) {
        let _ = self.tx.send(Message::Song { title, artist });
    }

    pub fn playing(&self, playing: bool) {
        let _ = self.tx.send(Message::Playing(playing));
    }

    pub fn stopped(&self) {
        let _ = self.tx.send(Message::Stopped);
    }
}

#[cfg(any(target_os = "linux", windows))]
fn run(window: Option<usize>, rx: Receiver<Message>, keys: Sender<Key>) {
    use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

    let hwnd = window.map(|w| w as *mut std::ffi::c_void);
    #[cfg(not(windows))]
    let _ = hwnd;
    #[cfg(not(windows))]
    let hwnd = None;
    let config = PlatformConfig { dbus_name: "taiko_rs", display_name: "taiko.rs", hwnd };
    let mut controls = match MediaControls::new(config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("media keys unavailable: {e:?}");
            return;
        }
    };
    let attached = controls.attach(move |event| {
        let key = match event {
            MediaControlEvent::Toggle => Key::Toggle,
            MediaControlEvent::Play => Key::Play,
            MediaControlEvent::Pause => Key::Pause,
            MediaControlEvent::Next => Key::Next,
            MediaControlEvent::Previous => Key::Prev,
            _ => return,
        };
        let _ = keys.send(key);
    });
    if let Err(e) = attached {
        eprintln!("media keys unavailable: {e:?}");
        return;
    }
    for message in rx {
        let done = match message {
            Message::Song { title, artist } => controls.set_metadata(MediaMetadata { title: Some(&title), artist: Some(&artist), ..Default::default() }),
            Message::Playing(true) => controls.set_playback(MediaPlayback::Playing { progress: None }),
            Message::Playing(false) => controls.set_playback(MediaPlayback::Paused { progress: None }),
            Message::Stopped => controls.set_playback(MediaPlayback::Stopped),
        };
        if let Err(e) = done {
            eprintln!("media controls: {e:?}");
        }
    }
}

/// Nothing to talk to here: the channels just stay quiet.
#[cfg(not(any(target_os = "linux", windows)))]
fn run(_: Option<usize>, rx: Receiver<Message>, _keys: Sender<Key>) {
    for _ in rx {}
}
