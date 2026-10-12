# mcp-protocol-upgrade — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda sin fallas nuevas en cada commit.

## 1. rmcp 3.5.1, `server/discover` y sesión sin `initialize` (R1, R2, R3)

- [x] Tests `a_discover_probe_is_answered_with_what_the_server_offers` y `a_session_without_initialize_lists_the_tools_in_name_order` en `handshake.rs`.
- [x] Tests del guardia reescritos para lo que queda de él: `before_session` (sonda, ping, `initialize`, request sin `initialize`, notificación, respuesta suelta), `the_probe_reaches_rmcp_and_its_cancellation_does_not`, `after_the_session_starts_the_guard_forwards_everything_untouched`, `a_closed_stream_before_a_session_ends_it` y `a_bare_probe_and_its_cancellation_leave_the_legacy_handshake_working`.
- [x] rmcp 1.x → 3.5.1: `ContentBlock`, `ServerConfig`, `id` opcional en `ServerJsonRpcMessage::error`. `PreInitGuard` sólo descarta notificaciones y respuestas antes de una sesión.
- Commit: `feat(mcp): serve the 2026-07-28 lifecycle on rmcp 3.5.1` (`60869fe`)

## 2. Identidad desde la request (R4, R5)

- [x] Tests puros de `agent_id_from` y tests por el cable `a_session_without_initialize_saves_under_the_client_it_declares`, `a_request_without_client_info_keeps_the_name_declared_on_discover` y `memories_saved_through_initialize_stay_visible_without_it` en `server.rs`.
- [x] `agent_id_from(declarado, recordado, fallback)`; las herramientas leen `RequestContext::client_info()`; el servidor recuerda el último nombre declarado, también en `server/discover`.
- Commit: `fix(mcp): name the agent from each request, not only the handshake` (`533523d`)

## 3. El guardia sigue prendido después de una request rechazada (R2)

- [x] Tests `a_request_refused_for_missing_meta_does_not_start_the_session` y `a_refused_request_leaves_the_guard_on` en `handshake.rs`; `after_the_session_starts_the_guard_forwards_everything_untouched` ahora contesta `initialize` antes de mandar la notificación.
- [x] `PreInitGuard` anota el id de la request que puede empezar la sesión y se apaga recién cuando rmcp la contesta con un resultado.
- Commit: `fix(mcp): keep the pre-session guard on after a refused request` (`08dc9ba`)

## 4. Merge con visibility-policy

- [x] `agent_id_from` pasa a `client_name(declarado, recordado)`, que alimenta `resolve_agent(flag, client)`. Las siete herramientas leen la identidad del `RequestContext`.
- Commit: `merge: bring main into the MCP upgrade` (`53db597`)

## Verify

- [x] `cargo test --workspace` en Linux: sin fallas nuevas contra la base (ver `verify.md`).
- [ ] `cargo test --workspace --target-dir <otro>` en Windows: pendiente, máquina de Marcos (ver `verify.md`).
- [ ] Verificación manual con Claude Code real contra el perfil dev: pendiente, máquina de Marcos (ver `verify.md`).

## Archive (cuando esté todo lo de verify)

- [ ] Crear `openspec/specs/mcp/spec.md` con R1–R5, con las líneas de `main` después del merge.
- [ ] Mover el cambio a `openspec/changes/archive/AAAA-MM-DD-mcp-protocol-upgrade/` y poner `phase: archived` y `main_spec` en `state.yaml`.
- [ ] Actualizar la fila 10 del mapa en `openspec/README.md`.
