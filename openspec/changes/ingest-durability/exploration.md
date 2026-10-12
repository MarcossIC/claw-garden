# ingest-durability — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — | exploración (borrador) |

**Tesis:** hoy un `save_memory` que respondió `queued` puede no llegar nunca a la base, y el agente no tiene forma de enterarse.

## Problema (verificado)

**Qué recibe el agente**

- La herramienta MCP devuelve el texto `queued` (`crates/clawg-cli/src/mcp/server.rs:279`).
- El daemon responde `202 {"status":"queued"}` sin id (`crates/clawg-cli/src/daemon/api.rs:456-460`).
- No hay modo síncrono ni reintento. La única forma de comprobar el resultado es hacer polling sobre `GET /v1/memories`.

**Cola**

- Es `unbounded_channel` (`crates/clawg-core/src/runtime.rs:95`), sin backpressure.

**Fallo del embedder**

- Se descarta el evento entero con un `log::warn!` (`crates/clawg-core/src/memory/worker.rs:328-330`).

**Fallo del insert**

- Se descarta ese chunk con un `log::warn!` (`worker.rs:369-372`).

**Apagado**

- Los saves encolados se pierden al parar el daemon (`crates/clawg-cli/src/daemon/serve.rs:94`, `135-144`).
- Eso se resuelve en **data-preservation**, parte A, porque es la pérdida que ocurre en cada actualización. Este cambio se ocupa sólo del contrato del save: qué sabe el agente sobre el resultado.

**Bootstrap del embedder**

- Se lanza una sola vez, sin reintento (`serve.rs:85`, `crates/clawg-core/src/memory/bootstrap.rs:204`).
- Desde `f8df9c6` el worker retiene los eventos hasta que el embedder carga (`worker.rs:191-205`).
- Si la descarga falla, cada save sigue diciendo `queued` y la cola crece sin límite.
- El texto del 503 dice "search will work in a minute" (`server.rs:236`). Eso es falso cuando el bootstrap falló.

**Menor:** el `AccessTracker` acumula hasta 30 s en memoria, que también se pierden ante un kill.

### Lo que el análisis tenía mal

El análisis atribuye las pérdidas a "bloqueos de base de datos". Eso no pasa:

- Hay una sola conexión, serializada por `tokio_rusqlite`.
- WAL ya está activo (`crates/clawg-core/src/memory/db.rs:191-201`).
- El insert de la memoria, del vector y del FTS va en una sola transacción (`crates/clawg-core/src/memory/store_insert.rs:329-372`).

Las causas reales son tres: el embedder, el apagado y el bootstrap.

## Enfoques

| Opción | A favor | En contra |
|---|---|---|
| **A. Save síncrono con timeout**: el daemon espera el insert y devuelve los ids o un error | El agente sabe el resultado y obtiene ids para update/delete. Encaja con MCP, que ya es request/response | Suma la latencia del embedding a cada save (hay que medirla). Si el contenido tiene varios chunks, el save devuelve varios ids |
| **B. Asíncrono endurecido**: rechazar saves si el bootstrap falló y reintentar el bootstrap con backoff | Cambio chico, no toca el contrato de la herramienta | Un fallo posterior al ack sigue siendo invisible para el agente |
| **C. Outbox durable**: el evento se persiste en una tabla antes del ack y el worker la consume | Sobrevive a un crash o un kill | Agrega una migración de esquema y estados de outbox. Es más complejidad que el problema |

**Inclinación:** **A** para el contrato, más lo de **B** que aplica igual: reintento del bootstrap y mensaje honesto. El drenaje al parar va en data-preservation. **C** sólo si aparece un caso de kill frecuente.

## Decisiones abiertas

- [ ] ¿Síncrono (A) o asíncrono endurecido (B)?
- [ ] Si es A, ¿cuál es el timeout? ¿Qué devuelve si vence: `queued` como fallback o un error?
- [ ] ¿La cola pasa a ser acotada? ¿Qué responde un save con la cola llena?
- [ ] ¿Cuál es la política de reintento del bootstrap (backoff, tope) y cómo la expone `clawg status`?

## Cómo se mide

Tests RED primero:

1. Con el bootstrap fallido, `save_memory` devuelve un error y no `queued`.
2. (Si es A) Un fallo de embed llega al cliente MCP como error.
3. (Si es A) El save devuelve ids que después sirven para `delete_memory`.

Además, la latencia de save medida con el embedder real antes y después.
