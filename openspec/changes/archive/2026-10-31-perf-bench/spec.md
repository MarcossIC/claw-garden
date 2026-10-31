# perf-bench — Especificación

Suma una sección a [`specs/eval`](../../../../specs/eval/spec.md): «Banco de rendimiento del daemon». Ninguna otra spec cambia: el daemon, `metrics.db` y lo guardado quedan como están. Origen: [proposal](proposal.md).

Los tests de PB1 a PB6 están en `crates/clawg-cli/tests/perf_bench.rs` y corren con la suite (sin modelo, sin red, sin dejar nada en el temporal); los de PB7 y PB8, en `crates/clawg-eval/src/bin/perf-bench/`. El banco mismo (`bench`) es `#[ignore]`.

## PB1. El banco

`crates/clawg-cli/tests/perf_bench.rs` MUST tener un test `#[ignore]`, `bench`, que se corre con `cargo test -p clawg-cli --test perf_bench -- --ignored --nocapture` (con `--release` mide un daemon optimizado), en Linux, Windows y macOS. MUST arrancar el binario de la suite con `serve --profile dev --port 0`, un home temporal (`CLAWG_TEST_HOME`) y, salvo PB6, `CLAWG_TEST_EMBEDDER=fake`, y MUST terminarlo por su propio handle al final, falle o no.

Lo configuran estas variables, leídas por una función pura:

| Variable | Valor | Por defecto |
|---|---|---|
| `CLAWG_BENCH_ROWS` | filas sembradas, de 1 a 1 000 000 | 10 000 |
| `CLAWG_BENCH_SEED` | la semilla, un entero sin signo | 74 |
| `CLAWG_BENCH_TAG` | minúsculas, dígitos y guiones, para el nombre del archivo | ninguno |
| `CLAWG_BENCH_MODEL_DIR` | un directorio de datos con `models/` (PB6) | el embedder falso |

Un valor fuera de esto MUST hacer fallar el banco nombrando la variable, antes de arrancar nada.

#### Escenario: la configuración por defecto
- GIVEN ninguna variable
- WHEN se lee la configuración
- THEN son 10 000 filas, semilla 74, sin tag, embedder falso, y el archivo se llama `{fecha}-fake-perf-bench-10000.json`

**Tests:** `the_config_reads_its_defaults_and_refuses_what_it_cannot_use`.

## PB2. La siembra

El banco MUST sembrar el store con un export `clawg-memories` (`crates/clawg-core/src/memory/portability/format.rs`) que arma con la semilla y manda a `POST /v1/import`, y MUST fallar si el import no inserta todas las filas o declara alguna inválida. El export MUST:

- ser el mismo byte a byte para la misma semilla, filas y hora, y otro para otra semilla;
- repartir las filas entre cuatro agentes, tres proyectos (cada uno con su línea `project`) y un cuarto sin proyecto;
- dejar alrededor de una décima `superseded` (cada una por la fila siguiente, con causa `revision`) y una vigésima borrada;
- escribir textos de 12 a 40 palabras tomadas de 64 frecuentes, las primeras mucho más que las últimas, y, una de cada 25, un identificador `identN` de `filas / 20` posibles.

Las consultas MUST salir de la misma semilla: dos a cuatro palabras frecuentes, y un tercio con un identificador. Cada save MUST llevar un texto único (no lo absorbe la deduplicación) con las palabras que dan sus pedazos con el tokenizer falso de una palabra por token: 40 para uno, `96 + 72 × (n − 1)` para n, y a lo sumo 64 KiB (el límite de `content`). El corpus de docs MUST ser de 400 archivos Markdown de ocho secciones, también de la semilla.

#### Escenario: el import lee la siembra entera
- GIVEN 2 000 filas con la semilla por defecto
- WHEN el parser del import lee el export
- THEN no hay líneas inválidas, hay 2 000 memorias y 3 proyectos, entre 150 y 250 `superseded`, entre 50 y 130 borradas, entre 400 y 600 sin proyecto y más de 300 por agente

