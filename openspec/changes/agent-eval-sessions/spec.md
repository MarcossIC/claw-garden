# agent-eval-sessions — Especificación

Amplía el arnés de [agent-eval](../agent-eval/spec.md) (AE1 a AE9) y la sección «`agent-eval`: las reglas, el ruido y los brazos de entrega» de [`specs/eval`](../../specs/eval/spec.md) (MD1 a MD7), de la que modifica MD3. Ninguna spec del producto cambia. Origen: [proposal](proposal.md); decisiones en [design](design.md). Los tests corren con la suite, sin modelo real, red ni `claude`, y no dejan nada en el temporal; los que juegan una corrida usan el `clawg` `testing` que la suite construye, con el modelo falso. Los archivos de código están en `crates/clawg-eval/src/bin/agent-eval/` y los de integración en `crates/clawg-eval/tests/agent_eval.rs`.

## AS1. El orden del `MEMORY.md` (modifica MD3)

El `MEMORY.md` de `a4` MUST ir de la memoria más vieja a la más nueva y, a igual edad, el ruido antes que la tarea: el orden en que lo deja un agente que agrega notas al final. El export y las reglas de `l1` y `l2` MUST seguir de la más nueva a la más vieja, como pide MD3.

#### Escenario: la tarea C
- GIVEN una tarea con una decisión de 187 días y otra de 18 sobre el mismo tema
- WHEN se planea bajo `a4`
- THEN en `MEMORY.md` la de 187 días está antes que la de 18, y en `seed.jsonl` de `a3` después

**Tests:** `the_native_memory_goes_from_the_oldest_memory_to_the_newest` (`seed.rs`); `every_arm_with_memory_seeds_the_noise_with_the_task` (`arm.rs`), con la aserción de `a4` invertida; `the_seed_goes_from_the_newest_memory_to_the_oldest` (`seed.rs`) sigue pasando sin cambios.

## AS2. Una tarea de varias sesiones

Una tarea MUST tener `prompt` o `sessions`, nunca los dos ni ninguno. `sessions` es una lista de una o más sesiones, cada una con `prompt` y, opcionales, `tracers` y `checks`. Una tarea con `prompt` es una tarea de una sesión, y las tareas y los `run.json` escritos antes de este cambio MUST seguir leyéndose sin tocarlos.

Un trazador de sesión es un hecho que nace en el prompt de esa sesión. MUST estar en ese prompt, y MUST rechazarse, nombrando por qué, si está en el prompt de una sesión posterior, en una memoria sembrada, en el ruido o en un archivo del fixture. Un trazador de la tarea sigue como en AE1 y MUST no estar en el prompt de ninguna sesión.

Un check declarado en una sesión se evalúa sobre esa sesión, y MUST ser de los que leen el stream (`answer_contains`, `answer_lacks`, `tool_called`, `tool_failed_count`); un check de archivo en una sesión MUST rechazar la tarea. Un check de la tarea se evalúa al final: sobre la respuesta de la última sesión, los archivos como quedaron y las llamadas de todas las sesiones. Una tarea MUST tener al menos un check, en la tarea o en una sesión.

En una corrida paga, la ruta del directorio de trabajo MUST rechazarse también si contiene un trazador de sesión.

#### Escenario: un hecho que se repite
- GIVEN una tarea de dos sesiones cuyo trazador `4.5, 41` de la sesión 1 está también en el prompt de la sesión 2
- WHEN se valida
- THEN se rechaza nombrando el trazador y la sesión 2

**Tests:** `a_task_has_one_prompt_or_a_list_of_sessions`, `a_tracer_of_a_session_is_born_in_its_prompt_and_told_nowhere_else`, `a_check_of_a_session_reads_only_the_stream`, `a_task_written_before_sessions_still_reads`, `the_noise_holds_no_tracer_born_in_a_session` (`task.rs`); `a_working_directory_that_names_a_tracer_of_a_session_is_named` (`run.rs`).

## AS3. Los fallos de una herramienta, el shell de una tarea y la categoría `E`

