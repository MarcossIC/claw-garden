# agent-eval-sessions — Diseño

Decisiones del cambio, cada una con lo que se descartó. Requisitos en la [spec](spec.md); el código citado está en `crates/clawg-eval/src/bin/agent-eval/`, en `8a2f3723`.

## D1. Una sesión larga es un proceso por parte, el segundo con `--continue`

`joined` lanza un `claude -p` por sesión, y desde la segunda le agrega `--continue`, que sigue la conversación más reciente del directorio. El bucle es el mismo que el de `split`; la diferencia es un argumento que decide `arm::plan`.

| Alternativa | Por qué no |
|---|---|
| Un prompt con todas las partes | El agente ve la parte 2 desde el principio. Mide otra tarea |
| Un proceso con los prompts por stdin (`--input-format stream-json`) | El corredor tendría que hablar el protocolo y esperar cada resultado, y `replay-client` no lo puede reproducir |
| `--session-id` en la primera y `--resume ID` en las demás | Más explícito, pero el plan necesita un id generado. Con un `CLAUDE_CONFIG_DIR` y un directorio de trabajo propios de la corrida, la conversación más reciente es la de la sesión anterior, sin ambigüedad |

Costo: `SessionStart` corre otra vez con `source` `resume`, así que el hook de clawg inyecta su contexto también en la segunda parte, cosa que una sesión que nunca se cerró no vería. Se informa (los eventos de hook de cada sesión están en su registro) y no se corrige. Que la conversación continuó se comprueba con el contexto del primer pedido de la segunda parte (AS8), no con un supuesto.

## D2. `sessions` al lado de `prompt`

`Task` gana `sessions: Vec<SessionSpec>` y `prompt` pasa a tener valor por omisión; los dos con `#[serde(default)]` y `skip_serializing_if`, y `problems` exige exactamente uno. Una función `Task::prompts()` da la lista en los dos casos, y es lo único que usa el resto.

| Alternativa | Por qué no |
|---|---|
| Sólo `sessions`, y migrar las ocho tareas | `run.json` guarda la tarea entera y `score_dir` la relee (`run.rs:710`): las 60 corridas pagas ya hechas dejarían de puntuarse |
| Un deserializador a mano que acepte las dos formas en un campo | Más código para el mismo resultado, y `deny_unknown_fields` (`task.rs:25`) deja de ser un derive |

## D3. El plan guarda la primera sesión donde estaba y las demás aparte

`RunPlan.client` sigue siendo el comando de la primera sesión (`arm.rs:187`) y se agrega `later: Vec<Step>`, vacío y sin serializar en una corrida de una sesión. Un `run.json` anterior se lee como una corrida de una sesión sin migrar nada, y el plan de una tarea de una sesión es byte a byte el de hoy, cosa que un test fija.

Alternativa: `sessions: Vec<Step>` en lugar de `client`. Es más simétrico, pero rompe la lectura de todo `run.json` existente y de los tests que leen `plan.client`.

Los nombres de los archivos de cada sesión salen de una función pura de su número y de cuántas hay. En una corrida de una sesión son los de hoy.

## D4. El alcance de un check es dónde está escrito

Un check dentro de una sesión se evalúa sobre esa sesión; uno de la tarea, al final. `Check` no cambia de forma.

| Alternativa | Por qué no |
|---|---|
| Un campo `session` en cada variante de `Check` | Toca cada variante y cada lugar que las construye |
| Un envoltorio con `session` y el check aplanado | `serde` no admite `flatten` con `deny_unknown_fields` |

Un check de archivo dentro de una sesión se rechaza: el puntaje se rehace desde lo que quedó en disco (AE6), y el directorio de trabajo sólo se conserva como quedó al final. Guardar una copia por sesión lo permitiría, pero ninguna tarea lo pide.

## D5. La bifurcación restaura en el lugar

`both` corre así: daemon, siembra, sesión 1, `clawg stop`; copia de `work/`, `claude-config/` y el directorio de datos sin `models/` a `fork/`; daemon, sesión 2 con `--continue`, `clawg stop`; se mueve `work/` a `joined/work/` y se restaura todo desde `fork/`; daemon, sesión 2 nueva, `clawg stop`; se mueve `work/` a `split/work/`. La rama continua va primero para que encuentre caliente la caché de la primera sesión.

