# metrics — Especificación

Fuente de verdad del comportamiento actual de las métricas de uso del daemon: qué registra mientras atiende (las peticiones, el proceso, el store, la búsqueda, el ingest, el embedder y los docs), qué eventos de cada ejecución guarda, qué cuenta de cada sesión del cliente (Claude Code o Cursor), qué guarda de eso en `<data>/metrics.db`, cuánto lo guarda, y cómo se lee (`GET /v1/metrics`, `clawg metrics report`, `clawg metrics sessions`). Las rutas del daemon, el guard y los transportes están en [`specs/daemon-access`](../daemon-access/spec.md); el apagado y sus fases, en [`specs/storage`](../storage/spec.md).

**Origen:**
- [usage-metrics](../../changes/archive/2026-10-22-usage-metrics/proposal.md), archivado el 2026-10-22.
- [ram-footprint](../../changes/archive/2026-10-24-ram-footprint/proposal.md), archivado el 2026-10-24 (`process.private_peak_bytes` en R6, que es RF3 de esa spec, y `--by minute` en R8, que es RF2).
- [pipeline-metrics](../../changes/archive/2026-10-30-pipeline-metrics/proposal.md), archivado el 2026-10-30: P1 a P9 de esa spec son R10 a R18, P10 entra en R9, y cambia R2, R3, R4, R7 y R8.
- [search-arm-timing](../../changes/archive/2026-11-03-search-arm-timing/proposal.md), archivado el 2026-11-03, que sale en 0.73.0: A1 a A3 de esa spec cambian R14 (qué mide cada brazo de `search.duration` y cómo esperan su turno en la conexión).
- [session-start-context](../../changes/archive/2026-11-04-session-start-context/proposal.md), archivado el 2026-11-04, que salió en 0.76.0: SC4 de esa spec suma a R14 el bloque `workspace` de `context.cut`.
- [retire-legacy-kinds](../../changes/archive/2026-11-07-retire-legacy-kinds/proposal.md), archivado el 2026-11-07, que sale en 0.79.0 (sólo los brazos de `search.duration`, sin `knn_episodic` ni `knn_skill`, y `search.knn_retry`, que sólo lleva `table` `memory`: lo que [`specs/retrieval`](../retrieval/spec.md) RL4 quita).
- [session-usage](../../changes/archive/2026-11-11-session-usage/proposal.md), archivado el 2026-11-11, sale en 0.81.0: SU1 a SU12 de esa spec, con los mismos ids; cambian R3 y R4, el primer invariante y el límite «Sin etiqueta de agente».

## Definiciones

- **Serie:** un nombre de métrica y sus etiquetas, escritas `k=v` ordenadas por clave y unidas por `,` (`canonical_labels`, `crates/clawg-core/src/metrics/recorder.rs:38`). Un `,` o un `=` dentro de un valor pasa a `_`.
- **Histograma:** cuentas por bucket de fronteras fijas según la unidad, más `count`, `sum`, `min` y `max` (`crates/clawg-core/src/metrics/histogram.rs`).
- **Gauge:** las muestras de un valor en una ventana: `count`, `sum`, `min`, `max` y `last`.
- **Minuto:** `epoch_ms / 60 000`, en UTC. **Hora:** el minuto dividido por 60.
- **Ejecución:** un `clawg serve`, desde que abre `metrics.db` hasta que se apaga.
- **Recuento:** lo que cuenta ocurrencias, o lo que no es tiempo ni bytes, se registra como gauge, y el recuento es su número de muestras (`samples` de `metrics report`). No hay una unidad `count` de histograma: un binario anterior no la reconocería al plegar los minutos y dejaría de escribir métricas.
- **Evento:** algo raro que hace una ejecución, una fila con su `kind`, de un conjunto cerrado (`EventKind`, `crates/clawg-core/src/metrics/event.rs:11`), y un `detail` de números y valores `&'static str` (`DetailValue`).
- **Sesión:** una sesión del cliente (Claude Code o Cursor), nombrada por el id que el cliente da en sus hooks.
- **Fila:** una fila de la tabla `session`. Una sesión tiene una fila, o más de una si siguió después de cerrarse (SU3).
- **Evento de hook:** lo que un hook manda al daemon con el id exacto de la sesión: un arranque (`start`), un prompt (`prompt`) o un cierre.
- **Actividad atribuida:** lo que llega sin id de sesión y el daemon le asigna a la sesión vigente de un agente y un proyecto (`SessionRegistry::current`, `crates/clawg-cli/src/daemon/sessions.rs:67`): una llamada a una herramienta, un save aceptado o una búsqueda sin hits.
- **Sesión vigente:** la última que un hook registró para ese agente y ese proyecto en esta ejecución del daemon. Es la misma con la que se sella un save.
- **Conjunto cerrado:** una lista de valores escrita en el código. Un valor que no está en la lista se guarda como `other`.

## R1. Un histograma de buckets fijos que se suma

Un histograma MUST contar cada valor en el primer bucket cuya frontera superior lo contiene, con fronteras fijas por unidad (`MICROS_BOUNDS`, `histogram.rs:17`; `BYTES_BOUNDS`, `:25`):

- microsegundos (`us`): 50, 100, 250, 500, 1 000, 2 500, 5 000, 10 000, 25 000, 50 000, 75 000, 100 000, 250 000, 500 000, 750 000, 1 000 000, 2 500 000, 5 000 000, 7 500 000, 10 000 000, 30 000 000, 60 000 000, 300 000 000 y un bucket abierto;
- bytes: 256, 1 KiB, 4 KiB, 16 KiB, 64 KiB, 256 KiB, 1 MiB, 4 MiB, 16 MiB, 64 MiB, 256 MiB y un bucket abierto.

Sumar dos histogramas de la misma unidad (`merge`, `:107`) MUST dar el de la unión de sus muestras; sumar unidades distintas MUST fallar sin cambiar nada. Un percentil (`quantile`, `:132`) MUST interpolarse dentro del bucket del valor de rango `ceil(q·count)` y acotarse con `min` y `max`, así que su error es a lo sumo el ancho de ese bucket. Los buckets MUST guardarse como `u64` little-endian y leerse de vuelta iguales; una cantidad de bytes que no corresponde a la unidad MUST rechazarse.

**Tests:** `merging_two_histograms_is_the_histogram_of_the_union`, `a_percentile_falls_in_the_bucket_of_the_exact_one`, `a_single_value_is_every_percentile`, `a_value_on_a_bound_belongs_to_the_bucket_it_closes`, `histograms_of_different_units_do_not_merge`, `the_buckets_round_trip_through_bytes` en `histogram.rs`.

## R2. El registro por minuto

`Recorder` (`recorder.rs:113`) MUST acumular por serie un histograma del minuto en curso y uno de toda la ejecución, gauges del minuto y el último valor de cada gauge. `take_minute` (`:164`) MUST devolver el minuto y dejarlo vacío sin tocar la ejecución. Una serie sin datos en el minuto MUST NOT aparecer en él. `writing` (`:185`) MUST decir si el escritor está guardando en `metrics.db`.

El `Recorder` vive en el store (`MemoryStore::metrics`, `crates/clawg-core/src/memory/store/db.rs:328`), compartido por cada clon; `MemoryRuntime::metrics` (`crates/clawg-core/src/runtime.rs:162`) es ese mismo, y el daemon escribe y sirve el del runtime. La búsqueda, el worker, el outbox, los snapshots y la carga del modelo miden en él sin cambiar ninguna firma pública, y un runtime de test lleva el suyo, así dos runtimes de un proceso no mezclan sus series.

