//! Self-update from the rolling GitHub release that CI publishes on every push: "latest" from the
//! main branch, "dev" (test builds) from the `dev` branch. A build updates from the channel it was
//! built for; switching channels installs the other channel's build.
//!
//! A build knows the commit it was built from (`GITHUB_SHA`, set by GitHub Actions) and the
//! release says which commit it was built from, so different = newer. Local builds have no
//! commit and never update themselves. Installing downloads this platform's archive, unpacks the
//! binary next to the running one and swaps it in: Linux and macOS (where the binary is the one
//! inside the .app, and the rest of the app stays as it is) rename over the running file; Windows
//! can't overwrite a running exe but can rename it, so the old one becomes `taiko-rs.exe.old`
//! (deleted on the next start). Then the game restarts itself.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The commit this binary was built from (CI builds only).
pub const BUILD: Option<&str> = option_env!("GITHUB_SHA");
const REPO: &str = match option_env!("GITHUB_REPOSITORY") {
    Some(r) => r,
    None => "Lynerrr/taiko-rs",
};
/// The channel this binary was built for (`TAIKO_CHANNEL` in CI).
pub const BUILD_CHANNEL: Channel = match option_env!("TAIKO_CHANNEL") {
    // (`str` can't be matched in a const.)
    Some(c) if matches!(c.as_bytes(), [b'd', b'e', b'v']) => Channel::Dev,
    _ => Channel::Stable,
};
const USER_AGENT: &str = concat!("taiko-rs/", env!("CARGO_PKG_VERSION"), " (updater)");
#[cfg(windows)]
const ASSET: &str = "taiko-rs-windows-x64.zip";
#[cfg(target_os = "macos")]
const ASSET: &str = "taiko-rs-macos.tar.gz";
#[cfg(not(any(windows, target_os = "macos")))]
const ASSET: &str = "taiko-rs-linux-x64.tar.gz";
/// Where the binary is in that archive.
#[cfg(target_os = "macos")]
const ARCHIVED_BINARY: &str = "taiko-rs.app/Contents/MacOS/taiko-rs";
#[cfg(not(any(windows, target_os = "macos")))]
const ARCHIVED_BINARY: &str = "taiko-rs/taiko-rs";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Channel {
    Stable,
    Dev,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Dev => "dev",
        }
    }

    /// The branch its builds come from.
    pub fn branch(self) -> &'static str {
        match self {
            Channel::Stable => "worktree-taiko-client",
            Channel::Dev => "dev",
        }
    }

    /// The release it's published as.
    fn tag(self) -> &'static str {
        match self {
            Channel::Stable => "latest",
            Channel::Dev => "dev",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// A local build, or checking is turned off.
    Off,
    Checking,
    UpToDate,
    /// A newer build (or the other channel's): its commit, the archive's URL and size.
    Available { commit: String, url: String, size: u64 },
    Downloading,
    /// Swapped in: restart into it.
    Installed,
    Failed(String),
}

pub struct Updater {
    status: Arc<Mutex<Status>>,
    /// The channel checked and installed from.
    channel: Arc<Mutex<Channel>>,
    /// Download progress: bytes so far and in total.
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    /// When the last check started (startup counts, checked or not).
    last_check: Mutex<std::time::Instant>,
}

/// Short form of a commit hash for display.
pub fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

impl Updater {
    /// Checks for a newer build in the background when `check` (and this is a CI build).
    pub fn new(check: bool) -> Updater {
        let u = Updater {
            status: Arc::new(Mutex::new(Status::Off)),
            channel: Arc::new(Mutex::new(BUILD_CHANNEL)),
            done: Arc::default(),
            total: Arc::default(),
            last_check: Mutex::new(std::time::Instant::now()),
        };
        // An APK can't replace itself; Android installs updates.
        if check && !cfg!(target_os = "android") {
            u.check();
        }
        u
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Download progress 0..1 while downloading.
    pub fn progress(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed).max(1);
        (self.done.load(Ordering::Relaxed) as f32 / total as f32).min(1.0)
    }

    pub fn channel(&self) -> Channel {
        *self.channel.lock().unwrap()
    }

