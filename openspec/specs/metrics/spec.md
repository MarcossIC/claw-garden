# metrics — Especificación

Fuente de verdad del comportamiento actual de las métricas de uso del daemon: qué registra mientras atiende, qué guarda de eso en `<data>/metrics.db`, cuánto lo guarda, y cómo se lee (`GET /v1/metrics`, `clawg metrics report`). Las rutas del daemon, el guard y los transportes están en [`specs/daemon-access`](../daemon-access/spec.md); el apagado y sus fases, en [`specs/storage`](../storage/spec.md).

**Origen:**
- [usage-metrics](../../changes/archive/2026-10-22-usage-metrics/proposal.md), archivado el 2026-10-22.

## Definiciones

- **Serie:** un nombre de métrica y sus etiquetas, escritas `k=v` ordenadas por clave y unidas por `,` (`canonical_labels`, `crates/clawg-core/src/metrics/recorder.rs:33`). Un `,` o un `=` dentro de un valor pasa a `_`.
- **Histograma:** cuentas por bucket de fronteras fijas según la unidad, más `count`, `sum`, `min` y `max` (`crates/clawg-core/src/metrics/histogram.rs`).
- **Gauge:** las muestras de un valor en una ventana: `count`, `sum`, `min`, `max` y `last`.
- **Minuto:** `epoch_ms / 60 000`, en UTC. **Hora:** el minuto dividido por 60.
- **Ejecución:** un `clawg serve`, desde que abre `metrics.db` hasta que se apaga.

## R1. Un histograma de buckets fijos que se suma

Un histograma MUST contar cada valor en el primer bucket cuya frontera superior lo contiene, con fronteras fijas por unidad (`MICROS_BOUNDS`, `histogram.rs:17`; `BYTES_BOUNDS`, `:25`):

- microsegundos (`us`): 50, 100, 250, 500, 1 000, 2 500, 5 000, 10 000, 25 000, 50 000, 75 000, 100 000, 250 000, 500 000, 750 000, 1 000 000, 2 500 000, 5 000 000, 7 500 000, 10 000 000, 30 000 000, 60 000 000, 300 000 000 y un bucket abierto;
- bytes: 256, 1 KiB, 4 KiB, 16 KiB, 64 KiB, 256 KiB, 1 MiB, 4 MiB, 16 MiB, 64 MiB, 256 MiB y un bucket abierto.

Sumar dos histogramas de la misma unidad (`merge`, `:107`) MUST dar el de la unión de sus muestras; sumar unidades distintas MUST fallar sin cambiar nada. Un percentil (`quantile`, `:132`) MUST interpolarse dentro del bucket del valor de rango `ceil(q·count)` y acotarse con `min` y `max`, así que su error es a lo sumo el ancho de ese bucket. Los buckets MUST guardarse como `u64` little-endian y leerse de vuelta iguales; una cantidad de bytes que no corresponde a la unidad MUST rechazarse.

**Tests:** `merging_two_histograms_is_the_histogram_of_the_union`, `a_percentile_falls_in_the_bucket_of_the_exact_one`, `a_single_value_is_every_percentile`, `a_value_on_a_bound_belongs_to_the_bucket_it_closes`, `histograms_of_different_units_do_not_merge`, `the_buckets_round_trip_through_bytes` en `histogram.rs`.

## R2. El registro por minuto

`Recorder` (`recorder.rs:104`), en `AppCtx::metrics`, MUST acumular por serie un histograma del minuto en curso y uno de toda la ejecución, gauges del minuto y el último valor de cada gauge. `take_minute` (`:130`) MUST devolver el minuto y dejarlo vacío sin tocar la ejecución. Una serie sin datos en el minuto MUST NOT aparecer en él. `writing` MUST decir si el escritor está guardando en `metrics.db`.

**Tests:** `taking_the_minute_empties_it_and_keeps_the_run`, `labels_are_canonical_whatever_their_order`, `a_gauge_keeps_its_extremes_and_last_value` en `recorder.rs`.

## R3. `metrics.db`, aparte del store

