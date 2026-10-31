# perf-bench — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal (soporte de medición) | M | 71 y 73, archivados: el banco lee `GET /v1/metrics` para el desglose. 57 y 62 lo usan como puerta | ver [`state.yaml`](state.yaml) |

## Problema

Las métricas de uso (71, 73) dicen qué pasó en una ejecución, pero son observacionales: entre dos versiones cambian también el tamaño del store, las consultas, la carga y la máquina. Hoy cada verificación mide a mano y una vez (la latencia del save en 36, la memoria por recarga en 46, los planes en 56, el transporte en 59), y no hay una corrida que se repita igual en la próxima versión y diga si algo empeoró. Verificado contra el código de hoy:

- **El único banco que arranca el binario es de transporte.** `crates/clawg-cli/tests/transport_bar.rs` (`#![cfg(windows)]`, `:11`; `#[ignore]`, `:308`) mide `/health` y una lista contra un store de 60 filas (`seed_and_calibrate`, `:176-206`), con un cliente que espera cada respuesta antes de mandar la siguiente (`run`, `:222-295`): no mide búsquedas, contextos ni saves, y un daemon que se traba esconde cuántas peticiones habrían esperado detrás.
- **`rag-eval --agent-view` da p50 y p99 sobre 46 casos** (`crates/clawg-eval/src/bin/rag-eval/agent_view/stats.rs:28-29`), en proceso, sin carga concurrente ni guardados.
- **Las puertas de 57 y 62 piden mediciones que nadie produce**: la espera de un save, de `session-start` y de una búsqueda detrás de un refresco de docs o de un save largo ([read-connections](../read-connections/exploration.md)), y la CPU en reposo y bajo carga ([embedder-footprint](../embedder-footprint/exploration.md)).
- **No hay con qué comparar dos corridas de latencia.** `rag-eval --diff` compara casos pareados (`crates/clawg-eval/src/bin/rag-eval/diff.rs:283`); dos corridas de un banco mandan peticiones distintas en instantes distintos, así que se comparan distribuciones.

## Objetivo

Transversal: que una versión se pueda declarar más lenta que otra con un número que se repite, y que 57 y 62 corran sus puertas sobre un banco común. No cambia el daemon, ningún dato guardado ni ningún ranking.

## Decisiones

1. **Un test ignorado en `crates/clawg-cli/tests/perf_bench.rs`, como `transport_bar`** (enfoque A de la [exploración](exploration.md)). Usa el binario de la suite (`CARGO_BIN_EXE_clawg`), un `testing` build, con `--port 0`, un home temporal y el embedder falso. Corre en todos los sistemas, no sólo en Windows: las preguntas de 57 y 62 son de los dos. CI nunca lo corre (`#[ignore]`), y las pruebas de sus partes puras corren con la suite.
2. **La siembra pasa por `POST /v1/import`** (decisión abierta de la exploración). Con una semilla fija el banco arma un export `clawg-memories` (`crates/clawg-core/src/memory/portability/format.rs`): cuatro agentes, tres proyectos y filas sin proyecto, una décima `superseded`, una vigésima borrada, palabras frecuentes y raras. Lo importa el daemon por el mismo camino que `clawg import`, que embebe y escribe en lotes como producción. Se descarta sembrar por SQL directo: copiaría el esquema en el test, y cada migración lo rompería en silencio. Se descarta sembrar por el worker save por save: 10 000 saves de a uno es medir el worker, no sembrar. La corrida de prueba sembró 10 000 filas en 10,6 s en un build debug.
3. **La carga es de lazo abierto.** Cada petición sale en el instante que su tasa le asigna, sin esperar a las anteriores, y su latencia se cuenta desde ese instante: un daemon que se traba paga por cada petición que esperó detrás (sin omisión coordinada). Las tasas están por debajo de lo que un build debug atiende con 10 000 filas, para que el banco mida latencia y no una cola que crece.
4. **Escenarios.** Solos, uno detrás de otro: `search`, `context`, `session_start` (el contexto sin consulta del hook), `save_1`, `save_10` y `save_100` (saves de 1, 10 y 100 pedazos). Y con una operación larga de fondo: un save de 100 pedazos, un backup y el primer refresco del índice de docs de 400 archivos Markdown, mientras se miden `search`, `context`, `session_start` y `save_1` a la vez (`{flujo}_during_{fondo}`). Son las puertas de 57. El save y el backup de fondo se repiten hasta cubrir 10 s, para que uno rápido deje muestras.
5. **Un archivo de resultado por corrida.** `docs/eval/results/{fecha}-{fake|real}-perf-bench-{filas}[-{tag}].json`, que git ignora (`.gitignore:31`), nunca sobre uno existente. Lleva la versión, el build, el embedder, la máquina (sistema, arquitectura, hilos), la semilla y el tamaño, cada escenario con sus muestras crudas y su resumen (p50, p95, p99, máximo), la CPU del proceso en reposo y bajo carga, su pico de memoria (la puerta de 62) y los histogramas de la ejecución de `GET /v1/metrics` sin sus buckets, más los gauges `embedder.*` del minuto. Se escribe también con el embedder falso: mide SQLite, ranking e ingest, que es para lo que el banco existe.
6. **El modelo real es opcional.** `CLAWG_BENCH_MODEL_DIR` nombra un directorio de datos cuyo `models/` se copia al home temporal; el daemon lo verifica contra sus SHA-256 como en cualquier arranque. Nunca se lee un perfil por defecto: quien corre el banco con el modelo nombra el directorio, y eso necesita autorización.
7. **`perf-bench diff A B` en `clawg-eval`.** Un binario nuevo compara dos archivos escenario por escenario: el p50 y el p95 de cada lado, el cambio relativo de los dos con un intervalo bootstrap del 95 % (cada corrida remuestreada por su lado) y la prueba de rangos de Mann-Whitney como lectura. **Umbral** (decisión abierta de la exploración): un escenario es `slower` si su p95 sube más del 20 % y el intervalo deja fuera el cero, o si la segunda corrida tiene fallos que la primera no; con menos de 20 muestras por lado no decide. `--fail-on-regression` sale con 6, como `rag-eval --diff`. Avisa cuando las corridas difieren en build, embedder, máquina, filas o semilla.

