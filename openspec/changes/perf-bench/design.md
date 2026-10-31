# perf-bench — Diseño

Decisiones con alternativas. Requisitos en la [spec](spec.md) (PB1 a PB8); alcance en la [propuesta](proposal.md). Lo puro (siembra, calendario, cuantiles, configuración, nombres, estadística) está separado de lo que hace I/O (el daemon, el disco, el proceso).

## D1. Dónde vive el banco (PB1)

| Opción | A favor | En contra |
|---|---|---|
| **a. Un test ignorado en `crates/clawg-cli/tests/perf_bench.rs`** (elegida) | `CARGO_BIN_EXE_clawg` es el `testing` build que lee `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER`; `transport_bar.rs` ya probó el patrón; las partes puras se prueban en la suite | Un test que escribe en `docs/eval/results/`; se corre con `--ignored` |
| b. Un binario `perf-bench` en `clawg-eval` que arranca el daemon | Junto al `diff` | `clawg-eval` no tiene un `testing` build del daemon a mano: tendría que compilarlo o pedir su ruta, y el embedder falso sólo lo lee un build con esa feature |
| c. Bancos de instrucciones en CI (`gungraun`) | Estables en CI | Sin E/S ni Windows; los tiempos de pared en runners compartidos no sirven |

El banco no es `#![cfg(windows)]`: las preguntas de 57 (la conexión única) y 62 (los hilos del embedder) son de los dos sistemas. La CPU y la memoria del proceso del daemon se leen por su pid con lo que cada sistema da (D6).

## D2. Cómo se siembra (PB2)

| Opción | A favor | En contra |
|---|---|---|
| **a. Un export `clawg-memories` armado con los tipos de `portability::format` y mandado a `POST /v1/import`** (elegida) | El camino de producción: valida, enmascara, embebe en lotes y escribe índices y FTS como `clawg import`. Los tipos (`Line`, `MemoryLine`, `render`) son `pub` y el test los usa, así que un campo nuevo rompe la compilación y no la siembra en silencio | Un cuerpo de varios MB por HTTP (el límite es 256 MiB, `MAX_IMPORT_BYTES`) |
| b. SQL directo con `rusqlite` | Lo más rápido | Copia el esquema (vectores, FTS, proyectos) en el test; cada migración lo rompe o, peor, lo deja sembrando un store que el daemon no habría escrito |
| c. Un save por fila por el worker | Ejercita el ingest | 10 000 a 100 000 saves de a uno miden el worker; tarda órdenes de magnitud más |

Los proyectos van como líneas `project` con nombre y sin carpeta, así el import crea uno por nombre; el banco lee sus ids con `GET /v1/projects`. El texto empieza con `Row {id}:`, así dos filas nunca son iguales y la deduplicación no se lleva ninguna. La hora (`now_ms`) es un parámetro: con la misma hora el export es el mismo byte a byte, y la edad de las filas (hasta 365 días) cuenta desde ella.

## D3. El generador (PB2)

SplitMix64, copiado como en `rag-eval` (`stats.rs`) y `golden-mine`: una secuencia fija por semilla, sin depender de la versión de `rand`. Las palabras frecuentes se eligen con el cuadrado de un uniforme sobre 64 (la primera sale en casi todas las filas); un identificador `identN` sale una vez cada 25 palabras, entre `filas / 20`, así cada uno lo comparten unas pocas filas y el canal léxico de identificadores tiene trabajo. Cada consulta, save y petición deriva su generador de la semilla, del nombre del escenario (FNV-1a) y de su número, de modo que agregar un escenario no mueve los textos de los otros.

## D4. La carga de lazo abierto (PB3)

| Opción | A favor | En contra |
|---|---|---|
| **a. Cada petición en una tarea propia, lanzada en su instante (`sleep_until`), latencia desde ese instante** (elegida) | Una traba del daemon se cobra en todas las peticiones que la esperaron; el calendario no depende de las respuestas | Por encima de la capacidad la cola crece y el escenario mide la cola |
| b. Lazo cerrado, como `transport_bar` | Simple | Omisión coordinada: durante una traba no sale nada, y el p99 la esconde |
| c. Tasa adaptativa hasta la saturación | Da la capacidad | Otra pregunta; dos versiones no se comparan con la misma carga |

Las tasas se eligieron con una corrida debug de 10 000 filas: a 40 búsquedas por segundo la cola crecía hasta 20 s, y con las de ahora cada escenario solo queda por debajo de un tercio de lo que el daemon atiende. Durante una operación de fondo la cola sí crece, porque el save largo ocupa la única conexión del store: eso es lo que 57 pregunta. Varios flujos a la vez se intercalan con `earliest`, pura: sale el que vence antes, y en un empate el primero.

## D5. Cuándo termina una operación de fondo (PB4)