`tool_failed_count` MUST ser un check con `name`, `max` y, opcional, `text`: pasa si las llamadas a la herramienta `name` que fallaron son `max` o menos. Sin `text`, falló la que volvió marcada como error; con `text`, la que tiene `text` en su resultado, sin distinguir mayúsculas y esté o no marcada, porque un comando que no existe detrás de un `|` vuelve sin la marca. Cuentan también las llamadas de los subagentes.

Una tarea MAY llevar `shell`, una lista de nombres de comando (letras, dígitos, `-` y `_`). Con ella, todo brazo MUST dejar de negar `Bash` y permitir sólo esos comandos, cada uno como una regla con alcance; sin ella, todo brazo MUST seguir negando `Bash` como pide AE2. Una tarea MAY llevar `missing_commands`, comandos que no deben existir donde corre la sesión: cada uno MUST estar en `shell`, y una corrida paga MUST negarse, antes de preparar nada, si encuentra alguno en el `PATH`.

`E` MUST ser una categoría: un error del entorno que una sesión posterior puede evitar.

#### Escenario: un comando que no existe detrás de un pipe
- GIVEN un stream con una llamada a `Bash` cuyo resultado dice `eza: command not found` y no está marcado como error
- WHEN se evalúa `tool_failed_count` de `Bash` con `text` `eza: command not found` y `max` 0
- THEN el check falla, y sin `text` pasa

**Tests:** `the_failed_calls_of_a_tool_are_those_marked_or_those_that_hold_a_text` (`score.rs`); `a_task_names_the_shell_commands_it_allows_and_those_that_must_be_missing`, `e_is_a_category` (`task.rs`); `a_task_with_a_shell_allows_its_commands_and_no_other`, `every_arm_may_edit_its_directory_and_none_may_run_a_shell` (`arm.rs`), que sigue pasando para las tareas sin `shell`; `a_paid_run_refuses_a_command_that_should_be_missing` (`run.rs`).

## AS4. El plan de una corrida de varias sesiones

`arm::plan` MUST decidir un comando de cliente por sesión, cada uno con el prompt de su sesión y los mismos argumentos, variables y archivos. La forma de la corrida es `split`, `joined` o `both`, dada con `--sessions` y `split` por omisión: en `joined`, el comando de cada sesión posterior a la primera MUST llevar `--continue`; en `split` ninguno. `both` MUST aceptarse sólo para una tarea de dos sesiones, y una tarea de una sesión MUST aceptar sólo `split`; lo demás es un error de argumentos que nombra por qué.

El plan de una tarea de una sesión MUST serializarse igual que antes de este cambio, y un `run.json` anterior MUST leerse como una corrida de una sesión. `run.json` MUST registrar la forma.

Los archivos de cada sesión MUST llamarse `stream.jsonl` y `client.log` en una corrida de una sesión, y `stream-K.jsonl` y `client-K.log`, con K desde 1, en una de varias.

#### Escenario: una corrida continua
- GIVEN una tarea de dos sesiones
- WHEN se planea `joined`
- THEN el primer comando lleva el prompt de la sesión 1 sin `--continue`, y el segundo el de la sesión 2 con `--continue`

**Tests:** `a_task_of_one_session_plans_what_it_planned_before`, `each_session_has_its_prompt_and_a_joined_one_continues_the_conversation`, `a_plan_of_several_sessions_reads_back_and_an_older_plan_reads_as_one` (`arm.rs`); `the_shape_of_a_run_parses_and_is_refused_where_the_task_cannot_take_it` (`main.rs`); `the_files_of_a_session_carry_its_number_only_when_there_are_several` (`run.rs`).

## AS5. El corredor de varias sesiones

`agent-eval run` MUST jugar las sesiones en orden, dentro de una misma corrida: un directorio de trabajo, un home, una configuración del cliente y, en un brazo con clawg, un daemon que arranca antes de la primera sesión, carga la siembra una vez y se detiene después de la última. Una sesión cuyo cliente no sale con 0, o cuyo resultado es un error o falta, MUST no dejar que empiece la siguiente; los comandos de después MUST correr igual. Las sesiones que sí terminaron MUST puntuarse, y la corrida MUST salir como no terminada.

Antes de cada sesión, en un brazo con memoria nativa, el corredor MUST copiar el `MEMORY.md` de ese momento a `memory-before-K.md`; después de cada sesión de una corrida de varias MUST mover el registro de instrucciones a `instructions-K.jsonl`.