| Alternativa | Por qué no |
|---|---|
| Dos corridas separadas, una `joined` y una `split` | Paga dos veces la primera parte, que en la tarea de relevo es más de la mitad del costo, y compara dos primeras partes distintas |
| Copiar la corrida a otro directorio y jugar ahí la segunda rama | El proyecto de clawg es la raíz del directorio de trabajo y la memoria nativa se guarda bajo su ruta (`seed.rs:100-102`): en otra ruta la sesión nueva no encuentra nada de la primera, por un motivo que no es el que se mide |
| `--fork-session` | Bifurca la conversación, no los archivos ni el store |

Los modelos quedan fuera de la copia: son 470 MB que ninguna sesión escribe. El daemon está detenido mientras se copia, porque en Windows un archivo abierto no se reemplaza.

Cada rama deja un `result.json` de una corrida de dos sesiones en su subdirectorio, y la raíz no tiene uno: `report` ya busca hasta tres niveles y no baja dentro de un directorio que tiene resultado (`report.rs:265-287`), así que las ramas le llegan como dos corridas, cada una con su forma y la marca del par.

Sólo para tareas de dos sesiones. Con más, habría que decidir en qué sesión se bifurca, y ninguna tarea lo necesita.

## D6. El cliente reproducido llama al servidor de la corrida

`replay-client` recibe el `mcp.json` de la corrida. Por cada llamada del stream a una herramienta de ese servidor, lanza el servidor como lo nombra el archivo, le habla JSON-RPC por stdio (`initialize`, `notifications/initialized`, `tools/call`, una línea por mensaje, como hace `crates/clawg-cli/tests/binary.rs:384-400`) y pone la respuesta en el lugar del resultado grabado. Un proceso del servidor por sesión, lanzado en la primera llamada.

| Alternativa | Por qué no |
|---|---|
| Dejar los resultados grabados | La sesión 2 no puede encontrar lo que guardó la 1: el relevo, la bifurcación y la clasificación de un hecho no se podrían ensayar sin pagar |
| Llamar a la API HTTP del daemon | No es el camino del agente: se saltea el puente, sus textos y el proyecto que resuelve por el directorio |

En seco el modelo es el falso, que arma el vector con un hash del texto: una búsqueda encuentra lo que guardó la sesión anterior sólo por un identificador en común (con letras, como `STAGING_PORT`) o por el texto idéntico, así que los streams de ejemplo buscan por identificador.

Decidir qué llamadas van al servidor y armar el stream con las respuestas es puro; hablar con el proceso es la única parte con I/O. No corre los hooks del cliente: lo que inyecta `session-start` no se ensaya así.

## D7. La memoria nativa: el archivo real, cortado como lo carga el cliente

Antes de cada sesión el corredor copia el `MEMORY.md` del momento. `delivered` (`run.rs:185-193`) usa esa copia, y `seed::native_loaded` la corta en las primeras 200 líneas o los primeros 25.000 bytes, lo que documenta el cliente.

| Alternativa | Por qué no |
|---|---|
| Seguir con el archivo del plan, entero | En una corrida de varias sesiones el agente lo cambió; con 300 reglas da por entregadas 234 que el cliente no carga |
| Sembrar un índice de una línea por memoria con archivos de tema | Es la forma que el cliente describe, pero 300 líneas tampoco entran, y un índice ya resumido es inventar lo que el agente habría hecho |
| Leer el archivo al puntuar | La sesión siguiente ya lo cambió, y el puntaje dejaría de ser repetible |

El corte es un modelo, no una medida: un canario pago lo comprueba con los tokens del primer pedido, y las dos constantes están en un solo lugar. Con 25 KB hay ambigüedad entre 25.000 y 25.600 bytes; con tres líneas por regla el tope de líneas llega mucho antes.

## D8. Un comando que no existe: un shell acotado, no un hook

Una tarea con `shell` saca `Bash` de lo negado y permite cada comando como `Bash(<comando>:*)`. `missing_commands` nombra los que no deben estar, y una corrida paga los busca en el `PATH` antes de empezar.

| Alternativa | Por qué no |
|---|---|
| Un hook `PreToolUse` del arnés que niega una herramienta nativa con un mensaje | No es un fallo de la herramienta. El hook que 131 quiere construir mira los fallos después de la llamada y no lo vería |
| Un servidor MCP del arnés con una herramienta rota | Hay que escribir un servidor, y `a0` dejaría de estar sin servidores |
| `--disallowedTools` con el nombre | Saca la herramienta del contexto: el agente no la puede intentar |

