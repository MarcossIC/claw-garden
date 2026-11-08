# session-usage — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal (medir el uso real por sesión, proyecto y agente; sirve a 1, 2 y 4 porque dice qué sesiones buscan, guardan y cierran con resumen) | M, por partes (A es M; B y C son S; D es S y opcional) | —; se coordina con la parte A de [session-lifecycle](../session-lifecycle/exploration.md) (99), que arreglaría la atribución con sesiones en paralelo; [agent-eval](../agent-eval/proposal.md) (96) cuenta lo mismo en corridas controladas y sirve de contraste | exploración (borrador) |

**Tesis.** Hoy clawg sabe a qué sesión pertenece un save, pero no sabe nada de la sesión en sí: ni cuándo empezó, ni cuándo terminó, ni cuántas veces llamó a clawg. Una sesión es sólo un texto en un mapa en memoria, uno por agente y proyecto, que se pierde al reiniciar el daemon. El uso por sesión no entra en las series de `metrics.db`: su spec prohíbe ids y etiquetas de agente, y con razón, porque abrirían series sin cota. Lo que falta es un **registro de sesiones**: una fila por sesión del cliente, con su comienzo, su última actividad y contadores, al lado de las series y no dentro de ellas. El fin de una sesión no se puede esperar del cliente, que no garantiza avisarlo. Se toma de la última actividad, y un aviso explícito sólo agrega el motivo.

## El problema verificado contra el código

1. **Una sesión es un id en un mapa en memoria.** `SessionRegistry` es un `HashMap<(agente, proyecto), id>` (`crates/clawg-cli/src/daemon/sessions.rs:33-35`). Se pierde al reiniciar el daemon (`:6-7`), y el último en registrarse gana (`:29-31`). No guarda comienzo, fin, última actividad ni contador.
2. **El id lo traen sólo los hooks.** Lo registran los dos eventos de Claude Code, en cada evento (`crates/clawg-cli/src/hook.rs:445-449`), con `POST /v1/sessions` (`crates/clawg-cli/src/daemon/api.rs:1127`, `register_session` en `:1748`). El cuerpo trae agente, proyecto e id y nada más (`SessionRequest`, `api.rs:407-412`). Cursor sólo instala `sessionStart` (`crates/clawg-cli/src/setup/cursor.rs:31`), así que registra una vez por sesión y no hay un evento por prompt.
3. **El save ya lleva la sesión.** Al aceptar un save, el daemon lee la sesión registrada para su agente y proyecto (`api.rs:1445`) y la escribe en `memory.source_session_id` (`crates/clawg-core/migrations/V1__init.sql:16`). `clawg memories --session` filtra por ella (`crates/clawg-cli/src/cli.rs:598-600`). **Las memorias por sesión ya se pueden contar hoy**, con dos límites: con dos sesiones del mismo agente en el mismo proyecto, el save sale con la sesión de la última que habló, y después de un reinicio sale sin sesión hasta el próximo hook.
4. **El puente no sabe la sesión ni el nombre de la herramienta que llama.** Cada herramienta pasa por `ClawgMcp::call` (`crates/clawg-cli/src/mcp/server.rs:747`), con el agente resuelto por `agent_for` (`:712`). Llega al daemon como una petición HTTP con agente y proyecto en el cuerpo y un bearer, sin el nombre de la herramienta. El daemon ve la ruta, y varias herramientas comparten ruta. El puente no avisa al daemon cuando arranca ni cuando termina.
5. **`metrics.db` mide el daemon, no al cliente.** Una `run` es una vida del daemon (`crates/clawg-core/src/metrics/store.rs:384`), y las series se agrupan por minuto y por hora (`:398-419`). Las peticiones se etiquetan por ruta, método y estado ([`specs/metrics`](../../specs/metrics/spec.md) R5). La spec prohíbe que una etiqueta o una fila lleve un id (`spec.md:209`) y deja fuera la etiqueta de agente porque «es autodeclarado y abriría series sin cota» (`spec.md:217`). Los minutos se guardan 14 días y las horas y los eventos 400 (`store.rs:26`, `:29`). `clawg metrics report` agrupa por día, hora, minuto o versión, nunca por proyecto, agente o sesión.
6. **Ningún hook ve el fin de la sesión.** `setup` instala sólo `SessionStart` y `UserPromptSubmit` (`crates/clawg-cli/src/setup.rs:51`), y `HookEvent` no tiene más variantes (`cli.rs:915-918`). `SessionEnd` y `Stop` no están.
7. **El registro de hooks no sirve para esto.** `hook-runs.jsonl` guarda sólo las corridas de `session-start`, en un anillo de dos archivos de 64 KiB (`crates/clawg-cli/src/hook_log.rs:20-22`). No guarda el id de sesión: de la entrada guarda sólo los nombres de las claves (`input_keys`, `:77`, `:90`).
8. **Lo que ya cuenta por agente y proyecto es lo guardado, no la actividad.** `GET /v1/stats` cuenta filas, con filtro de agente y proyecto (`api.rs:1140`, `stats` en `:1810`). `clawg doctor` lista filas por agente.

