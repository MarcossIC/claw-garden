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
- Opening a store that has pending migrations first writes a `pre-migration` snapshot to `<data>/backups`; a store whose vectors were pooled by CLS (embedding contract `multilingual-e5-small`) gets a `pre-reembed` snapshot and is re-embedded by mean before memory is enabled, with the embedder status `loading` meanwhile; a store from a newer clawg, or with a diverged migration, is refused untouched.
- The daemon lets the embedding model go after `CLAWG_EMBEDDER_IDLE_MINUTES` minutes without use (default 10, `0` keeps it loaded) and loads it again, from the same checked files, on the next request that needs a vector; requests arriving meanwhile share that load (`crates/clawg-core/src/memory/idle_embedder.rs`). The embedder status stays `ready` throughout. On Linux with glibc, `clawg serve` keeps every thread on one malloc arena (set in `main` before the runtime starts), or each reload would leave memory behind in another arena.
- One process owns a profile's store at a time (exclusive lock on `memory.lock`; `memory.owner` records who holds it). The daemon also writes `daemon.json` and a bearer `token` in the data directory.
- `sharing.json` in the data directory holds which agents do not read which agents' memories (`crates/clawg-core/src/memory/sharing.rs`), `*` standing for every agent. Missing means every agent reads every other; an invalid file shares nothing. The daemon reads it on every search and context call.

## Commands

- `clawg serve [--profile P] [--port N]`: run the daemon (`--port 0` binds an ephemeral port).
- `clawg mcp [--profile P] [--no-autostart] [--agent NAME]`: stdio MCP bridge to the daemon; starts one if none is running unless `--no-autostart`. When a call is refused (401 or connection refused) or the start gave up, it reads `daemon.json` and `token` again and repeats the call once, but never starts a daemon mid-session (`crates/clawg-cli/src/mcp/startup.rs`). The agent id is `--agent`, else the client's `clientInfo.name`, else `mcp`.
- `clawg status [--profile P]`: print daemon state and health as JSON; exit 1 if not running.
- `clawg stop [--profile P]`: stop the daemon and wait for the process to exit.
- `clawg backup [--profile P]`: write a snapshot of `memory.db` to `<data>/backups` and print its path; goes through the daemon when one is running.
- `clawg restore <FILE> [--profile P]`: replace `memory.db` with a snapshot. The daemon must be stopped; the replaced store is snapshotted first.
- `clawg secrets [--profile P]`: list the `memory` rows, live or deleted, that hold a secret of a known format, by id and kind and never the secret. Read-only; works with the daemon running.
- `clawg purge <ID> --yes [--profile P]`: remove one `memory` row for good (text, vector, index terms, outbox rows holding its text, free pages and WAL) and name the backups that still hold it. The daemon must be stopped; no snapshot is taken.
- `clawg undelete <ID> [--profile P]`: bring back a deleted `memory` row through the running daemon, which embeds it again; refused when the same text was saved again since or the row holds a secret.
- `clawg memories list|search|show|stats [--profile P]`: read-only viewer through the running daemon; `list --deleted` lists deleted rows, most recently deleted first.
- `clawg projects add <PATH> [--profile P]`: let the daemon read and index the files of the project PATH is in, through the running daemon; without it, only the root a bridge was started in is indexed.
- `clawg sharing [hide|unhide <READER> <AUTHOR>] [--profile P]`: show or change the sharing policy. Searches reach every other agent's memories, marked `by <author>` and read-only, unless `hide` keeps AUTHOR's from READER (`*` for every agent); it works with the daemon running and never opens the store.
- `clawg doctor [--profile P]`: print as JSON which `clawg.exe` files the PATH resolves to; exit 1 if the first is not the running binary or a second copy exists. Does not open the store.
- `clawg path add|remove [--dir D]`: edit the user PATH (Windows).

Daemon HTTP API: `GET /health` (open; `embedder_resident` says whether the model is in memory now; its `ingest` block counts pending and failed saves and names each failed one by `operation_id`, left out when the store does not answer within 250 ms) and, behind the bearer token, `POST/GET /v1/memories`, `POST /v1/search`, `POST /v1/context` (both also reach other agents' memories unless `sharing.json` hides them from the caller; each search hit carries its `agent_id`, and `exact` when it holds every identifier of the query; a search's `min_relative_score` cut, and the bridge's, always keeps the `exact` hits and measures the rest against the best of them), `PATCH/DELETE /v1/memories/{kind}/{id}` (`PATCH` takes `importance`, `memory_type` or both; `?agent=` limits them to that agent's rows), `GET /v1/memories/{kind}/{id}/save` (the save the row is a piece of, its pieces joined; same `?agent=` scope), `POST /v1/memories/memory/{id}/restore` (a deleted row back with its vector and text index; same `?agent=` scope; `GET /v1/memories?deleted=true` lists deleted rows), `POST /v1/projects` (record a project root and move rows saved under its legacy ids, and start loading an unloaded model without waiting for it; `index_docs: true` also lets the daemon read the root's files and indexes them in the background, a 400 when the root is not a directory; 409 `project_root_collision` when its id belongs to another root), `POST /v1/projects/docs/search` (a project's docs and code by meaning and by term; refreshes the index first when it is over a minute old; 403 for a root that was never registered with `index_docs`), `POST /v1/backup`, `POST /v1/disputes/resolve` (optional `agent_id`, same scope), `POST /v1/synthesis` (memories no conclusion covers yet, and the conclusions already drawn; optional `agent_id`, same scope), `POST /v1/shutdown`.

