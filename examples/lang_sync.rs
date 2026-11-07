//! Writes every text the game asks for (`tr!` in src/) into the translations, assets/lang/*.toml:
//! texts new to the code come in empty, texts the code no longer has go, and the ones there keep
//! their translations. `cargo run --example lang_sync -- --new es` starts a file for a new
//! language; `-- --check` only says whether the files are up to date (docs/translating.md).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const HEADER: &str = "\
# The game's text in this language: \"English\" = \"translation\". Leave \"\" where there's none
# yet (the game shows the English). Keep each {} and {name} (in any order you like); {n|form|form}
# picks a word's form by the count n. A [table] holds texts whose English is used somewhere else
# too and wants other words there; [unused] keeps translations of English the game no longer
# has, to reuse or delete. See docs/translating.md. Written by `cargo run --example lang_sync`,
# which keeps only these comments.
";

/// A `tr!` text: its context ("" for none) and English, and the file it's first in.
struct Text {
    ctx: String,
    en: String,
    file: String,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|a| a == "--check");
    let new = args
        .iter()
        .position(|a| a == "--new")
        .map(|i| args.get(i + 1).expect("--new <language code>").clone());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let texts = texts(root);
    let dir = root.join("assets/lang");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("assets/lang")
        .map(|e| e.expect("assets/lang").path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    if let Some(code) = new {
        let path = dir.join(format!("{code}.toml"));
        assert!(!path.exists(), "{} is there already", path.display());
        files.push(path);
    }
    files.sort();
    let mut stale = false;
    for path in files {
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        let had = translations(&old, &path);
        let (text, done, gone) = write(&texts, &had);
        let name = path.file_name().unwrap().to_string_lossy();
        println!(
            "{name}: {done} of {} translated, {gone} kept under [unused]",
            texts.len()
        );
        if text != old {
            stale = true;
            if !check {
                std::fs::write(&path, text).expect("writing the translation");
            }
        }
    }
    if check && stale {
        eprintln!("the translations are out of date: cargo run --example lang_sync");
        std::process::exit(1);
    }
}

/// What a file has: (context, English) -> translation.
fn translations(text: &str, path: &Path) -> HashMap<(String, String), String> {
    let table: toml::Table = text
        .parse()
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = HashMap::new();
    for (key, value) in table {
        match value {
            toml::Value::String(t) => {
                out.insert((String::new(), key), t);
            }
            toml::Value::Table(ctx) => {
                for (en, t) in ctx {
                    let t = t.as_str().expect("a translation in quotes").to_string();
                    out.insert((key.clone(), en), t);
                }
            }
            _ => panic!("{}: {key:?} isn't text", path.display()),
        }
    }
    out
}

/// The file for `had`'s language with every one of `texts`, grouped by the file they're first in,
/// and the translations of texts the code no longer has under `[unused]`; and how many of `texts`
/// it translates and how many it keeps unused.
fn write(texts: &[Text], had: &HashMap<(String, String), String>) -> (String, usize, usize) {
    let mut out = String::from(HEADER);
    let mut by_ctx: BTreeMap<&str, Vec<&Text>> = BTreeMap::new();
    for t in texts {
        by_ctx.entry(&t.ctx).or_default().push(t);
    }
    let mut done = 0;
    for (ctx, list) in by_ctx {
        if !ctx.is_empty() {
            out.push_str(&format!("\n[{ctx}]\n"));
        }
        let mut file = "";
        for t in list {
            if t.file != file {
                file = &t.file;
                out.push_str(&format!("\n# {file}\n"));
            }
            let tr = had
                .get(&(t.ctx.clone(), t.en.clone()))
                .map_or("", String::as_str);
            done += usize::from(!tr.is_empty());
            out.push_str(&format!("{} = {}\n", quote(&t.en), quote(tr)));
        }
    }
    let used = |k: &(String, String)| texts.iter().any(|t| t.ctx == k.0 && t.en == k.1);
    let unused: BTreeMap<&str, &str> = had
        .iter()
        .filter(|(k, t)| !t.is_empty() && !used(k))
        .map(|(k, t)| (k.1.as_str(), t.as_str()))
        .collect();
    if !unused.is_empty() {
        out.push_str("\n[unused]\n");
        for (en, t) in &unused {
            out.push_str(&format!("{} = {}\n", quote(en), quote(t)));
        }
    }
    (out, done, unused.len())
}

/// `s` as a TOML string.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Every `tr!` text in src/, once each, in the order of the files and then of the code.
fn texts(root: &Path) -> Vec<Text> {
    let mut files = Vec::new();
    let mut dirs = vec![root.join("src")];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(&dir).expect("src") {
            let p = e.expect("src").path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut out: Vec<Text> = Vec::new();
    for path in files {
        let src = std::fs::read_to_string(&path).expect("a source file");
        let file = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let mut at = 0;
        while let Some(i) = src[at..].find("tr!(") {
            let start = at + i;
            at = start + 4;
            let line_start = src[..start].rfind('\n').map_or(0, |n| n + 1);
            let word_before = src[..start].chars().next_back().is_some_and(is_ident);
            if word_before || src[line_start..start].contains("//") {
                continue;
            }
            // Not a call: tr!( written inside some text.
            let Some((ctx, en)) = call(&src[at..]) else {
                continue;
            };
            if !out.iter().any(|t| t.ctx == ctx && t.en == en) {
                let file = file.clone();
                out.push(Text { ctx, en, file });
            }
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The context and English at the start of a `tr!(` call's insides.
fn call(s: &str) -> Option<(String, String)> {
    let s = s.trim_start();
    let ident_len = s.find(|c: char| !is_ident(c)).unwrap_or(0);
    let (ctx, s) = match s[ident_len..].trim_start().strip_prefix(':') {
        Some(rest) if ident_len > 0 && !rest.starts_with(':') => (&s[..ident_len], rest),
        _ => ("", s),
    };
    Some((ctx.to_string(), literal(s.trim_start())?))
}

/// The value of the Rust string literal `s` starts with.
fn literal(s: &str) -> Option<String> {
    if let Some(raw) = s.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let body = raw[hashes..].strip_prefix('"')?;
        let end = body.find(&format!("\"{}", "#".repeat(hashes)))?;
        return Some(body[..end].to_string());
    }
    let mut chars = s.strip_prefix('"')?.chars();
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                '\n' => {
                    let rest = chars.as_str().trim_start();
                    chars = rest.chars();
                }
                'u' => {
                    let rest = chars.as_str().strip_prefix('{')?;
                    let end = rest.find('}')?;
                    let code = u32::from_str_radix(&rest[..end].replace('_', ""), 16).ok()?;
                    out.push(char::from_u32(code)?);
                    chars = rest[end + 1..].chars();
                }
                'x' => {
                    let rest = chars.as_str();
                    out.push(u8::from_str_radix(rest.get(..2)?, 16).ok()? as char);
                    chars = rest[2..].chars();
                }
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
}
