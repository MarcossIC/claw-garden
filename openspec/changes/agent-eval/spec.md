# agent-eval — Especificación

Suma al arnés de evaluación un binario, `agent-eval`. Ninguna spec del producto cambia. Origen: [proposal](proposal.md). Todos los tests corren con la suite, sin modelo real, red ni `claude`, y no dejan nada en el temporal; los del corredor (AE7) arrancan el `clawg` `testing` que la suite construye, con el modelo falso.

## AE1. Las tareas

Una tarea MUST ser un archivo JSON en `data/agent-tasks/tasks/` con id (minúsculas, dígitos y guiones), categoría (`K1` a `K5`, `S`, `C`, `N`, `Q`, `D`, `R`), fixture (un directorio de `fixtures/`), prompt, memorias sembradas, trazadores y al menos un check (`answer_contains`, `answer_lacks`, `file_contains`, `file_lacks`, `tool_called`). Un campo desconocido, un tipo de memoria que `save_memory` no toma, una importancia fuera de 0 a 10, una ruta de check que sale del directorio de trabajo, un trazador que no está en ninguna memoria o que está en el prompt, un id repetido o un fixture ausente MUST rechazar la tarea nombrando por qué. `agent-eval tasks` MUST además fallar si un trazador está en un archivo del fixture.

Las categorías dicen qué mide la tarea: `K1` una preferencia de la persona, `K2` un hecho del proyecto o del equipo que el código no dice, `K3` una decisión con su motivo, `S` una memoria vieja que el código ya contradice, `C` dos memorias vivas que se contradicen, `N` un control que no necesita memoria, `Q` la calidad de lo que el agente guarda, `D` una decisión que se revisa entre dos sesiones y `R` una sesión retomada. `K4` y `K5` todavía no tienen definición ni tareas. En `K` el brazo sin memoria tiene que fallar; en `S` y `C` aprueba solo, y lo que se mide es el daño, `success(a0) − success(brazo)`. Una tarea o un fixture que viene de otro proyecto MUST nombrar en `data/agent-tasks/SOURCES.md` su fuente, su commit, su licencia y qué escribió la tarea por su cuenta, y un fixture MUST copiarse sin cambios: lo que está dentro de su directorio es lo que ve el agente.

#### Escenario: un trazador filtrado
- GIVEN una tarea cuyo trazador `48213` está en `deploy.env` de su fixture
- WHEN se audita
- THEN el resultado nombra el trazador y el archivo

**Tests:** `a_task_reads_with_its_defaults`, `an_unknown_field_or_check_is_refused`, `a_task_names_what_is_wrong_with_it`, `a_tracer_in_the_prompt_or_the_fixture_gives_the_fact_away`, `a_path_must_stay_inside`, `load_reads_every_task_and_refuses_a_missing_fixture_or_a_repeated_id` (`task.rs`); `the_example_tasks_are_valid_and_leak_no_tracer` (`tests/agent_eval.rs`).

## AE2. Los brazos

`arm::plan` MUST decidir, sin tocar disco, los archivos, los comandos y las variables de una corrida bajo `a0`, `a2`, `a3`, `a3-eager`, `a4` o `b`. Toda corrida MUST tener su home y su `CLAUDE_CONFIG_DIR` propios, `--mcp-config` con `--strict-mcp-config` y salida `stream-json`. Los brazos con clawg MUST llevar el servidor que `setup::plan` registraría; `a3`, `a3-eager` y `b` además MUST llevar el `settings.json` y el `CLAUDE.md` que ese plan escribe, en el idioma de la corrida, y `preferences.json` cuando hay idioma. `a3-eager` MUST apagar la búsqueda de herramientas, `b` MUST quitar `search_memory`, `read_memory` y `list_memories`, y `a4` MUST llevar las mismas memorias en el `MEMORY.md` nativo, sin servidor. `a0` MUST no tener servidor, siembra ni memoria nativa. Una ruta de `clawg` relativa MUST rechazarse. Como en `-p` nada puede aprobar un permiso, todo brazo MUST correr con `--permission-mode acceptEdits` (ediciones aprobadas sólo dentro del directorio de trabajo) y negar `Bash`, y los brazos con clawg MUST permitir todas las herramientas de su servidor (`--allowedTools mcp__<servidor>`); `b` MUST negar sus tres herramientas de lectura en el mismo `--disallowedTools` que `Bash`.

