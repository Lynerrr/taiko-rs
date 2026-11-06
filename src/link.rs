//! `taiko-rs://` links: a score's "Watch replay" button (on the website, and under the Discord
//! bot's plays) opens that score in the game, ready to watch. The game makes itself their
//! handler when it starts (Windows: the user's registry; Linux: a desktop entry), and a link
//! opened while it's running goes to the running game over a local socket instead of starting
//! a second one.
//!
//! The same goes for a map archive (`.rsz`) opened from the file manager: a later launch hands
//! its path to the running game over the socket, which imports it. Paths only ever come that
//! way, from a program on this computer, never in a link a web page could open.

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
    let rest = text
        .get(..11)
        .filter(|s| s.eq_ignore_ascii_case("taiko-rs://"))
        .map(|_| &text[11..])?;
    let path = rest.split(['?', '#']).next()?.trim_end_matches('/');
    match path.split_once('/')? {
        ("score" | "s", id) => id.parse().ok().map(Link::Score),
        _ => None,
    }
}

/// What a later launch says to the running game, before the link; it answers "ok".
const HELLO: &str = "taiko-rs link";
/// The same, before the path of a file to open.
const OPEN: &str = "taiko-rs open";

/// What a later launch handed over.
#[derive(Debug, PartialEq)]
pub enum Arrival {
    Link(Link),
    /// A map archive to import (an absolute path).
    Open(PathBuf),
}

/// Where the running game says which port it listens on.
fn port_file() -> Option<PathBuf> {
    Some(crate::config::user_dir(true)?.join("link-port"))
}

fn running_port() -> Option<u16> {
    port_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| t.trim().parse::<u16>().ok())
}

/// Hands `link` to a running game; false if none took it (then this launch opens it).
pub fn forward(link: Link) -> bool {
    running_port().is_some_and(|port| send(port, &format!("{HELLO} {}", link.url())))
}

/// Hands the file at `path` to a running game to open; false if none took it.
pub fn forward_open(path: &Path) -> bool {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let Some(text) = path.to_str().filter(|t| !t.contains('\n')) else {
        return false;
    };
    running_port().is_some_and(|port| send(port, &format!("{OPEN} {text}")))
}

/// Says `line` to whatever listens on `port`; true if it was a game and took it.
fn send(port: u16, line: &str) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(
        &(Ipv4Addr::LOCALHOST, port).into(),
        Duration::from_millis(500),
    ) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if writeln!(stream, "{line}").is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Takes links and files from later launches (`forward`, `forward_open`) for as long as the
/// game runs.
pub fn listen(on_link: impl Fn(Arrival) + Send + 'static) {
    let Some(file) = port_file() else { return };
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else {
        return;
    };
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    let written = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&file, addr.port().to_string()));
    if let Err(e) = written {
        eprintln!("links: {}: {e}", file.display());
        return;
    }
    serve(listener, on_link);
}

/// The local path in a `file://` URL (what some file managers hand a program to open), with
/// its `%xx` escapes undone. Anything else, or a file on another host, is `None`.
pub fn file_url(text: &str) -> Option<PathBuf> {
    let rest = text
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("file://"))
        .map(|_| &text[7..])?;
    // `file:///path` or `file://localhost/path`.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    let path = String::from_utf8(out).ok()?;
    // Windows: `file:///C:/maps/a.rsz`.
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => path[1..].to_string(),
        _ => path,
    };
    Some(PathBuf::from(path))
}

/// What a later launch's line asks for.
fn arrival(line: &str) -> Option<Arrival> {
    let line = line.trim_end_matches(['\r', '\n']);
    if let Some(path) = line.strip_prefix(OPEN).and_then(|p| p.strip_prefix(' ')) {
        let path = PathBuf::from(path);
        return (path.is_absolute() && crate::library::is_map_archive(&path))
            .then_some(Arrival::Open(path));
    }
    line.trim()
        .strip_prefix(HELLO)
        .and_then(parse)
        .map(Arrival::Link)
}

