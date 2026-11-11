//! The seeded memories that are rules, as the arms that deliver rules write
//! them: one line each with its age, at the start of the session through a
//! hook of the harness, or in a rules file the client loads when the agent
//! works on a file its globs match. Pure: it builds the texts.

use clawg_core::preferences::Lang;

use crate::task::SeedMemory;

/// What heads every set of rules: where they come from, and that an old one
/// may no longer hold, which the age on each line lets the agent weigh.
fn header(lang: Option<Lang>) -> &'static str {
    match lang {
        Some(Lang::Es) => "Reglas de la memoria de clawg, cada una con su antigüedad. Una regla vieja puede estar desactualizada.",
        _ => "Rules from clawg's memory, each with its age. An old rule can be out of date.",
    }
}

fn age(days: u32, lang: Option<Lang>) -> String {
    match (lang, days) {
        (Some(Lang::Es), 0) => "hoy".to_owned(),
        (Some(Lang::Es), 1) => "hace 1 día".to_owned(),
        (Some(Lang::Es), n) => format!("hace {n} días"),
        (_, 0) => "today".to_owned(),
        (_, 1) => "1 day ago".to_owned(),
        (_, n) => format!("{n} days ago"),
    }
}

/// `memory` on one line: its age, then its subject line and every other
/// line of its text, joined with ` — `.
pub fn line(memory: &SeedMemory, lang: Option<Lang>) -> String {
    let parts: Vec<&str> = memory.content.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    format!("- [{}] {}", age(memory.age_days, lang), parts.join(" — "))
}

fn block(rules: &[&SeedMemory], lang: Option<Lang>) -> String {
    let mut out = format!("{}\n", header(lang));
    for rule in rules {
        out.push_str(&line(rule, lang));
        out.push('\n');
    }
    out
}

/// What the startup hook prints: every rule of `memories` or, when the rules
/// with globs go `by_file`, only those without; `None` when there is none.
pub fn startup(memories: &[SeedMemory], by_file: bool, lang: Option<Lang>) -> Option<String> {
    let rules: Vec<&SeedMemory> = memories.iter().filter(|m| m.rule.as_ref().is_some_and(|r| !by_file || r.paths.is_empty())).collect();
    (!rules.is_empty()).then(|| block(&rules, lang))
}

/// The rules files of the rules with globs, by name: one per set of globs,
/// in the order the sets first appear, with the globs in its `paths:`.
pub fn files(memories: &[SeedMemory], lang: Option<Lang>) -> Vec<(String, String)> {
    let mut sets: Vec<(&[String], Vec<&SeedMemory>)> = Vec::new();
    for memory in memories {
        let Some(rule) = memory.rule.as_ref().filter(|r| !r.paths.is_empty()) else { continue };
        match sets.iter_mut().find(|(paths, _)| *paths == rule.paths.as_slice()) {
            Some((_, rules)) => rules.push(memory),
            None => sets.push((&rule.paths, vec![memory])),
        }
    }
    sets.into_iter()
        .enumerate()
        .map(|(i, (paths, rules))| {
            let mut body = String::from("---\npaths:\n");
            for path in paths {
                // A JSON string is a double-quoted YAML scalar, and a glob that
                // starts with `*` must be quoted in YAML.
                body.push_str(&format!("  - {}\n", serde_json::Value::from(path.as_str())));
            }
            body.push_str("---\n");
            body.push_str(&block(&rules, lang));
            (format!("rules-{}.md", i + 1), body)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Rule;

    fn memory(content: &str, age_days: u32, rule: Option<&[&str]>) -> SeedMemory {
        SeedMemory {
            content: content.into(),
            memory_type: "architectural_decision".into(),
            importance: 5.0,
            age_days,
            access_count: 1,
            topic_key: None,
            user_stated: false,
            rule: rule.map(|paths| Rule { paths: paths.iter().map(|p| (*p).to_owned()).collect() }),
        }
    }

    #[test]
    fn a_rule_is_one_line_with_its_age() {
        let m = memory("Lists use an asterisk\n  Chosen because a hyphen reads as negative.\n\nThe ADR is 0011.\n", 120, Some(&[]));
        assert_eq!(line(&m, None), "- [120 days ago] Lists use an asterisk — Chosen because a hyphen reads as negative. — The ADR is 0011.");
        assert_eq!(line(&m, Some(Lang::Es)), "- [hace 120 días] Lists use an asterisk — Chosen because a hyphen reads as negative. — The ADR is 0011.");
        assert_eq!(line(&memory("Today", 0, None), Some(Lang::En)), "- [today] Today");
        assert_eq!(line(&memory("Yesterday", 1, None), None), "- [1 day ago] Yesterday");
        assert_eq!(line(&memory("Ayer", 1, None), Some(Lang::Es)), "- [hace 1 día] Ayer");
    }

    #[test]
    fn the_startup_holds_every_rule_or_only_those_without_paths() {
        let memories = [memory("Always\nEvery session.", 10, Some(&[])), memory("Docs\nFor Markdown.", 20, Some(&["**/*.md"])), memory("A fact\nNot a rule.", 30, None)];
        let all = startup(&memories, false, None).unwrap();
        assert_eq!(all, format!("{}\n- [10 days ago] Always — Every session.\n- [20 days ago] Docs — For Markdown.\n", header(None)));
        assert!(header(None).contains("out of date"));
        let at_start = startup(&memories, true, Some(Lang::Es)).unwrap();
        assert_eq!(at_start, format!("{}\n- [hace 10 días] Always — Every session.\n", header(Some(Lang::Es))));
        assert!(header(Some(Lang::Es)).contains("desactualizada"));
        assert_eq!(startup(&memories[1..], true, None), None, "a rule with globs goes by file");
        assert_eq!(startup(&memories[2..], false, None), None, "no rule, no startup");
    }

    #[test]
    fn the_rules_by_file_share_a_file_by_their_globs() {
        let memories = [
            memory("Docs\nFor Markdown.", 20, Some(&["**/*.md"])),
            memory("Always\nEvery session.", 10, Some(&[])),
            memory("Pins\nFor requirements.", 30, Some(&["requirements*.txt", "a \"b\".txt"])),
            memory("Headings\nSentence case.", 40, Some(&["**/*.md"])),
            memory("A fact", 50, None),
        ];
        let by_file = files(&memories, None);
        assert_eq!(by_file.len(), 2);
        assert_eq!(
            by_file[0],
            (
                "rules-1.md".to_owned(),
                format!("---\npaths:\n  - \"**/*.md\"\n---\n{}\n- [20 days ago] Docs — For Markdown.\n- [40 days ago] Headings — Sentence case.\n", header(None))
            )
        );
        assert_eq!(by_file[1].0, "rules-2.md");
        assert!(by_file[1].1.starts_with("---\npaths:\n  - \"requirements*.txt\"\n  - \"a \\\"b\\\".txt\"\n---\n"), "{}", by_file[1].1);
        assert!(files(&memories[1..2], None).is_empty(), "a rule of every session has no file");
    }
}
