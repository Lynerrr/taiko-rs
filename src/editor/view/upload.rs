//! Sending the map to taiko.rs: File > Upload opens a checklist; Send packs the map's folder
//! and posts it on a thread. The set's number is kept beside the map, so the next upload updates
//! the same set.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use super::chrome::{DIM, WARN, wrap};
use super::{Btn, Modal, View};
use crate::custom;
use crate::custom::approved::{self, Approved, Kept};
use crate::hit::Canvas;
use crate::render::rgba;

/// A button of the upload window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum UpAct {
    Send,
    /// Send as a new set, though the folder was sent before.
    SendNew,
    Copy,
    Close,
    /// Keep the changes to approved difficulties as new ones and put the approved back.
    KeepAsNew,
    /// Open the game's Discord, where admins can take an approval back.
    AskAdmin,
}

/// The approved difficulties' yellow, and a failure's red.
const GOLD: u32 = 0xffd34d;
const FAIL: u32 = 0xff6b6b;
const OK: u32 = 0x6fdc8c;

type Fixing = Result<Vec<(PathBuf, PathBuf)>, String>;
/// The website's answer about which difficulties of a set are approved.
type Answer = (u32, Result<Vec<Approved>, String>);

/// One line of the checklist.
#[derive(Clone, PartialEq, Debug)]
pub struct Check {
    pub ok: bool,
    /// Sending waits until it is fine.
    pub blocking: bool,
    pub text: String,
}

type Sending = Result<(custom::Sent, Vec<String>, Vec<String>), String>;

#[derive(Default)]
pub struct UploadUi {
    pub checks: Vec<Check>,
    /// The set this folder was sent as before.
    pub id: Option<u32>,
    rx: Option<Receiver<Sending>>,
    /// How it ended: the set's page, or what went wrong.
    pub done: Option<Result<String, String>>,
    /// Difficulties that stayed behind, from the last send.
    pub left_out: Vec<String>,
    /// The difficulties that went.
    pub sent_diffs: Vec<String>,
    pub version: u32,
    /// The approved difficulties of the set this folder was sent as, and how the folder has
    /// them: an update must bring them back as they were.
    pub approved: Vec<(Approved, Kept)>,
    /// What the website said last about which are approved, for the set with this number.
    remote: Option<(u32, Vec<Approved>)>,
    asking: Option<Receiver<Answer>>,
    /// Why the website couldn't be asked.
    pub ask_error: Option<String>,
    fixing: Option<Receiver<Fixing>>,
    /// How putting the approved difficulties back went.
    pub fixed: Option<Result<String, String>>,
}

impl UploadUi {
    pub fn can_send(&self) -> bool {
        self.rx.is_none()
            && self.fixing.is_none()
            && !self.approved_changed()
            && self.done.as_ref().is_none_or(Result::is_err)
            && !self.checks.iter().any(|c| c.blocking && !c.ok)
    }

    /// An approved difficulty is changed or gone: the server would refuse the update.
    pub fn approved_changed(&self) -> bool {
        self.approved.iter().any(|(_, kept)| *kept != Kept::Same)
    }

    /// Compares the folder with what the website said, if it said it about set `id`.
    fn compare(&mut self, folder: &Path, id: Option<u32>) {
        self.approved = match (&self.remote, id) {
            (Some((asked, list)), Some(id)) if *asked == id => approved::compare(folder, list),
            _ => Vec::new(),
        };
    }

    /// The website's answer about the approved difficulties, once it came; true when it did.
    fn take_answer(&mut self) -> bool {
        let Some(rx) = &self.asking else { return false };
        let got = match rx.try_recv() {
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => None,
            Ok(got) => Some(got),
        };
        self.asking = None;
        match got {
            Some((id, Ok(list))) => {
                self.remote = Some((id, list));
                self.ask_error = None;
            }
            Some((_, Err(e))) => self.ask_error = Some(e),
            None => self.ask_error = Some("the question stopped unexpectedly".into()),
        }
        true
    }

    pub fn sending(&self) -> bool {
        self.rx.is_some()
    }
}

