//! What a call to a shell tool says about the programs the machine does not
//! have: the shell prints `X: command not found`, and clawg reads it from the
//! tool's output without the agent judging anything.
//!
//! What was learned is kept in a ledger, `<data>/tool-failures.jsonl`, written
//! the way the record of `session-start` is: one write per call, renamed and
//! not rewritten when it grows, and never the cause of a failure of the hook.
//! A line holds the program, the tool and the session, and nothing of the call.
//!
//! What is decided is pure: it takes the texts, the lines and the time it is
//! given, so no file, port or process is needed to test it. Only [`read`],
//! [`append`] and [`on_path`] touch the system.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clawg_core::memory::redact::redact;
use serde::{Deserialize, Serialize};

use crate::hook_log;

/// Which of the client's two events after a tool the call came with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    /// The tool ended well as far as the client could tell. A pipe or a `;`
    /// can end a command with code 0 after a program was missing.
    Success,
    Failure,
}

/// What a call showed about one program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The shell said the program is not there.
    Fail,
    /// The program was called and the call showed no sign of it missing.
    Ok,
}

/// One program a call showed something about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub kind: Kind,
    pub program: String,
}

/// The only tool whose calls are read.
const SHELL_TOOL: &str = "Bash";

/// The most characters a program name has.
const NAME_CHARS: usize = 32;

/// The ledger, in the data directory.
pub const FILE: &str = "tool-failures.jsonl";
/// What the ledger becomes when it reaches [`hook_log::ROLL_AT`].
pub const ROLLED: &str = "tool-failures.1.jsonl";

/// The most characters of a session id a line keeps.
const SESSION_CHARS: usize = 64;

/// How far back a line counts, in milliseconds. It is also how long a rule
/// lasts: thirty days after the last failure it is forgotten.
pub const WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// The most rules there are at a time.
pub const MAX_RULES: usize = 5;

/// Sessions with a failure that make a rule.
const SESSIONS_FOR_A_RULE: usize = 2;

/// Words that run what follows them: the program is the next word.
const PREFIXES: [&str; 6] = ["sudo", "time", "env", "command", "exec", "xargs"];

/// Every text a JSON value holds, at any depth: the client does not type the
/// shape of a shell tool's response, so none is assumed.
pub fn texts(value: &serde_json::Value) -> Vec<&str> {
    match value {
        serde_json::Value::String(text) => vec![text.as_str()],
        serde_json::Value::Array(items) => items.iter().flat_map(texts).collect(),
        serde_json::Value::Object(fields) => fields.values().flat_map(texts).collect(),
        _ => Vec::new(),
    }
}

/// The programs a text says are missing, in order and once each, as the shell
/// wrote them and before any check of the name: `bash` and `sh` end a line in
/// `: command not found` (with `line N: ` before the name or not), `dash` says
/// `sh: N: X: not found`, and `zsh` says `zsh: command not found: X`.
pub fn missing_programs(text: &str) -> Vec<&str> {
    if !text.contains("not found") {
        return Vec::new();
    }
    let mut found: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(program) = missing_in(line) {
            if !found.contains(&program) {
                found.push(program);
            }
        }
    }
    found
}