`Recorder::event` (`:140`) MUST encolar un evento hasta que el escritor lo toma con el minuto (`take_events`, `:154`, una sola vez) y guardar los últimos `RUN_EVENTS_KEPT` (100, `:14`) de la ejecución (`run_events`, `:159`).

**Tests:** `taking_the_minute_empties_it_and_keeps_the_run`, `labels_are_canonical_whatever_their_order`, `a_gauge_keeps_its_extremes_and_last_value`, `events_wait_for_the_writer_and_the_run_keeps_the_latest` en `recorder.rs`; `a_test_runtime_carries_its_own_recorder` en `crates/clawg-core/src/runtime.rs`.

## R3. `metrics.db`, aparte del store

El daemon MUST escribir las métricas en `<data>/metrics.db` con su propia conexión, nunca en `memory.db`; ningún snapshot, `restore`, `purge`, `export`, `import` ni `doctor` MUST tocarlo. El esquema es `PRAGMA user_version = 3` (`SCHEMA_VERSION`, `crates/clawg-core/src/metrics/store.rs:21`) con las tablas `run`, `hist_minute`, `hist_hour`, `gauge_minute`, `gauge_hour`, `event` (`run_id`, `at_ms`, `kind`, `detail` en JSON) y `session` y `session_count` (SU1; `SCHEMA`, `:521`). Un fichero del esquema 1 o 2 MUST recibir las tablas que le faltan (`event`, y `session` y `session_count`) cuando se abre para escribir, con `CREATE TABLE IF NOT EXISTS`, sin perder ni cambiar ninguna fila de las tablas que ya tenía; abierto en sólo lectura, uno del esquema 1 se lee como uno sin eventos ni sesiones, y uno del 1 o del 2, como uno sin sesiones. Un fichero de un esquema más nuevo MUST rechazarse sin tocarlo, así un binario anterior a los eventos o a las sesiones no escribe métricas hasta el próximo arranque de uno que los conoce.

Al arrancar, el escritor (`Writer::start`, `crates/clawg-cli/src/daemon/metrics.rs:146`) MUST registrar la ejecución (`run`: inicio, versión, `dist_target`, perfil, SO, arquitectura, CPUs, modelo de embedding, minutos de `CLAWG_EMBEDDER_IDLE_MINUTES` con `0` para nunca soltarlo, desfase de la hora local en segundos) con `stopped_at_ms` nulo. En cada cambio de minuto del reloj (`run_writer`, `:287`) MUST muestrear el proceso (R6) y escribir el minuto anterior en una tarea bloqueante, con los eventos encolados desde la escritura anterior y las filas de sesión que cambiaron (SU4; `Writer::flush`, `:207`); una serie de un minuto ya escrito MUST sumarse, nunca reemplazarse. Un apagado ordenado MUST registrar el evento `shutdown` después del drain y escribir entonces el minuto en curso y `stopped_at_ms`, dentro de `METRICS_CLOSE` (1 s, `crates/clawg-cli/src/daemon/serve.rs:88`; el orden en `:315-321`), una de las fases que `STOP_TIMEOUT` cubre, así lo que el drain ingirió y el propio evento caen en el último minuto; pasado el plazo, o con el proceso matado, `stopped_at_ms` MUST quedar nulo.

En Unix el fichero MUST crearse `0600` (`create_private`, `store.rs:742`). Un fichero que SQLite no reconoce (`NotADatabase`, `DatabaseCorrupt`) MUST apartarse como `metrics.db.corrupt-<ms>`, con sus bytes, y recrearse, con un `warn` (`MetricsDb::open`, `store.rs:138`). Un fallo al abrirlo o al escribir, sean minutos, eventos o sesiones, MUST registrarse una vez como `warn` y apagar la escritura hasta el próximo arranque (`Writer::flush`), sin afectar ninguna petición; el registro en memoria y `GET /v1/metrics` siguen.

`CLAWG_METRICS=off`, en cualquier caso y con espacios alrededor, MUST apagar la escritura: no se crea ni se toca `metrics.db` (`writing_enabled`, `metrics.rs:112`). Cualquier otro valor la deja encendida, también en `prod`. No apaga ninguna medición: el registro, la sonda del store (R10) y `GET /v1/metrics` siguen.

**Tests:** `a_run_is_recorded_open_and_closed_on_stop`, `a_minute_is_written_with_its_series`, `a_corrupt_file_is_set_aside_and_recreated`, `a_newer_schema_is_refused_untouched`, `a_file_of_schema_1_gets_its_events_and_keeps_its_rows`, `a_file_of_schema_2_gets_its_sessions_and_keeps_its_rows`, `the_file_is_private_to_its_owner` (Unix) en `store.rs`; `metrics_off_is_the_only_way_to_turn_writing_off`, `a_writer_turned_off_creates_nothing`, `each_minute_and_the_stop_are_written`, `a_failed_write_turns_writing_off_and_serving_goes_on`, `a_file_that_cannot_be_opened_turns_writing_off`, `the_events_of_a_minute_are_kept_with_it_and_served_with_the_run` en `crates/clawg-cli/src/daemon/metrics.rs`; `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` en `stop.rs`; `metrics_off_leaves_no_metrics_file` en `crates/clawg-cli/tests/binary.rs`.

## R4. Retención

Los minutos MUST guardarse `MINUTES_KEPT_MS` (14 días, `store.rs:28`); los más viejos MUST sumarse en la hora de su ejecución (buckets sumados; gauges con `count` y `sum` sumados, `min` de los `min`, `max` de los `max` y el `last` del minuto más nuevo) y borrarse en la misma transacción (`prune`, `:389`). Las horas, los eventos y las filas de sesión (SU5) MUST guardarse `HOURS_KEPT_MS` (400 días, `:32`), y una ejecución de más de 400 días sin filas de minutos, horas ni eventos MUST borrarse. El escritor MUST aplicarlo al arrancar y una vez por hora.

**Tests:** `old_minutes_fold_into_their_hour`, `hours_past_the_retention_are_dropped`, `events_past_the_retention_go_with_their_run`, `sessions_past_the_retention_are_dropped` en `store.rs`.

## R5. Latencia HTTP

Un middleware sobre el router entero, por fuera del guard y del token (`time_request`, `crates/clawg-cli/src/daemon/metrics.rs:75`; `crates/clawg-cli/src/daemon/api.rs:1198`), MUST contar toda petición, por el socket o por TCP, a cualquier ruta, `/health` incluida y también las que el guard (403) o el token (401) rechazan:

- `http.duration` (`us`, desde que llega hasta que se forma la respuesta) con `method` (`GET`, `POST`, `PATCH`, `DELETE`, `PUT`, `HEAD`, `OPTIONS`, si no `other`), `route` (la plantilla que axum emparejó, como `/v1/memories/{kind}/{id}`, o `unmatched`), `status` (`1xx` a `5xx`) y `transport`;
- `http.request_bytes` (bytes, de `Content-Length`; `0` sin él o con uno inválido) con `method` y `route`;
- `http.in_flight`, un gauge de las peticiones en curso, que baja también cuando el cliente se va.

`transport` MUST ser `unix` o `tcp` según el servidor: `serve` marca cada copia del router con `router_on` (`api.rs:1202`; `serve.rs:267`, `:276`), y un router sin marca (los tests en proceso, `serve_ephemeral`) es `tcp`. Ningún valor de etiqueta MUST venir de la URL concreta, de la consulta ni del cuerpo, que nunca se lee.

