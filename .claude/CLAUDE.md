# Claw Garden

Local-first memory for coding agents, written in Rust. It stores, classifies and retrieves memories and context, served by a loopback HTTP daemon and a stdio MCP bridge (Claude Code is the current client).

## This file

It holds only what every session needs: the rules, and where to look for the rest. How clawg behaves lives in `docs/reference/` and in `openspec/specs/<area>/spec.md`; read the part a task needs, or ask `search_project`. A change in behaviour updates those documents, never this file. This file changes only when a rule or a pointer changes, and stays under 200 lines.

| Topic | Document |
| --- | --- |
| Data directory and profiles, migrations and snapshots, model download and idle unload, store lock, socket and TCP, metrics, projects (identity, merge, delete, subfolders), sharing, preferences | `docs/reference/data-profiles.md` |
| Every `clawg` command | `docs/reference/commands.md` |
| Daemon HTTP API, docs index, ingest (masking, outbox, pieces, topic keys, contradiction detector, provenance), MCP tools, protocol and languages | `docs/reference/daemon.md` |
| The suite, `testing` builds, CI and release workflows, eval tools (`rag-eval`, `revision-eval`, `load-eval`, `perf-bench`, `agent-eval`) and the daemon bench | `docs/reference/testing.md` |
| Versioning, publishing and release keys | `docs/reference/release.md` |
| Design documents | `docs/` |

## Workspace

| Crate | Role |
| --- | --- |
| `crates/clawg-core` | Memory engine: SQLite storage, ONNX embeddings, ranking, background workers, the exclusive lock on `memory.db`, the daemon's usage metrics. Library `clawg_core`. |
| `crates/clawg-cli` | The `clawg` binary (`serve`, `mcp`, `hook`, `setup`, `doctor`, …): the daemon in `src/daemon/`, the MCP bridge in `src/mcp/`. Also a library `clawg_cli`. |
| `crates/clawg-eval` | Evaluation tools (`rag-eval`, `golden-mine`, `conflict-eval`, `revision-eval`, `load-eval`, `perf-bench`, `agent-eval`), through clawg-core's `eval` feature, which production builds never link. |

## Data profiles

| Profile | Data directory | Default port | MCP server name |
| --- | --- | --- | --- |
| `dev` | `~/.clawg-dev` | 27381 | `clawg-dev` |
| `prod` | `~/.clawg` | 27380 | `clawg` |

Selection: `--profile dev|prod`, then `CLAWG_PROFILE`, then the build type (`dev` for debug, `prod` for release), in `crates/clawg-core/src/paths.rs`. A debug build refuses `prod`: a debug binary must never open `~/.clawg`.

## Code conventions

- Comments explain only a non-obvious why. No commented-out code, no `#[allow]` to hide dead code.
- Errors in library code are typed with `thiserror`.
- Unit tests sit next to the code in `#[cfg(test)] mod tests`; integration tests live in each crate's `tests/`.
- TDD: write the failing test first, then the code that makes it pass.
- Keep parsing/decision logic pure and separate from I/O so it is testable without a filesystem, port or database (see `cli.rs`, `Profile::resolve`).
- Do not add dependencies, visibility or abstractions without a real use.

## Rules for agents

- Code, comments, test names, error messages and commit messages never reference plans, phases or work packages.
- Everything committed is written for whoever clones the repository. A step says what it needs to run (an OS, the real model, the gitignored golden set in `docs/eval/`, a data profile), not whose machine it runs on; a decision says what was chosen and why, not who chose it. No person's name, username or home path in docs, code, tests or commit messages.
- Commits: conventional commits, atomic (one coherent change each). Each commit is checked with the tests of the files it changes; the full suite and both clippy run once, on the last commit of a series, never on every commit. No `Co-Authored-By` and no AI attribution.
- `docs/migration-plan.md` is a working log and is NEVER committed. Stage by explicit path; never `git add -A`, `git add .` or `git add -u`.
- Do not run release builds, installs, or touch the real data profiles (`~/.clawg*`) without explicit authorization.
- Do not kill running `clawg.exe` processes; they may be the editor's own MCP.
- No test reads an instruction file for agents (this file, `AGENTS.md`, `.cursor/`, `.gemini/`).
- Prefer `rg` for searching and the Read/Edit/Write tools over shell text utilities.

## Versioning

- The version lives only in `[workspace.package] version` of the root `Cargo.toml`; every crate inherits it with `version.workspace = true`.
- Bump it when work closes: an archived openspec change raises the minor (`0.4.0` → `0.5.0`), a fix outside openspec raises the patch (`0.4.0` → `0.4.1`).
- After the edit, run `cargo update --workspace` and commit both files together as `chore: bump version to X.Y.Z`; CI builds with `--locked`.
- Merging a version bump to `main` publishes the release. Never create a tag or a release by hand, and never build a release locally to publish it.

## Testing

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -A clippy::all -D warnings
```

- On Windows add `--target-dir target-test` to both: live `clawg.exe` processes lock `target\debug\clawg.exe`.
- The bar is that the whole suite passes. Never record how many tests it has.
- Compiler warnings, `dead_code` and `unused` among them, are errors; `tests/conventions.rs` fails when a source hides dead code behind an `allow`.
- A test leaves nothing in the system temp dir. In clawg-core take the directory from `crate::test_dir::test_dir()` and close what the test opened (`MemoryRuntime::close`, `EvalIngest::finish`).
- Tests that run the binary use a temporary home (`CLAWG_TEST_HOME`), the fake model (`CLAWG_TEST_EMBEDDER=fake`) and `--port 0`; they need clawg-core's `testing` feature, which only `[dev-dependencies]` may enable.
- What the model sees of the MCP server is pinned in `crates/clawg-cli/src/mcp/agent_contract.json` and `agent_contract.es.json`. When a change to it is intended, run `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file` (or `the_spanish_agent_contract_matches_its_file`), review the diff and run the tests again.
- A ranking change is judged with `rag-eval --diff`; read its rules in `docs/reference/testing.md` before drawing a conclusion.

### Build directories

- Tests and clippy use one directory: `target`, or `target-test` on Windows. Never create another one (`target-apply`, `target/verify-win`, a directory outside the repository).
- Before testing a new version (another branch or openspec change, a version bump, another worktree's sources): `cargo clean --workspace --target-dir target-test`. After `rustup update` or a dependency change in `Cargo.lock`: `cargo clean --target-dir target-test`. Within one change's TDD loop the directory is reused, not cleaned.
- On Windows, `target` is cleaned only when no `clawg.exe` built there is running; never stop one to clean it.
- A directory a one-off comparison needs (a worktree arm, a baseline build) is deleted, with its worktree, by the task that made it.
- A task ends with no build directory other than `target` and `target-test`.
