# usage-metrics — Especificación

Dominio nuevo: al archivar pasa a `specs/metrics/spec.md`.

## R1. Un histograma de buckets fijos que se suma

`clawg_core::metrics::Histogram` MUST contar cada valor en el primer bucket cuya frontera superior lo contiene, con fronteras fijas por unidad:

- microsegundos (`us`): 50, 100, 250, 500, 1 000, 2 500, 5 000, 10 000, 25 000, 50 000, 75 000, 100 000, 250 000, 500 000, 750 000, 1 000 000, 2 500 000, 5 000 000, 7 500 000, 10 000 000, 30 000 000, 60 000 000, 300 000 000 y un bucket abierto;
- bytes: 256, 1 KiB, 4 KiB, 16 KiB, 64 KiB, 256 KiB, 1 MiB, 4 MiB, 16 MiB, 64 MiB, 256 MiB y un bucket abierto.

Guarda además `count`, `sum`, `min` y `max`. Sumar dos histogramas de la misma unidad MUST dar el histograma de la unión de sus muestras; sumar unidades distintas MUST fallar. Un percentil MUST estimarse dentro del bucket que contiene al percentil exacto (el valor de rango `ceil(q·count)`), acotado por `min` y `max`. Los buckets MUST poder guardarse como bytes y leerse de vuelta iguales.

#### Escenario: suma y percentil
- GIVEN dos conjuntos de duraciones registrados en dos histogramas
- WHEN se suman y se estima p50, p95 y p99
- THEN cada estimación es la misma que la del histograma de la unión, y queda entre las fronteras del bucket del percentil exacto

**Tests:** `merging_two_histograms_is_the_histogram_of_the_union`, `a_percentile_falls_in_the_bucket_of_the_exact_one`, `histograms_of_different_units_do_not_merge`, `the_buckets_round_trip_through_bytes` en `crates/clawg-core/src/metrics/histogram.rs`.

## R2. El registro por minuto

`clawg_core::metrics::Recorder` MUST acumular, por serie (nombre y etiquetas canónicas `k=v` ordenadas por clave), un histograma del minuto en curso y uno de toda la ejecución, y gauges del minuto con `count`, `sum`, `min`, `max` y `last`. Tomar el minuto MUST devolverlo y dejarlo vacío sin tocar el acumulado de la ejecución. Una serie sin datos en el minuto MUST NOT aparecer en él.

**Tests:** `taking_the_minute_empties_it_and_keeps_the_run`, `labels_are_canonical_whatever_their_order` y `a_gauge_keeps_its_extremes_and_last_value` en `crates/clawg-core/src/metrics/recorder.rs`.

## R3. `metrics.db`, aparte del store

El daemon MUST escribir las métricas en `<data>/metrics.db`, con su propia conexión, nunca en `memory.db`. Al arrancar MUST registrar la ejecución (`run`: inicio, versión, `dist_target`, perfil, SO, arquitectura, CPUs, modelo de embedding, `CLAWG_EMBEDDER_IDLE_MINUTES`, desfase de la hora local en segundos) con `stopped_at_ms` nulo; cada minuto MUST volcar el minuto anterior (`hist_minute`, `gauge_minute`), y un apagado ordenado MUST volcar el minuto en curso y escribir `stopped_at_ms`. Un proceso matado MUST dejar `stopped_at_ms` nulo.

En Unix el fichero MUST crearse `0600`. Un `metrics.db` que SQLite no reconoce como base o encuentra corrupto MUST apartarse como `metrics.db.corrupt-<ms>` y recrearse, con un `warn`. Un fallo al abrirlo o al escribir MUST registrarse una vez como `warn` y apagar la escritura hasta el próximo arranque, sin afectar ninguna petición. `CLAWG_METRICS=off` MUST apagar la escritura (no se crea ni se toca `metrics.db`); cualquier otro valor la deja encendida.

`metrics.db` MUST NOT viajar en los snapshots ni cambiar con `restore`, `purge`, `export`, `import` o `doctor`.

#### Escenario: fichero corrupto
- GIVEN un `metrics.db` con bytes que no son SQLite
- WHEN se abre para escribir
- THEN queda un `metrics.db.corrupt-<ms>` con esos bytes y un `metrics.db` nuevo con el esquema

**Tests:** `a_run_is_recorded_open_and_closed_on_stop`, `a_minute_is_written_with_its_series`, `a_corrupt_file_is_set_aside_and_recreated`, `the_file_is_private_to_its_owner` (Unix) en `crates/clawg-core/src/metrics/store.rs`; `metrics_off_is_the_only_way_to_turn_writing_off` en `crates/clawg-cli/src/daemon/metrics.rs`; `a_failed_write_turns_writing_off_and_serving_goes_on` en el mismo módulo.

## R4. Retención

Los minutos MUST guardarse 14 días; pasado ese plazo MUST sumarse en la hora de su ejecución (`hist_hour`, `gauge_hour`; los buckets se suman, así que los percentiles por hora no pierden nada) y borrarse en la misma transacción. Las horas MUST guardarse 400 días, y una ejecución sin filas y de más de 400 días MUST borrarse. El daemon MUST aplicarlo al arrancar y una vez por hora.

**Tests:** `old_minutes_fold_into_their_hour` y `hours_past_the_retention_are_dropped` en `store.rs`.

## R5. Latencia HTTP

