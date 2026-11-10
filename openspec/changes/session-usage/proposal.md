# session-usage — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal (medir el uso real por sesión, proyecto y agente) | M, por partes | —; se coordina con la parte A de [session-lifecycle](../session-lifecycle/exploration.md) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md), verificado otra vez contra `develop` (`68b8851`).

- **Una sesión es un id en un mapa en memoria.** `SessionRegistry` guarda la última sesión de cada agente y proyecto (`crates/clawg-cli/src/daemon/sessions.rs:33-35`) y nada más: ni comienzo, ni fin, ni contador. Se pierde al reiniciar el daemon (`:6-7`).
- **El id lo traen sólo los hooks, y sin decir qué pasó.** `session-start` y `user-prompt-submit` mandan el mismo cuerpo a `POST /v1/sessions` (`crates/clawg-cli/src/hook.rs:447-451`; `SessionRequest`, `crates/clawg-cli/src/daemon/api.rs:407-412`): agente, proyecto e id. El daemon no distingue un arranque de un prompt.
- **El daemon no sabe qué herramienta se llamó.** Cada herramienta del puente pasa por `ClawgMcp::call` (`crates/clawg-cli/src/mcp/server.rs:747`) y llega como peticiones HTTP sin su nombre. Una herramienta hace varias peticiones, y varias herramientas comparten ruta.
- **`metrics.db` mide el daemon.** Sus series se etiquetan por ruta, método y estado, y su spec prohíbe ids y deja fuera la etiqueta de agente ([`specs/metrics`](../../specs/metrics/spec.md), «Invariantes» y «Límites conocidos»). `clawg metrics report` no agrupa por proyecto, agente ni sesión (`crates/clawg-cli/src/metrics.rs:16`).
- **Ningún hook ve el fin.** `setup` instala `SessionStart` y `UserPromptSubmit` en Claude Code (`crates/clawg-cli/src/setup.rs:51`) y `sessionStart` en Cursor (`crates/clawg-cli/src/setup/cursor.rs:31`); `HookEvent` no tiene más variantes (`crates/clawg-cli/src/cli.rs:915-918`).

Con esto no se puede contestar cuánto se usa clawg: cuántas sesiones hay por proyecto, cuánto duran, cuántas veces llaman, cuántas guardan y cuántas cierran con resumen. Una sesión que llama 40 veces y no guarda nada no deja ninguna fila en `memory`.

## Objetivo que sirve

Transversal. Sirve a 1, 2 y 4 porque dice qué sesiones buscan, guardan y cierran con resumen, por proyecto y por agente, en uso real y no sólo en las corridas controladas de [agent-eval](../agent-eval/proposal.md).

## Decisiones

