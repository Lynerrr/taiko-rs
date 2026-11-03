//! Admins' chat commands ("/ban troll", "/mute troll 30m"): typed in any chat box of the F9
//! panel or in its ADMIN tab, sent as `Up::Admin` (the server checks every one).

use crate::online::AdminOp;

/// What a command asks for.
#[derive(Debug, PartialEq)]
pub enum Command {
    Op(AdminOp),
    /// Watch anyone's solo play, without them being told.
    Watch(String),
    Help,
}

/// The commands, for /help.
pub const HELP: &str = "/ban /unban /kick /mute name 30m|2h|1d /unmute /rename old new /nominator name on|off /rmpfp /purge /lookup /watch /announce text /delplay id /pp id value|reset /roomkick room name /closeroom room";

/// Whether `text` is meant as an admin command (else it's a chat line, `/np` included).
pub fn is_command(text: &str) -> bool {
    let word = text.split_whitespace().next().unwrap_or("");
    matches!(
        word.to_ascii_lowercase().as_str(),
        "/ban" | "/unban" | "/kick" | "/mute" | "/unmute" | "/rename" | "/nominator" | "/rmpfp" | "/purge" | "/lookup" | "/watch" | "/announce" | "/delplay" | "/pp" | "/roomkick" | "/closeroom" | "/overview" | "/help"
    )
}

/// A mute's length: "30m", "2h", "1d", or plain minutes.
fn minutes(s: &str) -> Result<u32, String> {
    let s = s.trim().to_ascii_lowercase();
    let (n, unit) = match s.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => (&s[..i], &s[i..]),
        None => (s.as_str(), "m"),
    };
    let n: u32 = n.parse().map_err(|_| format!("how long is \"{s}\"? say 30m, 2h or 1d"))?;
    let per = match unit {
        "m" | "min" => 1,
        "h" => 60,
        "d" => 1440,
        _ => return Err(format!("how long is \"{s}\"? say 30m, 2h or 1d")),
    };
    Ok(n.saturating_mul(per))
}

/// `text` as a command.
pub fn parse(text: &str) -> Result<Command, String> {
    let mut words = text.split_whitespace();
    let cmd = words.next().unwrap_or("").to_ascii_lowercase();
    let args: Vec<&str> = words.collect();
    let need = |n: usize, usage: &str| if args.len() < n { Err(format!("usage: {usage}")) } else { Ok(()) };
    let name = |i: usize| args[i].to_string();
    let id = |i: usize| args[i].parse::<u64>().map_err(|_| format!("\"{}\" isn't a play's number", args[i]));
    let op = |o: AdminOp| Ok(Command::Op(o));
    match cmd.as_str() {
        "/help" => Ok(Command::Help),
        "/overview" => op(AdminOp::Overview),
        "/ban" | "/unban" => {
            need(1, &format!("{cmd} name"))?;
            op(AdminOp::Ban { name: name(0), on: cmd == "/ban" })
        }
        "/kick" => {
            need(1, "/kick name")?;
            op(AdminOp::Kick { name: name(0) })
        }
        "/mute" => {
            need(1, "/mute name [30m|2h|1d]")?;
            let m = if args.len() > 1 { minutes(args[1])? } else { 30 };
            if m == 0 {
                return Err("a mute lasts at least a minute (/unmute lets them back)".into());
            }
            op(AdminOp::Mute { name: name(0), minutes: m })
        }
        "/unmute" => {
            need(1, "/unmute name")?;
            op(AdminOp::Mute { name: name(0), minutes: 0 })
        }
        "/rename" => {
            need(2, "/rename name new_name")?;
            op(AdminOp::Rename { name: name(0), new: name(1) })
        }
        "/nominator" => {
            need(1, "/nominator name [on|off]")?;
            let on = !matches!(args.get(1).map(|s| s.to_ascii_lowercase()).as_deref(), Some("off" | "0" | "no"));
            op(AdminOp::Nominator { name: name(0), on })
        }
        "/rmpfp" => {
            need(1, "/rmpfp name")?;
            op(AdminOp::RemoveAvatar { name: name(0) })
        }
        "/purge" => {
            need(1, "/purge name")?;
            op(AdminOp::PurgeGlobal { name: name(0) })
        }
        "/lookup" => {
            need(1, "/lookup name")?;
            op(AdminOp::Lookup { name: name(0) })
        }
        "/watch" => {
            need(1, "/watch name")?;
            Ok(Command::Watch(name(0)))
        }
        "/announce" => {
            let rest = text.trim_start().get(cmd.len()..).unwrap_or("").trim();
            if rest.is_empty() {
                return Err("usage: /announce text".into());
            }
            op(AdminOp::Announce(rest.to_string()))
        }
        "/delplay" => {
            need(1, "/delplay id")?;
            op(AdminOp::DeletePlay { id: id(0)? })
        }
        "/pp" => {
            need(2, "/pp id value|reset")?;
            let pp = if args[1].eq_ignore_ascii_case("reset") { None } else { Some(args[1].parse::<f64>().map_err(|_| "pp is a number, or reset".to_string())?) };
            op(AdminOp::SetPp { id: id(0)?, pp })
        }
        "/roomkick" => {
            need(2, "/roomkick room name")?;
            let room = args[0].parse().map_err(|_| format!("\"{}\" isn't a lobby's number", args[0]))?;
            op(AdminOp::KickFromRoom { room, name: name(1) })
        }
        "/closeroom" => {
            need(1, "/closeroom room")?;
            let room = args[0].parse().map_err(|_| format!("\"{}\" isn't a lobby's number", args[0]))?;
            op(AdminOp::CloseRoom { room })
        }
        _ => Err(format!("no such command: {cmd} (/help lists them)")),
    }
}

