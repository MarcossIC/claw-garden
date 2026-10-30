# usage-metrics — Exploración

Objetivo transversal: poder comparar el rendimiento del daemon por día, por franja horaria y por versión con datos de uso real, sin sacar nada de la máquina. Nace de una propuesta de medición escrita sobre 0.26.0 (doce propuestas, M0 a M12); este cambio toma su base: el histograma agregable, el fichero aparte, la latencia HTTP por ruta, el proceso y una forma de leerlo. Lo que se apoya en esta base va en [pipeline-metrics](../2026-10-30-pipeline-metrics/exploration.md) (73) y [perf-bench](../../perf-bench/exploration.md) (74); la rotación del log, en [serve-log-rotation](../2026-10-22-serve-log-rotation/exploration.md) (72).

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa (de la propuesta, sobre 0.26.0) | Estado en `develop` (0.45.0) |
|---|---|
| No hay métricas persistentes ni histogramas en el producto | **Sigue cierta.** Ningún `metrics`, `Histogram` ni fichero de series en `clawg-core` ni `clawg-cli`; lo único con percentiles es `KnnLatency` (`crates/clawg-core/src/memory/latency.rs:109`), una ventana de las últimas 256 búsquedas que se alimenta en `crates/clawg-core/src/memory/search.rs:1357` y sólo sale en la línea `retrieval=` con nivel `debug` (`search.rs:1364-1372`); el nivel por defecto es `info` (`crates/clawg-cli/src/main.rs:105`). Dos percentiles de esa ventana no se pueden combinar ni comparar entre períodos. |
| `/health` da una foto: versión, pid, embedder, `uptime_s`, outbox | **Cierta, con más campos.** `HealthBody` (`crates/clawg-cli/src/daemon/api.rs:480-505`, handler en `:969`) suma `embedder_error`, `embedder_retryable`, `embedder_resident` y el bloque `ingest` con `oldest_pending_s` y los `problems`, que se omite si el store no contesta en 250 ms. Sigue sin latencias, memoria ni contadores. |
| El router permite un middleware único por ruta | **Cierta, y ahora cubre los dos transportes.** Las rutas con token están en `api.rs:862-884` (con `require_bearer` en `:884`) y `/health` aparte (`:887`); el mismo router se sirve por `<data>/clawg.sock` y por TCP desde local-transport (58) y windows-socket (59). Un middleware sobre el router completo mide los dos; conviene etiquetar el transporte. |
| `windows-sys` ya es dependencia y sólo falta una feature | **Cierta.** `crates/clawg-cli/Cargo.toml:62` tiene `Win32_System_Threading` (para `GetProcessTimes`) pero no `Win32_System_ProcessStatus` (para `GetProcessMemoryInfo`); `windows-sys` 0.61.2 ya está en `Cargo.lock`, así que la feature no agrega un crate. En Unix, `libc` ya es dependencia (`Cargo.toml:81`) para `getrusage`. |
| Las métricas de runtime de tokio son estables en la versión del lock | **Cierta.** El lock fija tokio 1.51.0: `num_alive_tasks` y `global_queue_depth` son públicas sin `tokio_unstable` (`tokio-1.51.0/src/runtime/metrics/runtime.rs:74`, `:100`), y `worker_total_busy_duration` está bajo `cfg_64bit_metrics!` (`:104-141`), disponible en los dos targets que se publican. |
| La memoria del daemon en Windows tras soltar el modelo nunca se midió | **Falsa hoy.** embedder-idle-unload (46) la midió con el modelo real (21,3 MB sobre la línea de base en diez recargas, sin crecer). Lo que sigue sin serie es la memoria en uso real a lo largo de días, y el pico de carga, que embedder-footprint (62) tiene en su puerta. |
| Un apagado sin SIGTERM no se cuenta | **A medias.** shutdown-budget (55) hizo de SIGTERM un apagado ordenado en Unix; en Windows los eventos de consola siguen sin manejarse (`.claude/CLAUDE.md`, `clawg serve`). Contar las ejecuciones que no terminaron limpias sigue siendo útil, sobre todo en Windows. |
| `clawg status` imprime el JSON de `/health` tal cual | **Falsa.** `status_report` (`crates/clawg-cli/src/status.rs:26-43`) lo anida en `health`, junto a `daemon` (el `daemon.json`) y `store`. Un sondeo externo lee `.health.pid`, `.health.version`, `.health.ingest.pending`, no `.pid`. |
| La línea de arranque no dice la versión | **Cierta.** `serve.rs:204-209` dice perfil, transporte y directorio. La versión sí va en `daemon.json` (`serve.rs:188`), que se borra al salir. Va en 72. |

