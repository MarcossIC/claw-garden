# ram-footprint — Especificación

RF1 cambia «Registrar una raíz aceptada la indexa en segundo plano» de [`specs/project-docs`](../../../specs/project-docs/spec.md) y la nota del registro en [`specs/project`](../../../specs/project/spec.md). RF2 y RF3 se suman a [`specs/metrics`](../../../specs/metrics/spec.md), RF4 a [`specs/storage`](../../../specs/storage/spec.md) («Ciclo de vida del embedder») y RF5 a [`specs/portability`](../../../specs/portability/spec.md), junto a IT1.

## RF1. Sólo un registro que lo pide refresca el índice de docs

`POST /v1/projects` MUST lanzar el refresco de fondo del índice de una raíz sólo cuando la petición trae `index_docs: true` y, después del registro, la raíz está aceptada (`register`, `crates/clawg-cli/src/daemon/api.rs:2070`; `spawn_doc_refresh`, `:2079`). Un registro sin `index_docs`, o con `false`, MUST NOT lanzarlo, aunque la raíz ya estuviera aceptada: la marca se guarda igual y sólo sube ([`specs/project`](../../../specs/project/spec.md), R4). Los hooks registran la raíz en cada `session-start` y cada `user-prompt-submit` sin `index_docs`. La frescura del índice es de `search_project`, que refresca un índice de más de `REFRESH_INTERVAL` ([`specs/project-docs`](../../../specs/project-docs/spec.md), «Búsqueda»).

Lo demás del requisito no cambia: la respuesta no espera el refresco y no cambia de forma, y un refresco fallido queda en el log y como último error del proyecto.

#### Escenario: un hook registra una raíz ya aceptada
- GIVEN una raíz registrada con `index_docs: true` cuyo índice tiene su `README.md`, y un `NOTES.md` que se escribió después
- WHEN se registra otra vez sin `index_docs`
- THEN la respuesta es 200, la raíz sigue aceptada y su índice sigue con un doc

**Tests:** `registering_an_accepted_root_without_asking_leaves_its_index_alone` (nuevo), `registering_a_root_indexes_its_docs_in_the_background` y `a_root_registered_without_its_docs_is_not_indexed` en `crates/clawg-cli/tests/api.rs`.

## RF2. `metrics report --by minute`

`clawg metrics report` MUST aceptar `--by minute` (`ReportBy::Minute`, `crates/clawg-cli/src/cli.rs`) y agrupar por minuto local con la clave `AAAA-MM-DD HH:MM`, con el desplazamiento de la ejecución de cada fila, como `--by day` (`By::Minute`, `crates/clawg-core/src/metrics/report.rs:97`). El JSON MUST decir `"by": "minute"`.

Por minuto, el reporte MUST empezar en el mayor entre lo que pide `--since` y la primera hora entera de los minutos guardados (`since_for`, `report.rs:27`; `MINUTES_KEPT_MS`). Las filas más viejas son horas plegadas, y una hora a medias en el borde se leería como su primer minuto. `since_ms` del JSON MUST ser ese comienzo. Las otras agrupaciones no cambian.

#### Escenario: dos minutos de una ejecución tres horas detrás de UTC
- GIVEN filas de las 01:05 y las 02:00 UTC del 2026-10-20, de una ejecución con desplazamiento −3 h
- WHEN se agrupan por minuto
- THEN las claves son `2026-10-19 22:05` (con las dos series de ese minuto sumadas) y `2026-10-19 23:00`

#### Escenario: un `--since` más largo que los minutos guardados
- GIVEN `--since 30d` a las HH:20
- WHEN se reporta por minuto
- THEN el reporte empieza 14 días atrás, en la hora entera siguiente

**Tests:** `a_report_by_minute_keys_each_local_minute` y `a_report_by_minute_starts_where_the_minutes_are_kept` en `report.rs`; `a_report_by_minute_reads_the_minutes_of_the_file` en `crates/clawg-cli/src/metrics.rs`; `metrics_report_takes_a_metric_labels_a_grouping_and_an_age` en `crates/clawg-cli/src/cli.rs`.

## RF3. El pico de memoria privada en Windows