#### Escenario: dos sesiones sobre un store
- GIVEN una tarea de ejemplo de dos sesiones y sus dos streams
- WHEN se reproduce bajo `a3`
- THEN `clawg import` corrió una vez, antes de la primera sesión, `clawg stop` una vez, después de la segunda, y la segunda sesión encontró en el directorio de trabajo el archivo que escribió la primera

**Tests:** `a_session_that_ends_in_an_error_starts_no_other` (`run.rs`); `a_replayed_run_of_two_sessions_shares_its_daemon_and_its_working_directory`, `a_session_that_fails_stops_the_run_and_still_stops_the_daemon` (`tests/agent_eval.rs`).

## AS6. La bifurcación

Con `--sessions both` la corrida MUST jugar la primera sesión una vez y la segunda dos veces desde el mismo estado: primero continuando la conversación, después en una sesión nueva. Entre una y otra MUST restaurar el directorio de trabajo, la configuración del cliente y el directorio de datos de clawg (sin sus modelos) como quedaron al terminar la primera sesión, con el daemon detenido mientras se copian. Las dos ramas MUST correr en las mismas rutas.

Cada rama MUST quedar en su subdirectorio, `joined/` y `split/`, con el stream de su segunda sesión, su registro de instrucciones, el directorio de trabajo como lo dejó y su `result.json`, que es el resultado de una corrida de dos sesiones de esa forma, marcado como par de la otra. El directorio de la corrida MUST no tener un `result.json` propio. `agent-eval score --run DIR` MUST puntuar las dos ramas.

#### Escenario: una rama no ve a la otra
- GIVEN una corrida bifurcada en seco cuya rama continua escribe `only-joined.txt` y guarda una memoria con `7719`
- WHEN termina
- THEN `joined/work/` tiene `only-joined.txt` y `split/work/` no, y la búsqueda de `7719` de la rama nueva no lo trae

**Tests:** `a_fork_plays_the_second_session_continued_and_then_new` (`run.rs`); `a_forked_run_plays_the_second_session_both_ways_from_one_first_session`, `a_branch_of_a_fork_does_not_see_what_the_other_did` (`tests/agent_eval.rs`).

## AS7. La corrida en seco de varias sesiones, con el servidor de la corrida

`--replay` MUST aceptar un stream, para una tarea de una sesión, o un directorio con un stream por sesión: `session-1.jsonl` a `session-N.jsonl`, y para una bifurcación `session-1.jsonl`, `joined-2.jsonl` y `split-2.jsonl`. Un directorio al que le falta o le sobra un stream MUST rechazarse antes de preparar nada.

En un brazo con clawg, el cliente reproducido MUST mandar cada llamada del stream a una herramienta del servidor de la corrida a ese servidor, lanzado como lo nombra el `mcp.json` de la corrida, e imprimir en el stream lo que el servidor contestó en lugar del resultado grabado. Las llamadas a otras herramientas y los brazos sin clawg MUST quedar como estaban. Una llamada que el servidor no pudo atender MUST hacer fallar al cliente nombrando la herramienta.

#### Escenario: la sesión 2 encuentra lo que guardó la sesión 1
- GIVEN dos streams: en el primero una llamada a `save_memory` con `48213`, en el segundo una a `search_memory` cuyo resultado grabado no lo tiene
- WHEN se reproducen bajo `a3`
- THEN el resultado de la búsqueda en `stream-2.jsonl` tiene `48213`, y bajo `a0` no

**Tests:** `a_replayed_call_to_the_server_of_the_run_carries_what_the_server_answered`, `a_call_to_any_other_tool_stays_as_recorded` (`replay.rs`); `a_replay_is_a_stream_or_a_directory_with_one_per_session` (`main.rs`); `the_second_replayed_session_finds_what_the_first_one_saved` (`tests/agent_eval.rs`).

## AS8. Los tokens de cada pedido y de cada llamada

