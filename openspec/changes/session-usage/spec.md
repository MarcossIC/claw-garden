# session-usage — Especificación

Cambia [`specs/metrics`](../../specs/metrics/spec.md): R3 (el esquema de `metrics.db`), R4 (la retención), el primer invariante y el límite «Sin etiqueta de agente». Suma rutas a [`specs/daemon-access`](../../specs/daemon-access/spec.md), un evento y un hook a lo que instala `setup`, y un subcomando a `clawg metrics`. Al archivar, estos requisitos pasan a esas specs.

## Definiciones

- **Sesión:** una sesión del cliente (Claude Code o Cursor), nombrada por el id que el cliente da en sus hooks.
- **Fila:** una fila de la tabla `session`. Una sesión tiene una fila, o más de una si siguió después de cerrarse (SU3).
- **Evento de hook:** lo que un hook manda al daemon con el id exacto de la sesión: un arranque (`start`), un prompt (`prompt`) o un cierre.
- **Actividad atribuida:** lo que llega sin id de sesión y el daemon le asigna a la sesión vigente de un agente y un proyecto (`SessionRegistry::current`): una llamada a una herramienta, un save aceptado o una búsqueda sin hits.
- **Sesión vigente:** la última que un hook registró para ese agente y ese proyecto en esta ejecución del daemon. Es la misma con la que se sella un save.
- **Conjunto cerrado:** una lista de valores escrita en el código. Un valor que no está en la lista se guarda como `other`.

## SU1. El esquema 3 de `metrics.db`

`metrics.db` MUST pasar a `PRAGMA user_version = 3` con dos tablas más:

- `session`: `session_id`, `started_at_ms`, `agent_id`, `project_id` (puede ser nulo), `client`, `utc_offset_s`, `last_seen_at_ms`, `last_save_at_ms`, `ended_at_ms`, `end_reason`, `prompts` y `empty_searches`, con clave (`session_id`, `started_at_ms`);
- `session_count`: `session_id`, `started_at_ms`, `kind`, `name` y `count`, con clave (`session_id`, `started_at_ms`, `kind`, `name`). `kind` es `start`, `call`, `failed_call` o `save`.

Un archivo del esquema 2 MUST recibir las dos tablas cuando se abre para escribir, sin perder ni cambiar ninguna fila de las tablas que ya tenía. Abierto en sólo lectura, un archivo del esquema 1 o 2 MUST leerse como uno sin sesiones. Un archivo de un esquema más nuevo MUST rechazarse sin tocarlo, como hasta ahora.

`memory.db` MUST NOT cambiar: ni su esquema, ni sus migraciones, ni lo que se guarda en él.

#### Escenario: un archivo del esquema 2 con una ejecución, un minuto y un evento
- GIVEN un `metrics.db` con `user_version = 2`, una fila en `run`, una en `gauge_minute` y una en `event`
- WHEN se lo lee en sólo lectura, y después el daemon lo abre para escribir
- THEN la lectura no da ninguna sesión y no cambia la versión; al abrirlo para escribir queda en la versión 3, las tres filas siguen iguales y una sesión escrita se lee de vuelta

**Tests:** `a_file_of_schema_2_gets_its_sessions_and_keeps_its_rows`, `a_newer_schema_is_refused_untouched` (ya existe) en `crates/clawg-core/src/metrics/store.rs`.

## SU2. Una fila por sesión

El primer evento de hook de un id que no tiene una fila abierta MUST abrir una, con `started_at_ms` y `last_seen_at_ms` en la hora del evento, el agente, el proyecto (o nulo) y el cliente del evento, y el desfase de la hora local. Cada evento de hook y cada actividad atribuida MUST llevar `last_seen_at_ms` a su hora y contar:

| Qué | Dónde |
|---|---|
| un arranque | `session_count` con `kind` `start` y `name` el `source` |
| un prompt | `prompts` |
| una llamada a una herramienta | `kind` `call` y `name` la herramienta; si falló, también `kind` `failed_call` |
| un save aceptado | `kind` `save` y `name` el tipo de memoria, y `last_save_at_ms` |
| una búsqueda sin ningún hit | `empty_searches` |

Cada `name` y el `client` MUST salir de un conjunto cerrado:

- `source`: `startup`, `resume`, `clear`, `compact`, `fork`; sin `source`, `unspecified`;
- herramienta: las del contrato del agente (SU8);
- tipo de memoria: los de `MemoryType`; un save que no declara tipo, `unspecified`;
- `client`: `claude-code`, `cursor`;
- `end_reason`: `clear`, `resume`, `logout`, `prompt_input_exit`, `completed`, `aborted`, `error`, `window_close`, `user_close`, `idle`.