fn serve(listener: TcpListener, on_link: impl Fn(Arrival) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let Some(arrival) = arrival(&line) else {
                continue;
            };
            let _ = (&stream).write_all(b"ok\n");
            on_link(arrival);
        }
    });
}

/// A build run from its cargo `target` folder: it doesn't take links over from an installed
/// game that still exists.
#[cfg(any(windows, target_os = "linux", test))]
fn dev_build(exe: &Path) -> bool {
    exe.components().any(|c| c.as_os_str() == "target")
}

/// Makes this game the handler of `taiko-rs://` links and of `.rsz` map archives (on a thread:
/// it runs `reg` or `xdg-mime`), unless it already is.
pub fn register() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    std::thread::spawn(move || register_exe(&exe));
}

#[cfg(windows)]
fn register_exe(exe: &Path) {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const KEY: &str = r"HKCU\Software\Classes\taiko-rs";
    // `.rsz` files are of the type `taiko-rs.map`, which this game opens.
    const EXT: &str = r"HKCU\Software\Classes\.rsz";
    const MAP: &str = r"HKCU\Software\Classes\taiko-rs.map";
    let command = format!("\"{}\" \"%1\"", exe.display());
    let open = format!(r"{KEY}\shell\open\command");
    let map_open = format!(r"{MAP}\shell\open\command");
    // `reg query` prints "    (Default)    REG_SZ    <value>".
    let query = |key: &str| {
        Command::new("reg")
            .args(["query", key, "/ve"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()))
            })
    };
    let current = query(&open);
    if current.as_deref() == Some(command.as_str())
        && query(&map_open).as_deref() == Some(command.as_str())
        && query(EXT).as_deref() == Some("taiko-rs.map")
    {
        return;
    }
    let current_exe = current
        .as_deref()
        .and_then(|c| c.strip_prefix('"')?.split('"').next())
        .map(PathBuf::from);
    if dev_build(exe) && current_exe.is_some_and(|e| e.is_file()) {
        return;
    }
    let icon = format!("\"{}\",0", exe.display());
    let values: [(&str, Option<&str>, &str); 8] = [
        (KEY, None, "URL:taiko.rs"),
        (KEY, Some("URL Protocol"), ""),
        (&format!(r"{KEY}\DefaultIcon"), None, &icon),
        (&open, None, &command),
        (MAP, None, "taiko.rs map"),
        (&format!(r"{MAP}\DefaultIcon"), None, &icon),
        (&map_open, None, &command),
        (EXT, None, "taiko-rs.map"),
    ];
    for (key, name, data) in values {
        let mut reg = Command::new("reg");
        reg.args(["add", key]);
        match name {
            Some(n) => reg.args(["/v", n]),
            None => reg.arg("/ve"),
        };
        let ok = reg
            .args(["/d", data, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .is_ok_and(|o| o.status.success());
        if !ok {
            eprintln!("links: couldn't register taiko-rs:// ({key})");
            return;
        }
    }
    eprintln!("links: taiko-rs:// and .rsz open {}", exe.display());
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
    let quiet = |cmd: &mut Command| {
        cmd.stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    // The `.rsz` file type, for file managers to know (shared-mime-info).
    if let Some(mime) = dir.parent().map(|d| d.join("mime")) {
        let packages = mime.join("packages");
        let wrote = std::fs::create_dir_all(&packages)
            .and_then(|_| std::fs::write(packages.join("taiko-rs.xml"), MIME_PACKAGE));
        match wrote {
            Ok(()) => {
                let _ = quiet(Command::new("update-mime-database").arg(&mime));
            }
            Err(e) => eprintln!("links: {}: {e}", packages.display()),
        }
    }
    let _ = quiet(Command::new("update-desktop-database").arg("-q").arg(&dir));
    let links =
        quiet(Command::new("xdg-mime").args(["default", NAME, "x-scheme-handler/taiko-rs"]));
    let maps = quiet(Command::new("xdg-mime").args(["default", NAME, RSZ_MIME]));
    if links && maps {
        eprintln!("links: taiko-rs:// and .rsz open {}", exe.display());
    } else {
        eprintln!("links: no xdg-mime to register taiko-rs:// and .rsz with");
    }
}

/// The MIME type of `.rsz` files on Linux.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const RSZ_MIME: &str = "application/x-taiko-rs-archive";

/// What tells file managers that `*.rsz` is that type.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const MIME_PACKAGE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/x-taiko-rs-archive">
    <comment>taiko.rs map</comment>
    <sub-class-of type="application/zip"/>
    <glob pattern="*.rsz"/>
  </mime-type>
</mime-info>
"#;

#[cfg(not(any(
    windows,
    all(unix, not(target_os = "macos"), not(target_os = "android"))
)))]
fn register_exe(_exe: &Path) {}

