# memory-update-tool — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. La suite queda sin fallas nuevas en cada commit.

## 1. La regla del tier y el cambio en el core (D2, D3, D4, R1, R2, R3, R6)

La regla del tier iba a ser un commit propio (D6), pero sin llamador deja un warning de código muerto, así que va junto con el core.

- [x] Tests de `retier` en `tier.rs` (los siete de R2 y `a_tier_lost_to_inactivity_is_not_given_back_by_lowering`).
- [x] Tests de `MemoryPatch` (`a_change_needs_at_least_one_field`, `importance_outside_the_scale_is_refused`, `an_unknown_memory_type_is_refused`, `only_memory_rows_have_a_type_to_change`) y de `update_memory_core` (`an_update_writes_what_was_asked_and_counts_itself`, `an_update_leaves_status_accesses_and_disputes_alone`, `lowering_importance_takes_away_the_tier_it_gave`, `rows_from_the_previous_version_can_be_updated`) en `admin.rs`; los de `set_importance_core` pasan a `update_memory_core`.
- [x] `retier`; `MemoryPatch`, `PatchError`, `UpdatedRow` y `update_memory_core`, que reemplaza a `set_importance_core`. El `PATCH` pasa a usarla, con 400 para una importancia fuera de escala.
- Commit: `feat(core): let a memory's importance and type be corrected` (e1d3b70)

## 2. HTTP (D1, R4)

- [x] Tests `patch_changes_importance_and_type_and_reports_the_row` y `an_invalid_patch_changes_nothing` en `crates/clawg-cli/tests/api.rs`.
- [x] `PATCH` con `{importance?, memory_type?}`, `deny_unknown_fields`, 400 para lo inválido y la fila en la respuesta; `DaemonClient::update`.
- Commit: `feat(daemon): accept a memory type in the update endpoint` (5beee5e)

## 3. MCP (D5, R5)

- [x] Tests `update_memory_changes_an_own_memory_and_says_how_it_ended`, `an_agent_cannot_update_another_agents_memory` en `server.rs`; `format_update_names_the_fields_and_the_tier` en `tools.rs`; el texto del refuerzo; los conteos de herramientas en `server.rs` y `handshake.rs`.
- [x] Herramienta `update_memory`; `save_memory` la nombra en su descripción y en el refuerzo.
- Commit: `feat(mcp): let the agent correct a memory's importance and type` (6e9e826)

## 4. Docs

- [x] `.claude/CLAUDE.md` (la herramienta y el cuerpo del `PATCH`) y la lista de herramientas del `README.md`.
- Commit: `docs: list the update_memory tool` (475ec12)

## Verify

- [x] `cargo test --workspace` en Linux en cada commit, contra la base: sin fallas nuevas ([`verify.md`](verify.md#linux)).
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows (máquina de Marcos).
- [ ] `rag-eval --real --diff` contra el baseline: diff vacío (máquina de Marcos).
- [ ] Prueba manual con el perfil dev en Claude Code: bajar la importancia de una memoria en tier 1 y ver que sale del workspace; cambiar el tipo; intentar cambiar una de otro agente (máquina de Marcos).

## Archive

- [ ] `specs/update/spec.md` con R1–R6; `specs/mcp` con nueve herramientas; tests de R5 en `specs/visibility` R4.
- [ ] Carpeta a `changes/archive/AAAA-MM-DD-memory-update-tool/`, `archive: done` en `state.yaml` y fila 8 de `openspec/README.md`.
