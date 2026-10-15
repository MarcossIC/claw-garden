# long-save-context — Diseño

Las decisiones de producto (puntero y contexto, la cabecera sólo embebida, las filas existentes sin vínculo) están en la [propuesta](proposal.md). Acá van las de implementación.

## D1. Dónde vive la cabecera

| Opción | A favor | En contra |
|---|---|---|
| a. Dentro de `content` | El vector y el texto guardado coinciden sin nada más | Cambia el hash, el FTS y lo que lee el agente; cada pedazo repite 48 tokens en `search_memory` y en `recall_context` |
| b. Recalcularla del pedazo 1 al re-embeber | Nada que guardar | `reembed_all` necesitaría el tokenizer, y si el pedazo 1 se borró, el re-embed cambia el vector del 2 sin que nadie lo pida |
| **c. En el vínculo (`memory_piece.context`)** | El re-embed la lee tal cual; el contenido, el hash y el FTS no cambian | Unos 200 bytes por pedazo |

**Elegida c.** El vínculo dice con qué contexto se armó el vector de esa fila, y nada más. Por eso una fila absorbida lleva `context` vacío: su vector se hizo antes y sin ese contexto (spec R3).

## D2. El tope de los pedazos

`PIECE_MAX_TOKENS = CHUNK_MAX_TOKENS - SAVE_HEAD_TOKENS - HEAD_JOIN_RESERVE` = 480 − 48 − 8 = 424, con un `const _: () = assert!(...)` al lado de los de `chunker/mod.rs:23-38`. La reserva de 8 tokens cubre el separador `"\n…\n"` y los tokens que el tokenizer puede sumar al unir dos textos (la cuenta de tokens no es aditiva en el borde).

| Opción | A favor | En contra |
|---|---|---|
| **a. Un tope para todos los pedazos de un save largo** | El splitter ya parte con un tope uniforme (`TokenSplitter::split`, `chunker/token_splitter.rs:40`) | El pedazo 1 queda 56 tokens más corto de lo que podría |
| b. 480 para el pedazo 1 y 424 para el resto | Un pedazo menos en algún save | Hay que partir en dos pasadas y volver a calcular el solapamiento en el borde |

**Elegida a.** La decisión de partir sigue siendo la de hoy: si `split(texto, 480, 64)` da un pedazo, ése es el resultado (spec R1). Si no, `split(texto, 424, 64)`. La cabecera es `split(texto, 48, 0)[0]`. Las tres son funciones puras de `ChatChunker`, que devuelve los pedazos y la cabecera juntos (`LongSplit { pieces, head: Option<String> }`), así el worker no vuelve a tokenizar.

`embedding_text(context: &str, piece: &str) -> Cow<str>` vive en `chunker/mod.rs`: devuelve el pedazo si `context` está vacío, y `context + "\n…\n" + piece` si no. La usan el worker y `reembed_all` (spec R7).

## D3. El vínculo: tabla aparte

```sql
CREATE TABLE IF NOT EXISTS memory_piece (
    save_id    INTEGER NOT NULL,
    piece      INTEGER NOT NULL,
    memory_id  INTEGER NOT NULL,
    context    TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (save_id, piece)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_memory_piece_memory ON memory_piece (memory_id);
```

| Opción | A favor | En contra |
|---|---|---|
| a. Columnas `save_id` y `piece` en `memory` | Un `JOIN` menos | Una fila sólo puede estar en un save, y un pedazo absorbido por el dedup ya es de otro save o de ninguno. Volver atrás pide `DROP COLUMN` sobre la tabla más grande |
| **b. Tabla `memory_piece`** | Una fila absorbida puede estar en varios saves; volver atrás es un `DROP TABLE`; es la forma de `memory_derivation` (`V10__synthesis.sql`) | Un `JOIN` al buscar y al leer |

**Elegida b.** Sin claves foráneas, por la misma razón que `memory_derivation`: las filas de `memory` no se borran, sólo se marcan, y el vínculo a una fila borrada es historia que vale la pena tener (`read_memory` la nombra como faltante).

`save_id` es `COALESCE(MAX(save_id), 0) + 1` dentro de la misma transacción que escribe los vínculos. La conexión es una sola y serializada (`tokio_rusqlite`), así que dos saves no pueden leer el mismo máximo.

## D4. Cuándo se escribe el vínculo

| Opción | A favor | En contra |
|---|---|---|
| a. En la transacción de cada insert | El vínculo nunca falta para una fila insertada | El `save_id` hace falta antes del primer insert, los pedazos absorbidos no pasan por un insert, y un save cortado deja un grupo a medias |
| **b. Al final del evento, en una transacción** | Un save completo o ningún vínculo; los absorbidos entran igual | Si el proceso muere entre el último insert y el vínculo, las filas quedan sueltas |

**Elegida b.** Lo que queda sin vínculo es exactamente el estado de hoy, y guardar el mismo texto de nuevo lo arregla: todos los pedazos se absorben por hash y el vínculo se escribe (spec R3). El método es `MemoryStore::link_pieces(&[(piece, memory_id, context)])` en `pieces.rs`, y el worker lo llama con lo que ya tiene en `stored` (`worker.rs:477`) y las cabeceras de las filas que insertó.