**Tests:** `the_seeded_store_is_the_same_for_a_seed_and_another_for_another`, `the_seeded_store_is_an_export_the_import_reads_whole`, `rows_share_frequent_words_and_few_share_an_identifier`, `queries_and_saves_repeat_for_a_seed`, `a_save_has_the_words_of_its_pieces_and_fits_the_content_limit`, `the_docs_corpus_repeats_for_a_seed`.

## PB3. La carga de lazo abierto

Cada petición MUST salir en el instante que le da su tasa (la i-ésima a `i / tasa` del comienzo del escenario), en una tarea propia, sin esperar a las anteriores, y su latencia MUST contarse desde ese instante hasta la respuesta completa. Una respuesta que no es 2xx o un error de transporte MUST contarse como fallo y no como muestra. Cuando varios flujos corren a la vez, MUST salir primero el que vence antes, y ante un empate el primero de la lista.

Antes de medir, el banco MUST dejar el daemon 5 s sin peticiones (la CPU en reposo) y mandar unas peticiones de calentamiento que no cuentan.

#### Escenario: tres flujos a la vez
- GIVEN flujos de 20, 10 y 2 por segundo
- WHEN se toman ocho peticiones
- THEN salen en el orden (0, 0 ms), (1, 0), (2, 0), (0, 50), (0, 100), (1, 100), (0, 150), (0, 200)

**Tests:** `requests_leave_at_a_fixed_rate`, `streams_send_in_the_order_their_requests_fall_due`.

## PB4. Los escenarios

Solos, uno después del otro, MUST correr:

| Escenario | Petición |
|---|---|
| `search` | `POST /v1/search` con una consulta, un agente y un proyecto (o ninguno) de la semilla, `limit` 10 |
| `context` | `POST /v1/context` con consulta |
| `session_start` | `POST /v1/context` sin consulta, como el hook al empezar una sesión |
| `save_1`, `save_10`, `save_100` | `POST /v1/memories` de 1, 10 y 100 pedazos, que responde cuando el save quedó guardado |

Y, para cada operación de fondo (`save_100`, `backup` con `POST /v1/backup`, `docs_refresh` con `POST /v1/projects` del corpus con `index_docs`), los flujos `search`, `context`, `session_start` y `save_1` a la vez mientras la operación corre, como `{flujo}_during_{fondo}`. El save y el backup de fondo MUST repetirse hasta cubrir 10 s; el refresco corre una vez y termina cuando `docs.refresh.duration` de `GET /v1/metrics` cuenta uno más. Cada snapshot del backup MUST borrarse en cuanto se escribe.

## PB5. El archivo de resultado

Al final el banco MUST escribir `docs/eval/results/{fecha UTC}-{fake|real}-perf-bench-{filas}[-{tag}].json` y MUST NOT reemplazar uno existente: si ya está, falla antes de arrancar el daemon. Lleva:

| Campo | Qué es |
|---|---|
| `format`, `version` | `clawg-perf-bench`, 1 |
| `clawg`, `build`, `embedder` | la versión, `debug` o `release`, `fake` o `real` |
| `machine` | `os`, `arch`, `threads` (`available_parallelism`) |
| `started_at_ms` | cuándo empezó |
| `store` | `rows`, `seed`, `seed_seconds` (lo que tardó el import), `by_state` del import, `docs_files` y la respuesta de `GET /v1/stats` |
| `process` | `cpu_idle_ratio` y `cpu_load_ratio` (CPU del proceso sobre tiempo de reloj, sin dividir por núcleos), `rss_peak_bytes`, `private_peak_bytes` (sólo Windows); lo que un sistema no da es `null` |
| `scenarios[]` | `name`, `rate_per_s`, `sent`, `failed`, `background_seconds` y `background_runs` (sólo los de fondo), `summary` (`count`, `mean_us`, `p50_us`, `p95_us`, `p99_us`, `max_us`, por rango más cercano) y `samples_us`, las muestras crudas |
| `metrics` | `histograms`: los de la ejecución de `GET /v1/metrics` sin `buckets`; `embedder_gauges`: los gauges `embedder.*` del minuto |

El banco MUST fallar después de escribir el archivo si algún escenario tuvo fallos. Ningún campo lleva texto de una memoria.

#### Escenario: un archivo nunca se reemplaza
- GIVEN un archivo escrito
- WHEN se escribe otra vez en la misma ruta
- THEN falla y el archivo conserva su contenido