Están cerradas en la exploración ([«Decisiones (2026-11-10)»](exploration.md#decisiones-2026-11-10)). Esta propuesta no las reabre; dice cómo se aplican.

1. **El registro vive en `metrics.db`, esquema 3.** Dos tablas nuevas, `session` y `session_count`. `memory.db` no cambia: ni su esquema, ni sus migraciones, ni lo que guarda. En particular, la sesión con la que se sella un save (`memory.source_session_id`) se sigue leyendo del mismo mapa en memoria y con la misma regla que hoy. Con `CLAWG_METRICS=off` no se registran sesiones.
2. **La duración va del inicio al cierre, y 6 horas sin actividad cierran la sesión** con motivo `idle` y con su fin en la última actividad (`IDLE_CLOSE_MS`). Una sesión cerrada, por la regla o por el hook, no se reabre: lo que llegue después con el mismo id abre una fila nueva. La fila guarda la hora del último save aceptado.
3. **Unir dos proyectos pasa sus sesiones al que queda; borrar un proyecto borra sus sesiones.**
4. **El hook de cierre se instala en este cambio**, en Claude Code (`SessionEnd`) y en Cursor (`sessionEnd`). Quien ya corrió `setup` lo corre otra vez para tenerlo.
5. **El reporte es `clawg metrics sessions`**, al lado de `clawg metrics report`, y también lee sin daemon.
6. **Vigilar el proceso del cliente queda fuera.**

Lo que la exploración no fijaba y se fija acá, con sus alternativas en el [diseño](design.md):

- **Cómo llega el nombre de la herramienta (parte B).** El puente avisa al daemon cuando una herramienta termina, con una petición propia, `POST /v1/sessions/calls`: agente, proyecto de la ventana, nombre de la herramienta y si falló. No va como cabecera de las peticiones de la herramienta, porque una herramienta hace varias y sólo el puente sabe cuándo terminó y qué le contestó al modelo.
- **A qué sesión va una llamada.** A la vigente del agente y del proyecto de la carpeta abierta en la ventana, en el mismo mapa que sella los saves. Con dos sesiones en paralelo del mismo agente y proyecto, va a la última que habló: el límite que ya tienen los saves y que arreglaría la parte A de session-lifecycle.
- **Los saves por tipo y las búsquedas sin hits** los cuenta el daemon en sus rutas (`POST /v1/memories`, `POST /v1/search`), para la sesión del agente y del proyecto de la petición: la misma con la que se sella el save.
- **El reporte nombra los proyectos por id.** No abre `memory.db`: `clawg projects list` da el nombre de cada id.

## Alcance

1. `crates/clawg-core/src/metrics/`: el registro en memoria y sus reglas (`sessions.rs`), el esquema 3 con sus lecturas y escrituras (`store.rs`), el reporte (`session_report.rs`) y el registro colgado del `Recorder`.
2. `crates/clawg-cli/src/daemon/`: `POST /v1/sessions` con `event`, `source` y `client`; `POST /v1/sessions/end`; `POST /v1/sessions/calls`; los saves aceptados y las búsquedas vacías; la unión y el borrado de proyectos; el escritor, que guarda las sesiones con cada minuto y las retoma al arrancar.
3. `crates/clawg-cli/src/mcp/`: el puente avisa cada llamada con su nombre. El contrato del agente no cambia.
4. `crates/clawg-cli/src/hook.rs` y `cli.rs`: `clawg hook session-end`, y el evento y el `source` en los dos hooks de hoy.
5. `crates/clawg-cli/src/setup*`: `SessionEnd` en Claude Code (usuario y `--local`) y `sessionEnd` en Cursor.
6. `crates/clawg-cli/src/metrics.rs` y `cli.rs`: `clawg metrics sessions`.
7. Docs: `docs/reference/` (comandos, daemon, `metrics.db` y cómo se vuelve a una versión anterior) y `README.md`.
8. Lo que el CI del pull request marque en Windows.

## Fuera de alcance

- Lo que la exploración pone en [«Fuera de alcance»](exploration.md#fuera-de-alcance): tokens y costo, mandar algo fuera de la máquina, una página o tablero, separar los subagentes y vigilar el proceso del cliente.
- Lo que pone en «No se registra»: texto de consultas, prompts o memorias, rutas, el usuario, las llamadas que no llegan al daemon y los comandos de `clawg` por CLI.
- De «Qué más vale registrar», el `model` de `SessionStart` y los avisos de guardado. La decisión 2 deja lo que se capture en el medio para después, según sirva. Las llamadas por prompt salen de dos columnas que el reporte ya trae.
- La atribución exacta con sesiones en paralelo (`PostToolUse`, o el checkout en la clave).
- Subir la versión y archivar.

## Cómo se mide

Tests que hoy fallan, todos en Linux con el embedder falso:

- `a_daemon_keeps_each_session_across_a_restart_and_reports_them` (`crates/clawg-cli/tests/binary.rs`), el de «Cómo se mide» de la exploración: un `session-start` con id S, dos llamadas por el puente y un `user-prompt-submit`; un `session-start` con `source=clear` e id S2; un reinicio del daemon y un prompt de S2. Tiene que dar S con 2 llamadas y 1 prompt, S2 con su prompt en la misma fila de antes del reinicio, y el reporte por proyecto con las dos. Hoy falla porque `clawg metrics sessions` no existe.
- `a_session_end_closes_the_session_and_what_follows_opens_another` (mismo archivo): un `session-end` de S con motivo `clear` la cierra con ese motivo, y un evento posterior de S abre una fila nueva.
- `six_hours_without_activity_close_a_session_as_idle` (`crates/clawg-core/src/metrics/sessions.rs`), sin reloj ni base: un evento a las 5 h 59 min del anterior sigue la fila; uno a las 6 h la cierra con `idle` en la hora del anterior y abre una nueva.
- Los demás tests de la [spec](spec.md).

`the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` tienen que pasar sin bendecir.

## Datos guardados, migración y vuelta atrás

- **`memory.db`:** no cambia.
- **`metrics.db`:** pasa de `PRAGMA user_version = 2` a `3`. Un archivo del esquema 2 recibe las dos tablas cuando el daemon lo abre para escribir, con `CREATE TABLE IF NOT EXISTS`, sin tocar ninguna fila. Un test abre un archivo del esquema 2 con filas y comprueba que siguen ahí (`a_file_of_schema_2_gets_its_sessions_and_keeps_its_rows`).
- **Vuelta atrás.** Un clawg anterior rechaza un `metrics.db` de esquema 3 sin tocarlo (`MetricsError::Newer`, `crates/clawg-core/src/metrics/store.rs:37`; lo comprueba al abrir para escribir, `:166`, y para leer, `:155`). El daemon anterior sigue atendiendo todo lo demás: deja un `warn` y no escribe métricas en esa ejecución (`Writer::start`, `crates/clawg-cli/src/daemon/metrics.rs:146-160`), y `clawg metrics report` sale con 1 nombrando el esquema. Para que vuelva a escribir, con el daemon parado se aparta o se borra `metrics.db`, y el próximo arranque crea uno nuevo; para conservar la historia de las series, se le quitan las dos tablas y se vuelve a poner `user_version` en 2. Ninguno de los dos pasos toca `memory.db`. Queda escrito en `docs/reference/data-profiles.md`.

## Riesgos

- **Un hook más al salir y en cada `/clear`,** dentro del presupuesto de 1,5 s de `SessionEnd`: `session-end` hace una sola petición al daemon, no registra el proyecto (`POST /v1/projects` despierta el modelo) y se da 1 s.
- **Una petición más por llamada a una herramienta,** del puente al daemon por loopback, acotada a 500 ms y sin efecto en lo que la herramienta contesta.
- **Si cerrar la ventana de la terminal dispara `SessionEnd`** no está verificado: queda en local ([verify](verify.md)). Si no lo dispara, esas sesiones cierran con `idle`.
- **Los campos de `sessionEnd` de Cursor** se leyeron de su documentación, no de una corrida: quedan en local.