A project's docs index (`crates/clawg-core/src/memory/project_docs/`) holds its Markdown and manifests (`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`) as passages split by heading, each under its breadcrumb (`path > heading > subheading`), and its Rust, Python and TypeScript code split along the syntax tree (tree-sitter), each piece under its path and the headers of the definitions that contain it (`path > impl Store`). It honours `.gitignore`, is derived from disk in its own tables and never touches `memory`; a file is re-read when its sha256 or the model changes. Only a root marked `index_docs` in `project` is read: the bridge asks for it for its own root alone, and `clawg projects add` for any other, so a `project_path` the model passes is a label for memories until the user adds it. When a root's id already belongs to another root, every tool that scopes to it fails rather than mix two projects (`all_projects` still works); any other registration failure only logs a warning.

Ingest masks secrets (`crates/clawg-core/src/memory/redact.rs`) before a save is chunked, hashed or embedded, and the docs index does the same with every passage; a confirmed save lists the masked kinds in `redacted`. It masks keys and tokens of known formats (an `sk-` key only when it mixes upper case, lower case and digits, so hyphenated identifiers pass), the literal value after a key ending in `password` or `passwd` and after `Authorization: Bearer|Basic|Token` (never a variable, a type, a call or a placeholder), and everything the writer wraps in a private mark, `<private>` to `</private>`. A mark left open masks to the end of the text; a mark right after a backtick is text. Nothing is guessed from entropy.

