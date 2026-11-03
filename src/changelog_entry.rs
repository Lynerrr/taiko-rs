//! Player-facing notes shared by the game and the Discord bot, including legacy history.

pub struct Entry {
    pub title: String,
    pub points: Vec<String>,
}

/// Conventional Commits require explicit notes; older subjects keep their title and bullets.
/// Explicit notes always take precedence, and `Changelog: skip` hides either format.
pub fn parse(message: &str) -> Option<Entry> {
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
            title = Some(text.to_string());
        } else {
            points.push(text.to_string());
        }
    }
    if let Some(title) = title {
        return Some(Entry { title, points });
    }
    if conventional(subject) {
        return None;
    }
    let mut points: Vec<String> = Vec::new();
    for line in message.lines().skip(1) {
        let lower = line.to_ascii_lowercase();
        if ["co-authored-by:", "claude-session:", "signed-off-by:"].iter().any(|key| lower.starts_with(key)) {
            continue;
        }
        if let Some(point) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            points.push(point.trim().to_string());
        } else if line.starts_with("  ") && !line.trim().is_empty()
            && let Some(last) = points.last_mut()
        {
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    Some(Entry { title: subject.to_string(), points })
}

/// Recognize only the project's known types, never arbitrary prefixes such as `Windows:`.
fn conventional(subject: &str) -> bool {
    let Some((header, description)) = subject.split_once(": ") else { return false };
    if description.trim().is_empty() {
        return false;
    }
    let header = header.strip_suffix('!').unwrap_or(header);
    let kind = if let Some((kind, scope)) = header.split_once('(') {
        let Some(scope) = scope.strip_suffix(')') else { return false };
        if scope.trim().is_empty() || scope.contains(['(', ')']) {
            return false;
        }
        kind
    } else {
        header
    };
    matches!(kind, "feat" | "fix" | "refactor" | "perf" | "test" | "docs" | "build" | "ci" | "chore")
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
            "docs(changelog): document the format\n\nUse Changelog: to describe a feature.\n* Changelog: A quoted bullet",
        ] {
            assert!(parse(message).is_none(), "{message:?}");
        }
    }

    #[test]
    fn legacy_titles_and_bullets_keep_their_history() {
        let title = "Windows: the offset no longer drifts the longer the game stays open.";
        let entry = parse(&format!("{title}\n\n- Hits stay in sync\n  during long sessions\n* Timing is stable\n\nCo-Authored-By: Someone <x@y>\nSigned-off-by: Someone\nClaude-Session: https://example.com\n")).unwrap();
        assert_eq!(entry.title, title);
        assert_eq!(entry.points, ["Hits stay in sync during long sessions", "Timing is stable"]);
        assert_eq!(parse("A new mod").unwrap().title, "A new mod");
        assert!(parse("Windows: a private change\n\nChangelog: skip").is_none());
        let marked = parse("Windows: technical subject\n\nChangelog: Player-facing text\n* Fixed lint errors").unwrap();
        assert_eq!(marked.title, "Player-facing text");
        assert!(marked.points.is_empty());
    }

    #[test]
    fn only_known_conventional_types_require_explicit_notes() {
        for kind in ["feat", "fix", "refactor", "perf", "test", "docs", "build", "ci", "chore"] {
            for subject in [format!("{kind}(scope): change"), format!("{kind}: change"), format!("{kind}(scope)!: change"), format!("{kind}!: change")] {
                assert!(parse(&format!("{subject}\n\n* Fixed lint errors")).is_none(), "{subject}");
                assert_eq!(parse(&format!("{subject}\n\nChangelog: Player-facing text")).unwrap().title, "Player-facing text");
            }
        }
        for subject in ["Windows: fix audio", "Audio: fix timing", "unknown(scope): old title", "fix(): old malformed title", "fix(scope: old malformed title"] {
            assert_eq!(parse(subject).unwrap().title, subject);
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