**Tests:** `every_request_is_timed_by_route_status_and_transport`, `a_server_marked_unix_labels_its_requests_so`, `refused_requests_are_timed_too`, `no_label_comes_from_the_request` en `crates/clawg-cli/tests/api.rs`; `a_status_falls_in_its_class`, `a_size_without_a_valid_content_length_is_zero` en `daemon/metrics.rs`.

## R6. Proceso y runtime

En cada cambio de minuto y al apagarse, el escritor MUST registrar como gauges (`Writer::sample`; las lecturas en `crates/clawg-cli/src/daemon/process.rs`):

| Gauge | Linux | Windows | Otros Unix |
|---|---|---|---|
| `process.rss_bytes`, `process.rss_peak_bytes` | `VmRSS`, `VmHWM` de `/proc/self/status` | `WorkingSetSize`, `PeakWorkingSetSize` | — |
| `process.private_bytes` | `RssAnon` | `PrivateUsage` | — |
| `process.private_peak_bytes` | — | `PeakPagefileUsage` | — |
| `process.threads` | `Threads` | — | — |
| `process.open_files` | entradas de `/proc/self/fd` | — | — |
| `process.handles` | — | `GetProcessHandleCount` | — |
| `process.cpu_ratio` | `getrusage` | `GetProcessTimes` | `getrusage` |

`process.cpu_ratio` es el tiempo de CPU del proceso (usuario y sistema) desde la muestra anterior sobre el tiempo de reloj entre las dos, sin dividir por núcleos; falta en la primera muestra. También `embedder.resident` (1 o 0, si hay embedder), `rt.alive_tasks` y `rt.global_queue_depth` de tokio, y `rt.worker_busy_ratio`: el tiempo ocupado de todos los workers desde la muestra anterior sobre el tiempo de reloj por el número de workers. Lo que un sistema no da MUST faltar, nunca valer 0. El desfase local de la ejecución sale de `localtime_r` (`tm_gmtoff`) en Unix y de `GetTimeZoneInformation` en Windows (`utc_offset_s`, `process.rs:162`, `:177`).

`process.private_peak_bytes` es el máximo de la memoria privada desde que arrancó el proceso, aunque haya durado menos de un minuto. Sale de la misma llamada a `GetProcessMemoryInfo` que da `PrivateUsage` (`ProcessSample::private_peak_bytes`, `process.rs:18`, leído en `:114`; el escritor, `crates/clawg-cli/src/daemon/metrics.rs:179`). Linux no lleva un pico de `RssAnon`, así que allí la serie MUST faltar.

**Tests:** `the_linux_status_file_is_read_in_bytes`, `a_ratio_is_the_delta_over_the_wall_time`, `the_sample_of_this_process_has_its_memory` (Linux y Windows; en Windows el pico es mayor o igual que `private_bytes`, en Linux falta), `the_utc_offset_is_within_a_day`, `the_runtime_sample_counts_its_workers` en `process.rs`; `each_minute_and_the_stop_are_written` en `daemon/metrics.rs`.

## R7. `GET /v1/metrics`

Detrás del token (`metrics`, `api.rs:1888`), MUST devolver en JSON (`metrics_body`, `metrics.rs:425`) `version`, `started_at_ms`, `writing`, el minuto en curso (`minute.histograms` con `name`, `labels`, `unit`, `count`, `sum`, `min`, `max`, `p50`, `p95`, `p99` y `buckets` con su `le`, `null` para el abierto; `minute.gauges` con `count`, `mean`, `min`, `max` y `last`), `run.histograms`, los de toda la ejecución, y `run.events`, los últimos eventos de la ejecución (R2), cada uno con `at_ms`, `kind` y `detail`. Con `?format=prometheus` MUST devolver los histogramas de la ejecución y el último valor de cada gauge, sin eventos, en el formato de texto de Prometheus, `text/plain; version=0.0.4` (`prometheus_text`, `metrics.rs:453`): nombres `clawg_<nombre con _>`, los `us` en segundos con sufijo `_seconds`, los bytes con sufijo `_bytes`, buckets acumulados con `le` (`+Inf` el último), `_sum` y `_count`, y los valores de etiqueta escapados. Otro `format` MUST ser un 400. No lee `metrics.db`.

**Tests:** `metrics_needs_the_token_and_answers_the_minute_and_the_run`, `metrics_speaks_prometheus_text` en `crates/clawg-cli/tests/api.rs`; `a_label_value_is_escaped_for_prometheus`, `the_events_of_a_minute_are_kept_with_it_and_served_with_the_run` en `daemon/metrics.rs`.

## R8. `clawg metrics report`

`clawg metrics report --metric M [--label K=V]... [--by day|hour|minute|version] [--since Nd|Nh] [--profile P]` MUST abrir `metrics.db` en sólo lectura, sin daemon y sin el lock del store, y MUST imprimir un JSON (`crates/clawg-cli/src/metrics.rs:58`; la agregación pura en `report`, `crates/clawg-core/src/metrics/report.rs:82`) con `metric`, `kind` (`histogram` o `gauge`), `unit` (de un histograma), `by`, `since_ms`, `runs` y `groups`:

- los grupos suman minutos y horas desde `--since` (por defecto `30d`) de las series de nombre `M` que tienen cada par `K=V`;
- `--by version` (por defecto) agrupa por la versión de la ejecución, en el orden en que cada una corrió primero; `--by day`, por la fecha local; `--by hour`, por la hora del día local, `00` a `23`, juntando todos los días; `--by minute`, por el minuto local, con la clave `AAAA-MM-DD HH:MM` (`By::Minute`, `report.rs:97`). La hora local de una fila es la de su ejecución (`utc_offset_s`), así que un cambio de horario a mitad de una ejecución corre una hora su agrupación;
- por minuto, el reporte MUST empezar en el mayor entre lo que pide `--since` y la primera hora entera de los minutos que se guardan (`since_for`, `report.rs:27`; R4), y `since_ms` MUST ser ese comienzo. Una fila más vieja es una hora plegada, y una hora que el último `prune` dejó a medias en el borde se leería como su primer minuto. Las otras agrupaciones empiezan en `--since`;
- cada grupo de un histograma trae `count`, `mean`, `min`, `max`, `p50`, `p95` y `p99`; el de un gauge, `samples`, `mean`, `min` y `max`; los dos, `runs` y `not_stopped` de las ejecuciones con filas en el grupo;
- `runs` cuenta todas las ejecuciones que empezaron desde `--since`, con o sin filas: `total` y `not_stopped` (sin `stopped_at_ms`: matadas, caídas o todavía en marcha);
- una ventana sin filas de una métrica conocida da `groups` vacío con su `kind`.

Sin `metrics.db` MUST salir con 1 diciendo que el daemon lo escribe salvo con `CLAWG_METRICS=off`, sin crearlo; con una métrica que no existe, con 1 y los nombres que hay. `--label` sin `=` o con un lado vacío, y un `--since` que no sea un entero de días u horas, son errores de uso.

`clawg metrics report --events [--since Nd|Nh] [--profile P]` MUST abrir `metrics.db` de la misma forma e imprimir, en lugar de una métrica, `since_ms` y `runs`: cada ejecución con eventos desde `--since` (por defecto `30d`), con `run`, `version`, `started_at_ms`, `stopped_at_ms` y sus `events` (`at_ms`, `kind`, `detail`), del más viejo al más nuevo (`events`, `crates/clawg-cli/src/metrics.rs:125`; `events_since`, `store.rs:236`). Un fichero del esquema 1 da `runs` vacío. `--events` con `--metric`, `--label` o `--by` es un error de uso, y sin `--events` la métrica es obligatoria.

