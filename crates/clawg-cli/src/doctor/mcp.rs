//! Whether Claude Code would start this profile's server: read from the
//! `.claude.json` `clawg setup claude-code` reads, never written.

use std::path::Path;

use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;
use serde_json::{json, Value};

use super::finding::{Finding, Severity};
use super::identity;

/// `.claude.json` as it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Config {
    Missing,
    Unreadable(String),
    Text(String),
}

/// The finding for the profile's server in `config`, read from `path`.
/// `exists` says whether an absolute command is there.
pub fn mcp(lang: Lang, path: &Path, config: &Config, profile: Profile, exe: &Path, exists: impl Fn(&Path) -> bool) -> Vec<Finding> {
    let name = profile.mcp_server_name();
    let setup = format!("clawg setup claude-code --profile {}", profile.as_str());
    let unreadable = |error: String| {
        vec![Finding::new(
            "mcp",
            Severity::Warn,
            "config_unreadable",
            match lang {
                Lang::En => format!("{} cannot be read: {error}", path.display()),
                Lang::Es => format!("no se puede leer {}: {error}", path.display()),
            },
        )
        .evidence(json!({ "config": path, "error": error }))]
    };
    let not_registered = || {
        vec![Finding::new(
            "mcp",
            Severity::Info,
            "not_registered",
            match lang {
                Lang::En => format!("Claude Code has no `{name}` server for this profile"),
                Lang::Es => format!("Claude Code no tiene un servidor `{name}` para este perfil"),
            },
        )
        .evidence(json!({ "config": path, "server": name }))
        .step(setup.clone())]
    };
    let text = match config {
        Config::Missing => return not_registered(),
        Config::Unreadable(error) => return unreadable(error.clone()),
        Config::Text(text) => text,
    };
    let parsed: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return unreadable(format!("not valid JSON ({e})")),
    };
    let Some(server) = parsed.get("mcpServers").and_then(|m| m.get(name)) else {
        return not_registered();
    };
    let Some(command) = server.get("command").and_then(Value::as_str) else {
        return unreadable(format!("the `{name}` server has no command"));
    };
    let args: Vec<&str> =
        server.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let evidence = json!({ "config": path, "server": name, "command": command, "args": args });

    let mut found = Vec::new();
    let command_path = Path::new(command);
    if command_path.is_absolute() {
        if !exists(command_path) {
            found.push(
                Finding::new(
                    "mcp",
                    Severity::Error,
                    "binary_missing",
                    match lang {
                        Lang::En => format!("Claude Code starts `{name}` with {command}, which does not exist"),
                        Lang::Es => format!("Claude Code arranca `{name}` con {command}, que no existe"),
                    },
                )
                .evidence(evidence.clone())
                .step(setup.clone()),
            );
        } else if identity(command_path) != identity(exe) {
            found.push(
                Finding::new(
                    "mcp",
                    Severity::Warn,
                    "other_binary",
                    match lang {
                        Lang::En => format!("Claude Code starts `{name}` with {command}, not with this binary"),
                        Lang::Es => format!("Claude Code arranca `{name}` con {command}, no con este binario"),
                    },
                )
                .evidence(json!({ "config": path, "server": name, "command": command, "exe": exe })),
            );
        }
    }
    if !args.contains(&"--agent") {
        found.push(
            Finding::new(
                "mcp",
                Severity::Warn,
                "no_agent",
                match lang {
                    Lang::En => {
                        format!("`{name}` is started without --agent, so its memories go under whatever name the client reports")
                    }
                    Lang::Es => format!(
                        "`{name}` arranca sin --agent, así que sus memorias quedan bajo el nombre que dé el cliente"
                    ),
                },
            )
            .evidence(evidence.clone())
            .step(setup),
        );
    }
    if found.is_empty() {
        let message = match lang {
            Lang::En => format!("Claude Code starts `{name}` with {command} and --agent"),
            Lang::Es => format!("Claude Code arranca `{name}` con {command} y --agent"),
        };
        found.push(Finding::ok("mcp", message).evidence(evidence));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn exe(dir: &str) -> PathBuf {
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        PathBuf::from(root).join(dir).join(if cfg!(windows) { "clawg.exe" } else { "clawg" })
    }

    fn config(command: &Path, args: &[&str]) -> Config {
        Config::Text(
            json!({ "mcpServers": { "clawg-dev": { "type": "stdio", "command": command, "args": args } } }).to_string(),
        )
    }

    fn kinds(found: &[Finding]) -> Vec<(Severity, &'static str)> {
        found.iter().map(|f| (f.severity, f.reason_code)).collect()
    }

    const AGENT: [&str; 5] = ["mcp", "--profile", "dev", "--agent", "claude-code"];

    #[test]
    fn mcp_findings() {
        let here = exe("bin");
        let file = Path::new("/home/u/.claude.json");
        let check = |config: &Config| mcp(Lang::En, file, config, Profile::Dev, &here, |p| p == here.as_path() || p == exe("old"));

        assert_eq!(kinds(&check(&config(&here, &AGENT))), [(Severity::Ok, "healthy")]);
        let missing = check(&Config::Missing);
        assert_eq!(kinds(&missing), [(Severity::Info, "not_registered")]);
        assert_eq!(missing[0].safe_next_step.as_deref(), Some("clawg setup claude-code --profile dev"));
        assert_eq!(
            kinds(&check(&Config::Text(r#"{"mcpServers":{"clawg":{"command":"x"}}}"#.into()))),
            [(Severity::Info, "not_registered")],
            "the other profile's server is not this one"
        );
        assert_eq!(kinds(&check(&Config::Text("{".into()))), [(Severity::Warn, "config_unreadable")]);
        assert_eq!(kinds(&check(&Config::Unreadable("denied".into()))), [(Severity::Warn, "config_unreadable")]);
        assert_eq!(kinds(&check(&config(&exe("gone"), &AGENT))), [(Severity::Error, "binary_missing")]);
        assert_eq!(kinds(&check(&config(&exe("old"), &AGENT))), [(Severity::Warn, "other_binary")]);
        assert_eq!(
            kinds(&check(&config(&here, &["mcp", "--profile", "dev"]))),
            [(Severity::Warn, "no_agent")]
        );
        assert_eq!(
            kinds(&check(&config(Path::new("clawg"), &AGENT))),
            [(Severity::Ok, "healthy")],
            "a command by name is the PATH check's"
        );
    }

    #[test]
    fn the_mcp_findings_are_the_same_in_both_languages_but_their_message() {
        let here = exe("bin");
        let file = Path::new("/home/u/.claude.json");
        let configs = [
            config(&here, &AGENT),
            Config::Missing,
            Config::Text("{".into()),
            Config::Text(r#"{"mcpServers":{"clawg-dev":{}}}"#.into()),
            Config::Unreadable("denied".into()),
            config(&exe("gone"), &AGENT),
            config(&exe("old"), &["mcp"]),
        ];
        for config in &configs {
            let check = |lang| mcp(lang, file, config, Profile::Dev, &here, |p| p == here.as_path() || p == exe("old"));
            let (en, es) = (check(Lang::En), check(Lang::Es));
            assert_eq!(en.len(), es.len());
            for (en, es) in en.iter().zip(&es) {
                assert_ne!(en.message, es.message, "{} has one message in both languages", en.reason_code);
                assert_eq!(Finding { message: String::new(), ..en.clone() }, Finding { message: String::new(), ..es.clone() });
            }
        }
    }
}
