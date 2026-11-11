# session-usage — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal (medir el uso real por sesión, proyecto y agente; sirve a 1, 2 y 4 porque dice qué sesiones buscan, guardan y cierran con resumen) | M, por partes (A es M; B, C y D son S) | —; se coordina con la parte A de [session-lifecycle](../../session-lifecycle/exploration.md) (99), que arreglaría la atribución con sesiones en paralelo; [agent-eval](../../agent-eval/proposal.md) (96) cuenta lo mismo en corridas controladas y sirve de contraste | exploración (decidida) |

**Tesis.** Hoy clawg sabe a qué sesión pertenece un save, pero no sabe nada de la sesión en sí: ni cuándo empezó, ni cuándo terminó, ni cuántas veces llamó a clawg. Una sesión es sólo un texto en un mapa en memoria, uno por agente y proyecto, que se pierde al reiniciar el daemon. El uso por sesión no entra en las series de `metrics.db`: su spec prohíbe ids y etiquetas de agente, y con razón, porque abrirían series sin cota. Lo que falta es un **registro de sesiones**: una fila por sesión del cliente, con su comienzo, su última actividad y contadores, al lado de las series y no dentro de ellas. El fin de una sesión no se puede esperar sólo del cliente, que no lo ve en todos los casos. Se toma del hook de cierre cuando llega y, si no, de la última actividad, con la sesión dada por cerrada después de 6 horas sin actividad.

## El problema verificado contra el código

1. **Una sesión es un id en un mapa en memoria.** `SessionRegistry` es un `HashMap<(agente, proyecto), id>` (`crates/clawg-cli/src/daemon/sessions.rs:33-35`). Se pierde al reiniciar el daemon (`:6-7`), y el último en registrarse gana (`:29-31`). No guarda comienzo, fin, última actividad ni contador.
2. **El id lo traen sólo los hooks.** Lo registran los dos eventos de Claude Code, en cada evento (`crates/clawg-cli/src/hook.rs:445-449`), con `POST /v1/sessions` (`crates/clawg-cli/src/daemon/api.rs:1127`, `register_session` en `:1748`). El cuerpo trae agente, proyecto e id y nada más (`SessionRequest`, `api.rs:407-412`). Cursor sólo instala `sessionStart` (`crates/clawg-cli/src/setup/cursor.rs:31`), así que registra una vez por sesión y no hay un evento por prompt.
3. **El save ya lleva la sesión.** Al aceptar un save, el daemon lee la sesión registrada para su agente y proyecto (`api.rs:1445`) y la escribe en `memory.source_session_id` (`crates/clawg-core/migrations/V1__init.sql:16`). `clawg memories --session` filtra por ella (`crates/clawg-cli/src/cli.rs:598-600`). **Las memorias por sesión ya se pueden contar hoy**, con dos límites: con dos sesiones del mismo agente en el mismo proyecto, el save sale con la sesión de la última que habló, y después de un reinicio sale sin sesión hasta el próximo hook.
4. **El puente no sabe la sesión ni el nombre de la herramienta que llama.** Cada herramienta pasa por `ClawgMcp::call` (`crates/clawg-cli/src/mcp/server.rs:747`), con el agente resuelto por `agent_for` (`:712`). Llega al daemon como una petición HTTP con agente y proyecto en el cuerpo y un bearer, sin el nombre de la herramienta. El daemon ve la ruta, y varias herramientas comparten ruta. El puente no avisa al daemon cuando arranca ni cuando termina.
5. **`metrics.db` mide el daemon, no al cliente.** Una `run` es una vida del daemon (`crates/clawg-core/src/metrics/store.rs:384`), y las series se agrupan por minuto y por hora (`:398-419`). Las peticiones se etiquetan por ruta, método y estado ([`specs/metrics`](../../../specs/metrics/spec.md) R5). La spec prohíbe que una etiqueta o una fila lleve un id (`spec.md:209`) y deja fuera la etiqueta de agente porque «es autodeclarado y abriría series sin cota» (`spec.md:217`). Los minutos se guardan 14 días y las horas y los eventos 400 (`store.rs:26`, `:29`). `clawg metrics report` agrupa por día, hora, minuto o versión, nunca por proyecto, agente o sesión.
6. **Ningún hook ve el fin de la sesión.** `setup` instala sólo `SessionStart` y `UserPromptSubmit` (`crates/clawg-cli/src/setup.rs:51`), y `HookEvent` no tiene más variantes (`cli.rs:915-918`). `SessionEnd` y `Stop` no están.
7. **El registro de hooks no sirve para esto.** `hook-runs.jsonl` guarda sólo las corridas de `session-start`, en un anillo de dos archivos de 64 KiB (`crates/clawg-cli/src/hook_log.rs:20-22`). No guarda el id de sesión: de la entrada guarda sólo los nombres de las claves (`input_keys`, `:77`, `:90`).
8. **Lo que ya cuenta por agente y proyecto es lo guardado, no la actividad.** `GET /v1/stats` cuenta filas, con filtro de agente y proyecto (`api.rs:1140`, `stats` en `:1810`). `clawg doctor` lista filas por agente.

