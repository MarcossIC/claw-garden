# agent-eval — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla, por la aserción o porque no compila. Cada grupo es un commit convencional atómico, comprobado con las pruebas de los archivos que cambia; la suite completa y los dos clippy corren una vez, sobre el último commit. Requisitos en la [spec](spec.md) (AE1 a AE6) y decisiones en la [propuesta](proposal.md).

## 1. El tier de una fila sembrada (AE3)

- [x] RED: `a_seeded_row_earns_the_tier_the_store_promotes_it_to` en `eval_api.rs`.
- [x] GREEN: `eval_api::earned_tier`.
- Commit: `feat(eval): expose the tier a seeded row earns`

## 2. El binario (AE1 a AE6)

Un binario es un crate: una función que sólo usa otro módulo todavía ausente es código muerto, así que los módulos entran juntos.

- [x] RED: los tests de `task.rs`, `arm.rs`, `seed.rs`, `stream.rs`, `score.rs`, `rubric.rs`, `replay.rs`, `run.rs` y `main.rs`.
- [x] GREEN: los módulos y el `[[bin]]` en `crates/clawg-eval/Cargo.toml`.
- Commit: `feat(eval): lay out agent sessions by arm and score their stream`

## 3. Las tareas de ejemplo y la corrida en seco (AE1, AE6)

- [x] `data/agent-tasks/`: `example-staging-port` (K1) y `example-count-lines` (N) sobre el fixture `example-tally`, y los streams `example-staging-port.a3`, `example-staging-port.a0` y `example-count-lines.a3`.
- [x] `tests/agent_eval.rs`.
- Commit: `test(eval): replay hand-written sessions of example agent tasks`

## 4. La referencia

- [x] `docs/reference/testing.md`; los punteros de `.claude/CLAUDE.md`; esta propuesta, la spec, las tareas y el mapa de `openspec/README.md`.
- Commit: `docs: describe agent-eval and its dry run`

## 5. Verificación

- [x] Suite completa y clippy sobre el último commit: pasan.

## 6. El daemon de la corrida (AE7)

- [x] RED: `the_arms_with_clawg_start_a_daemon_of_the_run_on_an_ephemeral_port`, `every_process_of_a_run_finds_its_home_also_in_a_testing_build`, `the_plan_reads_back_from_its_json` (`arm.rs`); `on_windows_a_run_with_clawg_needs_a_testing_build`, `the_mark_is_found_wherever_a_read_cuts_it`, `a_daemon_listens_once_its_state_file_names_it` (`run.rs`); en `tests/agent_eval.rs` la siembra y el cierre de las corridas de `a3`, ningún clawg en `a0`, `a_client_that_fails_still_stops_the_daemon_of_its_run` y `a_daemon_that_never_listens_starts_no_client`.
- [x] GREEN: el daemon y `CLAWG_TEST_HOME` en el plan, `run::execute` y `check_isolation`.
- Commit: `feat(eval): run a session against a daemon of its run's home`

## 7. El cliente real (AE7)

- [x] RED: `the_client_is_claude_as_planned_or_a_replay_of_a_stream`, `the_version_claude_reports_is_its_first_word` (`run.rs`); `a_run_without_a_replay_starts_claude_only_with_a_budget_and_a_pinned_version` (`main.rs`); `a_run_with_claude_refuses_another_version_before_laying_out_the_run` (`tests/agent_eval.rs`).
- [x] GREEN: `Client` en `run.json`, `--claude-version`, `check_client_version` y `run` sin `--replay`.
- Commit: `feat(eval): run claude pinned to a Claude Code version and a spending cap`

## 8. La referencia del corredor

- [x] `docs/reference/testing.md`; esta propuesta, la spec, las tareas y el mapa de `openspec/README.md`.
- Commit: `docs: describe the agent-eval runner`

## 9. Verificación del corredor

- [x] Suite completa y clippy sobre el último commit: pasan.

## 10. El modelo listo antes de la siembra (AE7)

En Windows `clawg import` salía con 1: un import real pide el embedder y el daemon responde `503 embedder_not_ready` mientras el modelo carga, y la siembra se mandaba apenas aparecía `daemon.json`.

- [x] RED: `the_model_is_ready_once_the_status_of_this_daemon_says_so`, `a_step_that_fails_is_told_by_its_last_line` (`run.rs`), el paso `status` en `the_arms_with_clawg_start_a_daemon_of_the_run_on_an_ephemeral_port` (`arm.rs`), y en `tests/agent_eval.rs` el `clawg status` con el modelo listo antes del import y de la sesión.
- [x] GREEN: `Daemon::status` en el plan, la espera de `Served::ready` y la última línea de un paso que falla en su error.
- Commit: `fix(eval): wait for the run's daemon to have its model ready before the seed`
- Commit: `docs: say that the agent-eval runner waits for the model`
- [x] Suite completa y clippy sobre el último commit: pasan.

## Después de este cambio

- [ ] Las seis tareas del piloto y su fixture, revisadas antes de correr.
- [ ] El piloto, con autorización y tope de gasto: la primera sesión real de `claude` confirma los nombres del cliente (decisión 8).