fn missing_in(line: &str) -> Option<&str> {
    if let Some(head) = line.strip_suffix(": command not found") {
        return Some(head.rsplit_once(": ").map_or(head, |(_, program)| program));
    }
    if let Some(program) = line.strip_prefix("zsh: command not found: ") {
        return Some(program);
    }
    let head = line.strip_suffix(": not found")?;
    let (head, program) = head.rsplit_once(": ")?;
    let (shell, number) = head.rsplit_once(": ")?;
    (shell.ends_with("sh") && !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then_some(program)
}

/// Whether `name` can be the name of a program the ledger keeps: its shape,
/// and nothing the secret detector finds in it.
pub fn is_program_name(name: &str) -> bool {
    has_program_shape(name) && redact(name).kinds.is_empty()
}

fn has_program_shape(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
        && name.len() <= NAME_CHARS
}

/// The first word of each segment of `command`, which a `|`, `||`, `&&`, `;`,
/// `&`, a new line or a `(` starts, past assignments and the words that run
/// the next one. The command is not read further than this, and a mistake
/// here can only cost an observation, never invent one.
pub fn command_words(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut words = Vec::new();
    let mut start = 0;
    for (at, c) in command.char_indices() {
        let separator = match c {
            '|' | ';' | '\n' | '(' => true,
            // `2>&1` and `&>` redirect: they do not start a command.
            '&' => !(at > 0 && matches!(bytes[at - 1], b'>' | b'<')) && bytes.get(at + 1) != Some(&b'>'),
            _ => false,
        };
        if separator {
            words.extend(first_word(&command[start..at]));
            start = at + 1;
        }
    }
    words.extend(first_word(&command[start..]));
    words
}

fn first_word(segment: &str) -> Option<&str> {
    let word = segment.split_whitespace().find(|word| !is_assignment(word) && !PREFIXES.contains(word))?;
    let word = word.trim_matches(['"', '\'', '(', ')']);
    (!word.is_empty()).then_some(word)
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

/// What a call to `tool` that ran `command` showed, from the `outputs` the
/// client gave: the `error` after a failure, or every text of the response
/// after a call that ended well.
///
/// A failure is a program the shell said is missing that is also a word of the
/// command: a script the command launched can miss its own dependencies, and
/// that is not the machine's. A success is a program the command called when
/// the call showed no missing program at all and the command does not hide its
/// own errors, with a `2>`, a `&>` or a `||`: a success erases what was
/// learned, so it asks for more than a failure does.
pub fn observe(tool: &str, command: &str, outputs: &[&str], after: After) -> Vec<Observation> {
    if tool != SHELL_TOOL {
        return Vec::new();
    }
    let words = command_words(command);
    let signatures: Vec<&str> = outputs.iter().flat_map(|text| missing_programs(text)).collect();
    let mut seen: Vec<&str> = Vec::new();
    let mut observed = Vec::new();
    for program in &signatures {
        if words.contains(program) && is_program_name(program) && !seen.contains(program) {
            seen.push(program);
            observed.push(Observation { kind: Kind::Fail, program: (*program).to_string() });
        }
    }
    let doubtful = !signatures.is_empty() || outputs.iter().any(|text| text.contains("is not recognized"));
    let hides_errors = ["2>", "&>", "||"].iter().any(|mark| command.contains(mark));
    if after == After::Success && !doubtful && !hides_errors {
        for word in words {
            if has_program_shape(word) && !seen.contains(&word) {
                seen.push(word);
                observed.push(Observation { kind: Kind::Ok, program: word.to_string() });
            }
        }
    }
    observed
}

/// One line of the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub at_ms: i64,
    pub event: Kind,
    pub tool: String,
    pub program: String,
    pub session: String,
}

/// The facts of a call that every line it leaves shares.
#[derive(Debug, Clone, Copy)]
pub struct Call<'a> {
    pub at_ms: i64,
    pub tool: &'a str,
    pub session: &'a str,
}

/// The lines in the bytes of a ledger, oldest first. A line that is not a line
/// of the ledger is skipped, and so is one with a byte that is not UTF-8.
pub fn parse_lines(bytes: &[u8]) -> Vec<Line> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// `lines` as the text that is appended: one JSON object each, built whole so
/// that the write is a single one.
pub fn render(lines: &[Line]) -> String {
    lines.iter().map(|line| serde_json::to_string(line).expect("a line serializes") + "\n").collect()
}

/// The lines of `call` worth keeping, given what the ledger already holds. A
/// failure is always kept. A success only when the newest line of its program
/// is a failure, so the ledger grows with the failures and not with the calls
/// that work, and a program that never failed never enters it.
pub fn to_write(call: &Call, observations: &[Observation], ledger: &[Line]) -> Vec<Line> {
    observations
        .iter()
        .filter(|o| o.kind == Kind::Fail || newest(ledger, call.tool, &o.program) == Some(Kind::Fail))
        .map(|o| Line {
            at_ms: call.at_ms,
            event: o.kind,
            tool: call.tool.to_string(),
            program: o.program.clone(),
            session: hook_log::cut(call.session, SESSION_CHARS),
        })
        .collect()
}

