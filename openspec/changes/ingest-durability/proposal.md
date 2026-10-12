# ingest-durability — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (comparte bordes con data-preservation, ver abajo) | propuesta (borrador) |

## Problema

Un `save_memory` que respondió `queued` puede no llegar nunca a la base, y el agente no tiene forma de enterarse.

**Contrato del save**

- La herramienta MCP devuelve el texto `queued` (`crates/clawg-cli/src/mcp/server.rs:279`), sin id.
- El daemon responde `202 {"status":"queued"}` (`crates/clawg-cli/src/daemon/api.rs:460`). El cliente descarta el cuerpo (`crates/clawg-cli/src/client.rs:346-347`).
- El handler documenta que la asincronía es a propósito: el dedup sólo es correcto con **un único consumidor secuencial**, y responder en el hilo del request metería un segundo escritor (`api.rs:420-431`). Cualquier solución tiene que conservar ese consumidor único.

**Fallos que el agente nunca ve** (todos en `crates/clawg-core/src/memory/worker.rs`, todos terminan en `log::warn!`)

- Embedder no disponible o batch fallido: se descarta el evento entero (`:322-331`).
- El embedder devuelve menos vectores que chunks: se descarta el evento (`:333-342`).
- Falla el insert de un chunk: se descarta ese chunk y siguen los demás (`:369-372`). Un save de varios chunks puede quedar a medias.
- El bucle externo sólo loguea (`:162-175`).

**Bootstrap del embedder**

- Se lanza una sola vez, sin reintento (`crates/clawg-cli/src/daemon/serve.rs:84-91`). Ante un error el cell queda vacío y sólo se emite un evento a `LogSink` (`crates/clawg-core/src/memory/bootstrap.rs:244-343`).
- El worker retiene los eventos mientras el cell esté vacío (`worker.rs:152-159`, `:191-206`). Si el bootstrap falló, cada save sigue diciendo `queued` y la cola crece para siempre.
- `/health` sólo distingue `ready` y `loading` (`api.rs:414`). Un bootstrap fallido se ve como `loading` eterno, también en `clawg status`.
- El 503 de búsqueda dice "search will work in a minute" (`server.rs:235-238`). Es falso si el bootstrap falló.

**Cola**

- `unbounded_channel` (`crates/clawg-core/src/runtime.rs:95`). El único productor es el handler del save (`api.rs:456`).

## Objetivo

1 — guardar memorias de forma confiable: un save que el agente ve como hecho existe en la base, y uno que falló llega como error.

## Alcance (decisiones por defecto, a confirmar)

1. **Save confirmado (enfoque A de la exploración).** El handler encola el evento junto con un canal de respuesta (`oneshot`) y espera el resultado del worker. El worker sigue siendo el único consumidor y el dedup no cambia. El canal de respuesta va en un sobre alrededor de `MemoryEvent`, no en el evento: `MemoryEvent` es serializable (`crates/clawg-core/src/memory/events.rs:13-33`).
   - **Éxito:** `201` con un resultado por chunk: `{"id": N, "outcome": "inserted" | "reinforced"}`. Un duplicado absorbido devuelve el id de la fila que reforzó (`reinforce_if_duplicate` ya lo da, `worker.rs:231-241`).
   - **Fallo:** un error con la causa (embed, insert). Si algunos chunks se escribieron antes del fallo, la respuesta lleva sus ids además del error, para que el agente pueda borrarlos o repetir sólo lo que faltó.
   - **MCP:** `save_memory` devuelve `saved #N` (o la lista de ids) en lugar de `queued`. Los ids sirven tal cual para `delete_memory`.
2. **Timeout.** Si el worker no contesta en **10 s** (valor inicial, se ajusta con la medición del punto "Éxito"), el save responde `202` con un texto honesto: aceptado, sin confirmar, verificable con `list_memories`. No es un error, porque el evento sigue en la cola y repetirlo sólo reforzaría la fila (`worker.rs:207-216`).
3. **Embedder cargando.** El save no espera la descarga del modelo: responde `202` al instante con "se guarda cuando cargue el modelo". Es el único caso que sigue siendo asíncrono.
4. **Estado del embedder con tres valores:** `loading`, `ready` y `failed { error, retryable }`. Lo publica `/health` y por lo tanto `clawg status`.
   - Con `failed`, el save se rechaza con `503` y la causa. Ya no se encola.
   - El mensaje MCP deja de prometer "in a minute" cuando el estado es `failed`.
