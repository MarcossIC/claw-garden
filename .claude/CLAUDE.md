# Claw Garden

Local-first memory for coding agents, written in Rust. It stores, classifies and retrieves memories and context, served by a loopback HTTP daemon and a stdio MCP bridge (Claude Code is the current client).

## Workspace

| Crate | Role |
| --- | --- |
| `crates/clawg-core` | Memory engine: SQLite storage, ONNX embeddings (fastembed), ranking, background workers, and the exclusive lock on `memory.db`. Library `clawg_core`. |
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
- Opening a store that has pending migrations first writes a `pre-migration` snapshot to `<data>/backups`; a store whose vectors were pooled by CLS (embedding contract `multilingual-e5-small`) gets a `pre-reembed` snapshot and is re-embedded by mean before memory is enabled, with the embedder status `loading` meanwhile; a store from a newer clawg, or with a diverged migration, is refused untouched.
- One process owns a profile's store at a time (exclusive lock on `memory.lock`; `memory.owner` records who holds it). The daemon also writes `daemon.json` and a bearer `token` in the data directory.
- `sharing.json` in the data directory holds which agents do not read which agents' memories (`crates/clawg-core/src/memory/sharing.rs`), `*` standing for every agent. Missing means every agent reads every other; an invalid file shares nothing. The daemon reads it on every search and context call.

## Commands

- `clawg serve [--profile P] [--port N]`: run the daemon (`--port 0` binds an ephemeral port).
- `clawg mcp [--profile P] [--no-autostart] [--agent NAME]`: stdio MCP bridge to the daemon; starts one if none is running unless `--no-autostart`. The agent id is `--agent`, else the client's `clientInfo.name`, else `mcp`.
- `clawg status [--profile P]`: print daemon state and health as JSON; exit 1 if not running.
- `clawg stop [--profile P]`: stop the daemon and wait for the process to exit.
- `clawg backup [--profile P]`: write a snapshot of `memory.db` to `<data>/backups` and print its path; goes through the daemon when one is running.
- `clawg restore <FILE> [--profile P]`: replace `memory.db` with a snapshot. The daemon must be stopped; the replaced store is snapshotted first.
- `clawg secrets [--profile P]`: list the `memory` rows, live or deleted, that hold a secret of a known format, by id and kind and never the secret. Read-only; works with the daemon running.
- `clawg purge <ID> --yes [--profile P]`: remove one `memory` row for good (text, vector, index terms, free pages and WAL) and name the backups that still hold it. The daemon must be stopped; no snapshot is taken.
- `clawg sharing [hide|unhide <READER> <AUTHOR>] [--profile P]`: show or change the sharing policy. Searches reach every other agent's memories, marked `by <author>` and read-only, unless `hide` keeps AUTHOR's from READER (`*` for every agent); it works with the daemon running and never opens the store.
- `clawg doctor [--profile P]`: print as JSON which `clawg.exe` files the PATH resolves to; exit 1 if the first is not the running binary or a second copy exists. Does not open the store.
- `clawg path add|remove [--dir D]`: edit the user PATH (Windows).

Daemon HTTP API: `GET /health` (open) and, behind the bearer token, `POST/GET /v1/memories`, `POST /v1/search`, `POST /v1/context` (both also reach other agents' memories unless `sharing.json` hides them from the caller; each search hit carries its `agent_id`), `PATCH/DELETE /v1/memories/{kind}/{id}` (`PATCH` takes `importance`, `memory_type` or both; `?agent=` limits them to that agent's rows), `GET /v1/memories/{kind}/{id}/save` (the save the row is a piece of, its pieces joined; same `?agent=` scope), `POST /v1/projects` (record a project root and move rows saved under its legacy ids; also indexes the root's docs in the background), `POST /v1/projects/docs/search` (a project's docs and code by meaning and by term; refreshes the index first when it is over a minute old), `POST /v1/backup`, `POST /v1/disputes/resolve` (optional `agent_id`, same scope), `POST /v1/synthesis` (memories no conclusion covers yet, and the conclusions already drawn; optional `agent_id`, same scope), `POST /v1/shutdown`.

A project's docs index (`crates/clawg-core/src/memory/project_docs/`) holds its Markdown and manifests (`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`) as passages split by heading, each under its breadcrumb (`path > heading > subheading`), and its Rust, Python and TypeScript code split along the syntax tree (tree-sitter), each piece under its path and the headers of the definitions that contain it (`path > impl Store`). It honours `.gitignore`, is derived from disk in its own tables and never touches `memory`; a file is re-read when its sha256 or the model changes.

Ingest masks secrets of known formats (`crates/clawg-core/src/memory/redact.rs`) before a save is chunked, hashed or embedded; a confirmed save lists the masked kinds in `redacted`.

A save longer than one piece is split at a smaller cap; every piece after the first is embedded behind the save's opening tokens, and `memory_piece` links the pieces (`crates/clawg-core/src/memory/pieces.rs`). Search keeps the best piece of each save and labels it `piece=K/N`.

MCP tools (`crates/clawg-cli/src/mcp/server.rs`): `save_memory`, `search_memory`, `recall_context`, `search_project`, `list_memories`, `read_memory`, `update_memory`, `delete_memory`, `resolve_conflict`, `synthesize`, `memory_status`.

## Code conventions

- Comments explain only a non-obvious why. No commented-out code, no `#[allow]` to hide dead code.
- Errors in library code are typed with `thiserror`.
- Unit tests sit next to the code in `#[cfg(test)] mod tests`; integration tests live in each crate's `tests/`.
- TDD: write the failing test first, then the code that makes it pass.
- Keep parsing/decision logic pure and separate from I/O so it is testable without a filesystem, port or database (see `cli.rs`, `Profile::resolve`).
- Do not add dependencies, visibility or abstractions without a real use.

## Rules for agents

- Code, comments, test names, error messages and commit messages never reference plans, phases or work packages.
- Commits: conventional commits, atomic (each one compiles and passes the suite). No `Co-Authored-By` and no AI attribution.
- `docs/migration-plan.md` is a working log and is NEVER committed. Stage by explicit path; never `git add -A`, `git add .` or `git add -u`.
- Do not run release builds, installs, or touch the real data profiles (`~/.clawg*`) without explicit authorization.
- Do not kill running `clawg.exe` processes; they may be the editor's own MCP.

## Testing

```
cargo test --workspace
```

Current baseline (Windows): 1863 tests, all passing.

On Windows, live `clawg.exe` processes (for example the editor's MCP bridge) lock `target\debug\clawg.exe` and make the build fail. Use a separate target directory instead: `cargo test --workspace --target-dir <other-dir>`.

## Claude Code

- Prefer `rg` for searching and the Read/Edit/Write tools over shell text utilities.
