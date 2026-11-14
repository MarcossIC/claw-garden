# agent-eval-sessions — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional atómico, comprobado con las pruebas de los archivos que cambia (`cargo test -p clawg-eval --bin agent-eval <nombre>`, y `--test agent_eval` para los de integración; en Windows con `--target-dir target-test`). La suite completa y los dos clippy corren una vez, sobre el último commit. Requisitos en la [spec](spec.md) (AS1 a AS12) y decisiones en el [diseño](design.md) (D1 a D13). Los archivos están en `crates/clawg-eval/src/bin/agent-eval/` salvo que se diga otra cosa.

Antes de empezar: `cargo clean --workspace --target-dir target-test`, porque es otra versión de las fuentes.

## 1. El orden del `MEMORY.md` (AS1)

- [x] RED: `the_native_memory_goes_from_the_oldest_memory_to_the_newest` (`seed.rs`); en `every_arm_with_memory_seeds_the_noise_with_the_task` (`arm.rs`), la aserción de `a4` pasa a pedir la memoria más vieja primero.
- [x] GREEN: `seed::native_memory` escribe en el orden inverso al de `seed::seeded`; `seeded` no cambia.
- Commit: `fix(agent-eval): write the native memory from the oldest note to the newest`

## 2. Una tarea de varias sesiones (AS2)

- [x] RED: `a_task_has_one_prompt_or_a_list_of_sessions`, `a_tracer_of_a_session_is_born_in_its_prompt_and_told_nowhere_else`, `a_check_of_a_session_reads_only_the_stream`, `a_task_written_before_sessions_still_reads`, `the_noise_holds_no_tracer_born_in_a_session` (`task.rs`); `a_working_directory_that_names_a_tracer_of_a_session_is_named` (`run.rs`).
- [x] GREEN: `SessionSpec`, `Task::sessions` y `Task::prompts`; las reglas nuevas de `problems`, `load_noise` y `leaked_tracers`; `leaks` con los trazadores de sesión; `arm::plan` y `list_tasks` leen el prompt por `prompts()` y siguen planeando una sesión.
- Commit: `feat(agent-eval): read a task of several sessions and the facts born in them`

## 3. Los fallos de una herramienta y el shell de una tarea (AS3)

- [x] RED: `the_failed_calls_of_a_tool_are_those_marked_or_those_that_hold_a_text` (`score.rs`); `a_task_names_the_shell_commands_it_allows_and_those_that_must_be_missing`, `e_is_a_category` (`task.rs`); `a_task_with_a_shell_allows_its_commands_and_no_other` (`arm.rs`); `a_paid_run_refuses_a_command_that_should_be_missing` (`run.rs`).
- [x] GREEN: `Check::ToolFailedCount`; `Task::shell` y `Task::missing_commands` con su validación; `E` en `CATEGORIES`; los argumentos de permiso en `arm::plan`; `Leak::Present` y la búsqueda en el `PATH` en `check_paid_run`, con la decisión en `leaks`.
- Commit: `feat(agent-eval): count the failed calls of a tool and let a task allow named shell commands`

## 4. El plan de varias sesiones y su forma (AS4)

- [x] RED: `a_task_of_one_session_plans_what_it_planned_before`, `each_session_has_its_prompt_and_a_joined_one_continues_the_conversation`, `a_plan_of_several_sessions_reads_back_and_an_older_plan_reads_as_one` (`arm.rs`); `the_shape_of_a_run_parses_and_is_refused_where_the_task_cannot_take_it` (`main.rs`); `the_files_of_a_session_carry_its_number_only_when_there_are_several` (`run.rs`).
- [x] GREEN: `Shape`, `Options::shape`, `RunPlan::later` y `RunPlan::sessions()`; `--sessions` en `RUN_FLAGS` y en la ayuda; la forma en `run.json`; `run::session_files`.
- Commit: `feat(agent-eval): plan every session of a run, split or joined`

## 5. El corredor de varias sesiones (AS5, y de AS7 el directorio de streams)