**Lo que ya se descartó y por qué se reabre.** session-handoff descartó una tabla `session` (id, agente, proyecto, inicio, fin, resumen) porque lo que necesitaba entonces, el último resumen y las memorias de una sesión, salía de `memory` con la columna. Agregaba además un fin de sesión que el cliente no garantiza ([exploración](../2026-10-19-session-handoff/exploration.md), líneas 72 y 102). El dato nuevo es la pregunta: cuánto se usa clawg, cuánto dura una sesión y cuántas llamadas hace. La columna no puede contestarlo, porque una sesión que llama 40 veces y no guarda nada no deja ninguna fila. El argumento del fin sigue en pie, y por eso este cambio no depende sólo del cliente: el hook de cierre da el fin cuando llega (parte D), y si no llega, el fin es la última actividad, con la regla de las 6 horas (parte A).

## Qué da el cliente (documentación leída el 2026-11-08 y el 2026-11-10)

Fuente: https://code.claude.com/docs/en/hooks, salvo donde se nombra otra página.

| Dato | Claude Code | Cursor |
|---|---|---|
| Id de sesión | `session_id` en todo hook («Common input fields») | `session_id` en `sessionStart` y `sessionEnd`, y `conversation_id` en todos ([cursor.com/docs/agent/hooks](https://cursor.com/docs/agent/hooks)). En `sessionStart`, `session_id` es «same as `conversation_id`» |
| Cómo empezó | `source` en `SessionStart`: `startup`, `resume`, `clear`, `compact` o `fork`; a veces `model` | `composer_mode`, `is_background_agent` |
| `/clear` | Empieza una sesión nueva: `clear` es a la vez `source` de `SessionStart` y `reason` de `SessionEnd`. Que el id cambie no lo dice la página de hooks con esas palabras, pero sí las de `statusline` y `monitoring-usage` | — |
| `resume` | Si conserva el id: **sin verificar**. Para la cuenta no importa: una sesión cerrada no se reabre (decisión 2) | — |
| Fin | `SessionEnd` con `reason`: `clear` (`/clear`), `resume` (cambiar de sesión con `/resume` interactivo), `logout`, `prompt_input_exit` (salir con el prompt a la vista) y `other`. El presupuesto de 1,5 s rige al salir, en `/clear` y al cambiar con `/resume`. No puede bloquear. La página no dice que corra si se cierra la ventana de la terminal, si el proceso muere o si se apaga el equipo | `sessionEnd`, sin respuesta que se use, con `reason` (`completed`, `aborted`, `error`, `window_close`, `user_close`), `duration_ms` y `final_status` |
| PID del cliente | No lo da: ninguna variable de entorno del hook lo trae. Se puede sacar subiendo por los procesos padre desde el hook, con una consola en el medio | — |
| Cada llamada a una herramienta MCP | `PostToolUse` con matcher `mcp__clawg__.*`: `session_id`, `tool_name` y `duration_ms` | `afterMCPExecution`: `tool_name`, `mcp_server_name` y `duration` |
| Tokens | No hay en ningún hook. Ver [session-tokens](../../session-tokens/exploration.md) (122) | No hay |

## Partes

### A. El registro de sesiones (Incorporar; M)

Es una fila por sesión del cliente:

- **Identidad:** id del cliente, agente, proyecto (puede faltar) y cliente (`claude-code`, `cursor`).
- **Tiempos:** `started_at` (la primera vez que el daemon ve el id), `last_seen_at` (el último evento o llamada atribuida), `last_save_at` (el último save aceptado) y `ended_at` con `end_reason`: el motivo del hook de cierre (parte D), o `idle` cuando la cierra la regla de las 6 horas.
- **Cómo empezó y cuántas veces volvió:** contadores por `source` (`startup`, `resume`, `clear`, `compact`). Las compactaciones por sesión dicen cuánto se llena el contexto, que es lo que la parte B de 99 quiere saber antes de avisar.
- **Contadores:**
  - prompts (un `user-prompt-submit` es un prompt, y su id es exacto);
  - llamadas por herramienta, con cuántas fallaron;
  - saves aceptados por `memory_type`;
  - búsquedas sin ningún hit;
  - si guardó un `session_summary`.
- **Duración:** del inicio al cierre, sin umbral de tiempo activo. Una sesión abierta 12 horas con actividad dura 12 horas. Lo que se ve aparte es el hueco entre la última memoria guardada y el cierre: una sesión de 5 horas cuya última memoria fue 4 horas antes del cierre. La hora de cada memoria no se copia: ya está en `memory.created_at`, con su `source_session_id`.

**El fin.** Es el del hook de cierre cuando llega (parte D). Si no llega (la ventana de la terminal cerrada, sin verificar; un proceso que muere; el equipo que se apaga), el fin es `last_seen_at`, y la sesión se da por cerrada después de 6 horas sin actividad, con `end_reason` `idle`. Para eso no hace falta un proceso que revise: alcanza con mirar `last_seen_at` cuando llega el próximo evento del mismo id y cuando se arma el reporte. Una sesión cerrada no se reabre: si después llega actividad con el mismo id (un `resume`, o la misma ventana retomada a la mañana), empieza una fila nueva. Un reinicio del daemon no cierra nada: si el mismo id vuelve antes de las 6 horas, sigue la misma fila. El registro en memoria de hoy no puede hacerlo.

**La atribución de una llamada** usa el mismo registro que ya sella los saves: la sesión vigente de su agente y proyecto en ese momento (`api.rs:1445`). Tiene el mismo límite, con dos sesiones en paralelo del mismo agente y proyecto. Lo arregla la parte A de 99, que pone el checkout en la clave, y entonces se arregla para los saves y para las llamadas a la vez. Lo exacto sería `PostToolUse`, con el id en cada llamada, pero es un proceso más por cada llamada a clawg y otro nombre de herramienta si clawg se distribuye como plugin (`mcp__plugin_clawg_clawg__*`, [claude-code-plugin](../../claude-code-plugin/exploration.md)). Queda como alternativa si la medición muestra que las sesiones en paralelo pesan.

**Dónde vive.**

| Opción | A favor | En contra |
|---|---|---|
| **Tablas nuevas en `metrics.db`** (esquema 3) | Es telemetría: perderla no pierde ninguna memoria. Ya tiene poda, el interruptor `CLAWG_METRICS=off` y un lector sin daemon (`metrics report`), y no viaja en los exports de memoria | Hay que enmendar el invariante de `spec.md:209`: las series siguen cerradas, y estas tablas son el único lugar con ids (de sesión, de agente y de proyecto), nunca con texto. Para nombrar proyectos, el reporte lee también `memory.db` |
| Tabla nueva en `memory.db` | Se cruza directo con `memory.source_session_id` | Toca el esquema del activo más importante del usuario, con migración, snapshot y test de preservación, por datos que no son memoria. Agranda los backups |
| Un `sessions.db` aparte | Aislado | Otro archivo, otra versión de esquema y otra poda, para lo mismo que la primera opción |

Recomendado: `metrics.db`. Los contadores viven en memoria y se escriben con el volcado por minuto que ya existe (`Writer::flush`, `crates/clawg-cli/src/daemon/metrics.rs:202`). Un `kill` pierde como mucho un minuto, como las series. La retención es la de las horas, 400 días.

**Vuelta atrás:** un binario anterior rechaza un `metrics.db` de esquema más nuevo (`store.rs:37`, `:155`). La propuesta tiene que leer qué hace el daemon en ese caso y documentar cómo se vuelve, que como mucho es borrar o apartar `metrics.db`, sin tocar `memory.db`.

**Cómo se mide.** Un test de binario con `CLAWG_TEST_HOME`, el embedder falso y `--port 0`, que hoy falla:

1. Un `session-start` con id S.
2. Dos llamadas por el puente y un `user-prompt-submit`.
3. Un `session-start` con `source=clear` e id S2.
4. Un reinicio del daemon y un prompt de S2.
5. Lo que tiene que dar: S con 2 llamadas y 1 prompt, S2 con su prompt en la misma fila de antes del reinicio, y el reporte por proyecto con las dos.

La regla de las 6 horas es lógica pura y se prueba sin reloj ni base, con las horas como datos: un evento a las 5 h 59 min del anterior sigue la fila; uno a las 6 h la cierra con `idle` en la hora del anterior y abre una nueva.

### B. El nombre de la herramienta en cada llamada (Incorporar; S)

El puente le dice al daemon qué herramienta llama, como un valor de un conjunto cerrado (las herramientas del contrato del agente). El daemon cuenta con ese nombre y cae en `other` si no lo conoce. Sin esto, la parte A cuenta rutas, no herramientas. El contrato del agente no cambia, porque es lo que el puente le manda al daemon y no lo que el modelo ve.

### C. El reporte (Incorporar; S)

`clawg metrics sessions`, al lado de `clawg metrics report`, con `--by project|agent|day`, `--since` y salida JSON. Columnas:

- sesiones, y cuántas cerraron por cada motivo (`idle` incluido);
- duración, y tiempo entre la última memoria guardada y el cierre;
- prompts;
- llamadas por herramienta;
- saves por tipo;
- el porcentaje de sesiones que cerraron con resumen.

Con `--session ID`, una sesión entera. Lee sin daemon, como `metrics report`.

### D. El hook de cierre del cliente (Incorporar; S)

Instalar `SessionEnd` en Claude Code y `sessionEnd` en Cursor para escribir `ended_at` y `end_reason`. Da el fin real: una sesión que quedó abierta sin uso y se cerró con `/exit` dura hasta la salida, no hasta su última actividad. Agrega el motivo (`clear`, salida o `logout`) y el id exacto de la sesión que termina, también con sesiones en paralelo. No ve el equipo que se apaga ni el proceso que muere, y cerrar la ventana de la terminal está sin verificar; para esos casos queda la regla de las 6 horas. El costo:

- un hook más al salir y en cada `/clear`, dentro del presupuesto de 1,5 s;
- no puede llamar a `POST /v1/projects`, que despierta el modelo («Hallazgos del código», `openspec/README.md:151`);
- cambia `setup` (usuario, `--local` y Cursor) y sus tests;
- quien ya corrió `setup` lo vuelve a correr para tenerlo.

**Cómo se mide.** En el mismo test de binario de la parte A, un `session-end` de S con motivo `clear`: S queda cerrada con ese motivo, y un evento posterior de S abre una fila nueva.

## Qué más vale registrar, y qué no

**Vale, porque contesta preguntas del producto:**

- **Si la sesión cerró con resumen.** Es la señal directa de que el traspaso funciona, y uno de los criterios del piloto (`ROADMAP.md`, «Pasa si»).
- **Búsquedas sin ningún hit, por proyecto.** Dice dónde la memoria tiene huecos, sin guardar la consulta.
- **Llamadas por prompt.** Dice si el agente consulta a clawg o lo ignora, en uso real y no sólo en las corridas de 96.
- **Avisos de guardado** (`save_reminder`, `hook.rs`) y si siguió un save en la misma sesión. Dice si el aviso sirve.
- **El `model` de `SessionStart`**, cuando viene. Sirve para cruzar con los tokens de 122.

**No se registra:**

- **El texto de consultas, prompts o memorias, y las rutas.** El invariante de las métricas sigue valiendo para todo lo que no es un id.
- **El usuario.** clawg es local y de una persona. El `user_email` de Cursor y el de la telemetría de Claude Code son datos personales que no contestan nada que agente y proyecto no contesten.
- **Las llamadas que no llegan al daemon** (daemon caído). El puente las contesta con un error y nadie las cuenta. Es un límite que se documenta.
- **Los comandos de `clawg` por CLI.** No tienen sesión.

## Fuera de alcance

- Tokens y costo: [session-tokens](../../session-tokens/exploration.md) (122).
- Mandar algo fuera de la máquina.
- Una página o tablero: el JSON del reporte alcanza para armarla después.
- Separar los subagentes de su sesión: comparten el puente, y sus llamadas cuentan para la sesión principal.
- Vigilar el proceso del cliente para ver un cierre que el hook no ve (ver la decisión 6).

## Decisiones (2026-11-10)

1. **El registro vive en `metrics.db`** (esquema 3), con el invariante de `spec.md:209` enmendado sólo para estas tablas: llevan ids de sesión, de agente y de proyecto, nunca texto, rutas ni usuario. Por qué: perderlo no pierde ninguna memoria, no agranda los backups ni viaja en los exports, y ya tiene poda, interruptor y lector sin daemon. Se acepta que con `CLAWG_METRICS=off` tampoco se registren sesiones.
2. **Sin umbral de tiempo activo: la duración va del inicio al cierre, y 6 horas sin actividad cierran la sesión.** Lo que más importa es cuándo empieza y cuándo termina una sesión; lo que se capture en el medio se suma después, según sirva. Una sesión sin cierre se da por cerrada con 6 horas sin actividad, con su fin en la última actividad. Retomarla después ya no es la misma sesión, porque 6 horas sin uso es demasiado para seguirla, y abre una fila nueva; lo mismo después de un cierre por hook. Se guarda la hora de la última memoria para ver las sesiones que siguieron abiertas mucho después de su último save. Las 6 horas son una constante con su test.
3. **Unir dos proyectos pasa sus sesiones al que queda, y borrar un proyecto borra sus sesiones,** para que un proyecto borrado no deje rastro.
4. **D se instala en este cambio.** El fin real es lo que más importa medir, y sólo el cliente lo ve. La versión que lo trae es la misma que empieza a contar, así que volver a correr `setup` no cambia una medición en curso.
5. **El reporte es `clawg metrics sessions`,** al lado de `clawg metrics report`, que también lee sin daemon.
6. **Vigilar el proceso del cliente queda como seguimiento, fuera de este cambio.** El puente ya se entera cuando Claude Code se va (`crates/clawg-cli/src/mcp/mod.rs`, `waiting`), pero no sabe la sesión y puede morir junto con la ventana antes de avisar. Vigilar el PID desde el daemon sí ve una ventana cerrada, pero Claude Code no da su PID, Windows reutiliza los PID y esa parte no se prueba en Linux. Sólo agrega la hora real de un cierre con la X si la X no dispara `SessionEnd`. Se decide con dos datos: si la X lo dispara, que se prueba en local, y cuántas sesiones cierran con `idle` en el reporte.

## Esfuerzo e impacto

| Parte | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| A | Incorporar | Alto: es la base de todo el resto | Bajo (no toca `memory.db`) | Medio |
| B | Incorporar | Medio | Bajo | Bajo |
| C | Incorporar | Alto: sin reporte, el registro no se ve | Bajo | Bajo |
| D | Incorporar | Medio: da el fin real y su motivo | Medio (un hook más al salir) | Bajo |

Va en la nube: todo se prueba en Linux con el embedder falso y un daemon de test. Lo local es mirar el reporte después de una semana de uso real y probar si cerrar la ventana de la terminal dispara `SessionEnd`.

## Fuentes

- Código de `develop` en `5fb9a5f`, con las rutas citadas.
- https://code.claude.com/docs/en/hooks, leída el 2026-11-08: «Common input fields», «SessionStart», «SessionEnd», «Stop», «PostToolUse» y «Matcher patterns». También `statusline` («Available data») y `monitoring-usage` («Standard Attributes») para `/clear`.
- https://cursor.com/docs/agent/hooks, leída el 2026-11-08, a través de un resumen: los campos de Cursor están sin verificar uno por uno.
- El 2026-11-10: la sección «SessionEnd» entera de https://code.claude.com/docs/en/hooks.md, la lista de variables de entorno de los hooks, y `sessionStart` y `sessionEnd` de la página de Cursor.
- La exploración archivada de session-handoff (39) y la spec de métricas.
