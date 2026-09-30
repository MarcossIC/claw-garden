//! Guards on the files that shape a release build.
//!
//! None of these are code, so no other test would notice them drifting — and
//! each one fails silently at the worst moment: while building or installing
//! the first real release.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The version every artifact must carry: `[workspace.package]`, inherited by
/// this crate.
const WORKSPACE_VERSION: &str = env!("CARGO_PKG_VERSION");

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

// ── one version ─────────────────────────────────────────────────────────────

#[test]
fn the_tauri_config_carries_the_workspace_version() {
    let conf = read_json(&manifest_dir().join("tauri.conf.json"));
    assert_eq!(
        conf["version"], WORKSPACE_VERSION,
        "tauri.conf.json names the installer and the app's About version; \
         bump it together with [workspace.package]"
    );
}

#[test]
fn package_json_carries_the_workspace_version() {
    let pkg = read_json(&manifest_dir().join("..").join("package.json"));
    assert_eq!(
        pkg["version"], WORKSPACE_VERSION,
        "package.json is the frontend's half of the same product; \
         bump it together with [workspace.package]"
    );
}

// ── the sidecar lives only in the release overlay ───────────────────────────
//
// tauri-build processes `bundle.externalBin` on EVERY build of this crate, not
// only under `tauri build`: it requires the file to exist and copies it to
// `target/<profile>/clawg.exe`. Declared in a config that plain cargo reads,
// it would break `cargo test` wherever the sidecar was never built, and
// overwrite the `clawg.exe` that `clawg-cli` builds into the same target dir —
// the binary Claude Code runs as `clawg-dev`.

#[test]
fn the_configs_cargo_reads_declare_no_sidecar() {
    for file in ["tauri.conf.json", "tauri.windows.conf.json"] {
        let conf = read_json(&manifest_dir().join(file));
        assert!(
            conf["bundle"]["externalBin"].is_null(),
            "{file} is read by every cargo build; externalBin belongs in tauri.release.conf.json"
        );
    }
}

#[test]
fn the_release_overlay_bundles_clawg_and_builds_it_first() {
    let overlay = read_json(&manifest_dir().join("tauri.release.conf.json"));
    assert_eq!(overlay["bundle"]["externalBin"], serde_json::json!(["binaries/clawg"]));
    assert_eq!(overlay["build"]["beforeBuildCommand"], "bun run build:release");

    let pkg = read_json(&manifest_dir().join("..").join("package.json"));
    let scripts = &pkg["scripts"];
    assert!(
        scripts["build:release"].as_str().unwrap_or_default().contains("build:sidecar"),
        "the release frontend build must stage the sidecar: {scripts}"
    );
    assert_eq!(
        scripts["tauri:build"], "tauri build --config src-tauri/tauri.release.conf.json",
        "a bare `tauri build` would bundle without clawg"
    );
}

// ── the Windows installer ───────────────────────────────────────────────────

#[test]
fn the_windows_installer_is_a_per_user_nsis_with_hooks() {
    let conf = read_json(&manifest_dir().join("tauri.windows.conf.json"));
    assert_eq!(conf["bundle"]["targets"], serde_json::json!(["nsis"]));
    let nsis = &conf["bundle"]["windows"]["nsis"];
    assert_eq!(nsis["installMode"], "currentUser", "no admin prompt, %LOCALAPPDATA% install");
    let hooks = nsis["installerHooks"].as_str().expect("installerHooks is set");
    assert!(manifest_dir().join(hooks).is_file(), "{hooks} must exist next to tauri.conf.json");
}

/// The body of `!macro <name>` … `!macroend` in the hooks file.
fn hook_body<'a>(hooks: &'a str, name: &str) -> &'a str {
    let start = hooks
        .find(&format!("!macro {name}"))
        .unwrap_or_else(|| panic!("hooks.nsh defines no {name}"));
    let rest = &hooks[start..];
    &rest[..rest.find("!macroend").expect("unterminated macro")]
}

#[test]
fn the_hooks_stop_both_daemons_and_manage_the_path() {
    let hooks = std::fs::read_to_string(manifest_dir().join("windows").join("hooks.nsh"))
        .expect("read hooks.nsh");
    // Tauri's own running-app check covers only the main binary, and a daemon
    // of EITHER profile started from the installed clawg.exe holds it open
    // through an update or uninstall: the release binary may serve dev too
    // (`--profile dev`), which is exactly how phase 3 rehearses the migration.
    for hook in ["NSIS_HOOK_PREINSTALL", "NSIS_HOOK_PREUNINSTALL"] {
        for profile in ["prod", "dev"] {
            assert!(
                hook_body(&hooks, hook).contains(&format!("stop --profile {profile}")),
                "{hook} must stop the {profile} daemon"
            );
        }
    }
    assert!(hook_body(&hooks, "NSIS_HOOK_POSTINSTALL").contains("path add"));
    assert!(hook_body(&hooks, "NSIS_HOOK_PREUNINSTALL").contains("path remove"));
}
