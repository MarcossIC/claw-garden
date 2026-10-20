//! Measures what a `topic_key` does for a memory that is revised: how often an
//! old version stays live without a mark when the save names no key, whether
//! a key hides every old version, and what showing the key costs.

mod set;

use std::path::Path;

const EXIT_OK: i32 = 0;
const EXIT_INPUT_INVALID: i32 = 2;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/revision-chains.json");
    let set = match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| set::parse_set(&t)) {
        Ok(set) => set,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            return EXIT_INPUT_INVALID;
        }
    };
    println!(
        "chain set version {} sha256 {}: {} chains, {} transitions",
        set.version,
        set.sha256,
        set.chains.len(),
        set.transitions()
    );
    EXIT_OK
}
