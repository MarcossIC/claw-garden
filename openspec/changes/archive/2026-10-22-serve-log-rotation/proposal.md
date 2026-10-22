# serve-log-rotation — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); verificado otra vez contra `develop` (`443396f`, 0.49.0), donde las líneas se movieron:

1. **`serve.log` crece sin tope.** `spawn_serve_at` abre `<data>/logs/serve.log` con `create` y `append` (`crates/clawg-cli/src/client.rs:1284-1288`) y se lo pasa al hijo como stdout y stderr. Ningún código lo trunca, rota ni borra; cada autoarranque del puente (`ProcessSpawner::spawn_serve`, `client.rs:1245-1254`) y cada `clawg update` (`crates/clawg-cli/src/update/effects.rs`, que también lee su cola en `log_tail`) le suman.
2. **El log no dice qué versión escribió cada tramo.** La primera línea del daemon (`crates/clawg-cli/src/daemon/serve.rs:231-236`) nombra perfil, transporte y directorio; la versión sólo va a `daemon.json` (`serve.rs:199`), que un apagado ordenado borra, y a `metrics.db`. Con `update` cambiando el binario sobre el mismo `serve.log`, no se sabe qué versión escribió qué, ni la de un arranque que falló antes de escuchar.

## Objetivo que sirve

Transversal: poder diagnosticar un daemon por su log, también una ejecución que quedó sin `stopped_at_ms` en `metrics.db`, sin que el log crezca para siempre.

## Decisiones

- **Rotar al abrir, en `spawn_serve_at`** (enfoque A de la exploración). Rotar en el daemon pediría un writer propio que no vería los `panic` ni lo que ONNX Runtime escribe en stderr (B); truncar pierde la cola de la ejecución que murió (C).
- **Tope y ficheros:** si `serve.log` pasa de `SERVE_LOG_LIMIT` (10 MiB) al abrir, `serve.log.1` pasa a `serve.log.2` (reemplazándolo) y `serve.log` a `serve.log.1`: tres ficheros, a lo sumo unos 30 MiB más lo que un daemon vivo escriba de más. Un daemon que corre no rota nunca; el próximo arranque lo corta.
- **Una rotación que falla no impide arrancar:** se abre `serve.log` como hoy y su primera línea nueva dice por qué no se rotó.
- **La versión, primero:** `clawg serve` registra, antes de abrir el directorio de datos y el store, `clawg serve <versión> [(<dist_target>)] starting, profile <p>`, también cuando escribe en la terminal.

## Alcance

1. `crates/clawg-cli/src/serve_log.rs`: la decisión pura de qué renombrar (`rotation`), la rotación en disco y la apertura que usa `spawn_serve_at`.
2. `daemon/serve.rs`: la línea de arranque con la versión (`starting_line`, pura).
3. `.claude/CLAUDE.md` (`clawg serve`): dónde va el log, cuánto se guarda, y `RUST_LOG=info,clawg_core::memory::search=debug` para la línea `retrieval=` sin el resto del `debug`.

## Fuera de alcance

- Logs estructurados o migrar a `tracing`.
- Un hallazgo de `doctor` por un `serve.log` grande.
- Rotar el log de un `clawg serve` lanzado a mano, que escribe donde lo redirija quien lo lanza.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sin modelo real, red ni perfil:

- con un `serve.log` de más del tope en un home de prueba, el autoarranque de un puente deja `serve.log.1` con el contenido viejo y un `serve.log` nuevo cuya primera línea nombra la versión;
- la decisión de rotar es pura y se prueba sin disco; el renombrado, en un directorio temporal, también con el fichero abierto por otro escritor, en Unix y en Windows.

## Datos y vuelta atrás

No toca `memory.db`, el esquema, los identificadores ni el contrato de embedding. Sólo renombra ficheros de `logs/`. Volver atrás es volver al binario anterior, que sigue escribiendo en `serve.log` y deja los rotados donde están.