- [x] RED: `a_session_that_ends_in_an_error_starts_no_other` (`run.rs`); `a_replay_is_a_stream_or_a_directory_with_one_per_session` (`main.rs`); `a_replayed_run_of_two_sessions_shares_its_daemon_and_its_working_directory`, `a_session_that_fails_stops_the_run_and_still_stops_the_daemon` (`tests/agent_eval.rs`), sobre una tarea de ejemplo de dos sesiones y sus streams, que se agregan en este commit a `data/agent-tasks/`.
- [x] GREEN: el bucle de sesiones dentro de `execute`, con sus archivos por sesión, la copia de `MEMORY.md` y el movimiento del registro de instrucciones; `Client::Replay` con un directorio; `score_dir` puntúa por ahora la última sesión y no falla con varias.
- Commit: `feat(agent-eval): run the sessions of a task on one daemon and working directory`

## 6. El cliente reproducido llama al servidor (AS7)

- [x] RED: `a_replayed_call_to_the_server_of_the_run_carries_what_the_server_answered`, `a_call_to_any_other_tool_stays_as_recorded` (`replay.rs`); `the_second_replayed_session_finds_what_the_first_one_saved` (`tests/agent_eval.rs`).
- [x] GREEN: `replay::server_calls` y `replay::with_results`, puros; el proceso del servidor por stdio en `replay::run`; `replay-client STREAM [--mcp-config FILE]` y `Client::step` que lo pasa en los brazos con clawg.
- Commit: `feat(agent-eval): replay the calls to the run's server against the server`

## 7. Los tokens de cada pedido y de cada llamada (AS8)

- [x] RED: `each_request_of_the_main_loop_is_read_once_with_its_context`, `a_call_alone_in_its_message_adds_what_the_next_request_grew`, `calls_that_share_a_message_have_no_tokens_of_their_own` (`stream.rs`).
- [x] GREEN: `Transcript::requests`, `ToolCall::added_tokens`; `first_request_tokens` sale del primer pedido.
- Commit: `feat(agent-eval): read the tokens of every request and what each call adds`

## 8. El puntaje de cada sesión y el camino de un hecho (AS9)

- [x] RED: `a_check_of_a_session_reads_that_session_and_one_of_the_task_the_last`, `a_tracer_born_in_a_session_is_followed_in_the_later_ones`, `a_tracer_names_what_brought_it` (`score.rs`); `the_client_loads_the_first_200_lines_or_25_kb_of_the_native_memory` (`seed.rs`); `a_joined_session_has_the_earlier_prompts_in_its_context`, `the_native_memory_delivered_is_what_the_file_held_before_the_session`, `a_fact_is_told_as_not_saved_not_reached_not_used_or_used`, `the_relay_overhead_is_what_the_new_session_starts_with_and_reads_again`, `a_result_written_before_sessions_still_reads`, `score_dir_scores_every_session_of_the_run` (`run.rs`).
- [x] GREEN: `score::score_session` y el puntaje de la corrida; `seed::native_loaded`; `delivered` por sesión; `run::facts` y `run::overhead`; `RunResult` con `sessions`, `shape`, `noise_rules`, `facts`, `overhead` y `paired`; `score_dir` sobre todas las sesiones; `summary` con las sesiones.
- Commit: `feat(agent-eval): score each session and follow a fact from one to the next`

## 9. La bifurcación (AS6)

- [x] RED: `a_fork_plays_the_second_session_continued_and_then_new` (`run.rs`); `a_forked_run_plays_the_second_session_both_ways_from_one_first_session`, `a_branch_of_a_fork_does_not_see_what_the_other_did` (`tests/agent_eval.rs`), con los streams `joined-2.jsonl` y `split-2.jsonl` del ejemplo.
- [x] GREEN: `run::fork_steps`, puro; la copia y la restauración en `execute`; `score_dir` puntúa cada rama en su subdirectorio y `score --run` las dos.
- Commit: `feat(agent-eval): fork a run after its first session and play the second both ways`

## 10. El reporte, los costos y las llamadas (AS10)

