//! Getting into a mapping session, up to the editor opening on it.
//!
//! The host opens a session for a set on its disk: the server answers with the files it still
//! needs (sent in pieces), the session's state, and, for a new session, asks for the first
//! snapshot, which the host makes of its charts. Anyone else joining gets the state (with the
//! set's files listed by sha256), fetches the files it doesn't have into a folder of the set's
//! name, and gets the snapshot and the ops after it. Either way, once everything is in, the
//! editor opens on a difficulty of the session (`Ready`). What comes meanwhile (presence, chat,
//! comments) is kept for the session to take when it starts.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine as _;
use sha2::{Digest, Sha256};
use taiko_proto::mapping::{
    ASSET_PIECE, AssetRef, LinkEvent, MapDown, MapUp, Sequenced, SessionState, SetDescriptor,
    SnapshotData, SnapshotPieces, snapshot_lines,
};

use super::host_snapshot;

/// A file of a set may be at most this big, and all of them together (the custom map upload's
/// limits, which a session's result goes through).
pub const MAX_ASSET: u64 = 40 << 20;
pub const MAX_ASSETS: u64 = 200 << 20;
/// A piece of a file asked for and not come by then is asked for again.
const ASK_AGAIN: Duration = Duration::from_secs(3);

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// A chart's difficulty name (`Version`).
pub fn version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("Version:"))
        .map(|v| v.trim().to_string())
}

/// Where a file the session names goes in the set's folder: a relative path of plain parts
/// (storyboard images can be in folders), never outside it.
fn local_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut p = dir.to_path_buf();
    for part in name.split('/') {
        let bad = part.is_empty()
            || part == "."
            || part == ".."
            || part
                .chars()
                .any(|c| c.is_control() || r#"<>:"\|?*"#.contains(c));
        if bad {
            return None;
        }
        p.push(part);
    }
    Some(p)
}

/// A chart file: an `.osu`, or a `.tko` (an `.osu` with the client's effects on top).
fn is_chart(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("osu") || e.eq_ignore_ascii_case("tko"))
}

/// Whether a chart's text is a `.tko`.
pub fn is_tko_text(text: &str) -> bool {
    crate::tko::format::version_of(text).is_some()
}

/// The chart files in a folder: (path, text), one per difficulty. A `.tko` made from an `.osu`
/// keeps that `.osu` next to it under the same difficulty name; the `.tko` is the one mapped
/// on, so it stands for the difficulty.
pub fn charts_in(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_chart(p))
        .filter_map(|p| Some((p.clone(), std::fs::read_to_string(&p).ok()?)))
        .collect();
    out.sort();
    let tko_names: Vec<String> = out
        .iter()
        .filter(|(_, text)| is_tko_text(text))
        .filter_map(|(_, text)| version(text))
        .collect();
    out.retain(|(_, text)| {
        is_tko_text(text) || version(text).is_none_or(|v| !tko_names.contains(&v))
    });
    out
}

/// A set on the host's disk, as a session is opened for it.
pub struct HostSet {
    pub dir: PathBuf,
    /// (difficulty name, text) of each chart, in the session's order.
    pub charts: Vec<(String, String)>,
    /// Every other file, by its path in the folder.
    pub assets: Vec<(AssetRef, PathBuf)>,
}