## D5. El colapso, en la búsqueda misma

| Opción | A favor | En contra |
|---|---|---|
| **a. En `search_all_memories_inner`, antes del `truncate(k_final)`** (`search.rs:1115-1116`) | Un solo lugar: `search_memory`, `recall_context`, el detector y `rag-eval` ven el mismo top | El detector también ve un pedazo por save viejo |
| b. Sólo en `prompt_builder` y en el handler de `/v1/search` | El detector no cambia | Dos órdenes distintos para la misma consulta, y los dos tienen que pedir candidatos de más para llenar `k` |

**Elegida a.** Que el detector vea un pedazo por save es coherente con R8 de `specs/disputes`: un save es una afirmación de un autor. Mientras se guarda un save, sus propios pedazos todavía no tienen vínculo (se escribe al final, D4), así que el colapso no interfiere con la exclusión de hermanos. El pool por tabla (30/5/5) es el techo real (`prompt_builder.rs:124-130`), así que colapsar antes de cortar llena `k` con lo que ya se había traído.

`RankedHit` gana `piece: Option<PiecePosition { piece: i64, pieces: i64 }>`, serializado con `skip_serializing_if = "Option::is_none"`. Los vínculos de los hits `memory` se leen con una consulta (`SELECT memory_id, save_id, piece, (SELECT COUNT(*) ...) FROM memory_piece WHERE memory_id IN (...)`, quedándose con el menor `save_id` por fila), y `collapse_saves(hits, &links) -> Vec<RankedHit>` es pura. El ancla de `rag-eval` no tiene vínculos, así que la consulta vuelve vacía y el orden es el de hoy.

## D6. Leer el save: una ruta aparte

| Opción | A favor | En contra |
|---|---|---|
| a. Ampliar `GET /v1/memories/{kind}/{id}` (`api.rs:883-893`) | Una ruta menos | Hoy no tiene scope por agente; sumárselo cambia lo que ve quien ya la usa, y cada lectura pagaría el armado del save |
| **b. `GET /v1/memories/{kind}/{id}/save?agent=`** | El scope de visibility-policy desde el principio; la ruta vieja no cambia | Una ruta más |

**Elegida b.** `read_save_core(store, kind, id, owner) -> Option<SaveText>` en `pieces.rs`, con `SaveText { id, kind, pieces, ids: Vec<Option<i64>>, missing: Vec<i64>, text }`. `join_pieces(&[&str]) -> String` es pura (spec R6). Los pedazos de un save los inserta el agente del evento, pero uno absorbido puede ser de otra fila del mismo agente con otro proyecto; el filtro por `agent_id` y `deleted_at IS NULL` se aplica a cada posición, y lo que no pasa queda en `missing`.

`format_save(&SaveText) -> String` en `tools.rs`: `#58 is piece 2 of 4 of one save: #57, #58, (missing), #60` en la primera línea, y el texto debajo. Para una fila sin vínculos: `#58` y su contenido.

## D7. La medición

- **El subconjunto va al final de `generate`** (`synth/mod.rs:59-82`). Consume la semilla después de todo lo demás, así que las filas y los casos de hoy salen idénticos (spec R9); el sha del set de la semilla 7 cambia porque el archivo crece.
- **El relleno se arma con plantillas propias** (`templates::long_filler`), sin la clave, con números y palabras sorteados por fila para que dos filas `long` no compartan pedazos (el dedup los absorbería y el caso mediría otra cosa) y para que ninguna repita 6 palabras de la base (`shares_word_run`, el chequeo de los distractores).
- **El largo** se asegura con `ChatChunker::with_fallback` sobre el texto final: 3 pedazos con el tope de producción en modo sin tokenizer, que es el más conservador (4 bytes por token). Con el tokenizer real salen más.
- **Agent-view** lee `answer` y lo busca en el texto que el agente recibe. Para la respuesta alcanzable, el runner llama `GET .../save` con el id de la primera línea de búsqueda que carga la fila esperada (`SearchLine` gana `id`); si la ruta da 404 por no existir, cuenta lo visible.
- **La base** es el commit del eval: tiene el parser y el subconjunto, y guarda y busca como `main`.

## D8. Orden de los commits

1. Medición: `long`, `answer` y las métricas de agent-view, con el parser aceptando `piece=K/N` (todavía nadie lo imprime).
2. `V11__memory_piece.sql`, `pieces.rs` con `link_pieces`, `read_save_core` y `join_pieces`, y el test de datos de V10.
3. Chunker y worker: el tope, la cabecera, el texto embebido y el vínculo al final del evento; el re-embed con contexto.
4. Búsqueda: `PiecePosition`, el colapso y `piece=K/N` en los dos formatos.
5. HTTP y MCP: la ruta `/save`, `read_memory`, las descripciones y los conteos.
6. Docs: `.claude/CLAUDE.md` (herramienta y ruta) y la lista del `README.md`.

Cada uno compila y pasa la suite solo. Si una pieza queda sin llamador hasta el commit siguiente (y el compilador avisa código muerto), se junta con ése, como en memory-update-tool.
