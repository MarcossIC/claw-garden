# memory-delivery — Verificación

## El paso 1, en el arnés (2026-11-11)

En Windows, build debug, en `target-test`, sobre `eb909d62` (el último commit de código es `13261f71`): `cargo test --workspace` pasa entero y los dos `cargo clippy` salen sin ningún warning, como lo anotó `a865cb81` en las [tareas](tasks.md).

Cada requisito tiene sus tests, que existen hoy con esos nombres en `crates/clawg-eval`:

| Requisito | Tests |
|---|---|
| MD1 | `a_seeded_memory_the_person_stated_is_the_persons` (`seed.rs`); `a_task_reads_with_its_defaults` (`task.rs`) |
| MD2 | `a_rule_reads_with_or_without_paths`, `a_rule_path_must_stay_inside`, `the_noise_is_rules_that_hold_no_tracer`, `a_data_directory_without_noise_has_none` (`task.rs`); `the_example_tasks_are_valid_and_leak_no_tracer` (`tests/agent_eval.rs`) |
| MD3 | `the_seed_goes_from_the_newest_memory_to_the_oldest` (`seed.rs`); `every_arm_with_memory_seeds_the_noise_with_the_task` (`arm.rs`); `a_replayed_session_with_clawg_finds_the_fact_and_succeeds` (`tests/agent_eval.rs`) |
| MD4 | `a_rule_is_one_line_with_its_age`, `the_startup_holds_every_rule_or_only_those_without_paths`, `the_rules_by_file_share_a_file_by_their_globs` (`rules.rs`); `the_hook_output_is_the_additional_context_of_session_start` (`hooks.rs`); `l0_is_a3_without_the_protocol`, `l1_adds_the_rules_at_startup_through_a_hook_of_the_harness`, `l2_moves_the_rules_with_globs_to_rules_files`, `l1_without_rules_adds_no_hook` (`arm.rs`); `the_hook_commands_parse` (`main.rs`); `the_harness_hooks_print_the_context_and_log_their_input`, `a_prepared_l2_run_writes_the_rules_of_its_task_by_file` (`tests/agent_eval.rs`) |
| MD5 | `every_arm_logs_the_instruction_files_the_client_loads` (`arm.rs`); `a_log_line_keeps_the_input_on_one_line`, `the_log_reads_each_loaded_file_and_skips_what_is_not_an_entry` (`hooks.rs`); `score_dir_counts_what_reached_the_context_outside_the_stream` (`run.rs`) |
| MD6 | `a_tracer_in_what_reached_the_context_counts_as_retrieved` (`score.rs`); `the_native_memory_and_the_loaded_rules_files_are_delivered` (`run.rs`) |
| MD7 | `the_first_request_is_the_first_message_of_the_main_loop` (`stream.rs`); `rows_carry_the_median_first_request_and_the_rules_loaded_by_path` (`report.rs`) |

Nada cambió en `crates/clawg-core` ni en `crates/clawg-cli`: el cambio es del arnés y de sus datos.

## El CI (2026-11-12)

