//! The editor manual's words, assets/lang/manual/<code>.txt (a language without one shows the
//! English). Each chapter starts `=== Title`, then `try: label` for one with a button at the end;
//! its body is lines: `# ` a heading, `- ` a bullet, `> ` a tip, an empty line a gap, anything
//! else a paragraph. `{recipes}` stands for the list of node recipes, written from `tko::recipes`.

use std::sync::OnceLock;

use crate::lang::{self, Lang};

pub struct Chapter {
    pub title: &'static str,
    pub body: &'static str,
    /// A button at the end that goes and does the thing: (label, what).
    pub try_it: Option<(&'static str, TryIt)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TryIt {
    TimingWindow,
    Nodes,
    Tour,
    /// A set of clips laid over the map's first kiai, to see and pick apart.
    Showcase,
    /// Paths, a move, fakes, a freeze, the drum sliding and a lie laid over the map's first kiai.
    MotionShowcase,
}

pub const COUNT: usize = 12;

/// What each chapter's button does, in the files' order; their `try:` lines say what it reads.
const TRY: [Option<TryIt>; COUNT] = [
    Some(TryIt::Tour),
    Some(TryIt::TimingWindow),
    None,
    Some(TryIt::TimingWindow),
    None,
    Some(TryIt::Showcase),
    Some(TryIt::MotionShowcase),
    None,
    Some(TryIt::Nodes),
    Some(TryIt::Nodes),
    None,
    None,
];

const EN: &str = include_str!("../../../assets/lang/manual/en.txt");

/// `lang`'s own manual, if it has one.
fn file(lang: Lang) -> Option<&'static str> {
    match lang {
        Lang::En => Some(EN),
        Lang::Ja => None,
        Lang::Ru => Some(include_str!("../../../assets/lang/manual/ru.txt")),
    }
}

/// The chapters in `text`, a manual file, with their buttons from `TRY`.
fn parse(text: &'static str) -> Vec<Chapter> {
    let mut out = Vec::new();
    let mut rest = text;
    // The translators' notes before the first chapter.
    while !rest.is_empty() && !rest.starts_with("=== ") {
        rest = rest.split_once('\n').map_or("", |(_, r)| r);
    }
    while let Some(r) = rest.strip_prefix("=== ") {
        let (title, r) = r.split_once('\n').unwrap_or((r, ""));
        let (label, r) = match r.strip_prefix("try: ") {
            Some(t) => {
                let (label, r) = t.split_once('\n').unwrap_or((t, ""));
                (Some(label.trim_end()), r)
            }
            None => (None, r),
        };
        let end = r.find("\n=== ").map_or(r.len(), |i| i + 1);
        let body = r[..end].trim_end_matches('\n');
        // Its body ends in one line break, as the layout expects.
        let body = &r[..body.len() + usize::from(body.len() < end)];
        let try_it = TRY.get(out.len()).copied().flatten();
        out.push(Chapter {
            title: title.trim_end(),
            body,
            try_it: try_it.map(|t| (label.unwrap_or(""), t)),
        });
        rest = &r[end..];
    }
    out
}

/// The chapters in the current language.
pub fn chapters() -> &'static [Chapter] {
    static PARSED: [OnceLock<Vec<Chapter>>; Lang::ALL.len()] =
        [const { OnceLock::new() }; Lang::ALL.len()];
    let lang = lang::current();
    let (i, text) = match file(lang) {
        Some(text) => (Lang::ALL.iter().position(|&l| l == lang).unwrap_or(0), text),
        None => (0, EN),
    };
    PARSED[i].get_or_init(|| parse(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every manual has every chapter, each with its button's label where it has a button.
    #[test]
    fn each_manual_has_every_chapter_and_button() {
        for lang in Lang::ALL {
            let Some(text) = file(lang) else { continue };
            let chapters = parse(text);
            assert_eq!(chapters.len(), COUNT, "{}", lang.code());
            for (c, t) in chapters.iter().zip(TRY) {
                assert!(!c.title.is_empty() && c.body.ends_with('\n'), "{}", c.title);
                assert!(!c.body.ends_with("\n\n"), "{}", c.title);
                match c.try_it {
                    Some((label, _)) => assert!(!label.is_empty(), "{}: try:", c.title),
                    None => assert!(t.is_none()),
                }
            }
        }
        assert!(
            parse(EN)[0]
                .body
                .starts_with("The editor makes taiko maps.")
        );
    }
}
