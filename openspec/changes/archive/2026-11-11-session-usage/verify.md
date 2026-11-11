# session-usage — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, en `target`, sobre `606ad66`: el último commit de código y de `docs/reference`. El commit que sigue sólo toca esta carpeta.

- `cargo test --workspace --no-fail-fast` pasa.
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin bendecir: `agent_contract.json` y `agent_contract.es.json` no cambiaron. El nombre de la herramienta va del puente al daemon.
- El código `cfg(windows)` no se compiló ni se corrió acá. El cambio no agrega ninguno, pero pasa por código que lo tiene: el cliente del daemon por socket (`send_over_af_unix`, por donde van `POST /v1/sessions/end` y `POST /v1/sessions/calls`) y la línea del hook de Cursor para PowerShell. Lo revisa el job de Windows del CI del pull request.
- El CI del pull request #142 corrió clippy y la suite en Linux y en Windows. En Windows falló una vez: `an_empty_state_plans_the_server_and_the_hooks` esperaba la línea POSIX de `session-end` de Cursor con la ruta del binario sin comillas, y en Windows esa ruta lleva barras invertidas, que la forma POSIX pone entre comillas. `b17ddf5` corrigió el test (`sh_word` sobre la ruta), y la corrida sobre ese commit pasó en los dos sistemas. El código no cambió.

## Los tests en rojo

| Requisito | Test | Cómo fallaba |
|---|---|---|
| SU12, parte A | `a_daemon_keeps_each_session_across_a_restart_and_reports_them` | Se escribió y se corrió antes que cualquier otro código: `clawg metrics sessions` fallaba con `unrecognized subcommand 'sessions'`. Pasó recién con el reporte, la última pieza que usa |
| SU12, parte D | `a_session_end_closes_the_session_and_what_follows_opens_another` | Escrito cuando el hook y el reporte ya estaban: pasó a la primera. Lo que comprueba (el cierre con motivo y la fila nueva) lo fijaron antes `a_closed_session_is_not_reopened` y `a_session_end_closes_its_row_with_the_reason` |
| SU6 | `an_accepted_save_and_an_empty_search_count_for_their_session` | Falló por una suposición del test, no del código: mandaba dos veces el mismo `operation_id` esperando un solo save, y un save ya guardado no deja fila en el outbox, así que el segundo se acepta de nuevo. La spec dice ahora lo que el daemon hace |
| SU11 | `a_report_groups_sessions_by_project_agent_and_day` | Falló por una fecha mal calculada en el test; el dato se cambió para que el desfase de la fila decida el día |
| SU9 | `a_hook_names_its_event_and_its_source` | Falló porque el test leía las filas en dos tandas y la primera se llevaba la de Cursor |
| — | `a_hook_usage_error_exits_1` (ya existía) | Usaba `session-end` como ejemplo de un evento que `clawg hook` no conoce. Ahora usa `pre-compact` |

Los demás tests de la spec se escribieron junto con el código de su tarea y no compilaban antes: los módulos y las funciones que llaman no existían.

## La spec contra el código

| Requisito | Dónde |
|---|---|
| SU1 | `SCHEMA_VERSION` y las tablas `session` y `session_count` en `SCHEMA` (`crates/clawg-core/src/metrics/store.rs`); `sessions_where` lee como vacío un archivo sin la tabla |
| SU2 | `SessionLog::hook` y `SessionLog::activity`, `SessionRow`, y los conjuntos `SOURCES`, `END_REASONS`, `CLIENTS` con `label` (`crates/clawg-core/src/metrics/sessions.rs`); `TOOLS` y `tool_label` (`crates/clawg-cli/src/daemon/sessions.rs`) |
| SU3 | `IDLE_CLOSE_MS`, `idle_closed`, `State::settle`, `SessionLog::end` y `SessionLog::close_idle` (`sessions.rs`) |
| SU4 | `Recorder::sessions` y `Recorder::set_writing` (`recorder.rs`); `Writer::start`, `Writer::flush` y `Writer::flush_sessions` (`crates/clawg-cli/src/daemon/metrics.rs`); `MetricsDb::write_sessions`, `close_idle_sessions` y `open_sessions` (`store.rs`) |
| SU5 | `MetricsDb::prune` (`store.rs`), con `Pruned::dropped_sessions` |
| SU6 | `register_session`, `end_session`, `count_tool_call` y `count_empty_search`; el save en `save_memory`, en `Accepted::Queued` (`crates/clawg-cli/src/daemon/api.rs`) |
| SU7 | `sessions_follow` en las tres uniones y el borrado en `delete_projects` (`api.rs`); `SessionLog::merge_project`, `delete_project` y `changed`; `run_writer` escribe cuando el registro lo despierta |
| SU8 | `ClawgMcp::counted` y `window_project` en las trece herramientas (`crates/clawg-cli/src/mcp/server.rs`); `ClientHandle::peek` (`mcp/startup.rs`); `DaemonClient::report_call` (`client.rs`) |
| SU9 | `HookEvent::SessionEnd` (`cli.rs`); `end_session`, `SESSION_END_DEADLINE` y `deadline_of` (`hook.rs`); `SessionEvent` y `DaemonClient::end_session` (`client.rs`) |
| SU10 | `HOOK_EVENTS` en `setup.rs` y en `setup/cursor.rs`, con `Plan::hook_steps` |
| SU11 | `session_report::report`, `one_session` y `RowView::of` (`crates/clawg-core/src/metrics/session_report.rs`); `MetricsAction::Sessions` (`cli.rs`) y `sessions` (`crates/clawg-cli/src/metrics.rs`) |
| Invariantes | `no_text_path_or_user_reaches_a_session_row` (`crates/clawg-cli/tests/api.rs`); `memory.db`: ninguna migración nueva, y `a_hook_registers_the_session_of_the_saves` pasa sin cambios |

