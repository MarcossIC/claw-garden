# agent-eval — Especificación

Suma al arnés de evaluación un binario, `agent-eval`. Ninguna spec del producto cambia. Origen: [proposal](proposal.md). Todos los tests corren con la suite, sin modelo, red, daemon ni `claude`, y no dejan nada en el temporal.

## AE1. Las tareas

Una tarea MUST ser un archivo JSON en `data/agent-tasks/tasks/` con id (minúsculas, dígitos y guiones), categoría (`K1` a `K5`, `S`, `C`, `N`, `Q`, `D`, `R`), fixture (un directorio de `fixtures/`), prompt, memorias sembradas, trazadores y al menos un check (`answer_contains`, `answer_lacks`, `file_contains`, `file_lacks`, `tool_called`). Un campo desconocido, un tipo de memoria que `save_memory` no toma, una importancia fuera de 0 a 10, una ruta de check que sale del directorio de trabajo, un trazador que no está en ninguna memoria o que está en el prompt, un id repetido o un fixture ausente MUST rechazar la tarea nombrando por qué. `agent-eval tasks` MUST además fallar si un trazador está en un archivo del fixture.

#### Escenario: un trazador filtrado
- GIVEN una tarea cuyo trazador `48213` está en `deploy.env` de su fixture
- WHEN se audita
- THEN el resultado nombra el trazador y el archivo

**Tests:** `a_task_reads_with_its_defaults`, `an_unknown_field_or_check_is_refused`, `a_task_names_what_is_wrong_with_it`, `a_tracer_in_the_prompt_or_the_fixture_gives_the_fact_away`, `a_path_must_stay_inside`, `load_reads_every_task_and_refuses_a_missing_fixture_or_a_repeated_id` (`task.rs`); `the_example_tasks_are_valid_and_leak_no_tracer` (`tests/agent_eval.rs`).

## AE2. Los brazos

`arm::plan` MUST decidir, sin tocar disco, los archivos, los comandos y las variables de una corrida bajo `a0`, `a2`, `a3`, `a3-eager`, `a4` o `b`. Toda corrida MUST tener su home y su `CLAUDE_CONFIG_DIR` propios, `--mcp-config` con `--strict-mcp-config` y salida `stream-json`. Los brazos con clawg MUST llevar el servidor que `setup::plan` registraría; `a3`, `a3-eager` y `b` además MUST llevar el `settings.json` y el `CLAUDE.md` que ese plan escribe, en el idioma de la corrida, y `preferences.json` cuando hay idioma. `a3-eager` MUST apagar la búsqueda de herramientas, `b` MUST quitar `search_memory`, `read_memory` y `list_memories`, y `a4` MUST llevar las mismas memorias en el `MEMORY.md` nativo, sin servidor. `a0` MUST no tener servidor, siembra ni memoria nativa. Una ruta de `clawg` relativa MUST rechazarse.

#### Escenario: lo que instala setup
- GIVEN una tarea con memorias y el idioma `es`
- WHEN se planea bajo `a3`
- THEN el `settings.json` tiene los hooks `session-start` y `user-prompt-submit` de `setup`, el `CLAUDE.md` el protocolo en español y `preferences.json` dice `es`

**Tests:** los de `arm.rs`.

## AE3. La siembra

El export MUST ser un archivo `clawg-memories` que `clawg import` lee: la cabecera, el proyecto con la raíz del directorio de trabajo y cada memoria con `created_at` y `last_accessed_at` de su edad, sus accesos, su clave y el tier que `eval_api::earned_tier` da (el de `compute_tier` desde el tier 3). La memoria nativa MUST tener cada texto entero.

**Tests:** `a_seeded_row_earns_the_tier_the_store_promotes_it_to` (`eval_api.rs`); los de `seed.rs`.

## AE4. El stream

`stream::parse` MUST leer las llamadas a herramientas con su resultado, si son de un subagente y si hubo un `ToolSearch` antes, el texto de los eventos de hook, los textos del asistente, las compactaciones y el resultado final con su costo y sus tokens. Una línea de un tipo desconocido MUST contarse y una que no es JSON reportarse por número, sin fallar.

**Tests:** los de `stream.rs`.

## AE5. El puntaje y la rúbrica

Una corrida MUST ser `success` sólo si terminó sin error y pasó todos sus checks. Cada trazador MUST salir `used` si el agente lo escribió (respuesta, texto o entrada de una herramienta), `retrieved_but_ignored` si lo trajo un resultado o un hook y no lo escribió, y `never_reached` si no. Cada `save_memory` MUST juzgarse con los criterios de `rubric::CRITERIA`, cada uno `pass`, `fail` o `not_applicable`.

**Tests:** los de `score.rs` y `rubric.rs`; `evaluate_counts_clawg_calls_and_judges_the_saves`, `the_prod_server_is_counted_by_its_own_name` (`run.rs`).

## AE6. La corrida en seco

`agent-eval run … --replay STREAM` MUST preparar la corrida en un directorio ausente o vacío (nunca sobre otra), lanzar `agent-eval replay-client STREAM` en su directorio de trabajo con las variables del brazo, guardar su salida en `stream.jsonl` y escribir `result.json`. El cliente reproducido MUST aplicar los `Write` y `Edit` del stream y MUST rechazar una ruta fuera del directorio de trabajo. `agent-eval score --run DIR` MUST reproducir el resultado.

#### Escenario: con y sin clawg
- GIVEN la tarea `example-staging-port`
- WHEN se reproduce su stream de `a3` y el de `a0`
- THEN la primera sale `success` con `48213` `used` y una llamada a `search_memory` después de un `ToolSearch`, y la segunda `failure` con `48213` `never_reached`

**Tests:** los de `replay.rs`; `prepare_lays_out_the_run_and_never_writes_over_another`, `score_dir_reads_the_run_back_and_writes_its_result` (`run.rs`); `a_replayed_session_with_clawg_finds_the_fact_and_succeeds`, `a_replayed_session_without_memory_never_reaches_the_fact`, `a_replayed_edit_changes_the_working_directory_the_checks_read`, `a_run_never_writes_over_another` (`tests/agent_eval.rs`).
