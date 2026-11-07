//! The interface's language: English, Japanese or Russian (Settings > System > Language).
//!
//! Text is written in English where it's used, `tr!("Play")`, and that English is also its key
//! in the translations, `assets/lang/<code>.toml` (`"Play" = "Играть"`), built into the game
//! (build.rs). Text without a translation stays English. With arguments it formats like
//! `format!`: `tr!("{} maps", n)`, and variables it names go after a `;`: `tr!("{n} maps"; n)`.
//! The same English that wants different words somewhere gets a context in front,
//! `tr!(music: "Play")`, which is a `[music]` table in the files. `cargo run --example lang_sync`
//! writes every text the code asks for into the files (docs/translating.md).

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Ja,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::Ja, Lang::Ru];

    /// Its own name for itself, so anyone can find theirs.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ja => "日本語",
            Lang::Ru => "Русский",
        }
    }

    /// Its name in the config file.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ja => "ja",
            Lang::Ru => "ru",
        }
    }

    pub fn parse(s: &str) -> Option<Lang> {
        Lang::ALL
            .into_iter()
            .find(|l| l.code().eq_ignore_ascii_case(s.trim()))
    }

    /// The language `dir` steps away (wrapping).
    pub fn step(self, dir: i32) -> Lang {
        let i = Lang::ALL.iter().position(|&l| l == self).unwrap_or(0) as i32;
        Lang::ALL[(i + dir).rem_euclid(Lang::ALL.len() as i32) as usize]
    }
}

// The render thread draws what the main thread set; tests each draw in their own language.
#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(0);
#[cfg(test)]
thread_local! {
    static CURRENT: AtomicU8 = const { AtomicU8::new(0) };
}

thread_local! {
    /// The language `with` is running in.
    static BORROWED: std::cell::Cell<Option<Lang>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with the text it asks for in `lang`, on this thread only, whatever the interface is
/// showing meanwhile.
pub fn with<T>(lang: Lang, f: impl Fn() -> T) -> T {
    BORROWED.with(|b| b.set(Some(lang)));
    let out = f();
    BORROWED.with(|b| b.set(None));
    out
}

pub fn current() -> Lang {
    if let Some(lang) = BORROWED.with(std::cell::Cell::get) {
        return lang;
    }
    #[cfg(not(test))]
    let i = CURRENT.load(Ordering::Relaxed);
    #[cfg(test)]
    let i = CURRENT.with(|c| c.load(Ordering::Relaxed));
    Lang::ALL[i as usize % Lang::ALL.len()]
}

pub fn set(lang: Lang) {
    let i = Lang::ALL.iter().position(|&l| l == lang).unwrap_or(0) as u8;
    #[cfg(not(test))]
    CURRENT.store(i, Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.store(i, Ordering::Relaxed));
}

/// One translated text: (context, English, translation), sorted by context then English.
pub type Entry = (&'static str, &'static str, &'static str);

// The tables build.rs makes, `pub static JA: &[Entry]` and so on, one for each
// assets/lang/<code>.toml (one for a language the game doesn't offer yet builds too). A crate that
// borrows this file without the game's build.rs, the server, has none: it always speaks English.
#[allow(dead_code, unexpected_cfgs)]
mod tables {
    #[cfg(lang_tables)]
    include!(concat!(env!("OUT_DIR"), "/lang_tables.rs"));
    #[cfg(not(lang_tables))]
    pub static JA: &[super::Entry] = &[];
    #[cfg(not(lang_tables))]
    pub static RU: &[super::Entry] = &[];
}

/// Every translation `lang` has.
pub fn entries(lang: Lang) -> &'static [Entry] {
    match lang {
        Lang::En => &[],
        Lang::Ja => tables::JA,
        Lang::Ru => tables::RU,
    }
}

/// The current language's words for `en` (in `ctx`, "" for none), if it has them.
pub fn translation(ctx: &str, en: &str) -> Option<&'static str> {
    let table = entries(current());
    table
        .binary_search_by(|e| (e.0, e.1).cmp(&(ctx, en)))
        .ok()
        .map(|i| table[i].2)
}

