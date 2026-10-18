//! Conventions of the workspace that the compiler does not enforce.

use std::fs;
use std::path::{Path, PathBuf};

/// Whether `line` is an attribute that silences an unused-code lint: an
/// `allow` or `expect`, inner or outer, also behind a `cfg_attr`, naming
/// `dead_code`, `unused` or an `unused_*` lint. The repository's rule is to
/// delete such code, not to hide it.
fn hides_dead_code(line: &str) -> bool {
    let line = line.trim_start();
    let Some(attr) = line.strip_prefix("#![").or_else(|| line.strip_prefix("#[")) else {
        return false;
    };
    ["allow(", "expect("].iter().any(|opener| {
        attr.match_indices(opener).any(|(at, _)| {
            let lints = &attr[at + opener.len()..];
            let lints = &lints[..lints.find(')').unwrap_or(lints.len())];
            lints.split(',').map(str::trim).any(|lint| lint == "dead_code" || lint == "unused" || lint.starts_with("unused_"))
        })
    })
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn hides_dead_code_recognises_the_attribute_forms() {
    for hiding in [
        "#[allow(dead_code)]",
        "    #[allow(dead_code)] // kept for later",
        "#![allow(unused)]",
        "#[allow(unused_imports, unused_variables)]",
        "#[expect(dead_code)]",
        "#[cfg_attr(not(test), allow(dead_code))]",
        "#[allow(clippy::too_many_arguments, dead_code)]",
    ] {
        assert!(hides_dead_code(hiding), "{hiding}");
    }
    for fine in [
        "#[allow(clippy::too_many_arguments)]",
        "#[cfg(test)]",
        "// #[allow(dead_code)] in a comment",
        "/// Never `#[allow(dead_code)]`.",
        "let s = \"#[allow(dead_code)]\";",
        "#[allow(clippy::unused_io_amount)]",
    ] {
        assert!(!hides_dead_code(fine), "{fine}");
    }
}

#[test]
fn no_source_allows_dead_code() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the crates directory");
    let mut files = Vec::new();
    for krate in fs::read_dir(crates).expect("read crates/").flatten() {
        rust_files(&krate.path().join("src"), &mut files);
        rust_files(&krate.path().join("tests"), &mut files);
    }
    assert!(!files.is_empty(), "no sources found under {}", crates.display());

    let mut found = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for (n, line) in source.lines().enumerate() {
            if hides_dead_code(line) {
                let shown = file.strip_prefix(crates).unwrap_or(file);
                found.push(format!("{}:{}", shown.display(), n + 1));
            }
        }
    }
    assert!(found.is_empty(), "delete the unused code instead of allowing it:\n{}", found.join("\n"));
}

/// Lines of a manifest that turn on clawg-core's `testing` feature outside
/// `[dev-dependencies]`. That feature lets a binary read `CLAWG_TEST_HOME`
/// and `CLAWG_TEST_EMBEDDER`, so a release build must never get it.
fn testing_outside_dev_dependencies(manifest: &str) -> Vec<usize> {
    let mut section = "";
    let mut found = Vec::new();
    for (n, line) in manifest.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            section = line;
            continue;
        }
        let enables = line.contains("\"testing\"") || line.contains("/testing\"");
        if enables && !section.ends_with("dev-dependencies]") {
            found.push(n + 1);
        }
    }
    found
}

#[test]
fn testing_outside_dev_dependencies_reads_the_section() {
    let manifest = r#"
[dependencies]
clawg-core = { workspace = true, features = ["testing"] }
[dev-dependencies]
clawg-core = { workspace = true, features = ["testing"] }
[target.'cfg(unix)'.dev-dependencies]
clawg-core = { workspace = true, features = ["testing"] }
[features]
testing = []
default = ["testing"]
local = ["clawg-core/testing"]
"#;
    assert_eq!(testing_outside_dev_dependencies(manifest), [3, 10, 11]);
}

