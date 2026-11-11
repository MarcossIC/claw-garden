# memory-delivery — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| entrega y medición (1, 2): qué memoria le llega al agente, por qué canal y cuánto contexto cuesta | M–L, por pasos y con puerta: el paso 1 es del arnés y no toca el producto | [agent-eval](../agent-eval/proposal.md) (96), del que toma el arnés y el piloto pago; se coordina con [claude-code-plugin](../claude-code-plugin/exploration.md) (120), que espera esta medición para decidir el bloque del protocolo, y con [memory-anchors](../memory-anchors/exploration.md) (98), que avisa cuando una memoria nombra algo que ya no existe | exploración cerrada; el paso 1 sigue en la [propuesta](proposal.md) |

**Tesis.** En el piloto pago de agent-eval, la memoria nativa de Claude Code resolvió las tres tareas K (6 de 6) y clawg ninguna (0 de 6). clawg no buscó mal: su memoria nunca llegó al agente. El canal que empuja, el hook de arranque, sólo toma filas que las memorias sembradas no cumplían, y el agente no tiró de ella buscando. Mientras tanto clawg paga unos 4.400 tokens fijos por sesión. Este cambio decide qué le llega al agente, por cuál de los canales que el cliente ya tiene y con cuánto contexto, midiéndolo con brazos de agent-eval, y construye en el producto sólo lo que gane. La meta es superar a la memoria nativa con menos contexto, no igualarla.

## Lo que mostró el piloto

Sonnet 5.5, esfuerzo `high`, Claude Code 2.1.322, dos corridas por tarea y brazo, en inglés. Cada tarea siembra de una a dos memorias de entre 30 y 145 días, con importancia 5 y autor `agent`.

| Tarea | `a0` sin memoria | `a3` clawg | `a3-eager` | `a4` nativa |
|---|---|---|---|---|
| K1 + K2 + K3 | 0/6 | 0/6 | 0/6 | 6/6 |
| S (una memoria vieja) | 2/2 | 2/2 | 2/2 | 0/2 |
| C (dos memorias que se contradicen) | 2/2 | 2/2 | 2/2 | 2/2 |
| N (el control) | 2/2 | 2/2 | 2/2 | 2/2 |
| Sesiones que llamaron a clawg | — | 0 de 12 | 4 de 12 | — |
| Costo medio por sesión (USD) | 0,020 | 0,033 | 0,046 | 0,026 |

- **El arranque no mostró nada.** En las 24 sesiones con clawg, los hooks `session-start` y `user-prompt-submit` corrieron, salieron con 0 y no imprimieron nada; el registro de hooks dice `empty_context` y 0 bytes en los 24 arranques. El arranque sólo toma las preferencias que dijo la persona (`created_by_type = 'user'`) con importancia 8 o más y en tier 1 (`AUTO_PREFERENCES_SQL`, `crates/clawg-core/src/memory/workspace.rs:341-354`), las decisiones con importancia 8 o más y en tier 1 (`:407-418`) y lo de los últimos 7 días.
- **La nativa no pide nada.** Ninguna sesión `a4` leyó `MEMORY.md` con una herramienta, y todas siguieron lo que decía: el cliente lo pone en el contexto de entrada.
- **Cuando clawg buscó, encontró.** En C, `a3-eager` trajo las dos decisiones, avisó que se contradicen y eligió la vigente. En K3 la decisión salió primera con `score=0.00` para una consulta sobre los bugs, y el agente la ignoró.
- **Empujar todo tiene un costo.** En S, la nativa siguió una decisión de hace 145 días que el script del repositorio ya contradice, y falló las dos veces.

## Dónde se van los tokens

La mediana del primer pedido de cada sesión (entrada, caché leída y caché escrita del primer mensaje del agente), sobre 12 sesiones por brazo:

| Brazo | Tokens | Sobre `a0` |
|---|---|---|
| `a0` | 16.056 | — |
| `a4` (1 o 2 memorias en `MEMORY.md`) | 17.059 | +1.003 |
| `a3` | 20.469 | +4.413 |
| `a3-eager` | 38.608 | +22.552 |

Lo que pone clawg, en bytes, en `a3`:

- **El bloque del protocolo**, 10.763 bytes en el `CLAUDE.md` de la corrida. Lo escribe `setup` a partir de `crates/clawg-cli/src/mcp/protocol.md` (10.891 bytes; la versión en español, `protocol.es.md`, 11.654). Es la parte más grande.
- **Las instrucciones del servidor**, 2.030 bytes. Son la apertura del mismo protocolo (`opening`, `crates/clawg-cli/src/mcp/protocol.rs:33`), así que el agente las lee dos veces. `the_instructions_fit_older_clients` (`crates/clawg-cli/src/mcp/server.rs:2950`) las acota a 2.048 caracteres por los clientes anteriores a v2.1.296.
- **Las 13 herramientas**, diferidas detrás de `ToolSearch`: en el contexto de entrada va sólo su nombre. Sus definiciones completas suman 19.271 bytes (`crates/clawg-cli/src/mcp/agent_contract.json`); las más grandes son `save_memory` (3.649), `update_memory` (2.405) y `search_memory` (2.233).