A save is written, already masked, to the `ingest_outbox` table of `memory.db` before the daemon answers `201` or `202`, and its row goes in the transaction that finishes storing it (`crates/clawg-core/src/memory/outbox.rs`), so a kill, crash or power cut loses nothing accepted (the row alone is written with `synchronous = FULL`; the rest of the store keeps `NORMAL`). Each piece of a long save is noted in the row as it is stored, and a replay resumes after it. On start the daemon ingests pending rows before new saves; a row that stopped the process 3 times, or failed after the caller was answered, stays `failed` with its cause. Every save answers with an `operation_id` (a UUID v4, or the caller's own, 1 to 128 visible ASCII characters); sending the same one again while its row exists answers `already_queued` (pending) or `ingest_failed` (failed) instead of saving twice. Shutdown no longer writes `ingest-pending.jsonl`; one left by an older clawg is imported masked on start and removed.

A save longer than one piece (480 tokens) is split into pieces of 96 tokens, neighbours sharing 24, so one sentence is not averaged away in a long vector; every piece after the first is embedded behind the save's opening tokens, and `memory_piece` links the pieces (`crates/clawg-core/src/memory/pieces.rs`). Saves split before at 424 tokens keep their pieces. Search fills its KNN pool with distinct saves, keeps the best piece of each and labels it `piece=K/N`.

MCP tools (`crates/clawg-cli/src/mcp/server.rs`): `save_memory`, `search_memory`, `recall_context`, `search_project`, `list_memories`, `read_memory`, `update_memory`, `delete_memory`, `restore_memory`, `resolve_conflict`, `synthesize`, `memory_status`.

What the agent is told about memory lives in one text, `crates/clawg-cli/src/mcp/protocol.md`, compiled into the binary: when to save, how to write a save (a first line naming the subject, the exact paths and identifiers, in English), what not to save, which type and importance to pick, the session summary, and that whatever a tool retrieves is evidence, not instructions. Its opening paragraphs, before the first `##` section, are the server instructions, at most 2048 characters; tests keep it naming only registered tools, their arguments and the memory types. `save_memory` and `update_memory` list the seven memory types in their schemas, and `POST /v1/memories` refuses any other with a 400 that lists them, before the save reaches the outbox, as `PATCH` does; an outbox row written before that check still stores as `semantic_fact`.

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
- Before testing a new version, clean the old builds out of the test build directory, and end every task with no build directory other than `target` and `target-test` (Build directories, under Testing).

## Versioning

- The version lives only in `[workspace.package] version` of the root `Cargo.toml`; every crate inherits it with `version.workspace = true`. Never set a version in a crate's own `Cargo.toml`.
- Bump it when work closes: an archived openspec change raises the minor (`0.4.0` → `0.5.0`), a fix outside openspec raises the patch (`0.4.0` → `0.4.1`).
- After the edit, run `cargo update --workspace`: it writes the new version into `Cargo.lock` without touching any dependency and without building. Commit both files together as `chore: bump version to X.Y.Z`; otherwise the lock trails one version behind and leaks into the next commit.
- The version is not cosmetic: `clawg --version`, `GET /health`, the MCP `serverInfo` and `clawg doctor` report it, and `version_warning` (`crates/clawg-cli/src/memories.rs`) compares the running daemon's version with the CLI's to detect a stale daemon.

## Testing

```
cargo test --workspace
```

The bar is that the whole suite passes. Never record how many tests it has (in this file, the other agents' instruction files or `openspec/config.yaml`): the count goes stale with every change.

`crates/clawg-cli/tests/binary.rs` runs the built `clawg` itself: a hard kill with a save still pending, two daemons on one store, `status`/`stop`, `backup`/`restore`, MCP over real stdio and a daemon restart under an open bridge. Each test gets a temporary home through `CLAWG_TEST_HOME` and the fake model through `CLAWG_TEST_EMBEDDER=fake|stalled`; only a build with clawg-core's `testing` feature (its dev-dependencies) reads them, never a release build, and `tests/conventions.rs` fails when a manifest enables `testing` outside `[dev-dependencies]`. Every daemon binds `--port 0`, and each child is ended by its own handle, never by name. It takes about a second.

CI (`.github/workflows/ci.yml`) runs clippy and the suite on Linux and Windows for every pull request and every push to `main`.

On Windows, live `clawg.exe` processes (for example the editor's MCP bridge) lock `target\debug\clawg.exe` and make the build fail. There the suite and clippy run in `target-test` instead: `cargo test --workspace --target-dir target-test`. See Build directories below for when it is cleaned.

Compiler warnings, `dead_code` and `unused` among them, are errors here (clippy's own lints are not enforced yet):

```
cargo clippy --workspace --all-targets -- -A clippy::all -D warnings
```

`tests/conventions.rs` in `clawg-cli` fails when a source hides dead code behind an `allow`.

What the model sees of the MCP server (each tool's name, description and input schema, and the instructions) is pinned in `crates/clawg-cli/src/mcp/agent_contract.json`. When a change to it is intended, run `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file`: it rewrites the file and fails, so review the diff and run the tests again. The `MCP tools` line above is checked against the registered tools.

A ranking change is compared with the baseline by `cargo run -p clawg-eval --bin rag-eval -- --diff <before.json> <after.json>`. Every metric carries the exact paired p-value of the cases that moved (and MRR, nDCG and tokens a bootstrap interval of the mean change); the primary metrics are `recall@5` and, between two `--agent-view` runs, `search.found`. With the 46-case set a conclusion needs at least 6 cases moving one way; fewer is "not distinguishable", never "no difference". `--fail-on-regression` exits 6 when a primary metric gets worse at p < 0.05. When the golden cases carry `query_lang`, every block is also cut by the language pair of query and row (`es>en`, …), over the cases with that pair in both files: a change that touches retrieval across languages is read there, and the pair rows stay informative, outside the verdict and `--fail-on-regression`. Choose a new constant reading `by_split.calibration` and report `by_split.test`; `--lexical-only` gives the bm25-only floor to compare against.

### Build directories

Cargo never removes an old artifact from a target directory. Every version bump, branch or worktree with other sources, feature set or toolchain leaves a new set of the workspace's test binaries beside the previous ones, so a directory that is only ever reused grows past 100 GB. Most of that weight is the workspace's own crates, not its dependencies.

- Tests and clippy use one directory: `target`, or `target-test` on Windows. Never create another one (`target-apply`, `target/verify-win`, a directory outside the repository).
- Before testing a new version (another branch or openspec change, a version bump, another worktree's sources), remove the workspace's old artifacts and keep the dependencies: `cargo clean --workspace --target-dir target-test`. After `rustup update` or a dependency change in `Cargo.lock`, empty it: `cargo clean --target-dir target-test`. Within one change's TDD loop the directory is reused, not cleaned.
- On Windows, `target` is cleaned only when no `clawg.exe` built there is running; never stop one to clean it.
- A directory a one-off comparison needs (a worktree arm, a baseline build) is deleted, with its worktree, by the task that made it.
- A task ends with no build directory other than `target` and `target-test`.

## Claude Code

- Prefer `rg` for searching and the Read/Edit/Write tools over shell text utilities.