fn check(ok: bool, blocking: bool, text: impl Into<String>) -> Check {
    Check {
        ok,
        blocking,
        text: text.into(),
    }
}

/// What has to be right before the map at `path` (saved, if `saved`) can be sent.
pub fn checks(path: &Path, saved: bool, logged_in: bool) -> Vec<Check> {
    let mut out = vec![
        check(
            logged_in,
            true,
            if logged_in {
                "Logged in to taiko.rs"
            } else {
                "Log in to your taiko.rs account first (the main menu, Multiplayer)"
            },
        ),
        check(
            saved,
            true,
            if saved {
                "Saved"
            } else {
                "Save the map first (Ctrl+S): what is sent is the saved file"
            },
        ),
    ];
    let Some(meta) = crate::beatmap::read_meta_file(path) else {
        out.push(check(false, true, "Can't read the saved map"));
        return out;
    };
    let folder = path.parent().unwrap_or(Path::new("."));
    let (mode, notes) = std::fs::read_to_string(path)
        .map(|t| custom::mode_and_notes(&t))
        .unwrap_or((None, 0));
    out.push(check(
        mode == Some(1),
        true,
        if mode == Some(1) {
            "A taiko map"
        } else {
            "Only taiko maps (Mode: 1) can be sent"
        },
    ));
    let enough = notes >= custom::MIN_NOTES;
    out.push(check(
        enough,
        true,
        if enough {
            format!("{notes} notes")
        } else {
            format!(
                "Needs at least {} notes (it has {notes})",
                custom::MIN_NOTES
            )
        },
    ));
    for (what, value) in [
        ("Artist", &meta.artist),
        ("Title", &meta.title),
        ("Mapper name", &meta.creator),
    ] {
        let ok = !value.trim().is_empty();
        out.push(check(
            ok,
            true,
            if ok {
                format!("{what}: {value}")
            } else {
                format!("{what} is empty: fill it in File > Song setup")
            },
        ));
    }
    let audio = folder.join(&meta.audio_file);
    let music = !meta.audio_file.is_empty() && audio.is_file();
    out.push(check(
        music,
        true,
        if music {
            format!("Music: {}", meta.audio_file)
        } else {
            "The music file isn't in the map's folder".to_string()
        },
    ));
    let own = meta.set_id.is_none();
    out.push(check(
        own,
        true,
        if own {
            "Your own map (no osu! set number)".to_string()
        } else {
            "The map names an osu! set (BeatmapSetID): only your own maps can be sent".to_string()
        },
    ));
    if let Some(g) = std::fs::read_to_string(path)
        .ok()
        .filter(|t| crate::tko::format::version_of(t).is_some())
        .and_then(|t| crate::tko::format::read(&t).ok())
        .map(|p| p.gimmick)
    {
        out.extend(tko_checks(&g));
    }
    let background = meta
        .background
        .as_ref()
        .is_some_and(|b| folder.join(b).is_file());
    out.push(check(
        background,
        false,
        if background {
            "Background picture"
        } else {
            "No background picture: the page will show a plain colour (not required)"
        },
    ));
    out
}

/// What a player should know about a `.tko` map's tricks before it goes out: nothing blocks.
fn tko_checks(g: &crate::tko::gimmick::Gimmick) -> Vec<Check> {
    use crate::tko::gimmick::ShowColour;
    let mut out = Vec::new();
    let lying = g.groups.iter().filter(|gr| !gr.show.is_true());
    let lies: usize = lying.clone().map(|gr| gr.notes.len()).sum();
    if lies > 0 {
        out.push(check(
            false,
            false,
            format!(
                "{lies} notes show as another colour or size: players can turn this off (Settings, Gameplay, Show true notes)"
            ),
        ));
    }
    let blind = lying
        .filter(|gr| gr.show.colour != ShowColour::Same && gr.show.reveal == 0.0)
        .map(|gr| gr.notes.len())
        .sum::<usize>();
    if blind > 0 {
        out.push(check(
            false,
            false,
            format!(
                "{blind} of them show their true colour only at the hit: they can't be read, only learned"
            ),
        ));
    }
    if !g.fakes.is_empty() {
        out.push(check(
            true,
            false,
            format!("{} fake notes (drawn, never hit)", g.fakes.len()),
        ));
    }
    out
}

