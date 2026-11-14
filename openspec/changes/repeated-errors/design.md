# repeated-errors — Diseño

Las decisiones de fondo están en la [propuesta](proposal.md#decisiones) y no se reabren. Acá van las de implementación, con lo que se descartó. Las citas son `ruta:línea` de `develop` en `8a2f3723`.

## D1. Un módulo nuevo, `tool_failures`, con el núcleo puro y la E/S separados

`crates/clawg-cli/src/tool_failures.rs`, hermano de `hook_log.rs`, en tres partes.

- **Puro:**
  - `missing_programs(text) -> Vec<&str>`: la firma, línea por línea (RE2).
  - `command_words(command) -> Vec<&str>`: el corte de la definición.
  - `observe(tool, command, outputs, event) -> Vec<Observation>`: fallos y éxitos.
  - `Line` (serde), con `parse_lines(bytes)` y `line(&Line)`.
  - `rules(lines, now_ms, on_path) -> Vec<Rule>` (RE5).
  - `to_write(observations, lines) -> Vec<Line>`: un `ok` sólo si la línea más nueva de su programa es `fail`.
- **E/S:**
  - `read(dir) -> Vec<Line>`: los dos archivos, el rotado primero.
  - `append(dir, &[Line])`.
  - `on_path(program) -> bool`, con `std::env::var_os("PATH")`, `PATHEXT` en Windows y `doctor::analyze`.
- **Texto:** `same_turn_line(lang, programs)` y `rules_block(lang, rules, now_ms)`, puros y en los dos idiomas, una `match` sobre `Lang` por frase, como `language_line` (`crates/clawg-cli/src/hook.rs:103-112`).

`hook.rs` sólo junta los hechos y llama.

**Descartado:** poner todo en `hook.rs`. Ya tiene 1425 líneas, y `doctor` necesita `read` y `rules` sin depender del hook.

## D2. `append` de `hook_log` se generaliza en lugar de copiarse

`hook_log::append` (`crates/clawg-cli/src/hook_log.rs:132-141`) sabe renombrar a los 64 KiB y agregar con una sola escritura, pero tiene fijos `FILE` y `ROLLED` (`hook_log.rs:18-22`). Pasa a recibir los dos nombres: `append_rolled(dir, file, rolled, roll_at, text)`. `hook_log::record` la llama con los suyos, y `tool_failures::append` con `tool-failures.jsonl` y `tool-failures.1.jsonl`. Las líneas de una misma llamada (dos programas ausentes) se arman en un solo texto y van en una sola escritura. Los tests de HR4 no cambian.

**Descartado:** un segundo `append` copiado. Son dos copias de la regla «renombrar, no reescribir», y la segunda podría perderla.

## D3. Los eventos nuevos salen antes del camino del daemon

`respond_reading` ya resuelve aparte `session-end` (`hook.rs:338-341`) y Cursor con `user-prompt-submit` (`hook.rs:335-337`). Los eventos nuevos se resuelven ahí mismo, con una función `after_tool(event, host, base_dir, input, deadline)`. Esa función:

1. lee stdin con `read_input` (`hook.rs:425-429`);
2. parsea `HookInput`, que suma `tool_name`, `tool_input` (`serde_json::Value`), `tool_response` (`Value`) y `error`;
3. si el cliente es Cursor, la entrada trae `cursor_version` o `tool_name` no es `Bash`, termina sin nada;
4. clasifica con `observe`;
5. si no hay ningún fallo ni ningún éxito candidato (por ejemplo, en un comando con `2>`), termina sin leer el libro ni la preferencia (el camino rápido);
6. si las hay, en un hilo propio con `read_by` (`hook.rs:416-418`), lee el libro, escribe lo que da `to_write`, decide el aviso de RE3 y, sólo si hay aviso, lee la preferencia.

El todo va bajo `POST_TOOL_DEADLINE`, 300 ms. `deadline_of` (`hook.rs:39-44`) suma los dos eventos. `envelope` (`hook.rs:207-219`) suma `PostToolUse` y `PostToolUseFailure`. No hay `Run`: `respond_and_record` sigue registrando sólo `SessionStart` (`hook.rs:366`).

Los éxitos se deciden después de leer el libro: un éxito de P se escribe sólo si P tiene una línea `fail` como la más nueva. Así una llamada que anda y no nombra ningún programa con fallo no escribe nada. Casi toda llamada a Bash que anda tiene un éxito candidato, así que casi siempre se lee el libro. Es una lectura de, como mucho, 128 KiB del caché del sistema, y la puerta de latencia la incluye.

**Descartado:** decidir los éxitos sin leer el libro, escribiendo un `ok` por cada llamada que anda. `rg` solo son 11.091 llamadas en los datos de A2: el libro rotaría cada pocos días y perdería los fallos.

**Descartado:** `PostToolBatch`, un proceso por tanda en lugar de uno por llamada. Sus `tool_calls` no traen `error` (los tipos del cliente lo dejan sin ese campo), así que igual haría falta `PostToolUseFailure`. Además es más nuevo y no se confirmó qué versiones del cliente lo corren.

## D4. La firma se reconoce a mano

Por cada línea del texto, sin `\r` final:

- `…: command not found`: el programa es lo que está entre el último `": "` anterior y ese final;
- `sh: N: X: not found`;
- `zsh: command not found: X`.

Después se aplican la forma de nombre válido y `redact` (`crates/clawg-core/src/memory/domain/redact.rs:235`, que pasa a `pub`). Primero se busca la subcadena `not found` en el texto entero, así que una salida sin ella no se parte en líneas.

**Descartado:** la dependencia `regex` en `clawg-cli`. Compilar el patrón cuesta en cada proceso de hook, y las tres formas se cubren con `ends_with` y `strip_prefix`.

## D5. `observe` no interpreta el shell

`command_words` corta por los separadores de la definición, salta asignaciones y prefijos, y devuelve la primera palabra de cada segmento sin comillas alrededor. Los dos errores posibles son seguros:

- Un programa que se toma por palabra de comando sin serlo sólo puede dar un fallo si además aparece en una firma, y la firma la escribe el shell.
- Uno que no se reconoce no da nada.

Un éxito pide una condición extra: el comando no tiene `2>`, `&>` ni `||` en ningún lugar. Es más estricto de lo necesario, a propósito: un éxito borra una regla.

**Descartado:** atribuir un `Exit code 127` sin firma al único programa del comando que figura en el libro. Con stderr redirigido no hay forma de saber qué programa faltó, y el costo de equivocarse es una regla falsa.

## D6. El bloque del arranque se arma antes de pedir el contexto

`answer` pide el contexto con `SESSION_BUDGET` (`hook.rs:494`). El orden pasa a ser este:

1. La lectura del libro y el cálculo de las reglas corren en un hilo propio, junto con la lectura del idioma (`respond_reading`, `hook.rs:346`) y con la misma espera, `LANGUAGE_WAIT`.
2. El bloque resultante se pasa a `answer`.
3. `answer` pide `SESSION_BUDGET - bloque.len()`.
4. `session_start_text` (`hook.rs:117-180`) lo pone después de la línea del idioma.

Si el daemon no está o falla, el texto de respaldo es la línea del idioma y el bloque, en lugar de la línea sola (`hook.rs:360-364`). Nada cambia para `user-prompt-submit`.

El tope del bloque es de 5 reglas con nombres de hasta 32 caracteres. La cabecera, las líneas y el cierre en español, el idioma más largo, quedan por debajo de 600 bytes. `what_the_hook_adds_stays_small` (`hook.rs:655-667`) se amplía con ese peor caso y conserva su aserción de 1500 bytes.

**Descartado:** que el bloque sume encima del contexto. Con el contexto lleno, la línea del idioma, los avisos y el bloque pasarían de 10.000 caracteres, y el cliente corta.

**Descartado:** pedirle las reglas al daemon. La regla tiene que llegar justo cuando el daemon no está, que es cuando el arranque queda sin memoria.

## D7. `setup` lleva el matcher en la tabla de eventos

`HOOK_EVENTS` (`crates/clawg-cli/src/setup.rs:51-52`) pasa de `(&str, &str)` a `(&str, &str, Option<&str>)`, con el matcher de cada evento:

| Evento | Matcher |
|---|---|
| `SessionStart` | `SESSION_START_MATCHER` |
| `UserPromptSubmit` | ninguno |
| `SessionEnd` | ninguno |
| `PostToolUse` | `Bash` |
| `PostToolUseFailure` | `Bash` |

`install_hooks` (`setup.rs:530-532`) usa ese matcher en lugar de preguntar por `SessionStart`. `remove_hooks` (`setup.rs:564-590`) y `--local` (`setup/local.rs:52`) recorren la misma tabla, así que no cambian. El timeout de los dos hooks nuevos es el de todos, `HOOK_TIMEOUT_SECS` (`setup.rs:47`). El tope real es el propio del hook (300 ms).

**Descartado:** un timeout más corto en `settings.json` para los hooks nuevos. Pediría un timeout por evento en `planned_hook` (`setup.rs:336-338`), y un hook igual al del plan con otro timeout pasaría a ser un conflicto en las instalaciones a mano. El tope propio ya es corto.

## D8. `doctor` reusa la misma función pura

`doctor::run` (`crates/clawg-cli/src/doctor/mod.rs:139`) lee el libro con `tool_failures::read` y calcula las reglas con `tool_failures::rules` y el `on_path` del proceso de `doctor`. La búsqueda de cada programa en los `CLAUDE.md` es pura: palabra entera ASCII, con mayúsculas exactas. Se hace sobre el `CLAUDE.md` del usuario (la tercera ruta de `setup::locations`, como en `protocol_block`, `doctor/mod.rs:278-282`) y sobre el `CLAUDE.md` y el `.claude/CLAUDE.md` de la carpeta actual. Los hallazgos los arma `checks::tool_rules(lang, &rules, &named, now_ms)`.

Que falten los hooks se mira con una función pura sobre el texto del `settings.json`, la segunda ruta de `locations`, y del `.claude/settings.local.json` de la carpeta actual. Si un archivo tiene el hook `session-start` de este perfil (`is_clawg_hook`, `setup.rs:438`) y le falta `post-tool-use` o `post-tool-use-failure`, hay hallazgo.

**Descartado:** que `doctor` mire la `hooks.runs` de los eventos nuevos. Los eventos nuevos no dejan corridas, a propósito (RE1).

## D9. La medición se apoya en 129 y agrega lo mínimo

`a3-notools` es `a3` con las claves `PostToolUse` y `PostToolUseFailure` sacadas del `settings.json` que da `setup::plan` (`crates/clawg-eval/src/bin/agent-eval/arm.rs:243-249`). Es el `a3` de antes de este cambio, sin mantener una copia de los hooks viejos.

La tarea (`shelf-eza-missing`), el shell acotado y `tool_failed_count` son de [129](../agent-eval-sessions/proposal.md#lugar-para-131-y-132). Si su diseño deja que el check reciba el texto de la firma, se le pasa `eza: command not found`, que es la forma que reconoce RE2. Este cambio no le pide más.

**Descartado:** comparar `a3` contra el resultado de `a3` de la campaña de 129. Esa campaña corre con otra versión del binario y con 3 corridas, y la regla de decisión pide 5 con todo lo demás igual. Su `a0` sí se reusa: no corre clawg.

## Probar a mano

En una carpeta de prueba, con un build de `testing`, `CLAWG_TEST_HOME` apuntando a un directorio temporal y `--profile dev`:

1. Pasar por stdin a `clawg hook post-tool-use-failure --agent claude-code --profile dev` el JSON `{"session_id":"s1","tool_name":"Bash","tool_input":{"command":"eza -la"},"error":"Exit code 127\n/usr/bin/bash: line 1: eza: command not found"}`. Imprime la línea del mismo turno.
2. Lo mismo con `"session_id":"s2"`.
3. `clawg hook session-start --agent claude-code --profile dev` con `{"session_id":"s3","source":"startup"}` imprime el bloque que nombra `eza`.
4. Borrar `tool-failures.jsonl` del directorio de datos de prueba. El paso 3 ya no lo nombra.
