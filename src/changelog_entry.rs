//! Explicit player-facing notes shared by the game and the Discord bot.

pub struct Entry<'a> {
    pub title: &'a str,
    pub points: Vec<&'a str>,
}

/// Only standalone `Changelog: ...` lines in the commit body are public notes.
/// Additional marked lines become points; unmarked prose, bullets and trailers stay private.
pub fn parse(message: &str) -> Option<Entry<'_>> {
    let mut lines = message.lines();
    let subject = lines.next()?.trim();
    if subject.is_empty() || subject.starts_with("Merge ") || message.to_ascii_lowercase().contains("krokodil") {
        return None;
    }
    let mut title = None;
    let mut points = Vec::new();
    for line in lines {
        let Some((key, text)) = line.trim().split_once(':') else { continue };
        if !key.eq_ignore_ascii_case("Changelog") {
            continue;
        }
        let text = text.trim();
        if text.eq_ignore_ascii_case("skip") {
            return None;
        }
        if text.is_empty() {
            continue;
        }
        if title.is_none() {
            title = Some(text);
        } else {
            points.push(text);
        }
    }
    Some(Entry { title: title?, points })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_notes_are_shown() {
        let message = "feat(player): add song search (#114)\n\nImplementation details.\n* Fixed lint errors\nChangelog: Find songs by title or artist.\n- Refactor the search index\n  Technical continuation\nChangelog: Search results update as you type.\nCo-Authored-By: Example <example@example.com>\n";
        let entry = parse(message).unwrap();
        assert_eq!(entry.title, "Find songs by title or artist.");
        assert_eq!(entry.points, ["Search results update as you type."]);
    }

    #[test]
    fn unmarked_empty_and_subject_only_notes_are_hidden() {
        for message in [
            "",
            "\nChangelog: No subject",
            "fix(lint): fix warnings\n\n* Fixed lint errors",
            "feat(player): add search\n\nChangelog:   ",
            "Changelog: This is the subject, not a body note",
            "docs(changelog): document the format\n\nUse Changelog: to describe a feature.\n* Changelog: A quoted bullet",
        ] {
            assert!(parse(message).is_none(), "{message:?}");
        }
    }

    #[test]
    fn skips_merges_and_secrets_remain_hidden() {
        for message in [
            "feat(player): add search\n\nChangelog: Search songs\nChangelog: skip",
            "feat(player): add search\n\nChangelog: SKIP\nChangelog: Search songs",
            "Merge pull request #114 from x/y\n\nChangelog: Search songs",
            "feat(mods): add krokodil mode\n\nChangelog: A private mode",
        ] {
            assert!(parse(message).is_none(), "{message:?}");
        }
    }

    #[test]
    fn notes_accept_crlf_unicode_and_case_insensitive_markers() {
        let entry = parse("fix(audio): align hitsounds\r\n\r\nchangelog:   Удары звучат вовремя.  \r\nCHANGELOG: Timing stays consistent.\r\n").unwrap();
        assert_eq!(entry.title, "Удары звучат вовремя.");
        assert_eq!(entry.points, ["Timing stays consistent."]);
    }
}
