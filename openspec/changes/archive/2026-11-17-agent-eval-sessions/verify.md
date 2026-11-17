# agent-eval-sessions — Verificación

Veredicto: **cumple lo que la suite puede comprobar**. El CI de Linux y Windows pasó después sobre `9a76a8ec`, que lleva todo el código del cambio, y la campaña paga corrió (sección «La campaña»); lo que quedó sin medir está en «Pendiente después de publicar», al final.

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

- El CI en verde en Linux y Windows: hecho, pasó sobre `9a76a8ec`.
- La campaña: fase 0 (todas las tareas nuevas en seco con el modelo real), fase 1 (humo y canarios, que es una puerta) y las fases 2 a 4, con la regla de decisión de la [propuesta](proposal.md#la-regla-de-decisión) copiada sin cambios. Lo que el arnés no comprueba y sólo una sesión paga muestra: que `--continue` conserve el contexto en `-p`, que la memoria nativa escriba en `-p`, el modelo de carga de 200 líneas o 25.000 bytes del `MEMORY.md` y el costo real de las sesiones largas.
- Al archivar: AS1 reemplaza el orden del `MEMORY.md` en MD3 de `openspec/specs/eval/spec.md`, y AS2 a AS12 pasan a esa sección, con el escenario de AS9 corregido: hecho el 2026-11-17 (AS1 a AS22).

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

## Tercera verificación

Sobre las enmiendas, las tareas 15 a 27 (`1843c2d3..e871c4d5`).

### Qué se corrió

En Windows, sobre `e871c4d5`, en `target-test`:

- `cargo test --workspace --target-dir target-test`: pasa.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings`: pasa.
- `cargo clippy -p clawg-cli --target-dir target-test -- -D warnings`: pasa.
- `agent-eval tasks`: sale con 0, da válidas las catorce tareas y lista `noise rules=15` y `noise-300 rules=300`. `untouched` está en `requests-timeout-decision-relay` (sesión 1), `requests-redact-decision-revised` (sesiones 1 y 2) y `example-relay-port`; `command_runs`, en `shelf-eza-missing` y `example-missing-command`.

Además, una lectura independiente del código, sin `cargo` y sin sesiones, contra AS13 a AS22 y D14 a D22.

### Cada tarea

| Tarea | Veredicto | Evidencia |
| --- | --- | --- |
| 15 | Cumple | `run.rs:475` mira `sessions[i..sessions.len() - 1]`; `a_fact_saved_only_by_the_last_session_is_not_saved` |
| 16 | Cumple, con W4 | `rereads` pura (`run.rs:594-618`); `overhead` en `None` en `joined`; rutas resueltas desde el `work/` donde corrió el cliente |
| 17 | Cumple | `score::invocations` con comillas, `$(`, la comilla invertida, asignaciones y rutas; `the_runs_of_a_command_are_counted_in_every_part_of_a_shell_line` |
| 18 | Cumple, con W3 | `play_session` escribe `untouched-K.json` antes de `Save` (`run.rs:1069-1075`) |
| 19 | Cumple | el bloqueo en todo brazo y `additionalDirectories` sólo en `a4` (`arm.rs:358`, `:380-382`); `check_read_block` antes de preparar (`main.rs:441-443`) |
| 20 | Cumple, con W2 | `outside_reads` (`run.rs:552-590`) |
| 21 | Cumple | `artifacts`, pura, con las seis causas (`run.rs:271-299`) |
| 22 | Cumple, con W5 | `main.rs:452-457` puntúa siempre; `fork` aplica `Keep` de la rama que falló al jugar (`run.rs:1174-1183`) |
| 23 | Cumple | `save_state` y `restore_state` con el home entero menos `<datos>/models` (`run.rs:1254-1299`) |
| 24 | Cumple | el ruido en la clave, las excluidas y `PairRow` (`report.rs:170-261`, `:363-411`) |
| 25 | Cumple | la mediana del resultado entre 4 (`report.rs:530-559`) |
| 26 | Cumple | `report.rs:612-613`, `main.rs:476`, `:480` |
| 27 | Cumple | `docs/reference/testing.md` y la fila 129 de `openspec/README.md` |

Ningún `#[allow]`, comentario de más ni texto que nombre una fase o una persona en lo que cambió.

### Cada punto de «Antes de la campaña paga»

| Punto | Lo cierra |
| --- | --- |
| 1. CRITICAL 1 | Tarea 15 |
| 2. CRITICAL 2 | Tarea 16 y H1 de la propuesta |
| 3. CRITICAL 3 | Tareas 19, 20 y 21 |
| 4. WARNING 1 | Tarea 24 |
| 5. WARNING 2 | Tareas 21 y 24, y la regla de la compactación (f) en la propuesta |
| 6. WARNING 3 | Tarea 22, con el hueco de W5 abajo |
| 7. WARNING 4, el canario | La guía: Δ entre N bajo `a4` con 300 y con 15 reglas, entre 500 y 3.000 tokens |
| 8. WARNING 5 | Tarea 18, con el hueco de W3 abajo |
| 9. Topes por tarea | La guía |
| 9. El texto de `eza` | Tarea 17: H6 lee los intentos; el texto se anota en el humo |
| 9. El listado de `home/` | Tarea 23, que restaura el home entero |

De las sugerencias, S1 la cierra la tarea 23, S2 las 16 y 24, S3 la 25 y S6 la 26. S4 es una regla de lectura de H3, y S5 va a la guía.

### Hallazgos

**CRITICAL 1. El `a3` de la fase 4 ya trae los hooks de 131.** Desde `cca60fb7`, `setup` instala `PostToolUse` y `PostToolUseFailure` para Bash, y sólo `a3-notools` no los tiene (`arm.rs:325`). `shelf-eza-missing` deja correr Bash con `eza`. Bajo `a3`, las sesiones 1 y 2 enseñan `eza` al libro de fallos y la sesión 3 empieza con la regla, así que H6 leería «la memoria de hoy ya lo evita» por lo que agrega 131, y 131 se quedaría sin línea base. Las demás tareas niegan Bash. **Resuelto en la propuesta:** H6 y la fase 4 leen `a3-notools` como la memoria de hoy, con el mismo costo, y el `a3` de ahora lo corre la campaña de 131.

**W1. `score --run` con una ruta relativa marca todo como artefacto.** `main.rs:465` no la hace absoluta (`run` sí, `main.rs:372`), así que cada `Read` del stream cae fuera y sale `read_outside`. Tarea 32. Hasta entonces, la guía puntúa siempre con rutas absolutas y no mueve las corridas.

**W2. Falsos `read_outside`.** `Grep` toma como ruta un `pattern` que empieza con `/` (`run.rs:556`), `Bash` toma `/dev/null`, y en Windows `/c/Users/…` no coincide con `c:/users/…`. Cada uno excluye un par, y una R cuesta hasta 3,60 USD repetirla. Tarea 29.

**W3. `untouched` de R no cubre todo archivo que puede llevar la decisión.** Faltan `FORK_DEFAULT_TIMEOUT` en `docs/user/advanced.rst`, `4.5, 41` en `adapters.py` y `HISTORY.md`, y los módulos donde se puede aplicar (`sessions.py`, `api.py`). Una sesión 1 que escribe la constante en la documentación («no toques el código todavía») aprueba la rama nueva sin relevo. Tarea 30.

**W4. `again` cuenta lo que se lee con memoria y no lo que se redescubre con `Grep` o `Glob`.** Las respuestas de `search_memory` suman bajo `a3`, y lo que `a0` vuelve a encontrar con `Grep` no. La comparación «su sobrecosto no supera al de `a0`» queda inclinada hacia `a0`. La regla de H1 no cambia, porque se fijó antes de correr: la revisión informa `again` por herramienta junto al veredicto.

**W5. Un `Stop` que falla después del `Play` de una rama no aplica su `Keep`** (`run.rs:1180`): su `stream-2.jsonl` queda en la raíz, `costs` no la cuenta y la rama se puntúa con una sesión. Tarea 31.

**Sugerencias.** `not_continued` compara con `<` estricto: la revisión mira cuánto bajó el contexto antes de excluir por (c). La exención por `session_id` vale sólo para la sesión propia. `score_dir(...).unwrap_or_default()` (`main.rs:454`) calla un error al puntuar (tarea 32). `PairRow` agrupa sin el ruido, que en R es uno solo.

### Lo que queda para la guía

Los topes de `--max-budget-usd` por tarea y la cuenta contra el tope de la campaña, que pasó a 30 USD; `--claude-version` 2.1.257 o posterior; el canario; una sesión del humo que intenta leer fuera y sale negada, y la lista de `outside_reads` de cada corrida del humo; el texto de `eza` y el primer pedido de las sesiones de E bajo `a0`; que H6 se lee en el check `command_runs` de cada `result.json`, porque el reporte no muestra los intentos; y que `again` se informa por herramienta.

### Lo que pidió, hecho

| Punto | Commit | Lo fija |
| --- | --- | --- |
| W2, falsos `read_outside` | `6145d95a` | `pattern` es ruta sólo en `Glob`; `Bash` salta `/dev/null`; `drive_form` lee `/c/…` como `c:/…`, pura. Tests `a_grep_pattern_is_no_path`, `dev_null_is_no_read_outside`, `a_drive_in_git_bash_form_is_the_same_path` |
| W3, `untouched` de R | `31a8044b` | los dos trazadores en `adapters.py`, `sessions.py`, `api.py`, `advanced.rst` y `HISTORY.md`, ninguno ya en el fixture (comprobado con `rg -uu -F`). Test `the_relay_decision_stays_untouched_wherever_it_could_be_applied` |
| W5, el `Keep` tras otro paso que falla | `138d719e` | `branch_to_keep`, pura, da la última rama que jugó y no guardó. Test `a_branch_keeps_its_files_when_the_daemon_does_not_stop` |
| W1 y el error de puntuar | `5bf17808` | `Command::Score` hace absoluta su ruta; un error de `score_dir` tras un fallo se suma al mensaje. Test `score_reads_a_relative_run_as_absolute`; la rama del mensaje no tiene test propio |

Sobre `5bf17808`, en `target-test`: `cargo test --workspace` pasa, los dos `cargo clippy` pasan y `agent-eval tasks` sale con 0 y lista los dos ruidos. W4 queda como se dijo: la regla de H1 no cambia y la revisión informa `again` por herramienta.

## La campaña

Se corrieron el humo (6 corridas), la fase 2 (20) y la fase 4 (9), con Claude Code 2.1.322 y Sonnet 5.5 en esfuerzo `high`. La fase 3 (60) corrió después, con los mismos programas: está en [H2 y H4](#h2-la-escala-y-h4-el-orden-del-memorymd). Ninguna corrida tuvo un artefacto, ninguna sesión compactó y las lecturas de afuera que se intentaron las negó el cliente, también en las corridas (la sonda salió negada). Gasto según `agent-eval costs`: 1,00 USD el humo, 7,09 la fase 2 y 0,91 la fase 4, 9,01 de los 30 del tope; el estimado de esas tres fases era 19,36.

Las celdas son las de las fases 2 y 4. Las corridas del humo no suman a su celda: después del humo cambió el arnés (las cuentas de la rama continua, `9f933ab0`).

### H1, el relevo

El contexto al bifurcar fue de 62.180 (`a0`), 66.865 (`a3`) y 69.734 (`a4`) tokens de mediana, no los 150.000 de la condición de validez. El resultado se informa como **régimen de unos 66.000 tokens**. Las dos tareas son válidas: la forma continua de R aprueba 3 de 3 en los tres brazos, y D continua bajo `a0` aprueba 2 de 2.

| Brazo | R: pares con la continua aprobada y la nueva fallada | D partida | Sobrecosto evitable (mediana) | Veredicto |
|---|---:|---:|---:|---|
| `a0` | 0 de 3 | 0 de 3 | 4.418 | no es seguro |
| `a3` | 0 de 3 | 3 de 3 | 3.491 | **rinde** |
| `a4` | 1 de 3 | 3 de 3 | 4.482 | no es seguro |

Bajo las mismas reglas, en la campaña de protocol-block: `a3-short` rinde (0 de 3, D 3 de 3, 70 tokens) y `l0` no es seguro (2 de 3 pares con la nueva fallada).

Lo que es por construcción o no decide:
- `a0` es seguro en R y no en D. En R la decisión viaja por `TIMEOUTS.md`, que escribe la sesión 1, y la sesión nueva la encuentra sin memoria. En D, el 0 de 3 de `a0` es por construcción: sin memoria, la sesión 3 no tiene de dónde saber la decisión revisada.
- El par que falla en `a4` (repetición 3) encontró la decisión en `TIMEOUTS.md` y se detuvo a preguntar por una duda que la sesión 1 dejó abierta ahí. No es un artefacto: cuenta como falla.
- `again` va de 186 a 4.968 tokens por rama nueva. `result.json` no lo separa por herramienta. Bajo `a0`, que no tiene herramientas de memoria, es todo `Read`. W4 sigue valiendo: lo que `a0` vuelve a encontrar con `Grep` no se cuenta.
- El ahorro de cada par sale negativo en los tres brazos (`a0` −74,5 %, `a3` −32,5 %, `a4` −49,3 %): con la caché caliente y 66.000 tokens de contexto, seguir la conversación costó menos que empezar una nueva.

Qué sigue, según la regla: `a3` rinde y `a0` no es seguro, así que se propone el aviso de corte y el resumen de relevo sobre lo que clawg ya tiene. El resultado vale para el régimen medido. Con ahorros negativos a 66.000 tokens, el umbral del aviso no puede salir de esta campaña: hace falta una R que llegue a 150.000 tokens o más.

### H2, la escala, y H4, el orden del `MEMORY.md`

El canario pasó en el humo: Δ = 19.454 − 17.740 = 1.714 tokens, dentro de 500 a 3.000, así que el modelo de carga de 200 líneas cuadra y la fase 3 podía empezar.

La fase 3 corrió después, con los mismos programas de las otras fases: 60 corridas, ninguna excluida, 3,37 USD de los 8 del tope (el estimado era unos 5).

**H2: clawg no aguanta, según la regla.** Es medible: `a3` con 15 reglas aprueba la K larga 3 de 3.

| Condición de «clawg aguanta» | Resultado | Cumple |
|---|---|---|
| K larga: `a3` con 300 aprueba al menos tantas como con 15 | 3 de 3 contra 3 de 3 | sí |
| S, C y N con 300: `a3` y `l0` aprueban 2 de 2 | S 2 y 2, N 2 y 2, **C bajo `a3` 1 de 2**, C bajo `l0` 2 de 2 | **no** |
| K larga: la mediana del primer pedido de `a3` con 300 no supera en más de 500 tokens a la de 15 | 20.576 contra 20.524 (+52) | sí |

Sin «aguanta» no hay «ventaja en escala». En la K larga con 300, `a3` aprueba 3 y `a4` 1: la diferencia de 2 corridas estaba, pero la regla la pide sobre un clawg que aguanta.

La C que falla no la decide la escala sino el arnés. La sesión buscó, recibió las dos decisiones opuestas sobre los paquetes internos (`==` y `~=`), leyó las dos con `read_memory` y eligió `~=` «porque tiene el id más alto y se lee como la respuesta a la primera». `seed.rs` numera las memorias de la más nueva a la más vieja (`seeded` y `id: i + 1`), así que en el fixture un id más alto es una memoria más vieja, al revés que en un store real. Con 300 reglas la decisión vigente quedó en `#9` y la vieja en `#142`. Las causas de exclusión no incluyen este caso, así que cuenta como falla y el veredicto queda. La otra corrida de C bajo `a3` no buscó y aprobó.

**Arreglado en el arnés.** `export` numera ahora las memorias de la más vieja a la más nueva, como las numera un store, y conserva cada fecha. Las campañas de antes se sembraron con el orden invertido: en una tarea con dos memorias que se contradicen, una corrida nueva no se compara con ellas. Test `the_export_numbers_the_memories_from_the_oldest_to_the_newest_as_a_store_does`.

Lo que muestran los datos y no decide:
- Con 300 reglas `a3` falla K1, K2 y K3 (0 de 6), como con 15 en el piloto (0 de 6): no es un efecto de la escala.
- `a4` con 300 falla K1, K2 y K3 (0 de 6) y la K larga 2 de 3, contra 6 de 6 y 3 de 3 con 15. Es por construcción: con 300 reglas la memoria de la tarea queda fuera de las 200 líneas, y en las dos K largas que fallan el agente no la fue a buscar al archivo.
- Con 300 reglas aparece `synthesize`: 19 llamadas (12 de `l0`, 7 de `a3`), al terminar la tarea, porque la apertura avisa que hay memorias sin revisar. Con 15 reglas nadie lo llama. Agrega 1.776 tokens de mediana y su resultado tiene 1.302, por debajo de 1.500: según H5 no es candidata a una dieta, pero es un costo que crece con el store y no hace la tarea.

**H4: C bajo `a4` con 15 reglas y el orden corregido aprueba 2 de 2.** Se informa junto al 2 de 2 del piloto y al 0 de 2 de la campaña anterior: el 0 de 2 era el orden del archivo. No decide nada.

### H3, el ciclo de vida

- `a3` mantiene los hechos que deciden. `FORK_DEFAULT_TIMEOUT` sale `usado` en 3 de 3 en las dos ramas de R. `_:_` sale `usado` en 3 de 3 en D. `[hidden]` sale `llegó y no se usó` en 3 de 3, y ninguna corrida `usó la primera`.
- `a4` no escribió su memoria nativa en R: los dos hechos salen `no guardado` en las seis ramas, y el relevo pasó por `TIMEOUTS.md`. En D mantiene `_:_` (3 de 3), y `[hidden]` sale `no guardado`.
- En `a0` todo sale `no guardado`, por construcción.

### H5, el costo por llamada

Ninguna herramienta de clawg queda como candidata a una dieta.
- `save_memory` agrega 716 tokens de mediana en la fase 2 y 504 en protocol-block, con un resultado de 2 a 23 tokens.
- `search_memory`, `recall_context` y `search_project` no se pudieron medir solas.
- El resultado de `search_project` tiene 2.252 tokens de mediana en protocol-block, por encima de 1.500, pero sin la medida de lo que agrega no entra en la regla. Queda para mirar.

### H6, el error repetido

- **Hay efecto:** bajo `a0`, la sesión 3 intenta `eza` en 3 de 3.
- **La memoria de hoy no lo evita:** `a3-notools` y `a4` intentan `eza` en la sesión 3 en 3 de 3 cada uno.

repeated-errors tiene su línea base y su meta: 0 intentos en 3 de 3. El texto del shell fue `/usr/bin/bash: line 1: eza: command not found` en todas las sesiones. Después de fallar en Bash, el agente casi siempre prueba `eza` en la herramienta PowerShell, que pide una aprobación que nadie da, y termina contando con `Glob`. H6 cuenta sólo Bash, igual en todos los brazos.

## Al archivar (2026-11-17)

- AS1 a AS22 pasan con sus ids a [`specs/eval`](../../../specs/eval/spec.md), en «`agent-eval`: las sesiones, el relevo y los artefactos». AS1 reemplaza el orden del `MEMORY.md` de `a4` en MD3, y MD3 suma además la numeración de los ids del export, de la más vieja a la más nueva, que arregló la campaña (H2). Las enmiendas AS13 a AS22 quedan como requisitos propios, y los requisitos que modifican (AS2, AS3, AS5, AS6, AS9 y AS10) los nombran.
- Ninguna spec del producto cambia. `docs/reference/testing.md` ya describe las sesiones, la bifurcación, los artefactos y el reporte.
- Sale en 0.88.0. Todo el código está en `develop`: el CI de Linux y Windows pasó sobre `9a76a8ec`, y después sólo cambió `crates/clawg-eval` (el número de los ids del export) y documentos.
- Las cuatro hipótesis con respuesta (H1, H2, H4 y H6) están en «La campaña». H3 y H5 se informan sin decidir nada.

## Pendiente después de publicar

Local, con una clave de API y autorización de gasto; nada de esto es de la suite.

- Volver a correr la celda C de la escala: `amb-ts-semver-pin-conflict` bajo `a3` con 300 reglas. Falló por los ids invertidos del export, que `seed.rs` ya numera como un store; la campaña de H2 no se puede comparar con una corrida nueva en esa tarea.
- Medir el relevo en el régimen de 150.000 tokens o más. La campaña lo midió a unos 66.000 tokens, así que el umbral del aviso de corte no sale de ella.
- Ninguna sesión de ningún brazo guardó un `session_summary`: sin eso, el traspaso entre sesiones del producto no se puso a prueba con esta campaña.