impl View {
    /// File > Upload.
    pub(super) fn open_upload(&mut self) {
        let folder = self.ed.path.parent().map(Path::to_path_buf);
        self.upload.checks = checks(&self.ed.path, !self.ed.modified(), self.account.is_some());
        self.upload.id = folder.as_deref().and_then(custom::sidecar_id);
        if !self.upload.sending() && self.upload.done.as_ref().is_some_and(Result::is_err) {
            self.upload.done = None;
        }
        if let Some(folder) = &folder {
            self.upload.compare(folder, self.upload.id);
        }
        // Whether an approval was given or taken back meanwhile: asked each time it opens.
        if let Some(id) = self.upload.id
            && self.upload.asking.is_none()
        {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send((id, approved::fetch(id)));
            });
            self.upload.asking = Some(rx);
        }
        self.modal = Some(Modal::Upload);
    }

    /// Keeps the changed approved difficulties as new ones, and puts the approved files back
    /// from the website's copy of the set.
    fn keep_as_new(&mut self) {
        let (Some(id), Some(folder)) = (self.upload.id, self.ed.path.parent()) else {
            return;
        };
        if self.upload.fixing.is_some() || !self.upload.approved_changed() {
            return;
        }
        let (folder, list) = (folder.to_path_buf(), self.upload.approved.clone());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let fixed = approved::download(id).and_then(|osz| {
                approved::restore(&folder, &list, &osz, |path| {
                    crate::editor::new_difficulty(path, true)
                })
            });
            let _ = tx.send(fixed);
        });
        self.upload.fixing = Some(rx);
        self.upload.fixed = None;
        self.status = "putting the approved difficulties back...".into();
    }

    /// The upload window, opened on a difficulty that was just made from changes to an approved
    /// one, with what happened.
    pub fn show_upload_after_keep(&mut self, note: String) {
        self.open_upload();
        self.upload.fixed = Some(Ok(note));
    }

    /// Takes the answers of the website and of putting the approved difficulties back.
    fn poll_approved(&mut self) {
        let folder = self.ed.path.parent().map(Path::to_path_buf);
        if self.upload.take_answer()
            && let Some(folder) = &folder
        {
            self.upload.compare(folder, self.upload.id);
        }
        let Some(rx) = &self.upload.fixing else {
            return;
        };
        let got = match rx.try_recv() {
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("it stopped unexpectedly".to_string()),
            Ok(got) => got,
        };
        self.upload.fixing = None;
        match got {
            Ok(moved) => {
                let name = |p: &Path| {
                    crate::beatmap::read_meta_file(p)
                        .map_or_else(|| p.display().to_string(), |m| m.version)
                };
                let news: Vec<String> = moved.iter().map(|(_, new)| name(new)).collect();
                self.upload.fixed = Some(Ok(if news.is_empty() {
                    "The approved difficulties are back as they were approved.".to_string()
                } else {
                    format!(
                        "Your changes are now the new difficulty {}; the approved ones are back as they were. Send it to add it to the set.",
                        news.join(", ")
                    )
                }));
                // The open file went back to what was approved: its changes are in the copy.
                if let Some((_, new)) = moved.iter().find(|(old, _)| *old == self.ed.path) {
                    let note = self.upload.fixed.clone().and_then(Result::ok);
                    self.want_open = Some((new.clone(), note));
                }
                self.status = "the approved difficulties are back".into();
            }
            Err(e) => {
                self.status = "couldn't put the approved difficulties back".into();
                self.upload.fixed = Some(Err(e));
            }
        }
        if let Some(folder) = &folder {
            self.upload.compare(folder, self.upload.id);
        }
    }

    pub(super) fn press_upload(&mut self, act: UpAct) {
        match act {
            UpAct::Close => self.modal = None,
            UpAct::KeepAsNew => self.keep_as_new(),
            UpAct::AskAdmin => {
                if !crate::services::system::open_url(crate::menu::DISCORD_INVITE) {
                    self.status = format!("open {}", crate::menu::DISCORD_INVITE);
                }
            }
            UpAct::Copy => {
                if let Some(Ok(url)) = &self.upload.done {
                    self.status = if crate::clipboard::set(url) {
                        "link copied".into()
                    } else {
                        "couldn't copy the link".into()
                    };
                }
            }
            UpAct::Send | UpAct::SendNew => {
                let was = self.ed.path.clone();
                let folder: PathBuf = was.parent().unwrap_or(Path::new(".")).to_path_buf();
                self.open_upload();
                if !self.upload.can_send() {
                    return;
                }
                let Some(token) = self.account.clone() else {
                    return;
                };
                let id = self.upload.id.filter(|_| act == UpAct::Send);
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let sent = custom::pack(&folder).and_then(|packed| {
                        custom::upload(&token, id, &packed.bytes).map(|s| {
                            custom::write_sidecar(&folder, s.id);
                            (s, packed.sent, packed.left_out)
                        })
                    });
                    let _ = tx.send(sent);
                });
                self.upload.rx = Some(rx);
                self.upload.done = None;
                self.status = "sending the map...".into();
            }
        }
    }

    /// Takes the answer of a send in progress.
    pub(super) fn poll_upload(&mut self) {
        self.poll_approved();
        let Some(rx) = &self.upload.rx else { return };
        let got = match rx.try_recv() {
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the sending stopped unexpectedly".to_string()),
            Ok(sent) => sent,
        };
        self.upload.rx = None;
        match got {
            Ok((sent, diffs, left_out)) => {
                self.upload.id = Some(sent.id);
                self.upload.version = sent.version;
                self.upload.left_out = left_out;
                self.upload.sent_diffs = diffs;
                self.status = format!("sent: set {}", sent.id);
                self.upload.done = Some(Ok(custom::page_url(sent.id)));
            }
            Err(e) => {
                self.status = "couldn't send the map".into();
                self.upload.done = Some(Err(e));
            }
        }
    }

    /// The approved difficulties' part of the window, from `y`; `draw` false only measures.
    /// Returns its height.
    fn draw_approved(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, draw: bool) -> f32 {
        if self.upload.id.is_none() {
            return 0.0;
        }
        let u = &self.upload;
        let mut h = 0.0;
        if !u.approved_changed() {
            let line = if !u.approved.is_empty() {
                let names: Vec<&str> = u.approved.iter().map(|(a, _)| a.version.as_str()).collect();
                Some((
                    "OK",
                    GOLD,
                    format!("Approved, kept as they were: {}", names.join(", ")),
                ))
            } else if u.asking.is_some() && u.remote.is_none() {
                Some((
                    "..",
                    DIM,
                    "Asking the website about approved difficulties".into(),
                ))
            } else {
                u.ask_error.as_ref().map(|e| {
                    (
                        "--",
                        DIM,
                        format!("Couldn't ask about approved difficulties: {e}"),
                    )
                })
            };
            if let Some((mark, col, text)) = line {
                if draw {
                    c.text(x, y, 7.0, rgba(col, 1.0), mark);
                    c.text(
                        x + 22.0,
                        y,
                        7.0,
                        rgba(col, 1.0),
                        &Canvas::fit(7.0, w - 22.0, &text),
                    );
                }
                h += 14.0;
            }
        } else {
            let pad = 8.0;
            let about = wrap(
                6.5,
                w - pad * 2.0,
                "Plays on an approved difficulty give pp, so an update has to bring it back exactly as it was approved. Keep your changes as a new difficulty, or ask an admin to take the approval back first.",
            );
            let box_h = pad
                + 13.0
                + about.len() as f32 * 9.5
                + 5.0
                + u.approved.len() as f32 * 12.0
                + 6.0
                + 20.0
                + pad;
            if draw {
                c.round_rect(
                    [x - 4.0, y],
                    [w + 8.0, box_h],
                    3.0,
                    rgba(GOLD, 0.07),
                    rgba(GOLD, 0.85),
                    1.0,
                );
                let (bx, mut by) = (x - 4.0 + pad, y + pad);
                c.text(
                    bx,
                    by,
                    7.5,
                    rgba(GOLD, 1.0),
                    "Approved difficulties can't change",
                );
                by += 13.0;
                for line in &about {
                    c.text(bx, by, 6.5, rgba(0xd8d8e6, 1.0), line);
                    by += 9.5;
                }
                by += 5.0;
                let rows: Vec<(&str, u32, String)> = u
                    .approved
                    .iter()
                    .map(|(a, kept)| match kept {
                        Kept::Same => ("OK", OK, format!("{}: as approved", a.version)),
                        Kept::Changed(_) => (
                            "!!",
                            WARN,
                            format!("{}: changed since it was approved", a.version),
                        ),
                        Kept::Missing => ("!!", WARN, format!("{}: not in the folder", a.version)),
                    })
                    .collect();
                for (mark, col, text) in rows {
                    c.text(bx, by, 7.0, rgba(col, 1.0), mark);
                    c.text(
                        bx + 22.0,
                        by,
                        7.0,
                        rgba(col, 1.0),
                        &Canvas::fit(7.0, w - pad * 2.0 - 22.0, &text),
                    );
                    by += 12.0;
                }
                by += 6.0;
                if self.upload.fixing.is_some() {
                    c.text(
                        bx,
                        by + 6.0,
                        7.0,
                        rgba(0xffffff, 1.0),
                        "Putting the approved difficulties back...",
                    );
                } else {
                    let mut bx = bx;
                    for (label, act, on) in [
                        (
                            "Keep my changes as a new difficulty",
                            UpAct::KeepAsNew,
                            true,
                        ),
                        ("Ask an admin on Discord", UpAct::AskAdmin, false),
                    ] {
                        let bw = Canvas::text_width(7.0, label) + 14.0;
                        self.button(c, [bx, by, bw, 20.0], label, Btn::U(act), on);
                        bx += bw + 6.0;
                    }
                }
            }
            h += box_h + 6.0;
        }
        if let Some(fixed) = &self.upload.fixed {
            let (col, text) = match fixed {
                Ok(t) => (OK, t),
                Err(e) => (FAIL, e),
            };
            let lines = wrap(6.5, w, text);
            if draw {
                let mut ly = y + h;
                for line in &lines {
                    c.text(x, ly, 6.5, rgba(col, 1.0), line);
                    ly += 9.5;
                }
            }
            h += lines.len() as f32 * 9.5 + 6.0;
        }
        h
    }

    /// How the send ended, from `y`; `draw` false only measures. Returns its height.
    fn draw_result(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, draw: bool) -> f32 {
        match &self.upload.done {
            Some(Ok(url)) => {
                if draw {
                    c.text(
                        x,
                        y,
                        7.5,
                        rgba(OK, 1.0),
                        &format!(
                            "Sent (version {}). Anyone can play it now.",
                            self.upload.version.max(1)
                        ),
                    );
                    c.text(
                        x,
                        y + 14.0,
                        7.0,
                        rgba(0xffffff, 1.0),
                        &Canvas::fit(7.0, w, url),
                    );
                    let sent = format!("Sent: {}", self.upload.sent_diffs.join(", "));
                    c.text(
                        x,
                        y + 28.0,
                        6.5,
                        rgba(DIM, 1.0),
                        &Canvas::fit(6.5, w, &sent),
                    );
                    if !self.upload.left_out.is_empty() {
                        let left = format!("Left out: {}", self.upload.left_out.join(", "));
                        c.text(
                            x,
                            y + 40.0,
                            6.5,
                            rgba(DIM, 1.0),
                            &Canvas::fit(6.5, w, &left),
                        );
                    }
                }
                64.0
            }
            Some(Err(e)) => {
                let pad = 8.0;
                let mut lines = wrap(7.0, w - pad * 2.0, e);
                lines.truncate(8);
                let box_h = pad + 13.0 + lines.len() as f32 * 10.0 + pad - 2.0;
                if draw {
                    c.round_rect(
                        [x - 4.0, y],
                        [w + 8.0, box_h],
                        3.0,
                        rgba(FAIL, 0.08),
                        rgba(FAIL, 0.85),
                        1.0,
                    );
                    let (bx, mut by) = (x - 4.0 + pad, y + pad);
                    c.text(bx, by, 7.5, rgba(FAIL, 1.0), "The server didn't take it");
                    by += 13.0;
                    for line in &lines {
                        c.text(bx, by, 7.0, rgba(0xf2dede, 1.0), line);
                        by += 10.0;
                    }
                }
                box_h + 6.0
            }
            None if self.upload.sending() => {
                if draw {
                    c.text(x, y, 7.5, rgba(0xffffff, 1.0), "Sending...");
                }
                14.0
            }
            None => 0.0,
        }
    }

    pub(super) fn draw_upload(&mut self, c: &mut Canvas) {
        c.rect(0.0, 0.0, c.width, 480.0, rgba(0x000000, 0.62));
        let w = 470.0_f32.min(c.width - 20.0);
        let inner = w - 28.0;
        let checks = self.upload.checks.len() as f32 * 14.0;
        let approved = self.draw_approved(c, 0.0, 0.0, inner, false);
        let result = self.draw_result(c, 0.0, 0.0, inner, false);
        let h = 48.0 + checks + 6.0 + approved + result + 36.0;
        let (x, y) = ((c.width - w) / 2.0, ((480.0 - h) / 2.0 - 10.0).max(4.0));
        self.overlay_rect = [x, y, w, h];
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x1b1b26, 1.0),
            rgba(0x6a6a88, 1.0),
            1.0,
        );
        c.text(
            x + 14.0,
            y + 12.0,
            9.0,
            rgba(0xffffff, 1.0),
            "Upload to taiko.rs",
        );
        let about = match self.upload.id {
            Some(id) => format!(
                "Sent before as set {id}: this updates it. Approved difficulties stay as they are."
            ),
            None => "Public as CUSTOM at once; gives pp once it is approved.".to_string(),
        };
        c.text(
            x + 14.0,
            y + 30.0,
            6.5,
            rgba(DIM, 1.0),
            &Canvas::fit(6.5, inner, &about),
        );
        let mut ty = y + 48.0;
        for k in &self.upload.checks {
            let (mark, col) = match (k.ok, k.blocking) {
                (true, _) => ("OK", OK),
                (false, true) => ("!!", WARN),
                (false, false) => ("--", DIM),
            };
            c.text(x + 14.0, ty, 7.0, rgba(col, 1.0), mark);
            c.text(
                x + 36.0,
                ty,
                7.0,
                rgba(if k.ok { 0xd0d0e0 } else { col }, 1.0),
                &Canvas::fit(7.0, w - 50.0, &k.text),
            );
            ty += 14.0;
        }
        ty += 6.0;
        ty += self.draw_approved(c, x + 14.0, ty, inner, true);
        self.draw_result(c, x + 14.0, ty, inner, true);
        let by = y + h - 28.0;
        let mut bx = x + 14.0;
        let sent = matches!(self.upload.done, Some(Ok(_)));
        let can = self.upload.can_send();
        if sent {
            let label = "Copy link";
            let bw = Canvas::text_width(7.0, label) + 14.0;
            self.button(c, [bx, by, bw, 20.0], label, Btn::U(UpAct::Copy), true);
            bx += bw + 6.0;
        } else if can {
            let label = if self.upload.id.is_some() {
                "Update the set"
            } else {
                "Send"
            };
            let bw = Canvas::text_width(7.0, label) + 14.0;
            self.button(c, [bx, by, bw, 20.0], label, Btn::U(UpAct::Send), true);
            bx += bw + 6.0;
            if self.upload.id.is_some() {
                let label = "Send as a new set";
                let bw = Canvas::text_width(7.0, label) + 14.0;
                self.button(c, [bx, by, bw, 20.0], label, Btn::U(UpAct::SendNew), false);
                bx += bw + 6.0;
            }
        }
        let label = if sent { "Done" } else { "Close" };
        let bw = Canvas::text_width(7.0, label) + 14.0;
        self.button(c, [bx, by, bw, 20.0], label, Btn::U(UpAct::Close), false);
    }
}