Un evento de hook que no dice qué pasó (el cuerpo de `POST /v1/sessions` de un clawg anterior) MUST abrir o seguir la fila sin contar nada. Una actividad atribuida a un id del que este daemon no tiene fila abierta ni cerrada MUST descartarse: sólo un evento de hook trae la identidad con la que se abre una fila.

El proyecto de una fila es el del evento que la abrió y no cambia con los eventos siguientes.

**Tests:** `a_session_opens_on_its_first_event_and_counts_what_follows`, `activity_of_an_unknown_session_is_dropped`, `a_name_outside_its_set_is_other` en `crates/clawg-core/src/metrics/sessions.rs`.

## SU3. El cierre

Una fila MUST cerrarse de una de dos formas:

- **Por el hook de cierre:** `ended_at_ms` es la hora del aviso y `end_reason` su motivo.
- **Por la regla de las 6 horas:** cuando pasan `IDLE_CLOSE_MS` (6 horas) o más desde `last_seen_at_ms` sin ningún evento ni actividad, `ended_at_ms` es `last_seen_at_ms` y `end_reason` es `idle`.

La regla MUST aplicarse cuando llega el próximo evento o actividad del mismo id, antes de contarlo, y cuando se arma el reporte (SU11). No necesita un proceso que revise; el escritor además la aplica con cada minuto (SU4).

Una fila cerrada MUST NOT reabrirse. Un evento de hook o una actividad atribuida que llega después con el mismo id MUST abrir una fila nueva, con la identidad de la anterior si es una actividad atribuida. Un aviso de cierre no abre nada: uno que llega a una fila que la regla ya cerró la deja cerrada con `idle`, y uno de un id sin fila abierta se descarta.

Un reinicio del daemon MUST NOT cerrar ninguna fila: el daemon que arranca retoma las filas abiertas (SU4), y un evento del mismo id antes de las 6 horas sigue la misma fila.

#### Escenario: 5 h 59 min y 6 h
- GIVEN una sesión cuyo último evento fue a la hora T
- WHEN llega un prompt a T + 5 h 59 min, y otro 6 h después de ese
- THEN el primero sigue la fila; el segundo la cierra con `idle` y `ended_at_ms` en la hora del primero, y abre una fila nueva que empieza a la hora del segundo

#### Escenario: un cierre por `/clear` y lo que sigue
- GIVEN una sesión S abierta
- WHEN llega su cierre con motivo `clear`, y después un prompt de S
- THEN S queda con una fila cerrada con `clear` y una fila nueva abierta

Una fila que se abre en el mismo milisegundo en que cerró la anterior del mismo id MUST empezar un milisegundo después: (`session_id`, `started_at_ms`) es la clave.

**Tests:** `six_hours_without_activity_close_a_session_as_idle`, `a_closed_session_is_not_reopened`, `an_end_after_six_idle_hours_leaves_the_row_idle`, `a_row_opened_as_the_one_before_it_closed_begins_a_millisecond_later` en `sessions.rs`.

## SU4. Se guarda con cada minuto y se retoma al arrancar

El registro vive en memoria, colgado del `Recorder` del runtime (`Recorder::sessions`). El escritor MUST guardar en `metrics.db`, con cada minuto y al apagarse (`Writer::flush`), las filas que cambiaron desde la escritura anterior, con su estado entero, y aplicar antes la regla de las 6 horas a las abiertas. Un `kill` pierde como mucho un minuto, como las series.

Al arrancar, el escritor MUST cerrar con `idle` las filas abiertas del archivo que ya pasaron las 6 horas y cargar las demás en el registro.

Con `CLAWG_METRICS=off`, o con la escritura apagada por un fallo, el registro MUST NOT guardar nada en memoria ni en disco. Un fallo al escribir las sesiones apaga la escritura como el de un minuto (R3 de `specs/metrics`), sin afectar ninguna petición.

**Tests:** `sessions_are_written_whole_and_read_back`, `open_sessions_past_six_hours_are_closed_on_start` en `store.rs`; `sessions_are_kept_with_each_minute_and_resumed_by_the_next_run`, `a_writer_turned_off_keeps_no_sessions` en `crates/clawg-cli/src/daemon/metrics.rs`.

## SU5. Retención

Una fila cuya `last_seen_at_ms` tiene más de `HOURS_KEPT_MS` (400 días) MUST borrarse con sus contadores, en la misma poda que las horas y los eventos.