/// How long ago, or how long for, in a few characters: "3h 12m", "2d 4h", "45s".
pub fn span(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h {}m", secs / 3600, secs / 60 % 60),
        _ => format!("{}d {}h", secs / 86400, secs / 3600 % 24),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_become_admin_ops() {
        assert_eq!(parse("/ban troll"), Ok(Command::Op(AdminOp::Ban { name: "troll".into(), on: true })));
        assert_eq!(parse("/UNBAN troll"), Ok(Command::Op(AdminOp::Ban { name: "troll".into(), on: false })));
        assert_eq!(parse("/mute troll 2h"), Ok(Command::Op(AdminOp::Mute { name: "troll".into(), minutes: 120 })));
        assert_eq!(parse("/mute troll"), Ok(Command::Op(AdminOp::Mute { name: "troll".into(), minutes: 30 })));
        assert_eq!(parse("/mute troll 1d"), Ok(Command::Op(AdminOp::Mute { name: "troll".into(), minutes: 1440 })));
        assert_eq!(parse("/unmute troll"), Ok(Command::Op(AdminOp::Mute { name: "troll".into(), minutes: 0 })));
        assert_eq!(parse("/nominator ann off"), Ok(Command::Op(AdminOp::Nominator { name: "ann".into(), on: false })));
        assert_eq!(parse("/announce  restart in 5 minutes!"), Ok(Command::Op(AdminOp::Announce("restart in 5 minutes!".into()))));
        assert_eq!(parse("/pp 12 reset"), Ok(Command::Op(AdminOp::SetPp { id: 12, pp: None })));
        assert_eq!(parse("/pp 12 345.5"), Ok(Command::Op(AdminOp::SetPp { id: 12, pp: Some(345.5) })));
        assert_eq!(parse("/roomkick 3 troll"), Ok(Command::Op(AdminOp::KickFromRoom { room: 3, name: "troll".into() })));
        assert_eq!(parse("/watch bob"), Ok(Command::Watch("bob".into())));
        assert!(parse("/ban").unwrap_err().starts_with("usage"));
        assert!(parse("/mute troll forever").is_err());
        assert!(parse("/delplay x").is_err());
        assert!(parse("/frobnicate").is_err());
    }

    #[test]
    fn only_admin_commands_are_taken_from_chat() {
        assert!(is_command("/ban troll") && is_command("  /help"));
        assert!(!is_command("/np") && !is_command("hello /ban") && !is_command(""));
    }

    #[test]
    fn spans_are_short() {
        assert_eq!(span(42), "42s");
        assert_eq!(span(5 * 60), "5m");
        assert_eq!(span(3 * 3600 + 12 * 60), "3h 12m");
        assert_eq!(span(2 * 86400 + 4 * 3600), "2d 4h");
    }
}
