# agent-sharing — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. La suite queda sin fallas nuevas en cada commit.

## 1. Borrar `enum Scope` (R9)

- [ ] Borrar `enum Scope` de `record.rs` y su reexport de `memory/mod.rs`.
- Commit: `refactor(core): drop the unused Scope enum`

## 2. La política (D1, D2, R1, R7)

- [ ] Tests de `SharingPolicy` en `sharing.rs` (los de R1 y los de la parte pura de R7) y `an_unreadable_policy_shares_nothing` sobre un directorio temporal.
- [ ] `paths::sharing_file`; `SharingPolicy`, `SharingError`, `load` y `save`.
- Commit: `feat(core): read and write the sharing policy`

## 3. Búsqueda con lo concedido (D3, D4, R2, R3)

- [ ] Tests de R2 y `a_hit_carries_its_author` en `search.rs`; el del plan con `json_each`; `contradiction_probe_ignores_granted_authors` en `contradiction.rs`.
- [ ] `SearchScope::shared_from` y `with_shared`; el predicado `IN (SELECT value FROM json_each(?N))` en las siete consultas; `agent_id` en `RawHit` y `RankedHit`.
- Commit: `feat(core): let a search reach the memories of granted agents`

## 4. El autor en `recall_context` (D6, R4, R5)

- [ ] Tests `a_shared_hit_names_its_author`, `an_own_hit_keeps_its_line`, `the_shared_note_appears_only_with_shared_hits` en `prompt_builder.rs`.
- [ ] `build_memory_prompt` recibe los autores concedidos; `format_hit_line` con el lector; bloque `SHARED` con `SHARED_NOTE`.
- Commit: `feat(core): name the author of a shared memory in the context`

## 5. Daemon (R1, R3)

- [ ] Tests `search_returns_the_author_of_each_hit` y los de la política en `/v1/search` y `/v1/context` en `crates/clawg-cli/tests/api.rs`; `a_hit_from_an_old_daemon_reads_as_own` en `client.rs`.
- [ ] `/v1/search` y `/v1/context` leen la política del directorio de datos; `Hit::agent_id` con `serde(default)`.
- Commit: `feat(daemon): apply the sharing policy to search and context`

## 6. Puente MCP (D6, D7, R4, R5, R6, R8)

- [ ] Tests de `format_search` y `format_status` en `tools.rs`; los de R4, R5, R6 y R8 en `server.rs`.
- [ ] `search_memory` marca lo ajeno; `memory_status` con la línea `shared:`; descripciones de `delete_memory`, `update_memory` y `resolve_conflict`.
- Commit: `feat(mcp): mark shared memories and say whom the agent reads`

## 7. CLI (D7, R7)

- [ ] Tests `sharing_shows_grants_and_revokes` y `sharing_rejects_a_blank_or_self_grant` en `cli.rs`.
- [ ] `clawg sharing [grant|revoke]`.
- Commit: `feat(cli): manage the sharing policy with clawg sharing`

## 8. Docs

- [ ] `.claude/CLAUDE.md` (comando, archivo de datos, API) y `README.md`.
- Commit: `docs: describe sharing between agents`

## Verify

- [ ] `cargo test --workspace` en Linux en cada commit, contra la base (1.674 pasan, 10 fallas de entorno).
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows.
- [ ] `rag-eval --real --diff` contra el baseline: diff vacío en el ancla y en agent-view.
- [ ] Prueba manual con el perfil dev y dos agentes (`clawg mcp --agent a` y `--agent b`): sin política B no ve lo de A; `clawg sharing grant b a` y B lo ve con `by a`; B no lo puede borrar; `clawg sharing revoke b a` y deja de verlo, sin reiniciar el daemon.

## Archive

- [ ] `openspec/specs/sharing/spec.md` con R1–R9, las rutas y líneas del código y la línea **Origen**.
- [ ] `specs/visibility`: la introducción enlaza a `specs/sharing`; R3 dice que lo compartido sigue siendo "no existe" para modificar.
- [ ] `specs/retrieval` y `specs/mcp`: el autor en cada hit y la línea `shared:` de `memory_status`.
- [ ] `git mv openspec/changes/agent-sharing openspec/changes/archive/AAAA-MM-DD-agent-sharing`.
- [ ] `state.yaml`: `phase: archived`, `archive: done`, `main_spec: openspec/specs/sharing/spec.md`.
- [ ] `openspec/README.md`: fila 16 como archivada (parte C), con la parte D pendiente; el ítem de `enum Scope` sale del backlog.