/// `en` in the current language: what `tr!` without arguments gives.
pub fn text(ctx: &str, en: &'static str) -> &'static str {
    translation(ctx, en).unwrap_or(en)
}

/// How many forms a word takes for a count in `lang`, in the order `plural` picks them.
#[cfg(test)]
pub fn plural_forms(lang: Lang) -> usize {
    match lang {
        Lang::En => 2,
        Lang::Ja => 1,
        Lang::Ru => 3,
    }
}

/// Which of `lang`'s forms a word takes for `n` of it: English one / other; Russian one (1, 21,
/// 31...), few (2-4, 22-24..., and fractions) or many (0, 5-20, 25-30...).
pub fn plural(lang: Lang, n: f64) -> usize {
    let whole = n.fract() == 0.0;
    match lang {
        Lang::Ja => 0,
        Lang::En => usize::from(n != 1.0),
        Lang::Ru if !whole => 1,
        Lang::Ru => {
            let n = n.abs() as u64;
            match (n % 10, n % 100) {
                (1, r) if r != 11 => 0,
                (2..=4, r) if !(12..=14).contains(&r) => 1,
                _ => 2,
            }
        }
    }
}

/// One `{...}` in a text: what it shows and how.
struct Hole<'a> {
    /// A position (`{}` counts on, `{0}` says which) or a name.
    arg: Arg<'a>,
    /// What follows a `:` (`+.0`), empty if nothing does.
    spec: &'a str,
    /// The word forms after `|`s, to pick from by the argument's count.
    forms: Option<&'a str>,
}

#[derive(PartialEq)]
enum Arg<'a> {
    Pos(usize),
    Name(&'a str),
}

/// Each piece of `text`: plain text (with `{{` / `}}` made single), or a hole. `None` if a brace
/// isn't closed.
fn pieces(text: &str) -> Option<Vec<Result<std::borrow::Cow<'_, str>, Hole<'_>>>> {
    let mut out = Vec::new();
    let mut next = 0;
    let mut rest = text;
    while !rest.is_empty() {
        let stop = rest.find(['{', '}']).unwrap_or(rest.len());
        if stop > 0 {
            out.push(Ok(rest[..stop].into()));
        }
        rest = &rest[stop..];
        if let Some(r) = rest.strip_prefix("{{").or_else(|| rest.strip_prefix("}}")) {
            out.push(Ok(rest[..1].to_string().into()));
            rest = r;
        } else if let Some(r) = rest.strip_prefix('{') {
            let end = r.find('}')?;
            let inside = &r[..end];
            let (head, forms) = match inside.split_once('|') {
                Some((h, f)) => (h, Some(f)),
                None => (inside, None),
            };
            let (name, spec) = head.split_once(':').unwrap_or((head, ""));
            let arg = if name.is_empty() {
                next += 1;
                Arg::Pos(next - 1)
            } else if let Ok(i) = name.parse() {
                Arg::Pos(i)
            } else {
                Arg::Name(name)
            };
            out.push(Err(Hole { arg, spec, forms }));
            rest = &r[end + 1..];
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

/// `v` written the way `spec` says, for the specs the texts use (anything else writes it plainly).
fn write_spec(out: &mut String, v: &dyn std::fmt::Display, spec: &str) {
    use std::fmt::Write;
    let _ = match spec {
        ".0" => write!(out, "{v:.0}"),
        ".1" => write!(out, "{v:.1}"),
        ".2" => write!(out, "{v:.2}"),
        ".3" => write!(out, "{v:.3}"),
        "+.0" => write!(out, "{v:+.0}"),
        "+.1" => write!(out, "{v:+.1}"),
        "+.2" => write!(out, "{v:+.2}"),
        "02" => write!(out, "{v:02}"),
        "03" => write!(out, "{v:03}"),
        _ => write!(out, "{v}"),
    };
}

/// The specs `write_spec` knows.
pub const SPECS: [&str; 10] = ["", ".0", ".1", ".2", ".3", "+.0", "+.1", "+.2", "02", "03"];

/// A translation `t` of the English `en` filled in with the arguments `tr!` was given: a hole
/// takes its spec from the English hole for the same argument unless it has its own, and
/// `{n|form|form}` picks a form by `n`'s count. A hole with no argument stays as written.
pub fn fill(
    t: &str,
    en: &str,
    pos: &[&dyn std::fmt::Display],
    named: &[(&str, &dyn std::fmt::Display)],
) -> String {
    let Some(parts) = pieces(t) else {
        return t.to_string();
    };
    let en_holes = pieces(en).unwrap_or_default();
    let en_spec = |arg: &Arg| {
        en_holes.iter().find_map(|p| match p {
            Err(h) if h.arg == *arg => Some(h.spec),
            _ => None,
        })
    };
    let lang = current();
    let mut out = String::with_capacity(t.len() + 16);
    for part in &parts {
        let h = match part {
            Ok(s) => {
                out.push_str(s);
                continue;
            }
            Err(h) => h,
        };
        let value = match h.arg {
            Arg::Pos(i) => pos.get(i).copied(),
            Arg::Name(n) => named.iter().find(|a| a.0 == n).map(|a| a.1),
        };
        let Some(v) = value else {
            out.push('{');
            out.push_str(match h.arg {
                Arg::Pos(_) => "",
                Arg::Name(n) => n,
            });
            out.push('}');
            continue;
        };
        if let Some(forms) = h.forms {
            let n: f64 = v.to_string().trim().parse().unwrap_or(0.0);
            let forms: Vec<&str> = forms.split('|').collect();
            out.push_str(forms[plural(lang, n).min(forms.len() - 1)]);
        } else {
            let spec = if h.spec.is_empty() {
                en_spec(&h.arg).unwrap_or("")
            } else {
                h.spec
            };
            write_spec(&mut out, v, spec);
        }
    }
    out
}

/// Has a `{` or `}`: text for `format!`, which a `tr!` without arguments would show as it is.
pub const fn has_brace(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' || b[i] == b'}' {
            return true;
        }
        i += 1;
    }
    false
}