## Alcance

- `perf_bench.rs`: la siembra, las consultas y los textos con semilla, la carga de lazo abierto, los escenarios, la CPU y la memoria del proceso del daemon por su pid (Linux: `/proc/<pid>/stat` y `status`; Windows: `GetProcessTimes` y `GetProcessMemoryInfo`), el archivo y su nombre, la configuración por variables de entorno (`CLAWG_BENCH_ROWS`, `CLAWG_BENCH_SEED`, `CLAWG_BENCH_TAG`, `CLAWG_BENCH_MODEL_DIR`). Las partes puras con tests que corren en la suite.
- `crates/clawg-eval/src/bin/perf-bench/`: el `diff`, su estadística pura y su ayuda.
- `docs/reference/testing.md` y `specs/eval` («Banco de rendimiento del daemon»).

## No-alcance

- Correr el banco en CI o publicar sus resultados con el release.
- Perfilado (`ort` con profiling, `tokio-console`, `perf`, `samply`, WPR).
- Las decisiones de 57 y 62: este cambio les da el instrumento, no corre sus puertas con el modelo real.
- Cambiar el daemon para medir: lo que el banco necesita ya lo exponen `/v1/import`, `/v1/backup`, `/v1/projects`, `/v1/metrics` y el proceso.

## Cómo se mide el éxito

Tests que hoy fallan (no existen) y corren en la suite sin modelo ni red: la siembra es la misma para una semilla y otra para otra, el import la lee entera y tiene las proporciones pedidas; las consultas, los textos y el corpus de docs se repiten; un save de 100 pedazos cabe en 64 KiB; el calendario, el orden de los flujos y el cuantil por rango; la configuración y el nombre del archivo; un archivo nunca se reemplaza; la CPU de `/proc/<pid>/stat` con un comando que lleva paréntesis. En `clawg-eval`: el cuantil, `erfc`, Mann-Whitney contra un valor calculado a mano, el bootstrap determinista, el veredicto en tabla, los avisos y la lectura del archivo.

Y el banco se verifica a sí mismo, como pide la exploración, en un build debug con el embedder falso y 10 000 filas:

- Dos corridas sobre el mismo commit: `perf-bench diff` no marca ningún escenario `slower`.
- Una tercera con una espera de 50 ms inyectada en `POST /v1/search` (sin commitear), la mitad del p95 de una búsqueda en ese build: `diff` marca `slower` los escenarios de búsqueda y ningún otro.
- La suite y los dos clippy pasan.

## Datos y vuelta atrás

No cambia nada guardado, ningún perfil se lee ni se escribe y el binario que se distribuye no cambia (el banco es un test; `perf-bench` es de `clawg-eval`, que release nunca enlaza). Para volver atrás se revierten los commits y se borran los archivos `*-perf-bench-*.json` de `docs/eval/results/`, que git no ve.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Una tasa por encima de lo que el daemon atiende mide la cola, no la latencia | Tasas elegidas con el build debug y 10 000 filas; el archivo lleva las muestras crudas, y un p50 que crece con el orden de envío se ve |
| Dos corridas en máquinas o builds distintos se leen como un cambio del código | `diff` avisa de cada diferencia de build, embedder, máquina, filas y semilla |
| La varianza de una máquina de escritorio da un falso `slower` | El umbral del 20 % y el intervalo que debe dejar fuera el cero; la verificación corre el mismo commit dos veces |
| El refresco de docs y el backup varían con el disco | Se miden como fondo, no como escenario: lo que se compara son las peticiones que esperan detrás |