    /// Switches to `channel` and looks for its build (unless one is being installed).
    pub fn set_channel(&self, channel: Channel) {
        if matches!(self.status(), Status::Downloading | Status::Installed) {
            return;
        }
        *self.channel.lock().unwrap() = channel;
        if matches!(self.status(), Status::Checking) {
            // The check under way is for the other channel; let it know it's stale.
            *self.status.lock().unwrap() = Status::Off;
        }
        self.check();
    }

    /// Checks again once `every` has passed since the last check, so a game left running hears
    /// about newer builds. An update already found stays as it is (a failed look must not hide
    /// its badge).
    pub fn check_every(&self, every: Duration) {
        let found = matches!(self.status(), Status::Available { .. });
        if cfg!(target_os = "android") || found || self.last_check.lock().unwrap().elapsed() < every {
            return;
        }
        self.check();
    }

    pub fn check(&self) {
        let Some(build) = BUILD else { return };
        if matches!(self.status(), Status::Checking | Status::Downloading | Status::Installed) {
            return;
        }
        *self.last_check.lock().unwrap() = std::time::Instant::now();
        *self.status.lock().unwrap() = Status::Checking;
        let (status, current) = (self.status.clone(), self.channel.clone());
        let channel = self.channel();
        std::thread::spawn(move || {
            let result = release(channel).map(|(commit, url, size)| {
                if commit == build && channel == BUILD_CHANNEL { Status::UpToDate } else { Status::Available { commit, url, size } }
            });
            let mut status = status.lock().unwrap();
            // The channel was switched meanwhile: that check reports instead.
            if *current.lock().unwrap() != channel {
                return;
            }
            *status = result.unwrap_or_else(|e| {
                eprintln!("update check: {e}");
                Status::Failed(e)
            });
        });
    }

    /// Downloads and swaps in the available build. Status goes to `Installed` (then call
    /// `restart`) or `Failed`.
    pub fn install(&self) {
        let Status::Available { url, size, .. } = self.status() else { return };
        *self.status.lock().unwrap() = Status::Downloading;
        self.done.store(0, Ordering::Relaxed);
        self.total.store(size, Ordering::Relaxed);
        let (status, done, total) = (self.status.clone(), self.done.clone(), self.total.clone());
        std::thread::spawn(move || {
            let result = install(&url, &done, &total);
            *status.lock().unwrap() = match result {
                Ok(()) => Status::Installed,
                Err(e) => {
                    eprintln!("update: {e}");
                    Status::Failed(e)
                }
            };
        });
    }
}

/// The running binary's path, looked up once at startup: after the swap, Linux reports the
/// running file as deleted.
fn exe_path() -> Result<PathBuf, String> {
    static EXE: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| format!("where am I: {e}"))).clone()
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// Removes what an update leaves behind (the replaced exe on Windows, half-done downloads).
/// Call at startup.
pub fn cleanup() {
    let Ok(exe) = exe_path() else { return };
    for suffix in [".old", ".new", ".download"] {
        let _ = std::fs::remove_file(sibling(&exe, suffix));
    }
}

/// Starts the (new) binary with the same arguments. The caller then quits.
pub fn restart() -> Result<(), String> {
    let exe = exe_path()?;
    std::process::Command::new(&exe).args(std::env::args_os().skip(1)).spawn().map_err(|e| format!("restart: {e}"))?;
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(10))).build().into()
}

/// A GitHub API call about the repository (`path` after `repos/<repo>/`): the JSON it returns.
pub fn api_get(path: &str) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{REPO}/{path}");
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into();
    agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
}

/// The channel's release: its commit and this platform's archive (URL, size).
fn release(channel: Channel) -> Result<(String, String, u64), String> {
    #[derive(serde::Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
        size: u64,
    }
    #[derive(serde::Deserialize)]
    struct Release {
        target_commitish: String,
        assets: Vec<Asset>,
    }
    let url = format!("https://api.github.com/repos/{REPO}/releases/tags/{}", channel.tag());
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into();
    let text = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let release: Release = serde_json::from_str(&text).map_err(|e| format!("release: {e}"))?;
    let asset = release.assets.into_iter().find(|a| a.name == ASSET).ok_or_else(|| format!("no {ASSET} in the {} release", channel.name()))?;
    Ok((release.target_commitish, asset.browser_download_url, asset.size))
}

