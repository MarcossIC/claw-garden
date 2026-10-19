# shutdown-budget — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. Lo que sigue al drenaje tiene tope (R1, D3)

- [ ] Test primero: `shutdown_gives_up_on_a_worker_that_never_ends` (`crates/clawg-core/tests/shutdown.rs`).
- [ ] `AFTER_DRAIN` en `runtime.rs`; `shutdown_by` bajo `timeout(drain_deadline + AFTER_DRAIN)`.
- Commit: `fix(core): bound what a shutdown does after the drain deadline`

## 2. La fase HTTP tiene tope y SIGTERM apaga en orden (R1, R3, D2, D5)

- [ ] Tests primero: `a_server_that_does_not_finish_is_left_after_the_grace`, `a_server_that_finishes_in_time_gives_its_result` (`serve.rs`); `sigterm_stops_the_daemon_in_order` (`binary.rs`, Unix).
- [ ] `HTTP_GRACE`, `within_grace`, el aviso por `oneshot`; el brazo de SIGTERM.
- Commit: `fix(daemon): bound the HTTP phase of a shutdown and stop in order on SIGTERM`

## 3. El desmontaje del runtime tiene tope (R1, D4)

- [ ] Test primero: `stop_ends_a_daemon_whose_model_load_never_returns` (`binary.rs`); `TestEmbedder::Blocked` en `parse` (`bootstrap.rs`).
- [ ] `RUNTIME_TEARDOWN`; `shutdown_timeout` para `serve` en `main.rs`.
- Commit: `fix(cli): let clawg serve exit without waiting for a model load`

## 4. El presupuesto (R2, D1)

- [ ] Test primero: `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` (`stop.rs`), en lugar de `the_drain_deadline_leaves_room_inside_the_stop_timeout`.
- [ ] `STOP_TIMEOUT` = 30 s y su doc.
- Commit: `fix(cli): give clawg stop a timeout that covers every phase of a shutdown`

## 5. `update` ante un `stop` que vence (R4, D6)

- [ ] Tests primero: `a_daemon_that_does_not_stop_in_time_is_started_again_and_nothing_is_renamed`, `a_stop_that_never_reached_the_daemon_starts_nothing` (`plan.rs`); `a_stop_that_ran_out_of_time_does_not_say_the_daemon_was_not_touched`, `a_daemon_still_shutting_down_says_how_to_start_it_once_it_ends` (`report.rs`).
- [ ] `StepError`; `stopping` y la espera en `undo StartOldDaemon`; `daemon_state`.
- Commit: `fix(update): start the old daemon again after a stop that ran out of time`

## 6. Docs y archivo

- [ ] `.claude/CLAUDE.md`: `clawg stop`, `CLAWG_TEST_EMBEDDER=fake|stalled|blocked`.
- [ ] `openspec/specs/storage` (definición de apagado ordenado, el presupuesto, el límite conocido), `openspec/specs/self-update` (S10 y S11), backlog y fila 55 de `openspec/README.md`.
- [ ] `verify.md`, archivo y versión 0.35.0.