El daemon MUST escribir las métricas en `<data>/metrics.db` con su propia conexión, nunca en `memory.db`; ningún snapshot, `restore`, `purge`, `export`, `import` ni `doctor` MUST tocarlo. El esquema es `PRAGMA user_version = 1` con las tablas `run`, `hist_minute`, `hist_hour`, `gauge_minute` y `gauge_hour` (`SCHEMA`, `crates/clawg-core/src/metrics/store.rs:337`); un fichero de un esquema más nuevo MUST rechazarse sin tocarlo.

Al arrancar, el escritor (`Writer::start`, `crates/clawg-cli/src/daemon/metrics.rs:146`) MUST registrar la ejecución (`run`: inicio, versión, `dist_target`, perfil, SO, arquitectura, CPUs, modelo de embedding, minutos de `CLAWG_EMBEDDER_IDLE_MINUTES` con `0` para nunca soltarlo, desfase de la hora local en segundos) con `stopped_at_ms` nulo. En cada cambio de minuto del reloj (`run_writer`, `:261`) MUST muestrear el proceso (R6) y escribir el minuto anterior en una tarea bloqueante; una serie de un minuto ya escrito MUST sumarse, nunca reemplazarse. Un apagado ordenado MUST escribir el minuto en curso y `stopped_at_ms` después de que paran los servidores, dentro de `METRICS_CLOSE` (1 s, `crates/clawg-cli/src/daemon/serve.rs:88`), una de las fases que `STOP_TIMEOUT` cubre; pasado el plazo, o con el proceso matado, `stopped_at_ms` MUST quedar nulo.

En Unix el fichero MUST crearse `0600` (`create_private`, `store.rs:541`). Un fichero que SQLite no reconoce (`NotADatabase`, `DatabaseCorrupt`) MUST apartarse como `metrics.db.corrupt-<ms>`, con sus bytes, y recrearse, con un `warn` (`MetricsDb::open`, `store.rs:131`). Un fallo al abrirlo o al escribir MUST registrarse una vez como `warn` y apagar la escritura hasta el próximo arranque (`Writer::flush`, `metrics.rs:201`), sin afectar ninguna petición; el registro en memoria y `GET /v1/metrics` siguen.

`CLAWG_METRICS=off`, en cualquier caso y con espacios alrededor, MUST apagar la escritura: no se crea ni se toca `metrics.db` (`writing_enabled`, `metrics.rs:112`). Cualquier otro valor la deja encendida, también en `prod`.

**Tests:** `a_run_is_recorded_open_and_closed_on_stop`, `a_minute_is_written_with_its_series`, `a_corrupt_file_is_set_aside_and_recreated`, `a_newer_schema_is_refused_untouched`, `the_file_is_private_to_its_owner` (Unix) en `store.rs`; `metrics_off_is_the_only_way_to_turn_writing_off`, `a_writer_turned_off_creates_nothing`, `each_minute_and_the_stop_are_written`, `a_failed_write_turns_writing_off_and_serving_goes_on`, `a_file_that_cannot_be_opened_turns_writing_off` en `crates/clawg-cli/src/daemon/metrics.rs`; `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` en `stop.rs`; `metrics_off_leaves_no_metrics_file` en `crates/clawg-cli/tests/binary.rs`.

## R4. Retención

Los minutos MUST guardarse `MINUTES_KEPT_MS` (14 días, `store.rs:24`); los más viejos MUST sumarse en la hora de su ejecución (buckets sumados; gauges con `count` y `sum` sumados, `min` de los `min`, `max` de los `max` y el `last` del minuto más nuevo) y borrarse en la misma transacción (`prune`, `:214`). Las horas MUST guardarse `HOURS_KEPT_MS` (400 días, `:27`), y una ejecución de más de 400 días sin filas MUST borrarse. El escritor MUST aplicarlo al arrancar y una vez por hora.

**Tests:** `old_minutes_fold_into_their_hour`, `hours_past_the_retention_are_dropped` en `store.rs`.

## R5. Latencia HTTP

Un middleware sobre el router entero, por fuera del guard y del token (`time_request`, `crates/clawg-cli/src/daemon/metrics.rs:75`; `crates/clawg-cli/src/daemon/api.rs:900`), MUST contar toda petición, por el socket o por TCP, a cualquier ruta, `/health` incluida y también las que el guard (403) o el token (401) rechazan:

