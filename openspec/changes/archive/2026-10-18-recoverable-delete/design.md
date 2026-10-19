# recoverable-delete — Diseño

Las decisiones de producto (superficie, tablas, dedup, secretos, disputas, recencia) están en la [propuesta](proposal.md). Acá van las de implementación.

## D1. El core recibe el embedder, no el vector

| Opción | A favor | En contra |
|---|---|---|
| **a. `restore_memory_core(store, embedder: &dyn Embedder, id, owner)`** | El core arma el texto de embedding (contexto del pedazo incluido) en un solo lugar, y los tests usan los embedders de prueba que ya hay | El core llama al embedder fuera de la transacción, en dos pasos |
| b. El daemon calcula el vector y lo pasa | El core no toca el embedder | El daemon tendría que leer la fila y el vínculo para armar el texto: la lógica del re-embed duplicada fuera del core |

**Elegida a.** Pasos:

1. Leer la fila (`content`, `agent_id`, `deleted_at`) y su contexto de pedazo con una consulta, la misma subconsulta de `select_reembed_batch` (`crates/clawg-core/src/memory/maintenance.rs:640-643`). Si no hay fila, está viva o es ajena: `NotFound`.
2. Chequeo puro del secreto: `redact(&content).kinds` (`crates/clawg-core/src/memory/redact.rs:131`). Va antes del embedder para no gastar una inferencia en algo que se va a rechazar.
3. `embedder.embed_with_role(embedding_text(context, content), EmbedRole::Passage)` y `to_unit` (`crates/clawg-core/src/memory/vector.rs:41`). Fuera de la transacción: el embedder es async y lento, y la única conexión queda libre mientras tanto.
4. En una transacción: releer `deleted_at` y el dueño (otro restore pudo ganar en el medio: `NotFound`), buscar la gemela, `UPDATE`, vector, FTS, reposo.

## D2. El FTS se suma a mano, con la misma forma que el trigger de insert

```sql
INSERT INTO memory_fts(rowid, content, agent_id, project_id, memory_type)
SELECT id, content, agent_id, project_id, memory_type FROM memory WHERE id = ?1
```

Corre después del `UPDATE ... SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL` y sólo si ese `UPDATE` tocó una fila. Así la suma ocurre exactamente una vez por transición borrada → viva, que es el par simétrico de `memory_fts_au_deleted` (`crates/clawg-core/migrations/V8__memory_fts.sql:73-78`).

| Opción | A favor | En contra |
|---|---|---|
| **a. Sentencia en la transacción del restore** | Sin migración; rollback = cambiar de binario | Un camino futuro que des-borre sin pasar por acá desbalancea el índice |
| b. Trigger `AFTER UPDATE OF deleted_at WHEN old NOT NULL AND new IS NULL` (V13) | Cubre cualquier camino, también un `UPDATE` a mano | Migración con snapshot pre-migración, y un binario anterior rechaza la base: el rollback pasa por `clawg restore` y pierde lo guardado después |

**Elegida a.** El comentario de V8 que dice que no hay camino que recupere una memoria no se puede editar (una migración aplicada no cambia, `migrations.rs` compara su checksum). La aclaración va en el comentario de la constante del restore.

## D3. Un error tipado por rechazo

`RestoreError` con `thiserror` en `admin.rs`:

- `NotFound` — inexistente, viva, ajena o purgada;
- `OnlyMemory(MemoryKind)` — se valida antes de abrir nada;
- `LiveTwin { id, twin }`;
- `HoldsSecret { id, kinds: Vec<&'static str> }`;
- `Embed(EmbedError)`;
- `Store(MemoryError)`.

El daemon los mapea a 404, 400, 409, 409, 500 y 500. El 503 lo decide antes `ready_embedder` (`crates/clawg-cli/src/daemon/api.rs:756`), como en la búsqueda. Devuelve `RestoredRow { id, status, superseded_by }`.

## D4. Ruta: `POST /v1/memories/{kind}/{id}/restore`

| Opción | A favor | En contra |
|---|---|---|
| **a. `POST .../restore`** | Una acción con nombre, como `/disputes/resolve`; no se confunde con un cambio de campos | Una ruta más |
| b. `PATCH` con `deleted: false` | Sin ruta nueva | Mezcla dos contratos: `update_memory` contesta 404 a una borrada y valida "al menos un campo" ([`specs/update`](../../../specs/update/spec.md) R1 y R3) |
| c. `DELETE` inverso (`PUT`) | Simétrico | Nadie lo lee como "deshacer" |

**Elegida a.**

## D5. El listado de borradas

`ListParams` gana `deleted: bool`. Con `true`, los tres `query_*_rows` cambian `deleted_at IS NULL` por `deleted_at IS NOT NULL` y ordenan por `deleted_at DESC, id DESC`; la mezcla de `kind: None` ordena igual. `list_params_from_query` rechaza `deleted=true` con `q` (las borradas no tienen FTS), en la función pura de hoy (`api.rs:198`).

El puente no lo expone: `list_memories` mantiene sus cinco argumentos (`the_viewer_is_not_exposed_to_the_model`, `crates/clawg-cli/src/mcp/server.rs:1528`).

## D6. `clawg undelete`, por el daemon

| Opción | A favor | En contra |
|---|---|---|
| **a. `clawg undelete <ID>`, sólo con el daemon corriendo** | Usa el embedder cargado y el lock que el daemon ya tiene | Sin daemon no anda |
| b. Abrir la base y cargar el modelo en el proceso del CLI | Anda sin daemon | Cargar ONNX en otro proceso, y el contrato de embedding lo chequea el arranque del daemon (`bootstrap.rs`) |
| c. `clawg memories undelete` | Agrupado con el visor | `clawg memories` promete no escribir (`crates/clawg-cli/src/memories.rs:1`, `cli.rs:115`) |
| d. `clawg restore --row` | Una palabra conocida | `clawg restore` es el de snapshots (`cli.rs:74`), y un flag que cambia de qué se habla confunde |

**Elegida a.** Como `memories`, no arranca un daemon: sin uno, el error dice cómo arrancarlo. Sólo `memory`, así que no lleva `--kind`, como `purge`.