**Test:** `sessions_past_the_retention_are_dropped` en `store.rs`.

## SU6. Las rutas del daemon

Todas detrás del token, y todas contestan `204` sin cuerpo.

- `POST /v1/sessions` MUST seguir registrando la sesión vigente del agente y del proyecto como hasta ahora, y además aceptar `event` (`start` o `prompt`), `source` y `client`, los tres opcionales. Es un evento de hook de esa sesión (SU2). Un `event` desconocido MUST contar como uno que no dice qué pasó.
- `POST /v1/sessions/end` (`session_id`, `reason` opcional) MUST cerrar la fila abierta de ese id (SU3). No toma proyecto y no registra ninguno. Un `session_id` inválido es un 400, con la misma regla que `POST /v1/sessions`.
- `POST /v1/sessions/calls` (`agent_id`, `project_id` opcional, `tool`, `failed`) MUST contar una llamada para la sesión vigente de ese agente y ese proyecto. Sin sesión vigente, no cuenta nada.

`POST /v1/memories` MUST contar un save cuando lo acepta en el outbox (no cuando contesta por un `operation_id` cuyo save sigue esperando o falló) y lo selló con una sesión. `POST /v1/search` MUST contar una búsqueda vacía cuando no devuelve ningún hit, para la sesión vigente del agente y del proyecto de la petición; una búsqueda sobre varios proyectos no se atribuye.

Nada de esto MUST cambiar lo que esas rutas contestan ni lo que se guarda en `memory.db`.

**Tests:** `a_hook_event_opens_the_row_of_its_session_and_counts`, `a_daemon_that_keeps_no_metrics_keeps_no_sessions`, `a_session_end_closes_its_row_with_the_reason`, `a_tool_call_counts_for_the_session_of_its_agent_and_project`, `an_accepted_save_and_an_empty_search_count_for_their_session` en `crates/clawg-cli/tests/api.rs`.

## SU7. Unir y borrar proyectos

`POST /v1/projects/merge` MUST pasar al proyecto que queda las filas del proyecto que se une, y `POST /v1/projects/delete` MUST borrar las filas del proyecto borrado, con sus contadores, en memoria y en `metrics.db`. El escritor MUST aplicarlo en cuanto la operación termina, sin esperar al cambio de minuto. Deshacer una unión o un borrado MUST NOT devolver las filas: el registro no guarda de qué proyecto venían ni lo que borró. Una unión o un borrado en seco, o uno que no escribió nada, MUST NOT tocar ninguna fila.

**Tests:** `merging_a_project_moves_its_sessions_and_deleting_it_drops_them` en `sessions.rs`; `project_changes_reach_the_stored_sessions` en `store.rs`; `a_merge_moves_the_sessions_and_a_delete_drops_them` en `tests/api.rs`.

## SU8. El puente avisa cada llamada con su nombre

Cuando una herramienta termina, el puente MUST mandar `POST /v1/sessions/calls` con el agente de la llamada, el proyecto de la carpeta abierta en la ventana (ninguno si hay varias o ninguna), el nombre de la herramienta y si su resultado fue un error, también cuando el puente la rechazó sin pedirle nada al daemon. El aviso MUST NOT cambiar lo que la herramienta contesta, MUST NOT arrancar ni esperar un daemon, MUST NOT registrar ningún proyecto y MUST acotarse a `CALL_REPORT_WAIT` (500 ms).

El proyecto de la carpeta es el id que el daemon ya le dio a este puente por ella, o si todavía no la registró, el que se deriva de su raíz; el daemon lo sigue hasta su proyecto vivo. Para saber qué carpeta hay abierta, cada herramienta MUST pedirle las carpetas al cliente que las declara (`roots/list`), también las cinco que no las usan para otra cosa (`list_memories`, `delete_memory`, `restore_memory`, `update_memory`, `resolve_conflict`).

El nombre sale del conjunto cerrado de las herramientas del contrato del agente (`TOOLS`); el daemon MUST guardar `other` para uno que no conoce. Lo que el modelo ve del servidor MUST NOT cambiar.