- [x] RED: `rows_are_split_by_the_shape_of_the_run_and_add_up_its_sessions`, `a_forked_run_reports_its_pairs_and_what_the_new_session_saved`, `a_pair_that_continued_cold_does_not_count_for_the_cost`, `the_costs_have_a_line_per_session_with_its_number_and_shape`, `the_calls_table_tells_what_each_tool_adds` (`report.rs`); `the_calls_command_parses` (`main.rs`).
- [x] GREEN: `Row` con la forma y las columnas nuevas; `report::pairs`; las dos columnas de `costs`; `report::calls` y `agent-eval calls --runs DIR`; la ayuda.
- Commit: `feat(agent-eval): report runs by shape, the pairs of a fork and what each tool adds`

## 11. El ruido de una corrida y las 300 reglas (AS11)

- [x] RED: `a_run_is_seeded_with_the_noise_it_names` (`task.rs`); `the_startup_rules_must_fit_what_the_client_injects` (`arm.rs`); `the_noise_of_a_run_parses` (`main.rs`); `the_large_noise_is_300_distinct_rules_older_than_a_week` y la línea de cada ruido en `the_example_tasks_are_valid_and_leak_no_tracer` (`tests/agent_eval.rs`).
- [x] GREEN: `load_noise` con nombre; `--noise`; `ArmError::StartupTooLong`; `list_tasks` recorre cada `noise*.json`; `data/agent-tasks/noise-300.json`; lo escrito acá en `data/agent-tasks/SOURCES.md`.
- Commit: `feat(agent-eval): seed a run with the noise it names and add one of 300 rules`

## 12. Los fixtures y las tareas nuevas (AS12)

- [x] Copiar `psf-requests` de su fuente en el commit de la [propuesta](proposal.md#las-tareas-nuevas), sin cambios; comprobar con `rg` que ninguna cadena de los checks ni de los trazadores está en el fixture. Escribir `shelf-notes`, con su `CLAUDE.md`, y `fixtures/.ignore` (D13).
- [x] RED: `the_tasks_of_several_sessions_are_listed_with_their_sessions`, `the_replayed_relay_example_follows_its_fact_to_the_second_session`, `the_replayed_missing_command_example_counts_its_failures` (`tests/agent_eval.rs`); `the_example_tasks_are_valid_and_leak_no_tracer` falla hasta que las cuatro tareas son válidas.
- [x] GREEN: las cuatro tareas, la segunda tarea de ejemplo con sus streams, `list_tasks` con las sesiones de cada tarea, y `SOURCES.md` con la fuente, el commit, la licencia, las rutas tomadas y las que no, y qué escribió cada tarea.
- Commit: `feat(agent-eval): add the relay, revised decision, scale and repeated error tasks`

## 13. La referencia

- [x] `docs/reference/testing.md`: las sesiones de una tarea, las tres formas, la bifurcación, los archivos por sesión, `--noise`, `--sessions`, `agent-eval calls`, el shell de una tarea, lo que el cliente reproducido manda al servidor, y el orden del `MEMORY.md`, que hoy dice el contrario (`docs/reference/testing.md:65`).
- [x] `openspec/README.md`: el estado de la fila 129.
- Commit: `docs: describe the sessions, the fork and the new tasks of agent-eval`

## 14. Verificación

- [x] `cargo test --workspace` y los dos `cargo clippy` de `docs/reference/testing.md`, sobre el último commit, en `target-test` en Windows.
- [x] `agent-eval tasks` da válidas las tareas y lista los dos ruidos.
- [x] `verify.md` con lo anterior. El CI de Linux y Windows queda pendiente, y está en el verify.

## Después de este cambio

Local, con una clave de API y autorización de gasto; nada de esto es de la suite.

- [ ] La guía de la campaña, fuera de git, con las fases, los topes y la regla de decisión de la [propuesta](proposal.md#la-regla-de-decisión) copiada sin cambios.
- [ ] Fase 0: todas las tareas nuevas en seco con el modelo real.
- [ ] Fase 1, humo y canarios. Es una puerta: la campaña sigue sólo si pasa.
- [ ] Fases 2 a 4, y el resultado contra la regla, en `verify.md`.
- [ ] Al archivar: AS1 reemplaza el orden del `MEMORY.md` en MD3 de [`specs/eval`](../../specs/eval/spec.md) (`openspec/specs/eval/spec.md:1491`), y AS2 a AS12 pasan a esa sección.
