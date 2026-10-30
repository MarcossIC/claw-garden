# import-job — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite. El orden es el de [design D11](design.md#d11-orden-de-los-commits); los IJ son los de la [spec](spec.md). Las pruebas se corren en `target-test` (`cargo test --workspace --target-dir target-test`). Las líneas de `.claude/CLAUDE.md` que describen un comportamiento cambian en el commit que lo cambia, y la tarea 10 revisa que no quede ninguna sin cambiar.

## 1. La guarda del import síncrono (IJ8, IJ11)

- [x] Guarda, escrita primero y **no un RED**: pasa con el código de hoy y debe seguir pasando. `the_synchronous_import_still_cancels_when_the_client_goes` (`crates/clawg-cli/tests/api.rs`): con el embedder que retiene, la tarea del cliente se aborta tras `entered()`; `http.in_flight` vuelve a 0 sin soltar el lote, y no hay filas ni `memory_import`.
- [x] Lo que la guarda necesita para existir: `GatedEmbedder` y `Gate` (`crates/clawg-core/src/memory/embedder.rs`, bajo `cfg(any(test, feature = "testing"))`) y `EmbedderState::Gated` en el `Harness` de `tests/api.rs`.
- Commit: `test(import): a synchronous import is cut when its client goes`

## 2. El progreso de una importación (IJ5, IJ1, IJ10)

- [x] Tests primero (fallan por no compilar: no existe `import_parsed` ni `ImportControl`):
  - `a_clawg_file_is_parsed_once_for_a_job` (IJ1, IJ10; `crates/clawg-core/src/memory/portability/import.rs`): `import_parsed` sobre un `Parsed` da el mismo informe que `import` sobre el texto, y no tiene texto que parsear.
  - `the_counters_do_not_depend_on_the_batch_sizes` (IJ5; `import.rs`): archivos de 31, 32, 33, 255, 256 y 257 filas.
  - `the_counters_of_a_finished_import_match_its_report` (IJ5; `import.rs`): `rows_embedded == rows_to_embed`, `rows_written == report.rows`, `notes_written == notes`, para un archivo de clawg y uno de otra herramienta con notas largas en piezas.
  - `batch_notes_counts_the_notes_of_each_write_batch` (IJ5; `import.rs`, función pura junto a `note_batches`).
- [x] `crates/clawg-core/src/memory/portability/control.rs` con `ImportControl`, `Counters`, `Progress` y `begin`; `ImportInput` e `import_parsed`; `prepare_file` pasa a `prepare_parsed` sin parseo adentro; `batch_notes`; `Step::name` pública (`timing.rs`). `import` e `import_foreign` envuelven a `import_parsed`, todavía públicas.
- Commit: `feat(import): an import reports its step and its counters as it runs`

## 3. Las paradas de una importación (IJ6, IJ7, IJ18)

- [x] Tests primero (fallan por no compilar o por la aserción):
  - `the_first_stop_cause_wins` (`control.rs`, sobre `ImportControl::stop`).
  - `each_step_of_an_import_stops_where_the_table_says` (IJ6; `import.rs`): `on_step` levanta `Cancel` al entrar en cada paso; se mira `Stopped.step`, las filas, `memory_import` y los archivos de `backups`.
  - `an_abandoned_embedding_batch_does_not_block_the_stop` (IJ6; `import.rs`, `GatedEmbedder` sin permisos).
  - `a_stop_between_write_batches_returns_the_committed_ledger` (IJ7; `import.rs`): `inserted` contra `memory_import_row`, `finished_at` nulo, `store.importing` vacío.
  - `an_abandoned_import_sends_nothing_after_its_cut` (IJ18; `import.rs`): una importación cuya tarea se aborta no manda más llamadas a la conexión.
- [x] La señal (`stop`, `raised`, `cause`), `or_stop`, `Cut`, el `cut` de `Written`, `Imported::Stopped` y `Stopped`, `on_step` y el control en `plan`, `prepare_parsed`, `prepare_export` y `write`.
- Commit: `feat(import): an import stops at the safe point of its step`

