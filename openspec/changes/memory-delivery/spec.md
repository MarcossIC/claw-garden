# memory-delivery — Especificación (paso 1)

Amplía el arnés de [agent-eval](../agent-eval/spec.md); ninguna spec del producto cambia. Origen: [proposal](proposal.md). Los tests corren con la suite, sin modelo real, red ni `claude`, y no dejan nada en el temporal.

## MD1. Una memoria que dijo la persona

Una memoria sembrada MAY llevar `user_stated: true`. El export MUST sembrarla con `created_by_type = 'user'`, como guarda `save_memory` una memoria explícita que dijo la persona, y sin el campo MUST sembrarla con `'agent'`, como hasta ahora. Un `run.json` anterior, sin el campo, MUST seguir leyéndose.

#### Escenario: la preferencia de K1
- GIVEN una tarea cuya preferencia lleva `user_stated: true`
- WHEN se exporta la siembra
- THEN su fila dice `created_by_type` `user` y las demás `agent`

**Tests:** `a_seeded_memory_the_person_stated_is_the_persons` (`seed.rs`); `a_task_reads_with_its_defaults` (`task.rs`).

## MD2. Las reglas y el ruido

Una memoria sembrada MAY llevar `rule`: `{}` es una regla de toda sesión y `{"paths": [...]}` una regla de los archivos que esos globs nombran, relativos al directorio de trabajo. Un glob vacío o que sale del directorio de trabajo MUST rechazar la tarea nombrando por qué. `data/agent-tasks/noise.json`, cuando existe, MUST ser una lista de memorias sembradas, cada una válida como las de una tarea y cada una con `rule`; un trazador de cualquier tarea en una de ellas MUST rechazarse, en `agent-eval tasks` y en cada corrida, nombrando la tarea, el trazador y la memoria. Un directorio de datos sin el archivo no tiene ruido. `agent-eval tasks` MUST listar cuántas reglas tiene el ruido.

#### Escenario: un ruido que da el hecho
- GIVEN un ruido con una memoria que contiene `48213`, el trazador de una tarea
- WHEN se cargan las tareas y el ruido
- THEN la carga falla nombrando la tarea, `48213` y la memoria

**Tests:** `a_rule_reads_with_or_without_paths`, `a_rule_path_must_stay_inside`, `the_noise_is_rules_that_hold_no_tracer`, `a_data_directory_without_noise_has_none` (`task.rs`); `the_example_tasks_are_valid_and_leak_no_tracer` (`tests/agent_eval.rs`).

## MD3. Lo que siembra cada brazo

Todo brazo con memoria MUST sembrar el ruido junto a las memorias de la tarea, en un solo orden, de la más nueva a la más vieja y, a igual edad, el ruido antes que la tarea: en el export, en el `MEMORY.md` de `a4` y en las reglas de `l1` y `l2`. `a0` MUST seguir sin siembra ni memoria nativa. Una tarea sin memorias MUST sembrar el ruido.

**Tests:** `the_seed_goes_from_the_newest_memory_to_the_oldest` (`seed.rs`); `every_arm_with_memory_seeds_the_noise_with_the_task` (`arm.rs`); `a_replayed_session_with_clawg_finds_the_fact_and_succeeds` (`tests/agent_eval.rs`).

## MD4. Los brazos `l0`, `l1` y `l2`

`l0` MUST ser `a3` sin el `CLAUDE.md` del protocolo: el mismo servidor, los mismos hooks de `setup` y la misma siembra. `l1` MUST ser `l0` más un hook `SessionStart` que corre `agent-eval hook-context FILE` sobre un archivo del directorio de la corrida con las reglas sembradas. `l2` MUST ser `l1` con cada regla que tiene globs en un archivo de `.claude/rules/clawg/` del directorio de trabajo, con esos globs en su `paths:`, y fuera del texto del arranque; las reglas de los mismos globs MUST compartir archivo. Una regla MUST escribirse en una línea con su antigüedad en días, el título y el cuerpo de la memoria unidos con ` — `, bajo una cabecera que dice que una regla vieja puede estar desactualizada, en el idioma de la corrida (inglés salvo `--lang es`). Sin reglas que entregar, `l1` y `l2` MUST no agregar el hook ni el archivo. `agent-eval hook-context FILE` MUST imprimir el texto de FILE como el `additionalContext` de `SessionStart`.