#### Escenario: dos minutos de una ejecución tres horas detrás de UTC
- GIVEN filas de las 01:05 y las 02:00 UTC del 2026-10-20, de una ejecución con desplazamiento −3 h
- WHEN se agrupan por minuto
- THEN las claves son `2026-10-19 22:05` (con las dos series de ese minuto sumadas) y `2026-10-19 23:00`

#### Escenario: un `--since` más largo que los minutos guardados
- GIVEN `--since 30d` a las HH:20
- WHEN se reporta por minuto
- THEN el reporte empieza 14 días atrás, en la hora entera siguiente

**Tests:** `a_report_groups_by_version_day_and_hour`, `a_label_filter_keeps_only_matching_series`, `a_gauge_report_gives_samples_and_extremes`, `a_report_by_minute_keys_each_local_minute` y `a_report_by_minute_starts_where_the_minutes_are_kept` en `report.rs`; `a_report_reads_the_file_without_a_daemon`, `a_window_without_rows_keeps_the_kind_of_the_metric`, `a_missing_file_or_metric_is_named`, `a_report_by_minute_reads_the_minutes_of_the_file`, `the_events_are_listed_by_run` en `crates/clawg-cli/src/metrics.rs`; `metrics_report_takes_a_metric_labels_a_grouping_and_an_age` en `cli.rs`.

## R9. De punta a punta

**Tests** en `crates/clawg-cli/tests/binary.rs`: `a_daemon_records_its_requests_and_report_reads_them_after_stop` (un daemon con `--port 0` atiende un guardado y una búsqueda por el socket y por TCP; `GET /v1/metrics` trae cada búsqueda con su `transport` y `writing: true`; tras `clawg stop`, `report` cuenta las dos con `not_stopped: 0`, una con `transport=unix`, y lee `process.rss_bytes` como gauge); `a_killed_daemon_leaves_its_run_not_stopped` (una ejecución matada queda en `metrics.db` con `stopped_at_ms` nulo); `metrics_off_leaves_no_metrics_file`; `a_daemon_probes_its_store_without_being_asked` (las series de R10 a R13 sin ninguna petición); `a_daemon_measures_its_pipeline_and_keeps_its_events` (con el embedder falso guarda un texto de varias piezas, busca y refresca un índice de docs pequeño, encuentra en `GET /v1/metrics` las series de R10 a R17 y, tras `clawg stop`, los eventos de R18 en `metrics.db`; `embedder.batch`, `embedder.batch_tokens` y `embedder.wait` quedan para los tests del embedder, porque el modelo falso no tiene sesión que mida lotes).

## R10. La espera en la conexión del store

`MemoryStore::probe_wait` (`crates/clawg-core/src/memory/store/conn_probe.rs:28`) MUST tomar la hora antes de encolar en la única conexión de `memory.db` una llamada sin SQL y devolver, como primera instrucción de esa llamada en el hilo de la conexión, el tiempo transcurrido: la cola, no la sentencia. La tarea de la sonda (`spawn`, `crates/clawg-cli/src/daemon/store_probe.rs:96`) MUST sondear al arrancar y luego cada `PROBE_INTERVAL` (10 s, `:14`), con `MissedTickBehavior::Delay`, así una sonda que esperó detrás de una operación larga atrasa la siguiente en lugar de dejar una ráfaga, y registrar `db.probe_wait` (`us`, `probe`, `:39`). Una sonda que no corre (la conexión cerrada) MUST NOT registrar nada. `serve` MUST abortar la tarea cuando paran los servidores y antes del drain y del cierre del store (`serve.rs:311`), así ninguna sonda queda en la cola del cierre.

#### Escenario: una sonda detrás de una llamada larga
- GIVEN una llamada que ocupa la conexión 200 ms
- WHEN una sonda se encola detrás
- THEN la sonda mide una espera de al menos 150 ms

**Tests:** `the_probe_waits_behind_a_call_in_progress`, `a_closed_connection_is_an_error_not_a_reading` en `conn_probe.rs`; `a_probe_records_its_wait` en `store_probe.rs`.

## R11. La caché de SQLite

En la primera sonda y luego en cada sexta (`MINUTE_EVERY`, `store_probe.rs:17`), la lectura por minuto (`read_minute`, `:50`) MUST leer, en una llamada en la conexión, `sqlite3_db_status` por `rusqlite::ffi` (`MemoryStore::cache_status`, `conn_probe.rs:38`) y registrar como gauges `db.cache_hit`, `db.cache_miss`, `db.cache_write` y `db.cache_spill`, lo contado desde la lectura anterior porque cada lectura los pone a cero, y `db.cache_used_bytes` y `db.stmt_used_bytes`, lo que hay ahora. Un contador que SQLite no da MUST faltar.

#### Escenario: los contadores empiezan de cero en cada lectura
- GIVEN una lectura del estado y luego unas consultas
- WHEN se lee dos veces seguidas
- THEN la primera trae los aciertos de esas consultas y la segunda ninguno

**Tests:** `the_cache_counters_restart_at_each_reading` en `conn_probe.rs`.

## R12. El tamaño de los ficheros del store

En la misma lectura, en un `spawn_blocking` (`store_files`, `store_probe.rs:29`), el daemon MUST registrar el gauge `db.file_bytes` con `file` en `db` (`memory.db`), `wal` (`memory.db-wal`) y `backups` (la suma de los ficheros que están directamente en `<data>/backups`, sin bajar a sus subdirectorios). Un fichero o directorio que no existe MUST dejar su serie fuera, nunca en 0.

**Tests:** `the_store_files_are_measured_and_a_missing_one_is_left_out` en `store_probe.rs`.

## R13. El outbox en el tiempo

En la misma lectura, de la consulta que contesta el bloque `ingest` de `/health` (`OutboxSummary`), el daemon MUST registrar los gauges `outbox.pending` y `outbox.failed` y, con una fila pendiente, `outbox.oldest_pending_s`; sin filas pendientes, esa serie MUST faltar. Una parte de la lectura que falla (R11, R12 o R13) deja fuera sólo sus series, con un `debug`.

**Tests:** `a_minute_reading_records_the_cache_the_files_and_the_outbox` en `store_probe.rs`; `a_daemon_probes_its_store_without_being_asked` en `crates/clawg-cli/tests/binary.rs`.

## R14. Búsqueda y contexto

Cada búsqueda de memorias MUST registrar `search.duration` (`us`, `record_arm`, `crates/clawg-core/src/memory/retrieval/search.rs:1472`) con `arm` en `knn_memory`, `lexical`, `identifiers` y `total`, cada rama que corrió (una que el alcance apagó, o el canal léxico y los identificadores sin texto de consulta, no deja serie). Un brazo MUST ser el tiempo que sus llamadas corrieron en el hilo de la conexión, sin la espera en su cola: el reloj arranca dentro de la llamada (`KnnOutcome::ran`, `:439`; `run_fail_soft`, `:852`, para los identificadores y el canal léxico, que suma sus candidatos y el puntaje de los que el denso no trajo). El filtro de identificadores que corre después, fuera del hilo, no cuenta; un brazo sin nada que pedirle a la conexión (una consulta sin identificadores para `identifiers`, sin tokens para `lexical`) registra 0, y una consulta léxica o de identificadores que falla no hace fallar la búsqueda y cuenta lo que corrió. `total` MUST medir la búsqueda de punta a punta, esperas incluidas. Los brazos MUST mandarse a la conexión uno detrás de otro, cada uno cuando volvió el anterior, en el orden `identifiers`, `knn_memory` y el léxico (`:1326`): una llamada que llega durante la búsqueda puede correr entre dos, y un error de un KNN hace fallar la búsqueda sin mandar los que siguen. Hasta 0.72.2 cada brazo tomaba su reloj antes de encolarse y sumaba la espera detrás de lo que tenía delante, sus hermanos incluidos; el reloj de ahora sale en 0.73.0. Cada fila de `metrics.db` es de una ejecución con su versión, y `clawg metrics report --by version` (R8) separa los dos significados, que `--by day`, `--by hour` o `--by minute` juntan en una ventana que cruza la actualización. Cada búsqueda MUST registrar además el recuento `search.knn_retry` con `table` `memory` cuando el KNN se repitió con una ventana más ancha (`:1367`). `POST /v1/search` MUST registrar los gauges `search.hits` y `search.withheld` (`crates/clawg-cli/src/daemon/api.rs:1615`). Cada `POST /v1/context` MUST registrar (`record_context`, `api.rs:1778`) el recuento `context.retrieval_state` con `state` (`searched`, `recent`, `embedder_not_ready`, `embedder_failed`, `search_failed`), el histograma `context.prompt_bytes` (bytes) y, por bloque del que el presupuesto sacó algo, el recuento `context.cut` con `block` (`recent`, `long_term`, `last_session`, `conflicts`, y `workspace` por las entradas de su sección AUTO; `PromptBlock::as_str`, `crates/clawg-core/src/memory/retrieval/prompt_builder.rs:151`).