La muestra de cada minuto MUST llevar en Windows `process.private_peak_bytes`, que es `PeakPagefileUsage` de la misma llamada a `GetProcessMemoryInfo` que da `PrivateUsage` (`crates/clawg-cli/src/daemon/process.rs:114`). Es el máximo de la memoria privada desde que arrancó el proceso, aunque haya durado menos de un minuto. En Linux no existe un pico de `RssAnon`, así que la serie MUST quedar afuera, nunca en 0 (`ProcessSample::private_peak_bytes`, `process.rs:18`; el escritor, `crates/clawg-cli/src/daemon/metrics.rs:174`).

#### Escenario
- GIVEN una muestra con `private_peak_bytes`
- WHEN el escritor la registra en dos minutos
- THEN `metrics.db` tiene `process.private_peak_bytes` en los dos

**Tests:** `each_minute_and_the_stop_are_written` en `crates/clawg-cli/src/daemon/metrics.rs`; `the_sample_of_this_process_has_its_memory` en `process.rs` (en Windows, el pico es mayor o igual que `private_bytes`; en Linux no está).

## RF4. Cada carga del modelo dice quién la pidió y cuánto costó

Cuando `IdleEmbedder` vuelve a cargar el modelo MUST dejar en el log una línea de `info` con la duración, quién la pidió y, si el daemon le dio una sonda de memoria, la memoria privada antes y después (`load_line`, `crates/clawg-core/src/memory/idle_embedder.rs:85`):

```
embedding model loaded again in 9234 ms for a wake; private memory 284.0 MiB before, 740.0 MiB after
```

Quién la pidió MUST ser `for a wake` (un registro, `Embedder::wake`), `for a call that embeds` o `for a call that found its worker gone` (una recuperación). Cuando la cargó un `wake`, la primera llamada que use el modelo MUST registrar `the model a wake loaded was first used N s later` (`:153`). Si el modelo se descarga por ocio sin que ninguna llamada lo haya usado, la línea de la descarga MUST terminar con `; no call used it after the wake that loaded it` (`unload_if_idle`, `:231`).

La sonda es un `MemoryProbe` (`fn() -> Option<u64>`, `crates/clawg-core/src/metrics/mod.rs:15`). `clawg serve` pasa `process::private_bytes` (`crates/clawg-cli/src/daemon/serve.rs:153`). Sin sonda, la línea no lleva memoria. Nada de esto cambia cuándo se carga o se descarga el modelo.

#### Escenario: un `wake` que nadie usó
- GIVEN un modelo suelto
- WHEN un `wake` lo carga y pasa el tiempo de ocio sin llamadas
- THEN la descarga queda marcada como sin uso después del `wake`
- AND si un `wake` lo carga y después una llamada embebe, la descarga no queda marcada

#### Escenario: una carga que pidió una llamada
- GIVEN un modelo suelto
- WHEN una llamada que embebe lo carga y después pasa el tiempo de ocio
- THEN la descarga no queda marcada

**Tests:** `the_sweep_tells_a_woken_model_nobody_used_from_one_a_call_used`, `a_model_a_call_loaded_is_never_counted_as_woken` y `a_load_names_who_asked_and_the_memory_around_it` en `idle_embedder.rs`; `memory_reads_in_mib` en `metrics/mod.rs`. Las pruebas del ciclo de vida que ya existen pasan sin cambiar ninguna aserción.

## RF5. Cada paso del import dice la memoria con que terminó

En el daemon, cada línea de paso de una importación (IT1) MUST terminar con `private N MiB`: la memoria privada del proceso al terminar el paso (`Timing::private_bytes`; `ImportTimings::with_memory`, `crates/clawg-core/src/memory/portability/timing.rs:152`; el handler, `crates/clawg-cli/src/daemon/api.rs:1900`). Sin sonda no se agrega nada. La línea del resumen no cambia. Sigue sin llevar texto, ids ni rutas: la memoria es un número.

```
import: embed 1142.0 s (51068 rows, 1596 batches, 44.7 rows/s, slowest 2300 ms, private 812.0 MiB)
```

#### Escenario
- GIVEN un `ImportTimings` con sonda
- WHEN se registran dos pasos
- THEN los dos llevan la memoria y la línea del primero termina en `(private 1.0 MiB)`
- AND sin sonda, el paso no la lleva

**Tests:** `a_step_reads_as_its_numbers` y `each_step_carries_the_memory_as_it_ended_when_there_is_a_probe` en `timing.rs`.
