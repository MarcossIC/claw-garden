# visibility-policy — Tareas

Partes A (identidad) y B (scope de lo que modifica). Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda sin fallas nuevas en cada commit.

## 1. Scope en el core (R4)

- [ ] Tests `an_owner_mismatch_deletes_nothing` y `an_owner_mismatch_changes_no_importance` en `admin.rs`; `a_pair_of_another_agent_is_not_found` en `dispute.rs`.
- [ ] `owner: Option<&str>` en `delete_memory_core`, `set_importance_core` y `resolve_dispute_core`; los llamadores actuales pasan `None`.
- Commit: `feat(core): scope delete, importance and dispute resolution to an agent`

## 2. Scope en el daemon y el cliente (R4)

- [ ] Tests HTTP `delete_with_another_agent_is_404_and_keeps_the_row`, `patch_with_another_agent_is_404_and_keeps_the_importance` y `resolve_with_another_agent_is_404_and_changes_nothing` en `crates/clawg-cli/tests/api.rs`.
- [ ] `?agent=` en `DELETE`/`PATCH`, `agent_id` en `ResolveRequest`; `DaemonClient::delete`, `set_importance` y `resolve` lo mandan cuando lo reciben.
- Commit: `feat(daemon): accept an agent scope on delete, patch and resolve`

## 3. El puente sólo toca lo propio (R3)

- [ ] Tests `an_agent_cannot_delete_another_agents_memory`, `an_agent_lists_only_its_own_memories` y `an_agent_cannot_resolve_another_agents_conflict` en `server.rs`.
- [ ] `list_memories`, `delete_memory` y `resolve_conflict` resuelven el agente del peer y lo pasan al daemon.
- Commit: `fix(mcp): keep each agent to its own memories when listing, deleting and resolving`

## 4. `--agent` (R1, R2, datos de la versión anterior)

- [ ] Tests `mcp_takes_an_agent` y `mcp_rejects_a_blank_agent` en `cli.rs`; `the_configured_agent_wins_over_the_client`, `without_a_flag_the_client_name_wins_over_the_fallback`, `a_configured_agent_keeps_its_memories_without_client_info`, `memory_status_names_the_agent_and_where_it_came_from` y `rows_and_workspace_of_a_client_name_survive_pinning_it` en `server.rs`; `format_status_names_the_agent` en `tools.rs`.
- [ ] Flag `--agent` en `clawg mcp`, `resolve_agent` puro, `memory_status` con el agente y su origen.
- Commit: `feat(mcp): let the user pin the agent identity with --agent`

## 5. Ruta del workspace (R5)

- [ ] Tests `a_safe_agent_id_is_its_own_segment`, `an_unsafe_agent_id_is_encoded_inside_the_root`, `encoding_never_makes_two_agents_share_a_folder` y `adopting_a_legacy_workspace_keeps_an_encoded_folder` en `workspace.rs`.
- [ ] `workspace_segment` y su uso en `workspace_path`; `adopt_legacy` arma la ruta desde la carpeta en disco.
- Commit: `fix(core): keep every agent's workspace inside the workspaces folder`

## 6. Documentación

- [ ] `CLAUDE.md`: `clawg mcp --agent`, el `agent` opcional de `DELETE`/`PATCH` y `agent_id` de resolve.
- Commit: `docs: document the agent flag and scope`

## Verify

- [ ] `cargo test --workspace` en Linux: sin fallas nuevas contra la base (ver `verify.md`).
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows: pendiente, máquina de Marcos.
- [ ] Prueba manual en el perfil dev: pendiente, máquina de Marcos (ver `verify.md`).

## Archive (cuando se cierre lo pendiente de verify)

- [ ] Crear `openspec/specs/visibility/spec.md` con R1–R5 de esta spec.
- [ ] Mover `changes/visibility-policy/` a `changes/archive/AAAA-MM-DD-visibility-policy/`; dejar en `changes/` una exploración para la parte C (compartir), con las preguntas 2, 3, 4, 7, 8 y 9 de esta exploración, y anotar la parte D.
- [ ] `state.yaml`: `phase: archived`, `archive: done`, `main_spec`.
- [ ] Fila 6 del mapa: archivado (A + B), con C y D pendientes; el backlog de `enum Scope` pasa a la parte C.
