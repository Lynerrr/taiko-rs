//! The interface's language: English, Japanese or Russian (Settings > General > Language).
//! Text is translated where it's written: `tr!("Play", "プレイ", "Играть")` picks one for the
//! current language, and with format arguments after a `;` (`tr!("{} maps", "{}譜面",
//! "{} карт"; n)`, or `tr!("{n} maps", "{n}譜面", "{n} карт";)` for captured variables) it formats
//! it like `format!`. Plain `"..."` text stays English.

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
        Lang::ALL.into_iter().find(|l| l.code().eq_ignore_ascii_case(s.trim()))
    }

    /// The language `dir` steps away (wrapping).
    pub fn step(self, dir: i32) -> Lang {
        let i = Lang::ALL.iter().position(|&l| l == self).unwrap_or(0) as i32;
        Lang::ALL[(i + dir).rem_euclid(3) as usize]
    }
}

// The render thread draws what the main thread set; tests each draw in their own language.
#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(0);
#[cfg(test)]
thread_local! {
    static CURRENT: AtomicU8 = const { AtomicU8::new(0) };
}

pub fn current() -> Lang {
    #[cfg(not(test))]
    let i = CURRENT.load(Ordering::Relaxed);
    #[cfg(test)]
    let i = CURRENT.with(|c| c.load(Ordering::Relaxed));
    Lang::ALL[i as usize % 3]
}

pub fn set(lang: Lang) {
    let i = Lang::ALL.iter().position(|&l| l == lang).unwrap_or(0) as u8;
    #[cfg(not(test))]
    CURRENT.store(i, Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.store(i, Ordering::Relaxed));
}

/// Russian's form of a word for `n` of it: one (1, 21, 31...), few (2-4, 22-24...) or many
/// (0, 5-20, 25-30...): `ru_plural(n, ["карта", "карты", "карт"])`.
pub fn ru_plural<N: TryInto<u64>>(n: N, [one, few, many]: [&'static str; 3]) -> &'static str {
    let n: u64 = n.try_into().unwrap_or(0);
    match (n % 10, n % 100) {
        (1, r) if r != 11 => one,
        (2..=4, r) if !(12..=14).contains(&r) => few,
        _ => many,
    }
}

/// Has a `{` or `}`: text for `format!`, which the plain `tr!` would show as it is.
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

/// Text in the current language: `tr!(english, japanese, russian)`, or with `format!`
/// arguments after a `;` (each translation must use them all; just `;` formats in captured
/// variables).
macro_rules! tr {
    ($en:literal, $ja:literal, $ru:literal $(,)?) => {{
        const {
            assert!(
                !($crate::lang::has_brace($en) || $crate::lang::has_brace($ja) || $crate::lang::has_brace($ru)),
                "tr! text with braces needs a ; after it to be formatted"
            )
        };
        match $crate::lang::current() {
            $crate::lang::Lang::En => $en,
            $crate::lang::Lang::Ja => $ja,
            $crate::lang::Lang::Ru => $ru,
        }
    }};
    ($en:literal, $ja:literal, $ru:literal; $($arg:tt)*) => {
        match $crate::lang::current() {
            $crate::lang::Lang::En => format!($en, $($arg)*),
            $crate::lang::Lang::Ja => format!($ja, $($arg)*),
            $crate::lang::Lang::Ru => format!($ru, $($arg)*),
        }
    };
}

/// The server answered something that makes no sense where it was asked.
pub fn odd_reply() -> String {
    tr!("odd reply from the server", "サーバーからの応答がおかしい", "странный ответ сервера").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_current_language() {
        set(Lang::Ja);
        assert_eq!(tr!("Play", "プレイ", "Играть"), "プレイ");
        // A word only one language needs is captured, not passed (format! wants every argument
        // used).
        let n = 3;
        let maps = ru_plural(n, ["карта", "карты", "карт"]);
        set(Lang::Ru);
        assert_eq!(tr!("{n} maps", "{n}譜面", "{n} {maps}";), "3 карты");
        set(Lang::En);
        assert_eq!(tr!("{} maps", "{}譜面", "{} карт"; 5), "5 maps");
        assert_eq!(Lang::parse("JA"), Some(Lang::Ja));
        assert_eq!(Lang::En.step(-1), Lang::Ru);
    }

    #[test]
    fn russian_plurals() {
        let f = |n| ru_plural(n, ["карта", "карты", "карт"]);
        assert_eq!([f(1), f(2), f(5), f(11), f(12), f(21), f(22), f(25), f(111), f(0)], ["карта", "карты", "карт", "карт", "карт", "карта", "карты", "карт", "карт", "карт"]);
    }
}