#### Escenario: lo que instala setup
- GIVEN una tarea con memorias y el idioma `es`
- WHEN se planea bajo `a3`
- THEN el `settings.json` tiene los hooks `session-start` y `user-prompt-submit` de `setup`, el `CLAUDE.md` el protocolo en español y `preferences.json` dice `es`

**Tests:** los de `arm.rs`, entre ellos `every_arm_may_edit_its_directory_and_none_may_run_a_shell`, `the_arms_with_clawg_may_call_its_server_without_a_prompt` y `a_denies_only_bash_while_b_also_denies_its_read_tools_in_one_value`.

## AE3. La siembra

El export MUST ser un archivo `clawg-memories` que `clawg import` lee: la cabecera, el proyecto con la raíz del directorio de trabajo y cada memoria con `created_at` y `last_accessed_at` de su edad, sus accesos, su clave y el tier que `eval_api::earned_tier` da (el de `compute_tier` desde el tier 3). La memoria nativa MUST tener cada texto entero.

**Tests:** `a_seeded_row_earns_the_tier_the_store_promotes_it_to` (`eval_api.rs`); los de `seed.rs`.

## AE4. El stream

`stream::parse` MUST leer las llamadas a herramientas con su resultado, si son de un subagente y si hubo un `ToolSearch` antes, el texto de los eventos de hook, los textos del asistente, las compactaciones, la credencial que pagó (`apiKeySource` de la línea `init`) y el resultado final con su costo y sus tokens. Los tokens del resultado MUST incluir los del bucle principal (`usage`), con las escrituras de caché de 5 minutos y de 1 hora por separado (`usage.cache_creation`), y los de cada modelo (`modelUsage`): entrada, salida, lectura y escritura de caché, el costo que estima el cliente y la tabla con que lo estimó (`costBasis`). `modelUsage` cuenta también los subagentes y la respuesta que pasó `--max-budget-usd`, que `usage` deja afuera. Un campo que el stream no trae MUST leerse como ausente o cero. Una línea de un tipo desconocido MUST contarse y una que no es JSON reportarse por número, sin fallar.

**Tests:** los de `stream.rs` (entre ellos `the_outcome_carries_the_tokens_of_each_model_and_the_cache_writes_by_duration` y `a_stream_without_the_newer_fields_reads_them_as_absent`).

## AE5. El puntaje y la rúbrica

Una corrida MUST ser `success` sólo si terminó sin error y pasó todos sus checks. Un check de archivo MUST comparar con `\r\n` vuelto `\n` en el archivo y en el texto, porque el agente puede escribir CRLF. Cada trazador MUST salir `used` si el agente lo escribió (respuesta, texto o entrada de una herramienta), `retrieved_but_ignored` si lo trajo un resultado o un hook y no lo escribió, y `never_reached` si no. El id de la tarea MUST quitarse de cada texto antes de buscar el trazador, sin distinguir mayúsculas: el directorio de una corrida sin pago puede llevar el nombre de su tarea, y entonces cada ruta que la sesión escribe o lee lo contiene, y un id como `memorycode-d14-import-secrets` contiene el trazador `secrets`. Cada `save_memory` MUST juzgarse con los criterios de `rubric::CRITERIA`, cada uno `pass`, `fail` o `not_applicable`.

**Tests:** los de `score.rs` (entre ellos `file_checks_ignore_carriage_returns` y `a_tracer_in_the_task_id_is_not_seen_in_the_paths_of_the_run`) y `rubric.rs`; `evaluate_counts_clawg_calls_and_judges_the_saves`, `the_prod_server_is_counted_by_its_own_name` (`run.rs`).

## AE6. La corrida en seco

`agent-eval run … --replay STREAM` MUST preparar la corrida en un directorio ausente o vacío (nunca sobre otra), lanzar `agent-eval replay-client STREAM` en su directorio de trabajo con las variables del brazo, guardar su salida en `stream.jsonl` y escribir `result.json`. El cliente reproducido MUST aplicar los `Write` y `Edit` del stream y MUST rechazar una ruta fuera del directorio de trabajo. `agent-eval score --run DIR` MUST reproducir el resultado. `result.json` MUST llevar, en `session`, la credencial que pagó y cuándo empezó y terminó la sesión en UTC: la creación y la última escritura de `stream.jsonl`, que se crea justo antes de lanzar el cliente y se cierra cuando sale. Una plataforma que no da la creación de un archivo deja el inicio ausente.

