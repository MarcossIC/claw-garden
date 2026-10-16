# ingest-outbox — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (toca el esquema: V14; tiene que cerrar antes de agent-lifecycle-hooks, 25) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código); lo que sigue está verificado otra vez contra `main` (`210c692`). La exploración reservaba V13, que tomó project-root-guard: este cambio usa **V14**.

1. **Un `202` vive sólo en RAM.** El handler mete el evento en el `mpsc` con `try_send` (`crates/clawg-cli/src/daemon/api.rs:757`) y responde `202 embedder_loading` sin esperar (`api.rs:765`) o `202 unconfirmed` cuando vence la espera de 10 s (`SAVE_CONFIRM_TIMEOUT`, `api.rs:90`). No hay otra copia hasta el volcado de un apagado ordenado (`spill::write`, `crates/clawg-core/src/runtime.rs:230`). Un kill, un crash o el cierre de sesión pierden esos saves: el daemon sólo atiende Ctrl-C (`crates/clawg-cli/src/daemon/serve.rs:168-169`), y el autoarrancado en Windows nace sin consola (`DETACHED_PROCESS`, `crates/clawg-cli/src/client.rs:903-904`), así que la única vía ordenada es `clawg stop`. La ventana más grande es la de cada arranque: el modelo carga mientras el puente ya guarda.
2. **Un fallo después del `202` no llega a nadie.** Sólo queda un `log::warn!` (`crates/clawg-core/src/memory/worker.rs:259`); `/health` no cuenta nada (`HealthBody`, `api.rs:420`) y `list_memories` lista filas de `memory`, que un save fallido no tiene. El texto de `save_memory` promete «it will be stored once the embedding model finishes loading» (`crates/clawg-cli/src/mcp/server.rs:520`), una promesa que hoy un kill rompe.
3. **El volcado guarda el texto crudo.** El enmascarado ocurre en el worker (`worker.rs:431`), no en el handler, y el volcado serializa el evento tal cual. Mover la cola a la base sin enmascarar antes dejaría secretos en `memory.db` y en cada snapshot.
4. **Un replay a medias daña un save largo.** Cada pieza se inserta en su transacción y el vínculo `memory_piece` va aparte (`link_save`, `worker.rs:583`). Reinyectar un save cortado reabsorbe las primeras piezas como `reinforced` (suma `access_count` que nadie generó) y las vincula sin la cabecera con la que se embebieron.

## Decisiones

Defaults tomados donde la exploración dejaba la pregunta abierta. La primera se preguntó en el hilo con una tarjeta; mientras no se elija otra, se sigue con la recomendada.

