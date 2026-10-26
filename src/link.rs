//! `taiko-rs://` links: a score's "Watch replay" button (on the website, and under the Discord
//! bot's plays) opens that score in the game, ready to watch. The game makes itself their
//! handler when it starts (Windows: the user's registry; Linux: a desktop entry), and a link
//! opened while it's running goes to the running game over a local socket instead of starting
//! a second one.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What a link opens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Link {
    /// An online score, by its `OnlineScore::id`.
    Score(u64),
}

impl Link {
    pub fn url(self) -> String {
        match self {
            Link::Score(id) => format!("taiko-rs://score/{id}"),
        }
    }
}

/// `taiko-rs://score/123` (browsers may add a slash or a query).
pub fn parse(text: &str) -> Option<Link> {
    let text = text.trim();
    let rest = text.get(..11).filter(|s| s.eq_ignore_ascii_case("taiko-rs://")).map(|_| &text[11..])?;
    let path = rest.split(['?', '#']).next()?.trim_end_matches('/');
    match path.split_once('/')? {
        ("score" | "s", id) => id.parse().ok().map(Link::Score),
        _ => None,
    }
}

/// What a later launch says to the running game, before the link; it answers "ok".
const HELLO: &str = "taiko-rs link";

/// Where the running game says which port it listens on.
fn port_file() -> Option<PathBuf> {
    Some(crate::config::user_dir(true)?.join("link-port"))
}

/// Hands `link` to a running game; false if none took it (then this launch opens it).
pub fn forward(link: Link) -> bool {
    let Some(port) = port_file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| t.trim().parse::<u16>().ok()) else { return false };
    send(port, link)
}

/// Says `link` to whatever listens on `port`; true if it was a game and took it.
fn send(port: u16, link: Link) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_millis(500)) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if writeln!(stream, "{HELLO} {}", link.url()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Takes links from later launches (`forward`) for as long as the game runs.
pub fn listen(on_link: impl Fn(Link) + Send + 'static) {
    let Some(file) = port_file() else { return };
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else { return };
    let Ok(addr) = listener.local_addr() else { return };
    let written = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&file, addr.port().to_string()));
    if let Err(e) = written {
        eprintln!("links: {}: {e}", file.display());
        return;
    }
    serve(listener, on_link);
}

fn serve(listener: TcpListener, on_link: impl Fn(Link) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let Some(link) = line.trim().strip_prefix(HELLO).and_then(parse) else { continue };
            let _ = (&stream).write_all(b"ok\n");
            on_link(link);
        }
    });
}

/// A build run from its cargo `target` folder: it doesn't take links over from an installed
/// game that still exists.
fn dev_build(exe: &Path) -> bool {
    exe.components().any(|c| c.as_os_str() == "target")
}

/// Makes this game the handler of `taiko-rs://` links (on a thread: it runs `reg` or
/// `xdg-mime`), unless it already is.
pub fn register() {
    let Ok(exe) = std::env::current_exe() else { return };
    std::thread::spawn(move || register_exe(&exe));
}

