//! omp builtin slash commands that make no sense through Coocoo (they need the TUI, act on the
//! host's screen or files only, or pull the session away from what Coocoo manages). They are
//! dropped from the catalog (`agents.commands`) and refused when typed (`agents.prompt`).
//! Skills and custom commands are never touched: omp runs builtins first, so a typed name always
//! means the builtin, but a listed non-builtin entry with the same name stays.

use coocoo_proto::SlashCommand;

struct Hidden {
    /// Command name and its aliases (as omp matches them: case-sensitive).
    names: &'static [&'static str],
    /// Only this subcommand (lowercase, as omp's subcommand parser); `None` = the whole command.
    sub: Option<&'static str>,
    reason: &'static str,
}

const MOVE: &str = "Coocoo manages the agent's directory: after it, the diff, the terminal and resuming would still point at the old one.";
const TUI_ONLY: &str = "It needs omp's terminal UI (editor, clipboard or HUD), which Coocoo does not have.";

const HIDDEN: &[Hidden] = &[
    Hidden { names: &["move"], sub: None, reason: MOVE },
    Hidden {
        names: &["wt", "worktree"],
        sub: None,
        reason: "Coocoo manages the agent's directory: after it, the diff, the terminal and resuming would still point at the old one. For an agent in a worktree use New agent → worktree.",
    },
    Hidden {
        names: &["session"],
        sub: Some("delete"),
        reason: "It deletes the session file Coocoo resumes the agent from. Delete the agent in Coocoo instead.",
    },
    Hidden { names: &["pin"], sub: None, reason: "It pins omp's resume list, which Coocoo does not use." },
    Hidden {
        names: &["trace"],
        sub: None,
        reason: "It starts a dashboard on the host's localhost, which is not reachable from a phone or the LAN.",
    },
    Hidden {
        names: &["stats"],
        sub: None,
        reason: "It starts a dashboard on the host's localhost, which is not reachable from a phone or the LAN.",
    },
    Hidden {
        names: &["browser"],
        sub: None,
        reason: "Coocoo provides the browser (Browser panel); this would switch omp's global browser setting.",
    },
    Hidden {
        names: &["export"],
        sub: None,
        reason: "It only writes an HTML file on the host; there is no download in Coocoo.",
    },
    Hidden { names: &["dump"], sub: None, reason: "It writes the transcript to the host; the chat shows it already." },
    Hidden { names: &["share"], sub: None, reason: "Sharing sessions is turned off in Coocoo." },
    Hidden { names: &["todo"], sub: Some("edit"), reason: TUI_ONLY },
    Hidden { names: &["todo"], sub: Some("copy"), reason: TUI_ONLY },
    Hidden { names: &["todo"], sub: Some("expand"), reason: TUI_ONLY },
    Hidden { names: &["todo"], sub: Some("collapse"), reason: TUI_ONLY },
    Hidden { names: &["advisor"], sub: Some("configure"), reason: TUI_ONLY },
    Hidden { names: &["advisor"], sub: Some("dump"), reason: TUI_ONLY },
];

/// Is `name` (or one of `aliases`) one of `names`?
fn named(entry: &Hidden, name: &str, aliases: &[String]) -> bool {
    entry.names.iter().any(|n| *n == name || aliases.iter().any(|a| a == n))
}

/// The catalog without the hidden builtins and builtin subcommands. A list-style hint
/// (`[info|delete|pin [account]]`) is rebuilt from the remaining subcommands.
pub(crate) fn filter(commands: Vec<SlashCommand>) -> Vec<SlashCommand> {
    commands
        .into_iter()
        .filter_map(|mut c| {
            if c.source != "builtin" {
                return Some(c);
            }
            let hidden: Vec<&Hidden> = HIDDEN.iter().filter(|h| named(h, &c.name, &c.aliases)).collect();
            if hidden.iter().any(|h| h.sub.is_none()) {
                return None;
            }
            let before = c.subcommands.len();
            c.subcommands.retain(|s| !hidden.iter().any(|h| h.sub == Some(s.name.to_lowercase().as_str())));
            if c.subcommands.len() != before
                && let Some(hint) = &c.hint
                && let (Some(open @ ('[' | '<')), true) = (hint.chars().next(), hint.contains('|'))
            {
                let close = if open == '[' { ']' } else { '>' };
                let items: Vec<String> = c
                    .subcommands
                    .iter()
                    .map(|s| match s.usage.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
                        Some(usage) => format!("{} {usage}", s.name),
                        None => s.name.clone(),
                    })
                    .collect();
                c.hint = Some(format!("{open}{}{close}", items.join("|")));
            }
            Some(c)
        })
        .collect()
}