- `http.duration` (`us`, desde que llega hasta que se forma la respuesta) con `method` (`GET`, `POST`, `PATCH`, `DELETE`, `PUT`, `HEAD`, `OPTIONS`, si no `other`), `route` (la plantilla que axum emparejó, como `/v1/memories/{kind}/{id}`, o `unmatched`), `status` (`1xx` a `5xx`) y `transport`;
- `http.request_bytes` (bytes, de `Content-Length`; `0` sin él o con uno inválido) con `method` y `route`;
- `http.in_flight`, un gauge de las peticiones en curso, que baja también cuando el cliente se va.

`transport` MUST ser `unix` o `tcp` según el servidor: `serve` marca cada copia del router con `router_on` (`api.rs:904`; `serve.rs:246`, `:255`), y un router sin marca (los tests en proceso, `serve_ephemeral`) es `tcp`. Ningún valor de etiqueta MUST venir de la URL concreta, de la consulta ni del cuerpo, que nunca se lee.

**Tests:** `every_request_is_timed_by_route_status_and_transport`, `a_server_marked_unix_labels_its_requests_so`, `refused_requests_are_timed_too`, `no_label_comes_from_the_request` en `crates/clawg-cli/tests/api.rs`; `a_status_falls_in_its_class`, `a_size_without_a_valid_content_length_is_zero` en `daemon/metrics.rs`.

## R6. Proceso y runtime

En cada cambio de minuto y al apagarse, el escritor MUST registrar como gauges (`Writer::sample`; las lecturas en `crates/clawg-cli/src/daemon/process.rs`):

| Gauge | Linux | Windows | Otros Unix |
|---|---|---|---|
| `process.rss_bytes`, `process.rss_peak_bytes` | `VmRSS`, `VmHWM` de `/proc/self/status` | `WorkingSetSize`, `PeakWorkingSetSize` | — |
| `process.private_bytes` | `RssAnon` | `PrivateUsage` | — |
| `process.threads` | `Threads` | — | — |
| `process.open_files` | entradas de `/proc/self/fd` | — | — |
| `process.handles` | — | `GetProcessHandleCount` | — |
| `process.cpu_ratio` | `getrusage` | `GetProcessTimes` | `getrusage` |

`process.cpu_ratio` es el tiempo de CPU del proceso (usuario y sistema) desde la muestra anterior sobre el tiempo de reloj entre las dos, sin dividir por núcleos; falta en la primera muestra. También `embedder.resident` (1 o 0, si hay embedder), `rt.alive_tasks` y `rt.global_queue_depth` de tokio, y `rt.worker_busy_ratio`: el tiempo ocupado de todos los workers desde la muestra anterior sobre el tiempo de reloj por el número de workers. Lo que un sistema no da MUST faltar, nunca valer 0. El desfase local de la ejecución sale de `localtime_r` (`tm_gmtoff`) en Unix y de `GetTimeZoneInformation` en Windows (`utc_offset_s`, `process.rs:152`, `:167`).

**Tests:** `the_linux_status_file_is_read_in_bytes`, `a_ratio_is_the_delta_over_the_wall_time`, `the_sample_of_this_process_has_its_memory` (Linux y Windows), `the_utc_offset_is_within_a_day`, `the_runtime_sample_counts_its_workers` en `process.rs`; `each_minute_and_the_stop_are_written` en `daemon/metrics.rs`.

## R7. `GET /v1/metrics`

Detrás del token (`metrics`, `api.rs:1424`), MUST devolver en JSON `version`, `started_at_ms`, `writing`, el minuto en curso (`minute.histograms` con `name`, `labels`, `unit`, `count`, `sum`, `min`, `max`, `p50`, `p95`, `p99` y `buckets` con su `le`, `null` para el abierto; `minute.gauges` con `count`, `mean`, `min`, `max` y `last`) y `run.histograms`, los de toda la ejecución. Con `?format=prometheus` MUST devolver los histogramas de la ejecución y el último valor de cada gauge en el formato de texto de Prometheus, `text/plain; version=0.0.4` (`prometheus_text`, `metrics.rs:396`): nombres `clawg_<nombre con _>`, los `us` en segundos con sufijo `_seconds`, los bytes con sufijo `_bytes`, buckets acumulados con `le` (`+Inf` el último), `_sum` y `_count`, y los valores de etiqueta escapados. Otro `format` MUST ser un 400. No lee `metrics.db`.