## Frente a lo previsto

| Punto | Previsto | Hecho | Por qué |
|---|---|---|---|
| El proyecto del aviso del puente | el de la carpeta abierta, registrada como hace el hook | el id que el puente ya tiene de esa carpeta, o el derivado de su raíz, sin registrar nada | Registrar un proyecto escribe en `memory.db`, y una herramienta que hoy no registra ninguno pasaría a hacerlo sólo para ser contada ([diseño](design.md), D3) |
| Las carpetas del cliente | sin cambios | las cinco herramientas que van por id de memoria también le piden las carpetas al cliente (`roots/list`) | Sin eso, en un cliente que declara carpetas no se sabe de qué ventana es la llamada |
| Los nombres del reporte | con el nombre de cada proyecto, leyendo `memory.db` | por id | Leer `memory.db` sin daemon pide las reglas de `export`; el id se cruza con `clawg projects list` ([diseño](design.md), D7) |
| Un save repetido | no cuenta dos veces | cuenta cada vez que el outbox lo acepta | Es lo que el daemon hace con un `operation_id` cuyo save ya se guardó |
| `agent-eval` | sin cambios | el brazo que instala lo que `setup` escribe (`arm.rs`) instala también `SessionEnd` | Arma su `settings.json` con `setup::plan`. Su test sólo mira los otros dos eventos y pasa |

## En local, Windows, 2026-11-11

Sobre `develop` en `aa025d6`, el mismo código que el pull request. Un build debug con la feature `testing`, copiado a una carpeta llamada `bin con espacios`, y un daemon de prueba (`CLAWG_TEST_HOME`, embedder falso, `--port 0`) que servía por el socket y por TCP. El cliente fue Claude Code 2.1.322 real, con una carpeta de configuración propia (`CLAUDE_CONFIG_DIR`) donde `clawg setup claude-code` registró el servidor, los hooks y el protocolo; la configuración de Claude Code de la máquina no cambió. Cursor no se abrió: `clawg setup cursor` escribió `hooks.json` en el home de prueba, y sus dos líneas se corrieron como las corre Cursor en Windows, con `powershell -NoProfile -NonInteractive -Command "<línea>"` y la entrada de Cursor por stdin. Cada fila se leyó con `clawg metrics sessions --session <id>`, con el id del nombre del transcript, pasado el minuto en que se escribe.

1. **Claude Code, un arranque y un prompt reales: pasa.** Una sesión con un prompt que pidió `search_memory`: `client` `claude-code`, `starts` `startup` 1, `prompts` 1, `calls` `search_memory` 1 sin fallas y `empty_searches` 1 (la memoria de prueba estaba vacía). El aviso de la herramienta y el cierre llegaron al daemon por el socket.
2. **Cursor: pasa lo que se puede simular.** La fila tiene `client` `cursor`, `starts` `unspecified` 1 y `prompts` 0. `session-start` leyó las claves de la entrada (`hook-runs.jsonl`) y registró la carpeta, escrita con barras invertidas o con barras normales: las dos dan el mismo proyecto. Que el `session_id` de `sessionStart` sea el mismo que el de `sessionEnd` sólo lo dice Cursor real: queda en prod.
3. **Los cierres: pasa.**
   - Al salir con `/exit`, la fila cierra con `prompt_input_exit`.
   - Con `/clear`, la fila cierra con `clear`, y 55 ms después se abre una fila de otro id con `starts` `clear` 1, que cuenta su prompt y cierra con `prompt_input_exit` al salir.
   - `logout` no se probó: es otro valor de la misma lista. En Claude Code 2.1.322, el `reason` de `SessionEnd` es uno de `clear`, `resume`, `logout`, `prompt_input_exit` y `other` (el esquema del evento en el ejecutable del cliente), y los cinco están en `END_REASONS`.
   - Con la entrada de Cursor, `user_close`, `window_close`, `completed`, `aborted` y `error` quedan con su nombre, y un motivo inventado queda como `other`. Los motivos que manda Cursor real quedan en prod.
