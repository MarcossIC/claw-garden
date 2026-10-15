# agent-sharing — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. La suite queda sin fallas nuevas en cada commit.

Las tareas 1 a 8 se hicieron con la primera versión de la propuesta (nada compartido hasta que el usuario concede). La 9 y la 10 llevan todo a "todo compartido, el usuario excluye", que es lo que eligió Marcos; los nombres de commit de 2 a 8 quedan como se publicaron.

## 1. Borrar `enum Scope` (R9)

- [x] Borrar `enum Scope` de `record.rs` y su reexport de `memory/mod.rs`.
- Commit: `refactor(core): drop the unused Scope enum`

## 2. La política (D1, D2)

- [x] `paths::sharing_file`; `SharingPolicy`, `SharingError`, `load` y `save`, con sus tests sobre un directorio temporal.
- Commit: `feat(core): read and write the sharing policy`

## 3. Búsqueda con lo ajeno (D3, D4, R3)

- [x] Un parámetro de agentes visibles en las siete consultas; `agent_id` en `RawHit` y `RankedHit`; `a_hit_carries_its_author`; el detector de contradicciones queda en lo propio.
- Commit: `feat(core): let a search reach the memories of granted agents`

## 4. El autor en `recall_context` (D6, R4, R5)

- [x] Tests `a_shared_hit_names_its_author`, `an_own_hit_keeps_its_line`, `the_shared_note_appears_only_with_shared_hits`, `the_workspace_and_conflicts_stay_own` en `prompt_builder.rs`.
- [x] `build_memory_prompt` recibe el alcance; `format_hit_line` con el lector; bloque `SHARED` con `SHARED_NOTE`.
- Commit: `feat(core): name the author of a shared memory in the context`

## 5. Daemon (R1, R3)

- [x] Tests `search_returns_the_author_of_each_hit` y los de la política en `/v1/search` y `/v1/context` en `crates/clawg-cli/tests/api.rs`; `a_hit_from_an_old_daemon_reads_as_own` en `client.rs`.
- [x] `/v1/search` y `/v1/context` leen la política del directorio de datos; `Hit::agent_id` con `serde(default)`.
- Commit: `feat(daemon): apply the sharing policy to search and context`

## 6. Puente MCP (D6, D7, R4, R5, R6, R8)

- [x] Tests de `format_search` y `format_status` en `tools.rs`; los de R4, R5, R6 y R8 en `server.rs`.
- [x] `search_memory` marca lo ajeno; `memory_status` con la línea `shared:`; descripciones de `delete_memory`, `update_memory` y `resolve_conflict`.
- Commit: `feat(mcp): mark shared memories and say whom the agent reads`

## 7. CLI (D7, R7)

- [x] `clawg sharing` con sus subcomandos, parseo en `cli.rs` y ejecución en `sharing.rs`.
- Commit: `feat(cli): manage the sharing policy with clawg sharing`

## 8. Docs

- [x] `.claude/CLAUDE.md` (comando, archivo de datos, API) y `README.md`.
- Commit: `docs: describe sharing memories between agents`

## 9. Todo compartido, el usuario excluye (R1, R2, R6, R7, R8)

- [x] Tests nuevos de R1 en `sharing.rs` (`a_missing_policy_shares_everything`, `every_agent_stands_for_either_side`, …), de R2 en `search.rs` (`a_hidden_author_stays_out_in_every_channel`, `an_agent_never_hides_its_own_rows`, `the_eligible_count_follows_the_reach`, …), de R7 en `cli.rs` y `sharing.rs` de la CLI; los del daemon y el puente pasan a esperar lo ajeno sin política y a ocultarlo con `hide`.
- [x] `Reach` y `reach_for`; `hides` en el archivo con `*`; `SearchScope::reach` y el predicado con `->>` y `json_each(?N, '$.hidden')`; el daemon da `Own` ante un archivo inválido; `memory_status` dice `reads every agent [but …]`; `clawg sharing hide|unhide`. Sale el test que fijaba el plan con el índice por agente (D3).
- Commit: `feat!: share every agent's memories unless the user hides them`

## 10. Docs

- [x] `README.md` ("Compartir memorias entre agentes" y la introducción de la identidad del agente) y `.claude/CLAUDE.md`.
- Commit: `docs: describe sharing by default and how to hide an agent`

## Verify

- [x] `cargo test --workspace` en Linux en cada commit, contra la base (1.674 pasan, 10 fallas de entorno). Ver [`verify.md`](verify.md).
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows.
- [ ] `rag-eval --real --diff` contra el baseline: diff vacío en el ancla y en agent-view.
- [ ] Prueba manual con el perfil dev y dos agentes (`clawg mcp --agent a` y `--agent b`): sin política B ve lo de A con `by a`; B no lo puede borrar; `clawg sharing hide b a` y deja de verlo, sin reiniciar el daemon; `clawg sharing unhide b a` y vuelve.

## Archive

- [ ] `openspec/specs/sharing/spec.md` con R1–R9, las rutas y líneas del código y la línea **Origen**.
- [ ] `specs/visibility`: la introducción enlaza a `specs/sharing`; R3 dice que lo compartido sigue siendo "no existe" para modificar.
- [ ] `specs/retrieval` y `specs/mcp`: el autor en cada hit y la línea `shared:` de `memory_status`.
- [ ] `git mv openspec/changes/agent-sharing openspec/changes/archive/AAAA-MM-DD-agent-sharing`.
- [ ] `state.yaml`: `phase: archived`, `archive: done`, `main_spec: openspec/specs/sharing/spec.md`.
- [ ] `openspec/README.md`: fila 16 como archivada (parte C), con la parte D pendiente.