/// The (hidden) desktop entry that opens links with `exe`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry(exe: &Path) -> String {
    // Inside quotes the desktop entry spec escapes ", `, $ and \ with a backslash.
    let quoted: String = exe
        .to_string_lossy()
        .chars()
        .flat_map(|c| {
            if "\"`$\\".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect();
    format!(
        "[Desktop Entry]\nType=Application\nName=taiko.rs\nComment=Opens taiko.rs links (scores to watch) and maps (.rsz)\nExec=\"{quoted}\" %u\nIcon=taiko-rs\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/taiko-rs;{RSZ_MIME};\n"
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
        assert!(
            entry.contains("Exec=\"/opt/my \\\"games\\\"/taiko-rs\" %u")
                && entry
                    .contains("MimeType=x-scheme-handler/taiko-rs;application/x-taiko-rs-archive;"),
            "{entry}"
        );
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
        let score = format!("{HELLO} {}", Link::Score(5).url());
        assert!(send(port, &score));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)),
            Ok(Arrival::Link(Link::Score(5)))
        );
        // A map archive opened from the file manager.
        let rsz = std::env::temp_dir().join("my map.rsz");
        assert!(send(port, &format!("{OPEN} {}", rsz.display())));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)),
            Ok(Arrival::Open(rsz))
        );
        // Something else on the port the file names (the game long gone): this launch opens it.
        let other = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = other.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (s, _) = other.accept().unwrap();
            let _ = BufReader::new(&s).read_line(&mut String::new());
        });
        assert!(!send(port, &score));
    }

    #[test]
    fn file_urls_become_paths() {
        assert_eq!(
            file_url("file:///home/me/My%20Map%20%5B1%5D.rsz"),
            Some(PathBuf::from("/home/me/My Map [1].rsz"))
        );
        assert_eq!(
            file_url("file://localhost/tmp/%D0%BA%D0%B0%D1%80%D1%82%D0%B0.rsz"),
            Some(PathBuf::from("/tmp/карта.rsz"))
        );
        assert_eq!(
            file_url("FILE:///C:/maps/a.rsz"),
            Some(PathBuf::from("C:/maps/a.rsz"))
        );
        assert_eq!(file_url("file://server/share/a.rsz"), None);
        assert_eq!(file_url("/home/me/a.rsz"), None);
        assert_eq!(file_url("taiko-rs://score/1"), None);
    }

    #[test]
    fn only_a_map_archive_at_a_full_path_is_opened() {
        let dir = std::env::temp_dir();
        let open = |p: &Path| arrival(&format!("{OPEN} {}\n", p.display()));
        assert_eq!(
            open(&dir.join("a.rsz")),
            Some(Arrival::Open(dir.join("a.rsz")))
        );
        assert!(open(&dir.join("set.OSZ")).is_some());
        assert_eq!(open(Path::new("a.rsz")), None, "relative");
        assert_eq!(open(&dir.join("notes.txt")), None);
        assert_eq!(arrival("taiko-rs opener /x.rsz"), None);
    }
}