fn install(url: &str, done: &AtomicU64, total: &AtomicU64) -> Result<(), String> {
    let exe = exe_path()?;
    let download = sibling(&exe, ".download");
    let new = sibling(&exe, ".new");
    let result = (|| {
        fetch(url, &download, done, total)?;
        let binary = unpack(&download)?;
        if binary.len() < 1 << 20 {
            return Err("the downloaded binary looks broken".into());
        }
        write_executable(&new, &binary)?;
        swap(&exe, &new)
    })();
    let _ = std::fs::remove_file(&download);
    let _ = std::fs::remove_file(&new);
    result
}

fn fetch(url: &str, path: &Path, done: &AtomicU64, total: &AtomicU64) -> Result<(), String> {
    let mut response = agent().get(url).header("User-Agent", USER_AGENT).call().map_err(|e| e.to_string())?;
    let body = response.body_mut();
    if let Some(len) = body.content_length() {
        total.store(len, Ordering::Relaxed);
    }
    let mut reader = body.with_config().limit(256 << 20).reader();
    let mut file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    file.flush().map_err(|e| e.to_string())
}

/// The game binary out of the release archive.
#[cfg(windows)]
fn unpack(archive: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut entry = zip.by_name("taiko-rs.exe").map_err(|e| format!("taiko-rs.exe: {e}"))?;
    let mut out = Vec::new();
    entry.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// The game binary out of the release archive (a gzipped tar).
#[cfg(not(windows))]
fn unpack(archive: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(file).read_to_end(&mut tar).map_err(|e| format!("gzip: {e}"))?;
    tar_file(&tar, ARCHIVED_BINARY).ok_or_else(|| format!("{ARCHIVED_BINARY} not in the archive"))
}

/// A regular file's contents out of an (uncompressed, ustar) tar.
#[cfg_attr(windows, allow(dead_code))]
fn tar_file(tar: &[u8], wanted: &str) -> Option<Vec<u8>> {
    let mut at = 0;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            return None;
        }
        let field = |range: std::ops::Range<usize>| {
            let raw = &header[range];
            String::from_utf8_lossy(&raw[..raw.iter().position(|&b| b == 0).unwrap_or(raw.len())]).into_owned()
        };
        let name = field(0..100);
        let prefix = field(345..500);
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let size = u64::from_str_radix(field(124..136).trim(), 8).ok()? as usize;
        let data = at + 512;
        if path.trim_start_matches("./") == wanted && matches!(header[156], b'0' | 0) {
            return tar.get(data..data + size).map(<[u8]>::to_vec);
        }
        at = data + size.div_ceil(512) * 512;
    }
    None
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Puts `new` where `exe` is while `exe` is running.
fn swap(exe: &Path, new: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let old = sibling(exe, ".old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old).map_err(|e| format!("moving the old exe aside: {e}"))?;
        if let Err(e) = std::fs::rename(new, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(format!("putting the new exe in place: {e}"));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    std::fs::rename(new, exe).map_err(|e| format!("replacing {}: {e}", exe.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_a_file_in_a_tar() {
        // A tar with one directory entry and one file, built by hand.
        fn header(name: &str, size: usize, kind: u8) -> Vec<u8> {
            let mut h = vec![0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            let size = format!("{size:011o}\0");
            h[124..136].copy_from_slice(size.as_bytes());
            h[156] = kind;
            h
        }
        let mut tar = header("taiko-rs/", 0, b'5');
        tar.extend(header("taiko-rs/README.md", 3, b'0'));
        tar.extend(b"hi\n");
        tar.resize(tar.len().div_ceil(512) * 512, 0);
        tar.extend(header("taiko-rs/taiko-rs", 5, b'0'));
        tar.extend(b"\x7fELF!");
        tar.resize(tar.len().div_ceil(512) * 512 + 1024, 0);
        assert_eq!(super::tar_file(&tar, "taiko-rs/taiko-rs").as_deref(), Some(&b"\x7fELF!"[..]));
        assert_eq!(super::tar_file(&tar, "taiko-rs/missing"), None);
    }
}