El save y el backup responden cuando terminan, así que su tarea marca el fin al volver. El refresco de docs lo lanza `POST /v1/projects` en segundo plano (`spawn_doc_refresh`, `crates/clawg-cli/src/daemon/api.rs`), así que el banco espera a que el histograma `docs.refresh.duration` de `GET /v1/metrics` cuente uno más, consultando cada 250 ms: una lectura que no toca el store. Se descartó esperar con `POST /v1/projects/docs/search`, que espera el refresco pero pasa por el embedder y el store y entraría en lo medido. Un backup de un store chico tarda decenas de milisegundos y dejaría una o dos muestras; por eso el save y el backup de fondo se repiten hasta 10 s.

## D6. La CPU y la memoria del daemon (PB5)

| Opción | A favor | En contra |
|---|---|---|
| **a. Leer el proceso hijo por su pid desde el test** (elegida) | Ningún cambio en el daemon; en Linux `/proc/<pid>/stat` (campos 14 y 15, en ticks de `sysconf(_SC_CLK_TCK)`) y `parse_proc_status` (ya `pub`) para `VmHWM`; en Windows `OpenProcess` con `PROCESS_QUERY_LIMITED_INFORMATION`, `GetProcessTimes` y `GetProcessMemoryInfo`, con las features de `windows-sys` que `clawg-cli` ya declara | Código por sistema en un test; en macOS queda en `null` |
| b. Los gauges `process.*` de `GET /v1/metrics` | Ya existen | El escritor los muestrea al cambiar de minuto (`specs/metrics` R6): la CPU del minuto en curso no está, y el pico de una corrida de 3 minutos depende de dónde caen los minutos |
| c. Una función `sample_of(pid)` en `daemon::process` | Comparte el código de Windows | Código en el binario que se distribuye que sólo usa un test ignorado |

`cpu_ratio` es la CPU usada sobre el tiempo de reloj entre dos lecturas, sin dividir por núcleos, como `process.cpu_ratio`. El parseo de `stat` toma los campos después del último `)`, porque el nombre del comando puede llevar espacios y paréntesis.

## D7. Qué del daemon va al archivo (PB5)

Los histogramas de la ejecución (`run.histograms`) son el desglose: `http.duration` por ruta, `search.duration` por rama, `ingest.stage` por etapa, `docs.refresh.duration`, `embedder.wait`. Se guardan sin `buckets`, que repiten lo que dicen `p50`/`p95`/`p99` y multiplican el tamaño. De los gauges sólo los `embedder.*` del minuto (`embedder.batch_tokens`, la pregunta de 62): los gauges no tienen agregado de la ejecución, y el resto (proceso, store) lo cubre D6 o no cambia entre versiones.

Las muestras crudas van al archivo porque el `diff` remuestrea; con las tasas de D4 son unos pocos cientos por escenario.

## D8. El `diff` (PB7)

| Opción | A favor | En contra |
|---|---|---|
| **a. Un binario `perf-bench` en `clawg-eval` con `diff`** (elegida) | `clawg-eval` es donde viven las herramientas de comparación y nunca entra al release; lee los JSON sin depender del test | Un binario más |
| b. `rag-eval --perf-diff` | Comparte `stats.rs` | Mezcla dos instrumentos; `rag-eval` compara casos pareados |
| c. El diff dentro del test, con variables de entorno | Un archivo | Un test que compara archivos de otras corridas no es un test |

La estadística es otra que la de `rag-eval --diff`: no hay casos pareados sino dos muestras independientes. El bootstrap remuestrea cada lado por su cuenta y calcula el cambio relativo del cuantil; el cuantil es el de rango más cercano, el mismo del banco, y el de cada remuestreo sale con `select_nth_unstable` (lineal) en lugar de ordenar. Mann-Whitney U se informa como lectura: detecta un corrimiento de toda la distribución, pero con cientos de muestras separa diferencias de 2 % que no importan, así que el veredicto lo decide el p95 con el umbral. `erfc` usa el ajuste de Chebyshev de Numerical Recipes (error relativo menor que 1,2e-7), sin dependencia nueva. Las funciones se copian (`SplitMix64`, el cuantil) en lugar de compartirse, como hacen `rag-eval` y `revision-eval`: los binarios de `clawg-eval` no tienen biblioteca común.

**Umbral.** 20 % de p95 con el intervalo fuera de cero, el candidato de la exploración. Un umbral sin intervalo marcaría el ruido de una máquina de escritorio; un intervalo sin umbral, con cientos de muestras, marcaría cambios de pocos por ciento. Los fallos nuevos son regresión aunque las latencias no se muevan. Con menos de 20 muestras por lado el p95 es el máximo o casi, y no se decide.

## D9. Lo que queda fuera de los tests

El banco mismo (arrancar, sembrar, cargar, leer el proceso) es I/O y es lo que verifican las corridas de PB8; correrlo en la suite tomaría minutos. Sus piezas puras tienen tests en el mismo archivo, que corren sin `--ignored`. El código de Windows de D6 lo compila el clippy de CI en Windows (`--all-targets`).