impl HostSet {
    /// Reads the set in `dir`: its `.osu` charts and its other files (in folders too, but none
    /// hidden), within the size limits.
    pub fn read(dir: &Path) -> Result<HostSet, String> {
        let charts: Vec<(String, String)> = charts_in(dir)
            .into_iter()
            .map(|(p, text)| {
                let name = version(&text).unwrap_or_else(|| {
                    p.file_stem()
                        .map_or("?".into(), |s| s.to_string_lossy().into_owned())
                });
                (name, text)
            })
            .collect();
        if charts.is_empty() {
            return Err("the set has no .osu or .tko difficulty".into());
        }
        let mut assets = Vec::new();
        let mut total = 0;
        let mut dirs = vec![(dir.to_path_buf(), String::new())];
        while let Some((d, prefix)) = dirs.pop() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(&d)
                .map_err(|e| e.to_string())?
                .flatten()
                .map(|e| e.path())
                .collect();
            entries.sort();
            for p in entries {
                let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                let rel = format!("{prefix}{name}");
                if p.is_dir() {
                    dirs.push((p.clone(), format!("{rel}/")));
                    continue;
                }
                if prefix.is_empty() && is_chart(&p) {
                    continue;
                }
                let bytes = std::fs::read(&p).map_err(|e| format!("{rel}: {e}"))?;
                let size = bytes.len() as u64;
                if size > MAX_ASSET {
                    return Err(format!("{rel} is over 40 MB"));
                }
                total += size;
                if total > MAX_ASSETS {
                    return Err("the set's files are over 200 MB".into());
                }
                let asset = AssetRef {
                    name: rel,
                    sha256: sha256_hex(&bytes),
                    size,
                };
                assets.push((asset, p));
            }
        }
        Ok(HostSet {
            dir: dir.to_path_buf(),
            charts,
            assets,
        })
    }

    pub fn descriptor(&self) -> SetDescriptor {
        let folder = self
            .dir
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let title = self
            .charts
            .first()
            .and_then(|(_, text)| {
                let key = |k: &str| {
                    text.lines()
                        .find_map(|l| l.strip_prefix(k))
                        .map(|v| v.trim().to_string())
                };
                Some(format!("{} - {}", key("Artist:")?, key("Title:")?))
            })
            .unwrap_or_else(|| folder.clone());
        SetDescriptor {
            title,
            folder,
            diffs: self.charts.iter().map(|(n, _)| n.clone()).collect(),
            assets: self.assets.iter().map(|(a, _)| a.clone()).collect(),
        }
    }

    /// The lines that send the files the server needs, in pieces.
    fn uploads(&self, needs: &[String]) -> Result<Vec<MapUp>, String> {
        let mut out = Vec::new();
        for (a, path) in self
            .assets
            .iter()
            .filter(|(a, _)| needs.contains(&a.sha256))
        {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", a.name))?;
            let total = bytes.len() as u64;
            for (i, piece) in bytes.chunks(ASSET_PIECE).enumerate() {
                out.push(MapUp::Asset {
                    sha256: a.sha256.clone(),
                    offset: (i * ASSET_PIECE) as u64,
                    total,
                    data: b64().encode(piece),
                });
            }
        }
        Ok(out)
    }
}

/// In the session with everything the editor needs.
#[derive(Debug)]
pub struct Ready {
    pub session: u64,
    pub epoch: u32,
    pub me: u32,
    pub state: SessionState,
    pub seq: u64,
    pub data: SnapshotData,
    /// Ops after the snapshot, and the rest of what came while joining.
    pub tail: Vec<Sequenced>,
    pub later: Vec<MapDown>,
    /// The difficulty to open and its chart file.
    pub open: u32,
    pub path: PathBuf,
}

pub struct Connecting {
    /// Where a joining game puts the set (its Songs folder).
    songs: PathBuf,
    host: Option<HostSet>,
    dir: Option<PathBuf>,
    /// The difficulty to open if the session has it: the one we were on when we left.
    prefer: Option<u32>,
    joined: Option<(u64, u32, u32)>,
    state: Option<SessionState>,
    pieces: SnapshotPieces,
    snapshot: Option<(u64, SnapshotData)>,
    tail: Vec<Sequenced>,
    later: Vec<MapDown>,
    fetch: VecDeque<(AssetRef, PathBuf)>,
    got: Option<(AssetRef, PathBuf, Vec<u8>)>,
    /// Since when the piece asked for last hasn't come (set on the first `tick` after asking).
    waiting: Option<Instant>,
    /// Asked again for a piece that didn't come: the host is still sending the set.
    pub stalled: bool,
    /// Bytes of the set's files fetched so far, and to fetch in all.
    pub fetched: u64,
    pub to_fetch: u64,
}

impl Connecting {
    /// The host, opening a session for its set.
    pub fn host(set: HostSet) -> Connecting {
        let mut c = Connecting::guest(PathBuf::new());
        c.dir = Some(set.dir.clone());
        c.host = Some(set);
        c
    }

    /// Back into a session left earlier: the set is the one in `dir`, where we worked on it
    /// (the host's own folder too), brought up to date.
    pub fn rejoin(dir: PathBuf, diff: u32) -> Connecting {
        let mut c = Connecting::guest(PathBuf::new());
        c.dir = Some(dir);
        c.prefer = Some(diff);
        c
    }

