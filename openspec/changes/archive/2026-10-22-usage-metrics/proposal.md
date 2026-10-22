# usage-metrics — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M–L | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (0.48.0):

1. **No hay historia.** Lo que el daemon expone es una foto: `/health` (`HealthBody`, `crates/clawg-cli/src/daemon/api.rs:480`, handler en `:969`) y `GET /v1/stats` (`api.rs:1405`). Sin una serie no se puede decir si una versión es más lenta que la anterior, si la memoria crece con los días o a qué hora se satura.
2. **No se mide la latencia de ninguna petición.** El único tiempo es el del KNN, una ventana de 256 búsquedas (`crates/clawg-core/src/memory/latency.rs`) que sólo sale con nivel `debug`; el nivel por defecto es `info` (`crates/clawg-cli/src/main.rs:105`). Dos percentiles de esa ventana no se combinan ni se comparan entre períodos.
3. **No se mide el proceso.** Ni memoria residente, ni privada, ni CPU, ni hilos o descriptores.
4. **Cada verificación mide a mano.** Los `verify.md` de 36, 46, 56 y 59 armaron cada uno su medición; un dato de uso real habría dicho si lo medido se sostiene después del release.

## Objetivo que sirve

Transversal: poder comparar el daemon por día, por franja horaria y por versión con su uso real, sin que nada salga de la máquina. Es la base de 73 (pipeline-metrics) y 74 (perf-bench).

## Decisiones

Las decisiones abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **Encendido por defecto**, en `dev` y en `prod`: sin historia no hay comparación, y nada sale de la máquina ni lleva texto. `CLAWG_METRICS=off` apaga la escritura de `metrics.db`; el registro en memoria y `GET /v1/metrics` siguen.
- **`diff`** queda fuera: cuando haga falta irá como binario de `clawg-eval`, que reutiliza la estadística de `rag-eval` y no entra en el release. `report` ya da por versión lo que un `diff` compararía.
- **Sin etiqueta de agente**: el `agent_id` es autodeclarado y abriría series sin cota.
- **`/health` no cambia**: es abierto, y la memoria del proceso sale en `GET /v1/metrics`, detrás del token.
- **Sin tabla de eventos**: los eventos de cada ejecución (cargas, recuperaciones, mantenimiento) son de 73. De este cambio queda la fila de la ejecución, con `stopped_at_ms` nulo si no terminó limpia.

## Alcance

1. `clawg-core::metrics`: el histograma de buckets fijos (microsegundos o bytes), puro y agregable; el registro por minuto con histogramas y gauges; `metrics.db` (ejecución, minutos, horas, retención, fichero corrupto apartado) y la agregación pura de `report`.
2. Daemon: un middleware sobre el router entero, fuera del guard y del token, que mide `http.duration` y `http.request_bytes` por método, plantilla de ruta, clase de estado y transporte, y `http.in_flight`; el muestreo por minuto del proceso (memoria residente y su pico, privada, CPU, hilos, descriptores o handles) y del runtime de tokio; el volcado por minuto y al apagarse en su propia conexión.
3. `GET /v1/metrics` detrás del token (JSON y `?format=prometheus`).
4. `clawg metrics report --metric M [--label k=v]... [--by day|hour|version] [--since 30d]`, que lee `metrics.db` sin daemon y sin lock.
5. Docs: `.claude/CLAUDE.md`, y al archivar un dominio nuevo, `specs/metrics`.

## Fuera de alcance

- Instrumentar ingest, las ramas de la búsqueda, el embedder, los docs y la espera de SQLite (73), y el banco por versión (74).
- `clawg metrics diff` con pruebas estadísticas.
- Lo que ve el modelo: `agent_contract.json` no cambia.
- Enviar nada fuera de la máquina; un exportador de OpenTelemetry.
- Reemplazar `KnnLatency`.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sin modelo real, red ni perfil:

- sumar dos histogramas y estimar un percentil da lo mismo que estimarlo sobre la unión de las muestras, y el error contra el percentil exacto queda dentro del bucket;
- el daemon de prueba (`--port 0`, embedder falso) atiende un guardado y una búsqueda por el socket y por TCP; `GET /v1/metrics` y, tras `clawg stop`, `clawg metrics report` traen las series con su `transport`; una ejecución matada deja `stopped_at_ms` nulo y `report` la cuenta como no limpia;
- ningún valor de etiqueta viene de la URL concreta ni del cuerpo;
- un `metrics.db` corrupto se aparta y se recrea, y un fallo al escribir no tumba el servicio.

El sobrecoste (latencia del guardado corto y de `POST /v1/search` con y sin `CLAWG_METRICS=off`) se mide en el perfil dev con el modelo real cuando haya autorización para usarlo; el umbral: p95 sin diferencia distinguible. El registro de una petición son dos búsquedas binarias y un lock sin contención, frente a milisegundos de embebido.

## Datos y vuelta atrás

`memory.db` no cambia: ni esquema, ni identificadores, ni estados, ni contrato de embedding. `metrics.db` es un fichero nuevo y aparte en el directorio de datos: no viaja en los snapshots ni lo tocan `restore`, `purge`, `export`, `import` o `doctor`. Volver atrás es volver al binario anterior, que lo ignora; borrarlo sólo pierde la historia de métricas.