5. **Reintento del bootstrap.** Un fallo de descarga o carga se reintenta con backoff exponencial (30 s, tope de 30 min, sin límite de intentos). Un contrato que no coincide **no** se reintenta: `bootstrap.rs:298` ya lo declara permanente.
6. **Cola acotada.** Capacidad de 1.024 eventos. Con la cola llena, el save responde `503 ingest_queue_full`. En la práctica sólo se llena mientras carga el modelo.

## Fuera de alcance

- **Drenaje al parar**, flush del `AccessTracker` y checkpoint: son data-preservation, parte A. Con este cambio la ventana de pérdida se achica a los saves que respondieron `202` (timeout o modelo cargando), pero no desaparece.
- **Remedio de un contrato que no coincide** (`clawg reembed`): data-preservation, parte D.
- **Outbox durable** (enfoque C): sólo si aparece un caso de kill frecuente.
- Mensajes de downgrade y checksum (`server.rs:231-233`): data-preservation, parte C.
- Cambiar el chunker, el dedup o la detección de contradicciones.

## Bordes con data-preservation

No son independientes del todo. Hay tres puntos de contacto:

| Punto | data-preservation | Propuesta para este cambio |
|---|---|---|
| Rechazar saves con la memoria desactivada | Parte D lo menciona "coordinado con ingest-durability" | Lo implementa este cambio (estado `failed`, punto 4). D sólo agrega el remedio |
| Mensaje "search will work in a minute" (`server.rs:235-238`) | Parte C, junto con los de downgrade y checksum | Este cambio arregla el del embedder. C se queda con los otros dos |
| `serve.rs` (arranque del bootstrap y del worker) | Parte A guarda el `JoinHandle` del worker (`:94`) para drenar | Este cambio reemplaza el `spawn` del bootstrap (`:84-91`) por el bucle de reintento. Mismo archivo, líneas distintas: conflicto de merge posible, no de diseño |

El `AccessTracker` que menciona la exploración queda entero en data-preservation, parte A.

## Éxito

`cargo test --workspace` en verde contra la base de 1.304. Tests que fallan hoy:

1. Con el bootstrap fallido, `POST /v1/memories` responde `503` con la causa, y `save_memory` devuelve un error y no `queued`.
2. Con un embedder que falla, el save devuelve un error y no inserta nada.
3. El save devuelve un id, y `delete_memory` con ese id borra la fila.
4. Un save repetido devuelve el mismo id con `reinforced`.
5. Con el bootstrap fallido por descarga, un segundo intento que funciona deja el embedder en `ready`. Con contrato distinto no hay segundo intento.
6. `/health` reporta `failed` y la causa después de un bootstrap fallido.
7. Con la cola llena, el save responde `503 ingest_queue_full`.
8. Un worker que no contesta en el timeout produce `202` y el texto de no confirmado.

**Latencia (antes de fijar el timeout):** medir el p50 y el p99 de un save con el embedder real, antes y después. Hoy no se puede medir en el entorno en la nube porque la descarga del runtime ONNX está bloqueada. Se mide en local, en el perfil dev, con autorización.

## Datos guardados

No cambia el esquema ni ningún dato guardado. Cambia el contrato HTTP del save (`202` → `201` con ids en el caso normal) y el texto que devuelve la herramienta MCP.

- El único cliente HTTP del repo es `DaemonClient::save` (`client.rs:337-348`), que entra en el alcance.
- El rollback es revertir los commits. No hay migración.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| El save suma la latencia del embedding | Alta | Es el costo aceptado de saber el resultado. Se mide antes de fijar el timeout |
| Un save largo (muchos chunks) supera el timeout | Media | Responde `202` honesto, no un error. Se mide con un save largo |
| Un modelo que tiene prompts que dicen "queued" | Baja | El texto nuevo es más informativo; no hay parser en el repo que dependa de `queued` salvo el test `server.rs:774`, que se actualiza |
| Conflicto de merge con data-preservation en `serve.rs` | Media | Se toca sólo el bloque del bootstrap. El que mergea segundo resuelve |

## Design

Hace falta un `design.md` corto para dos decisiones con alternativas: dónde vive el canal de respuesta (sobre vs. campo en el evento) y dónde vive el estado del embedder (un `watch` junto al `OnceCell` vs. reemplazar el `OnceCell`).