## 4. Las comprobaciones que comparten las dos rutas (IJ2, IJ11)

- [x] Guarda, no un RED: las pruebas de importación de `crates/clawg-cli/tests/api.rs` (`an_import_stores_rows_with_their_state_and_a_dry_run_stores_none`, `import_answers_400_for_an_unknown_version_before_the_embedder`, `an_import_is_undone_by_its_number` y las demás) y las de `import.rs` pasan sin cambiar una aserción antes y después.
- [x] Test primero de lo puro que se extrae: `too_large_names_the_size_and_the_limit` y `parse_input_reads_both_sources` (unidades en `crates/clawg-cli/src/daemon/api.rs`, `mod tests`).
- [x] `ImportFields`, `too_large`, `checked_fields`, `parse_input`, `tokenizer_for` y `embedder_for`; `POST /v1/import` sobre `import_parsed` con un control que nadie levanta. `import` e `import_foreign` pasan a `#[cfg(test)]` y salen del `pub use` de `portability/mod.rs`; `file_of` y `notes_of` pasan el control.
- Commit: `refactor(daemon): both import routes share their checks`

## 5. Una importación como trabajo (IJ1 a IJ5, IJ7, IJ9 a IJ11, IJ13, IJ14)

- [x] Tests primero en `crates/clawg-cli/tests/api.rs` (hoy `/v1/imports` es un `404`):
  - IJ1: `an_import_job_is_accepted_without_waiting_for_the_embedder`, `a_job_import_refuses_a_path_that_is_not_an_absolute_readable_regular_file_within_the_limit`, `a_job_import_refuses_an_unknown_field` (fija el `400`, no el `422`).
  - IJ2: `a_job_import_refuses_what_the_synchronous_import_refuses`, una tabla que recorre las dos rutas con el mismo cuerpo salvo `text`/`path`.
  - IJ3: `a_second_import_job_while_one_runs_is_refused_with_its_id`, `a_job_import_checks_the_body_before_the_running_job_and_the_job_before_the_file`, `accepting_a_real_job_wakes_the_embedder_and_a_dry_run_does_not`.
  - IJ4: `a_job_reports_its_phase_and_rows_embedded_advance_inside_embedding`, `only_the_last_finished_job_is_remembered`, `an_unknown_job_is_a_404`, `a_dry_run_job_ends_done_after_deciding`, `a_failing_reload_fails_the_job_without_writing`.
  - IJ5: `a_finished_jobs_counters_match_its_report`.
  - IJ9: `accepting_a_job_reloads_a_released_model_before_embedding`.
  - IJ10: `the_job_route_has_no_body_limit_sized_for_the_file`.
  - IJ11: `the_report_of_a_job_equals_the_report_of_the_synchronous_import`.
  - IJ13: las filas `("POST", "/imports", Some("body.project_map[].project"))` y `("GET", "/imports/{id}", None)` de `PROJECT_INPUTS`, y el brazo de `POST /imports` en `every_route_that_takes_a_project_follows_a_merge` y en `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule`.
  - IJ14: `the_job_routes_require_the_guard_and_the_token` (`POST` y `GET`; el commit 7 suma `DELETE`).