fn newest(ledger: &[Line], tool: &str, program: &str) -> Option<Kind> {
    ledger.iter().rev().find(|line| line.tool == tool && line.program == program).map(|line| line.event)
}

/// A program the machine has been seen to lack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub tool: String,
    pub program: String,
    /// The different sessions in which it failed.
    pub sessions: usize,
    pub last_failure_ms: i64,
}

#[derive(Default)]
struct Tally<'a> {
    sessions: Vec<&'a str>,
    last_failure_ms: i64,
    worked: bool,
}

/// The rules the `lines` give at `now_ms`, newest failure first and at most
/// [`MAX_RULES`]. A program is a rule when, in the [`WINDOW_MS`] before
/// `now_ms`, it failed in two sessions or more, never worked, and `on_path`
/// does not find it: a success says it can run, and being on the path says it
/// can now, whatever the agent last tried.
pub fn rules(lines: &[Line], now_ms: i64, on_path: impl Fn(&str) -> bool) -> Vec<Rule> {
    let since = now_ms.saturating_sub(WINDOW_MS);
    let mut tallies: BTreeMap<(&str, &str), Tally> = BTreeMap::new();
    for line in lines.iter().filter(|line| line.at_ms >= since) {
        let tally = tallies.entry((&line.tool, &line.program)).or_default();
        match line.event {
            Kind::Ok => tally.worked = true,
            Kind::Fail => {
                if !tally.sessions.contains(&line.session.as_str()) {
                    tally.sessions.push(&line.session);
                }
                tally.last_failure_ms = tally.last_failure_ms.max(line.at_ms);
            }
        }
    }
    let mut candidates: Vec<Rule> = tallies
        .into_iter()
        .filter(|(_, tally)| !tally.worked && tally.sessions.len() >= SESSIONS_FOR_A_RULE)
        .map(|((tool, program), tally)| Rule {
            tool: tool.to_string(),
            program: program.to_string(),
            sessions: tally.sessions.len(),
            last_failure_ms: tally.last_failure_ms,
        })
        .collect();
    candidates.sort_by(|a, b| b.last_failure_ms.cmp(&a.last_failure_ms).then_with(|| a.program.cmp(&b.program)));
    candidates.into_iter().filter(|rule| !on_path(&rule.program)).take(MAX_RULES).collect()
}

/// The lines of a data directory, oldest first: the rolled file, then the
/// current one. Missing or unreadable files read as empty, and a damaged byte
/// costs the line it is in, not the file.
pub fn read(dir: &Path) -> Vec<Line> {
    [ROLLED, FILE].iter().filter_map(|name| std::fs::read(dir.join(name)).ok()).flat_map(|bytes| parse_lines(&bytes)).collect()
}

/// Appends `lines` to the ledger of `dir`, in one write. Any failure is
/// swallowed, and a directory that does not exist is left that way.
pub fn append(dir: &Path, lines: &[Line]) {
    if lines.is_empty() {
        return;
    }
    let _ = hook_log::append_rolled(dir, FILE, ROLLED, hook_log::ROLL_AT, &render(lines));
}

/// Whether `program` is in a folder of `entries`, bare or with any of the
/// `extensions` a system adds to a program's name (`.exe` on Windows, none
/// elsewhere), as `exists` says.
pub fn on_path_in(program: &str, entries: &[PathBuf], extensions: &[String], exists: impl Fn(&Path) -> bool + Copy) -> bool {
    std::iter::once(String::new()).chain(extensions.iter().cloned()).any(|extension| {
        let name = format!("{program}{extension}");
        !crate::doctor::analyze(Path::new(""), entries, &name, exists).installs.is_empty()
    })
}