**Tests:** `a_search_records_how_long_each_arm_took`, `an_arm_measures_its_run_on_the_connection_not_its_wait`, `a_call_that_arrives_during_a_search_runs_between_its_arms`, `search_all_memories_inner_fails_soft_when_memory_fts_is_missing`, `a_widened_window_is_counted_by_its_table` en `search.rs`; `the_fit_names_each_block_it_cut` en `prompt_builder.rs`; `a_search_records_its_arms_its_hits_and_what_it_withheld`, `a_context_records_its_state_its_size_and_the_blocks_it_cut` en `crates/clawg-cli/tests/api.rs`.

## R15. El ingest por etapa

`ingest_pieces` (`crates/clawg-core/src/memory/ingest/worker.rs:949`) corre una función por etapa, `split_event` (`:808`), `gate_pieces` (`:850`), `embed_pieces` (`:920`) y `store_pieces` (`:1043`), y luego enlaza las piezas; partirla no cambió lo que guarda. Cada toma de un save MUST registrar `ingest.stage` (`us`, `record_stage`, `:747`) con `stage` (`split`, `gate`, `embed`, `store`, `link`) por cada etapa que corrió, y el recuento `ingest.outcome` con `outcome` (`stored`, `retrying`, `partially_saved`, `failed`; `outcome_of`, `:757`); una toma que un drain pausó no registra ninguno. Un save guardado MUST registrar además el gauge `ingest.pieces` y, con su fila del outbox, `ingest.end_to_end` (`us`, desde el `created_at` de la fila; `record_outcome`, `:771`). El worker MUST registrar el gauge `ingest.queue_depth`, lo que sigue esperando en la cola, cada vez que toma un save (`:342`).

**Tests:** `the_split_masks_the_text_and_leaves_out_what_stores_nothing`, `the_gate_absorbs_a_stored_piece_and_takes_a_resumed_one_as_it_was`, `the_embed_puts_the_head_before_every_piece_but_the_first_and_names_its_failure`, `the_store_writes_each_piece_notes_it_and_pauses_when_a_drain_is_due`, `a_take_is_named_by_how_it_ended` en `worker.rs`; `an_accepted_save_records_its_stages_its_pieces_and_how_long_it_took` en `crates/clawg-core/src/runtime.rs`.

## R16. El embedder

Cada lote de la sesión del modelo MUST registrar los gauges `embedder.batch` (textos) y `embedder.batch_tokens` (los tokens sobre los que corre, con el relleno) con `role` (`query`, `passage`), y `embedder.wait` (`us`, hasta que el hilo de inferencia toma el lote) con `role` (`crates/clawg-core/src/memory/embedding/embedder.rs:454`, `:522`, `:657`). Cada carga MUST registrar `embedder.load` (`us`, `record_load`, `crates/clawg-core/src/memory/upkeep/bootstrap.rs:403`) con `phase`: `verify`, la lectura de la caché con sus hashes, y `session`, la construcción del embedder; una carga que falla no registra nada. La primera carga de un arranque pasa por `ensure`, cuyo hash va junto con una posible descarga, y no registra `verify`.

**Tests:** `every_batch_records_its_texts_by_role_and_the_worker_its_wait` en `embedder.rs`; `a_load_records_how_long_it_verified_and_built_and_a_failure_records_nothing` en `bootstrap.rs`.

## R17. Los docs

Cada refresco del índice de docs de un proyecto, en segundo plano o no, MUST registrar `docs.refresh.duration` (`us`) y, si terminó, el gauge `docs.refresh.files` con `state` (`indexed`, `unchanged`, `removed`, `failed`) (`record_refresh`, `crates/clawg-cli/src/daemon/api.rs:3271`); cada `POST /v1/projects/docs/search` que esperó un refresco, `docs.search.inline_wait` (`us`, `:3383`).

**Tests:** `a_docs_search_records_the_refresh_and_how_long_it_waited_for_it` en `crates/clawg-cli/tests/api.rs`.

## R18. Los eventos

Cada uno de estos MUST registrarse como un evento (R2) que el escritor guarda en `event` con su minuto (R3):

| `kind` | Cuándo | `detail` |
|---|---|---|
| `model_load` | cada carga del modelo (`bootstrap.rs:597` la del arranque; `idle_embedder.rs:208` las demás, por `LoadWatch`) | `cause` (`start`, `wake`, `call`, `recovery`), `outcome` (`loaded`, `failed`), `ms` |
| `model_unload` | el modelo se suelta por no usarse (`idle_embedder.rs:290`) | `idle_s`, `woken_unused` |
| `embedder_recovery` | un pedido encuentra muerto el hilo de inferencia y carga el modelo otra vez (`idle_embedder.rs:238`) | — |
| `model_download` | una descarga del modelo; una caché verificada no registra ninguno (`bootstrap.rs:349`) | `result` (`ok`, `failed`), `bytes`, `ms` |
| `maintenance` | cada pasada de mantenimiento (`crates/clawg-core/src/memory/upkeep/maintenance.rs:141`) | `ms`, `swept`, `released`, `revived`, `dropped_pairs`, `classified` |
| `backup` | cada snapshot (`crates/clawg-core/src/memory/store/db.rs:297`) | `reason`, `ms`, `bytes` |
| `import` | cada import que termina con su informe, también en seco, por `POST /v1/import` y como tarea (`record_import`, `api.rs:2383`, llamado en `:2372` y `:2572`); uno que falla o se para no registra ninguno | `source`, `dry_run`, `ms`, `rows`, `inserted`, `present` |
| `outbox_failed` | una fila del outbox que pasa a `failed` (`crates/clawg-core/src/memory/ingest/outbox.rs:355`, `:385`, `:548`) | `cause` (`failure`, `gave_up`, `unparsable`), `attempts` salvo `unparsable` |
| `shutdown` | un apagado ordenado, después del drain (`serve.rs:317`) | `http_ms`, `drain_ms`, `within_bounds` |