- [x] Tests primero en `mod tests` de `crates/clawg-cli/src/daemon/api.rs`, con `accept_job` y un `on_step`: `a_job_stopped_in_writing_counts_exactly_the_committed_batches` (IJ5), `a_job_stopped_in_writing_leaves_whole_notes_and_frees_its_number`, `importing_again_finishes_a_stopped_job` y `undoing_a_stopped_job_works` (IJ7), `a_job_stopped_before_its_first_write_has_no_report` (IJ4).
- [x] Tests primero en `crates/clawg-cli/src/daemon/imports.rs`, sobre `Jobs` sin runtime (IJ3, IJ4): `a_reservation_is_atomic_and_released_when_dropped`, `a_reserved_id_that_is_never_accepted_is_a_404`, `only_the_last_finished_job_is_kept`, `phase_of_covers_every_step`.
- [x] Test primero por el binario, `a_job_import_ends_done_with_the_report_of_the_synchronous_import` (IJ4; `crates/clawg-cli/tests/binary.rs`, cerca de `:792` y `:830`, con `CLAWG_TEST_EMBEDDER=fake`): el `clawg` real sirve las dos rutas por HTTP (el CLI todavía no usa la nueva) y el `report` del trabajo es igual, salvo snapshot e `import_id`, al de `POST /v1/import`.
- [x] `daemon/imports.rs` (registro `ImportJobs`), `imports: Default::default()` en los doce sitios que arman un `AppCtx` (`serve.rs`, `stop.rs`, `socket/mod.rs`, `client.rs` dos veces, `mcp/mod.rs`, `mcp/server.rs` dos veces, `api.rs`, `tests/api.rs`, `clawg-eval` `revision-eval/run.rs` y `rag-eval/agent_view/runner.rs`); `POST /v1/imports` y `GET /v1/imports/{id}` con `accept_job`, `run_job`, `JobView` y `job_id` en `ErrorBody`. `.claude/CLAUDE.md`: las rutas `POST` y `GET /v1/imports` en la API del daemon.
- Commit: `feat(daemon): run an import as a job`

## 6. El apagado con un trabajo activo (IJ18, IJ3, IJ19)

- [x] Tests primero en `mod tests` de `crates/clawg-cli/src/daemon/api.rs` (IJ18): `shutdown_stops_a_job_in_embedding_within_the_grace`, `shutdown_stops_a_job_in_writing_between_whole_notes` (tiempo real) y `an_abandoned_job_sends_nothing_to_the_connection` (`start_paused = true`).
- [x] Test primero (IJ3): `a_job_import_during_shutdown_is_refused_with_a_retryable_503` (`api.rs`, `mod tests`).
- [x] Tests primero sobre el registro (IJ18, IJ3; `daemon/imports.rs`): `a_closed_registry_refuses_to_reserve`, `an_abandoned_job_is_interrupted_and_named` (`abandoned: true` y el paso), `a_finish_after_abandon_changes_nothing`.
- [x] Guardas, no un RED, que deben seguir verdes: `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` (`crates/clawg-cli/src/stop.rs`, sin cambios) y `published_migration_checksums_are_pinned` (`crates/clawg-core/src/memory/migrations.rs`, IJ19: no hay migración).
- [x] `close`, `settled`, `abandon`, `503 shutting_down`; en `serve.rs`, `close` al observar la señal, la espera como tercer futuro del `try_join!` y `abandon` cuando `within_grace` vence, con la advertencia nueva. `.claude/CLAUDE.md`: el párrafo del apagado de `clawg serve`.
- Commit: `feat(daemon): a shutdown stops the import job within the grace`

## 7. Cancelar un trabajo y nombrarlo en `/health` (IJ6, IJ8, IJ12, IJ14)

- [x] Tests primero en `crates/clawg-cli/tests/api.rs`:
  - IJ6: `deleting_a_job_in_embedding_leaves_no_row_no_journal_and_no_snapshot`, `a_delete_on_a_finished_job_changes_nothing` (el `200` del terminado, el `202` del activo con `stopping`, el `404`).
  - IJ8: `a_job_outlives_the_client_that_accepted_it`.
  - IJ12: `health_carries_the_active_jobs_import_block_without_the_connection`, `health_has_no_import_block_without_an_active_job`.
  - IJ14: `the_job_routes_require_the_guard_and_the_token` suma `DELETE`.
  - IJ13: la fila `("DELETE", "/imports/{id}", None)` de `PROJECT_INPUTS`.