#[cfg(windows)]
fn register_exe(exe: &Path) {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const KEY: &str = r"HKCU\Software\Classes\taiko-rs";
    let command = format!("\"{}\" \"%1\"", exe.display());
    let open = format!(r"{KEY}\shell\open\command");
    // `reg query` prints "    (Default)    REG_SZ    <value>".
    let current = Command::new("reg")
        .args(["query", &open, "/ve"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string())));
    if current.as_deref() == Some(command.as_str()) {
        return;
    }
    let current_exe = current.as_deref().and_then(|c| c.strip_prefix('"')?.split('"').next()).map(PathBuf::from);
    if dev_build(exe) && current_exe.is_some_and(|e| e.is_file()) {
        return;
    }
    let icon = format!("\"{}\",0", exe.display());
    let values: [(&str, Option<&str>, &str); 4] =
        [(KEY, None, "URL:taiko.rs"), (KEY, Some("URL Protocol"), ""), (&format!(r"{KEY}\DefaultIcon"), None, &icon), (&open, None, &command)];
    for (key, name, data) in values {
        let mut reg = Command::new("reg");
        reg.args(["add", key]);
        match name {
            Some(n) => reg.args(["/v", n]),
            None => reg.arg("/ve"),
        };
        let ok = reg.args(["/d", data, "/f"]).creation_flags(CREATE_NO_WINDOW).output().is_ok_and(|o| o.status.success());
        if !ok {
            eprintln!("links: couldn't register taiko-rs:// ({key})");
            return;
        }
    }
    eprintln!("links: taiko-rs:// opens {}", exe.display());
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
fn register_exe(exe: &Path) {
    use std::process::{Command, Stdio};
    const NAME: &str = "taiko-rs-link.desktop";
    let Some(dir) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("applications"))
    else {
        return;
    };
    let file = dir.join(NAME);
    let entry = desktop_entry(exe);
    let old = std::fs::read_to_string(&file).ok();
    if old.as_deref() == Some(entry.as_str()) {
        return;
    }
    let old_exe = old.as_deref().and_then(entry_exe);
    if dev_build(exe) && old_exe.is_some_and(|e| e.is_file()) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, &entry)) {
        eprintln!("links: {}: {e}", file.display());
        return;
    }
    let quiet = |cmd: &mut Command| cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    let _ = quiet(Command::new("update-desktop-database").arg("-q").arg(&dir));
    if quiet(Command::new("xdg-mime").args(["default", NAME, "x-scheme-handler/taiko-rs"])) {
        eprintln!("links: taiko-rs:// opens {}", exe.display());
    } else {
        eprintln!("links: no xdg-mime to register taiko-rs:// with");
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "android")))))]
fn register_exe(_exe: &Path) {}

/// The (hidden) desktop entry that opens links with `exe`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry(exe: &Path) -> String {
    // Inside quotes the desktop entry spec escapes ", `, $ and \ with a backslash.
    let quoted: String = exe.to_string_lossy().chars().flat_map(|c| if "\"`$\\".contains(c) { vec!['\\', c] } else { vec![c] }).collect();
    format!(
        "[Desktop Entry]\nType=Application\nName=taiko.rs\nComment=Opens taiko.rs links (scores to watch)\nExec=\"{quoted}\" %u\nIcon=taiko-rs\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/taiko-rs;\n"
    )
}

/// The game a desktop entry of ours opens.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn entry_exe(entry: &str) -> Option<PathBuf> {
    let exec = entry.lines().find_map(|l| l.strip_prefix("Exec=\""))?;
    let mut out = String::new();
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            '"' => return Some(PathBuf::from(out)),
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_links_parse() {
        assert_eq!(parse("taiko-rs://score/42"), Some(Link::Score(42)));
        assert_eq!(parse(" TAIKO-RS://score/42/ "), Some(Link::Score(42)));
        assert_eq!(parse("taiko-rs://s/7?from=discord"), Some(Link::Score(7)));
        assert_eq!(parse(&Link::Score(9).url()), Some(Link::Score(9)));
        assert_eq!(parse("taiko-rs://score/"), None);
        assert_eq!(parse("taiko-rs://score/x"), None);
        assert_eq!(parse("taiko-rs://map/1"), None);
        assert_eq!(parse("https://score/1"), None);
        assert_eq!(parse("/home/me/Songs"), None);
    }

    #[test]
    fn desktop_entries_name_their_game() {
        let exe = Path::new("/opt/my \"games\"/taiko-rs");
        let entry = desktop_entry(exe);
        assert!(entry.contains("Exec=\"/opt/my \\\"games\\\"/taiko-rs\" %u") && entry.contains("MimeType=x-scheme-handler/taiko-rs;"), "{entry}");
        assert_eq!(entry_exe(&entry).as_deref(), Some(exe));
        assert!(dev_build(Path::new("/src/taiko-rs/target/release/taiko-rs")) && !dev_build(exe));
    }

    #[test]
    fn a_running_game_takes_links_from_later_launches() {
        // No user folder in tests, so no port file: nobody to forward to.
        assert!(!forward(Link::Score(1)));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        serve(listener, move |l| tx.send(l).unwrap());
        assert!(send(port, Link::Score(5)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Link::Score(5)));
        // Something else on the port the file names (the game long gone): this launch opens it.
        let other = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = other.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (s, _) = other.accept().unwrap();
            let _ = BufReader::new(&s).read_line(&mut String::new());
        });
        assert!(!send(port, Link::Score(5)));
    }
}
