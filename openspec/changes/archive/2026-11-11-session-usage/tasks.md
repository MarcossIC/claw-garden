# session-usage — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Cada tarea es un commit convencional y se comprueba con los tests de los archivos que cambia. La suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencias a planes, fases, cambios ni números de tarea.

Los requisitos están en la [spec](spec.md); las decisiones, en la [exploración](exploration.md#decisiones-2026-11-10) y en el [diseño](design.md).

El test de «Cómo se mide» de la parte A (`a_daemon_keeps_each_session_across_a_restart_and_reports_them`) se escribe primero y se corre en rojo antes de cualquier otro código. Pasa recién con la tarea 7, que es la última pieza que usa, así que entra en ese commit.

## 0. Antes del código

- [x] El problema de la exploración, verificado otra vez sobre `68b8851`.
- [x] `proposal.md`, `spec.md`, `design.md`, este archivo y `state.yaml`.
- Commit: `docs(openspec): propose session-usage`

## 1. Las reglas del registro (SU2, SU3, SU7; parte A)

- [x] Test de punta a punta en rojo: `a_daemon_keeps_each_session_across_a_restart_and_reports_them` (`tests/binary.rs`).
- [x] Tests en rojo en `crates/clawg-core/src/metrics/sessions.rs`: `a_session_opens_on_its_first_event_and_counts_what_follows`, `activity_of_an_unknown_session_is_dropped`, `a_name_outside_its_set_is_other`, `six_hours_without_activity_close_a_session_as_idle`, `a_closed_session_is_not_reopened`, `an_end_after_six_idle_hours_leaves_the_row_idle`, `merging_a_project_moves_its_sessions_and_deleting_it_drops_them`.
- [x] `SessionLog`, `SessionRow`, `IDLE_CLOSE_MS` y los conjuntos cerrados; `Recorder::sessions`, prendido y apagado por `set_writing`.
- Commit: `feat(metrics): keep a log of client sessions with their counts`

## 2. El esquema 3 (SU1, SU4, SU5, SU7)

- [x] Tests en rojo en `store.rs`: `a_file_of_schema_2_gets_its_sessions_and_keeps_its_rows`, `sessions_are_written_whole_and_read_back`, `open_sessions_past_six_hours_are_closed_on_start`, `sessions_past_the_retention_are_dropped`, `project_changes_reach_the_stored_sessions`.
- [x] `session` y `session_count`, `SCHEMA_VERSION = 3`, las lecturas y escrituras, y la poda.
- Commit: `feat(metrics): store the sessions in metrics.db, schema 3`

## 3. El escritor (SU4)

- [x] Tests en rojo en `daemon/metrics.rs`: `sessions_are_kept_with_each_minute_and_resumed_by_the_next_run`, `a_writer_turned_off_keeps_no_sessions`.
- [x] `Writer::start` retoma las filas abiertas; `Writer::flush` escribe las que cambiaron; `run_writer` escribe también cuando el registro lo despierta.
- Commit: `feat(daemon): write the sessions with each minute and resume them on start`

## 4. Las rutas del daemon (SU6, SU7; partes A y B del lado del daemon)

- [x] Tests en rojo en `tests/api.rs`: `a_hook_event_opens_the_row_of_its_session_and_counts`, `a_session_end_closes_its_row_with_the_reason`, `a_tool_call_counts_for_the_session_of_its_agent_and_project`, `an_accepted_save_and_an_empty_search_count_for_their_session`, `a_merge_moves_the_sessions_and_a_delete_drops_them`, `no_text_path_or_user_reaches_a_session_row`.
- [x] `POST /v1/sessions` con `event`, `source` y `client`; `POST /v1/sessions/end`; `POST /v1/sessions/calls` con `TOOLS` y `tool_label`; el save aceptado y la búsqueda vacía; la unión y el borrado.
- Commit: `feat(daemon): count hook events, tool calls, saves and empty searches per session`

## 5. Los hooks (SU9; partes A y D)

- [x] Tests en rojo: `a_hook_names_its_event_and_its_source`, `a_session_end_closes_the_session_and_prints_nothing`, `a_session_end_without_a_daemon_or_an_id_does_nothing` (`hook.rs`); `hook_takes_the_session_end_event` (`cli.rs`).
- [x] `HookEvent::SessionEnd`, `SESSION_END_DEADLINE`, y el evento, el `source` y el cliente en el registro de los otros dos.
- Commit: `feat(hook): name each event to the daemon and answer session-end`

## 6. El puente avisa cada llamada (SU8; parte B)

- [x] Tests en rojo en `mcp/server.rs`: `the_counted_tools_are_the_tools_of_the_bridge`, `a_tool_reports_its_call_to_the_session_of_the_open_folder`, `a_failed_tool_is_reported_as_failed`.
- [x] `ClientHandle::peek`, `DaemonClient::report_call`, y cada herramienta avisa al terminar, con el proyecto de la carpeta abierta y sin registrar nada. `the_agent_contract_matches_its_file` y su par en español pasan sin bendecir.
- Commit: `feat(mcp): tell the daemon which tool each call ran`

## 7. El reporte (SU11, SU12; parte C)

- [x] Tests en rojo: los de `session_report.rs`, `the_sessions_report_reads_the_file_without_a_daemon` y `one_session_is_listed_with_each_of_its_rows` (`metrics.rs`), `metrics_sessions_takes_a_grouping_an_age_and_a_session` (`cli.rs`).
- [x] `session_report::report`, `MetricsAction::Sessions` y `clawg metrics sessions`.
- [x] `a_daemon_keeps_each_session_across_a_restart_and_reports_them` pasa.
- Commit: `feat(cli): report the sessions with clawg metrics sessions`

## 8. `setup` instala el hook de cierre (SU10, SU12; parte D)

- [x] Tests en rojo: `a_run_over_an_install_without_the_session_end_hook_adds_only_it` (`setup.rs` y `setup/cursor.rs`), `a_local_run_over_an_install_without_the_session_end_hook_adds_only_it` (`setup/local.rs`), y los que listan los eventos instalados.
- [x] `SessionEnd` en `HOOK_EVENTS`; `sessionEnd` en Cursor.
- [x] `a_session_end_closes_the_session_and_what_follows_opens_another` (`tests/binary.rs`).
- Commit: `feat(setup): install the session-end hook in Claude Code and Cursor`

## 9. Docs

- [x] `docs/reference/commands.md` (`metrics sessions`, `hook session-end`, lo que instala `setup`), `docs/reference/daemon.md` (las tres rutas), `docs/reference/data-profiles.md` (el esquema 3, las tablas, la retención y cómo se vuelve a una versión anterior) y `README.md`.
- Commit: `docs: describe the session log, its report and the session-end hook`

## 10. Verificación

- [x] `cargo test --workspace` y `cargo clippy --workspace --all-targets -- -D warnings` en Linux, sobre el último commit.
- [ ] El CI del pull request en Linux y Windows; lo que marque se arregla.
- [x] `verify.md`, con lo que queda pendiente en local, y `state.yaml`.
- Commit: `docs(openspec): record session-usage applied on Linux`

## 11. Fuera de este cambio

- [ ] En local: lo pendiente de [verify](verify.md).
- [ ] La fila 121 del `README.md` de openspec y la línea del cambio en `ROADMAP.md`.
- [ ] Archivar: llevar SU1 a SU12 y el invariante enmendado a `specs/metrics`, las rutas a `specs/daemon-access`, y subir el minor.
