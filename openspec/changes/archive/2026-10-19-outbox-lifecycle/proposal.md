# outbox-lifecycle — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (después de 61; antes de 44, 54 y 55) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`18c3c4f`).

1. **Un fallo pasajero espera al próximo arranque.** El outbox sólo se lee al arrancar (`crates/clawg-core/src/runtime.rs:421`). Tras `IngestFailure::EmbedderUnavailable` el worker anota la causa, devuelve el intento (`after_failure`, `crates/clawg-core/src/memory/outbox.rs:76-80`) y sigue (`crates/clawg-core/src/memory/worker.rs:331-336`); la fila queda `pending` hasta que el daemon se reinicie. Con el que guardó esperando, el handler además la borra (`crates/clawg-cli/src/daemon/api.rs:1126-1130`) y contesta `503`.
2. **Media versión viva.** Si el insert del pedazo k de un save largo falla con el que guardó esperando, los pedazos 1..k-1 quedan vivos, con la clave y sin vínculo (`process_tracked` vuelve antes de `outbox_finish`, `worker.rs:599-601`), la versión anterior de la clave sigue viva, y el handler borra la fila con su progreso (`api.rs:1126-1130`): nada termina el save.
3. **El apagado no retoma lo que podría.** `drain_step` se mira una vez por evento (`worker.rs:304`); el bucle por pedazo (`worker.rs:797-852`) no mira el plazo, aunque el progreso ya permite retomar un save a medias (`resume_positions`, `outbox.rs:98-108`).
4. **Una fila `failed` no tiene salida.** Sólo la borran `outbox_forget`, `outbox_finish` y `purge`. Cuenta en `/health` para siempre y reenviar su `operation_id` responde `500 ingest_failed` (`api.rs:1109-1111`).
5. **Dos reglas de `failed`.** `.claude/CLAUDE.md` dice que una fila que «failed after the caller was answered» queda `failed`; el código la deja `pending` hasta gastar 3 intentos (`after_failure`), como dice `openspec/specs/storage/spec.md`.

## Objetivo que sirve

1: un save aceptado termina de guardarse, o queda en un estado que el agente y el usuario ven y pueden resolver. Va antes de 44 (history-import), que trae miles de notas largas por este camino.

## Decisiones

Las decisiones abiertas de la exploración se cierran con sus recomendaciones, salvo el reintento, que retiene la cola en vez de apartar el save; el porqué está en el [diseño](design.md).

- **Fallo parcial con el que esperaba (D):** una fila que guardó algún pedazo no se olvida. La respuesta es `202` con `reason: "partially_saved"`, los pedazos guardados en `memories`, la causa en `detail` y el `operation_id`; el save se termina en el próximo arranque o se descarta a mano.
- **Fallo sin nada guardado:** sin cambio, `500 ingest_failed` y la fila se borra.
- **Reintento (E, retenido):** sólo lo pasajero. El worker contesta al que espera `202` con `reason: "retrying"` y retiene la cola reintentando el mismo save con espera creciente (1 s, 2 s, … tope 60 s), como retiene un save mientras el modelo carga. Sin barrido.
- **Qué significa `failed` (G):** una fila es `failed` cuando gastó `MAX_ATTEMPTS` (3) tomas que terminaron en fallo no pasajero o en muerte del proceso. Un fallo pasajero y una pausa del apagado devuelven su intento. El código ya lo hace; `.claude/CLAUDE.md` se corrige.
- **Pausa del apagado (I):** antes de cada pedazo de un save con fila, el worker mira el plazo de drenaje; vencido, se detiene con el progreso anotado y devuelve el intento.
- **Salida de una fila `failed` (J):** `clawg outbox retry|discard <OPERATION_ID> [--agent NAME]`, por el daemon. Descartar borra suavemente (con vuelta atrás por `clawg undelete`) los pedazos que ese save insertó. Sin herramienta MCP: `memory_status` muestra las filas `failed` con los pedazos que guardaron y los dos comandos.
- **Orden con 54:** este cambio primero; 54 se escribe sobre el bucle ya pausable.

## Alcance

1. `worker.rs`: retener y reintentar un save con fila tras un fallo pasajero; pausar entre pedazos al vencer el plazo de drenaje.
2. `outbox.rs`: devolver el intento de una pausa; reintentar y descartar una fila `failed` por `operation_id` y agente; los pedazos insertados de cada fila con causa en el resumen de `/health`.
3. `api.rs`: `202 partially_saved` y `202 retrying`; `POST /v1/outbox/retry` y `POST /v1/outbox/discard`.
4. Cliente, puente y CLI: `SaveOutcome::Queued` con `memories` y `detail`, los textos de `save_memory`, la línea de `memory_status` y `clawg outbox`.
5. Docs: `.claude/CLAUDE.md`, y al archivar `openspec/specs/storage`, `openspec/specs/ingest` y `openspec/specs/pieces`.

## Fuera de alcance

- Un save en una sola transacción y cualquier cambio de esquema: el retraso vive en memoria y el progreso ya está en la fila.
- Enmascarar `last_error` (51, ya archivado), los chequeos de 31, topes de tamaño (54), el presupuesto de `stop` (55).
- Cambiar `MAX_ATTEMPTS`, `DRAIN_DEADLINE` o `STOP_TIMEOUT`.
- Una herramienta MCP para reintentar o descartar.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), con `FakeEmbedder` y embedders de prueba, sin modelo real, red ni perfil:

- un save con fila cuyo embedder falla una vez con `NotReady` queda guardado en la misma ejecución del worker; hoy queda `pending`;
- un segundo save con la misma clave detrás de uno que se reintenta queda como versión viva; hoy el reintento lo reemplazaría;
- un save de varios pedazos cuyo insert falla en el segundo con el que guardó esperando responde `202 partially_saved` y su fila sigue con el progreso; hoy responde `500` y la fila se borra;
- un drenaje que vence durante un save largo deja la fila con su progreso y su intento devuelto, y una segunda ejecución lo termina sin duplicados; tres pausas no la dejan `failed`;
- una fila `failed` se reintenta o se descarta por su `operation_id`, y al descartarla sus pedazos insertados quedan borrados y se pueden restaurar.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados de `memory` ni el contrato de embedding. Las filas del outbox siguen con las mismas columnas y los mismos dos estados; reintentar una fila la vuelve a escribir con otro `id` y el mismo `operation_id`. Descartar borra pedazos con el borrado suave de siempre, reversible con `clawg undelete`. Una versión anterior abre la base sin nada que hacer; sus `202` nuevos (`partially_saved`, `retrying`) los lee un puente anterior como un `202` sin confirmar.