/// [`on_path_in`] for the `PATH` of this process and, on Windows, its `PATHEXT`.
pub fn on_path(program: &str) -> bool {
    let entries: Vec<PathBuf> = std::env::var_os("PATH").map(|raw| std::env::split_paths(&raw).collect()).unwrap_or_default();
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
    on_path_in(program, &entries, &extensions, |path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASH: &str = "/usr/bin/bash: line 1: eza: command not found";

    fn fails(observed: &[Observation]) -> Vec<&str> {
        observed.iter().filter(|o| o.kind == Kind::Fail).map(|o| o.program.as_str()).collect()
    }

    fn oks(observed: &[Observation]) -> Vec<&str> {
        observed.iter().filter(|o| o.kind == Kind::Ok).map(|o| o.program.as_str()).collect()
    }

    #[test]
    fn a_missing_program_is_read_from_each_shell_form() {
        assert_eq!(missing_programs(BASH), ["eza"]);
        assert_eq!(missing_programs(&format!("Exit code 127\n{BASH}")), ["eza"], "with the exit code in front");
        assert_eq!(missing_programs(&format!("{BASH}\n{BASH}\n")), ["eza"], "repeated, it is one program");
        let two = "/usr/bin/bash: line 1: eza: command not found\r\n/usr/bin/bash: line 2: fd: command not found\r\n";
        assert_eq!(missing_programs(two), ["eza", "fd"], "in order, and a Windows line end is not part of the name");
        assert_eq!(missing_programs("bash: fd: command not found"), ["fd"]);
        assert_eq!(missing_programs("sd: command not found"), ["sd"], "no shell name in front");
        assert_eq!(missing_programs("sh: 1: eslint: not found"), ["eslint"]);
        assert_eq!(missing_programs("/bin/sh: 12: eslint: not found"), ["eslint"]);
        assert_eq!(missing_programs("zsh: command not found: bat"), ["bat"]);
        assert_eq!(missing_programs("src/a.rs:3: file not found in the index"), Vec::<&str>::new(), "a line that only says not found");
        assert_eq!(missing_programs("everything went well"), Vec::<&str>::new());
    }

    #[test]
    fn a_failure_needs_the_program_in_command_position() {
        let dependency = observe("Bash", "npm test", &["sh: 1: eslint: not found"], After::Failure);
        assert_eq!(dependency, [], "a script's own dependency is not the machine's");

        let chained = observe("Bash", "cd src && eza -la", &[BASH], After::Failure);
        assert_eq!(fails(&chained), ["eza"]);
        let prefixed = observe("Bash", "FOO=1 sudo env BAR=2 eza", &[BASH], After::Failure);
        assert_eq!(fails(&prefixed), ["eza"], "assignments and prefixes are skipped");
        let quoted = observe("Bash", "(cd src; \"eza\")", &[BASH], After::Failure);
        assert_eq!(fails(&quoted), ["eza"]);
        assert_eq!(observe("Bash", "ls", &[BASH], After::Failure), [], "the signature of a program the command never named");

        assert_eq!(
            command_words("cd /x && eza -la | head; FOO=1 sudo fd x 2>&1\nbat y & sd z"),
            ["cd", "eza", "head", "fd", "bat", "sd"]
        );
        assert_eq!(command_words("echo $(eza) || true"), ["echo", "eza", "true"]);
    }

    #[test]
    fn a_pipe_with_exit_zero_still_shows_the_failure() {
        let response = serde_json::json!({ "stdout": "", "stderr": BASH, "interrupted": false, "isImage": false });
        let observed = observe("Bash", "eza -la | head", &texts(&response), After::Success);
        assert_eq!(fails(&observed), ["eza"]);
        assert_eq!(oks(&observed), Vec::<&str>::new(), "a call that showed a missing program is no success of the other one");

        let plain = serde_json::json!(format!("Exit code 0\n{BASH}"));
        assert_eq!(fails(&observe("Bash", "eza | head", &texts(&plain), After::Success)), ["eza"], "a response that is only text");
        let nested = serde_json::json!({ "content": [{ "type": "text", "text": BASH }], "n": 3, "ok": true, "none": null });
        assert_eq!(texts(&nested), ["text", BASH], "every string leaf, at any depth, and nothing else");
    }

    #[test]
    fn a_redirected_or_chained_command_is_no_success() {
        assert_eq!(oks(&observe("Bash", "rg todo src", &["src/a.rs:1: todo"], After::Success)), ["rg"]);
        assert_eq!(oks(&observe("Bash", "cd src && rg todo | head", &[""], After::Success)), ["cd", "rg", "head"]);
        for hiding in ["rg todo 2>/dev/null", "rg todo 2>&1", "rg todo &> out", "rg todo || true", "eza 2>/dev/null | head"] {
            assert_eq!(observe("Bash", hiding, &[""], After::Success), [], "{hiding}");
        }
        assert_eq!(observe("Bash", "rg todo", &[], After::Failure), [], "a success is only for a call that ended well");
        assert_eq!(observe("Read", "rg todo", &[""], After::Success), [], "only the shell tool is read");
        assert_eq!(observe("Bash", "", &[""], After::Success), []);
    }

    #[test]
    fn a_secret_or_an_odd_name_is_never_a_program() {
        // Built at run time so no literal looks like a live credential.
        let key = format!("AKIA{}", "Q7ZX".repeat(4));
        assert!(has_program_shape(&key), "the key has the shape of a name, so the detector is what stops it");
        let said = format!("/usr/bin/bash: line 1: {key}: command not found");
        assert_eq!(observe("Bash", &format!("{key} --help"), &[&said], After::Failure), []);

        let long = "a".repeat(33);
        let too_long = format!("bash: {long}: command not found");
        assert_eq!(observe("Bash", &long, &[&too_long], After::Failure), []);
        assert!(is_program_name(&"a".repeat(32)));
        for odd in ["-x", ".hidden", "./run.sh", "/usr/bin/eza", "éza", "a b", ""] {
            assert!(!is_program_name(odd), "{odd:?}");
        }
        for fine in ["eza", "7z", "g++", "python3.12", "cargo-nextest", "my_tool"] {
            assert!(is_program_name(fine), "{fine:?}");
        }
        let script = observe("Bash", "./run.sh", &["bash: line 1: ./run.sh: command not found"], After::Failure);
        assert_eq!(script, [], "a path is not a program of the machine");
    }

    #[test]
    fn exit_127_alone_and_other_shells_say_nothing() {
        assert_eq!(observe("Bash", "eza", &["Exit code 127"], After::Failure), []);
        let cmd = "'eza' is not recognized as an internal or external command,\r\noperable program or batch file.";
        assert_eq!(observe("Bash", "eza", &[cmd], After::Failure), []);
        assert_eq!(observe("Bash", "eza", &[cmd], After::Success), [], "not even a success: the call did not work");
        let powershell = "The term 'eza' is not recognized as the name of a cmdlet, function, script file, or operable program.";
        assert_eq!(observe("Bash", "eza", &[powershell], After::Failure), []);
    }

    const DAY: i64 = 24 * 60 * 60 * 1000;
    const NOW: i64 = 1_800_000_000_000;

    fn line(at_ms: i64, event: Kind, program: &str, session: &str) -> Line {
        Line { at_ms, event, tool: "Bash".into(), program: program.into(), session: session.into() }
    }

    fn fail(days_ago: i64, program: &str, session: &str) -> Line {
        line(NOW - days_ago * DAY, Kind::Fail, program, session)
    }

    fn ok(days_ago: i64, program: &str, session: &str) -> Line {
        line(NOW - days_ago * DAY, Kind::Ok, program, session)
    }

    fn nowhere(_: &str) -> bool {
        false
    }

    fn programs(rules: &[Rule]) -> Vec<&str> {
        rules.iter().map(|r| r.program.as_str()).collect()
    }

    fn call(session: &str) -> Call<'_> {
        Call { at_ms: NOW, tool: "Bash", session }
    }

    #[test]
    fn a_line_keeps_only_the_program_the_tool_and_the_session() {
        let said = BASH;
        let observed = observe("Bash", "cd /home/u/secreto && eza --token=abc", &[said], After::Failure);
        let written = to_write(&call("s1"), &observed, &[]);
        let text = render(&written);

        let value: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
        let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["at_ms", "event", "tool", "program", "session"], "nothing beyond its definition");
        assert_eq!((value["event"].as_str(), value["tool"].as_str(), value["program"].as_str()), (Some("fail"), Some("Bash"), Some("eza")));
        assert_eq!((value["session"].as_str(), value["at_ms"].as_i64()), (Some("s1"), Some(NOW)));
        assert!(text.ends_with('\n') && text.matches('\n').count() == 1, "one line, built whole: {text:?}");
        for left_out in ["secreto", "abc", "/home", "token"] {
            assert!(!text.contains(left_out), "{left_out:?} in {text}");
        }

        let long = to_write(&call(&"s".repeat(500)), &observed, &[]);
        assert_eq!(long[0].session.chars().count(), 64, "a session id is cut");
    }

    #[test]
    fn only_the_first_success_after_a_failure_is_written() {
        let worked = [Observation { kind: Kind::Ok, program: "rg".into() }];
        let written = |ledger: &[Line]| to_write(&call("s"), &worked, ledger);

        assert_eq!(written(&[]), [], "a program that never failed is not entered");
        assert_eq!(written(&[fail(1, "eza", "a")]), [], "a failure of another program");
        let after_a_failure = written(&[fail(1, "rg", "a")]);
        assert_eq!(after_a_failure.iter().map(|l| (l.event, l.program.as_str())).collect::<Vec<_>>(), [(Kind::Ok, "rg")]);
        assert_eq!(written(&[fail(2, "rg", "a"), ok(1, "rg", "a")]), [], "the newest line is already a success");
        assert_eq!(written(&[ok(2, "rg", "a"), fail(1, "rg", "b")]).len(), 1, "a failure after a success is followed by a success again");

        let failed = [Observation { kind: Kind::Fail, program: "rg".into() }];
        assert_eq!(to_write(&call("s"), &failed, &[fail(1, "rg", "a")]).len(), 1, "every failure is written");
        let mixed = [failed[0].clone(), Observation { kind: Kind::Ok, program: "cd".into() }];
        assert_eq!(to_write(&call("s"), &mixed, &[]).len(), 1, "a failure and a success that is not entered");
    }

    #[test]
    fn the_ledger_rolls_at_64_kib_without_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join(FILE);
        let rolled = dir.path().join(ROLLED);
        let one = [fail(0, "eza", "s")];

        std::fs::write(&current, vec![b'a'; hook_log::ROLL_AT as usize - 1]).unwrap();
        append(dir.path(), &one);
        assert!(!rolled.exists(), "one byte under the limit keeps the file");

        std::fs::write(&current, vec![b'b'; hook_log::ROLL_AT as usize]).unwrap();
        std::fs::write(&rolled, b"older").unwrap();
        append(dir.path(), &one);
        assert_eq!(std::fs::read(&rolled).unwrap(), vec![b'b'; hook_log::ROLL_AT as usize], "the old rolled file is replaced, not rewritten");
        assert_eq!(read_current(dir.path()), one, "a new file starts with the line");

        append(dir.path(), &[]);
        assert_eq!(read_current(dir.path()).len(), 1, "nothing to write, nothing written");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    fn read_current(dir: &Path) -> Vec<Line> {
        parse_lines(&std::fs::read(dir.join(FILE)).unwrap())
    }

    #[test]
    fn no_data_directory_no_ledger() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join(".clawg");
        append(&missing, &[fail(0, "eza", "s")]);
        assert!(!missing.exists());
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
        assert_eq!(read(&missing), []);
    }

    #[test]
    fn a_damaged_line_costs_only_itself() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (fail(2, "eza", "s1"), ok(1, "rg", "s2"));
        let mut bytes = render(std::slice::from_ref(&a)).into_bytes();
        bytes.extend_from_slice(b"half a li\xc3\n");
        bytes.extend_from_slice(b"{\"at_ms\":1,\"event\":\"unheard_of\",\"tool\":\"Bash\",\"program\":\"x\",\"session\":\"s\"}\n\n");
        bytes.extend_from_slice(render(std::slice::from_ref(&b)).as_bytes());
        std::fs::write(dir.path().join(FILE), bytes).unwrap();
        let older = fail(3, "fd", "s0");
        std::fs::write(dir.path().join(ROLLED), render(std::slice::from_ref(&older))).unwrap();

        assert_eq!(read(dir.path()), [older, a, b], "the rolled file first, then the current one");
    }

    #[test]
    fn two_sessions_make_a_rule_and_one_does_not() {
        let one_session = [fail(3, "eza", "s1"), fail(2, "eza", "s1"), fail(1, "eza", "s1")];
        assert_eq!(rules(&one_session, NOW, nowhere), [], "three failures of one session are one session");
        let two = [fail(3, "eza", "s1"), fail(1, "eza", "s2")];
        assert_eq!(
            rules(&two, NOW, nowhere),
            [Rule { tool: "Bash".into(), program: "eza".into(), sessions: 2, last_failure_ms: NOW - DAY }]
        );
        assert_eq!(rules(&[fail(1, "eza", "s1")], NOW, nowhere), []);
    }

    #[test]
    fn a_success_in_the_window_cancels_the_rule() {
        let lines = [fail(3, "rg", "s1"), ok(3, "rg", "s1"), fail(2, "rg", "s2"), ok(2, "rg", "s2"), fail(1, "rg", "s3")];
        assert_eq!(rules(&lines, NOW, nowhere), [], "each failure was followed by a success");
        let before_the_failures = [ok(5, "eza", "s0"), fail(2, "eza", "s1"), fail(1, "eza", "s2")];
        assert_eq!(rules(&before_the_failures, NOW, nowhere), [], "a success anywhere in the window counts");
        let outside = [ok(40, "eza", "s0"), fail(2, "eza", "s1"), fail(1, "eza", "s2")];
        assert_eq!(programs(&rules(&outside, NOW, nowhere)), ["eza"], "one older than the window is forgotten");
    }

    #[test]
    fn a_program_on_the_path_is_no_rule() {
        let lines = [fail(3, "gh", "s1"), fail(1, "gh", "s2"), fail(3, "eza", "s1"), fail(2, "eza", "s2")];
        let installed = |program: &str| program == "gh";
        assert_eq!(programs(&rules(&lines, NOW, installed)), ["eza"]);

        let entries = [PathBuf::from("/bin"), PathBuf::from("/opt/tools")];
        let exists = |path: &Path| path == Path::new("/opt/tools").join("gh.exe") || path == Path::new("/bin").join("rg");
        assert!(on_path_in("rg", &entries, &[], exists));
        assert!(!on_path_in("gh", &entries, &[], exists), "with no extension to try, gh.exe is another name");
        assert!(on_path_in("gh", &entries, &[".COM".into(), ".exe".into()], exists));
        assert!(!on_path_in("eza", &entries, &[".exe".into()], exists));
    }

    #[test]
    fn failures_older_than_thirty_days_are_forgotten() {
        let old = [fail(31, "fd", "s1"), fail(31, "fd", "s2")];
        assert_eq!(rules(&old, NOW, nowhere), []);
        let thirty = [fail(30, "fd", "s1"), fail(29, "fd", "s2")];
        assert_eq!(programs(&rules(&thirty, NOW, nowhere)), ["fd"], "the window is thirty days");
        let half = [fail(31, "fd", "s1"), fail(1, "fd", "s2")];
        assert_eq!(rules(&half, NOW, nowhere), [], "only the failures inside the window count");
    }

    #[test]
    fn at_most_five_rules_newest_first() {
        let names = ["a1", "b2", "c3", "d4", "e5", "f6", "g7"];
        let lines: Vec<Line> = names
            .iter()
            .enumerate()
            .flat_map(|(i, name)| [fail(20 - i as i64, name, "s1"), fail(10 - i as i64, name, "s2")])
            .collect();
        let found = rules(&lines, NOW, nowhere);
        assert_eq!(programs(&found), ["g7", "f6", "e5", "d4", "c3"], "the newest five, newest first");
        assert!(found.windows(2).all(|w| w[0].last_failure_ms >= w[1].last_failure_ms));
    }

    #[test]
    fn what_a2_finds_is_one_rule() {
        let mut lines = vec![fail(5, "eza", "s1"), fail(4, "eza", "s2")];
        lines.extend([fail(9, "rg", "r1"), ok(9, "rg", "r1"), fail(8, "rg", "r2"), ok(8, "rg", "r2"), fail(7, "rg", "r3"), ok(7, "rg", "r3")]);
        lines.extend([fail(6, "gh", "g1"), fail(5, "gh", "g2")]);
        lines.extend([fail(31, "fd", "f1"), fail(31, "fd", "f2")]);
        let found = rules(&lines, NOW, |program| program == "gh");
        assert_eq!(found.iter().map(|r| (r.program.as_str(), r.sessions)).collect::<Vec<_>>(), [("eza", 2)]);
    }
}