**Lo que ya se descartó y por qué se reabre.** session-handoff descartó una tabla `session` (id, agente, proyecto, inicio, fin, resumen) porque lo que necesitaba entonces, el último resumen y las memorias de una sesión, salía de `memory` con la columna. Agregaba además un fin de sesión que el cliente no garantiza ([exploración](../archive/2026-10-19-session-handoff/exploration.md), líneas 72 y 102). El dato nuevo es la pregunta: cuánto se usa clawg, cuánto dura una sesión y cuántas llamadas hace. La columna no puede contestarlo, porque una sesión que llama 40 veces y no guarda nada no deja ninguna fila. El argumento del fin sigue en pie, y por eso este cambio no depende de él: el fin es la última actividad (parte A), y el aviso del cliente sólo es un agregado (parte D).

## Qué da el cliente (documentación leída el 2026-11-08)

Fuente: https://code.claude.com/docs/en/hooks, salvo donde se nombra otra página.

| Dato | Claude Code | Cursor |
|---|---|---|
| Id de sesión | `session_id` en todo hook («Common input fields») | `session_id` en `sessionStart` y `sessionEnd`, y `conversation_id` en todos ([cursor.com/docs/agent/hooks](https://cursor.com/docs/agent/hooks)). La relación entre los dos está sin verificar |
| Cómo empezó | `source` en `SessionStart`: `startup`, `resume`, `clear`, `compact` o `fork`; a veces `model` | `composer_mode`, `is_background_agent` |
| `/clear` | Empieza una sesión nueva: `clear` es a la vez `source` de `SessionStart` y `reason` de `SessionEnd`. Que el id cambie no lo dice la página de hooks con esas palabras, pero sí las de `statusline` y `monitoring-usage` | — |
| `resume` | Si conserva el id: **sin verificar** | — |
| Fin | `SessionEnd` con `reason` (`clear`, `resume`, `logout`, `prompt_input_exit`, `other`). Tiene un presupuesto de 1,5 s, no puede bloquear y la página no dice que corra si el proceso muere o se cierra la terminal | `sessionEnd` con `reason`, `duration_ms` y `final_status` (según el resumen de la página; sin verificar campo por campo) |
| Cada llamada a una herramienta MCP | `PostToolUse` con matcher `mcp__clawg__.*`: `session_id`, `tool_name` y `duration_ms` | `afterMCPExecution`: `tool_name`, `mcp_server_name` y `duration` |
| Tokens | No hay en ningún hook. Ver [session-tokens](../session-tokens/exploration.md) (122) | No hay |

## Partes

### A. El registro de sesiones (Incorporar; M)

Es una fila por sesión del cliente:

- **Identidad:** id del cliente, agente, proyecto (puede faltar) y cliente (`claude-code`, `cursor`).
- **Tiempos:** `started_at` (la primera vez que el daemon ve el id), `last_seen_at` (el último evento o llamada atribuida) y `ended_at` con `end_reason`, sólo cuando un aviso lo dice.
- **Cómo empezó y cuántas veces volvió:** contadores por `source` (`startup`, `resume`, `clear`, `compact`). Las compactaciones por sesión dicen cuánto se llena el contexto, que es lo que la parte B de 99 quiere saber antes de avisar.
- **Contadores:**
  - prompts (un `user-prompt-submit` es un prompt, y su id es exacto);
  - llamadas por herramienta, con cuántas fallaron;
  - saves aceptados por `memory_type`;
  - búsquedas sin ningún hit;
  - si guardó un `session_summary`.
- **Tiempo activo:** la suma de los intervalos entre eventos seguidos que no pasan de un umbral (decisión abierta, propuesto 15 minutos). Una sesión que quedó abierta toda la noche dura lo que se trabajó, no 14 horas. Con el tiempo activo, retomar una sesión vieja tampoco infla la cuenta, aunque `resume` conserve el id.

**El fin es `last_seen_at`.** Un reinicio del daemon no cierra nada: si el mismo id vuelve a registrarse, sigue la misma fila. El registro en memoria de hoy no puede hacerlo.

**La atribución de una llamada** usa el mismo registro que ya sella los saves: la sesión vigente de su agente y proyecto en ese momento (`api.rs:1445`). Tiene el mismo límite, con dos sesiones en paralelo del mismo agente y proyecto. Lo arregla la parte A de 99, que pone el checkout en la clave, y entonces se arregla para los saves y para las llamadas a la vez. Lo exacto sería `PostToolUse`, con el id en cada llamada, pero es un proceso más por cada llamada a clawg y otro nombre de herramienta si clawg se distribuye como plugin (`mcp__plugin_clawg_clawg__*`, [claude-code-plugin](../claude-code-plugin/exploration.md)). Queda como alternativa si la medición muestra que las sesiones en paralelo pesan.

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

### B. El nombre de la herramienta en cada llamada (Incorporar; S)

El puente le dice al daemon qué herramienta llama, como un valor de un conjunto cerrado (las herramientas del contrato del agente). El daemon cuenta con ese nombre y cae en `other` si no lo conoce. Sin esto, la parte A cuenta rutas, no herramientas. El contrato del agente no cambia, porque es lo que el puente le manda al daemon y no lo que el modelo ve.

### C. El reporte (Incorporar; S)

Un subcomando de `clawg metrics` (nombre por decidir; propuesto `clawg metrics sessions`) con `--by project|agent|day`, `--since` y salida JSON. Columnas:

- sesiones;
- tiempo activo y tiempo total;
- prompts;
- llamadas por herramienta;
- saves por tipo;
- el porcentaje de sesiones que cerraron con resumen.

Con `--session ID`, una sesión entera. Lee sin daemon, como `metrics report`.

### D. El aviso de fin del cliente (Opcional; S)

Instalar `SessionEnd` en Claude Code y `sessionEnd` en Cursor sólo para escribir `ended_at` y `end_reason`. No cambia la duración, que ya sale de `last_seen_at`. Agrega el motivo (`clear`, salida o `logout`) y el id exacto de la sesión que termina, también con sesiones en paralelo. El costo:

- un hook más al salir y en cada `/clear`, dentro del presupuesto de 1,5 s;
- no puede llamar a `POST /v1/projects`, que despierta el modelo («Hallazgos del código», `openspec/README.md:151`);
- cambia `setup` (usuario, `--local` y Cursor) y sus tests.

Recomendado: esperar a ver el reporte de A. Si el motivo no cambia ninguna decisión, no se instala.

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

- Tokens y costo: [session-tokens](../session-tokens/exploration.md) (122).
- Mandar algo fuera de la máquina.
- Una página o tablero: el JSON del reporte alcanza para armarla después.
- Separar los subagentes de su sesión: comparten el puente, y sus llamadas cuentan para la sesión principal.

## Decisiones abiertas

- **Dónde vive el registro.** Recomendado `metrics.db`, con el invariante enmendado.
- **El umbral del tiempo activo.** Propuesto 15 minutos, como constante con su test.
- **Qué pasa con las sesiones de un proyecto que se borra o se une a otro.** Recomendado: una unión las pasa al proyecto que queda, y un borrado las borra, para que un proyecto borrado no deje rastro.
- **Si se instala D.** Recomendado esperar al reporte.

## Esfuerzo e impacto

| Parte | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| A | Incorporar | Alto: es la base de todo el resto | Bajo (no toca `memory.db`) | Medio |
| B | Incorporar | Medio | Bajo | Bajo |
| C | Incorporar | Alto: sin reporte, el registro no se ve | Bajo | Bajo |
| D | Opcional | Bajo | Medio (un hook más al salir) | Bajo |

Va en la nube: todo se prueba en Linux con el embedder falso y un daemon de test. Lo único local es mirar el reporte después de una semana de uso real.

## Fuentes

- Código de `develop` en `5fb9a5f`, con las rutas citadas.
- https://code.claude.com/docs/en/hooks, leída el 2026-11-08: «Common input fields», «SessionStart», «SessionEnd», «Stop», «PostToolUse» y «Matcher patterns». También `statusline` («Available data») y `monitoring-usage` («Standard Attributes») para `/clear`.
- https://cursor.com/docs/agent/hooks, leída el 2026-11-08, a través de un resumen: los campos de Cursor están sin verificar uno por uno.
- La exploración archivada de session-handoff (39) y la spec de métricas.