#### Escenario: una regla por archivo
- GIVEN una tarea con una regla de `**/*.md` y otra de toda sesión
- WHEN se planea bajo `l2`
- THEN `.claude/rules/clawg/` tiene un archivo con `paths:` `"**/*.md"` y esa regla, y el texto del arranque tiene sólo la otra

**Tests:** `a_rule_is_one_line_with_its_age`, `the_startup_holds_every_rule_or_only_those_without_paths`, `the_rules_by_file_share_a_file_by_their_globs`, `the_hook_output_is_the_additional_context_of_session_start` (`rules.rs`); `l0_is_a3_without_the_protocol`, `l1_adds_the_rules_at_startup_through_a_hook_of_the_harness`, `l2_moves_the_rules_with_globs_to_rules_files`, `l1_without_rules_adds_no_hook` (`arm.rs`); `the_hook_commands_parse` (`main.rs`); `the_harness_hooks_print_the_context_and_log_their_input` (`tests/agent_eval.rs`).

## MD5. Los archivos de instrucciones que carga el cliente

Todo brazo MUST llevar un hook `InstructionsLoaded` que corre `agent-eval hook-log FILE`, con FILE el `instructions.jsonl` del directorio de la corrida; `hook-log` MUST agregar su entrada a FILE en una línea, y una entrada que no es JSON MUST guardarse como texto sin fallar. En los brazos con `setup` los hooks MUST sumarse a los del `settings.json` que escribe `setup`, sin quitar ninguno. `result.json` MUST llevar, en `session`, cada archivo cargado con su `load_reason`, y un registro ausente MUST leerse como ninguno.

**Tests:** `every_arm_logs_the_instruction_files_the_client_loads` (`arm.rs`); `a_log_line_keeps_the_input_on_one_line` (`main.rs`); `the_log_reads_each_loaded_file_and_skips_what_is_not_an_entry` (`run.rs`); `the_harness_hooks_print_the_context_and_log_their_input` (`tests/agent_eval.rs`).

## MD6. Lo que llegó sin pasar por el stream

Un trazador MUST salir `retrieved_but_ignored`, si el agente no lo escribió, cuando está en un texto que llegó al contexto sin pasar por el stream: el `MEMORY.md` que el plan de `a4` escribe, y todo archivo del plan que el registro de instrucciones da por cargado, comparando las rutas desde el directorio de la corrida, sin distinguir separadores ni mayúsculas.

#### Escenario: la nativa
- GIVEN una corrida de `a4` cuyo `MEMORY.md` tiene el trazador y un stream que no lo muestra
- WHEN se puntúa
- THEN el trazador sale `retrieved_but_ignored`, y bajo `a3` con el mismo stream `never_reached`

**Tests:** `a_tracer_in_what_reached_the_context_counts_as_retrieved` (`score.rs`); `the_native_memory_and_the_loaded_rules_files_are_delivered` (`run.rs`).

## MD7. Los tokens del primer pedido y el reporte

`stream::parse` MUST leer los tokens del primer pedido: la entrada, la caché leída y la caché escrita del `usage` del primer mensaje del asistente del bucle principal; un stream sin ese mensaje MUST dejarlos ausentes. `result.json` MUST llevarlos en `session`. `agent-eval report` MUST sumar a cada fila la mediana de los tokens del primer pedido de las corridas que los dan y la media de los archivos cargados por ruta (`load_reason` `path_glob_match`), y ordenar los brazos `l0`, `l1` y `l2` después de los demás.

**Tests:** `the_first_request_is_the_first_message_of_the_main_loop` (`stream.rs`); `rows_carry_the_median_first_request_and_the_rules_loaded_by_path` (`report.rs`).
