# agent-eval-sessions — Verificación

Veredicto: **cumple lo que la suite puede comprobar**. Quedan fuera de esta verificación el CI de Linux y Windows y la campaña, que es local y paga.

## Qué se corrió

En Windows, sobre el último commit de código, con `target-test` y la copia del árbol sin cambios sin commitear en `crates/`:

- `cargo test --workspace --target-dir target-test`: pasa.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings`: pasa.
- `cargo clippy -p clawg-cli --target-dir target-test -- -D warnings`: pasa.
- `agent-eval tasks`: sale con 0. Da válidas las catorce tareas, con las seis nuevas (`requests-timeout-decision-relay` con `sessions=2`, `requests-redact-decision-revised` con `sessions=3`, `requests-history-owner-convention`, `shelf-eza-missing` con `sessions=3`, y los ejemplos `example-relay-port` y `example-missing-command` con `sessions=2`), ninguna con un trazador en un archivo de su fixture, y lista los dos ruidos: `noise rules=15` y `noise-300 rules=300`.

El cambio no toca el ranking, así que no corre `rag-eval`. Ninguna corrida fue paga: no hubo `claude` ni `ANTHROPIC_API_KEY`.

## Requisitos

| Requisito | Dónde | Veredicto |
| --- | --- | --- |
| AS1 orden del `MEMORY.md` | `seed::native_memory` (commit anterior a esta tanda) | Cumple |
| AS2 tarea de varias sesiones | `task.rs`: `sessions`, `Task::prompts`, trazadores de sesión | Cumple |
| AS3 fallos de una herramienta, shell, `E` | `Check::ToolFailedCount`, `Task::shell`, `Task::missing_commands`, `arm::plan` | Cumple |
| AS4 plan de varias sesiones | `Shape`, `RunPlan::later`, `run::session_files` | Cumple |
| AS5 corredor de varias sesiones | `run::play_session`; una sesión que falla no deja empezar la siguiente y la corrida se puntúa con las que corrieron | Cumple |
| AS6 bifurcación | `run::fork_steps`, `Fork`, `save_state` y `restore_state`; ramas en `joined/` y `split/` | Cumple |
| AS7 corrida en seco contra el servidor | `replay::server_calls`, `replay::with_results`; directorio de streams, también el de una bifurcación | Cumple |
| AS8 tokens de cada pedido y de cada llamada | `Request`, `ToolCall::added_tokens`; ahora también `Request::cache_read` | Cumple |
| AS9 puntaje por sesión y camino de un hecho | `score::score_session`, `score::score`, `run::evaluate`, `run::facts`, `run::overhead`, `seed::native_loaded` | Cumple, con un matiz, ver abajo |
| AS10 reporte, costos y llamadas | `report::aggregate`, `pairs`, `costs`, `calls`; `agent-eval calls` | Cumple, con dos matices, ver abajo |
| AS11 ruido de una corrida | `--noise`, `task::load_noise`, `noise-300.json`, `ArmError::StartupTooLong` | Cumple |
| AS12 tareas nuevas y fuentes | cuatro tareas, `psf-requests` (64 archivos, 601855 bytes, 17811 líneas, tal como en la fuente), `shelf-notes`, `SOURCES.md` | Cumple |

## Desvíos y matices

1. **El escenario «un hecho que no se guardó» de AS9.** Dice que en la corrida `joined` el trazador sale `reached_not_used`, traído por `context`. La regla de clasificación de la misma sección da `not_saved` a todo hecho que ninguna sesión escribió en una memoria, sea cual sea la forma. Se implementó la regla: un hecho que sólo está en el prompt de una sesión anterior de una corrida continua sale `not_saved`, y su camino por sesión sí lo muestra traído por `context`. El escenario es el que hay que corregir al archivar.
2. **La tabla de `report` cambia de columnas.** AS10 pide que los tests de AE8 y MD7 sigan pasando. Sus aserciones sobre líneas exactas se actualizaron para las columnas nuevas (`shape` después del brazo, y al final las sesiones, el contexto, el sobrecosto y las clases de hecho). La semántica con resultados de una sesión no cambia.
3. **`report` no suma dos veces la primera sesión de una bifurcación.** Las dos ramas la llevan en su `result.json`. Los totales y `costs` la cuentan una vez, en la rama continua. Los promedios por fila sí son los de la corrida entera de cada forma.
4. **El ahorro de un par** usa el costo de la segunda sesión de cada rama, porque la primera es la misma. AS10 no lo precisa.
5. **El stream de una bifurcación en seco** es un directorio aparte, `replays/example-relay-port.a3.both/`, porque el directorio de la forma `split` ya tiene su `session-2.jsonl` y un directorio con un stream de más se rechaza. Los streams del ejemplo buscan por `STAGING_PORT` y `RETRY_BUDGET`, no por `48213` ni `7719`: con el modelo falso una búsqueda sólo encuentra por un identificador con letras o por el texto idéntico (nota en el [diseño](design.md), D6).
6. **Una corrida de varias sesiones que falla** deja un `result.json` con las sesiones que corrieron y no terminada; una de una sesión que falla sigue sin dejarlo, como antes.
7. **`docs/make.bat` de `psf-requests` conserva sus CRLF.** `.gitattributes` lo deja con `-text` para que el fixture quede tal como en la fuente, y `the_task_data_keeps_lf_line_endings_on_every_system` salta los `.bat`.
8. **Los `Session` de un `result.json` ganan `request_tokens` y `first_request_cache_read`**, con valor por omisión: los resultados anteriores se siguen leyendo.

## Pendiente en local

- El CI en verde en Linux y Windows.
- La campaña: fase 0 (todas las tareas nuevas en seco con el modelo real), fase 1 (humo y canarios, que es una puerta) y las fases 2 a 4, con la regla de decisión de la [propuesta](proposal.md#la-regla-de-decisión) copiada sin cambios. Lo que el arnés no comprueba y sólo una sesión paga muestra: que `--continue` conserve el contexto en `-p`, que la memoria nativa escriba en `-p`, el modelo de carga de 200 líneas o 25.000 bytes del `MEMORY.md` y el costo real de las sesiones largas.
- Al archivar: AS1 reemplaza el orden del `MEMORY.md` en MD3 de `openspec/specs/eval/spec.md`, y AS2 a AS12 pasan a esa sección, con el escenario de AS9 corregido.

## Segunda verificación

Lectura independiente del código y los datos de `111e6644..91046225` en `crates/clawg-eval`, contra la propuesta, la spec y el diseño. No se corrió `cargo` (la suite la informa la verificación de arriba) ni ninguna sesión. Las líneas citadas son las de `91046225`.

Veredicto: **la plomería cumple la spec, pero tres puntos sesgan lo que la regla de decisión lee y hay que resolverlos antes de pagar**. La bifurcación restaura bien lo que dice restaurar (`work/`, `claude-config/` con la memoria nativa y la conversación, y el directorio de datos sin `models/`, con el daemon detenido; `run.rs:950-978`), `--continue` es siempre el último argumento y la rama nueva lo pierde (`arm.rs:411-421`, `run.rs:853-859`), el corte del `MEMORY.md` y su orden son los de AS1 (`seed.rs:89-118`), y ningún trazador ni texto de un check de las tareas nuevas está en su fixture ni en `noise-300.json` (comprobado con `rg -uu`, porque `fixtures/.ignore` los saca de `rg`).

### CRITICAL

1. **Un hecho que sólo guardó la última sesión sale `used`.** `run::facts` mira `sessions[i..]` (`run.rs:353`), que incluye la última. La propuesta (H3) define `no guardado` como que «ninguna sesión *anterior*» lo escribió. Un brazo con memoria suele guardar al terminar: si la sesión 2 de R recibe la decisión por `TIMEOUTS.md` (o por otra vía) y la guarda al final, el hecho sale `used` aunque ninguna memoria lo haya traído, y suma para «mantiene un hecho en 3 de 3» de `a3` y `a4`. El test `a_fact_is_told_as_not_saved_not_reached_not_used_or_used` no cubre ese caso. Arreglo: `sessions[i..sessions.len() - 1]` y un caso del test con el guardado sólo en la última.
2. **El sobrecosto de R pasa de 20.000 tokens por cómo está hecha la tarea.** La sesión 2 tiene que editar `src/requests/adapters.py` (27.992 bytes), `docs/user/advanced.rst` (41.903) y `HISTORY.md` (64.563, 2.102 líneas), que la sesión 1 leyó entera por pedido del prompt, y el cliente exige un `Read` en la misma sesión antes de un `Edit`. `run::overhead` (`run.rs:371-388`) cuenta cada uno de esos `Read` a 4 bytes por token y no resta lo que relee la rama continua. Leídos enteros, con el prefijo de número de línea, son unos 35.000 tokens. Así, `rinde` (20.000 o menos) depende de si el agente lee por rangos y no de la memoria, y el «Qué sigue» de H1 (proponer el aviso de corte si `a3` rinde) queda decidido por la tarea. Antes de pagar hay que elegir y escribir en la propuesta una de estas: el sobrecosto como diferencia dentro del par (lo que relee la nueva menos lo que relee la continua de los mismos archivos), o sin los archivos que los checks piden editar. En los dos casos, agregar al resultado lo que relee la rama continua.
3. **Una sesión nueva puede leer lo que la corrida guardó de las otras, y nada lo detecta.** Al lado de `work/` están `run.json` (la tarea entera: prompts, trazadores y checks), `stream-K.jsonl` de las sesiones anteriores, `fork/` y, en la rama nueva de una bifurcación, `joined/work/`, con la decisión ya aplicada. Además, `claude-config/projects/` guarda la conversación de la sesión 1. El cliente limita las escrituras al directorio de trabajo, no las lecturas (a confirmar con la primera sesión real). `Read`, `Glob` y `Grep` no se niegan, y en E se permite `ls`. El prompt de la sesión 2 de R pide «la decisión que te di en la sesión anterior», que invita a buscarla. Si la encuentra, `brought_by` dice `Read`, igual que si la leyó en `TIMEOUTS.md`. Es la misma clase de fuga que K3 en el piloto, cuando el agente leyó la respuesta en el nombre de la carpeta. Arreglo mínimo: marcar como artefacto en `result.json` (y en el reporte) toda llamada cuya ruta caiga bajo la raíz de la corrida y fuera de `work/`. Mejor todavía, sumar reglas de denegación de lectura sobre esa raíz.

### WARNING

1. **El reporte junta los dos ruidos.** Las filas van por tarea, brazo y forma (`report.rs:159-167`), y `noise_rules` no entra en la clave. La propuesta define la celda con el ruido, y H2 compara 15 contra 300 sobre la misma tarea y el mismo brazo, en la misma fase 3. Hay que sumar el ruido a la clave o, como mínimo, poner cada ruido en su propio directorio de `--runs`.
2. **La puerta del humo se lee a mano.** El artefacto (c), «no continuó» (el primer pedido de la sesión 2 continua contra el último de la 1), no está en la tabla de pares, que sólo da el contexto al bifurcar (`report.rs:331`). Tampoco aparecen las compactaciones: con 150.000 a 200.000 tokens al bifurcar, una compactación automática en la sesión 1 puede caer, y la regla no dice si es artefacto. Hay que agregar las dos columnas y decidir lo de la compactación antes de la fase 1.
3. **Lo que se pierde cuando una sesión falla.** En una bifurcación, si falla la segunda sesión de una rama, `Keep` no corre. Su `stream-2.jsonl` queda en la raíz y `score_dir` puntúa la rama con una sola sesión (y en `a0`, sin directorio de rama, no puntúa nada). Una corrida de una sesión que falla no deja `result.json` (`main.rs:451`). En los dos casos `costs` no cuenta esa sesión, y el tope de 25 USD y la regla 4 (más de 2 excluidas detienen la fase) se llevan con números de menos. Arreglo: mover los archivos de la rama antes de devolver el error, y puntuar siempre.
4. **El umbral del canario no separa los dos modelos de carga.** Con `noise-300` y la memoria de la tarea K, el `MEMORY.md` simulado tiene 33.559 bytes y 904 líneas: las 200 primeras son 7.273 bytes (unos 1.800 tokens), el tope de 25 KB solo serían unos 6.000 tokens y el archivo entero unos 8.000. La diferencia con `a0` suma además las instrucciones de la memoria nativa, que `a0` no tiene. «Más de 8.000» sólo detecta el archivo entero, y apenas, y no distingue el corte por bytes del corte por líneas. Propuesta: comparar `a4` con 300 contra `a4` con 15 (N, unos 0,03 USD más en la fase 1) y tomar como modelo roto una diferencia de más de 3.000 tokens.
5. **La sesión 1 que desobedece pasa inadvertida.** R y D le piden a la sesión 1 (y D a la 2) no aplicar todavía la decisión, y un check de archivo por sesión no existe (D4). Si la sesión 1 escribe `FORK_DEFAULT_TIMEOUT` en `src/`, las dos ramas aprueban sin relevo. Hay que comprobar en `fork/work/` que ni `src/`, ni `docs/`, ni `HISTORY.md` tienen el trazador antes de contar el par (lo mejor es automatizarlo como artefacto), y en D revisar `stream-1` y `stream-2`.
6. **H6 lee el fallo por el texto de salida.** `tool_failed_count` con `text` (`score.rs:104-113`) depende de que el shell del cliente diga `eza: command not found`. Con otro shell u otro texto, H6 sale «no hay efecto» por el arnés. Conviene contar también las llamadas a `Bash` cuyo comando invoca `eza`, y confirmar el texto en la sesión 1 de E bajo `a0` del humo.
7. **El home no se restaura fuera del directorio de datos.** Lo que el cliente o un hook escriban en `home/` fuera de `.clawg*` pasa de la rama continua a la nueva, y una escritura de `a4` fuera del directorio planeado no cuenta en `native_memory_writes`. Hay que restaurar el home entero menos `models/`, o listar `home/` después de la rama continua en el humo.

### SUGGESTION

1. El test de la bifurcación es sólo de `a3`. Uno de `a4` fijaría que la rama nueva no ve el `MEMORY.md` que escribió la continua.
2. La columna `relay overhead` de las filas `joined` es la conversación entera. Mejor dejarla vacía, porque la regla lee la de `split`.
3. `added_tokens` (H5) incluye el pensamiento que el modelo hizo en ese mensaje, que dentro de un ciclo de herramientas sigue en el contexto. Junto a la mediana conviene dar los bytes del resultado entre 4, antes de marcar una herramienta para dieta.
4. `saved` busca el trazador literal. Un `save_memory` que dice «4,5 s y 41 s» cuenta como `not_saved`. R tiene dos trazadores: leer los dos.
5. En el humo, bajo `a0`, el primer pedido de las sesiones 2 y 3 de E debería ser igual al de la 1 (el cliente no trae nada de la sesión anterior), y la línea `init` no debería listar otro shell que `Bash`.
6. `report.rs:523` dice «of a run of a phase» (nombra una fase de la campaña), y `main.rs:475-476` corta la cadena con un salto de línea literal en lugar de `\n`.

### Los desvíos declarados

- **AS9, `not_saved` y no `reached_not_used` en la forma continua.** La implementación sigue la definición de H3 en la propuesta. No afecta a la campaña: el escenario se corrige al archivar. Lo que sí la afecta es CRITICAL 1.
- **`--max-budget-usd` por corrida.** Cada proceso recibe el mismo tope, así que en R las segundas partes llevan 1,20 y no 0,80. El máximo de una corrida de R pasa de 2,80 a 3,60 USD. No compromete la campaña si la guía fija 1,20 para R, 0,50 para D y la K larga y 0,15 para E y el piloto, y lleva el gasto contra los 25 USD con `costs`, una vez resuelto WARNING 3.
- **El `result.json` de una corrida de una sesión que falla.** Compromete el conteo de exclusiones y de gasto: ver WARNING 3. Hasta que se arregle, la guía tiene que correr `agent-eval score --run DIR` sobre cada corrida que falló.

### Antes de la campaña paga

1. CRITICAL 1: sacar la última sesión de `facts`, con su test.
2. CRITICAL 2: fijar en la propuesta cómo se mide el sobrecosto de R e implementarlo, con lo que relee la rama continua en el resultado.
3. CRITICAL 3: marcar como artefacto toda lectura fuera de `work/` bajo la raíz de la corrida y, si se puede, negarla.
4. WARNING 1: el ruido en la clave de las filas, o un directorio por ruido.
5. WARNING 2: la continuación y las compactaciones en la tabla de pares, y la regla de la compactación escrita.
6. WARNING 3: puntuar la rama o la corrida que falla, para que `costs` y el conteo de exclusiones la cuenten.
7. WARNING 4: el canario contra `a4` con 15, con su umbral.
8. WARNING 5: la comprobación de `fork/work/`, automática o en la guía.
9. En la guía: los topes por tarea, el texto de `eza` (WARNING 6) y el listado de `home/` (WARNING 7) en el humo.