`a3-eager` pone `ENABLE_TOOL_SEARCH=false` (`crates/clawg-eval/src/bin/agent-eval/arm.rs:263`), y eso es global: carga de entrada también las herramientas diferidas del propio cliente. Las sesiones `a3` y `a3-eager` ven los mismos 23 o 24 nombres de herramientas del cliente, así que la mayor parte de los +18.139 tokens de `a3-eager` sobre `a3` no son de clawg. La primera sesión `a3-eager` escribió 39.058 tokens de caché y las siguientes los leyeron de ahí.

## Por qué buscar en cada prompt no alcanza

Medido sin sesiones pagas, sobre una copia del store de una corrida `a3` con las cinco siembras cargadas. Se buscó con `search_memory`, por el puente MCP que usa el agente, con el prompt de cada tarea como consulta y `limit` 3. K1, K2 y K3 comparten fixture, así que quedaron en un mismo proyecto y cada una hizo de ruido para las otras:

| Tarea | Su memoria | Lo que devolvió la búsqueda |
|---|---|---|
| K1 | la preferencia de `_n` | primera con 0,13; las de K2 y K3 con 0,10 |
| K2 | la convención de `import secrets` (tier 3) | no salió; salieron la de K1 (0,14) y la de K3 (0,05) |
| K3 | la decisión del asterisco | segunda con 0,10, detrás de la preferencia de K1 (0,17) |
| S | la decisión vieja | 0,17 |
| C | la decisión vigente y la vieja | 0,35 y 0,14 |

El puntaje es absoluto: el coseno reescalado contra un piso del corpus, por la antigüedad, la importancia y el estado (`crates/clawg-core/src/memory/score.rs`). Una regla que vale para todo el código se parece al prompt de una tarea lo mismo que una memoria cualquiera. Un umbral alto en `user-prompt-submit`, la idea de las «Ideas evaluadas» del [README](../../README.md), no inyectaría nada en las K. Sin umbral, en el store del piloto, de una a tres memorias por proyecto, inyectaría todo, y eso no se sostiene en un store real.

## Los canales que el cliente ya tiene

Leídos en https://code.claude.com/docs/en/memory y https://code.claude.com/docs/en/hooks el 2026-11-11, y en https://code.claude.com/docs/en/mcp según las fuentes de la [exploración de agent-eval](../agent-eval/exploration.md#fuentes), leída el 2026-10-31:

| Canal | Cuándo entra | Lo usa clawg |
|---|---|---|
| `additionalContext` de `SessionStart` | al arrancar, compactar o retomar | sí, con un tope de 8.000 bytes (`DEFAULT_BUDGET`, `crates/clawg-core/src/memory/prompt_builder.rs:63`; `SESSION_BUDGET`, `crates/clawg-cli/src/hook.rs:65`) |
| `.claude/rules/*.md` sin `paths:` | al arrancar, con la prioridad de `.claude/CLAUDE.md` | no |
| `.claude/rules/*.md` con `paths:` (globs) | cuando el agente usa Read, Write o Edit sobre un archivo que coincide | no |
| `~/.claude/rules/` | igual, en todos los proyectos de la máquina | no |
| `additionalContext` de `PreToolUse` | con el resultado de la herramienta, no antes de que corra | no |
| `anthropic/alwaysLoad` en una herramienta MCP | carga esa herramienta de entrada y deja diferidas las demás (v2.1.285 o posterior) | no |
| `MEMORY.md` de la memoria automática | las primeras 200 líneas o 25 KB, al arrancar | es `a4` |

El evento `InstructionsLoaded` avisa cada archivo de instrucciones que entra, con `load_reason` (`session_start`, `path_glob_match`, …) y `trigger_file_path`, así que el arnés puede medir cuándo llegó una regla por ruta.

## Las palancas