#[test]
fn only_dev_dependencies_turn_on_the_test_stand_ins() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let crates = root.join("crates");
    let mut manifests = vec![root.join("Cargo.toml")];
    for krate in fs::read_dir(&crates).expect("read crates/").flatten() {
        manifests.push(krate.path().join("Cargo.toml"));
    }

    let mut found = Vec::new();
    for manifest in manifests.iter().filter(|m| m.is_file()) {
        let text = fs::read_to_string(manifest).unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
        for n in testing_outside_dev_dependencies(&text) {
            found.push(format!("{}:{n}", manifest.display()));
        }
    }
    assert!(found.is_empty(), "`testing` belongs only in [dev-dependencies]:\n{}", found.join("\n"));
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_repo_file(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The value written for `key` inside `[section]` of a manifest, as the raw
/// text after the `=`, without a trailing comment.
fn manifest_value(manifest: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            if name.trim() == key {
                let value = value.split(" #").next().unwrap_or(value);
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

#[test]
fn the_manifest_carries_the_publication_metadata() {
    let manifest = read_repo_file("Cargo.toml");
    let value = |key: &str| manifest_value(&manifest, "workspace.package", key).unwrap_or_default();

    assert_eq!(value("license"), "\"MIT\"");
    assert_eq!(value("repository"), "\"https://github.com/MarcossIC/claw-garden\"");
    assert_eq!(value("authors"), "[\"The Claw Garden contributors\"]");

    let license = read_repo_file("LICENSE");
    assert!(license.starts_with("MIT License"), "LICENSE is the MIT text");
    assert!(license.contains("The Claw Garden contributors"), "LICENSE names the holder");
}

#[test]
fn reqwest_uses_rustls_with_the_system_roots() {
    let manifest = read_repo_file("Cargo.toml");
    let declaration = manifest_value(&manifest, "workspace.dependencies", "reqwest").expect("reqwest is declared");

    assert!(declaration.contains("default-features = false"), "{declaration}");
    for feature in ["json", "stream", "rustls-tls-native-roots", "charset", "http2", "system-proxy"] {
        assert!(declaration.contains(&format!("\"{feature}\"")), "reqwest needs `{feature}`: {declaration}");
    }
    for feature in ["default-tls", "native-tls"] {
        assert!(!declaration.contains(feature), "reqwest must not enable `{feature}`: {declaration}");
    }
}

// The workflow tests read the files as text, line by line, like the manifest
// tests above: a YAML parser would be a dependency with no other use. A
// reformat that keeps the meaning can break them, and is fixed in the same commit.

/// The lines of a workflow with comments removed: a whole-line comment becomes
/// empty and a trailing ` #` comment is cut.
fn workflow_code(name: &str) -> Vec<String> {
    read_repo_file(&format!(".github/workflows/{name}"))
        .lines()
        .map(|line| {
            if line.trim_start().starts_with('#') {
                String::new()
            } else {
                line.split(" #").next().unwrap_or(line).trim_end().to_string()
            }
        })
        .collect()
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

struct Job {
    name: String,
    lines: Vec<String>,
}

impl Job {
    fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// The value of the job's own `key:` (at the job's key indentation), with the
    /// continuation lines of a folded or literal block joined by spaces.
    fn key(&self, key: &str) -> Option<String> {
        let prefix = format!("{key}:");
        let at = self.lines.iter().position(|l| indent_of(l) == 4 && l.trim_start().starts_with(&prefix))?;
        let mut value = self.lines[at].trim_start()[prefix.len()..].trim().to_string();
        for line in &self.lines[at + 1..] {
            if line.trim().is_empty() {
                continue;
            }
            if indent_of(line) <= 4 {
                break;
            }
            value.push(' ');
            value.push_str(line.trim());
        }
        Some(value)
    }

    /// The steps of the job, each as its own lines.
    fn steps(&self) -> Vec<Vec<String>> {
        let mut steps: Vec<Vec<String>> = Vec::new();
        let mut inside = false;
        for line in &self.lines {
            if indent_of(line) == 4 && line.trim_start().starts_with("steps:") {
                inside = true;
                continue;
            }
            if !inside {
                continue;
            }
            if indent_of(line) == 6 && line.trim_start().starts_with("- ") {
                steps.push(Vec::new());
            }
            if let Some(step) = steps.last_mut() {
                step.push(line.clone());
            }
        }
        steps
    }
}

fn jobs_of(lines: &[String]) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    let mut inside = false;
    for line in lines {
        if line == "jobs:" {
            inside = true;
            continue;
        }
        if !inside || line.trim().is_empty() {
            continue;
        }
        if indent_of(line) == 0 {
            break;
        }
        if indent_of(line) == 2 && line.trim_end().ends_with(':') {
            jobs.push(Job { name: line.trim().trim_end_matches(':').to_string(), lines: Vec::new() });
            continue;
        }
        if let Some(job) = jobs.last_mut() {
            job.lines.push(line.clone());
        }
    }
    jobs
}

fn job<'a>(jobs: &'a [Job], name: &str) -> &'a Job {
    jobs.iter().find(|j| j.name == name).unwrap_or_else(|| panic!("no job `{name}`"))
}

#[test]
fn third_party_actions_are_pinned_to_a_commit() {
    let raw = read_repo_file(".github/workflows/release.yml");
    let mut seen = 0;
    for (n, line) in raw.lines().enumerate() {
        let Some(at) = line.find("uses:") else { continue };
        if line.trim_start().starts_with('#') {
            continue;
        }
        let reference = line[at + "uses:".len()..].split_whitespace().next().unwrap_or("");
        if reference.starts_with("./") {
            continue;
        }
        let (_, pin) = reference.split_once('@').unwrap_or_else(|| panic!("release.yml:{}: `{reference}` has no @ref", n + 1));
        let is_commit = pin.len() == 40 && pin.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        assert!(is_commit, "release.yml:{}: `{reference}` is not pinned to a full commit SHA", n + 1);
        assert!(line.contains(" # "), "release.yml:{}: name the tag the SHA stands for in a trailing comment", n + 1);
        seen += 1;
    }
    assert!(seen > 0, "release.yml uses no action at all");
}

#[test]
fn the_release_workflow_publishes_only_for_a_successful_push_to_main() {
    let lines = workflow_code("release.yml");
    let has = |wanted: &str| lines.iter().any(|l| l.trim() == wanted);

    assert!(has("workflow_run:") && has("workflows: [CI]") && has("types: [completed]") && has("branches: [main]"));
    for other in ["pull_request", "pull_request_target", "push:", "workflow_dispatch", "schedule:"] {
        assert!(!lines.iter().any(|l| l.trim_start().starts_with(other)), "release.yml must not be triggered by `{other}`");
    }
    let permissions = lines.iter().position(|l| l == "permissions:").expect("top-level permissions");
    assert_eq!(lines[permissions + 1].trim(), "contents: read");
    let concurrency = lines.iter().position(|l| l == "concurrency:").expect("top-level concurrency");
    assert_eq!(lines[concurrency + 1].trim(), "group: release");
    assert_eq!(lines[concurrency + 2].trim(), "cancel-in-progress: false");

    let jobs = jobs_of(&lines);
    let plan = job(&jobs, "plan");
    let condition = plan.key("if").expect("the plan job is conditional");
    for wanted in [
        "github.event.workflow_run.conclusion == 'success'",
        "github.event.workflow_run.event == 'push'",
        "github.event.workflow_run.head_branch == 'main'",
        "github.event.workflow_run.head_repository.full_name == github.repository",
    ] {
        assert!(condition.contains(wanted), "plan runs without `{wanted}`: {condition}");
    }
    let text = plan.text();
    assert!(text.contains("ref: ${{ github.event.workflow_run.head_sha }}"), "plan must read the commit CI ran on");
    assert!(text.contains("refs/tags/v"), "plan must look for the version's tag");
    for other in jobs.iter().filter(|j| j.name != "plan") {
        assert!(other.key("needs").is_some_and(|n| n.contains("plan")), "job `{}` must wait for plan", other.name);
    }
}

#[test]
fn the_release_workflow_builds_only_the_cli_with_the_dist_mark() {
    let lines = workflow_code("release.yml");
    let jobs = jobs_of(&lines);
    let build = job(&jobs, "build");

    assert!(build.key("needs").is_some_and(|n| n.contains("plan")));
    assert!(build.key("if").is_some_and(|c| c.contains("needs.plan.outputs.publish == 'true'")));
    assert!(!lines.iter().any(|l| l.contains("-latest")), "runners are pinned, not `-latest`");
    for runner in ["os: windows-2022", "os: ubuntu-22.04"] {
        assert!(lines.iter().any(|l| l.trim().trim_start_matches("- ") == runner), "the matrix lacks `{runner}`");
    }
    for target in ["target: x86_64-pc-windows-msvc", "target: x86_64-unknown-linux-gnu"] {
        assert!(lines.iter().any(|l| l.trim() == target), "the matrix lacks `{target}`");
    }

    let builds: Vec<&String> = lines.iter().filter(|l| l.contains("cargo build")).collect();
    assert!(!builds.is_empty(), "release.yml builds nothing");
    for command in &builds {
        assert!(command.contains("-p clawg-cli"), "builds only the CLI: {command}");
        assert!(command.contains("--release") && command.contains("--locked"), "{command}");
        assert!(command.contains("--target ${{ matrix.target }}"), "{command}");
        assert!(!command.contains("--workspace") && !command.contains("--all"), "{command}");
    }
    for step in build.steps().iter().filter(|s| s.iter().any(|l| l.contains("cargo build"))) {
        assert!(
            step.iter().any(|l| l.trim() == "CLAWG_DIST_TARGET: ${{ matrix.target }}"),
            "the build step must set CLAWG_DIST_TARGET: {step:?}"
        );
    }

    let steps = build.steps();
    let smoke = steps
        .iter()
        .find(|s| s.iter().any(|l| l.contains("--version")))
        .expect("a step runs the built binary with --version");
    let smoke = smoke.join("\n");
    assert!(smoke.contains("clawg ${VERSION}"), "the smoke test compares with the planned version");
    assert!(smoke.contains("CLAWG_TEST_"), "the smoke test looks for the test stand-ins in the binary");

    let linkage = steps.iter().find(|s| s.iter().any(|l| l.contains("ldd"))).expect("a step runs ldd");
    let linkage = linkage.join("\n");
    assert!(linkage.contains("libssl") && linkage.contains("libcrypto"));
    assert!(linkage.contains("runner.os == 'Linux'"));
}

#[test]
fn the_release_workflow_publishes_last() {
    let lines = workflow_code("release.yml");
    let jobs = jobs_of(&lines);
    let publish = job(&jobs, "publish");

    let needs = publish.key("needs").expect("publish waits for other jobs");
    assert!(needs.contains("plan") && needs.contains("build"), "publish needs plan and build: {needs}");
    assert!(publish.key("if").is_some_and(|c| c.contains("needs.plan.outputs.publish == 'true'")));

    let only_in_publish = |needle: &str| {
        for other in jobs.iter().filter(|j| j.name != "publish") {
            assert!(!other.text().contains(needle), "`{needle}` appears in job `{}`", other.name);
        }
    };
    only_in_publish("gh release create");
    only_in_publish("environment:");
    only_in_publish("contents: write");
    only_in_publish("id-token: write");
    only_in_publish("attestations: write");
    only_in_publish("secrets.");
    assert_eq!(publish.key("environment").as_deref(), Some("release"));

    let text = publish.text();
    assert!(text.contains("gh release create"));
    assert!(text.contains("--target ${TARGET_SHA}") && text.contains("TARGET_SHA: ${{ github.event.workflow_run.head_sha }}"));
    for asset in [
        "dist/clawg-x86_64-pc-windows-msvc.exe",
        "dist/clawg-x86_64-unknown-linux-gnu",
        "dist/install.ps1",
        "dist/SHA256SUMS",
        "dist/SHA256SUMS.minisig",
    ] {
        let uploads = text.lines().filter(|l| l.trim().trim_end_matches('\\').trim() == asset).count();
        assert!(uploads >= 2, "`{asset}` must be attested and uploaded; found {uploads}");
    }

    let position = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("publish lacks `{needle}`"));
    assert!(position("minisign -S") < position("minisign -Vm dist/SHA256SUMS -p release/minisign.pub"));
    assert!(position("minisign -Vm") < position("actions/attest-build-provenance"));
    assert!(position("actions/attest-build-provenance") < position("gh release create"));
}