4. **La X de la terminal en Windows dispara `SessionEnd`.** La fila de una sesión cuya ventana de PowerShell se cerró con la X, sin `/exit`, cerró 11 s después de su último prompt, con `other`. Ese cierre no espera las 6 horas. Lo que queda para decidir si hace falta vigilar el proceso del cliente es cuántas sesiones cierran con `idle` en el reporte del piloto.
   - Una sesión que se cortó en el arranque, antes de su primer prompt (la persona cerró un aviso o salió enseguida), no avisó su cierre ni dejó transcript. Su fila quedó abierta y se cerrará con `idle` por la regla de las 6 horas. Es uno de los casos que ese reporte cuenta.
   - Un `clawg stop`, un arranque, la escritura de un minuto y otro `clawg stop` dejaron esa fila abierta, como era (SU3, SU4).
5. **`resume`: no se probó.** Queda en prod.
6. **El hook de cierre dentro del presupuesto: pasa.** `clawg hook session-end`, en diez corridas contra el daemon de prueba, tardó de 87 a 101 ms de punta a punta (con el arranque del proceso, en un build debug), contra los 1,5 s de `SessionEnd`. `/clear` no se sintió más lento.
7. **El reporte de varios días: queda en prod.** Sobre las filas de la prueba, `--by project`, `--by agent` y `--by day` dieron los grupos, cierres, arranques y llamadas esperados.
8. **El archivo de un clawg anterior: pasa.** Sobre una copia del `metrics.db` de un perfil en uso (esquema 2, 17 ejecuciones):
   - `metrics report`, `metrics report --events` y `metrics sessions` lo leyeron sin cambiarle un byte, y `metrics sessions` dio `groups` vacío.
   - El daemon lo llevó al esquema 3. Por versión, las series de `http.duration`, `process.private_bytes` y `search.duration` y los eventos quedaron iguales; sólo se sumó la ejecución nueva.
   - La vuelta: el SQL de `docs/reference/data-profiles.md`, sobre una copia del archivo ya en el esquema 3, deja `integrity_check` en `ok`, `user_version` 2 y las seis tablas del esquema 2 con las mismas filas.
   - La 0.80.0 no se corrió. Su `open_read_only` y su `open_writable` rechazan sólo un esquema mayor que 2 (`MetricsError::Newer`, con `a_newer_schema_is_refused_untouched`), así que abre ese archivo y rechaza uno del esquema 3 sin tocarlo.
9. **Windows, la línea de PowerShell de Cursor con una ruta con espacios: pasa.** `setup cursor` escribió `& "…\bin con espacios\clawg.exe" hook session-end --agent cursor --client cursor --profile dev`. Windows PowerShell 5.1 la corrió con la entrada por stdin y la fila cerró con su motivo. Con PowerShell 7 no se probó: no estaba instalado. El CI cubre el texto de la línea; esto cubre que PowerShell la lee así y le pasa la entrada al binario.

## Pendiente en prod

En el piloto del [ROADMAP](../../../ROADMAP.md), con 0.81.0 instalada y `clawg setup claude-code --local` y `clawg setup cursor` corridos otra vez, para tener el hook de cierre:

1. **Cursor real.** Que `sessionStart` y `sessionEnd` traen el mismo `session_id`, y que la fila queda con `client` `cursor`, `starts` `unspecified` y el motivo con el que Cursor cierra la conversación. Un motivo que quede como `other` es uno que falta en `END_REASONS`.
2. **`resume`.** Retomar una sesión con `claude --resume` y con `/resume`, y mirar si `starts` suma `resume` en la misma fila (mismo id) o si aparece una fila de otro id.
3. **El reporte de una semana.** `clawg metrics sessions --by project`, `--by agent` y `--by day`:
   - cuántas sesiones cierran con `idle` frente a las que cierran por hook, contando las que se cortan antes de su primer prompt;
   - el tiempo entre el último save y el cierre, y el porcentaje con resumen;
   - si las llamadas se atribuyen: una sesión con prompts y sin ninguna llamada, en un proyecto donde se usó clawg, es una que no se atribuyó.

   Con ese reporte se decide si hace falta vigilar el proceso del cliente.

## Al archivar

- SU1 a SU12, el invariante enmendado y los límites pasan a `specs/metrics`; las tres rutas, a `specs/daemon-access`; `session-end`, a `specs/hooks`; los hooks que instala `setup`, a `specs/setup`.
- La fila 121 del `README.md` de openspec pasa a `changes/archive/README.md`, y el párrafo del cambio en `ROADMAP.md` dice lo que salió; lo de «Pendiente en prod» pasa al párrafo del piloto.