**Tests:** `metrics_needs_the_token_and_answers_the_minute_and_the_run`, `metrics_speaks_prometheus_text` en `crates/clawg-cli/tests/api.rs`; `a_label_value_is_escaped_for_prometheus` en `daemon/metrics.rs`.

## R8. `clawg metrics report`

`clawg metrics report --metric M [--label K=V]... [--by day|hour|version] [--since Nd|Nh] [--profile P]` MUST abrir `metrics.db` en sólo lectura, sin daemon y sin el lock del store, y MUST imprimir un JSON (`crates/clawg-cli/src/metrics.rs:14`; la agregación pura en `report`, `crates/clawg-core/src/metrics/report.rs:66`) con `metric`, `kind` (`histogram` o `gauge`), `unit` (de un histograma), `by`, `since_ms`, `runs` y `groups`:

- los grupos suman minutos y horas desde `--since` (por defecto `30d`) de las series de nombre `M` que tienen cada par `K=V`;
- `--by version` (por defecto) agrupa por la versión de la ejecución, en el orden en que cada una corrió primero; `--by day`, por la fecha local; `--by hour`, por la hora del día local, `00` a `23`, juntando todos los días. La hora local de una fila es la de su ejecución (`utc_offset_s`), así que un cambio de horario a mitad de una ejecución corre una hora su agrupación;
- cada grupo de un histograma trae `count`, `mean`, `min`, `max`, `p50`, `p95` y `p99`; el de un gauge, `samples`, `mean`, `min` y `max`; los dos, `runs` y `not_stopped` de las ejecuciones con filas en el grupo;
- `runs` cuenta todas las ejecuciones que empezaron desde `--since`, con o sin filas: `total` y `not_stopped` (sin `stopped_at_ms`: matadas, caídas o todavía en marcha);
- una ventana sin filas de una métrica conocida da `groups` vacío con su `kind`.

Sin `metrics.db` MUST salir con 1 diciendo que el daemon lo escribe salvo con `CLAWG_METRICS=off`, sin crearlo; con una métrica que no existe, con 1 y los nombres que hay. `--label` sin `=` o con un lado vacío, y un `--since` que no sea un entero de días u horas, son errores de uso.

**Tests:** `a_report_groups_by_version_day_and_hour`, `a_label_filter_keeps_only_matching_series`, `a_gauge_report_gives_samples_and_extremes` en `report.rs`; `a_report_reads_the_file_without_a_daemon`, `a_window_without_rows_keeps_the_kind_of_the_metric`, `a_missing_file_or_metric_is_named` en `crates/clawg-cli/src/metrics.rs`; `metrics_report_takes_a_metric_labels_a_grouping_and_an_age` en `cli.rs`.

## R9. De punta a punta

**Tests** en `crates/clawg-cli/tests/binary.rs`: `a_daemon_records_its_requests_and_report_reads_them_after_stop` (un daemon con `--port 0` atiende un guardado y una búsqueda por el socket y por TCP; `GET /v1/metrics` trae cada búsqueda con su `transport` y `writing: true`; tras `clawg stop`, `report` cuenta las dos con `not_stopped: 0`, una con `transport=unix`, y lee `process.rss_bytes` como gauge); `a_killed_daemon_leaves_its_run_not_stopped` (una ejecución parada y una matada dan `runs: {total: 2, not_stopped: 1}`); `metrics_off_leaves_no_metrics_file`.

## Invariantes

- Nada de esto sale de la máquina, y ninguna etiqueta ni fila lleva texto de una memoria, una consulta, una ruta de fichero o un id.
- `memory.db`, su esquema y lo que ve el modelo no cambian por las métricas.
- Borrar `metrics.db` sólo pierde la historia de métricas; el próximo arranque lo crea de nuevo.

## Límites conocidos

- Sin eventos por ejecución (cargas del modelo, recuperaciones, mantenimiento) ni métricas del ingest, la búsqueda por rama, el embedder o los docs: es pipeline-metrics (73).
- `clawg metrics diff` con pruebas estadísticas no existe; `report --by version` da los números que compararía.
- Sin etiqueta de agente: el `agent_id` es autodeclarado y abriría series sin cota.
- `process.threads` falta en Windows (pediría recorrer un snapshot de Toolhelp) y la memoria falta en macOS.
- El sobrecoste por petición no se midió con el modelo real.