## Problema (verificado contra el código)

1. **No hay historia.** Todo lo que el daemon expone es una foto (`/health`, `GET /v1/stats`). Sin una serie no se puede decir si 0.45.0 es más lenta que 0.44.0, si la memoria crece con los días o a qué hora se satura.
2. **No se mide la latencia de ninguna petición.** El único tiempo registrado es el del KNN, a nivel `debug` y en una ventana por número de muestras. Ni guardar, ni `POST /v1/context`, ni `search_project`, ni `backup` dejan un tiempo.
3. **No se mide el proceso.** Ni RSS, ni memoria privada (lo que ve el usuario en Windows), ni CPU en reposo, ni hilos o descriptores.
4. **Las verificaciones de cada cambio miden a mano.** Los `verify.md` de 36 (latencia del save), 46 (memoria tras recargas), 56 (planes) y 59 (barra de transporte) armaron cada uno su medición; un dato de uso real habría dicho si lo medido se sostiene después del release.

## Lo que ya existe y se reutiliza

- `KnnLatency` (`latency.rs`) como modelo de estructura sin locks; el histograma nuevo no la reemplaza en este cambio.
- `build_info::version()` y `dist_target` (`crates/clawg-core/src/build_info.rs`) para la fila de la ejecución.
- `ensure_private_dir` (`crates/clawg-core/src/private_dir.rs`) para el directorio, y el aislamiento de perfiles: un `metrics.db` dentro de `<data>` hereda que un build debug nunca toca `~/.clawg`.
- El patrón de `backup`, que desde `b1f22f3` abre su propia conexión: las métricas no pasan por la conexión de `memory.db`.
- `crates/clawg-cli/tests/binary.rs` con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake` para probar que el daemon real escribe las series.

## Enfoques posibles

### A. Histograma de buckets fijos + `<data>/metrics.db` con una fila por minuto (recomendado)

Un histograma con fronteras fijas (en µs: 50, 100, 250, 500, 1 000, 2 500, 5 000, 10 000, 25 000, 50 000, 75 000, 100 000, 250 000, 500 000, 750 000, 1 s, 2,5 s, 5 s, 7,5 s, 10 s, 30 s, 60 s, 300 s, +∞), registrado con una búsqueda binaria y dos atómicos, y volcado una vez por minuto en su propia conexión a `<data>/metrics.db`. Sumar buckets da el histograma de cualquier intervalo (un día, los lunes de 9 a 12, una versión) y de él cualquier percentil, con un error acotado por el ancho del bucket. Contiene las fronteras que OpenTelemetry recomienda para `http.server.request.duration`, así que un exportador futuro las reutiliza.

Esquema mínimo: `run` (versión, `dist_target`, perfil, SO, arquitectura, CPUs, contrato del modelo, `CLAWG_EMBEDDER_IDLE_MINUTES`, desfase local al arrancar, `stopped_at_ms` nulo si no terminó limpia), `hist_minute` (`run_id`, minuto, nombre, etiquetas canónicas, `count`, `sum`, `min`, `max`, buckets como BLOB), `gauge_minute` y `event`. Sólo se escriben las series con datos en ese minuto.

Fuera de `memory.db` a propósito: no ocupa la única conexión del store (justo lo que mide read-connections, 57), no viaja en los snapshots ni lo tocan `restore`, `purge`, `export` o `doctor`, y se puede leer con el daemon andando.

### B. Líneas JSON en `logs/metrics.jsonl`

Más simple de escribir, pero agrupar por versión u hora pide leer todo el fichero, y rotarlo pierde historia. Se descarta como almacenamiento; puede ser el formato de `clawg metrics export`.

### C. Un crate de métricas (`metrics`, `opentelemetry`, `prometheus`)

Aportan un registro global y exportadores sin consumidor aquí, y una dependencia nueva sin uso real (lo que `config.yaml` pide evitar). Un histograma de buckets fijos son unas 150 líneas. Descartado; `?format=prometheus` en `GET /v1/metrics` cubre a quien quiera un Prometheus local, porque es texto.

### Recomendación: A

## Alcance candidato

1. **Histograma y registro en `clawg-core`**: tipo puro, sin E/S, con la suma y el percentil probados (sumar dos histogramas y calcular el percentil da lo mismo que calcularlo sobre la unión de las muestras, dentro del ancho del bucket).
2. **`metrics.db`**: creación con `ensure_private_dir`, archivo `0600` en Unix, volcado por minuto en su conexión, un fallo que se registra una vez y apaga el volcado sin afectar el servicio, y un fichero corrupto que se aparta y se recrea. Retención: minutos 14 días, después horas (suma de buckets, sin pérdida para percentiles por hora) 400 días.
3. **HTTP** (M2): middleware sobre el router completo, fuera de `require_bearer` para contar también los 401 y los 403 del guard: `http.duration{route, status_class, transport}` con la plantilla de `MatchedPath` (nunca la URL), `http.in_flight` (máximo del minuto) y `http.request_bytes{route}`.
4. **Proceso** (M3), una vez por minuto: RSS y pico (`/proc/self/status` en Linux, `GetProcessMemoryInfo` en Windows), memoria privada (`RssAnon`; `PrivateUsage`), CPU (`getrusage`; `GetProcessTimes`), hilos y descriptores, `embedder_resident`, y `rt.alive_tasks`, `rt.global_queue_depth`, `rt.worker_busy_ratio` de tokio. En macOS, lo que dé `getrusage`; el resto queda vacío.
5. **Lectura** (parte de M8): `GET /v1/metrics` detrás del token, con el minuto en curso y el acumulado de la ejecución (y `?format=prometheus`), y `clawg metrics report --metric M [--label k=v] --by day|hour|version [--since 30d]`, que lee `metrics.db` sin el daemon y sin lock, como `export`.
6. **Apagado**: `CLAWG_METRICS=off` desactiva el volcado.

### No-alcance

- Instrumentar ingest, búsqueda por rama, embedder, docs y la espera de SQLite: es 73.
- El banco controlado por versión: es 74.
- Ampliar `/health`: es abierto, y 38 dejó afuera recortarlo. Como mucho `rss_bytes`, que no dice nada del usuario; se decide en la propuesta.
- `memory_status` y lo que ve el modelo: no cambian (`agent_contract.json` igual).
- `clawg metrics diff` con pruebas estadísticas: la maquinaria de `rag-eval --diff` vive en `clawg-eval`, que el binario publicado no enlaza. Se decide abajo.
- Enviar nada fuera de la máquina.

## Decisiones abiertas

- [ ] **Encendido por defecto en `prod`.** Recomendación: sí, porque sin historia no hay comparación y no sale de la máquina; `dev` encendido siempre.
- [ ] **Dónde vive `diff`.** (a) Un binario en `clawg-eval` que lee `metrics.db`, reutiliza la estadística de `rag-eval` y no entra en el release; (b) mover la parte mínima a `clawg-core`. Recomendación: (a) primero.
- [ ] **Etiqueta de agente.** El `agent_id` es autodeclarado; si se usa, sólo los N más frecuentes y el resto `other`. Recomendación: sin agente en este cambio.
- [ ] **`/health`.** ¿Suma `rss_bytes` o queda igual?

## Cómo se mide

- Test unitario de la suma de histogramas y del percentil contra la unión de muestras.
- Test en `binary.rs`: el daemon de prueba (`--port 0`, embedder falso) atiende un guardado y una búsqueda por el socket y por TCP, y `GET /v1/metrics` y, tras `clawg stop`, `clawg metrics report` contienen las series esperadas con `transport` correcto; una ejecución matada deja `stopped_at_ms` nulo.
- Sobrecoste: la latencia del save corto y de `POST /v1/search` con y sin `CLAWG_METRICS=off` en el perfil dev, con el umbral fijado en la propuesta (candidato: p95 sin diferencia distinguible).

## Riesgos

- **Privacidad.** Sólo plantillas de ruta, clases de estado, transporte y tamaños; nunca texto, consultas ni rutas de fichero. Un test fija que ningún valor de etiqueta viene del cuerpo.
- **Disco.** Acotado por la retención (estimación, no medida: ~2 MB por día activo en minutos).
- **Relojes.** Duraciones con `Instant`; el minuto en UTC y el desfase local por ejecución. Un cambio de horario a mitad de una ejecución corre una hora su agrupación local.
- **Mantenimiento.** Una serie que se deja de alimentar no falla nada; el test de `binary.rs` es la red.