#### Escenario: con y sin clawg
- GIVEN la tarea `example-staging-port`
- WHEN se reproduce su stream de `a3` y el de `a0`
- THEN la primera sale `success` con `48213` `used` y una llamada a `search_memory` después de un `ToolSearch`, y la segunda `failure` con `48213` `never_reached`

**Tests:** los de `replay.rs`; `prepare_lays_out_the_run_and_never_writes_over_another`, `score_dir_reads_the_run_back_and_writes_its_result`, `score_dir_dates_the_session_by_its_stream_and_names_the_credential` (`run.rs`); `a_replayed_session_with_clawg_finds_the_fact_and_succeeds`, `a_replayed_session_without_memory_never_reaches_the_fact`, `a_replayed_edit_changes_the_working_directory_the_checks_read`, `a_run_never_writes_over_another` (`tests/agent_eval.rs`).

## AE7. El corredor

`agent-eval run` MUST correr la sesión que `prepare` dejó en `run.json`. En un brazo con clawg MUST arrancar `clawg serve --profile P --port 0` con el home de la corrida, esperar a que el `daemon.json` que publica nombre su pid y a que `clawg status` diga que el modelo de ese daemon está listo, cargar la siembra con `clawg import`, lanzar el cliente y cerrar con `clawg stop`, que MUST correr también cuando la siembra o el cliente fallaron; una siembra que no carga MUST no lanzar el cliente, y un daemon que termina antes de escuchar MUST no dejar que corra nada más. Un brazo sin clawg MUST no arrancar ningún clawg. Todo proceso de la corrida MUST recibir su home en `HOME`, `USERPROFILE` y `CLAWG_TEST_HOME`. En Windows un brazo con clawg MUST negarse, antes de preparar nada, si el `--clawg` no lleva `CLAWG_TEST_HOME`, la marca de un build `testing`. `run.json` MUST nombrar el cliente que juega la sesión, y `score --run` MUST puntuarla como de ese cliente. Sin `--replay` el cliente es `claude`: la corrida MUST pedir `--max-budget-usd` y `--claude-version` y negarse, antes de preparar nada, si `claude --version` informa otra versión. Después MUST negarse también, nombrando cada problema, si falta `ANTHROPIC_API_KEY` o está vacía (en `-p` la clave es la que paga y el directorio de configuración de la corrida no tiene login), si está `ANTHROPIC_AUTH_TOKEN` (le gana a la clave) o `CLAUDE_CODE_EFFORT_LEVEL` (le gana a `--effort`), si `--out` o alguno de sus ancestros tiene `.git`, `CLAUDE.md`, `CLAUDE.local.md` o `.claude/CLAUDE.md` (el cliente lee los archivos de instrucciones de cada ancestro de su directorio de trabajo y toma como proyecto el repositorio que lo contiene), o si la ruta del directorio de trabajo contiene, sin distinguir mayúsculas, el id de la tarea o uno de sus trazadores: el cliente le dice al agente dónde trabaja, y un directorio `madr-0011-asterisk-list-marker.a0.1` le da a `a0` la decisión que la tarea quiere medir.

#### Escenario: el daemon de la corrida
- GIVEN la tarea `example-staging-port`, un `clawg` `testing` y el modelo falso
- WHEN se reproduce su stream de `a3`
- THEN el store queda en el home de la corrida con la memoria importada, `clawg import` corrió después de que `clawg status` diera el modelo listo y `clawg stop` después de `clawg import`, no queda `daemon.json` y la corrida sale `success`

#### Escenario: un cliente que falla
- GIVEN un stream cuyo `Write` sale del directorio de trabajo
- WHEN se reproduce bajo `a3`
- THEN la corrida falla nombrando `client.log`, no escribe `result.json` y el daemon igual se detuvo

#### Escenario: un `claude` de otra versión
- GIVEN un `claude` en el `PATH` que no informa `2.1.300`
- WHEN se corre `run` sin `--replay` con `--claude-version 2.1.300`
- THEN la corrida se niega nombrando la versión y no existe su directorio