**Tests:** `a_file_of_schema_1_gets_its_events_and_keeps_its_rows`, `events_past_the_retention_go_with_their_run` en `crates/clawg-core/src/metrics/store.rs`; `events_wait_for_the_writer_and_the_run_keeps_the_latest` en `recorder.rs`; `the_events_of_a_minute_are_kept_with_it_and_served_with_the_run` en `crates/clawg-cli/src/daemon/metrics.rs`; por tipo, `each_load_unload_and_recovery_is_an_event` en `idle_embedder.rs`, `a_download_is_an_event_and_a_verified_cache_is_none` en `bootstrap.rs`, `a_maintenance_pass_is_an_event_with_what_it_did` en `maintenance.rs`, `a_snapshot_is_an_event_with_its_reason_and_size` en `db.rs`, `a_row_that_turns_failed_is_an_event_with_its_cause` en `outbox.rs`, `an_import_and_its_snapshot_are_events_of_the_run` en `crates/clawg-cli/tests/api.rs`, `a_shutdown_event_names_its_phases_in_milliseconds` en `serve.rs`; `the_events_are_listed_by_run` en `crates/clawg-cli/src/metrics.rs`.

## SU1. Las tablas `session` y `session_count`

`metrics.db` lleva, además de las tablas de R3, dos más (R3 dice cómo las recibe un archivo anterior y cómo se lee uno de un esquema más nuevo):

- `session`: `session_id`, `started_at_ms`, `agent_id`, `project_id` (puede ser nulo), `client`, `utc_offset_s`, `last_seen_at_ms`, `last_save_at_ms`, `ended_at_ms`, `end_reason`, `prompts` y `empty_searches`, con clave (`session_id`, `started_at_ms`);
- `session_count`: `session_id`, `started_at_ms`, `kind`, `name` y `count`, con clave (`session_id`, `started_at_ms`, `kind`, `name`). `kind` es `start`, `call`, `failed_call` o `save`.

`memory.db` MUST NOT cambiar: ni su esquema, ni sus migraciones, ni lo que se guarda en él.

#### Escenario: un archivo del esquema 2 con una ejecución, un minuto y un evento
- GIVEN un `metrics.db` con `user_version = 2`, una fila en `run`, una en `gauge_minute` y una en `event`
- WHEN se lo lee en sólo lectura, y después el daemon lo abre para escribir
- THEN la lectura no da ninguna sesión y no cambia la versión; al abrirlo para escribir queda en la versión 3, las tres filas siguen iguales y una sesión escrita se lee de vuelta

**Tests:** `a_file_of_schema_2_gets_its_sessions_and_keeps_its_rows`, `a_newer_schema_is_refused_untouched` en `crates/clawg-core/src/metrics/store.rs`.

## SU2. Una fila por sesión

El primer evento de hook de un id que no tiene una fila abierta MUST abrir una, con `started_at_ms` y `last_seen_at_ms` en la hora del evento, el agente, el proyecto (o nulo) y el cliente del evento, y el desfase de la hora local. Cada evento de hook y cada actividad atribuida MUST llevar `last_seen_at_ms` a su hora y contar:

| Qué | Dónde |
|---|---|
| un arranque | `session_count` con `kind` `start` y `name` el `source` |
| un prompt | `prompts` |
| una llamada a una herramienta | `kind` `call` y `name` la herramienta; si falló, también `kind` `failed_call` |
| un save aceptado | `kind` `save` y `name` el tipo de memoria, y `last_save_at_ms` |
| una búsqueda sin ningún hit | `empty_searches` |

Cada `name` y el `client` MUST salir de un conjunto cerrado:

- `source`: `startup`, `resume`, `clear`, `compact`, `fork`; sin `source`, `unspecified`;
- herramienta: las del contrato del agente (SU8);
- tipo de memoria: los de `MemoryType`; un save que no declara tipo, `unspecified`;
- `client`: `claude-code`, `cursor`;
- `end_reason`: `clear`, `resume`, `logout`, `prompt_input_exit`, `completed`, `aborted`, `error`, `window_close`, `user_close`, `idle`.

Un evento de hook que no dice qué pasó (el cuerpo de `POST /v1/sessions` de un clawg anterior) MUST abrir o seguir la fila sin contar nada. Una actividad atribuida a un id del que este daemon no tiene fila abierta ni cerrada MUST descartarse: sólo un evento de hook trae la identidad con la que se abre una fila.

El proyecto de una fila es el del evento que la abrió y no cambia con los eventos siguientes.

**Tests:** `a_session_opens_on_its_first_event_and_counts_what_follows`, `activity_of_an_unknown_session_is_dropped`, `a_name_outside_its_set_is_other` en `crates/clawg-core/src/metrics/sessions.rs`.

## SU3. El cierre

Una fila MUST cerrarse de una de dos formas:

- **Por el hook de cierre:** `ended_at_ms` es la hora del aviso y `end_reason` su motivo.
- **Por la regla de las 6 horas:** cuando pasan `IDLE_CLOSE_MS` (6 horas, `crates/clawg-core/src/metrics/sessions.rs:19`) o más desde `last_seen_at_ms` sin ningún evento ni actividad, `ended_at_ms` es `last_seen_at_ms` y `end_reason` es `idle`.

La regla MUST aplicarse cuando llega el próximo evento o actividad del mismo id, antes de contarlo, y cuando se arma el reporte (SU11). No necesita un proceso que revise; el escritor además la aplica con cada minuto (SU4).

Una fila cerrada MUST NOT reabrirse. Un evento de hook o una actividad atribuida que llega después con el mismo id MUST abrir una fila nueva, con la identidad de la anterior si es una actividad atribuida. Un aviso de cierre no abre nada: uno que llega a una fila que la regla ya cerró la deja cerrada con `idle`, y uno de un id sin fila abierta se descarta.

Un reinicio del daemon MUST NOT cerrar ninguna fila: el daemon que arranca retoma las filas abiertas (SU4), y un evento del mismo id antes de las 6 horas sigue la misma fila.

#### Escenario: 5 h 59 min y 6 h
- GIVEN una sesión cuyo último evento fue a la hora T
- WHEN llega un prompt a T + 5 h 59 min, y otro 6 h después de ese
- THEN el primero sigue la fila; el segundo la cierra con `idle` y `ended_at_ms` en la hora del primero, y abre una fila nueva que empieza a la hora del segundo

#### Escenario: un cierre por `/clear` y lo que sigue
- GIVEN una sesión S abierta
- WHEN llega su cierre con motivo `clear`, y después un prompt de S
- THEN S queda con una fila cerrada con `clear` y una fila nueva abierta

Una fila que se abre en el mismo milisegundo en que cerró la anterior del mismo id MUST empezar un milisegundo después: (`session_id`, `started_at_ms`) es la clave.

**Tests:** `six_hours_without_activity_close_a_session_as_idle`, `a_closed_session_is_not_reopened`, `an_end_after_six_idle_hours_leaves_the_row_idle`, `a_row_opened_as_the_one_before_it_closed_begins_a_millisecond_later` en `sessions.rs`.

## SU4. Se guarda con cada minuto y se retoma al arrancar

El registro vive en memoria, colgado del `Recorder` del runtime (`Recorder::sessions`, `recorder.rs:197`). El escritor MUST guardar en `metrics.db`, con cada minuto y al apagarse (`Writer::flush`), las filas que cambiaron desde la escritura anterior, con su estado entero, y aplicar antes la regla de las 6 horas a las abiertas. Un `kill` pierde como mucho un minuto, como las series.

Al arrancar, el escritor MUST cerrar con `idle` las filas abiertas del archivo que ya pasaron las 6 horas y cargar las demás en el registro.

Con `CLAWG_METRICS=off`, o con la escritura apagada por un fallo, el registro MUST NOT guardar nada en memoria ni en disco. Un fallo al escribir las sesiones apaga la escritura como el de un minuto (R3), sin afectar ninguna petición.

