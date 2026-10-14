# memory-update-tool — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. La suite queda sin fallas nuevas en cada commit.

## 1. La regla del tier (D3, R2)

- [ ] Tests `lowering_the_importance_that_gave_tier_1_takes_it_away`, `lowering_to_a_value_that_still_earns_the_tier_keeps_it`, `a_tier_earned_by_access_survives_lowering_the_importance`, `a_lost_tier_falls_only_to_what_the_rest_still_earns`, `a_type_that_gave_tier_1_takes_it_with_it`, `a_tier_the_rules_never_gave_survives_any_change` y `raising_still_promotes` en `tier.rs`.
- [ ] `retier` en `tier.rs`.
- Commit: `feat(core): work out the tier a corrected memory keeps`

## 2. El cambio en el core (D2, D4, R1, R3, R6)

- [ ] Tests de `MemoryPatch` (`a_change_needs_at_least_one_field`, `importance_outside_the_scale_is_refused`, `an_unknown_memory_type_is_refused`, `only_memory_rows_have_a_type_to_change`) y de `update_memory_core` (`an_update_writes_what_was_asked_and_counts_itself`, `an_update_leaves_status_accesses_and_disputes_alone`, `lowering_importance_takes_away_the_tier_it_gave`, `rows_from_the_previous_version_can_be_updated`) en `admin.rs`; los de `set_importance_core` pasan a `update_memory_core`.
- [ ] `MemoryPatch`, `PatchError`, `UpdatedRow` y `update_memory_core`, que reemplaza a `set_importance_core`.
- Commit: `feat(core): let a memory's importance and type be corrected`

## 3. HTTP (D1, R4)

- [ ] Tests `patch_changes_importance_and_type_and_reports_the_row` y `an_invalid_patch_changes_nothing` en `crates/clawg-cli/tests/api.rs`.
- [ ] `PATCH` con `{importance?, memory_type?}`, 400 para lo inválido y la fila en la respuesta; `DaemonClient::update`.
- Commit: `feat(daemon): accept a memory type in the update endpoint`

## 4. MCP (D5, R5)

- [ ] Tests `update_memory_changes_an_own_memory_and_says_how_it_ended`, `an_agent_cannot_update_another_agents_memory` en `server.rs`; `format_update_names_the_fields_and_the_tier` en `tools.rs`; el texto del refuerzo; los conteos de herramientas en `server.rs` y `handshake.rs`.
- [ ] Herramienta `update_memory`; `save_memory` la nombra en su descripción y en el refuerzo.
- Commit: `feat(mcp): let the agent correct a memory's importance and type`

## 5. Docs

- [ ] `.claude/CLAUDE.md`: la herramienta y el cuerpo del `PATCH`.
- Commit: `docs: list the update_memory tool`

## Verify

- [ ] `cargo test --workspace` en Linux en cada commit, contra la base (1.639 pasan, 10 fallan de entorno).
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows (máquina de Marcos).
- [ ] `rag-eval --real --diff` contra el baseline: diff vacío (máquina de Marcos).
- [ ] Prueba manual con el perfil dev en Claude Code: bajar la importancia de una memoria en tier 1 y ver que sale del workspace; cambiar el tipo; intentar cambiar una de otro agente (máquina de Marcos).

## Archive

- [ ] `specs/update/spec.md` con R1–R6; `specs/mcp` con nueve herramientas; tests de R5 en `specs/visibility` R4.
- [ ] Carpeta a `changes/archive/AAAA-MM-DD-memory-update-tool/`, `archive: done` en `state.yaml` y fila 8 de `openspec/README.md`.
