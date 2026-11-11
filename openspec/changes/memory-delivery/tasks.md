# memory-delivery — Tareas (paso 1)

Strict TDD: en cada tarea con conducta nueva el test va primero y falla, por la aserción o porque no compila. Cada grupo es un commit convencional atómico, comprobado con las pruebas de los archivos que cambia; la suite completa y los dos clippy corren una vez, sobre el último commit. Requisitos en la [spec](spec.md) (MD1 a MD7) y decisiones en la [propuesta](proposal.md).

## 1. Las reglas, lo que dijo la persona y el ruido (MD1, MD2, MD3)

- [x] RED: `a_task_reads_with_its_defaults`, `a_rule_reads_with_or_without_paths`, `a_rule_path_must_stay_inside`, `the_noise_is_rules_that_hold_no_tracer`, `a_data_directory_without_noise_has_none` (`task.rs`); `a_seeded_memory_the_person_stated_is_the_persons`, `the_seed_goes_from_the_newest_memory_to_the_oldest` (`seed.rs`); `every_arm_with_memory_seeds_the_noise_with_the_task` (`arm.rs`); el conteo del import y la línea del ruido en `tests/agent_eval.rs`.
- [x] GREEN: `SeedMemory::user_stated` y `SeedMemory::rule`, `task::load_noise`, `seed::seeded`, el ruido en `Options`, `data/agent-tasks/noise.json`, las memorias de las seis tareas del piloto marcadas como reglas y lo escrito aquí en `data/agent-tasks/SOURCES.md`.
- Commit: `feat(agent-eval): seed rules, memories the person stated and the noise of other topics`

## 2. Los brazos de reglas y los hooks del arnés (MD4, MD5)

- [x] RED: los tests de `rules.rs` y de `hooks.rs`; `l0_is_a3_without_the_protocol`, `l1_adds_the_rules_at_startup_through_a_hook_of_the_harness`, `l2_moves_the_rules_with_globs_to_rules_files`, `l1_without_rules_adds_no_hook`, `every_arm_logs_the_instruction_files_the_client_loads` (`arm.rs`); `the_hook_commands_parse` (`main.rs`); `the_harness_hooks_print_the_context_and_log_their_input`, `a_prepared_l2_run_writes_the_rules_of_its_task_by_file` (`tests/agent_eval.rs`).
- [x] GREEN: `rules.rs`, `hooks.rs`, `Arm::L0`, `Arm::L1` y `Arm::L2`, los hooks en el `settings.json` de cada brazo y `agent-eval hook-context` y `hook-log`.
- Commit: `feat(agent-eval): deliver the seeded rules at startup or by file in arms l0, l1 and l2`

## 3. Lo que llegó al contexto y el primer pedido (MD5, MD6, MD7)

- [x] RED: `the_first_request_is_the_first_message_of_the_main_loop` (`stream.rs`); `a_tracer_in_what_reached_the_context_counts_as_retrieved` (`score.rs`); `the_log_reads_each_loaded_file_and_skips_what_is_not_an_entry` (`hooks.rs`); `the_native_memory_and_the_loaded_rules_files_are_delivered`, `score_dir_counts_what_reached_the_context_outside_the_stream` (`run.rs`); `rows_carry_the_median_first_request_and_the_rules_loaded_by_path` (`report.rs`).
- [x] GREEN: `Transcript::first_request_tokens`, `score::add_delivered`, `run::delivered`, `hooks::read_log`, el registro de instrucciones en `score_dir` y `Session`, y las dos columnas del reporte.
- Commit: `feat(agent-eval): count what reached the context outside the stream and the first request`

## 4. La referencia

- [x] `docs/reference/testing.md`; esta propuesta, la spec, las tareas y el mapa de `openspec/README.md`.
- Commit: `docs: describe the rule arms, the noise and the instruction log of agent-eval`

## 5. Verificación

- [ ] Suite completa y clippy sobre el último commit: pasan.

## Después de este paso

- [ ] La guía de la campaña, fuera de git como la del piloto: `a4`, `a3`, `l0`, `l1` y `l2` sobre las seis tareas, dos corridas, con tope de gasto. La primera sesión de `l2` confirma que el cliente lee `.claude/rules/` en `-p` y avisa `InstructionsLoaded`.
- [ ] Aplicar la regla de decisión de la [exploración](exploration.md#el-experimento) y proponer el paso 2 con el ganador.
