# ingest-outbox — Diseño

Las referencias son a `main` (`210c692`).

## D1. Una tabla en `memory.db`

```sql
CREATE TABLE IF NOT EXISTS ingest_outbox (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT    NOT NULL,
    agent_id     TEXT    NOT NULL,
    content      TEXT    NOT NULL,
    event        TEXT    NOT NULL,
    state        TEXT    NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'failed')),
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    progress     TEXT    NOT NULL DEFAULT '[]',
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    UNIQUE (agent_id, operation_id)
);
CREATE INDEX IF NOT EXISTS idx_ingest_outbox_state ON ingest_outbox (state, id);
```

- `content` es el texto ya enmascarado, en su propia columna para que `purge` lo encuentre con `instr` (dentro del JSON estaría escapado). `event` es el `MemoryEvent` serializado con `content` vacío: viaja entero (los campos `#[serde(default)]` de `crates/clawg-core/src/memory/events.rs` siguen sirviendo para crecer) sin guardar el texto dos veces.
- `AUTOINCREMENT` porque el worker recuerda qué filas ya tomó en esta ejecución por su id (D2): un id reusado después de un borrado se confundiría con una fila ya vista.
- El `mpsc` sigue: deja de ser la verdad y pasa a ser el aviso de que hay una fila para procesar. El worker sigue siendo el único escritor de `memory` (`memory_worker`, `crates/clawg-core/src/memory/worker.rs:198`).
- Un evento sin fila (`IngestRequest::outbox == None`) se procesa como hoy. Es la ruta de los tests en proceso y de `EvalIngest` (`crates/clawg-core/src/eval_api.rs:496`), que mide con el mismo worker.

**Alternativas** (exploración, «Enfoques posibles» y «Descartado»): un manejador de señales con volcado periódico (B) sigue perdiendo ante un kill y deja texto crudo más tiempo; persistir sólo mientras carga el modelo (C) deja igual el `202 unconfirmed` y duplica los caminos; un archivo de registro propio no puede borrar su entrada en la transacción que inserta la memoria.

## D2. Progreso por pieza

El worker toma la fila (`outbox_claim`) antes de procesar. Cada escritura de una pieza (insertar o reforzar) recibe una nota `OutboxNote { row, progress }` y la aplica **en su misma transacción**: `UPDATE ingest_outbox SET progress = ?`. El progreso es una lista JSON de `{position, id, inserted, context}`: qué fila quedó en cada posición y con qué cabecera se embebió.

Al final, `outbox_finish` hace en una transacción el vínculo `memory_piece` (si hay dos piezas o más) y el `DELETE` de la fila. Para eso el cuerpo de `link_pieces` (`crates/clawg-core/src/memory/pieces.rs:180`) pasa a una función sobre una transacción abierta, que usan los dos.

Con eso:

- un kill después de una pieza deja la pieza **y** su progreso, o ninguna de las dos cosas;
- el replay salta las posiciones del progreso, no las refuerza y vincula con el `context` guardado;
- un kill entre la última pieza y el final deja el progreso completo: el replay sólo termina.

Las copias repetidas dentro del save (`multiplicity`) se refuerzan después de anotar su posición y sin nota: un kill en ese punto pierde a lo sumo un refuerzo, nunca agrega uno.

Si el progreso nombra una posición que el troceo de hoy no tiene (cambió `tokenizer.json` entre el corte y el replay), se descarta con un warn y el save sigue por el dedup, como hoy.

**Entrega doble.** Al arrancar, el worker lee las filas pendientes; una fila que el handler escribió antes de esa lectura llega también por el canal. El worker guarda los ids que tomó en esta ejecución y se salta el segundo.

**Alternativa:** todo el save en una transacción (A.1b). La detección de contradicciones es `async` y consulta la misma conexión pieza por pieza (`detect_contradiction`, `crates/clawg-core/src/memory/contradiction.rs:137`); habría que reescribir el corazón del worker como un closure síncrono.

## D3. Enmascarar antes de persistir

`MemoryRuntime::accept` llama a `redact` (`crates/clawg-core/src/memory/redact.rs`) antes de escribir la fila, y el evento encolado ya lleva el texto enmascarado. El worker conserva su pasada (`worker.rs:431`): sobre un texto ya enmascarado no encuentra nada nuevo (test `masking_twice_finds_nothing_new` de `redact.rs`). El `redacted` del `201` sale del enmascarado de `accept`, que el handler tiene en la mano, unido al del reporte (vacío salvo en un evento sin fila).

Se reabre la decisión D2 de secret-hygiene (`openspec/changes/archive/2026-10-14-secret-hygiene/design.md`) sólo para la copia persistida: el volcado viejo sí guardaba texto crudo, pero existía sólo entre un apagado y el próximo arranque; una fila puede vivir mientras el save esté pendiente o `failed`, y entra en cada snapshot.

## D4. Durabilidad

La inserción de la fila (y la importación del volcado viejo) se escribe con `synchronous = FULL` y después la conexión vuelve a su valor, `NORMAL` en WAL (`crates/clawg-core/src/memory/db.rs:197`). Así la fila sobrevive también a un corte de luz, a costa de un fsync por save aceptado; el resto de la base, incluidos el progreso y el `DELETE` del final, sigue en `NORMAL`. Perder el `DELETE` en un corte sólo hace que la fila se reproduzca, y el dedup la vuelve un refuerzo. Un `202` queda así más durable que un `201`, que es lo que se eligió en la tarjeta del hilo.