#[test]
fn the_signing_step_reads_the_key_password_from_stdin_without_echo() {
    let lines = workflow_code("release.yml");
    assert!(!lines.iter().any(|l| l.contains("set -x") || l.contains("xtrace")), "the shell trace would print secrets");

    let jobs = jobs_of(&lines);
    let publish = job(&jobs, "publish");
    let steps = publish.steps();
    let signing = steps.iter().find(|s| s.iter().any(|l| l.contains("minisign -S"))).expect("a step signs");

    let pipe = "printf '%s\\n' \"$MINISIGN_PASSWORD\" | minisign -S -s \"$key\" -t \"clawg v${VERSION} SHA256SUMS\" -m SHA256SUMS";
    assert!(signing.iter().any(|l| l.trim() == pipe), "the password goes in on stdin: {signing:?}");
    assert!(!signing.iter().any(|l| l.split_whitespace().any(|w| w == "-W")), "-W would sign without a password");
    assert_eq!(signing.iter().filter(|l| l.contains("$MINISIGN_PASSWORD")).count(), 1, "the password is used once, on stdin");
    assert!(!signing.iter().any(|l| l.contains("echo") && l.contains("MINISIGN_")), "a secret is echoed");
    for secret in ["MINISIGN_SECRET_KEY: ${{ secrets.MINISIGN_SECRET_KEY }}", "MINISIGN_PASSWORD: ${{ secrets.MINISIGN_PASSWORD }}"] {
        assert!(signing.iter().any(|l| l.trim() == secret), "the signing step lacks `{secret}`");
    }
    assert!(signing.iter().any(|l| l.contains("umask 077") || l.contains("chmod 600")), "the key file must be private");
    assert!(signing.iter().any(|l| l.contains("trap") && l.contains("rm -f")), "the key file must be removed at the end");

    for (n, line) in lines.iter().enumerate() {
        let outside = !signing.contains(line);
        assert!(!(outside && line.contains("MINISIGN_PASSWORD")), "release.yml:{}: the password leaves the signing step", n + 1);
        assert!(!(outside && line.contains("MINISIGN_SECRET_KEY")), "release.yml:{}: the key leaves the signing step", n + 1);
        assert!(!(outside && line.contains("secrets.")), "release.yml:{}: a secret outside the signing step", n + 1);
    }
}