    /// Anyone else, joining: the set goes in a folder in `songs` (or is found there).
    pub fn guest(songs: PathBuf) -> Connecting {
        Connecting {
            songs,
            host: None,
            dir: None,
            prefer: None,
            joined: None,
            state: None,
            pieces: SnapshotPieces::default(),
            snapshot: None,
            tail: Vec::new(),
            later: Vec::new(),
            fetch: VecDeque::new(),
            got: None,
            waiting: None,
            stalled: false,
            fetched: 0,
            to_fetch: 0,
        }
    }

    /// What joining is waiting for, in words for the player.
    pub fn stage(&self) -> String {
        if self.joined.is_none() || self.state.is_none() {
            return tr!("connecting", "接続中", "подключение").into();
        }
        if self.fetched < self.to_fetch && self.stalled {
            return tr!(
                "waiting for the host to send the set's files",
                "ホストがセットのファイルを送るのを待っています",
                "жду, пока хост отправит файлы сета"
            )
            .into();
        }
        if self.fetched < self.to_fetch {
            let mb = |b: u64| format!("{:.1}", b as f64 / 1_048_576.0);
            let (got, all) = (mb(self.fetched), mb(self.to_fetch));
            return tr!("downloading the set's files {got}/{all} MB", "セットのファイルをダウンロード中 {got}/{all} MB", "скачиваю файлы сета {got}/{all} МБ"; [] [got, all]);
        }
        if self.snapshot.is_none() {
            return tr!("getting the map", "マップを取得中", "получаю карту").into();
        }
        tr!("opening", "開いています", "открываю").into()
    }

    /// How far along joining is, 0 to 1.
    pub fn progress(&self) -> f32 {
        let steps = [
            self.joined.is_some(),
            self.state.is_some(),
            self.snapshot.is_some(),
        ];
        let done = steps.iter().filter(|s| **s).count() as f32;
        let files = if self.to_fetch == 0 {
            1.0
        } else {
            self.fetched as f32 / self.to_fetch as f32
        };
        (done + files) / 4.0
    }

    /// Takes what the connection reports; the lines to send, or why joining failed.
    pub fn take(&mut self, ev: LinkEvent) -> Result<Vec<MapUp>, String> {
        match ev {
            LinkEvent::Joined {
                session,
                epoch,
                me,
                needs,
                ..
            } => {
                self.joined = Some((session, epoch, me));
                match &self.host {
                    Some(set) => set.uploads(&needs),
                    None => Ok(Vec::new()),
                }
            }
            LinkEvent::Lost(why) | LinkEvent::Refused(why) => Err(why),
            LinkEvent::Down(d) => self.down(*d),
        }
    }

    fn down(&mut self, d: MapDown) -> Result<Vec<MapUp>, String> {
        match d {
            MapDown::State(state) => {
                let first = self.state.is_none();
                let assets = state.assets.clone();
                let folder = if state.folder.is_empty() {
                    state.title.clone()
                } else {
                    state.folder.clone()
                };
                self.state = Some(state);
                if first && self.host.is_none() {
                    let dir = match &self.dir {
                        Some(dir) => dir.clone(),
                        None => self.songs.join(crate::download::safe_folder_name(&folder)),
                    };
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    for a in assets {
                        let path = local_path(&dir, &a.name)
                            .ok_or_else(|| format!("a file name outside the set: {}", a.name))?;
                        let have = std::fs::read(&path).is_ok_and(|b| sha256_hex(&b) == a.sha256);
                        if !have {
                            self.to_fetch += a.size;
                            self.fetch.push_back((a, path));
                        }
                    }
                    self.dir = Some(dir);
                    return Ok(self.next_fetch().into_iter().collect());
                }
                Ok(Vec::new())
            }
            MapDown::AssetPart {
                sha256,
                offset,
                total,
                data,
            } => {
                let Some((a, path, buf)) = &mut self.got else {
                    return Ok(Vec::new());
                };
                if a.sha256 != sha256 || offset != buf.len() as u64 {
                    return Ok(Vec::new());
                }
                let bytes = b64().decode(data).map_err(|e| e.to_string())?;
                if total > MAX_ASSET || buf.len() as u64 + bytes.len() as u64 > total {
                    return Err(format!("{} is bigger than it should be", a.name));
                }
                self.fetched += bytes.len() as u64;
                buf.extend_from_slice(&bytes);
                self.waiting = None;
                self.stalled = false;
                // The server sends one piece for each ask: ask for the next one.
                if (buf.len() as u64) < total {
                    return Ok(vec![MapUp::GetAsset {
                        sha256: a.sha256.clone(),
                        offset: buf.len() as u64,
                    }]);
                }
                if sha256_hex(buf) != a.sha256 {
                    return Err(format!("{} came broken", a.name));
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&*path, &buf).map_err(|e| format!("{}: {e}", a.name))?;
                self.got = None;
                Ok(self.next_fetch().into_iter().collect())
            }
            MapDown::Snapshot {
                seq,
                part,
                parts,
                data,
            } => {
                if let Some(s) = self.pieces.add(seq, part, parts, data) {
                    self.snapshot = Some(s);
                    self.tail.clear();
                }
                Ok(Vec::new())
            }
            // A new session: the host's charts are its first snapshot.
            MapDown::SnapshotWanted { seq } if self.snapshot.is_none() => {
                let Some(set) = &self.host else {
                    return Ok(Vec::new());
                };
                let data = host_snapshot(&set.charts)?;
                let lines = snapshot_lines(seq, &data);
                self.snapshot = Some((seq, data));
                Ok(lines)
            }
            MapDown::Op(s) => {
                if self.snapshot.is_some() {
                    self.tail.push(s);
                }
                Ok(Vec::new())
            }
            MapDown::Closed(why) => Err(why),
            MapDown::Pong | MapDown::SnapshotWanted { .. } => Ok(Vec::new()),
            other => {
                self.later.push(other);
                Ok(Vec::new())
            }
        }
    }

