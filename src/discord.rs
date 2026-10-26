//! Discord Rich Presence ("Playing taiko.rs" with what you're doing) over Discord's local IPC
//! socket: a Unix socket on Linux/macOS, a named pipe on Windows. No SDK; when Discord isn't
//! running nothing happens and it retries now and then.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The taiko.rs application on Discord's developer portal (its name is the "Playing ..." line).
/// `discord_app_id` in the config overrides it.
pub const DEFAULT_APP_ID: &str = "1563250293225951262";

/// What the status shows: two lines and the elapsed time since `start` (Unix seconds).
#[derive(Clone, PartialEq, Debug)]
pub struct Activity {
    pub details: String,
    pub state: String,
    pub start: u64,
}

/// What the presence thread is asked to do.
enum Msg {
    Show(Option<Activity>),
    /// Clear the status and close the connection, then stop.
    Quit,
}

/// Handle to the presence thread. Dropping it clears the status and closes the connection
/// properly, waiting a moment for that: left to the socket closing as the game exits, some
/// clients (and Windows' pipes) kept showing it.
pub struct Presence {
    tx: Sender<Msg>,
    stopped: mpsc::Receiver<()>,
}

impl Drop for Presence {
    fn drop(&mut self) {
        if self.tx.send(Msg::Quit).is_ok() {
            let _ = self.stopped.recv_timeout(Duration::from_millis(1000));
        }
    }
}

impl Presence {
    pub fn start(app_id: String) -> Presence {
        let (tx, rx) = mpsc::channel::<Msg>();
        let (stopped_tx, stopped) = mpsc::channel();
        std::thread::spawn(move || {
            // Told when this thread ends, however it does.
            struct Stopped(Sender<()>);
            impl Drop for Stopped {
                fn drop(&mut self) {
                    let _ = self.0.send(());
                }
            }
            let _stopped = Stopped(stopped_tx);
            let mut want: Option<Activity> = None;
            // What Discord shows now (None = nothing sent on this connection yet).
            let mut shown: Option<Option<Activity>> = None;
            let mut conn: Option<Conn> = None;
            let mut next_try = Instant::now();
            let mut last_send: Option<Instant> = None;
            let mut quit = false;
            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(Msg::Show(a)) => want = a,
                    Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => quit = true,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                while let Ok(m) = rx.try_recv() {
                    match m {
                        Msg::Show(a) => want = a,
                        Msg::Quit => quit = true,
                    }
                }
                if quit {
                    // Nothing shown, then a goodbye: the status goes at once.
                    if let Some(c) = &mut conn {
                        if shown.as_ref().is_some_and(|s| s.is_some()) {
                            let _ = c.set(None);
                        }
                        c.close();
                    }
                    return;
                }
                if conn.is_none() && Instant::now() >= next_try {
                    next_try = Instant::now() + Duration::from_secs(15);
                    conn = Conn::open(&app_id);
                    shown = None;
                }
                let Some(c) = &mut conn else { continue };
                // Discord allows about 5 updates per 20 s.
                if shown.as_ref() != Some(&want) && last_send.is_none_or(|t| t.elapsed() >= Duration::from_secs(3)) {
                    last_send = Some(Instant::now());
                    if c.set(want.as_ref()).is_ok() {
                        shown = Some(want.clone());
                    } else {
                        conn = None;
                    }
                }
            }
        });
        Presence { tx, stopped }
    }

    pub fn set(&self, activity: Option<Activity>) {
        let _ = self.tx.send(Msg::Show(activity));
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(unix)]
type Stream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Stream = std::fs::File;

struct Conn {
    s: Stream,
    nonce: u64,
}

/// Where Discord (or a sandboxed / third-party client) listens.
fn socket_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(unix)]
    {
        let mut dirs: Vec<PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect();
        dirs.push(PathBuf::from("/tmp"));
        for d in dirs {
            for sub in ["", "app/com.discordapp.Discord", "app/com.discordapp.DiscordCanary", "snap.discord", "app/dev.vencord.Vesktop", ".flatpak/dev.vencord.Vesktop/xdg-run"] {
                for i in 0..10 {
                    out.push(d.join(sub).join(format!("discord-ipc-{i}")));
                }
            }
        }
    }
    #[cfg(windows)]
    for i in 0..10 {
        out.push(PathBuf::from(format!(r"\\.\pipe\discord-ipc-{i}")));
    }
    out
}