**Tests:** `the_counted_tools_are_the_tools_of_the_bridge`, `a_tool_reports_its_call_to_the_session_of_the_open_folder`, `a_failed_tool_is_reported_as_failed`, `a_call_is_not_reported_while_no_daemon_is_there` en `crates/clawg-cli/src/mcp/server.rs`; `a_tool_outside_the_set_is_counted_as_other` en `crates/clawg-cli/src/daemon/sessions.rs`; `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, sin cambios.

## SU9. Los hooks

`clawg hook session-start` y `clawg hook user-prompt-submit` MUST mandar en su registro el evento (`start` o `prompt`), el cliente y, el primero, el `source` de su entrada.

`clawg hook session-end` MUST leer `session_id` y `reason` de su entrada y mandar el cierre al daemon. MUST NOT imprimir nada, MUST NOT registrar el proyecto ni leer su carpeta, y MUST terminar dentro de `SESSION_END_DEADLINE` (1 s). Sin daemon, sin `session_id` o con una entrada que no es JSON, no hace nada y sale con 0. Un hook instalado para Claude Code que corre Cursor no hace nada, como los otros.

**Tests:** `a_hook_names_its_event_and_its_source`, `a_session_end_closes_the_session_and_prints_nothing`, `a_session_end_without_a_daemon_or_an_id_does_nothing` en `crates/clawg-cli/src/hook.rs`; `hook_takes_the_session_end_event` en `cli.rs`.

## SU10. `setup` instala el hook de cierre

`clawg setup claude-code`, con o sin `--local`, MUST instalar `SessionEnd` sin matcher junto a los otros dos, y `clawg setup cursor` MUST instalar `sessionEnd` junto a `sessionStart`. Sobre una instalación anterior, MUST agregar sólo el que falta y dejar los demás byte a byte. `--remove` MUST quitarlo. En Cursor cada evento tiene su paso y su línea en el reporte: un hook de clawg que no coincide con el plan en un evento queda como está y sale con 1, y el otro evento se instala igual, como ya pasa en Claude Code. `hook_installed`, con lo que `doctor` dice que el hook instalado no dejó corridas, sigue mirando sólo `sessionStart`, el único que las deja.

**Tests:** `a_run_over_an_install_without_the_session_end_hook_adds_only_it` en `crates/clawg-cli/src/setup.rs` y en `crates/clawg-cli/src/setup/cursor.rs`, `a_local_run_over_an_install_without_the_session_end_hook_adds_only_it` en `crates/clawg-cli/src/setup/local.rs`, `remove_takes_each_hook_that_is_there` y `the_session_start_hook_of_this_profile_is_found_in_hooks_json` en `cursor.rs`; `setup_dry_run_writes_nothing`, `setup_cursor_installs_and_remove_undoes_it` y `setup_local_installs_in_the_folder_and_remove_undoes_it` en `crates/clawg-cli/tests/binary.rs`.

## SU11. `clawg metrics sessions`

`clawg metrics sessions [--by project|agent|day] [--since Nd|Nh] [--session ID] [--profile P]` MUST abrir `metrics.db` en sólo lectura, sin daemon y sin el lock del store, e imprimir un JSON. No abre `memory.db`.

Sin `--session`: `by`, `since_ms`, `idle_close_ms` y `groups`, de las filas cuya `last_seen_at_ms` es de `--since` (por defecto `30d`) en adelante. `--by project` (por defecto) agrupa por `project_id`, con `key` nulo para las filas sin proyecto; `--by agent`, por `agent_id`; `--by day`, por la fecha local del comienzo de la fila. Cada grupo trae:

- `sessions`, `open` (las que siguen abiertas) y `ended`, cuántas cerraron por cada motivo, `idle` incluido;
- `duration_ms` (`total`, `mean` y `max`), del comienzo al cierre; de una fila abierta, hasta su última actividad;
- `since_last_save_ms` (`sessions`, `mean` y `max`), del último save al cierre, de las filas cerradas que guardaron algo;
- `prompts`, `empty_searches`, `starts` por `source`, `calls` por herramienta (`count` y `failed`) y `saves` por tipo;
- `with_summary` y `with_summary_pct`: las filas que guardaron un `session_summary`.

Una fila abierta cuya última actividad tiene 6 horas o más MUST leerse como cerrada con `idle` en esa hora, aunque el archivo todavía la tenga abierta.

Con `--session ID`: `session_id` y `rows`, cada fila de esa sesión entera, de la más vieja a la más nueva, sin importar `--since`. `--session` con `--by` es un error de uso.

Sin `metrics.db` MUST salir con 1 diciendo que el daemon lo escribe salvo con `CLAWG_METRICS=off`, sin crearlo.

#### Escenario: dos sesiones de un proyecto y una sin proyecto
- GIVEN dos filas del proyecto P, una cerrada con `clear` que guardó un `session_summary` y otra abierta hace 7 horas, y una fila sin proyecto
- WHEN se reporta por proyecto
- THEN el grupo P trae `sessions` 2, `ended` con `clear` 1 e `idle` 1, `open` 0 y `with_summary_pct` 50, y hay un grupo con `key` nulo

**Tests:** `a_report_groups_sessions_by_project_agent_and_day`, `an_open_session_past_six_hours_reads_as_idle`, `the_time_since_the_last_save_counts_closed_sessions_that_saved` en `crates/clawg-core/src/metrics/session_report.rs`; `the_sessions_report_reads_the_file_without_a_daemon`, `one_session_is_listed_with_each_of_its_rows` en `crates/clawg-cli/src/metrics.rs`; `metrics_sessions_takes_a_grouping_an_age_and_a_session` en `cli.rs`.

## SU12. De punta a punta

**Tests** en `crates/clawg-cli/tests/binary.rs`, con `CLAWG_TEST_HOME`, el embedder falso y `--port 0`:

- `a_daemon_keeps_each_session_across_a_restart_and_reports_them`: un `session-start` con id S; dos llamadas por el puente y un `user-prompt-submit`; un `session-start` con `source=clear` e id S2; un reinicio del daemon y un prompt de S2. Da S con 2 llamadas y 1 prompt, S2 con una sola fila que empezó antes del reinicio y tiene su prompt, y el reporte por proyecto con las dos.
- `a_session_end_closes_the_session_and_what_follows_opens_another`: un `session-end` de S con motivo `clear` la cierra con ese motivo, y un prompt posterior de S abre una fila nueva.

## Invariantes

El primer invariante de `specs/metrics` («ninguna etiqueta ni fila lleva texto de una memoria, una consulta, una ruta de fichero o un id») se enmienda así:

- Nada de esto sale de la máquina.
- **Las series y los eventos siguen cerrados:** ninguna etiqueta de una serie y ningún detalle de un evento lleva texto de una memoria, una consulta, una ruta de fichero o un id. Cada valor sale de un conjunto cerrado escrito en el código.
- **`session` y `session_count` son el único lugar de `metrics.db` con ids,** y sólo con tres: el de la sesión que da el cliente, el del agente y el del proyecto. Todo otro valor de texto de esas tablas (`client`, `end_reason`, `kind`, `name`) sale de un conjunto cerrado.
- **Sigue prohibido en todo `metrics.db`, estas tablas incluidas:** el texto de una memoria, de una consulta o de un prompt; una ruta de fichero o de carpeta, el `cwd` y el `transcript_path` del hook incluidos; y el usuario: su nombre, su email (el `user_email` de Cursor) o cualquier otro dato que lo identifique.

Los demás invariantes de `specs/metrics` no cambian: `memory.db`, su esquema y lo que ve el modelo no cambian por las métricas; ninguna medición cambia lo que el daemon hace ni lo que contesta; borrar `metrics.db` sólo pierde la historia.

**Test:** `no_text_path_or_user_reaches_a_session_row` en `crates/clawg-cli/tests/api.rs`: un prompt, una consulta, un save y un hook con `cwd`, `transcript_path` y `user_email` no dejan en las filas nada más que los tres ids y valores de los conjuntos cerrados.

## Límites conocidos

El límite «Sin etiqueta de agente» de `specs/metrics` sigue valiendo para las series. El agente aparece sólo en `session`, una fila por sesión y no una serie por agente.

- **Dos sesiones en paralelo del mismo agente y proyecto** no se distinguen en la actividad atribuida: va a la última que registró un hook, como los saves.
- **Cursor no tiene un hook por prompt:** sus filas no cuentan prompts.
- **Con varias carpetas abiertas, o ninguna,** el puente no nombra un proyecto y sus llamadas no se atribuyen. El hook de Cursor tampoco registra sesión en ese caso.
- **Una carpeta cuyo proyecto no es el que se deriva de su raíz** (un clon reconocido por su remoto, una subcarpeta declarada) no se atribuye hasta que una herramienta de ese puente la registra: las herramientas que van por id de memoria no registran nada.
- **Lo que no llega al daemon no se cuenta:** una llamada con el daemon caído, o un hook sin daemon.
- **Después de un reinicio del daemon,** la actividad atribuida no se cuenta hasta que un hook registra la sesión otra vez, igual que un save sale sin sesión.
- **El cierre que el cliente no avisa** (el proceso que muere, el equipo que se apaga) queda con `idle` en la última actividad.
- **Un proyecto borrado con `CLAWG_METRICS=off`** conserva en `metrics.db` las filas que ya tenía: con la escritura apagada el daemon no toca el archivo.
- **El archivo va hasta un minuto atrás** del daemon que corre: el reporte lee lo ya escrito.
