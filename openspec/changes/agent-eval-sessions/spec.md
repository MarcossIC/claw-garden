# agent-eval-sessions — Especificación

Amplía el arnés de [agent-eval](../agent-eval/spec.md) (AE1 a AE9) y la sección «`agent-eval`: las reglas, el ruido y los brazos de entrega» de [`specs/eval`](../../specs/eval/spec.md) (MD1 a MD7), de la que modifica MD3. Ninguna spec del producto cambia. Origen: [proposal](proposal.md); decisiones en [design](design.md). Los tests corren con la suite, sin modelo real, red ni `claude`, y no dejan nada en el temporal; los que juegan una corrida usan el `clawg` `testing` que la suite construye, con el modelo falso. Los archivos de código están en `crates/clawg-eval/src/bin/agent-eval/` y los de integración en `crates/clawg-eval/tests/agent_eval.rs`.

AS13 a AS22 son las enmiendas que pidió la [segunda verificación](verify.md#segunda-verificación) antes de la campaña paga. Modifican: la clasificación de un hecho de AS9 (AS13), el sobrecosto del relevo de AS9 (AS14), las sesiones de AS2 (AS15), los checks de AS3 (AS18), el corredor de AS5 (AS19), la bifurcación de AS6 (AS20) y el reporte de AS10 (AS22); AS16, AS17 y AS21 son nuevos. Donde una de ellas y una anterior digan otra cosa, vale la enmienda.

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

Cada trazador nacido en una sesión MUST clasificarse, sobre la última sesión (con la ventana de guardado que fija AS13): `not_saved` si ninguna sesión desde la suya lo escribió en la entrada de `save_memory` o `update_memory` ni en un `Write` o `Edit` al directorio de la memoria nativa; si no, `saved_not_reached`, `reached_not_used` o `used`, según su destino.

Una corrida de dos sesiones MUST llevar el sobrecosto del relevo (reemplazado por AS14): lo que el primer pedido de la sesión 2 tiene de más sobre el de la sesión 1, sin bajar de 0, más los bytes entre 4 de los resultados de la sesión 2 que releen, que son los `Read` de un archivo que la sesión 1 ya leyó, las llamadas a `search_memory`, `recall_context`, `read_memory` y `list_memories` y los `Read` del directorio de la memoria nativa.

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

## AS13. Un hecho lo guarda una sesión anterior a la que lo usa (modifica AS9)

Un trazador nacido en la sesión i MUST contar como guardado sólo si lo escribió una sesión de la i a la penúltima, en la entrada de `save_memory` o `update_memory` o en un `Write` o `Edit` al directorio de la memoria nativa. Lo que guarda la última sesión MUST no contar: no trajo nada a ella. La sesión en que nació sí cuenta.

#### Escenario: guarda al terminar
- GIVEN una corrida `split` de dos sesiones bajo `a3` cuya sesión 1 no guarda `FORK_DEFAULT_TIMEOUT`, y cuya sesión 2 lo escribe en un archivo y lo guarda con `save_memory` al final
- WHEN se puntúa
- THEN el hecho sale `not_saved`, y con el `save_memory` en la sesión 1 sale `used`

**Tests:** `a_fact_saved_only_by_the_last_session_is_not_saved` (`run.rs`), que hoy falla porque `run::facts` mira `sessions[i..]` (`run.rs:353`); `a_fact_is_told_as_not_saved_not_reached_not_used_or_used` sigue pasando.

## AS14. El sobrecosto evitable y las relecturas (reemplaza el sobrecosto de AS9)

Una corrida de dos sesiones MUST llevar `rereads`, lo que la sesión 2 relee, en tokens a 4 bytes por token, en dos partes:

- `again`: los resultados de los `Read` de un archivo que la sesión 1 leyó o escribió, salvo los de un archivo que la sesión 2 edita (con `Edit`, `MultiEdit`, `Write` o `NotebookEdit`, antes o después de leerlo) y que la sesión 1 no escribió; los de `search_memory`, `recall_context`, `read_memory` y `list_memories`; y los `Read` del directorio de la memoria nativa.
- `to_edit`: los resultados de los `Read` que `again` deja fuera por la edición.

Las rutas MUST compararse resueltas desde el directorio de trabajo, sin importar separadores ni mayúsculas.

`overhead`, el sobrecosto evitable, MUST ser, cuando la sesión 2 es nueva (una corrida `split` o la rama `split` de una bifurcación), lo que el primer pedido de la sesión 2 tiene de más sobre el de la sesión 1, sin bajar de 0, más `again`. Cuando la sesión 2 continúa la conversación, `overhead` MUST faltar y `rereads` MUST estar: su primer pedido es la conversación entera. Un `result.json` anterior, sin `rereads`, MUST seguir leyéndose.

#### Escenario: lo que una edición exige no cuenta
- GIVEN una sesión 1 que lee `a.py` y `b.py` y escribe `NOTES.md`, y una sesión 2 que lee `a.py` y lo edita, lee `b.py`, y lee `NOTES.md` y lo edita
- WHEN se puntúa como `split`
- THEN `again` son los resultados de `b.py` y `NOTES.md` entre 4, `to_edit` el de `a.py` entre 4, y `overhead` el crecimiento del primer pedido más `again`; como `joined`, `rereads` es el mismo y `overhead` falta

**Tests:** `the_relay_overhead_leaves_out_the_reads_an_edit_needs`, `a_file_the_first_session_wrote_always_counts_as_read_again`, `a_continued_session_has_rereads_and_no_relay_overhead` (`run.rs`); `the_relay_overhead_is_what_the_new_session_starts_with_and_reads_again` cambia su aserción a la regla nueva; `a_result_written_before_sessions_still_reads` sigue pasando.

## AS15. Lo que una sesión deja sin tocar (modifica AS2)

Una sesión MAY llevar `untouched`, una lista de `{"path", "text"}`: al terminar esa sesión, el archivo `path` del directorio de trabajo MUST no contener `text`. `path` MUST ser relativo y quedar dentro del directorio de trabajo, y `text` MUST no estar vacío; si no, la tarea se rechaza nombrando la sesión y la entrada. `untouched` no es un check y no decide el éxito.

El corredor MUST leer esos archivos justo después de la sesión, antes de que empiece la siguiente y antes de guardar el estado de una bifurcación, y escribir `untouched-K.json` con cada entrada y si su texto estaba; un archivo que falta no lo contiene, y el texto se busca como en los checks de archivo, con CRLF leído como LF. El registro de la sesión MUST llevar esas entradas, y una cuyo texto estaba es el artefacto `applied_early` (AS21).

La sesión 1 de `requests-timeout-decision-relay` MUST dejar `src/requests/adapters.py` y `HISTORY.md` sin `FORK_DEFAULT_TIMEOUT` y `docs/user/advanced.rst` sin `4.5, 41`; las sesiones 1 y 2 de `requests-redact-decision-revised` MUST dejar `src/requests/utils.py` sin `def redact_url_credentials`. La sesión 1 de `example-relay-port` MUST dejar `notes.md` sin `confirmed`.

#### Escenario: la sesión 1 aplicó antes de tiempo
- GIVEN `example-relay-port` en seco, con un stream de la sesión 1 que escribe `confirmed` en `notes.md`
- WHEN se reproduce bajo `a3`
- THEN `untouched-1.json` dice que el texto estaba y `result.json` lleva `applied_early` en la sesión 1; con el stream del ejemplo no lleva ninguno

**Tests:** `a_session_names_what_it_must_leave_untouched` (`task.rs`); `a_session_that_wrote_what_it_had_to_leave_untouched_is_found` y `the_tasks_that_hold_a_decision_back_name_what_stays_untouched` (`tests/agent_eval.rs`).

## AS16. Las lecturas fuera del directorio de trabajo se niegan

El `settings.json` de la configuración del cliente de todo brazo MUST llevar `permissions.blockReadsOutsideWorkingDirectories` en `true`, junto a lo que `setup` escribe en los brazos que lo tienen, que MUST conservarse. Bajo `a4` MUST llevar además, en `permissions.additionalDirectories`, el directorio de su memoria nativa; ningún otro brazo MUST llevar directorios adicionales.

Una corrida paga MUST negarse, antes de preparar nada, si `--claude-version` es anterior a 2.1.257, la primera versión del cliente con ese bloqueo, nombrando la mínima.

#### Escenario: `a4` lee sus notas y nada más
- GIVEN una tarea con memorias
- WHEN se planea bajo `a4` y bajo `a3`
- THEN los dos `settings.json` bloquean las lecturas fuera, el de `a4` tiene como único directorio adicional el de su memoria nativa, y el de `a3` conserva los hooks de `setup` y no tiene ninguno

**Tests:** `every_arm_blocks_the_reads_outside_its_working_directory`, `the_native_memory_is_a_working_directory_of_a4_only` (`arm.rs`); `a_paid_run_refuses_a_client_without_the_read_block` (`run.rs`).

## AS17. Las lecturas fuera del directorio de trabajo se detectan

El registro de cada sesión MUST llevar `outside_reads`: las llamadas, las de los subagentes incluidas, que leen fuera del directorio de trabajo y de los directorios adicionales del plan. Leen una ruta: `Read` por `file_path`; `Grep` y `Glob` por `path`, y por `pattern` cuando es una ruta absoluta; `LSP` por su archivo; y `Bash` por cada palabra de su comando que es una ruta absoluta (empieza con `/`, `~` o una letra de unidad) o tiene `..`. Una ruta relativa se resuelve desde el directorio de trabajo, y todas se comparan resueltas, sin importar separadores ni mayúsculas. Cada una MUST llevar la herramienta, la ruta y si el cliente la negó (su resultado vino marcado como error). Una ruta que contiene el `session_id` del `init` de la sesión MUST no listarse: son archivos que el cliente escribe para esa sesión y le pide leer.

Una lectura fuera que el cliente no negó es el artefacto `read_outside` (AS21); una negada se lista y no lo es.

#### Escenario: tres lecturas
- GIVEN una sesión con un `Read` de `../run.json` que devolvió su contenido, un `Grep` sobre la ruta absoluta de `fork/` que volvió marcado como error y un `Read` de `src/a.py`
- WHEN se puntúa
- THEN `outside_reads` lista las dos primeras, la segunda negada, y `result.json` lleva `read_outside` sólo por la primera

**Tests:** `a_read_outside_the_working_directory_is_listed_whatever_the_tool`, `a_denied_read_outside_is_listed_and_is_no_artifact`, `the_files_the_client_writes_for_the_session_are_not_outside` (`run.rs`).

## AS18. Los intentos de un comando (modifica AS3)

`command_runs` MUST ser un check con `command` y `max`: pasa si las llamadas a `Bash`, las de los subagentes incluidas, cuyo comando invoca `command` son `max` o menos, sea cual sea su resultado. Un comando invoca `command` si alguna de sus partes (separadas por `|`, `||`, `&&`, `;`, `&` o un salto de línea, o abiertas por `$(`, una comilla invertida o `(`) tiene como primera palabra, después de las asignaciones `NOMBRE=valor`, `command` o una ruta que termina en `/command`. Es de los que leen el stream, y `command` MUST estar en `shell` de la tarea.

La sesión 3 de `shelf-eza-missing` MUST llevar `command_runs` de `eza` con `max` 0, además de sus dos checks.

#### Escenario: dónde se invoca un comando
- GIVEN las llamadas `eza -1 src | wc -l`, `ls src && eza docs`, `/usr/bin/eza src`, `echo eza` y `ls src`
- WHEN se cuentan los intentos de `eza`
- THEN son 3

**Tests:** `the_runs_of_a_command_are_counted_in_every_part_of_a_shell_line` (`score.rs`); `a_command_run_check_names_a_command_of_the_shell` (`task.rs`); `the_replayed_missing_command_example_counts_its_failures` (`tests/agent_eval.rs`), con una aserción nueva sobre `command_runs` en la última sesión del ejemplo.

## AS19. Toda corrida que falla se puntúa (modifica AS5)

Una corrida que falla MUST dejar su `result.json` siempre que su primera sesión haya escrito un stream, también una de una sesión, con `success` en falso. En una bifurcación, si falla la segunda sesión de una rama, el corredor MUST mover a su subdirectorio los archivos de esa rama, como al terminarla bien, antes de devolver el error, y `score_dir` MUST puntuarla como una corrida de dos sesiones que no terminó; si falla la sesión 1, MUST dejar en la raíz un `result.json` de esa sesión, sin marca de par. `agent-eval costs` MUST contar cada sesión que dejó un stream con su costo, terminada o no.

#### Escenario: falla la rama nueva
- GIVEN una bifurcación en seco cuyo stream `split-2.jsonl` termina en error
- WHEN corre
- THEN `split/` tiene su `stream-2.jsonl`, su `work/` y un `result.json` de dos sesiones no terminado, y `costs` tiene una línea por cada una de las tres sesiones

**Tests:** `a_failed_run_of_one_session_leaves_its_result`, `a_branch_whose_second_session_fails_keeps_its_files_and_is_scored` (`tests/agent_eval.rs`); `the_costs_count_the_sessions_of_a_run_that_failed` (`report.rs`).

## AS20. La bifurcación restaura el home entero (modifica AS6)

La bifurcación MUST guardar y restaurar el home entero de la corrida, salvo el directorio `models/` del directorio de datos de clawg, que queda donde está sin copiarse. Lo demás de AS6 no cambia.

#### Escenario: lo que la rama continua dejó en el home
- GIVEN una bifurcación cuya rama continua escribe un archivo en `home/` fuera del directorio de datos y, bajo `a4`, una nota en su `MEMORY.md`
- WHEN corre la rama nueva
- THEN no encuentra ni el archivo ni la nota, y `models/` no se copió

**Tests:** `a_fork_restores_the_whole_home_but_the_models`, `a_fork_puts_back_the_native_memory_the_continued_branch_changed` (`run.rs`), sobre `save_state` y `restore_state` con el `Layout` de una corrida de `a4`. No es de integración: el cliente reproducido sólo escribe dentro del directorio de trabajo, y la memoria nativa está fuera.

## AS21. Los artefactos de una corrida

`result.json` MUST llevar `artifacts`, las causas de la regla 3 de la [propuesta](proposal.md#la-regla-de-decisión) que el resultado muestra, cada una con la sesión en que ocurrió:

| Causa | Cuándo |
|---|---|
| `session_failed` | una sesión sin resultado o cuyo resultado es un error (a) |
| `server_not_connected` | en un brazo con clawg, el servidor del perfil no figura `connected` en el `init` de una sesión (b) |
| `not_continued` | una sesión que continúa la conversación tiene en su primer pedido menos contexto que en el último pedido de la anterior (c) |
| `read_outside` | una lectura fuera que el cliente no negó (AS17) (e) |
| `compacted_early` | una compactación en una sesión que no es la última de la corrida; en una rama, la sesión 1 es anterior (f) |
| `applied_early` | una entrada de `untouched` cuyo texto estaba (AS15) (g) |

La causa (d) no tiene lugar en el resultado: una corrida paga con un comando que debía faltar se niega antes de empezar. Decidir las causas MUST ser puro, sobre los registros de las sesiones y la forma de la corrida. Un `result.json` anterior, sin `artifacts`, MUST leerse sin ninguno.

#### Escenario: una compactación al final no es artefacto
- GIVEN una corrida `joined` de dos sesiones con una compactación en la sesión 2
- WHEN se puntúa
- THEN `artifacts` está vacío, y con la compactación en la sesión 1 lleva `compacted_early` en la sesión 1

**Tests:** `the_artifacts_of_a_run_are_named_with_their_session`, `a_compaction_in_the_last_session_is_no_artifact` (`run.rs`).

## AS22. El reporte con los artefactos, el ruido y los pares (modifica AS10)

`agent-eval report` MUST:

- dar una fila por tarea, brazo, forma y ruido (la cantidad de reglas de la corrida), con el ruido como columna;
- dejar fuera de cada fila las corridas con algún artefacto, decir en una columna cuántas dejó fuera, y listar debajo de la tabla cada corrida excluida con su ruta y sus causas;
- en una fila de forma `joined`, dejar vacía la columna del sobrecosto del relevo y dar la mediana de `again` en la de relecturas;
- en la tabla de pares, dar sobre todos los pares cuántos tienen un artefacto en alguna rama, cuántos tienen `not_continued` y cuántos una compactación en cualquier sesión de cualquier rama; y, sólo sobre los pares sin artefacto, lo que AS10 ya pedía más la mediana del sobrecosto evitable de la rama nueva, la de su `to_edit` y la de `again` de la rama continua.

`agent-eval calls` MUST dar, junto a la mediana y el p90 de los tokens que agrega cada llamada, la mediana de los bytes de su resultado entre 4.

**Tests:** `rows_are_split_by_the_noise_of_the_run`, `rows_leave_out_the_runs_with_an_artifact_and_list_them`, `a_joined_row_has_rereads_and_no_relay_overhead`, `the_pairs_table_tells_what_was_excluded_continued_compacted_and_read_again`, `the_calls_table_gives_the_size_of_a_result_beside_what_a_call_adds` (`report.rs`). Los tests de AS10 siguen pasando con las columnas nuevas.