    fn next_fetch(&mut self) -> Option<MapUp> {
        if self.got.is_some() {
            return None;
        }
        let (a, path) = self.fetch.pop_front()?;
        let up = MapUp::GetAsset {
            sha256: a.sha256.clone(),
            offset: 0,
        };
        self.got = Some((a, path, Vec::new()));
        self.waiting = None;
        Some(up)
    }

    /// Time passing: a piece asked for that hasn't come is asked for again. The server has
    /// nothing to answer with until the host's upload of that file is complete (a guest can join
    /// while the host is still sending), and it doesn't say so.
    pub fn tick(&mut self, now: Instant) -> Vec<MapUp> {
        let Some((a, _, buf)) = &self.got else {
            self.waiting = None;
            return Vec::new();
        };
        let since = *self.waiting.get_or_insert(now);
        if now.duration_since(since) < ASK_AGAIN {
            return Vec::new();
        }
        self.waiting = Some(now);
        self.stalled = true;
        vec![MapUp::GetAsset {
            sha256: a.sha256.clone(),
            offset: buf.len() as u64,
        }]
    }

    /// Everything in: the session to open (once), with the chart files of the difficulties
    /// written where the set's folder doesn't have them yet.
    pub fn ready(&mut self) -> Option<Result<Ready, String>> {
        if self.got.is_some() || !self.fetch.is_empty() {
            return None;
        }
        let (session, epoch, me) = self.joined?;
        self.state.as_ref()?;
        self.snapshot.as_ref()?;
        let dir = self.dir.clone()?;
        let state = self.state.take()?;
        let (seq, data) = self.snapshot.take()?;
        // Back where we left, else on our own work, else the first difficulty.
        let open = self
            .prefer
            .filter(|p| state.documents.iter().any(|d| d.diff == *p))
            .or_else(|| {
                state
                    .documents
                    .iter()
                    .find(|d| d.owner == Some(me) || d.parts.iter().any(|p| p.assignee == Some(me)))
                    .or(state.documents.first())
                    .map(|d| d.diff)
            })
            .unwrap_or(0);
        let doc = data
            .docs
            .iter()
            .find(|d| d.diff == open)
            .or(data.docs.first());
        let Some(doc) = doc else {
            return Some(Err("the session has no difficulty".into()));
        };
        let open = doc.diff;
        // The file of that difficulty, of its kind: a .tko's, not the .osu it was made from.
        let tko = is_tko_text(&doc.text);
        let named: Vec<(PathBuf, String)> = charts_in(&dir)
            .into_iter()
            .filter(|(_, text)| version(text).as_deref() == Some(doc.name.as_str()))
            .collect();
        let existing = named
            .iter()
            .find(|(_, text)| is_tko_text(text) == tko)
            .or(named.first())
            .cloned();
        let path = match existing {
            Some((p, _)) => p,
            None => {
                let ext = if is_tko_text(&doc.text) { "tko" } else { "osu" };
                let name = format!("{} [{}].{ext}", state.title, doc.name);
                let p = dir.join(crate::download::safe_folder_name(&name));
                if let Err(e) = std::fs::write(&p, &doc.text) {
                    return Some(Err(e.to_string()));
                }
                p
            }
        };
        Some(Ok(Ready {
            session,
            epoch,
            me,
            state,
            seq,
            data,
            tail: std::mem::take(&mut self.tail),
            later: std::mem::take(&mut self.later),
            open,
            path,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::chart::tests::MAP;
    use super::*;
    use taiko_proto::mapping::{Document, Mode, Participant, Role};

    fn temp(what: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "taiko-rs-connect-{what}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A set on the host's disk: one difficulty, music over one piece long, a storyboard
    /// picture in a folder and a hidden file.
    fn host_dir() -> (PathBuf, Vec<u8>) {
        let dir = temp("host").join("1 Artist - Song");
        std::fs::create_dir_all(dir.join("sb")).unwrap();
        let map = MAP.replace("Title:Test", "Artist:Artist\r\nTitle:Song");
        std::fs::write(dir.join("Artist - Song (host) [Oni].osu"), map).unwrap();
        let music: Vec<u8> = (0..ASSET_PIECE + 1000).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("audio.mp3"), &music).unwrap();
        std::fs::write(dir.join("sb/star.png"), b"png").unwrap();
        std::fs::write(dir.join(".hidden"), b"x").unwrap();
        (dir, music)
    }

    fn state(set: &SetDescriptor) -> SessionState {
        SessionState {
            session: 9,
            title: set.title.clone(),
            mode: Mode::Guest,
            host: 1,
            participants: vec![Participant {
                id: 1,
                name: "host".into(),
                role: Role::Host,
                colour: 0xF2D45C,
                online: true,
            }],
            documents: vec![
                Document {
                    diff: 0,
                    name: "Oni".into(),
                    owner: None,
                    parts: Vec::new(),
                    locked: false,
                    accepted: false,
                },
                Document {
                    diff: 1,
                    name: "Anna's Inner".into(),
                    owner: Some(2),
                    parts: Vec::new(),
                    locked: false,
                    accepted: false,
                },
            ],
            comments: Vec::new(),
            folder: set.folder.clone(),
            assets: set.assets.clone(),
        }
    }

    fn joined(epoch: u32, me: u32, needs: Vec<String>) -> LinkEvent {
        LinkEvent::Joined {
            session: 9,
            epoch,
            me,
            needs,
            done: 0,
        }
    }

    fn down(d: MapDown) -> LinkEvent {
        LinkEvent::Down(Box::new(d))
    }

    /// What the server answers a joining game's ask for a file from `offset` with: the one
    /// piece of the host's upload that starts there.
    fn answer(ups: &[MapUp], sha: &str, at: u64) -> Option<MapDown> {
        ups.iter().find_map(|u| match u {
            MapUp::Asset {
                sha256,
                offset,
                total,
                data,
            } if sha256 == sha && *offset == at => Some(MapDown::AssetPart {
                sha256: sha256.clone(),
                offset: *offset,
                total: *total,
                data: data.clone(),
            }),
            _ => None,
        })
    }

    #[test]
    fn a_host_sends_what_the_server_needs_and_starts_the_session_with_its_charts() {
        let (dir, music) = host_dir();
        let set = HostSet::read(&dir).unwrap();
        let d = set.descriptor();
        assert_eq!(d.title, "Artist - Song");
        assert_eq!(d.folder, "1 Artist - Song");
        assert_eq!(d.diffs, vec!["Oni".to_string()]);
        let names: Vec<&str> = d.assets.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["audio.mp3", "sb/star.png"],
            "no charts, nothing hidden"
        );
        let sha = sha256_hex(&music);
        let mut c = Connecting::host(set);
        let ups = c.take(joined(1, 1, vec![sha.clone()])).unwrap();
        assert_eq!(
            ups.len(),
            2,
            "the music in two pieces, the picture not needed"
        );
        assert!(matches!(&ups[1], MapUp::Asset { offset, total, .. }
            if *offset == ASSET_PIECE as u64 && *total == music.len() as u64));
        assert!(c.take(down(MapDown::State(state(&d)))).unwrap().is_empty());
        assert!(c.ready().is_none(), "no snapshot yet");
        let snap = c.take(down(MapDown::SnapshotWanted { seq: 0 })).unwrap();
        assert!(matches!(
            snap[..],
            [MapUp::Snapshot {
                seq: 0,
                part: 0,
                parts: 1,
                ..
            }]
        ));
        let r = c.ready().unwrap().unwrap();
        assert_eq!((r.session, r.epoch, r.me, r.open, r.seq), (9, 1, 1, 0, 0));
        assert_eq!(r.path, dir.join("Artist - Song (host) [Oni].osu"));
        assert!(c.ready().is_none(), "once");
    }

    #[test]
    fn a_guest_fetches_the_set_and_opens_its_own_difficulty() {
        let (dir, music) = host_dir();
        let set = HostSet::read(&dir).unwrap();
        let d = set.descriptor();
        let uploads = set.uploads(&[d.assets[0].sha256.clone(), d.assets[1].sha256.clone()]);
        let uploads = uploads.unwrap();
        let mut charts = set.charts.clone();
        charts.push(("Anna's Inner".into(), MAP.replace("Oni", "Anna's Inner")));
        let snapshot = host_snapshot(&charts).unwrap();
        let songs = temp("songs");
        let mut c = Connecting::guest(songs.clone());
        assert_eq!(c.stage(), "connecting");
        assert!(c.take(joined(2, 2, Vec::new())).unwrap().is_empty());
        let first = c.take(down(MapDown::State(state(&d)))).unwrap();
        assert!(matches!(&first[..], [MapUp::GetAsset { offset: 0, .. }]));
        assert_eq!(c.to_fetch, music.len() as u64 + 3);
        assert!(
            c.stage().starts_with("downloading the set's files 0.0/"),
            "{}",
            c.stage()
        );
        // The server answers each ask with one piece, as the real one does.
        let mut asks = first;
        let mut answered = 0;
        while let Some(ask) = asks.pop() {
            let MapUp::GetAsset { sha256, offset } = ask else {
                panic!("only asks for files while joining: {ask:?}")
            };
            let piece = answer(&uploads, &sha256, offset).expect("a piece the host sent");
            answered += 1;
            asks.extend(c.take(down(piece)).unwrap());
        }
        assert_eq!(
            answered, 3,
            "the music piece by piece (two), then the picture"
        );
        assert_eq!(c.fetched, c.to_fetch);
        assert_eq!(c.stage(), "getting the map", "every file in");
        for up in snapshot_lines(5, &snapshot) {
            let MapUp::Snapshot {
                seq,
                part,
                parts,
                data,
            } = up
            else {
                unreachable!()
            };
            c.take(down(MapDown::Snapshot {
                seq,
                part,
                parts,
                data,
            }))
            .unwrap();
        }
        c.take(down(MapDown::Chat {
            who: 1,
            text: "welcome".into(),
            at: 0,
        }))
        .unwrap();
        assert!(c.progress() >= 0.99);
        assert_eq!(c.stage(), "opening");
        let r = c.ready().unwrap().unwrap();
        let folder = songs.join("1 Artist - Song");
        assert_eq!(std::fs::read(folder.join("audio.mp3")).unwrap(), music);
        assert_eq!(std::fs::read(folder.join("sb/star.png")).unwrap(), b"png");
        assert_eq!(r.open, 1, "the guest difficulty given to us");
        assert_eq!(r.path, folder.join("Artist - Song [Anna's Inner].osu"));
        assert_eq!(
            version(&std::fs::read_to_string(&r.path).unwrap()).as_deref(),
            Some("Anna's Inner")
        );
        assert_eq!(r.later.len(), 1, "the chat line waits for the session");

        // Joining again with the set on disk fetches nothing.
        let mut again = Connecting::guest(songs);
        again.take(joined(3, 2, Vec::new())).unwrap();
        assert!(
            again
                .take(down(MapDown::State(state(&d))))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn going_back_into_a_session_uses_the_set_where_it_was_left() {
        let (dir, _) = host_dir();
        let set = HostSet::read(&dir).unwrap();
        let d = set.descriptor();
        let snapshot = host_snapshot(&set.charts).unwrap();
        let mut c = Connecting::rejoin(dir.clone(), 0);
        c.take(joined(4, 1, Vec::new())).unwrap();
        assert!(
            c.take(down(MapDown::State(state(&d)))).unwrap().is_empty(),
            "every file is here already"
        );
        for up in snapshot_lines(7, &snapshot) {
            let MapUp::Snapshot {
                seq,
                part,
                parts,
                data,
            } = up
            else {
                unreachable!()
            };
            c.take(down(MapDown::Snapshot {
                seq,
                part,
                parts,
                data,
            }))
            .unwrap();
        }
        let r = c.ready().unwrap().unwrap();
        assert_eq!(r.path.parent(), Some(dir.as_path()), "not a copy in Songs");
        assert_eq!(r.seq, 7);
    }

    /// Back into a session, the editor opens on the difficulty we were on when we left, not
    /// the first one with work of ours (a guest who had gone to another difficulty landed where
    /// they couldn't map, as if their rights were gone).
    #[test]
    fn going_back_opens_the_difficulty_we_left_and_a_tko_its_own_file() {
        let (dir, _) = host_dir();
        let inner = MAP
            .replace("Title:Test", "Artist:Artist\r\nTitle:Song")
            .replace("Version:Oni", "Version:Anna's Inner");
        std::fs::write(dir.join("Artist - Song (host) [Anna's Inner].osu"), &inner).unwrap();
        // Anna (2) owns Anna's Inner; the documents in the set's order.
        let open = |c: &mut Connecting| {
            let set = HostSet::read(&dir).unwrap();
            let mut s = state(&set.descriptor());
            s.documents = set
                .charts
                .iter()
                .enumerate()
                .map(|(i, (name, _))| Document {
                    diff: i as u32,
                    name: name.clone(),
                    owner: (name == "Anna's Inner").then_some(2),
                    parts: Vec::new(),
                    locked: false,
                    accepted: false,
                })
                .collect();
            let snapshot = host_snapshot(&set.charts).unwrap();
            c.take(joined(4, 2, Vec::new())).unwrap();
            c.take(down(MapDown::State(s))).unwrap();
            for up in snapshot_lines(7, &snapshot) {
                let MapUp::Snapshot {
                    seq,
                    part,
                    parts,
                    data,
                } = up
                else {
                    unreachable!()
                };
                c.take(down(MapDown::Snapshot {
                    seq,
                    part,
                    parts,
                    data,
                }))
                .unwrap();
            }
            let r = c.ready().unwrap().unwrap();
            let name = set.charts[r.open as usize].0.clone();
            (name, r.path)
        };
        let diff = |name: &str| {
            let set = HostSet::read(&dir).unwrap();
            set.charts.iter().position(|(n, _)| n == name).unwrap() as u32
        };
        let (name, path) = open(&mut Connecting::rejoin(dir.clone(), diff("Oni")));
        assert_eq!(name, "Oni", "where we were, not our own difficulty");
        assert!(path.to_string_lossy().ends_with("[Oni].osu"));
        // A difficulty the session no longer has: our own work, as on a first join.
        let (name, _) = open(&mut Connecting::rejoin(dir.clone(), 99));
        assert_eq!(name, "Anna's Inner");
        // Beside the .osu, the session's .tko of that difficulty is the file it opens.
        let tko = crate::tko::format::write(&inner, &Default::default()).unwrap();
        std::fs::write(dir.join("Artist - Song (host) [Anna's Inner].tko"), &tko).unwrap();
        let (name, path) = open(&mut Connecting::rejoin(dir.clone(), diff("Anna's Inner")));
        assert_eq!(name, "Anna's Inner");
        assert!(
            path.to_string_lossy().ends_with("[Anna's Inner].tko"),
            "{}",
            path.display()
        );
    }

    #[test]
    fn a_file_named_outside_the_set_or_arriving_broken_ends_the_join() {
        let (dir, _) = host_dir();
        let d = HostSet::read(&dir).unwrap().descriptor();
        let mut s = state(&d);
        s.assets[0].name = "../../evil.dll".into();
        let mut c = Connecting::guest(temp("evil"));
        assert!(c.take(down(MapDown::State(s))).is_err());

        let mut c = Connecting::guest(temp("broken"));
        let s = state(&d);
        let sha = s.assets[0].sha256.clone();
        c.take(down(MapDown::State(s))).unwrap();
        let bad = MapDown::AssetPart {
            sha256: sha,
            offset: 0,
            total: 3,
            data: b64().encode(b"abc"),
        };
        assert!(c.take(down(bad)).unwrap_err().contains("broken"));
        assert!(
            Connecting::guest(temp("lost"))
                .take(LinkEvent::Lost("no route".into()))
                .is_err()
        );
    }

    #[test]
    fn a_piece_that_doesnt_come_is_asked_for_again() {
        let (dir, _) = host_dir();
        let d = HostSet::read(&dir).unwrap().descriptor();
        let mut c = Connecting::guest(temp("again"));
        c.take(joined(2, 2, Vec::new())).unwrap();
        let first = c.take(down(MapDown::State(state(&d)))).unwrap();
        let [MapUp::GetAsset { sha256, offset: 0 }] = &first[..] else {
            panic!("{first:?}")
        };
        // The host hasn't sent the file yet, so the server says nothing.
        let t0 = Instant::now();
        assert!(c.tick(t0).is_empty());
        assert!(c.tick(t0 + Duration::from_secs(2)).is_empty());
        let again = c.tick(t0 + Duration::from_secs(3));
        assert!(
            matches!(&again[..], [MapUp::GetAsset { sha256: s, offset: 0 }] if s == sha256),
            "{again:?}"
        );
        assert!(
            c.stage().starts_with("waiting for the host"),
            "{}",
            c.stage()
        );
        assert!(
            c.tick(t0 + Duration::from_secs(4)).is_empty(),
            "not every frame"
        );
        assert_eq!(c.tick(t0 + Duration::from_secs(6)).len(), 1);
    }

    /// A set made as a `.tko` from the start: no `.osu` in it at all.
    fn tko_dir() -> PathBuf {
        let dir = temp("tko").join("2 Artist - Song");
        std::fs::create_dir_all(&dir).unwrap();
        let map = MAP.replace("Title:Test", "Artist:Artist\r\nTitle:Song");
        let tko = crate::tko::format::write(&map, &Default::default()).unwrap();
        std::fs::write(dir.join("Artist - Song (host) [Oni].tko"), tko).unwrap();
        std::fs::write(dir.join("audio.mp3"), b"mp3").unwrap();
        dir
    }

    #[test]
    fn a_tko_only_set_opens_a_session_and_reaches_the_guest_as_a_tko() {
        let dir = tko_dir();
        let set = HostSet::read(&dir).expect("a .tko is a difficulty");
        let d = set.descriptor();
        assert_eq!(d.diffs, vec!["Oni".to_string()]);
        assert_eq!(d.assets.len(), 1, "the chart is not sent as a file");
        let snapshot = host_snapshot(&set.charts).unwrap();
        let mut host = Connecting::host(set);
        host.take(joined(1, 1, Vec::new())).unwrap();
        let mut s = state(&d);
        s.documents.truncate(1);
        host.take(down(MapDown::State(s.clone()))).unwrap();
        host.take(down(MapDown::SnapshotWanted { seq: 0 })).unwrap();
        let r = host.ready().unwrap().unwrap();
        assert_eq!(r.path, dir.join("Artist - Song (host) [Oni].tko"));

        let songs = temp("tko-songs");
        std::fs::create_dir_all(songs.join("2 Artist - Song")).unwrap();
        std::fs::write(songs.join("2 Artist - Song/audio.mp3"), b"mp3").unwrap();
        let mut guest = Connecting::guest(songs.clone());
        guest.take(joined(2, 2, Vec::new())).unwrap();
        assert!(guest.take(down(MapDown::State(s))).unwrap().is_empty());
        for up in snapshot_lines(1, &snapshot) {
            let MapUp::Snapshot {
                seq,
                part,
                parts,
                data,
            } = up
            else {
                unreachable!()
            };
            guest
                .take(down(MapDown::Snapshot {
                    seq,
                    part,
                    parts,
                    data,
                }))
                .unwrap();
        }
        let r = guest.ready().unwrap().unwrap();
        assert_eq!(r.path.extension().unwrap(), "tko", "{}", r.path.display());
        let text = std::fs::read_to_string(&r.path).unwrap();
        assert!(is_tko_text(&text));
    }

    #[test]
    fn a_tko_made_from_an_osu_stands_for_that_difficulty() {
        let dir = tko_dir();
        let map = MAP.replace("Title:Test", "Artist:Artist\r\nTitle:Song");
        std::fs::write(dir.join("Artist - Song (host) [Oni].osu"), &map).unwrap();
        std::fs::write(
            dir.join("Artist - Song (host) [Kantan].osu"),
            map.replace("Version:Oni", "Version:Kantan"),
        )
        .unwrap();
        let charts = charts_in(&dir);
        let names: Vec<String> = charts
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "Artist - Song (host) [Kantan].osu".to_string(),
                "Artist - Song (host) [Oni].tko".to_string()
            ]
        );
    }
}