`stream::parse` MUST leer cada pedido del bucle principal una vez, por el `id` de su mensaje, con su contexto (la entrada, la caché leída y la caché escrita de su `usage`) y las llamadas que hizo. Los tokens del primer pedido de MD7 MUST seguir siendo el contexto del primero. Una llamada que es la única de su mensaje, y a la que sigue otro pedido, MUST llevar los tokens que agregó: el contexto del pedido siguiente menos el del suyo, que cuenta su entrada, su resultado y el texto que el agente escribió con ella. Una llamada que comparte mensaje con otras, la del último pedido, o una cuyo pedido siguiente tiene menos contexto, MUST quedar sin ese número.

#### Escenario: un mensaje en dos eventos
- GIVEN un stream en que un mensaje del asistente llega en dos eventos con el mismo `id` y el mismo `usage`
- WHEN se lee
- THEN es un pedido, no dos

**Tests:** `each_request_of_the_main_loop_is_read_once_with_its_context`, `a_call_alone_in_its_message_adds_what_the_next_request_grew`, `calls_that_share_a_message_have_no_tokens_of_their_own` (`stream.rs`); `the_first_request_is_the_first_message_of_the_main_loop` (`stream.rs`) sigue pasando.

## AS9. El puntaje de cada sesión y el camino de un hecho

`result.json` MUST llevar, en `sessions`, el registro y el puntaje de cada sesión que corrió, y arriba, como hasta ahora, el registro de la última y el puntaje de la corrida: `success` sólo si todas las sesiones terminaron sin error y pasaron todos los checks, los de la tarea y los de cada sesión. MUST llevar la forma, cuántas reglas tenía el ruido y, en cada sesión, cuántos `Write` y `Edit` fueron al directorio de la memoria nativa. Un `result.json` anterior, sin esos campos, MUST seguir leyéndose como una corrida de una sesión.

En cada sesión, cada trazador de la tarea y cada trazador nacido en una sesión anterior MUST salir con su destino (AE5) y con lo que lo trajo: el nombre de cada herramienta cuyo resultado lo mostró, `hook`, o `context` cuando llegó sin pasar por el stream. Llegó sin pasar por el stream lo de MD6 y además: en la forma continua, lo que dicen los prompts de las sesiones anteriores; y en un brazo con memoria nativa, lo que el cliente carga del `MEMORY.md` que había antes de esa sesión, las primeras 200 líneas o los primeros 25.000 bytes, lo que llegue antes. Sin la copia de antes de la sesión vale el archivo del plan, cortado igual.

Cada trazador nacido en una sesión MUST clasificarse, sobre la última sesión: `not_saved` si ninguna sesión desde la suya lo escribió en la entrada de `save_memory` o `update_memory` ni en un `Write` o `Edit` al directorio de la memoria nativa; si no, `saved_not_reached`, `reached_not_used` o `used`, según su destino.

Una corrida de dos sesiones MUST llevar el sobrecosto del relevo: lo que el primer pedido de la sesión 2 tiene de más sobre el de la sesión 1, sin bajar de 0, más los bytes entre 4 de los resultados de la sesión 2 que releen, que son los `Read` de un archivo que la sesión 1 ya leyó, las llamadas a `search_memory`, `recall_context`, `read_memory` y `list_memories` y los `Read` del directorio de la memoria nativa.

#### Escenario: un hecho que no se guardó
- GIVEN una corrida `split` bajo `a3` cuya sesión 1 no llama a `save_memory` y cuya sesión 2 no muestra el trazador de la sesión 1
- WHEN se puntúa
- THEN el trazador sale `not_saved`, y en la misma corrida `joined` sale `reached_not_used`, traído por `context`

#### Escenario: el archivo de 900 líneas
- GIVEN un `MEMORY.md` de 900 líneas con el trazador en la línea 700
- WHEN se puntúa una sesión de `a4` cuyo stream no lo muestra
- THEN el trazador sale `never_reached`, y con el trazador en la línea 100 `retrieved_but_ignored`

**Tests:** `a_check_of_a_session_reads_that_session_and_one_of_the_task_the_last`, `a_tracer_born_in_a_session_is_followed_in_the_later_ones`, `a_tracer_names_what_brought_it` (`score.rs`); `the_client_loads_the_first_200_lines_or_25_kb_of_the_native_memory` (`seed.rs`); `a_joined_session_has_the_earlier_prompts_in_its_context`, `the_native_memory_delivered_is_what_the_file_held_before_the_session`, `a_fact_is_told_as_not_saved_not_reached_not_used_or_used`, `the_relay_overhead_is_what_the_new_session_starts_with_and_reads_again`, `a_result_written_before_sessions_still_reads`, `score_dir_scores_every_session_of_the_run` (`run.rs`).