/// A typed message that must not reach omp because it runs a hidden builtin: the command as
/// typed (`/session delete`) and why. Parsed like omp does: the name ends at whitespace or `:`,
/// the subcommand is the next word, lowercased.
pub(crate) fn refused(message: &str) -> Option<(String, &'static str)> {
    let body = message.strip_prefix('/')?;
    let end = body.find(|c: char| c.is_whitespace() || c == ':').unwrap_or(body.len());
    let (name, rest) = (&body[..end], body[end..].strip_prefix(':').unwrap_or(&body[end..]));
    if name.is_empty() {
        return None;
    }
    let sub = rest.split_whitespace().next().map(str::to_lowercase);
    HIDDEN
        .iter()
        .find(|h| h.names.contains(&name) && h.sub.is_none_or(|s| sub.as_deref() == Some(s)))
        .map(|h| (h.sub.map_or_else(|| format!("/{name}"), |s| format!("/{name} {s}")), h.reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use coocoo_proto::SlashSubcommand;

    fn cmd(name: &str, source: &str, aliases: &[&str], hint: Option<&str>, subs: &[(&str, Option<&str>)]) -> SlashCommand {
        SlashCommand {
            name: name.into(),
            source: source.into(),
            aliases: aliases.iter().map(|a| (*a).into()).collect(),
            description: None,
            hint: hint.map(Into::into),
            subcommands: subs.iter().map(|(n, u)| SlashSubcommand { name: (*n).into(), description: None, usage: u.map(Into::into) }).collect(),
        }
    }

    #[test]
    fn catalog_drops_hidden_builtins_and_subcommands() {
        let out = filter(vec![
            cmd("move", "builtin", &[], Some("[<path>]"), &[]),
            cmd("wt", "builtin", &["worktree"], None, &[]),
            cmd("session", "builtin", &[], Some("[info|delete|pin [account]]"), &[("info", None), ("delete", None), ("pin", Some("[account]"))]),
            cmd("todo", "builtin", &[], Some("<subcommand>"), &[("edit", None), ("append", Some("[<phase>] <task...>"))]),
            cmd("move", "skill", &[], Some("arguments"), &[]),
            cmd("fast", "builtin", &[], Some("[on|off]"), &[("on", None), ("off", None)]),
        ]);
        let names: Vec<(&str, &str)> = out.iter().map(|c| (c.name.as_str(), c.source.as_str())).collect();
        assert_eq!(names, [("session", "builtin"), ("todo", "builtin"), ("move", "skill"), ("fast", "builtin")]);
        let session = &out[0];
        assert_eq!(session.subcommands.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["info", "pin"]);
        assert_eq!(session.hint.as_deref(), Some("[info|pin [account]]"));
        // A placeholder hint is not a list of the subcommands: it stays.
        assert_eq!(out[1].hint.as_deref(), Some("<subcommand>"));
        assert_eq!(out[3].hint.as_deref(), Some("[on|off]"));
    }

    #[test]
    fn typed_hidden_commands_are_refused() {
        for text in ["/wt feature", "/worktree", "/session delete", "/session  DELETE", "/move /tmp", "/todo edit", "/session:delete"] {
            assert!(refused(text).is_some(), "{text}");
        }
        assert_eq!(refused("/session DELETE now").map(|(c, _)| c).as_deref(), Some("/session delete"));
        assert_eq!(refused("/wt feature").map(|(c, _)| c).as_deref(), Some("/wt"));
        for text in ["/session info", "/session", "/moved", "/MOVE", "/todo append x", "move", "/", "/fast on"] {
            assert!(refused(text).is_none(), "{text}");
        }
    }
}
