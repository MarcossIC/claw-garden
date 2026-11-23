# summary-on-stop — Plan

Corre en: nube · Necesita a la persona: no (las cuatro decisiones de la [exploración](exploration.md#decisiones-abiertas) están cerradas abajo); para correr la medición después de publicar, sí

## Problema

Una sesión deja dónde quedó sólo si el agente se acuerda de guardar un `session_summary`. Según `clawg metrics sessions`, 27 de las 122 sesiones que guardan algo dejan su resumen (`openspec/ROADMAP.md:11`), y la meta de la etapa pide más de la mitad. Ningún hook de clawg corre cuando el agente termina un turno: `HookEvent` tiene cinco eventos (`crates/clawg-cli/src/cli.rs:962-974`) y `setup` instala esos cinco (`HOOK_EVENTS`, `crates/clawg-cli/src/setup.rs:56-62`). `session-end` llega cuando el agente ya no puede escribir (`end_session`, `crates/clawg-cli/src/hook.rs:589-597`).

Lo que ya existe:

- El daemon cuenta los saves aceptados de cada sesión por tipo: `save_memory` toma el tipo (`unspecified` sin tipo, `crates/clawg-cli/src/daemon/api/memories.rs:865`; la constante en `crates/clawg-core/src/metrics/sessions.rs:28`) y suma `Activity::Save` a la sesión que el registro da para el agente y el proyecto (`memories.rs:884`). `SessionLog::activity` lo anota en la fila abierta (`metrics/sessions.rs:307-330`, `CountKind::Save` en `:52-61`). Es la misma cuenta que lee `clawg metrics sessions` para decir qué sesiones guardaron un resumen.
- El registro de sesiones del daemon vive en memoria (`SessionRegistry`, `crates/clawg-cli/src/daemon/sessions.rs:53-98`), y las rutas de sesiones son `POST /v1/sessions`, `/v1/sessions/end` y `/v1/sessions/calls` (`crates/clawg-cli/src/daemon/api.rs:275-277`). `end_session` del hook no registra el proyecto ni lee la carpeta, para no despertar el modelo (`hook.rs:586-588`).
- `main` imprime en stdout lo que devuelve el hook y sale con 0 (`crates/clawg-cli/src/main.rs:128-136`); el sobre por evento lo arma `envelope` (`hook.rs:290-302`).
- El hook `Stop` de Claude Code (documentación de hooks de Claude Code, sección «Stop»): recibe `session_id`, `transcript_path` y `stop_hook_active`; contestar `{"decision": "block", "reason": "…"}` con salida 0 hace que el agente siga con ese texto; con `stop_hook_active` en true el agente ya siguió por un hook de `Stop`, y el hook tiene que dejarlo terminar.

## Objetivo y fuera de alcance

Objetivo 1, y el criterio de la etapa «más de la mitad de las sesiones que guardan algo dejan su `session_summary`» (`ROADMAP.md:11`). Cuando el agente de Claude Code termina un turno de una sesión que guardó algo que vale resumir y ningún `session_summary`, clawg le pide una sola vez que lo guarde, y después lo deja terminar.

`Stop` corre al final de cada turno, no de la sesión. Con «una vez por sesión», el pedido llega al final del primer turno que guardó algo, y lo que la sesión haga después no entra en ese resumen. La persona lo eligió así (decisión 1); la exploración lo admite.

Fuera de alcance:

- Cursor (decisión 3): `clawg hook stop --client cursor` no hace nada, y `clawg setup cursor` no lo instala (su tabla de eventos es otra, `crates/clawg-cli/src/setup/cursor.rs:33`).
- Pedirlo otra vez cuando hay memorias nuevas después del resumen (decisión 1).
- Dos sesiones del mismo agente y proyecto en paralelo: los saves de una se cuentan en la otra (`SessionRegistry::current`, `sessions.rs:80-83`). Es session-lifecycle (99), dormido.
- Un save con `all_projects` se cuenta bajo la sesión del agente sin proyecto (`memories.rs:863`), que en Claude Code no existe: no suma.
- Una sesión que pasa más de 6 h sin eventos cierra su fila (`State::settle`, `metrics/sessions.rs:204-209`; `IDLE_CLOSE_MS`, `:19`) y la que sigue empieza sin saves; si guardó su resumen antes y un hecho después, recibe el pedido. Se acepta: es raro y cuesta un turno.
- El contenido del resumen: el pedido nombra las secciones del protocolo, y nada revisa que el agente las use.
- `agent-eval`: sus brazos con setup arman los hooks con `setup::plan` (`crates/clawg-eval/src/bin/agent-eval/arm.rs:327-333`), así que A3 y los demás brazos con setup reciben `Stop` como cualquier instalación. El código de `agent-eval` no cambia; la próxima campaña necesita una línea base nueva, y `verify` lo deja escrito.
- Nada toca `memory.db`, el ranking ni el contrato MCP fijado.

## Decisión

Cerradas por la persona el 2026-11-23:

1. **Una vez por sesión.** El daemon recuerda en memoria las sesiones a las que ya les pidió, y marca una sólo cuando le contesta que sí. Un reinicio del daemon lo olvida y, en el peor caso, pide una vez más.
2. **Qué cuenta como «guardó algo»:** un save aceptado de cualquier tipo que no sea `session_summary` ni `tool_error`, incluido el que llega sin tipo (`unspecified`). Una sesión que ya guardó un `session_summary`, guarde lo que guarde además, no recibe el pedido.
3. **Sólo Claude Code.**
4. **El texto del pedido.** Va en `crates/clawg-cli/src/summary_request.md` y `crates/clawg-cli/src/summary_request.es.md`, que el hook incluye con `include_str!` según la preferencia de idioma (inglés sin preferencia), sin cambiar nada:

   `summary_request.md`:
   > clawg: this session saved memories but no session summary yet. Before you finish, save one with `save_memory`, memory_type `session_summary` and its project as in any save, with these sections: Goal, Instructions, Discoveries, Accomplished, Next steps, Relevant files, Validation. Then end your turn. clawg asks this once per session.

   `summary_request.es.md`:
   > clawg: esta sesión guardó memorias pero todavía ningún resumen de sesión. Antes de terminar, guarda uno con `save_memory`, memory_type `session_summary` y su proyecto como en cualquier save, con estas secciones: Objetivo, Instrucciones, Descubrimientos, Logrado, Próximos pasos, Archivos relevantes, Validación. Después termina tu turno. clawg lo pide una sola vez por sesión.

   Las secciones son las de «Session summary» del protocolo (`crates/clawg-cli/src/mcp/protocol.md:128-139`, `protocol.es.md:132-144`) y del protocolo corto que `setup` escribe por defecto (`protocol.lean.md`, `protocol.lean.es.md`); un test lo ata.

De dónde sale el conteo:

- **A. El registro de sesiones de `metrics.db` (elegido).** Una lectura en memoria de la fila abierta que cuenta el save en el momento en que se acepta, y la misma cuenta que mide el criterio de la etapa. Sin métricas (`Recorder::set_writing(false)`, `crates/clawg-core/src/metrics/recorder.rs:191-194`) no hay fila y el hook no pide nada: tampoco habría reporte que lo midiera.
- **B. `memory.source_session_id`.** Sobrevive a un reinicio, pero no tiene índice (una lectura de toda la tabla en cada final de turno) y no ve un save que espera en el outbox.
- **C. Leer el transcript.** Depende de un formato que no es contrato y de los nombres de herramienta del puente (descartado en la exploración).

El plazo del hook es 1 s, el de `session-end` (`SESSION_END_DEADLINE`, `hook.rs:40`): `Stop` corre al final de cada turno y una pregunta al daemon local tarda milisegundos; un hook más lento atrasaría cada turno.

Si la respuesta del daemon llega después del plazo, la sesión queda marcada sin que el agente reciba el pedido. Se acepta: pasa sólo con un daemon trabado, y la alternativa (marcar en un segundo pedido) duplica las llamadas de cada turno.

## Hecho

- H1. Dada una sesión que guardó un `semantic_fact` y ningún `session_summary`, cuando el hook `stop` de Claude Code corre con su `session_id`, entonces MUST imprimir `{"decision":"block","reason":<el texto de summary_request.md>}` → `hook.rs::a_stop_asks_for_the_summary_of_a_session_that_saved`
- H2. Dada la misma sesión, en un segundo `stop` MUST NOT imprimir nada → `hook.rs::a_stop_asks_only_once`
- H3. Dada `stop_hook_active: true`, `stop` MUST NOT imprimir nada → `hook.rs::a_stop_that_already_continued_lets_the_agent_finish`
- H4. Dada `stop_hook_active: true`, `stop` MUST NOT preguntar al daemon → el mismo test, con un daemon de prueba que cuenta los pedidos
- H5. Dada una sesión cuyo primer turno no guardó nada, el daemon MUST contestar `ask: false` en ese turno y `ask: true` en el turno después de guardar un `semantic_fact` → `tests/api.rs::a_session_is_asked_for_its_summary_after_its_first_save`
- H6. Dada una sesión que sólo guardó un `tool_error`, el daemon MUST contestar `ask: false`, y después de un `semantic_fact`, `ask: true` → el mismo test
- H7. Dada una sesión que guardó un `session_summary` y un `semantic_fact`, el daemon MUST contestar `ask: false` → `daemon/api/sessions.rs::summary_due_counts_only_what_is_worth_a_summary` (pura)
- H8. Dado un save sin tipo (`unspecified`), cuenta como algo que vale resumir → el mismo test puro
- H9. Dado un `session_id` que el daemon no conoce, MUST contestar `ask: false`; con uno inválido, `400` → `tests/api.rs::the_summary_question_answers_no_for_an_unknown_session`
- H10. `POST /v1/sessions/summary` MUST NOT cargar el modelo → `tests/api.rs::the_summary_question_leaves_the_model_unloaded`
- H11. Sin daemon, con uno que contesta 404 o con una entrada que no es JSON, `stop` MUST NOT imprimir nada → `hook.rs::a_stop_without_an_answer_lets_the_agent_finish`
- H12. Con un daemon que no contesta, `stop` MUST volver dentro de `STOP_DEADLINE` (1 s) sin imprimir → `hook.rs::a_stop_against_a_silent_daemon_returns_by_its_deadline`
- H13. Con `--client cursor`, o una entrada con `cursor_version`, `stop` MUST NOT imprimir nada → `hook.rs::a_stop_from_cursor_does_nothing`
- H14. El binario, con `clawg hook stop`, MUST salir con 0 e imprimir el `block` → `tests/binary.rs::a_stop_hook_blocks_once_and_exits_0`
- H15. `clawg setup claude-code` MUST instalar `Stop` en el `settings.json` del usuario → `setup.rs::setup_installs_the_stop_hook`
- H16. `clawg setup claude-code --local` MUST instalarlo en `.claude/settings.local.json` → `setup/local.rs::a_local_setup_installs_the_stop_hook`
- H17. `--remove` MUST quitarlo → `setup.rs::remove_takes_the_stop_hook_away`
- H18. `clawg doctor` MUST dar un hallazgo `stop_hook` (info) para un settings que corre clawg al empezar la sesión y no tiene `Stop` → `doctor/checks.rs::a_settings_without_the_stop_hook_is_named`
- H19. El hallazgo `tool_hooks` MUST NOT cambiar: no nombra `Stop` → los tests de `tool_hooks` (`doctor/checks.rs:2215-2256`) y las afirmaciones sobre `missing` de `setup.rs:2053` y `:2060`, sin cambios
- H20. Cada texto del pedido MUST nombrar `save_memory` y `session_summary` → `hook.rs::the_summary_request_names_the_tool_and_the_type`
- H21. Cada texto MUST nombrar cada sección de «Session summary» de `protocol.md`/`protocol.es.md` y de su protocolo corto → `hook.rs::the_summary_request_names_the_sections_of_the_protocol`
- H22. `cli.rs` MUST aceptar `clawg hook stop` → `cli.rs::hook_takes_the_stop_event`
- No cambia nada fuera de: `cli.rs`, `hook.rs`, `client.rs`, `daemon/api.rs`, `daemon/api/sessions.rs`, `daemon/sessions.rs`, `clawg-core/src/metrics/sessions.rs`, `setup.rs`, `setup/local.rs`, `doctor/checks.rs`, `doctor/mod.rs`, los dos archivos del pedido, `tests/api.rs` (sus tests y la fila de `PROJECT_INPUTS`, `:5732`), `tests/binary.rs`, las specs `hooks`, `setup`, `doctor`, `daemon-access` (SU6, que dice que las rutas de sesiones contestan `204`) y `metrics` (`:300`, lo mismo), `openspec/specs/eval/spec.md` (`a3-notools`, `:1969`), `docs/reference/daemon.md`, `docs/reference/commands.md`, `docs/reference/testing.md` (`:65`) y esta carpeta.
- Los tests que afirman la lista exacta de eventos instalados suman `Stop` (`setup/local.rs:284-355`; en `setup.rs`, `:1481`, `:1714`, `:1726`, `:1792`, `:1801`, `:1845`, `:1870`, `:1886`, `:1969`, `:1989`, `:2002`, `:2030`, `:2050`, `:2059`, `:2141` y `:2171`); ninguno se afloja ni se borra.

## Pasos

1. Test de `SessionLog::saves` (las cuentas de `Save` de la fila abierta, `None` sin fila o con el log apagado) → la función en `metrics/sessions.rs`. Commit `feat(metrics): read the saves of an open session by type`.
2. Tests de `summary_due` (H7, H8), de `SessionRegistry::ask_summary_once` y de la ruta (H5, H6, H9, H10) → `summary_due` pura, la marca en el registro que se toma sólo si `summary_due` dio true, `POST /v1/sessions/summary` que contesta `{ "ask": bool }` sin registrar proyecto ni tocar el embedder, y su fila en `PROJECT_INPUTS`. Commit `feat(daemon): tell a hook once whether a session should save its summary`.
3. Test del cliente contra el daemon de prueba y contra uno sin la ruta → `DaemonClient::summary_due`, que da `false` ante un 404. Commit `feat(client): ask the daemon whether a session should save its summary`.
4. Tests de `cli.rs`, `hook.rs` y `tests/binary.rs` (H1 a H4, H11 a H14, H20 a H22) → `HookEvent::Stop`, `STOP_DEADLINE`, `stop_hook_active` en `HookInput`, la rama de `respond_reading` que lee la entrada y pregunta, el sobre `{"decision":"block","reason":…}` y los dos archivos del pedido. Commit `feat(hook): ask once for the session summary when the agent stops`.
5. Tests de `setup` (H15 a H17) → `Stop` en `HOOK_EVENTS`; los tests de la lista exacta de eventos suman `Stop`. Commit `feat(setup): install the stop hook`.
6. Test de `doctor` (H18, H19) → `missing_stop_hook` en `setup.rs`, aparte de `missing_tool_hooks`, y el hallazgo `stop_hook` en `doctor/checks.rs`, llamado desde `doctor/mod.rs`. Commit `feat(doctor): name a settings file without the stop hook`.
7. Specs y documentación: un requisito nuevo en `specs/hooks` (después de SU9) y R2 y R3 con el sexto evento y el `block`; `HOOK_EVENTS` en `specs/setup`; el hallazgo en `specs/doctor`; los hooks de `a3` y `a3-notools` en `specs/eval` y `docs/reference/testing.md`; la ruta en `specs/daemon-access` (SU6) y `specs/metrics`; `docs/reference/daemon.md` y `commands.md`. Commit `docs: describe the stop hook that asks once for the session summary`.
8. Cierre: la suite y clippy una vez, la revisión, `Verificación` y el archivo.

**Vuelta atrás:** primero `clawg setup claude-code --remove` con la versión nueva, que quita `Stop` con los otros hooks, y después instalar la anterior y correr su setup. Sin ese orden, `Stop` queda en `settings.json`, el binario anterior sale con 1 ante el evento desconocido (`crates/clawg-cli/src/main.rs:51-57`), y Claude Code muestra en cada turno un error de hook que no bloquea.

## Registro

- 2026-11-23: las cuatro decisiones las cerró la persona en la conversación (una vez por sesión; sin `tool_error` ni `session_summary`; sólo Claude Code; texto propuesto por el plan y fijado en un archivo). Pidió trabajar el pedido hasta terminarlo, así que el plan lo aprueba la revisión de plan, como en el trabajo autónomo.
- 2026-11-23, revisión de plan, ronda 1: BLOCKED. Se arregló: el texto de los dos pedidos, entero (C1); la marca se toma sólo cuando corresponde pedir, con H5 y H6 (C2); `doctor` tiene una función y un hallazgo propios para `Stop` y `tool_hooks` queda igual (C3, H19); `agent-eval` en el alcance (W1); la lista de archivos (W2); la cifra con `ROADMAP.md:11` (W3); las citas de `memories.rs` (W4); criterios de una afirmación, con test (W5); el resumen con un hecho y el save sin tipo (W6); el modelo descargado (W7); la vuelta atrás, el plazo y los huecos aceptados.
- 2026-11-23, revisión de plan, ronda 2: WARNINGS, plan aprobado. Se arregló en el plan: H19 deja fuera los `added` de `setup.rs:2050` y `:2059`, que suman `Stop` (W1); `specs/eval` y `testing.md` en la lista y en el paso 7 (W2); la lista completa de tests de lista exacta (W3); el orden de la vuelta atrás y su cita.

## Verificación

Pendiente.

En local, después de publicar y con la versión instalada (perfil real, con autorización): correr `clawg setup claude-code` para que se instale `Stop`, y medir con `clawg metrics sessions` la fracción de sesiones que guardan algo y dejan su resumen, frente a 27 de 122.
