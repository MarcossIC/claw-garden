# session-lifecycle — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 4 | M, por partes (A es S–M; B y C son investigaciones S) | el paso 1 de [memory-anchors](../memory-anchors/exploration.md) (98), que registra el checkout de cada save, para la parte A; [agent-eval](../agent-eval/exploration.md) (96) para medir B, C y D | exploración (borrador) |

**Tesis:** el ciclo «guardar un traspaso, limpiar la sesión y retomarlo» que proponen las investigaciones ya existe casi entero en clawg (`session_summary` con secciones fijas, LAST SESSION en cada contexto, `session-start` en `startup|resume|clear|compact`). Lo que falla es el caso que esas mismas investigaciones recomiendan como flujo diario: **varias sesiones en paralelo, una por worktree, del mismo proyecto.** Ahí cada sesión arranca con el traspaso de la otra y sus saves pueden quedar sellados con la sesión equivocada. Este cambio arregla eso (parte A) y deja medidas, no implementadas, tres ideas de las investigaciones sobre el ciclo de vida: avisar cuando el contexto se llena (B), guardar el resumen que hace el cliente al compactar (C) y darle contexto a los subagentes (D).

**De dónde sale.** De dos documentos de investigación previos, que no están en el repositorio: un diseño de integración con Claude Code («limpiar en vez de compactar», el esquema del traspaso, la señal de umbral en `UserPromptSubmit`, `PreCompact`, `Stop`) y un informe de estado del arte (la tabla de hooks recomendados, `SubagentStart`, una tarea por worktree). Se contrastan con `develop` en 0.68.0 (`4604b35`) y con la referencia de hooks de Claude Code (https://code.claude.com/docs/en/hooks, leída el 2026-10-31). Retoma lo que [agent-lifecycle-hooks](../archive/2026-10-18-agent-lifecycle-hooks/exploration.md) (18) y [session-handoff](../archive/2026-10-19-session-handoff/exploration.md) (39), archivados, ya decidieron, para no reabrirlo sin un dato nuevo.

## Premisas de la investigación, verificadas contra 0.68.0

| Premisa | Resultado |
|---|---|
| Hace falta un traspaso estructurado (objetivo, hecho, siguiente, archivos, pendiente, validar) y una herramienta `claw_handoff` | **Ya existe sin herramienta nueva.** El protocolo pide un `session_summary` con Goal, Instructions, Discoveries, Accomplished, Next steps y Relevant files (`crates/clawg-cli/src/mcp/protocol.md:115-131`), y todo contexto abre con el más nuevo del proyecto bajo `=== LAST SESSION ===`, entero aunque sea largo (`LAST_SESSION_SQL`, `crates/clawg-core/src/memory/prompt_builder.rs:905-919`; `with_last_session`, `:920`). Falta un campo de la investigación: **el comando exacto para validar**, que el resumen hoy no pide |
| La memoria duradera y el estado de trabajo tienen que separarse | **Ya están separados.** El protocolo prohíbe guardar «the state of the task at hand» como memoria (`protocol.md:20-21`, `:52`); un `session_summary` decae el doble de rápido que un hecho y nunca entra en un conflicto (`protocol.md:100-101`; `session_summary` 0,0077 en `crates/clawg-core/src/memory/score.rs:132`) |
| `SessionStart` con `compact` reinyecta tras compactar | **Ya existe.** `setup` instala el grupo de `startup`, `resume`, `clear` y `compact` y el hook dice cuándo la fuente fue `compact` (`crates/clawg-cli/src/hook.rs:254-255`) |
| `PreCompact` captura el estado y lo reinyecta | **No puede.** `PreCompact` recibe `trigger` y `custom_instructions` y sólo puede bloquear; `systemMessage` y `continue` se descartan (página de hooks, «PreCompact»). 18 ya lo descartó como punto de inyección (`agent-lifecycle-hooks/exploration.md:139`). Lo que sí trae dato nuevo es `PostCompact`, con `compact_summary` (parte C) |
| Un hook `UserPromptSubmit` mide el transcript y avisa pasado el ~50 % | **Posible, pero sin campo documentado.** La entrada de `UserPromptSubmit` no trae uso de contexto (página de hooks, «UserPromptSubmit»). La página define `context_tokens` (entrada, caché leída, caché escrita y salida de la última respuesta) sólo para `SessionStart` al reanudar y para `PreModelSwitch`. Un hook podría sumarlo de la última respuesta del transcript, cuyo formato el cliente no documenta; por eso 18 sacó de él sólo la fecha de creación del archivo (`agent-lifecycle-hooks/design.md:39`). Parte B |
| `Stop` como puerta: bloquear si hubo ediciones sin una validación en verde | **Fuera del producto.** `Stop` puede bloquear y agregar `additionalContext` (página de hooks, «Stop», con `stop_hook_active` para no entrar en bucle), pero decidir si «hubo una validación en verde» no es memoria: es un registro de comandos que clawg no tiene ni debería tener. Se descarta; queda en «Ideas evaluadas» |
| `SessionEnd` cierra el episodio | **Sigue descartado:** comparte un presupuesto de 1,5 s (página de hooks, «SessionEnd») y no corre siempre (`session-handoff/exploration.md:102`) |
| `SubagentStart` inyecta al subagente sólo lo de su tarea | **Posible.** Acepta `hookSpecificOutput.additionalContext`, agregado antes del primer prompt del subagente (página de hooks, «SubagentStart»). La memoria automática del cliente no carga en los subagentes (https://code.claude.com/docs/en/memory, «Auto memory»), y el `session-start` de clawg tampoco les llega. Parte D |

## El hallazgo: sesiones en paralelo en un mismo proyecto

Un worktree resuelve a la raíz del repositorio principal (`root_from_marker`, `crates/clawg-core/src/memory/identity.rs:144-159`), así que dos sesiones en dos worktrees del mismo repositorio son **el mismo proyecto**. Eso es lo que permite compartir memorias, y está bien. Pero:

1. **LAST SESSION es el más nuevo del proyecto, de cualquier checkout.** `LAST_SESSION_SQL` filtra por tipo, estado y `project_id` (`prompt_builder.rs:911-919`). Si la sesión del worktree `feat-a` guarda su resumen y después arranca la de `feat-b`, `feat-b` abre con el traspaso de `feat-a` como si fuera el suyo.
2. **El sello de sesión de un save puede ser de la otra sesión.** El registro guarda una sesión por `(agente, proyecto)` y «the last to register wins» (`crates/clawg-cli/src/daemon/sessions.rs:29-35`). Como los dos hooks registran en cada prompt, un save de `feat-a` hecho después de un prompt en `feat-b` sale con el `source_session_id` de `feat-b`.
3. **RECENT mezcla los dos.** Lista los saves propios de los últimos 7 días del proyecto, sin distinguir checkout ([`specs/retrieval`](../../specs/retrieval/spec.md)).

El punto 2 está documentado como límite en el código. Los puntos 1 y 2 juntos rompen el traspaso justo en el flujo que las investigaciones recomiendan (una tarea por worktree, `claude --worktree`).

## Partes

### A. El traspaso por checkout (Incorporar; S–M)

- El registro de sesiones pasa a tener por clave `(agente, proyecto, cima del checkout)`: el hook ya manda `cwd` y el puente conoce el suyo (`ids_for`, `crates/clawg-cli/src/mcp/server.rs:916`). La cima del checkout es la que ya mide `subpath_of` para las subcarpetas declaradas (`crates/clawg-core/src/memory/identity.rs:105`; `Ground::subpath`, `:378-380`).
- Cada save registra su checkout (el paso 1 de 98, `memory_origin`), y LAST SESSION prefiere el resumen más nuevo **del mismo checkout**; sin ninguno, el del proyecto, como hoy. Un repositorio con un solo checkout no cambia nada.
- **Cómo se mide:** un test de binario con dos worktrees de un repositorio, dos sesiones que se alternan y un resumen en cada una: hoy cada una abre con el resumen de la otra y su save sale con la sesión ajena; después, cada una con lo suyo.
- **Riesgo:** un worktree borrado deja su resumen sin checkout vivo; cae al resumen del proyecto, que es lo de hoy.

### B. Avisar antes de que el contexto se llene (Investigar; S)

La idea es buena para el protocolo: el resumen se pide «before the context is compacted» (`protocol.md:117`), pero el agente no sabe cuándo va a pasar. Dos caminos:

| Camino | A favor | En contra |
|---|---|---|
| `user-prompt-submit` lee la última respuesta del transcript y suma su uso; pasado un umbral, una línea pide guardar el `session_summary` | Determinista, sin LLM, en el hook que ya corre en cada prompt | El formato del transcript no está documentado; el tamaño de la ventana depende del modelo y de `/autocompact`; leer el final de un archivo grande dentro del plazo de 3 s del hook (`HOOK_DEADLINE`, `hook.rs:26`) |
| Una línea fija en el protocolo («antes de un `/clear` o cuando el usuario cierre la tarea, guardá el resumen») | Sin código | Depende de que el modelo se acuerde |

Se decide con 96: cuántas sesiones del piloto llegan a compactar sin haber guardado un resumen. Si son pocas, no hace falta el aviso.

### C. Guardar `compact_summary` desde `PostCompact` (Investigar; S)

Es la opción A5 que 18 dejó abierta como decisión del usuario (`agent-lifecycle-hooks/exploration.md:122-128`). Lo nuevo es que la página la sigue confirmando y que el ingest ya tiene todo lo que pedía: enmascarado de secretos antes de guardar, tope de 64 KiB (`MAX_CONTENT_BYTES`), sello de procedencia y el outbox. Sigue en contra lo mismo: es texto que el modelo no escribió pensando en la memoria y una escritura que el usuario no pidió. Recomendado: sólo detrás de una preferencia explícita, y con 96 comparando un resumen del cliente con uno del agente en las tareas reanudadas.

### D. Contexto para los subagentes (Posponer)

Un hook `SubagentStart` podría darle al subagente el workspace y las preferencias del proyecto. Antes hay que saber si los subagentes necesitan memoria (la mayoría explora y resume) y cuánto cuesta: 96 puede contar en el piloto cuántas veces un subagente llama a `search_memory` o `search_project`.

### E. El comando de validación en el resumen (Incorporar en el protocolo; S)

Sumar una línea «Validation: the exact command that checks the work» a las secciones del `session_summary` (`protocol.md:117-125`, y su par en `protocol.es.md`). No cambia código ni datos; cambia el contrato que fija `agent_contract.json` sólo si la línea entra en la apertura, y no entra. Se verifica con los tests que comparan los dos protocolos y con una tarea reanudada de 96.

## Descartado

- **Una herramienta `claw_handoff(save|load)` y un esquema validado de 400 tokens.** `save_memory` con `memory_type = session_summary` y LAST SESSION ya hacen el `save` y el `load`. Validar campos obligatorios pide parsear el texto de un save, cosa que clawg no hace con ningún tipo, y suma una herramienta al contrato del agente.
- **Ejecutar `/clear` desde clawg.** Ningún hook puede (lo dice la propia investigación).
- **Bloquear en `PreCompact` o en `Stop` para forzar un guardado.** Ya descartado en 18 por interferir con lo que el usuario pidió; la página no agrega nada que lo cambie.

## Decisiones abiertas

- **Si A espera al paso 1 de 98** o registra el checkout por su cuenta. Recomendado esperar: es la misma tabla y un solo test de preservación.
- **El umbral de B**, sólo si 96 muestra que hace falta.
- **Si C se ofrece** y bajo qué preferencia.

## Esfuerzo e impacto

| Parte | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| A | Incorporar | Alto con worktrees en paralelo; nulo con un solo checkout | Bajo | Medio |
| B | Investigar | Medio | Medio (formato no documentado) | Bajo |
| C | Investigar | Medio | Medio (escritura no pedida) | Bajo |
| D | Posponer | Bajo hasta medirlo | Bajo | Bajo |
| E | Incorporar | Bajo a Medio | Bajo | Bajo |

## Fuentes

- Código y specs de `develop` en `4604b35` (0.68.0), con las rutas citadas.
- https://code.claude.com/docs/en/hooks, leída el 2026-10-31: secciones «UserPromptSubmit», «Stop», «SubagentStart», «PreCompact», «PostCompact», «SessionEnd» y los campos `context_tokens` de `SessionStart` y `PreModelSwitch`.
- https://code.claude.com/docs/en/memory, sección «Auto memory», leída el 2026-10-31: la memoria automática no carga en los subagentes.
- Las exploraciones archivadas de 18 y 39.
