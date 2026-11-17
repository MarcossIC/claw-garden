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

## Las enmiendas antes de pagar

D14 a D22 resuelven lo que la [segunda verificación](verify.md#segunda-verificación) encontró en `91046225`. Requisitos en AS13 a AS22 de la [spec](spec.md).

## D14. La ventana de guardado de un hecho termina en la penúltima sesión

`run::facts` (`run.rs:348-365`) busca el guardado en `sessions[i..]`, que incluye la última. Pasa a `sessions[i..len - 1]`. La sesión en que nace el hecho sigue contando: un brazo que lo guarda en el momento en que lo oye es lo que se quiere medir.

| Alternativa | Por qué no |
|---|---|
| Guardado en una sesión estrictamente anterior a la última y posterior a la de nacimiento | Saca el caso más común, guardar al oírlo, y deja a `a3` sin poder mantener un hecho de dos sesiones |
| Contar el guardado de la última sólo si fue antes del primer uso del hecho | Pide ordenar llamadas dentro de una sesión para un caso que no trae nada de una sesión anterior: lo que llega ahí llegó por otro camino |

## D15. El sobrecosto deja fuera lo que una edición exige

La sesión 2 de R edita tres archivos que suman 134.458 bytes y que la sesión 1 leyó por pedido del prompt; el cliente exige un `Read` en la misma sesión antes de un `Edit`. Contadas, esas lecturas (unos 35.000 tokens) superan solas el umbral de H1 y el veredicto lo decide la tarea. Se elige: `again` cuenta los `Read` de archivos que la sesión 1 leyó o escribió, salvo los de un archivo que la sesión 2 edita y que la sesión 1 no escribió; `to_edit` guarda lo que queda fuera, y se informa. El umbral de `rinde` baja de 20.000 a 10.000 tokens porque la medida ya no lleva lo inevitable ([propuesta](proposal.md#h1-el-relevo)).

| Alternativa | Por qué no |
|---|---|
| La diferencia dentro del par: lo que relee la rama nueva menos lo que relee la continua | Depende de si el cliente exige el `Read` también al continuar, que no está documentado: si lo exige, la diferencia se cancela; si no, vuelven los 35.000. Suma además el ruido de dos trayectorias por par. Se informa, no decide |
| Excluir los archivos que nombran los checks de la tarea | Es un conocimiento de la tarea dentro de una medida del stream, y deja fuera `TIMEOUTS.md`, que nombra un check pero es justo el camino del relevo por el repositorio |
| Achicar los archivos de R | Su tamaño lleva la sesión 1 a 150.000 tokens, la condición de validez; el fixture dejaría de ser una copia sin cambios de su fuente; y la medida seguiría dependiendo del tamaño de lo que se edita |
| Dejar la medida y subir el umbral | Un umbral que contiene el tamaño de la tarea no se traslada a otra tarea ni a una sesión real |

Una lectura de un archivo que la sesión 1 escribió cuenta siempre, aunque la sesión 2 lo edite: es la nota que dejó la sesión anterior, lo que una memoria reemplaza. Se pierde: una sesión que lee entero un archivo que después toca en una línea sale igual que una que lo lee por rangos; eso es costo del trabajo, no del relevo, y queda en `to_edit`. La rama continua lleva `rereads` y no `overhead`, porque su primer pedido es la conversación entera (la sugerencia 2 de la verificación).

## D16. Las lecturas fuera de `work/`: el cliente las bloquea y el arnés las detecta

`settings.json` de `claude-config/` lo escribe el arnés para todo brazo (`arm.rs:356-357`, a partir del de `setup` en los brazos que lo tienen, `arm.rs:310-312`), así que el bloqueo es una clave más en un archivo que el plan ya decide: `permissions.blockReadsOutsideWorkingDirectories: true`. El cliente niega con ella `Read`, `Grep`, `Glob` y `LSP` fuera de los directorios de trabajo en todo modo, desde la versión 2.1.257, según su referencia de configuración. `a4` agrega su directorio de memoria nativa en `permissions.additionalDirectories`: el bloqueo deja leer «los archivos que el cliente necesita», pero la lista que documenta no nombra la memoria automática, y una `a4` que no puede leer sus notas perdería por el arnés.

La detección es pura sobre el stream: cada llamada que lee una ruta fuera del directorio de trabajo y de los adicionales queda en `outside_reads`, negada o no. La que devolvió contenido es el artefacto `read_outside`.

| Alternativa | Por qué no |
|---|---|
| Separar el árbol: `work/` fuera de la raíz de la corrida | `..` deja de llevar a `run.json`, pero cualquier otra ruta sigue legible, incluido el directorio de datos de este repositorio con las tareas y sus checks. Rompe además el directorio de una corrida como unidad que `score` y `report` recorren, y la ruta del trabajo de una rama (`joined/work/`) |
| Reglas `permissions.deny` con `Read(//<raíz>/run.json)`, `Read(//<raíz>/fork/**)` y demás | Hay que enumerar lo que hay que esconder, y lo que se olvide queda abierto; una regla de lectura no puede negar la raíz y permitir `work/` dentro, porque una negación le gana a un permiso. Su ventaja, que alcanza a `cat` y `head` en `Bash`, no hace falta: ningún brazo tiene esos comandos |
| Sólo detectar | Una corrida con la fuga se tira y se repite: se paga dos veces lo que el cliente puede negar gratis |
| El sandbox del cliente | Bloquea en el sistema operativo y alcanza a todo proceso, pero cambia cómo corren los comandos y los hooks del arnés y de clawg, y su soporte depende del sistema operativo |

Lo que no cubre: el shell. Sólo E lo tiene, con `eza` y `ls`, que no leen contenido, y E no tiene trazadores; la detección lista igual un `ls` fuera. Lo que el cliente carga solo (la conversación con `--continue`, el `CLAUDE.md` de la configuración, el `MEMORY.md`) no pasa por una herramienta y el bloqueo no lo toca, que es lo que se quiere: la rama continua tiene su contexto y la nueva no puede leerlo.

La versión mínima se comprueba sobre `--claude-version`, que `check_client_version` (`run.rs:702-711`) ya iguala a la que el cliente informa.

## D17. `untouched` se lee al terminar cada sesión

D4 rechaza un check de archivo dentro de una sesión porque el directorio de trabajo sólo se conserva al final. `untouched` no es un check: el corredor lee los archivos que nombra justo después de la sesión y deja `untouched-K.json`, que es lo único que el puntaje necesita, y el puntaje se sigue rehaciendo desde el disco.

| Alternativa | Por qué no |
|---|---|
| Comprobar `fork/work/` en la guía, a mano | Sirve sólo para la bifurcación (D corre `split` y `joined`, sin `fork/`), y una exclusión que se decide a mano es una que se puede reinterpretar |
| Comprobarlo desde el stream, por los `Write` y `Edit` de la sesión | Un `Edit` lleva sólo el fragmento cambiado, y un comando o un subagente puede escribir sin que el stream lo muestre igual |
| Un check de archivo por sesión que falle la corrida | Haría de la desobediencia de la sesión 1 un fallo de la memoria en la sesión 2. Es un artefacto: el relevo ya no se mide |
| Una copia del directorio de trabajo por sesión | Cuesta 600 KB por sesión para leer tres archivos |

## D18. Un intento de un comando es una parte de la línea que lo invoca

`command_runs` parte el comando de `Bash` por sus separadores y mira la primera palabra de cada parte. H6 lee los intentos y no la salida, así que no depende del texto del shell del cliente.

| Alternativa | Por qué no |
|---|---|
| Sólo el texto `eza: command not found` | Es lo que había: con otro shell u otro mensaje, H6 sale «no hay efecto» por el arnés |
| Buscar `eza` en cualquier lugar del comando | `echo eza` o un comentario lo cuentan, y un agente que explica por qué no lo usa sale como si lo hubiera intentado |
| Un analizador de shell completo | Ninguna tarea lo pide; las formas que no se cubren (`sh -c 'eza'`, `xargs eza`) son raras en una sesión y salen como 0, el lado conservador de «la memoria lo evita» |

## D19. Toda corrida que dejó un stream deja un resultado

Hoy una corrida de una sesión que falla no escribe `result.json` (`main.rs:451`), y en una bifurcación la rama que falla no corre `Keep` (`run.rs:874-886`). Se escribe siempre que haya un stream de la sesión 1, y la rama que falla mueve sus archivos antes de devolver el error. Así `costs` cuenta lo gastado y la regla 4 cuenta las exclusiones con todo.

Alternativa: dejarlo en la guía, con un `agent-eval score --run DIR` sobre cada corrida que falló. Es lo que la verificación propuso como paso provisorio; con más de cien corridas, un paso que se olvida es dinero que no se cuenta.

## D20. La bifurcación copia el home entero menos los modelos

`save_state` copia el directorio de datos de clawg y no el resto de `home/` (`run.rs:954-962`). Pasa a copiar `home/` entero con `models/` fuera, que `copy_dir` ya salta por nombre (`run.rs:639-653`); al restaurar, `clear_dir` conserva `<datos>/models`. Lo que el cliente o un hook escriba en el home deja de pasar de la rama continua a la nueva.

Alternativa: listar `home/` en el humo y decidir a mano. No dice nada de las corridas que vienen después, y la copia es un cambio de una ruta.

## D21. Las causas de un artefacto las decide el resultado

`run::artifacts` es puro sobre los registros de las sesiones, la forma y `untouched`, y deja la lista en `result.json`; `report` la lee. Así la regla 3 se aplica igual en cada corrida, y una exclusión tiene su causa escrita al lado.

| Alternativa | Por qué no |
|---|---|
| Que `report` las calcule al agregar | El reporte relee resultados, no streams, y la causa no quedaría en la corrida que la tiene |
| Que la guía las lea en cada `result.json` | Es leer a mano lo que la regla dice que se muestra «sin interpretación» |

La compactación entra así: en una sesión que no es la última es un artefacto, porque en R el par deja de comparar seguir con cortar; en la última se cuenta y se informa, porque es lo que hace una sesión real que sigue.

## D22. Lo que va al código y lo que va a la guía

Va al código lo que cambia un número que la regla lee o la decisión de excluir una corrida: el ruido en la clave de las filas (W1), la continuación y las compactaciones en la tabla de pares (W2), puntuar lo que falla (W3), `untouched` (W5), `command_runs` (W6) y el home entero (W7). Va a la guía lo que se lee una vez, en el humo, y no cambia ningún cálculo:

- los topes de `--max-budget-usd` por tarea y la cuenta de lo gastado contra el peor caso de cada corrida;
- el canario, Δ entre N bajo `a4` con 300 y con 15 reglas, que la propuesta fija entre 500 y 3.000 tokens (W4); el reporte ya da el primer pedido de cada fila;
- el texto con que el shell del cliente informa que falta `eza` en la sesión 1 de E bajo `a0`, que se anota y no decide (W6);
- bajo `a0` en E, que el primer pedido de las sesiones 2 y 3 sea el de la 1 y que la línea `init` no liste otro shell que `Bash` (la sugerencia 5);
- las lecturas negadas de cada corrida del humo, para confirmar que el bloqueo no le niega a una sesión lo que necesita.

Las sugerencias 3 (los bytes del resultado junto a lo que agrega una llamada) y 6 (un texto que nombra una fase, y un salto de línea literal) también van al código; la 4 (el trazador literal) es una regla de lectura de H3 en la propuesta.

`--max-budget-usd` sigue siendo uno por corrida. Uno por sesión, en la tarea, ataría a la tarea un parámetro de la campaña, y el peor caso de una corrida (procesos por tope) ya da la cuenta que la guía necesita.

Cada hallazgo de la segunda verificación, con lo que se hace:

| Hallazgo | Qué se hace |
|---|---|
| CRITICAL 1, el guardado de la última sesión | Aceptado: D14, AS13, tarea 15 |
| CRITICAL 2, el sobrecosto inflado por la tarea | Aceptado: D15, AS14, H1 reescrita, tarea 16 |
| CRITICAL 3, lo que una sesión lee fuera de `work/` | Aceptado: D16, AS16 y AS17, tareas 19 y 20 |
| W1, los dos ruidos en una fila | Aceptado: el ruido entra en la clave (AS22, tarea 24) |
| W2, la puerta del humo se lee a mano | Aceptado: continuación y compactaciones en la tabla de pares, la compactación como artefacto (f) (AS21, AS22, tareas 21 y 24) |
| W3, lo que se pierde cuando una sesión falla | Aceptado: D19, AS19, tarea 22 |
| W4, el umbral del canario | Aceptado en la propuesta: `a4` con 300 contra `a4` con 15, entre 500 y 3.000 tokens; se lee en la guía |
| W5, la sesión 1 que desobedece | Aceptado: `untouched` y el artefacto (g) (D17, AS15, tarea 18) |
| W6, H6 lee el texto de salida | Aceptado: `command_runs`, y H6 lee los intentos (D18, AS18, tarea 17); el texto se anota en el humo |
| W7, el home no se restaura | Aceptado: D20, AS20, tarea 23 |
| S1, la bifurcación sólo se prueba en `a3` | Aceptado: un test de `a4` en la tarea 23 |
| S2, el sobrecosto de las filas `joined` | Aceptado: vacío, con las relecturas (AS14, AS22) |
| S3, `added_tokens` incluye el pensamiento | Aceptado: los bytes del resultado entre 4 al lado (AS22, tarea 25); una herramienta se anota para dieta sólo si las dos medianas pasan de 1.500 |
| S4, el trazador literal | Aceptado como regla de lectura: en R decide `FORK_DEFAULT_TIMEOUT` (propuesta, H3); sin código |
| S5, el primer pedido de E bajo `a0` | Aceptado, en la guía del humo; no decide |
| S6, un texto que nombra una fase y un salto de línea literal | Aceptado: tarea 26 |

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
| `run::facts` con su ventana, `run::rereads`, `run::outside_reads`, `run::artifacts`, las invocaciones de un comando, la versión mínima del cliente, el bloqueo en `settings.json` | Leer los archivos de `untouched` después de cada sesión; copiar y restaurar el home |

## El directorio de una corrida

```
run.json
stream.jsonl client.log                 una sesión
stream-K.jsonl client-K.log             varias
instructions-K.jsonl memory-before-K.md
untouched-K.json                        una sesión con untouched
result.json                             split y joined, también si falló
fork/                                   both: el estado después de la sesión 1, con el home sin models/
joined/ split/                          both: stream-2.jsonl, work/, result.json de cada rama
```