- **A. La dieta del protocolo.** Sacar el bloque de `CLAUDE.md`, o dejarlo en lo que ninguna otra parte dice. La apertura ya llega como instrucciones del servidor, y las reglas que sólo importan ante un resultado (un conflicto, un proyecto ambiguo) ya vienen en el resultado de la herramienta que lo encuentra. Ahorra hasta unos 2.700 tokens por sesión. La exploración de 120 ya planteaba sacarlo si la medición no ve diferencia, y en el piloto el bloque completo no hizo buscar a `a3` en ninguna sesión.
- **B. Las herramientas centrales de entrada.** `anthropic/alwaysLoad` para `search_memory` y `save_memory`, con descripciones más cortas, y las demás diferidas. Cuesta unos 1.500 tokens y no carga las herramientas del cliente, a diferencia de `a3-eager`.
- **C. Las reglas al arrancar.** Las reglas vigentes del proyecto y las que no tienen proyecto, una línea cada una con su id y su antigüedad, dentro del tope del arranque. Una reemplazada no entra; una disputada entra marcada. Veinte reglas en una línea son unos 800 tokens. En producción, el 2026-11-03, 18 preferencias enteras ocupaban 14.400 bytes: el problema es mandar el texto entero, no la cantidad.
- **D. Las reglas por archivo.** clawg escribe `.claude/rules/clawg/<tema>.md` con `paths:`, por ejemplo `**/*.md` para el marcador de listas, `.gitignore` o `requirements*.txt`, y el cliente las carga sólo cuando el agente toca esos archivos. El archivo queda fuera de git por `.git/info/exclude`, así que no cambia el repositorio. Las reglas sin proyecto van en `~/.claude/rules/clawg/`. Fuera de esos archivos no agregan nada.
- **E. Qué memoria es una regla.** Es la parte difícil. La nativa lo resuelve porque el agente elige qué escribe en `MEMORY.md`. Las opciones son por tipo (preferencias y decisiones; deja afuera una convención guardada como hecho, como la de K2) o una marca explícita al guardar (`siempre` o unos globs), que es un cambio de esquema con su migración y su vuelta atrás.

## El riesgo: empujar una regla vieja

Empujar reglas repite el fallo de la nativa en S si la regla está vieja y nada lo dice. Las defensas que clawg tiene y la nativa no: una fila reemplazada no entra (`status <> 'superseded'`, `workspace.rs:352`), cada línea puede decir su antigüedad, y 98 puede avisar cuando la memoria nombra algo que ya no existe. Lo mide S, que tiene que quedar por encima de `a4` para que un brazo gane.

## El experimento

**Paso 1, en el arnés.** Sin tocar el producto:
- C y D se simulan desde la siembra: el brazo escribe el texto del arranque y los archivos de reglas. Así se miden el canal y el formato antes de construir la selección de E.
- `SeedMemory` suma `user_stated` (`crates/clawg-eval/src/bin/agent-eval/task.rs:46-62`), y una tarea puede marcar una memoria como regla, con sus globs.
- Cada brazo, también `a4` en su `MEMORY.md`, siembra un conjunto de reglas de otros temas que ningún check toca, para medir el ruido.
- En `a4`, un trazador cuenta como traído, porque `MEMORY.md` está en el contexto de entrada.
- Se cuentan los eventos `InstructionsLoaded` y los tokens del primer pedido.

Las seis tareas, dos corridas, unas 60 sesiones y unos 2 USD:

| Brazo | Qué tiene |
|---|---|
| `a4` | la nativa, la referencia |
| `a3` | clawg como hoy |
| `l0` | `a3` sin el bloque del protocolo (A) |
| `l1` | `l0` más las reglas al arrancar (C) |
| `l2` | `l1` más las reglas por archivo (D) |

**La regla de decisión, fijada antes de correr.** Un brazo gana si:
- resuelve en K al menos lo que resuelve `a4`;
- resuelve S más veces que `a4`;
- no empeora C ni N frente a `a0`;
- y su primer pedido no pasa del de `a3`.

Entre los que ganan, el de menos tokens. Con dos corridas, una diferencia de una o dos sesiones no es un resultado: los dos mejores repiten con cuatro corridas. B se mide en una segunda tanda, sobre el ganador, porque cambia la lista de herramientas del servidor.

**Paso 2, en el producto.** Se construye lo que ganó: la selección de E, la entrega de C o D y la dieta de A, cada una con su spec y sus tests.

**Paso 3.** El build del producto repite la campaña del ganador contra `a4`.

## Preguntas abiertas

- Si un `CLAUDE_CONFIG_DIR` propio lee `rules/` como reglas de usuario. Se ve en la primera corrida sin pago.
- Si una regla por archivo que llega con un `Write`, cuando el archivo es nuevo, hace que el agente lo corrija. Es el caso de K1 y K3, que escriben un archivo sin leer ninguno. Lo mide `l2`.
- Cuántos tokens agregan los resultados de las herramientas de clawg en cada llamada: `save_memory`, por ejemplo, devuelve las memorias cercanas con el pedido de juzgarlas.
- Cursor tiene su propio canal de reglas por archivo. Queda fuera de esta medición, pero D se traslada a él.

## Fuentes

- El piloto pago de agent-eval del 2026-11-11 y sus streams, puntuados con el arnés de `36d93188`.
- https://code.claude.com/docs/en/memory («Path-specific rules», «Auto memory») y https://code.claude.com/docs/en/hooks («PreToolUse decision control», «InstructionsLoaded»), leídas el 2026-11-11.
- Código de `develop` en `36d93188`, con las rutas y líneas citadas en cada punto.
