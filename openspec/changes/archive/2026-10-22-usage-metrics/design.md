# usage-metrics — Diseño

## D1. Histograma de buckets fijos, no un crate de métricas (R1)

**Elegido:** `Histogram` con fronteras fijas por unidad (`Unit::Micros`, `Unit::Bytes`), un vector de cuentas, `count`, `sum`, `min` y `max`. Registrar es una búsqueda binaria sobre las fronteras. Las de microsegundos contienen las que OpenTelemetry recomienda para `http.server.request.duration`, más finas abajo (50 µs a 1 ms) porque una petición del daemon por el socket tarda eso; un exportador futuro las reutiliza.

El percentil se interpola linealmente dentro del bucket del rango `ceil(q·count)` y se acota con `min` y `max`; el error contra el exacto es, como mucho, el ancho de ese bucket. Como los buckets se suman sin perder nada, el percentil de un día, de una franja o de una versión sale de sumar sus minutos.

| Alternativa | Por qué no |
|---|---|
| `metrics`, `opentelemetry`, `prometheus` | Registro global y exportadores sin consumidor aquí; una dependencia sin uso real. El texto de Prometheus se escribe en unas líneas |
| HDR o DDSketch | Error relativo menor, pero de tamaño variable y sin un formato que se sume en SQL; buckets fijos alcanzan para comparar versiones |
| Guardar cada muestra | Disco proporcional al tráfico; los percentiles de la unión salen igual de los buckets |

## D2. El registro (R2) y el middleware (R5)

**Elegido:** `Recorder` con un `Mutex` sobre dos `BTreeMap` (el minuto y la ejecución) y el último valor de cada gauge. La clave es el nombre (`&'static str`) y las etiquetas canónicas (`k=v` ordenadas, unidas por `,`). Un `Mutex` sin contención cuesta decenas de nanosegundos, frente a los milisegundos de un guardado; un diseño sin locks como `KnnLatency` no hace falta para unas decenas de series.

El middleware va sobre el router entero, por fuera de `admit_loopback` y de `require_bearer`, para contar también lo que rechazan. La ruta sale de `MatchedPath` (la plantilla que axum ya resolvió), nunca de la URI; sin plantilla es `unmatched`. El transporte no lo sabe el router, que es el mismo para los dos servidores (`crates/clawg-cli/src/daemon/serve.rs:218-236`): `serve` pone una `Extension(Transport)` distinta encima de cada copia del router, y el middleware la lee. El cuerpo nunca se lee; el tamaño sale de `Content-Length`.

El `Recorder` vive en `AppCtx` (`metrics: Arc<Recorder>`), así los tests en proceso miden igual que el daemon. Sin el `Extension` (los routers de los tests y de `serve_ephemeral`) el transporte es `tcp`, que es lo que son.

## D3. `metrics.db` y su escritor (R3, R4)

**Elegido:** un fichero SQLite aparte, con `rusqlite` (ya en `clawg-core`) y `PRAGMA user_version = 1`; el esquema se crea con `CREATE TABLE IF NOT EXISTS`. Las tablas: `run`, `hist_minute`, `hist_hour`, `gauge_minute`, `gauge_hour`. El minuto es `epoch_ms / 60 000` (UTC); la hora local se reconstruye al leer con `run.utc_offset_s`. Un cambio de horario a mitad de una ejecución corre una hora su agrupación local: es un límite conocido, no un error de los datos.

El escritor es una tarea del daemon (`daemon/metrics.rs`) que cada minuto toma el minuto del `Recorder`, muestrea el proceso y el runtime y lo escribe en un `spawn_blocking`, con su propia conexión. Nunca usa la conexión de `memory.db`, que es única y la que read-connections (57) mide. Al apagarse, después del drenaje, escribe el minuto en curso y `stopped_at_ms`; queda dentro de `AFTER_DRAIN` porque es una escritura de unas filas.

Un fallo al escribir apaga el escritor con un `warn` único: insistir cada minuto llenaría el log sin arreglar el disco. El registro en memoria sigue y `GET /v1/metrics` también. Un fichero que SQLite no reconoce (`NotADatabase`, `DatabaseCorrupt`) se aparta y se recrea, como el resto de lo que clawg encuentra roto y no es la memoria del usuario: las métricas no valen tanto como para negarse a arrancar.

La retención pliega en la misma transacción los minutos de más de 14 días en su hora (buckets sumados, gauges con `min` de los `min`, `max` de los `max`, `sum` y `count` sumados y el `last` del último minuto), y borra las horas de más de 400 días. Con un minuto por serie activa (unas 30 en uso normal) son unos 40 000 filas por día activo en minutos, del orden de 2 MB; las horas pesan 60 veces menos.

| Alternativa | Por qué no |
|---|---|
| Tablas en `memory.db` | Ocupa la única conexión del store, viaja en los snapshots y la tocarían `restore` y `purge` |
| `logs/metrics.jsonl` | Agrupar por versión u hora pide leer todo, y rotarlo pierde historia |

## D4. Proceso y runtime (R6)

**Elegido:** `daemon/process.rs`, con el parseo puro (`/proc/self/status` a bytes, la razón de CPU) separado de las llamadas. Linux lee `/proc/self/status` y `/proc/self/fd`; Unix, `getrusage` (`libc`, ya dependencia); Windows, `GetProcessMemoryInfo` con `PROCESS_MEMORY_COUNTERS_EX`, `GetProcessTimes` y `GetProcessHandleCount`, para lo que `windows-sys` suma las features `Win32_System_ProcessStatus` y `Win32_System_Time` (el desfase local, `GetTimeZoneInformation`), sin crate nuevo: 0.61.2 ya está en `Cargo.lock`. En Unix el desfase sale de `localtime_r` (`tm_gmtoff`). Los hilos en Windows pedirían recorrer un snapshot de Toolhelp; faltan, como dice R6.

Las métricas del runtime son las estables de tokio 1.51 (`num_alive_tasks`, `global_queue_depth`, `worker_total_busy_duration`), sin `tokio_unstable`.

## D5. Lectura (R7, R8)

`GET /v1/metrics` lee el `Recorder` y no toca `metrics.db`: es lo que pasa ahora. `clawg metrics report` lee `metrics.db` y no necesita daemon: es la historia. La agregación (`metrics::report`) es pura sobre filas ya leídas, y la lectura (`MetricsDb::rows_since`) abre el fichero con `SQLITE_OPEN_READ_ONLY`, sin el lock de `memory.lock`, como `export`. La salida es JSON, como `status` y `doctor`.

`--by hour` es la hora del día local (00 a 23), que es lo que contesta «a qué hora se satura»; un día concreto se pide con `--by day`.