/// Whether `a[from..to]` is `b`, in a `const`.
const fn same(a: &[u8], from: usize, to: usize, b: &str) -> bool {
    let b = b.as_bytes();
    if to - from != b.len() {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        if a[from + i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// What's wrong with the holes of `en`, a `tr!` text given the variables `vars`: 0 nothing, 1 a
/// `{name}` that isn't among `vars` (a translation couldn't show it), 2 a spec `write_spec`
/// doesn't know (a translation would write it plainly).
pub const fn hole_trouble(en: &str, vars: &[&str]) -> u8 {
    let b = en.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'{' {
            i += 1;
            continue;
        }
        if i + 1 < b.len() && b[i + 1] == b'{' {
            i += 2;
            continue;
        }
        let start = i + 1;
        let mut colon = start;
        while colon < b.len() && b[colon] != b'}' && b[colon] != b':' {
            colon += 1;
        }
        if colon > start && !b[start].is_ascii_digit() {
            let mut k = 0;
            let mut found = false;
            while k < vars.len() {
                found |= same(b, start, colon, vars[k]);
                k += 1;
            }
            if !found {
                return 1;
            }
        }
        let mut end = colon;
        while end < b.len() && b[end] != b'}' {
            end += 1;
        }
        if colon < end {
            let mut k = 0;
            let mut known = false;
            while k < SPECS.len() {
                known |= same(b, colon + 1, end, SPECS[k]);
                k += 1;
            }
            if !known {
                return 2;
            }
        }
        i = end + 1;
    }
    0
}

/// English text in the current language: `tr!("Play")`, `tr!(music: "Play")` in a context,
/// `tr!("{} maps", n)` with `format!` arguments, `tr!("{n} maps"; n)` with the variables it
/// names after a `;` (or both: `tr!("{} of {total}", done; total)`).
macro_rules! tr {
    ($($ctx:ident :)? $en:literal $(,)?) => {{
        const {
            assert!(
                !$crate::lang::has_brace($en),
                "tr! text with braces needs its arguments: tr!(\"{{}} maps\", n) or tr!(\"{{n}} maps\"; n)"
            )
        };
        $crate::lang::text(concat!($(stringify!($ctx))?), $en)
    }};
    ($($ctx:ident :)? $en:literal $(, $arg:expr)* $(,)? $(; $($var:ident),* $(,)?)?) => {{
        const {
            let vars: &[&str] = &[$($(stringify!($var)),*)?];
            assert!(
                $crate::lang::hole_trouble($en, vars) != 1,
                "tr!: each {{name}} in the text goes after a ; so translations get it: tr!(\"{{n}} maps\"; n)"
            );
            assert!(
                $crate::lang::hole_trouble($en, vars) != 2,
                "tr!: a format spec lang::write_spec doesn't know"
            );
        };
        match $crate::lang::translation(concat!($(stringify!($ctx))?), $en) {
            None => format!($en $(, $arg)*),
            Some(t) => $crate::lang::fill(
                t,
                $en,
                &[$(&$arg as &dyn ::std::fmt::Display),*],
                &[$($((stringify!($var), &$var as &dyn ::std::fmt::Display)),*)?],
            ),
        }
    }};
}

/// The server answered something that makes no sense where it was asked.
pub fn odd_reply() -> String {
    tr!("odd reply from the server").into()
}

#[cfg(test)]
#[allow(unexpected_cfgs)]
mod tests {
    use super::*;

    // Only with the game's translations (not in the server).
    #[cfg(lang_tables)]
    #[test]
    fn picks_the_current_language() {
        set(Lang::Ja);
        assert_eq!(tr!("Off"), "オフ");
        assert_eq!(tr!(music: "Play"), "再生");
        set(Lang::Ru);
        assert_eq!(tr!("Off"), "Выкл");
        let m = 3;
        assert_eq!(tr!("After {m} minutes"; m), "Через 3 минуты");
        set(Lang::En);
        assert_eq!(tr!("Off"), "Off");
        assert_eq!(tr!(music: "Play"), "Play");
        assert_eq!(tr!("After {m} minutes"; m), "After 3 minutes");
        assert_eq!(hole_trouble("{a:+.0} {} {{b}}", &["a"]), 0);
        assert_eq!(hole_trouble("{a} {b}", &["a"]), 1);
        assert_eq!(hole_trouble("{a:>8}", &["a"]), 2);
        assert_eq!(Lang::parse("JA"), Some(Lang::Ja));
        assert_eq!(Lang::En.step(-1), Lang::Ru);
    }

    #[test]
    fn fills_in_holes() {
        set(Lang::Ru);
        let x = 2.6_f64;
        let s = "a";
        let filled = fill(
            "{1} {0} {x} {x:.1} {{ok}} {n|раз|раза|раз} {missing}",
            "{} {:+.0} {x:.2} {n}",
            &[&s, &x],
            &[("x", &x), ("n", &21)],
        );
        set(Lang::En);
        // A hole takes its English spec unless it has its own.
        assert_eq!(filled, "+3 a 2.60 2.6 {ok} раз {missing}");
    }

    #[test]
    fn russian_plurals() {
        let f = |n: f64| ["карта", "карты", "карт"][plural(Lang::Ru, n)];
        let got = [1.0, 2.0, 5.0, 11.0, 12.0, 21.0, 22.0, 25.0, 111.0, 0.0, 1.5].map(f);
        let want = [
            "карта",
            "карты",
            "карт",
            "карт",
            "карт",
            "карта",
            "карты",
            "карт",
            "карт",
            "карт",
            "карты",
        ];
        assert_eq!(got, want);
    }

    /// Every translation reads: its braces close, it only asks for arguments its English has
    /// (or its counts), with specs `write_spec` knows, and its word forms fit its language.
    #[test]
    fn translations_fit_their_english() {
        for lang in Lang::ALL {
            let table = entries(lang);
            assert!(
                table
                    .windows(2)
                    .all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1))
            );
            for &(ctx, en, t) in table {
                let at = format!("{} [{ctx}] {en:?} = {t:?}", lang.code());
                let en_parts = pieces(en).unwrap_or_else(|| panic!("{at}: English braces"));
                let en_args: Vec<&Arg> = en_parts
                    .iter()
                    .filter_map(|p| p.as_ref().err().map(|h| &h.arg))
                    .collect();
                let parts = pieces(t).unwrap_or_else(|| panic!("{at}: a brace isn't closed"));
                for h in parts.iter().filter_map(|p| p.as_ref().err()) {
                    assert!(
                        en_args.contains(&&h.arg),
                        "{at}: no such argument in the English"
                    );
                    assert!(SPECS.contains(&h.spec), "{at}: unknown spec {:?}", h.spec);
                    if let Some(forms) = h.forms {
                        let n = forms.split('|').count();
                        assert_eq!(n, plural_forms(lang), "{at}: {n} word forms");
                    }
                }
                if en_args.is_empty() {
                    assert!(!has_brace(t), "{at}: braces in a text with no arguments");
                }
            }
        }
    }
}
