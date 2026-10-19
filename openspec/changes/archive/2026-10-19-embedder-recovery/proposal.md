# embedder-recovery — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | — (antes de 53 outbox-lifecycle) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`3383f3d`).

1. **Un pánico en la inferencia deja el modelo muerto.** El bucle de `embedder-worker` llama a `inference.embed(&texts)` sin `catch_unwind` (`crates/clawg-core/src/memory/embedder.rs:548-553`) y el perfil de release no aborta, así que un pánico mata sólo ese hilo. El que pidió el vector recibe `WorkerGone` sin causa, y cada llamada siguiente también (`embedder.rs:413-417`).
2. **Nada lo reemplaza.** `IdleEmbedder::lease` devuelve el modelo guardado mientras exista (`crates/clawg-core/src/memory/idle_embedder.rs:101-103`) y nada vacía la celda ante `WorkerGone` (`:206-214`). Cada llamada fallida cuenta como uso (`:88-93`), así que el barrido de ocio casi nunca lo suelta; con `CLAWG_EMBEDDER_IDLE_MINUTES=0` el modelo ni siquiera está envuelto (`crates/clawg-cli/src/daemon/serve.rs:114-123`). Búsqueda y `recall_context` responden `500` (`crates/clawg-cli/src/daemon/api.rs:1136-1140`) y el ingest deja cada save pendiente hasta reiniciar, mientras `/health` dice `ready` y `embedder_resident: true`.
3. **Inferencia para nadie.** Un lote cuyo llamante ya se fue se infiere igual (`embedder.rs:552`).

No se encontró un camino concreto que haga entrar en pánico al hilo (ver la exploración): es resiliencia, no un fallo observado.

## Objetivo que sirve

Transversal: sin vectores no se guarda ni se busca. Que un fallo del hilo de inferencia se arregle solo sostiene los cinco objetivos, y hace pasajeros de verdad los fallos del embedder que 53 quiere reintentar.

## Decisiones

Las cinco decisiones abiertas de la exploración se cierran con sus recomendaciones; el porqué está en el [diseño](design.md).

- **Tras un pánico (A2):** la petición que entró en pánico recibe `InferenceFailed("inference panicked: …")`, el hilo termina y suelta la sesión. Nunca se reusa una sesión que acaba de entrar en pánico.
- **La llamada que descubre el fallo reintenta una vez:** `IdleEmbedder` descarta el modelo cuyo hilo se fue, lo recarga y repite la llamada, así que nadie ve el hilo muerto salvo que la recarga también falle. Una recuperación por hilo muerto ocurre a lo sumo una vez por minuto: un fallo que se repite responde «no listo» en vez de recargar el modelo en cada llamada.
- **Ocio `0` (C1):** el daemon envuelve siempre el modelo en `IdleEmbedder`; con `0` el barrido no se arma.
- **`/health` sigue igual:** `embedder` queda en `ready` y `embedder_resident` pasa a `false` mientras el modelo está descartado.
- **Código de respuesta:** `WorkerGone` ya no sale de `IdleEmbedder`; lo que se ve es el `503 embedder_not_ready` de una recarga que no se pudo hacer, como con el modelo suelto. `api.rs` no cambia.

## Alcance

1. `embedder.rs`: el bucle del hilo pasa a `serve_requests`, que salta las peticiones cuyo llamante se fue y contesta un pánico con `InferenceFailed` antes de terminar.
2. `idle_embedder.rs`: descartar el modelo cuyo hilo se fue (comparando por `Arc::ptr_eq`), recargar y reintentar una vez, a lo sumo una recuperación por minuto; el ocio pasa a `Option<Duration>`.
3. `bootstrap.rs` y `serve.rs`: `unloading_when_idle` recibe `Option<Duration>` y `clawg serve` envuelve siempre.
4. Docs: `.claude/CLAUDE.md`, `openspec/specs/storage/spec.md` («Ciclo de vida del embedder») y el mapa de `openspec/README.md`.

## Fuera de alcance

- Cola con prioridad, canal acotado, un estado nuevo en `/health`.
- `memory.db`, el esquema, el contrato de embedding o los vectores.
- La descarga y la verificación de archivos (60, archivado) y la medición de recursos (62).
- Reintentar filas del outbox (53).

## Cómo se mide el éxito

Tests que fallan hoy, todos con mocks, sin modelo real, red ni perfil (ver [spec](spec.md)):

- un hilo cuya inferencia entra en pánico: hoy el llamante recibe `WorkerGone`; con el cambio recibe `InferenceFailed` que dice `panicked`, y el hilo termina;
- un lote cuyo llamante ya se fue no llega a la inferencia;
- un `IdleEmbedder` cuyo modelo devuelve `WorkerGone`: la misma llamada recarga y responde;
- dos llamadas que encuentran el hilo muerto a la vez comparten una sola recarga y no descartan el modelo nuevo;
- un modelo que muere en cada recarga se recarga a lo sumo una vez por minuto, y entretanto la llamada responde `NotReady`;
- con el ocio en `None`, el modelo no se suelta nunca y un hilo muerto igual se recupera.

## Datos y vuelta atrás

No toca `memory.db`, el esquema, los identificadores, los estados, el contrato de embedding ni los archivos del modelo: el cambio vive en cómo el proceso trata un hilo de inferencia caído. Volver a la versión anterior no pide nada.