- [x] Tests primero en `mod tests` de `crates/clawg-cli/src/daemon/api.rs` (IJ6, con `on_step`): `a_delete_during_links_ends_done`, `a_delete_during_snapshot_ends_cancelled_with_the_snapshot_and_no_rows`.
- [x] Guarda, no un RED: `a_client_reads_a_health_with_an_import_block` (`crates/clawg-cli/src/client.rs`), porque `Health` ya ignora lo que no conoce.
- [x] `DELETE /v1/imports/{id}` (`cancel`), el bloque `import` de `/health` desde `ImportJobs::active()`. `.claude/CLAUDE.md`: `DELETE /v1/imports/{id}` y el bloque `import` de `/health`.
- Commit: `feat(daemon): cancel an import job, and name it in health`

## 8. `clawg import` sigue su trabajo (IJ15, IJ16, IJ17, IJ8, IJ1)

- [x] Tests primero, unidades (`crates/clawg-cli/src/import.rs`): `the_job_messages_speak_both_languages` (IJ17), `phases_go_to_stderr_only_on_a_terminal_or_with_follow`, `any_final_state_but_done_exits_1`, `a_second_ctrl_c_leaves_without_waiting`, `a_vanished_daemon_names_the_last_import_id`, `a_404_on_its_own_job_while_following_is_a_vanished_daemon` y `a_stop_without_a_report_says_nothing_was_written` (IJ15), y la lógica de Ctrl-C con la señal inyectada (Windows).
- [x] Tests primero, unidades (`crates/clawg-cli/src/client.rs`): `the_client_sends_a_path_and_not_the_file` (IJ1), `route_missing_tells_a_bodyless_404_from_the_daemons_own` (IJ16) y `the_job_poll_lives_outside_the_client_and_sets_no_timeout` (guarda de IJ15: `only_health_sets_a_timeout` sigue verde).
- [x] Test primero (`crates/clawg-cli/src/cli.rs`): `follow_conflicts_with_undo`.
- [x] Tests primero por el binario (`crates/clawg-cli/tests/binary.rs`): `follow_prints_the_phases_in_order_on_stderr_and_the_same_report_on_stdout`, `killing_clawg_import_does_not_stop_the_job` (IJ8), `import_against_a_daemon_without_the_job_route_exits_1_without_sending_the_file` (IJ16), y `ctrl_c_sends_the_delete_and_prints_what_was_written` (Unix, `SIGINT`). Las importaciones por el binario de `binary.rs:792` y `:830` siguen verdes.
- [x] `start_import`, `import_job`, `cancel_import`, `ImportJob`, `route_missing` y `job_id` en `ErrorDetails`; `Follow`, `progress_line`, `gone`, `shows_progress`; `--follow` en `cli.rs` y el brazo `Command::Import` de `main.rs`. Se van `DaemonClient::import`, `ImportError::Read` y `TooLarge`. `.claude/CLAUDE.md`: el comando `clawg import`.
- Commit: `feat(cli): clawg import follows its job and Ctrl-C cancels it`

## 9. La medición de memoria (IJ10)

- [x] Sin RED: es una medición, no una conducta. `crates/clawg-cli/tests/import_memory.rs` con `by_body` y `by_path`, ambas `#[ignore]`, que no afirman nada; imprimen una línea JSON por formato y brazo (D10). `tests/conventions.rs` y la suite siguen verdes.
- Commit: `test(import): measure an import's memory by body and by path`

## 10. Docs

- [x] `.claude/CLAUDE.md`: revisar que `clawg import`, las rutas de la API del daemon (`POST/GET/DELETE /v1/imports`, `job_id` del error, `503 shutting_down`), `GET /health` y el párrafo del apagado de `clawg serve` dicen lo que hace el código final; los cambios por comportamiento ya entraron en los commits 5 a 8, esta tarea sólo cierra lo que falte. Un commit sólo si quedó algo: `docs: describe the import job`.
- [x] La fila 87 de `openspec/README.md`.

## 10b. Lo que dejó la verificación