- **Durabilidad: `synchronous = FULL` para la fila del outbox** (tarjeta del hilo). La fila sobrevive a un kill, un crash, una señal de terminar y también a un corte de luz, pagando un fsync por save aceptado; el resto de la base sigue en `NORMAL` ([diseño, D4](design.md#d4-durabilidad)).
- **Outbox en una tabla de `memory.db`** (enfoque A). La fila se confirma antes de contestar; el worker la borra cuando termina el save, y escribe su progreso en la misma transacción que cada pieza ([D1](design.md#d1-una-tabla-en-memorydb), [D2](design.md#d2-progreso-por-pieza)).
- **Progreso por pieza** (A.1a). Un replay retoma desde la pieza siguiente, no refuerza las ya guardadas y vincula con las cabeceras originales. Si el texto ya no da las mismas piezas (cambió el tokenizer), se cae al dedup de hoy.
- **Se enmascara en el handler, antes de escribir la fila.** El worker conserva su pasada, que no encuentra nada nuevo; el `redacted` del `201` sale del enmascarado del handler. Reabre la decisión D2 de secret-hygiene sólo para la copia persistida ([D3](design.md#d3-enmascarar-antes-de-persistir)).
- **Intentos: tope 3.** Un intento se cuenta al tomar la fila; `EmbedderUnavailable` no cuenta. Una fila que no termina se reintenta en el próximo arranque, y a los 3 intentos queda `failed`, con su causa, y el worker sigue con la siguiente ([D5](design.md#d5-intentos-y-fallo)).
- **Una fila `failed` se queda** hasta que alguien decida: se ve en `/health` y `memory_status`. No hay subcomando para reintentar o descartar, ni se borra sola.
- **Cola llena: `503 ingest_queue_full` y la fila se borra.** El contrato de `specs/ingest` no cambia: un save rechazado no queda guardado.
- **`operation_id`:** el daemon lo emite siempre (UUID) y lo devuelve en el `201` y el `202`; `POST /v1/memories` acepta uno opcional, único por agente mientras la fila exista. No se agrega a la herramienta `save_memory` ([D6](design.md#d6-operation_id)).
- **Cómo se entera el agente:** un bloque `ingest` en `/health`, que `memory_status` ya imprime, y el texto del `202` con el `operation_id`. Los contadores se leen de la tabla con un tope de tiempo, para que `/health` nunca espere detrás del worker ([D7](design.md#d7-ingest-en-health)).
- **El apagado deja de volcar.** Lo no ingerido ya está en la tabla. Un `ingest-pending.jsonl` de una versión anterior se importa una vez al arrancar, enmascarado, y se borra después del commit. El lector se mantiene hasta que se archive un cambio que lo quite.
- **`restore`:** las filas pendientes de la base reemplazada quedan en el snapshot `pre-restore` y no se traspasan. Se dice en la spec de storage.

## Alcance

1. Migración `V14`: la tabla `ingest_outbox` y sus índices; su fila en `PINNED`.
2. Operaciones del outbox en el store (insertar, tomar, anotar progreso, terminar, anotar fallo, olvidar, listar pendientes, resumen) y las decisiones puras (qué hacer al tomar una fila, cómo queda después de un fallo).
3. `MemoryRuntime::accept`: enmascara, escribe la fila, encola; borra la fila si la cola la rechaza.
4. Worker: toma la fila, se salta una entrega doble, escribe el progreso con cada pieza, termina con el vínculo y el borrado en una transacción, y anota el fallo.
5. Arranque: importa el volcado viejo y procesa las filas pendientes antes que lo nuevo. Apagado: deja de volcar.
6. Daemon: `POST /v1/memories` pasa por `accept`, devuelve `operation_id` y acepta uno; el handler olvida la fila de un fallo que ya le contestó al que guardó.
7. Bloque `ingest` en `/health`, en `Health` del cliente, en `memory_status` y en los textos del `202`.
8. `clawg purge` borra del outbox las filas que contienen el texto purgado y nombra los snapshots que las conservan.
9. Docs: `.claude/CLAUDE.md` y el mapa de `openspec/README.md`.

## Fuera de alcance

- Un manejador de SIGTERM o de cierre de consola (enfoque B): queda como endurecimiento, ya sin riesgo de pérdida.
- Una herramienta MCP nueva, un filtro de `list_memories` o un argumento nuevo en `save_memory`.
- Un comando `clawg outbox` para listar, reintentar o descartar filas `failed`; el chequeo de `doctor` sobre esas filas es de doctor-checks (31).
- Un aviso en `recall_context` (context-budget, 23, y agent-lifecycle-hooks, 25, usarán el bloque `ingest`).
- Cambiar el chunker, el dedup, la detección de contradicciones, `DRAIN_DEADLINE` o la capacidad de la cola.
- Ranking: `rag-eval` no aplica, y la ruta de `EvalIngest` (`crates/clawg-core/src/eval_api.rs:496`) no usa el outbox.
- Sobrevivir a un corte de luz (decisión de durabilidad).

## Datos guardados

Cambia el esquema: una tabla nueva, sin tocar ninguna otra ni ningún identificador guardado.

- **Migración:** `V14__ingest_outbox.sql` crea `ingest_outbox` y sus índices. No convierte datos: no hay filas del usuario que migrar. Abrir una base con la migración pendiente escribe el snapshot `pre-migration` de siempre en `<data>/backups`.
- **Volcado viejo:** si existe `ingest-pending.jsonl`, sus eventos válidos pasan a filas del outbox (enmascarados) en una transacción, y el archivo se borra después del commit. Las líneas ilegibles siguen yendo a `ingest-pending.rejected.jsonl`.
- **Test de la versión anterior:** una base sembrada hasta `V13` con memorias, piezas vinculadas y un proyecto abre con `V14`, cada fila idéntica, el outbox vacío; con un `ingest-pending.jsonl` al lado, sus eventos quedan en la base y el archivo desaparece.
- **Rollback:** una versión anterior rechaza una base con `V14` sin tocarla (`SchemaNewer`). Antes de bajar de versión, `clawg stop` con el binario nuevo drena lo pendiente; el camino de vuelta es `clawg restore` del snapshot `pre-migration`. Lo que quede pendiente o `failed` en la base no lo reinyecta el binario viejo.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- un save aceptado con el embedder cargando deja una fila confirmada antes del `202`, y otro runtime abierto sobre el mismo directorio **sin** apagado ordenado lo ingiere;
- lo mismo para un `202 unconfirmed`;
- un save confirmado no deja fila; uno que falla después del `202` deja la fila con su causa, visible en `/health` y en `memory_status` con su `operation_id`;
- un secreto guardado con el embedder cargando no está en ninguna columna del outbox, y el `201` sigue nombrando lo enmascarado;
- un save de 3 piezas cortado después de la segunda se retoma en la tercera, sin sumar `access_count` a las dos primeras y con las cabeceras originales en `memory_piece`;
- una entrega doble se salta; un evento que corta el proceso queda `failed` al tercer arranque y el worker sigue;
- el volcado viejo se importa enmascarado; la cola llena no deja fila; un `operation_id` repetido no crea otra fila;
- `purge` saca el texto también del outbox;
- la base de la versión anterior abre con todo igual.

Latencia del save, por agregar escrituras al camino caliente: el procedimiento de [ingest-durability](../2026-10-12-ingest-durability/verify.md) antes y después (100 saves secuenciales, corto y largo, perfil dev, modelo real). Umbral pre-comprometido: el p95 de un save corto no crece más de 20 ms (línea base: 32 ms p50 y 51 ms p99 corto; 546 ms y 702 ms largo, build debug). Se mide en Windows con el modelo real. `verify.md` da además el conteo de tests contra la base.
