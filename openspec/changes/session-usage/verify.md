# session-usage — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, en `target`, sobre `606ad66`: el último commit de código y de `docs/reference`. El commit que sigue sólo toca esta carpeta.

- `cargo test --workspace --no-fail-fast` pasa.
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin bendecir: `agent_contract.json` y `agent_contract.es.json` no cambiaron. El nombre de la herramienta va del puente al daemon.
- El código `cfg(windows)` no se compiló ni se corrió acá. El cambio no agrega ninguno, pero pasa por código que lo tiene: el cliente del daemon por socket (`send_over_af_unix`, por donde van `POST /v1/sessions/end` y `POST /v1/sessions/calls`) y la línea del hook de Cursor para PowerShell. Lo revisa el job de Windows del CI del pull request.

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

## Pendiente en local

Linux con el embedder falso no comprueba nada de esto. Hace falta un clawg de esta versión instalado, `clawg setup claude-code` y `clawg setup cursor` corridos otra vez, y un perfil de datos con su daemon.

1. **Claude Code, un arranque y un prompt reales.** Abrir una sesión, mandar un prompt que llame a una herramienta de clawg, y leer `clawg metrics sessions --session <id>` pasado un minuto: la fila tiene `client` `claude-code`, `starts` con `startup`, `prompts` 1 y la herramienta en `calls`.
2. **Cursor, un arranque real.** Abrir un chat en una ventana con una sola carpeta: la fila tiene `client` `cursor` y `starts` con `unspecified`. Confirmar los campos que la exploración leyó de la documentación y no de una corrida: que `sessionStart` trae `session_id` y que es el mismo que trae `sessionEnd`.
3. **Los cierres, con su motivo.** Que la fila quede con `ended_at_ms` y:
   - `prompt_input_exit` o `logout` al salir de Claude Code;
   - `clear` después de `/clear`, con una fila nueva de otro id para lo que sigue;
   - el motivo de Cursor (`user_close`, `window_close`, `completed`, …) al cerrar la conversación. Un motivo que quede como `other` es uno que falta en `END_REASONS`.
4. **Si cerrar la ventana de la terminal con la X dispara `SessionEnd` en Windows.** La decisión 6 de la exploración depende de esto. Si lo dispara, la fila cierra con el motivo que mande Claude Code. Si no, queda abierta y a las 6 horas se lee como `idle`, con el fin en la última actividad: ese es el caso que vigilar el proceso del cliente arreglaría, y se decide con este dato y con cuántas sesiones cierran con `idle` en el reporte.
5. **Si `resume` conserva el id de la sesión,** que la exploración deja sin verificar: retomar una sesión con `claude --resume` y con `/resume`, y mirar si `starts` suma `resume` en la misma fila (mismo id) o si aparece una fila con otro id. Para la cuenta no cambia nada: una sesión cerrada no se reabre.
6. **El hook de cierre dentro del presupuesto.** Que `clawg hook session-end` termine dentro de los 1,5 s de `SessionEnd` con el daemon real, y que `/clear` no se sienta más lento.
7. **El reporte después de varios días de uso real.** `clawg metrics sessions --by project`, `--by agent` y `--by day` sobre una semana: cuántas sesiones cierran con `idle` frente a las que cierran por hook, el tiempo entre el último save y el cierre, el porcentaje con resumen, y si las llamadas se atribuyen (una sesión con prompts y sin ninguna llamada, en un proyecto donde se usó clawg, es una que no se atribuyó).
8. **El archivo de un clawg anterior.** Arrancar esta versión sobre un `metrics.db` real del esquema 2 y comprobar con `clawg metrics report` que sus series siguen ahí. Volver a la versión anterior y seguir los pasos de `docs/reference/data-profiles.md`.
9. **Windows,** si el CI no lo cubre: el hook de Cursor con la línea de PowerShell y una ruta con espacios, para `session-end`.

## Al archivar

- SU1 a SU12 y el invariante enmendado pasan a `specs/metrics`; las tres rutas, a `specs/daemon-access`; `session-end`, a `specs/hooks`; los hooks que instala `setup`, a `specs/setup`.
- La fila 121 del `README.md` de openspec y la línea del cambio en `ROADMAP.md`, que este cambio no tocó.