**Tests:** `the_arms_with_clawg_start_a_daemon_of_the_run_on_an_ephemeral_port`, `every_process_of_a_run_finds_its_home_also_in_a_testing_build`, `the_plan_reads_back_from_its_json` (`arm.rs`); `on_windows_a_run_with_clawg_needs_a_testing_build`, `the_mark_is_found_wherever_a_read_cuts_it`, `a_daemon_listens_once_its_state_file_names_it`, `the_model_is_ready_once_the_status_of_this_daemon_says_so`, `a_step_that_fails_is_told_by_its_last_line`, `the_client_is_claude_as_planned_or_a_replay_of_a_stream`, `the_version_claude_reports_is_its_first_word`, `prepare_lays_out_the_run_and_never_writes_over_another`, `score_dir_reads_the_run_back_and_writes_its_result` (`run.rs`); `a_run_without_a_replay_starts_claude_only_with_a_budget_and_a_pinned_version` (`main.rs`); `a_replayed_session_with_clawg_finds_the_fact_and_succeeds`, `a_replayed_session_without_memory_never_reaches_the_fact`, `a_replayed_edit_changes_the_working_directory_the_checks_read`, `a_client_that_fails_still_stops_the_daemon_of_its_run`, `a_daemon_that_never_listens_starts_no_client`, `a_run_with_claude_refuses_another_version_before_laying_out_the_run` (`tests/agent_eval.rs`); `a_paid_run_with_its_key_and_a_clean_directory_has_no_leaks`, `a_missing_or_empty_key_a_token_and_an_effort_level_are_each_named`, `an_out_below_a_repository_or_an_instruction_file_is_named_with_the_ancestor_that_holds_it`, `the_probe_finds_what_an_ancestor_of_a_missing_out_holds`, `a_working_directory_that_names_its_task_or_a_tracer_is_named` (`run.rs`).

## AE8. El reporte

`agent-eval report --runs DIR` MUST leer cada `result.json` hasta tres niveles debajo de DIR, sin bajar dentro de una corrida, y escribir una tabla Markdown con una fila por tarea y brazo, en orden de tarea y de brazo: categoría, corridas, éxitos `k/n`, corridas que no terminaron bien, y las medias de costo, turnos, segundos y llamadas a clawg de las corridas que dan ese número, con el destino de los trazadores sumado. Una tarea de ejemplo MUST salir marcada. Al final MUST dar el total de corridas, de éxitos y de costo, y cuántas corridas no dieron costo, y en otra línea los tokens sumados: de cada modelo de una corrida que los da por modelo y del bucle principal de una que no, con las escrituras de caché de 5 minutos y de 1 hora del bucle principal. Un DIR sin ningún resultado, o con un `result.json` inválido, MUST ser un error.

**Tests:** `rows_come_in_task_then_arm_order_with_their_means_and_sums`, `the_totals_count_runs_successes_and_cost_with_the_runs_that_report_none`, `a_group_without_any_cost_has_none`, `the_table_is_markdown_with_the_example_marked`, `the_totals_add_the_tokens_of_every_model`, `load_finds_results_up_to_three_levels_down_and_skips_the_rest`, `load_reads_a_written_result_back_and_names_a_directory_without_any` (`report.rs`).

## AE9. Los costos por sesión

`agent-eval costs --runs DIR` MUST leer los mismos resultados que `report` y escribir un CSV con una línea por sesión y modelo: la ruta de la corrida debajo de DIR, la tarea, el brazo, el inicio y el fin en UTC, la credencial que pagó, el modelo, los tokens de entrada, salida, lectura y escritura de caché, las escrituras de caché de 5 minutos y de 1 hora, el costo que estimó el cliente y su `costBasis`. Las escrituras por duración son del bucle principal, así que MUST ir sólo en la línea del modelo de la sesión. Una sesión sin desglose por modelo MUST salir en una línea con los tokens de su bucle principal y su costo total. Una sesión que no imprimió resultado MUST salir en una línea sin cifras. Sirve para poner precio a una sesión por sus tokens y para encontrarla por su hora en el uso de la consola, que no conoce sesiones.

**Tests:** `the_costs_have_a_line_per_session_and_model_with_the_cache_durations_on_the_session_model`, `a_session_without_a_breakdown_by_model_is_costed_from_its_main_loop` (`report.rs`); `the_other_commands_parse` (`main.rs`).
