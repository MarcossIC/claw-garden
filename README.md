# Claw Garden

Local-first memory for coding agents. Claw Garden stores, classifies and retrieves memories and context, and serves them to an agent (Claude Code today) through a local HTTP daemon and a stdio MCP bridge. Everything runs on your machine: SQLite for storage, ONNX embeddings for semantic search.

## Build

```
cargo build -p clawg-cli
```

This produces the `clawg` binary (`target/debug/clawg`; `target/release/clawg` with `--release`).

## Run

```
clawg serve --profile dev     # the memory daemon
clawg mcp --profile dev       # stdio MCP bridge; starts a daemon if none is running
clawg status --profile dev    # daemon state and health as JSON (exit 1 if not running)
clawg stop --profile dev      # stop the daemon
clawg doctor                  # which clawg the PATH resolves to (exit 1 if shadowed or duplicated)
```

`clawg mcp --no-autostart` fails instead of starting a daemon. `clawg serve --port <n>` overrides the default port (`0` picks a free one). `clawg path add|remove [--dir <dir>]` edits the user PATH on Windows.

## Register the MCP server in Claude Code

The server is a stdio command, so everything after `--` is the command Claude Code launches. Dev profile, pointing at a debug build:

```
claude mcp add clawg-dev -- /path/to/claw-garden/target/debug/clawg mcp --profile dev
```

On Windows use the full path to `clawg.exe`. For the prod profile (release build only) register it as `clawg`:

```
claude mcp add clawg -- /path/to/clawg mcp --profile prod
```

The names `clawg-dev` and `clawg` are the identities each profile uses.

## MCP tools

- `save_memory`: store a fact, preference or decision so it survives across sessions.
- `search_memory`: find stored memories related to a query, with ids, kinds and relevance scores.
- `recall_context`: get a ready-to-read background block (workspace notes, recent and long-term memories) for a query.
- `list_memories`: browse stored memories, newest first, without searching.
- `delete_memory`: delete one memory by id and kind.
- `memory_status`: report whether the daemon is running, its profile and data directory, and whether the embedding model has loaded.

## Profiles and data

| Profile | Data directory | Default port |
| --- | --- | --- |
| `dev` | `~/.clawg-dev` | 27381 |
| `prod` | `~/.clawg` | 27380 |

The profile comes from `--profile dev|prod`, then the `CLAWG_PROFILE` environment variable, then the build type (debug is `dev`, release is `prod`). A debug build refuses `prod`, so it can never touch `~/.clawg`. The daemon listens on 127.0.0.1 only; `/v1` endpoints require a bearer token stored in the data directory.

Each data directory holds the SQLite memory store plus `daemon.json` (pid, port, version), `token`, and `memory.lock` / `memory.owner`, which ensure a single process owns the store.

## Workspace

- `crates/clawg-core`: memory engine (storage, embeddings, ranking, background workers).
- `crates/clawg-cli`: the `clawg` binary and its daemon and MCP bridge.
- `crates/clawg-eval`: evaluation tools, `rag-eval` and `golden-mine`.
- `docs/`: design documents and examples.

## Test

```
cargo test --workspace
```

See `AGENTS.md` for contributor and agent conventions.