**Tests:** `sessions_are_written_whole_and_read_back`, `open_sessions_past_six_hours_are_closed_on_start` en `store.rs`; `sessions_are_kept_with_each_minute_and_resumed_by_the_next_run`, `a_writer_turned_off_keeps_no_sessions` en `crates/clawg-cli/src/daemon/metrics.rs`.

## SU5. Retención de las sesiones

Una fila cuya `last_seen_at_ms` tiene más de `HOURS_KEPT_MS` (400 días) MUST borrarse con sus contadores, en la misma poda que las horas y los eventos (R4).

**Test:** `sessions_past_the_retention_are_dropped` en `store.rs`.

## SU6. Las rutas del daemon

Todas detrás del token, y todas contestan `204` sin cuerpo. Su lugar entre las rutas del daemon está en [`specs/daemon-access`](../daemon-access/spec.md), SU6.

- `POST /v1/sessions` MUST registrar la sesión vigente del agente y del proyecto, y además aceptar `event` (`start` o `prompt`), `source` y `client`, los tres opcionales. Es un evento de hook de esa sesión (SU2). Un `event` desconocido MUST contar como uno que no dice qué pasó.
- `POST /v1/sessions/end` (`session_id`, `reason` opcional) MUST cerrar la fila abierta de ese id (SU3). No toma proyecto y no registra ninguno. Un `session_id` inválido es un 400, con la misma regla que `POST /v1/sessions`.
- `POST /v1/sessions/calls` (`agent_id`, `project_id` opcional, `tool`, `failed`) MUST contar una llamada para la sesión vigente de ese agente y ese proyecto. Sin sesión vigente, no cuenta nada.

`POST /v1/memories` MUST contar un save cuando lo acepta en el outbox (no cuando contesta por un `operation_id` cuyo save sigue esperando o falló) y lo selló con una sesión. `POST /v1/search` MUST contar una búsqueda vacía cuando no devuelve ningún hit, para la sesión vigente del agente y del proyecto de la petición; una búsqueda sobre varios proyectos no se atribuye.

Nada de esto MUST cambiar lo que esas rutas contestan ni lo que se guarda en `memory.db`.

**Tests:** `a_hook_event_opens_the_row_of_its_session_and_counts`, `a_daemon_that_keeps_no_metrics_keeps_no_sessions`, `a_session_end_closes_its_row_with_the_reason`, `a_tool_call_counts_for_the_session_of_its_agent_and_project`, `an_accepted_save_and_an_empty_search_count_for_their_session` en `crates/clawg-cli/tests/api.rs`.

## SU7. Unir y borrar proyectos

`POST /v1/projects/merge` MUST pasar al proyecto que queda las filas del proyecto que se une, y `POST /v1/projects/delete` MUST borrar las filas del proyecto borrado, con sus contadores, en memoria y en `metrics.db`. El escritor MUST aplicarlo en cuanto la operación termina, sin esperar al cambio de minuto. Deshacer una unión o un borrado MUST NOT devolver las filas: el registro no guarda de qué proyecto venían ni lo que borró. Una unión o un borrado en seco, o uno que no escribió nada, MUST NOT tocar ninguna fila.

**Tests:** `merging_a_project_moves_its_sessions_and_deleting_it_drops_them` en `sessions.rs`; `project_changes_reach_the_stored_sessions` en `store.rs`; `a_merge_moves_the_sessions_and_a_delete_drops_them` en `tests/api.rs`.

## SU8. El puente avisa cada llamada con su nombre

Cuando una herramienta termina, el puente MUST mandar `POST /v1/sessions/calls` con el agente de la llamada, el proyecto de la carpeta abierta en la ventana (ninguno si hay varias o ninguna), el nombre de la herramienta y si su resultado fue un error, también cuando el puente la rechazó sin pedirle nada al daemon. El aviso MUST NOT cambiar lo que la herramienta contesta, MUST NOT arrancar ni esperar un daemon, MUST NOT registrar ningún proyecto y MUST acotarse a `CALL_REPORT_WAIT` (500 ms, `crates/clawg-cli/src/mcp/server.rs:524`).

El proyecto de la carpeta es el id que el daemon ya le dio a este puente por ella, o si todavía no la registró, el que se deriva de su raíz; el daemon lo sigue hasta su proyecto vivo. Para saber qué carpeta hay abierta, cada herramienta MUST pedirle las carpetas al cliente que las declara (`roots/list`), también las cinco que no las usan para otra cosa (`list_memories`, `delete_memory`, `restore_memory`, `update_memory`, `resolve_conflict`).

El nombre sale del conjunto cerrado de las herramientas del contrato del agente (`TOOLS`, `crates/clawg-cli/src/daemon/sessions.rs:32`); el daemon MUST guardar `other` para uno que no conoce. Lo que el modelo ve del servidor MUST NOT cambiar.

