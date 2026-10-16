# bridge-reconnect — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. La decisión pura (R1, D4)

- [x] Test `only_a_refused_token_a_refused_connection_or_an_unfinished_start_is_reresolved` (`mcp/startup.rs`): tabla con cada variante de `ClientError`.
- [x] Test `only_health_sets_a_timeout` (`client.rs`): una sola llamada con `.timeout(` en el fuente, la de `health`.
- [x] `on_failure` y `Next` en `startup.rs`.
- Commit: `feat(cli): decide which bridge failures a fresh daemon lookup can fix`

## 2. Releer y reintentar (R2, R3, R6, D1–D3, D5)

- [x] Helper de test: un daemon en proceso que publica `daemon.json` y `token` en un directorio y se puede detener (`Published`); un stub que cuenta pedidos y contesta 401 o 500 (`counting_stub`).
- [x] Tests `a_restarted_daemon_is_reached_with_its_new_token`, `a_refused_token_is_reread_once`, `an_error_the_daemon_answered_reaches_it_once`, `memory_status_after_a_restart_reports_the_new_daemon` (`mcp/server.rs`) y `a_daemon_that_comes_up_after_the_start_gave_up_is_used` (`mcp/server.rs`).
- [x] `ClientHandle` con `base_dir`, el cliente releído y `reresolve`; `pending` recibe `base_dir`.
- [x] `ClawgMcp::call` y cada `*_impl` pasa su cuerpo por él; dirección y versiones en el log.
- Commit: `fix(cli): reach a restarted daemon from an open MCP bridge`

## 3. Textos (R4, D5)

- [x] Tests `a_daemon_stopped_mid_session_is_reported_without_starting_one` y `the_refused_token_text_does_not_say_to_delete_it` (`mcp/server.rs`) (`a_daemon_that_did_not_start_points_at_status` sigue pasando sin cambios).
- [x] Ramas de `explain` para `NotRunning` y `Unauthorized`.
- Commit: `fix(cli): tell the agent how to get its daemon back`

## 4. Registro del proyecto (R5, D6)

- [x] Test `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` (`mcp/server.rs`).
- [x] `project_for` saca la raíz de `registered` ante una falla que se resuelve releyendo.
- Commit: `fix(cli): register the project again when its first registration never arrived`

## 5. Docs

- [x] `README.md`, «Si algo falla»: después de `clawg stop` o de actualizar, el puente encuentra el daemon nuevo solo; si no hay daemon, dice cómo arrancarlo.
- [x] `.claude/CLAUDE.md`: la línea de `clawg mcp`.
- [x] `openspec/README.md`: fila 39 y orden de lo pendiente.
- Commit: `docs: describe how the MCP bridge finds a restarted daemon`

## 6. Verificación

- [x] `cargo test --workspace` en cada commit; conteo contra la base.
- [x] `verify.md` con los pasos locales: la suite en Windows y, en el perfil dev, `clawg stop` y `clawg serve` con una sesión de Claude Code abierta.
- [x] Correr esos pasos.

## 7. Archive

- [x] `openspec/specs/mcp/spec.md`: R1 a R6 de acá como apartado nuevo y la línea **Origen**.
- [x] `git mv` a `changes/archive/AAAA-MM-DD-bridge-reconnect`, `state.yaml`, fila 39 y base de tests.
- [x] Subir el minor (`Cargo.toml` y `Cargo.lock`, con `cargo update --workspace`), en su propio commit.