Lo que se pierde: AE2 negaba el shell en todo brazo porque es la única forma de que una sesión sin nadie actúe fuera de su directorio. Se mantiene para toda tarea sin `shell`. Una con `shell` sólo puede nombrar comandos sueltos, y los datos sólo nombran comandos de lectura; una sesión ya puede leer cualquier ruta con `Read`. Qué comandos hacen falta en la tarea definitiva lo decide 131.

## D9. Lo que agrega una llamada es lo que creció el pedido siguiente

`Transcript` gana `requests`, uno por `id` de mensaje, y cada `ToolCall` un `added_tokens` opcional.

| Alternativa | Por qué no |
|---|---|
| Contar los tokens del resultado con un tokenizer | El de clawg no es el del modelo, y el resultado no es todo lo que entra: también la entrada de la llamada y lo que el cliente agrega alrededor |
| Bytes entre 4 | Es lo que usan las medidas sobre transcripts y se usa para el sobrecosto del relevo, para poder comparar con ellas. Para una herramienta sola hay un dato mejor |
| Restar la salida del mensaje | El `usage` de un evento parcial trae una salida a medio contar |

Lo que no se puede atribuir se deja sin número y el informe dice cuántas llamadas tienen uno.

## D10. El ruido es un nombre, no un reemplazo

`--noise noise-300` elige el archivo; `noise.json` sigue siendo el de omisión y las corridas de siempre no cambian.

| Alternativa | Por qué no |
|---|---|
| Reemplazar `noise.json` por las 300 | Las corridas ya hechas dejarían de ser comparables y `l1` y `l2` dejarían de poder correr |
| Generar las reglas al correr | Un ruido que no está en un archivo no se puede revisar ni comprobar contra los trazadores |

Las 300 reglas se escriben acá. Unas 120 son de temas vecinos a los fixtures (Python, HTTP, documentación, registros de cambios), para que compitan en la búsqueda, y ninguna toca lo que lee un check: cómo termina una entrada de `HISTORY.md`, nombres o valores de timeouts, máscaras de credenciales, comandos para listar archivos, y lo que ya evitaban las 15.

## D11. El resultado crece sin romper el de antes

`RunResult` conserva `session` y `score` y gana `sessions`, `shape`, `noise_rules`, `facts`, `overhead` y `paired`, todos con valor por omisión. Arriba queda el registro de la última sesión y el puntaje de la corrida; `report` suma desde `sessions` y, si está vacío, desde `session`. El destino de un trazador con lo que lo trajo es una estructura nueva dentro de cada sesión, y `score.tracers` no cambia de forma.

Alternativa: un formato nuevo con versión. Obliga a migrar o a leer dos formatos, para 60 resultados que ya se leen bien.

## D12. Una categoría `E`

`K4` y `K5` siguen sin definición. Una `K` falla sin memoria y mide éxito; ésta mide una conducta, cuántas veces se repite un fallo, y una categoría propia evita que el reporte la sume con las `K`.

## D13. Los fixtures quedan fuera del índice de este repositorio

Un `.ignore` en `data/agent-tasks/fixtures/` los saca del recorrido del índice de docs (`crates/clawg-core/src/memory/project_docs/corpus.rs:81-91`) y de `rg`. La corrida copia el contenido de un fixture, no ese archivo, así que el índice de la corrida no cambia. Sin él, 600 KB de `requests` entrarían en `search_project` de quien trabaja en este repositorio. Costo: quien busque dentro de un fixture con `rg` tiene que pedirlo con `--no-ignore`.

## Lo puro y lo que toca disco

| Puro | I/O |
|---|---|
| `task::problems`, `Task::prompts`, los trazadores de sesión | `task::load`, `load_noise` |
| `arm::plan` con la forma; los nombres de archivo por sesión | `run::prepare` |
| Si una sesión deja seguir a la siguiente; los pasos de una bifurcación | `run::execute`: lanzar, copiar, restaurar |
| `stream::parse` con los pedidos; `added_tokens` | — |
| `score` por sesión, `seed::native_loaded`, `delivered`, la clase de un hecho, el sobrecosto | `run::score_dir`: leer streams, copias y registros |
| Qué llamadas de un stream van al servidor y el stream con las respuestas | El proceso del servidor en `replay-client` |
| `report::aggregate`, los pares, la tabla de llamadas | `report::load` |

## El directorio de una corrida

```
run.json
stream.jsonl client.log                 una sesión
stream-K.jsonl client-K.log             varias
instructions-K.jsonl memory-before-K.md
result.json                             split y joined
fork/                                   both: el estado después de la sesión 1
joined/ split/                          both: stream-2.jsonl, work/, result.json de cada rama
```