**Tests:** `the_counted_tools_are_the_tools_of_the_bridge`, `a_tool_reports_its_call_to_the_session_of_the_open_folder`, `a_failed_tool_is_reported_as_failed`, `a_call_is_not_reported_while_no_daemon_is_there` en `crates/clawg-cli/src/mcp/server.rs`; `a_tool_outside_the_set_is_counted_as_other` en `crates/clawg-cli/src/daemon/sessions.rs`; `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`mcp/server.rs`), sin cambios.

## SU9. Los hooks

`clawg hook session-start` y `clawg hook user-prompt-submit` MUST mandar en su registro el evento (`start` o `prompt`), el cliente y, el primero, el `source` de su entrada ([`specs/hooks`](../hooks/spec.md), R6).

`clawg hook session-end` MUST leer `session_id` y `reason` de su entrada y mandar el cierre al daemon. MUST NOT imprimir nada, MUST NOT registrar el proyecto ni leer su carpeta, y MUST terminar dentro de `SESSION_END_DEADLINE` (1 s, `crates/clawg-cli/src/hook.rs:36`). Sin daemon, sin `session_id` o con una entrada que no es JSON, no hace nada y sale con 0. Un hook instalado para Claude Code que corre Cursor no hace nada, como los otros.

**Tests:** `a_hook_names_its_event_and_its_source`, `a_session_end_closes_the_session_and_prints_nothing`, `a_session_end_without_a_daemon_or_an_id_does_nothing` en `crates/clawg-cli/src/hook.rs`; `hook_takes_the_session_end_event` en `cli.rs`.

## SU10. `setup` instala el hook de cierre

`clawg setup claude-code`, con o sin `--local`, MUST instalar `SessionEnd` sin matcher junto a los otros dos, y `clawg setup cursor` MUST instalar `sessionEnd` junto a `sessionStart` ([`specs/setup`](../setup/spec.md), R3, CU3 y SL2). Sobre una instalación anterior, MUST agregar sólo el que falta y dejar los demás byte a byte. `--remove` MUST quitarlo. En Cursor cada evento tiene su paso y su línea en el reporte: un hook de clawg que no coincide con el plan en un evento queda como está y sale con 1, y el otro evento se instala igual, como ya pasa en Claude Code. `hook_installed`, con lo que `doctor` dice que el hook instalado no dejó corridas, sigue mirando sólo `sessionStart`, el único que las deja.

**Tests:** `a_run_over_an_install_without_the_session_end_hook_adds_only_it` en `crates/clawg-cli/src/setup.rs` y en `crates/clawg-cli/src/setup/cursor.rs`, `a_local_run_over_an_install_without_the_session_end_hook_adds_only_it` en `crates/clawg-cli/src/setup/local.rs`, `remove_takes_each_hook_that_is_there` y `the_session_start_hook_of_this_profile_is_found_in_hooks_json` en `cursor.rs`; `setup_dry_run_writes_nothing`, `setup_cursor_installs_and_remove_undoes_it` y `setup_local_installs_in_the_folder_and_remove_undoes_it` en `crates/clawg-cli/tests/binary.rs`.

## SU11. `clawg metrics sessions`

`clawg metrics sessions [--by project|agent|day] [--since Nd|Nh] [--session ID] [--profile P]` MUST abrir `metrics.db` en sólo lectura, sin daemon y sin el lock del store, e imprimir un JSON (`sessions`, `crates/clawg-cli/src/metrics.rs:35`; la agregación pura en `crates/clawg-core/src/metrics/session_report.rs`). No abre `memory.db`.

Sin `--session`: `by`, `since_ms`, `idle_close_ms` y `groups`, de las filas cuya `last_seen_at_ms` es de `--since` (por defecto `30d`) en adelante. `--by project` (por defecto) agrupa por `project_id`, con `key` nulo para las filas sin proyecto; `--by agent`, por `agent_id`; `--by day`, por la fecha local del comienzo de la fila. Cada grupo trae:

- `sessions`, `open` (las que siguen abiertas) y `ended`, cuántas cerraron por cada motivo, `idle` incluido;
- `duration_ms` (`total`, `mean` y `max`), del comienzo al cierre; de una fila abierta, hasta su última actividad;
- `since_last_save_ms` (`sessions`, `mean` y `max`), del último save al cierre, de las filas cerradas que guardaron algo;
- `prompts`, `empty_searches`, `starts` por `source`, `calls` por herramienta (`count` y `failed`) y `saves` por tipo;
- `with_summary` y `with_summary_pct`: las filas que guardaron un `session_summary`.

Una fila abierta cuya última actividad tiene 6 horas o más MUST leerse como cerrada con `idle` en esa hora, aunque el archivo todavía la tenga abierta.

Con `--session ID`: `session_id` y `rows`, cada fila de esa sesión entera, de la más vieja a la más nueva, sin importar `--since`. `--session` con `--by` es un error de uso.

Sin `metrics.db` MUST salir con 1 diciendo que el daemon lo escribe salvo con `CLAWG_METRICS=off`, sin crearlo.

#### Escenario: dos sesiones de un proyecto y una sin proyecto
- GIVEN dos filas del proyecto P, una cerrada con `clear` que guardó un `session_summary` y otra abierta hace 7 horas, y una fila sin proyecto
- WHEN se reporta por proyecto
- THEN el grupo P trae `sessions` 2, `ended` con `clear` 1 e `idle` 1, `open` 0 y `with_summary_pct` 50, y hay un grupo con `key` nulo

**Tests:** `a_report_groups_sessions_by_project_agent_and_day`, `an_open_session_past_six_hours_reads_as_idle`, `the_time_since_the_last_save_counts_closed_sessions_that_saved` en `session_report.rs`; `the_sessions_report_reads_the_file_without_a_daemon`, `one_session_is_listed_with_each_of_its_rows` en `crates/clawg-cli/src/metrics.rs`; `metrics_sessions_takes_a_grouping_an_age_and_a_session` en `cli.rs`.

## SU12. De punta a punta

**Tests** en `crates/clawg-cli/tests/binary.rs`, con `CLAWG_TEST_HOME`, el embedder falso y `--port 0`:

- `a_daemon_keeps_each_session_across_a_restart_and_reports_them`: un `session-start` con id S; dos llamadas por el puente y un `user-prompt-submit`; un `session-start` con `source=clear` e id S2; un reinicio del daemon y un prompt de S2. Da S con 2 llamadas y 1 prompt, S2 con una sola fila que empezó antes del reinicio y tiene su prompt, y el reporte por proyecto con las dos.
- `a_session_end_closes_the_session_and_what_follows_opens_another`: un `session-end` de S con motivo `clear` la cierra con ese motivo, y un prompt posterior de S abre una fila nueva.

## Invariantes

- Nada de esto sale de la máquina.
- **Las series y los eventos son cerrados:** ninguna etiqueta de una serie y ningún detalle de un evento lleva texto de una memoria, una consulta, una ruta de fichero o un id. Cada valor sale de un conjunto cerrado escrito en el código.
- **`session` y `session_count` son el único lugar de `metrics.db` con ids,** y sólo con tres: el de la sesión que da el cliente, el del agente y el del proyecto. Todo otro valor de texto de esas tablas (`client`, `end_reason`, `kind`, `name`) sale de un conjunto cerrado.
- **Está prohibido en todo `metrics.db`, estas tablas incluidas:** el texto de una memoria, de una consulta o de un prompt; una ruta de fichero o de carpeta, el `cwd` y el `transcript_path` del hook incluidos; y el usuario: su nombre, su email (el `user_email` de Cursor) o cualquier otro dato que lo identifique.
- `memory.db`, su esquema y lo que ve el modelo no cambian por las métricas.
- Ninguna serie cambia lo que el daemon hace ni lo que contesta. Una medición que falla se deja fuera; nunca falla una petición ni para un worker.
- Borrar `metrics.db` sólo pierde la historia de métricas; el próximo arranque lo crea de nuevo.

**Test:** `no_text_path_or_user_reaches_a_session_row` en `crates/clawg-cli/tests/api.rs`: un prompt, una consulta, un save y un hook con `cwd`, `transcript_path` y `user_email` no dejan en las filas nada más que los tres ids y valores de los conjuntos cerrados.

## Límites conocidos

- `clawg metrics diff` con pruebas estadísticas no existe; `report --by version` da los números que compararía.
- Sin etiqueta de agente en las series: el `agent_id` es autodeclarado y abriría series sin cota. El agente aparece sólo en `session` (SU1), una fila por sesión y no una serie por agente.
- `process.threads` falta en Windows (pediría recorrer un snapshot de Toolhelp) y la memoria falta en macOS.
- El sobrecoste por petición y el de la sonda del store no se midieron con el modelo real. La sonda no se apaga: `CLAWG_METRICS=off` sólo deja de escribir, así compararla pide un build sin ella.
- `embedder.batch_tokens` sólo lo da la sesión del modelo real, que la suite no carga.
- La espera en la conexión es la de una sonda cada 10 s, no la de cada llamada: no dice qué llamada hizo esperar.
- Dos sesiones en paralelo del mismo agente y proyecto no se distinguen en la actividad atribuida: va a la última que registró un hook, como los saves.
- Cursor no tiene un hook por prompt: sus filas no cuentan prompts.
- Con varias carpetas abiertas, o ninguna, el puente no nombra un proyecto y sus llamadas no se atribuyen. El hook de Cursor tampoco registra sesión en ese caso.
- Una carpeta cuyo proyecto no es el que se deriva de su raíz (un clon reconocido por su remoto, una subcarpeta declarada) no se atribuye hasta que una herramienta de ese puente la registra: las herramientas que van por id de memoria no registran nada.
- Lo que no llega al daemon no se cuenta: una llamada con el daemon caído, o un hook sin daemon.
- Después de un reinicio del daemon, la actividad atribuida no se cuenta hasta que un hook registra la sesión otra vez, igual que un save sale sin sesión.
- El cierre que el cliente no avisa (el proceso que muere, el equipo que se apaga) queda con `idle` en la última actividad.
- Una sesión que se corta en el arranque, antes de su primer mensaje, no avisa su cierre: su fila queda abierta hasta que la regla de las 6 horas la cierra con `idle`.
- Un proyecto borrado con `CLAWG_METRICS=off` conserva en `metrics.db` las filas que ya tenía: con la escritura apagada el daemon no toca el archivo.
- El archivo va hasta un minuto atrás del daemon que corre: el reporte lee lo ya escrito.