Costo por save: la inserción de la fila (antes de contestar), el `UPDATE` de intentos al tomarla y el `DELETE` del final, más el `UPDATE` de progreso dentro de las transacciones que ya existían. Tres commits chicos más; sólo el de la inserción paga un fsync.

## D5. Intentos y fallo

Decisiones puras en `crates/clawg-core/src/memory/outbox.rs`, sin I/O:

- `claim_step(attempts)`: con `attempts < MAX_ATTEMPTS` (3) se procesa y se suma uno; si no, la fila se rinde: queda `failed` con `last_error = "the daemon stopped N times while ingesting this save"`. Es lo que pasa con un evento que mata el proceso: deja de reinyectarse.
- `after_failure(attempts, transient)`: un fallo transitorio (`IngestFailure::EmbedderUnavailable`) devuelve el intento; uno real deja la fila `pending` con su causa hasta el tope, y `failed` al llegar.

Una fila que falló no se reintenta en la misma ejecución (el mismo texto falla igual): se reintenta en el próximo arranque.

**Quién borra la fila de un fallo que ya se contestó.** Si el handler recibe el reporte con un fallo y lo contesta (`500` o `503`), el que guardó ya se enteró: el handler borra la fila (`outbox_forget`). Lo hace el handler y no el worker porque sólo el handler sabe si el reporte llegó antes de que venciera la espera; si el worker lo decidiera por el resultado de `send`, un reporte enviado justo cuando vence el plazo borraría una fila de la que nadie se enteró.

## D6. `operation_id`

- El daemon emite un UUID v4 (`uuid` ya es dependencia de `clawg-core`) y lo devuelve en `201` y `202`.
- `SaveRequest` acepta `operation_id` opcional: de 1 a 128 caracteres ASCII visibles; si no, `400`. `SaveRequest` no usa `deny_unknown_fields`, así que el campo es aditivo.
- Un `operation_id` que ya tiene fila para ese agente no crea otra: con la fila `pending` responde `202` con `reason: "already_queued"`; con la fila `failed`, `500 ingest_failed` con su `last_error`. Después de completado, la fila ya no existe: un reenvío vuelve a guardar y el dedup lo absorbe como `reinforced`, lo mismo que hoy.
- No va en la herramienta `save_memory`: el modelo no tiene un id estable que repetir, y el esquema pagaría tokens en cada sesión.

## D7. `ingest` en `/health`

`HealthBody` suma `ingest`: `{pending, failed, oldest_pending_s, problems: [{operation_id, agent_id, state, attempts, error}]}`, con los hasta 5 problemas (filas con `last_error`) más recientes. Se lee con un `COUNT` agrupado y un `SELECT` sobre la tabla.

`/health` lo consulta `probe_running` antes de cada autoarranque con un tope de 1 s (`crates/clawg-cli/src/client.rs`), y la conexión es una sola y serializada: una escritura larga (un re-indexado) haría esperar a `/health`. Por eso la lectura tiene un tope propio de 250 ms; si vence, `ingest` se omite. Un cliente nuevo contra un daemon viejo, o contra uno ocupado, ve `ingest: null` y no inventa datos. La alternativa, contadores en memoria con el valor inicial leído al arrancar, puede divergir de la tabla.

El cliente agrega `#[serde(default)] ingest: Option<IngestHealth>` a `Health`, y `memory_status` lo imprime porque imprime el `Health` entero (`format_status`, `crates/clawg-cli/src/mcp/tools.rs:352`). Los textos del `202` en `save_outcome_text` (`crates/clawg-cli/src/mcp/server.rs:495`) nombran el `operation_id` y dicen que un fallo aparece en `memory_status`. No cambia ninguna descripción de herramienta: `agent_contract.json` queda igual.

## D8. Arranque y apagado

`ingest_worker_future` (`crates/clawg-core/src/runtime.rs:363`) deja de leer el volcado para reinyectarlo:

1. importa `ingest-pending.jsonl` si existe: cada evento válido, enmascarado, pasa a una fila nueva en una transacción; el archivo se borra recién después del commit (si el borrado falla, se avisa: la segunda importación la absorbe el dedup). Las líneas ilegibles siguen yendo a `ingest-pending.rejected.jsonl` con el lector de hoy (`spill::load`).
2. lee las filas `pending` en orden de id y las procesa antes que el canal, en el mismo lugar donde hoy va el volcado.

`MemoryRuntime::shutdown` (`runtime.rs:219`) mantiene el plazo de drenaje y deja de volcar: lo que el worker no empezó ya está en la tabla. Un evento sin fila que quede sin empezar (sólo en tests) se registra como no ingerido. `spill.rs` se reduce al lector y su formato.

## D9. `purge`

`purge` (`crates/clawg-core/src/memory/secrets.rs:81`) borra, en la misma transacción que la fila de `memory`, las filas del outbox cuyo `content` contiene el texto purgado (`instr(content, ?) > 0`), antes del `VACUUM` y el checkpoint. `backups_holding` (`secrets.rs:134`) nombra también los snapshots con una fila así; un snapshot anterior a V14 no tiene la tabla, y eso no cuenta como «puede tenerla».

## D10. `restore`

Un snapshot incluye la tabla. Un `restore` reemplaza las filas pendientes por las del snapshot; las de la base reemplazada quedan sólo en el `pre-restore` que `restore` ya escribe. No se traspasan: mezclar filas de dos bases rompería el orden y los ids de progreso. Se documenta en `specs/storage`.
