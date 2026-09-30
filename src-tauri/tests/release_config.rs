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
