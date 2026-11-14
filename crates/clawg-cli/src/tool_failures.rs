//! What a call to a shell tool says about the programs the machine does not
//! have: the shell prints `X: command not found`, and clawg reads it from the
//! tool's output without the agent judging anything.
//!
//! Everything here is pure: it decides from the texts it is given, so no file,
//! port or process is needed to test it.

use clawg_core::memory::redact::redact;

/// Which of the client's two events after a tool the call came with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    /// The tool ended well as far as the client could tell. A pipe or a `;`
    /// can end a command with code 0 after a program was missing.
    Success,
    Failure,
}

/// What a call showed about one program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}