// Only the regression suite (Linux only) draws it.
#[cfg(all(test, target_os = "linux"))]
impl View {
    /// The upload window as it is once the website answered about set `id`, after a send
    /// that ended as `done`.
    pub(crate) fn upload_window_for_tests(
        &mut self,
        id: u32,
        approved: Vec<(Approved, Kept)>,
        done: Option<Result<String, String>>,
    ) {
        self.upload.checks = checks(&self.ed.path, true, true);
        self.upload.id = Some(id);
        self.upload.remote = Some((id, approved.iter().map(|(a, _)| a.clone()).collect()));
        self.upload.approved = approved;
        self.upload.done = done;
        self.modal = Some(Modal::Upload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("taiko-upload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn map(notes: usize, set: Option<u32>) -> String {
        let mut text = format!(
            "osu file format v14\r\n\r\n[General]\r\nAudioFilename: audio.mp3\r\nMode: 1\r\n\r\n[Metadata]\r\nTitle:Song\r\nArtist:Band\r\nCreator:me\r\nVersion:Oni\r\n{}\r\n[Events]\r\n0,0,\"bg.png\",0,0\r\n\r\n[HitObjects]\r\n",
            set.map(|s| format!("BeatmapSetID:{s}\r\n"))
                .unwrap_or_default()
        );
        for k in 0..notes {
            text += &format!("256,192,{},1,0\r\n", 1000 + k * 200);
        }
        text
    }

    fn problems(list: &[Check]) -> Vec<&str> {
        list.iter()
            .filter(|c| c.blocking && !c.ok)
            .map(|c| c.text.as_str())
            .collect()
    }

    #[test]
    fn a_good_map_has_nothing_in_the_way() {
        let dir = folder("good");
        let path = dir.join("a [Oni].osu");
        std::fs::write(&path, map(20, None)).unwrap();
        std::fs::write(dir.join("audio.mp3"), [1u8; 8]).unwrap();
        std::fs::write(dir.join("bg.png"), [1u8; 8]).unwrap();
        let list = checks(&path, true, true);
        assert!(problems(&list).is_empty(), "{:?}", problems(&list));
        assert!(list.iter().all(|c| c.ok));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_missing_is_said_and_blocks_sending() {
        let dir = folder("bad");
        let path = dir.join("a [Oni].osu");
        std::fs::write(&path, map(3, Some(123))).unwrap();
        let list = checks(&path, false, false);
        let bad = problems(&list);
        assert!(bad.iter().any(|t| t.contains("Log in")));
        assert!(bad.iter().any(|t| t.contains("Save the map")));
        assert!(bad.iter().any(|t| t.contains("at least 10 notes")));
        assert!(bad.iter().any(|t| t.contains("music")));
        assert!(bad.iter().any(|t| t.contains("osu! set")));
        let ui = UploadUi {
            checks: list,
            ..Default::default()
        };
        assert!(!ui.can_send());
        // No background is only a remark.
        let missing = checks(&path, true, true);
        let bg = missing
            .iter()
            .find(|c| c.text.contains("background"))
            .unwrap();
        assert!(!bg.ok && !bg.blocking);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_map_that_lies_says_so_without_blocking() {
        use crate::tko::gimmick::{FakeNote, Gimmick, Group, ShowAs, ShowColour};
        let mut g = Gimmick::default();
        g.groups.push(Group {
            notes: vec![1000.0, 1200.0],
            show: ShowAs {
                colour: ShowColour::Swap,
                ..ShowAs::default()
            },
            ..Group::default()
        });
        g.fakes.push(FakeNote {
            t: 1100.0,
            ..FakeNote::default()
        });
        let list = tko_checks(&g);
        assert!(list.iter().all(|c| !c.blocking));
        assert!(list[0].text.starts_with("2 notes show as"), "{list:?}");
        assert!(list[1].text.contains("only at the hit"));
        assert!(list[2].ok && list[2].text.starts_with("1 fake"));
        g.groups[0].show.reveal = 0.5;
        assert_eq!(tko_checks(&g).len(), 2);
        assert!(tko_checks(&Gimmick::default()).is_empty());
    }
}