Los commits del paso 1 están en `9bd82ee4`, cuyo pull request a `main` pasó en `test (ubuntu-latest)` y en `test (windows-latest)`: [la corrida](https://github.com/MarcossIC/claw-garden/actions/runs/43249735426).

## La campaña (2026-11-11)

Paga, con un cliente real: Sonnet 5.5, esfuerzo `high`, Claude Code 2.1.322, en inglés, las seis tareas del piloto bajo `a3`, `a4`, `l0`, `l1` y `l2`, dos corridas cada una, con las 15 reglas de `noise.json` en todo brazo. Son 60 sesiones y 1,7362 USD, más 0,0718 USD de dos sesiones de humo (`l1` y `l2` sobre K3). El directorio de las corridas y la guía con que se corrió quedan fuera de git. `a0` no se repitió: la regla de decisión lo lee del piloto, donde dio S, C y N 2 de 2.

| | `a3` | `a4` | `l0` | `l1` | `l2` |
|---|---|---|---|---|---|
| K1 + K2 + K3 | 0/6 | 6/6 | 0/6 | 6/6 | 6/6 |
| S | 2/2 | 0/2 | 2/2 | 0/2 | 0/2 |
| C | 2/2 | 0/2 | 2/2 | 2/2 | 2/2 |
| N | 2/2 | 2/2 | 2/2 | 2/2 | 2/2 |
| Primer pedido, mediana por tarea (tokens) | 20.454 a 20.495 | 17.615 a 17.902 | 16.901 a 16.943 | 17.718 a 18.021 | 17.167 a 17.253 |
| Costo medio por sesión (USD) | 0,0379 | 0,0292 | 0,0230 | 0,0266 | 0,0280 |
| Reglas cargadas por ruta, media por sesión | 0 | 0 | 0 | 0 | 1 (2 en K3) |

**Ningún brazo cumple la regla de decisión** que la [exploración](exploration.md#el-experimento) fijó antes de correr (K al menos como `a4`, S más veces que `a4`, C y N sin empeorar frente a `a0`, y el primer pedido sin pasar del de `a3`):

- `a3` y `l0` no entregan la memoria: fallan K, y aprueban S sólo porque la regla vieja tampoco llega. Sus trazadores salen `never_reached`.
- `l1` y `l2` igualan a `a4` en K, pero empujan también la decisión vieja de S, y el agente la siguió en las 4 sesiones, aunque su línea decía la antigüedad (145 días) y la cabecera avisa que una regla vieja puede estar desactualizada. Ninguna sesión leyó el script del fixture que la contradice. Es el fallo de la nativa en S, repetido: no superan a `a4`, la igualan.
- Un corte por antigüedad que frene S y deje pasar K3 tendría que caer entre 121 y 145 días, porque la decisión de K3 tiene 120: sería ajustarlo al fixture, no una regla.

Con eso el paso 2 (construir en el producto la selección, la entrega y la dieta del ganador) y el paso 3 (repetir la campaña con el build) no se hacen bajo este cambio: no hay ganador que construir.

### Lo que la campaña sí mostró

- **El cliente lee `.claude/rules/` en `-p` y avisa `InstructionsLoaded`.** Toda sesión `l2` cargó su archivo de reglas con `load_reason` `path_glob_match`. Era el primer riesgo de la [propuesta](proposal.md#riesgos).
- **Una regla por archivo que llega después de un `Write` hace que el agente corrija.** En K3 bajo `l2` el agente escribió el archivo nuevo con guiones, el cliente cargó entonces la regla de `**/*.md` y el agente lo reescribió con asteriscos, en las dos corridas y en el humo. Cuesta un `Write` de más: 7 turnos y 0,0374 USD contra 6 y 0,0323 en `l1`. En K2 la regla de `**/*.py` llegó con el primer `Read`, antes de escribir. Responde la segunda pregunta abierta de la exploración.
- **El bloque del protocolo cuesta unos 3.550 tokens por sesión y no cambió ningún resultado.** `l0` da lo mismo que `a3` en las seis tareas con el primer pedido en unos 16.920 tokens contra unos 20.470. El límite de la lectura: en estas sesiones `a3` casi no llamó a clawg con el bloque (1,5 llamadas de media en C, 0 en las demás tareas), así que no dice qué pasa en una sesión larga, donde el agente sí busca. Es el dato que esperaba [claude-code-plugin](../../claude-code-plugin/exploration.md) (120).
- **Ninguna sesión `l1` o `l2` llamó a una herramienta de clawg.** Con las reglas delante el agente no buscó nada más.
- **El ruido no movió los controles.** N dio 2 de 2 en todos los brazos con 15 reglas de otros temas.

### Lo que no cuenta como resultado

El 0 de 2 de `a4` en C es del arnés, no de la memoria nativa. Desde MD3 el `MEMORY.md` se escribe de la memoria más nueva a la más vieja y sin fechas, así que la decisión vigente (18 días) queda arriba y la vieja (187 días) abajo; el agente eligió en las dos sesiones «la que aparece más tarde». En el piloto el archivo seguía el orden de la tarea, la vieja primero, y `a4` dio 2 de 2. La ventaja de `l1` y `l2` sobre `a4` en C no es un resultado. Antes de repetir una campaña con `a4`, su `MEMORY.md` tiene que ir de la más vieja a la más nueva, como lo deja un agente que agrega notas al final; MD3 pide hoy el otro orden.

### Lo que la campaña no podía medir

- Las tareas K sólo se pueden empatar con la nativa: con 16 memorias, las dos entregan todo, y gana quien entrega, no quien elige.
- S no se arregla entregando mejor. Es mantener la memoria al día: saber que la decisión vieja ya no vale.
- Las sesiones del arnés tienen de 2 a 7 turnos y unos 17.000 tokens. Lo que una memoria ahorra o cuesta en una sesión larga no aparece ahí.

## Preguntas de la exploración que siguen abiertas

- Si un `CLAUDE_CONFIG_DIR` propio lee `rules/` como reglas de usuario: no se probó, las reglas de `l2` van en el directorio de trabajo.
- Cuántos tokens agregan los resultados de las herramientas de clawg en cada llamada. La parte de `save_memory` la tomó [verdict-on-replace](../2026-11-12-verdict-on-replace/proposal.md) (128).
- B (`anthropic/alwaysLoad`) no se midió: iba en una segunda tanda sobre el ganador.
- El canal de reglas de Cursor.

## Al archivar (2026-11-13)

- MD1 a MD7 pasan con sus ids a [`specs/eval`](../../../specs/eval/spec.md), en «`agent-eval`: las reglas, el ruido y los brazos de entrega». La base del arnés (AE1 a AE6) sigue en la spec de [agent-eval](../../agent-eval/spec.md) (96), que no está archivado; esa sección la cita.
- Ninguna spec del producto cambia, y `docs/reference/testing.md` ya describe los brazos, el ruido y el registro de instrucciones desde `eb909d62`.
- Sale en 0.86.0. Como sólo agrega documentos sobre `c575889d`, la suite no se corrió al archivar.
- Lo que queda (sacar o no el bloque del protocolo, el orden del `MEMORY.md` de `a4`, B y las preguntas abiertas) no es de este cambio: cada cosa se propone aparte si se decide hacerla.