Toda petición, por el socket o por TCP, a cualquier ruta, `/health` incluida y también las que el guard (`403`) o el token (`401`) rechazan, MUST contarse en `http.duration` (`us`, desde que llega hasta que se forma la respuesta) con las etiquetas `method`, `route` (la plantilla de la ruta, `/v1/memories/{kind}/{id}`, o `unmatched`), `status` (`2xx`, `3xx`, `4xx`, `5xx`) y `transport` (`unix` o `tcp`), y en `http.request_bytes` (bytes, por `Content-Length`; `0` sin él) con `method` y `route`. `http.in_flight` MUST ser un gauge de las peticiones en curso. Un método fuera de los que el router conoce MUST etiquetarse `other`. Ningún valor de etiqueta MUST venir de la URL concreta, de la consulta ni del cuerpo.

#### Escenario: las etiquetas son plantillas
- GIVEN una petición `GET /v1/memories/memory/123?agent=x` con un cuerpo de texto
- WHEN se toma el minuto
- THEN la serie es `route=/v1/memories/{kind}/{id}` y ningún valor de etiqueta contiene `123`, `agent=x` ni el texto

**Tests:** `every_request_is_timed_by_route_status_and_transport`, `refused_requests_are_timed_too` y `no_label_comes_from_the_request` en `crates/clawg-cli/tests/api.rs`.

## R6. Proceso y runtime

Cada minuto el daemon MUST muestrear como gauges: `process.rss_bytes` y `process.rss_peak_bytes` (Linux: `VmRSS`, `VmHWM`; Windows: `WorkingSetSize`, `PeakWorkingSetSize`), `process.private_bytes` (Linux: `RssAnon`; Windows: `PrivateUsage`), `process.cpu_ratio` (tiempo de CPU del proceso sobre el tiempo de reloj desde la muestra anterior; Unix: `getrusage`, Windows: `GetProcessTimes`), `process.threads` (Linux), `process.open_files` (Linux, las entradas de `/proc/self/fd`), `process.handles` (Windows), `embedder.resident` (1 o 0), `rt.alive_tasks`, `rt.global_queue_depth` y `rt.worker_busy_ratio` (el tiempo ocupado de los workers de tokio sobre el de reloj por el número de workers). Lo que un sistema no da MUST faltar, nunca valer 0.

**Tests:** `the_linux_status_file_is_read_in_bytes` y `a_ratio_is_the_delta_over_the_wall_time` en `crates/clawg-cli/src/daemon/process.rs`; `the_sample_of_this_process_has_its_memory` (Linux y Windows) en el mismo módulo.

## R7. `GET /v1/metrics`

Detrás del token, MUST devolver en JSON la versión, el inicio de la ejecución, si la escritura está encendida, el minuto en curso (histogramas con `count`, `sum`, `min`, `max`, `p50`, `p95`, `p99` y sus buckets; gauges) y los histogramas acumulados de la ejecución. Con `?format=prometheus` MUST devolver el acumulado de la ejecución y el último valor de cada gauge en el formato de texto de Prometheus (`text/plain; version=0.0.4`), con nombres `clawg_<nombre>` (los `us` en segundos con sufijo `_seconds`, los bytes con `_bytes`) y buckets acumulados con `le`.

**Tests:** `metrics_needs_the_token_and_answers_the_minute_and_the_run` y `metrics_speaks_prometheus_text` en `crates/clawg-cli/tests/api.rs`.

## R8. `clawg metrics report`

`clawg metrics report --metric M [--label k=v]... [--by day|hour|version] [--since N(d|h)] [--profile P]` MUST leer `metrics.db` en sólo lectura, sin daemon y sin el lock del store, y MUST imprimir un JSON con un grupo por día local, por hora del día local (`00` a `23`) o por versión (por defecto `version`; desde `--since`, por defecto `30d`), sumando minutos y horas de las series de nombre `M` que tienen cada par `k=v` pedido. Cada grupo de un histograma MUST traer `count`, `mean`, `min`, `max`, `p50`, `p95` y `p99`; el de un gauge, `samples`, `mean`, `min` y `max`; los dos, `runs` y `not_stopped` de las ejecuciones con filas en el grupo. Arriba, `runs` MUST contar todas las ejecuciones que empezaron desde `--since`, con o sin filas, con `total` y `not_stopped` (sin `stopped_at_ms`: matadas, caídas o todavía en marcha). La hora local de cada fila es la de su ejecución (`utc_offset_s`). Sin `metrics.db`, o sin series con ese nombre, MUST salir con 1 y decirlo (en el segundo caso, con los nombres que hay).

**Tests:** `a_report_groups_by_version_day_and_hour` y `a_label_filter_keeps_only_matching_series` en `crates/clawg-core/src/metrics/report.rs`; el parseo en `crates/clawg-cli/src/cli.rs`.

## R9. De punta a punta

**Test:** `a_daemon_records_its_requests_and_report_reads_them_after_stop` en `crates/clawg-cli/tests/binary.rs`: un daemon con `--port 0` y el embedder falso atiende un guardado y una búsqueda por el socket y por TCP; `GET /v1/metrics` trae `http.duration` con `transport=unix` y `transport=tcp`; tras `clawg stop`, `clawg metrics report --metric http.duration --label route=/v1/search` cuenta las dos búsquedas con `not_stopped: 0`. `a_killed_daemon_leaves_its_run_not_stopped`: tras matar un daemon que no llegó a volcar ningún minuto, el report cuenta su ejecución en `runs.not_stopped`.
