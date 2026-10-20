# Claw Garden

Local-first memory for coding agents, written in Rust. It stores, classifies and retrieves memories and context, served by a loopback HTTP daemon and a stdio MCP bridge (Claude Code is the current client).

## Workspace

| Crate | Role |
| --- | --- |
| `crates/clawg-core` | Memory engine: SQLite storage, ONNX embeddings (ONNX Runtime), ranking, background workers, and the exclusive lock on `memory.db`. Library `clawg_core`. |
| `crates/clawg-cli` | The `clawg` binary: `serve`, `mcp`, `status`, `stop`, `doctor`, `path`. Also a library `clawg_cli`. |
| `crates/clawg-eval` | Evaluation tools: `rag-eval` (retrieval metrics over a golden set) and `golden-mine` (golden-set miner). They use `clawg-core` through its `eval` feature (`eval_api`), which production builds never link. |

Design documents live in `docs/`.

## Data profiles

`clawg` runs against one of two isolated profiles, resolved in `crates/clawg-core/src/paths.rs`:

| Profile | Data directory | Default port | MCP server name |
| --- | --- | --- | --- |
| `dev` | `~/.clawg-dev` | 27381 | `clawg-dev` |
| `prod` | `~/.clawg` | 27380 | `clawg` |

- Selection order: `--profile dev|prod`, then `CLAWG_PROFILE`, then the build type (`dev` for debug, `prod` for release).
- A debug build refuses `prod`, from both the flag and the env var. A debug binary must never open `~/.clawg`.
- One process owns a profile's store at a time (exclusive lock on `memory.lock`; `memory.owner` records who holds it). The daemon also writes `daemon.json` and a bearer `token` in the data directory.

## Commands

- `clawg serve [--profile P] [--port N]`: run the daemon (`--port 0` binds an ephemeral port).
- `clawg mcp [--profile P] [--no-autostart]`: stdio MCP bridge to the daemon; starts one if none is running unless `--no-autostart`.
- `clawg status [--profile P]`: print daemon state and health as JSON; exit 1 if not running.
- `clawg stop [--profile P]`: stop the daemon and wait for the process to exit.
- `clawg doctor [--profile P]`: print as JSON what is wrong with the install, the data directory and the store (`crates/clawg-cli/src/doctor/`): the fields of before (which `clawg.exe` files the PATH resolves to: `installs`, `winner`, `shadowed`, `duplicates`) and `data_dir`, `checks` and `summary`. Each check gives at least one finding, `ok` when it found nothing, each with `check_id`, `severity` (`ok`, `info`, `warn`, `error`), `reason_code`, `message`, `evidence` (ids, counts, paths and names, never a row's text, a verdict reason or a `topic_key`), `safe_next_step` (a command or null) and `requires_confirmation` (the step changes stored data). It checks the PATH, the data directory's mode (Unix), whether the socket path fits, who holds the store, the schema, the embedding contract against the one `clawg serve` verifies (without loading a model), the age of the newest snapshot by its name (`warn` past 30 days, or with none for a store with rows), the spill files an older clawg left, `sharing.json`, failed and waiting outbox rows, the agents and their rows (`warn` for live rows under the fallback `mcp`), `project` ids against their root, six rules of `memory` no schema imposes (one live save per topic key, `superseded_by` and `revived_from` naming a row, no two live rows with one `content_hash` in a scope, no save keeping the opening of a purged first piece, a `disputed` row with a live rival) and Claude Code's registration of the profile's server in the `.claude.json` `setup` reads. The store is read without its lock and without writing (`clawg_core::memory::inspect`, through `open_for_probe`), beside a running daemon or not; a newer or diverged store is read no further than its history. Writes nothing, repairs nothing. Exit 1 only with an `error` finding (a shadowed or duplicated PATH among them), 0 otherwise.
- `clawg path add|remove [--dir D]`: edit the user PATH (Windows).

Daemon HTTP API: `GET /health` (open) and, behind the bearer token, `POST/GET /v1/memories`, `POST /v1/search`, `POST /v1/context`, `PATCH/DELETE /v1/memories/{kind}/{id}`, `POST /v1/shutdown`.

MCP tools (`crates/clawg-cli/src/mcp/server.rs`): `save_memory`, `search_memory`, `recall_context`, `list_memories`, `delete_memory`, `memory_status`.

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
- Commits: conventional commits, atomic (each one compiles and passes the suite). No `Co-Authored-By` and no AI attribution.
- `docs/migration-plan.md` is a working log and is NEVER committed. Stage by explicit path; never `git add -A`, `git add .` or `git add -u`.
- Do not run release builds, installs, or touch the real data profiles (`~/.clawg*`) without explicit authorization.
- Do not kill running `clawg.exe` processes; they may be the editor's own MCP.

## Testing

```
cargo test --workspace
```

On Windows, live `clawg.exe` processes (for example the editor's MCP bridge) lock `target\debug\clawg.exe` and make the build fail. Use a separate target directory instead: `cargo test --workspace --target-dir <other-dir>`.