impl Conn {
    fn open(app_id: &str) -> Option<Conn> {
        for path in socket_paths() {
            #[cfg(unix)]
            let s = Stream::connect(&path).ok();
            #[cfg(windows)]
            let s = std::fs::OpenOptions::new().read(true).write(true).open(&path).ok();
            let Some(s) = s else { continue };
            #[cfg(unix)]
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let mut c = Conn { s, nonce: 0 };
            let hello = serde_json::json!({ "v": 1, "client_id": app_id });
            if c.send(0, &hello).is_ok() && c.recv().is_ok_and(|(op, body)| op == 1 && body.contains("READY")) {
                return Some(c);
            }
        }
        None
    }

    fn set(&mut self, a: Option<&Activity>) -> std::io::Result<()> {
        self.nonce += 1;
        let activity = a.map(|a| {
            let mut v = serde_json::json!({
                "details": field(&a.details),
                "timestamps": { "start": a.start },
                "assets": { "large_image": "logo", "large_text": "taiko.rs" },
            });
            if !a.state.is_empty() {
                v["state"] = field(&a.state).into();
            }
            v
        });
        let msg = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity },
            "nonce": self.nonce.to_string(),
        });
        self.send(1, &msg)?;
        // Read the reply so the socket doesn't fill up; a closed pipe is an error.
        self.recv().map(|_| ())
    }

    /// Says goodbye (opcode 2), so Discord drops the status right away rather than when it
    /// notices the socket closed.
    fn close(&mut self) {
        let _ = self.send(2, &serde_json::json!({}));
    }

    /// One frame: opcode, payload length (both little-endian u32), JSON.
    fn send(&mut self, op: u32, v: &serde_json::Value) -> std::io::Result<()> {
        let body = v.to_string();
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
        frame.extend_from_slice(body.as_bytes());
        self.s.write_all(&frame)?;
        self.s.flush()
    }

    fn recv(&mut self) -> std::io::Result<(u32, String)> {
        let mut head = [0u8; 8];
        self.s.read_exact(&mut head)?;
        let op = u32::from_le_bytes(head[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
        if len > 1 << 20 {
            return Err(std::io::Error::other("oversized frame"));
        }
        let mut body = vec![0u8; len];
        self.s.read_exact(&mut body)?;
        if op == 2 {
            return Err(std::io::Error::other("closed by Discord"));
        }
        Ok((op, String::from_utf8_lossy(&body).into_owned()))
    }
}

/// Discord wants 2..=128 characters per line.
fn field(s: &str) -> String {
    let mut out: String = s.chars().take(128).collect();
    while out.chars().count() < 2 {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_fit_discord_limits() {
        assert_eq!(field("a"), "a ");
        assert_eq!(field(&"x".repeat(300)).chars().count(), 128);
        assert_eq!(field("Playing"), "Playing");
    }

    #[cfg(unix)]
    #[test]
    fn speaks_the_ipc_protocol() {
        // A fake Discord: answers the handshake with READY and acknowledges one SET_ACTIVITY.
        let dir = std::env::temp_dir().join(format!("taiko-ipc-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("discord-ipc-0");
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut c = Conn { s, nonce: 0 };
            let (op, hello) = c.recv().unwrap();
            assert_eq!(op, 0);
            assert!(hello.contains("\"client_id\":\"123\""));
            c.send(1, &serde_json::json!({ "cmd": "DISPATCH", "evt": "READY" })).unwrap();
            let (_, set) = c.recv().unwrap();
            c.send(1, &serde_json::json!({ "cmd": "SET_ACTIVITY", "evt": null })).unwrap();
            // Leaving: the status cleared, then a goodbye (opcode 2, which `recv` reports).
            let (_, cleared) = c.recv().unwrap();
            c.send(1, &serde_json::json!({ "cmd": "SET_ACTIVITY", "evt": null })).unwrap();
            (set, cleared, c.recv().is_err())
        });
        let s = Stream::connect(&path).unwrap();
        let mut c = Conn { s, nonce: 0 };
        c.send(0, &serde_json::json!({ "v": 1, "client_id": "123" })).unwrap();
        assert!(c.recv().unwrap().1.contains("READY"));
        c.set(Some(&Activity { details: "Artist - Title [Oni]".into(), state: "Playing".into(), start: 5 })).unwrap();
        c.set(None).unwrap();
        c.close();
        let (set, cleared, goodbye) = server.join().unwrap();
        assert!(set.contains("SET_ACTIVITY") && set.contains("Artist - Title [Oni]") && set.contains("\"start\":5"));
        assert!(cleared.contains("\"activity\":null"), "{cleared}");
        assert!(goodbye);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