- [x] Las dos pruebas que la verificación encontró flojas: `accepting_a_job_reloads_a_released_model_before_embedding` mira la carga antes de `embedding` y muere sin `wake()` (IJ9), y la tabla de IJ2 suma `409 project_ambiguous` y `503 embedder_failed`. Commits: `test(daemon): an accepted job loads the model before it embeds` y `test(daemon): both import routes refuse an ambiguous project and a failed model alike`.
- [x] Un error pasajero del sondeo o del `DELETE` ya no se lee como un daemon que se fue (IJ15, escenario «un error pasajero»): `ClientError::means_daemon_gone`, `Event::Unanswered`, `MAX_UNANSWERED`. Tests primero, rojos por no compilar. Commit: `fix(cli): clawg import asks again before it says the daemon went away`; `.claude/CLAUDE.md` en el mismo.
- [x] `README.md`: el progreso, Ctrl-C y que cerrar o matar el comando deja la importación corriendo. Commit: `docs(readme): say what clawg import shows and how to stop it`.

## 11. Verificación

- [x] `cargo clean --workspace --target-dir target-test` antes de probar esta versión (`.claude/CLAUDE.md`, «Build directories»); durante el ciclo de un commit el directorio se reusa.
- [x] `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings` en verde.
- [x] `cargo test --workspace --target-dir target-test` en verde (nunca se anota cuántas pruebas).
- [x] La medición de memoria de D10, una sola vez y nunca en paralelo con otra corrida de modelo real: `by_body` y `by_path`, una por proceso, con
  `cargo test -p clawg-cli --test import_memory --target-dir target-test -- --ignored --exact by_body --nocapture` y lo mismo con `by_path`; los números van a mano a `verify.md`.
- [ ] El CI en verde en Linux y Windows.
- [x] Ningún directorio de build fuera de `target` y `target-test` al terminar.

## 12. Archive

- [ ] Los deltas a `specs/portability` (IJ1 a IJ11 y IJ15 a IJ17, y R3 y HI16 modificados), `specs/daemon-access` (IJ12 a IJ14) y `specs/storage` (IJ18, «Cada fase del apagado tiene tope»); `git mv` a `changes/archive/<fecha>-import-job`; `state.yaml` archivado.
- [ ] Subir el minor de `[workspace.package] version` del `Cargo.toml` raíz, `cargo update --workspace`, y commit de los dos archivos: `chore: bump version to X.Y.Z`.

## Cobertura IJ → tarea

| IJ | Tarea |
|---|---|
| IJ1 | 2 (un solo parseo), 5 (ruta, 202, 400), 8 (el cliente manda la ruta) |
| IJ2 | 4 (guarda y piezas), 5 (tabla de las dos rutas) |
| IJ3 | 5 (409, orden, `wake`), 6 (`503 shutting_down`) |
| IJ4 | 5 |
| IJ5 | 2 (contadores), 5 (al terminar y al parar en `writing`) |
| IJ6 | 3 (tabla de pasos), 7 (`DELETE`, `links`, `snapshot`) |
| IJ7 | 3 (el `Ledger`), 5 (notas enteras, número, `--undo`, reimportación) |
| IJ8 | 1 (guarda del síncrono), 7 (sobrevive al cliente), 8 (matar `clawg import`) |
| IJ9 | 5 |
| IJ10 | 2, 5 (sin tope de cuerpo), 9 (medición), 11 (corrida) |
| IJ11 | 1, 4 (guardas), 5 (mismo informe) |
| IJ12 | 7 |
| IJ13 | 5 (`POST`, `GET` y los dos brazos), 7 (`DELETE`) |
| IJ14 | 5, 7 |
| IJ15 | 8 |
| IJ16 | 8 |
| IJ17 | 8 |
| IJ18 | 3 (`an_abandoned_import_sends_nothing_after_its_cut`), 6 |
| IJ19 | 6 (guardas), 8 (CLI nuevo frente a daemon anterior) |

Todo IJ tiene al menos una tarea; ninguno queda sin cubrir.