## AS10. El reporte, los costos y las llamadas

`agent-eval report` MUST dar una fila por tarea, brazo y forma, y sumar a las columnas de AE8 y MD7: la forma, las sesiones por corrida, el costo y los turnos de la corrida entera (la suma de sus sesiones), la mediana de los tokens de contexto de todos sus pedidos, la mediana del sobrecosto del relevo y la suma de los hechos por su clase de AS9. Para las corridas bifurcadas MUST agregar una tabla por tarea y brazo con los pares: en cuántos aprobó cada forma, en cuántos aprobó la continua y falló la nueva, la mediana del contexto al bifurcar y la mediana del ahorro de costo de la nueva sobre la continua, sin contar el costo de un par cuya continuación leyó de caché menos del 80 % de su primer pedido.

`agent-eval costs` MUST agregar, al final de cada línea, el número de la sesión y la forma. `agent-eval calls --runs DIR` MUST dar una tabla por herramienta: llamadas, las que llevan tokens propios, y la mediana y el p90 de esos tokens.

**Tests:** `rows_are_split_by_the_shape_of_the_run_and_add_up_its_sessions`, `a_forked_run_reports_its_pairs_and_what_the_new_session_saved`, `a_pair_that_continued_cold_does_not_count_for_the_cost`, `the_costs_have_a_line_per_session_with_its_number_and_shape`, `the_calls_table_tells_what_each_tool_adds` (`report.rs`); `the_calls_command_parses` (`main.rs`). Los tests de AE8 y MD7 siguen pasando con resultados de una sesión.

## AS11. El ruido de una corrida

`--noise NAME` MUST sembrar el ruido del archivo `NAME.json` del directorio de datos, con `noise` por omisión; un nombre que no es `noise` ni empieza con `noise-`, o un archivo que no existe, MUST rechazarse. `agent-eval tasks` MUST validar cada `noise*.json` como MD2 y listar cuántas reglas tiene cada uno. `run.json` MUST registrar el nombre y la cantidad.

`data/agent-tasks/noise-300.json` MUST tener 300 reglas, con títulos distintos y más de 7 días.

`l1` y `l2` MUST negarse a planear una corrida cuyo texto de arranque pasa de 10.000 caracteres, lo que el cliente inyecta de la salida de un hook, nombrando el tamaño.

#### Escenario: `l1` con 300 reglas
- GIVEN el ruido `noise-300`
- WHEN se prepara una corrida bajo `l1`
- THEN se niega nombrando los caracteres del arranque y el tope

**Tests:** `a_run_is_seeded_with_the_noise_it_names` (`task.rs`); `the_startup_rules_must_fit_what_the_client_injects` (`arm.rs`); `the_noise_of_a_run_parses` (`main.rs`); `the_large_noise_is_300_distinct_rules_older_than_a_week`, `the_example_tasks_are_valid_and_leak_no_tracer` (`tests/agent_eval.rs`), que lista los dos ruidos.

## AS12. Las tareas nuevas y sus fuentes

Los datos MUST tener las cuatro tareas de la [propuesta](proposal.md#las-tareas-nuevas), con sus categorías, sesiones, trazadores y checks, los fixtures `psf-requests` (sin cambios sobre su fuente) y `shelf-notes`, y dos tareas de ejemplo de varias sesiones con sus streams. `data/agent-tasks/SOURCES.md` MUST nombrar la fuente, el commit y la licencia de `psf-requests`, las rutas que se tomaron y las que no, y qué escribió cada tarea. `agent-eval tasks` MUST dar todo válido y listar las sesiones de cada tarea.

**Tests:** `the_example_tasks_are_valid_and_leak_no_tracer`, `the_tasks_of_several_sessions_are_listed_with_their_sessions`, `the_replayed_relay_example_follows_its_fact_to_the_second_session`, `the_replayed_missing_command_example_counts_its_failures`, `the_task_data_keeps_lf_line_endings_on_every_system` (`tests/agent_eval.rs`).