**Tests:** `a_quantile_is_the_nearest_rank`, `the_date_of_a_file_is_utc`, `a_result_file_is_never_replaced`, `the_cpu_of_a_process_is_read_after_its_command`.

## PB6. El modelo real

Con `CLAWG_BENCH_MODEL_DIR`, el banco MUST copiar `models/` de ese directorio al directorio de datos temporal antes de arrancar, MUST arrancar el daemon sin `CLAWG_TEST_EMBEDDER` y MUST esperar a que `/health` diga `embedder: ready`. Un directorio sin `models/` MUST hacer fallar el banco antes de arrancar. El banco MUST NOT leer un perfil de datos que no se le nombró.

## PB7. `perf-bench diff`

```
perf-bench diff FIRST SECOND [--fail-on-regression]
```

`crates/clawg-eval/src/bin/perf-bench/` MUST leer los dos archivos, rechazar con salida 1 uno que no sea `clawg-perf-bench` versión 1 o no se pueda leer, y salir con 2 ante argumentos inválidos. Para cada escenario que está en los dos MUST dar:

- `n` de cada lado, el p50 y el p95 (rango más cercano);
- el cambio relativo del p50 y del p95 (segundo sobre primero, menos uno) con un intervalo bootstrap percentil del 95 %: 4 000 remuestreos, cada lado con reemplazo por su cuenta, semilla fija, así los mismos dos archivos dan siempre el mismo intervalo;
- el valor p bilateral de Mann-Whitney U, con la aproximación normal, la corrección por empates y la de continuidad;
- un veredicto: `slower` si la segunda corrida tiene más fallos que la primera, o si el p95 subió más del 20 % y el límite inferior del intervalo es mayor que cero; `faster` si bajó más del 20 % y el límite superior es menor que cero; `too few samples` con menos de 40 muestras de algún lado; si no, `no change`.

MUST avisar (`warning:`) cuando las corridas difieren en `build`, `embedder`, `machine`, `store.rows` o `store.seed`, y nombrar cada escenario que está en una sola. Con `--fail-on-regression` MUST salir con 6 si algún escenario es `slower`; si no, con 0.

#### Escenario: la misma corrida dos veces
- GIVEN un archivo comparado consigo mismo
- WHEN se calcula el diff
- THEN no hay avisos, cada escenario es `no change` y el valor p de rangos es 1

#### Escenario: un escenario más lento
- GIVEN un escenario cuyas 400 muestras subieron 8 ms sobre una base de 20 ms
- WHEN se calcula el diff
- THEN es `slower`, el valor p de rangos es menor que 1e-6 y `--fail-on-regression` sale con 6

**Tests:** `a_quantile_is_the_nearest_rank`, `erfc_matches_known_values`, `mann_whitney_matches_a_value_computed_by_hand`, `mann_whitney_sees_no_difference_between_equal_samples`, `mann_whitney_sees_a_shift_of_the_whole_distribution`, `the_bootstrap_is_deterministic_for_a_seed`, `the_same_distribution_gives_an_interval_around_zero`, `a_slower_run_gives_an_interval_above_zero`, `an_empty_side_or_a_zero_quantile_is_no_change` (`stats.rs`); `the_verdict_needs_the_threshold_and_an_interval_without_zero`, `the_same_run_twice_is_no_change`, `a_slower_scenario_is_a_regression`, `what_differs_between_the_runs_is_named`, `a_result_file_is_read_by_its_format_and_ignores_what_it_does_not_compare` (`diff.rs`); `diff_takes_two_files_and_the_flag`, `anything_else_is_refused` (`main.rs`).

## PB8. El banco se verifica a sí mismo

El cambio MUST cerrarse con tres corridas del banco en un build debug, con el embedder falso y 10 000 filas, anotadas en `verify.md`: dos sobre el mismo commit, cuyo `diff` no marca ningún escenario `slower`, y una con una espera de 50 ms inyectada al comienzo de `POST /v1/search` y sin commitear, cuyo `diff` contra la primera marca `slower` el escenario `search` y ninguno que no busque: la espera no ocupa la conexión del store, así que el resto no debe moverse.
