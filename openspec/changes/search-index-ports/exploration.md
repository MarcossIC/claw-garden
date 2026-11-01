# search-index-ports — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (1, 3) | M–L, con puerta | knn-prefilter, storage-read-side, storage-write-side | exploración (borrador) |

**Tesis:** si lo que se quiere es probar otro motor, lo que más cambia entre motores es la búsqueda vectorial y la léxica, no las tablas. Dos puertos chicos, `VectorIndex` y `LexicalIndex`, permiten comparar motores con `rag-eval --diff` sin tocar el resto del almacenamiento. El costo real no está en el trait: un motor fuera de SQLite rompe la transacción única de memoria, vector y texto.

## Esto no reabre el índice HNSW

La fila «Índice HNSW» de `openspec/README.md` lo descartó porque «vec0 es fuerza bruta y alcanza para un store personal» y «cambiar de motor rompe la transacción única memoria + vector + FTS». Las dos razones siguen en pie. Este cambio no propone adoptar otro motor. Propone que probar uno no requiera reescribir la búsqueda, y va con puerta: sin un candidato concreto, cierra sin código.

## Problema (verificado contra el código)

- **La búsqueda es SQL con `vec0` adentro.** `SEARCH_SQL_MEMORY` (`crates/clawg-core/src/memory/search.rs:259`) une el índice con `memory` y filtra en el mismo `WHERE`. El comentario de `:245-252` explica que el filtro corre después del corte a `k`.
- **Vectores repartidos.** `memory_vectors` se escribe desde `admin.rs`, `portability/import.rs`, `secrets.rs` y `store_insert.rs`. `project_doc_vectors` sólo desde `project_docs/mod.rs`, que está mejor contenido.
- **El detector de contradicciones usa la misma búsqueda** (`crates/clawg-core/src/memory/contradiction.rs`), así que un motor distinto cambia también qué candidatos ve.
- **Lo léxico depende de FTS5.** `memory_fts` y `project_doc_fts` (`crates/clawg-core/migrations/V8__memory_fts.sql:36`, `V12__project_docs.sql:45`), sincronizadas con triggers (`V8__memory_fts.sql:63`, `:73`, `:84`). La consulta es de `search.rs:722-759` y `project_docs/mod.rs:421-429`.
- **La partición ya existe en docs** (`V12__project_docs.sql:36-39`) y no en memorias (`V2__vectors.sql:6-13`). De esa diferencia trata [knn-prefilter](../knn-prefilter/exploration.md).

## Forma candidata

```rust
pub trait VectorIndex {
    /// Los más cercanos que `filter` admite. Cómo filtra es asunto del motor:
    /// antes del corte, después con reintento, o por distancia exacta.
    fn nearest(&self, r: &ReadTx, q: &UnitVector, k: usize, filter: &KnnFilter)
        -> Result<Vec<Neighbor>, StorageError>;
    fn upsert(&self, tx: &WriteTx, id: MemoryId, v: &UnitVector) -> Result<(), StorageError>;
    fn remove(&self, tx: &WriteTx, id: MemoryId) -> Result<(), StorageError>;
}
```

Tres cosas tiene que resolver el diseño:

1. **Filtro como dato.** `KnnFilter` (alcance, proyecto, estado) permite que cada motor decida. La decisión de knn-prefilter se vuelve el comportamiento del adaptador `vec0`, no algo fijado en el puerto.
2. **Índice transaccional o índice derivado.** `vec0` participa en la transacción. Un motor externo no. Un índice derivado necesita otro contrato: se puede reconstruir desde `memory`, se atrasa, y hay que saber hasta dónde llegó. Ese es el punto que hizo descartar HNSW, y no se puede esconder en el trait.
3. **Dónde vive el vector.** Con un motor externo, ¿`memory_vectors` sigue siendo la fuente? Mantenerlo permite reconstruir, y es lo que hace barato probar.

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Puertos angostos, el adaptador `vec0` como única implementación de producción y el candidato sólo en `clawg-eval`** | Se compara con `rag-eval --diff` sin que producción enlace nada nuevo, como ya hace la feature `eval` | Exige un candidato real, o queda un trait sin uso |
| B. Puerto de almacenamiento completo primero (storage-ports) | Una sola abstracción | Mucho más caro, para probar algo que sólo necesita esto |
| C. Comparar motores por fuera, exportando vectores | Sin tocar el código | Mide el motor solo, sin alcance, sin puntaje y sin el golden, así que no contesta la pregunta |

**Recomendado: A**, con la puerta de abajo.

## Alcance candidato

1. **Puerta:** nombrar un candidato y qué mejora se espera, con un umbral fijado antes. Sin eso, cierra sin código.
2. `VectorIndex` y su adaptador `vec0`, sin cambiar conducta.
3. Una batería de contrato que cualquier adaptador tiene que pasar.
4. El candidato como adaptador sólo en `clawg-eval`, medido con `rag-eval --diff`, `--agent-view` y `load-eval`.
5. `LexicalIndex`, sólo si hay un candidato léxico. FTS5 no está en discusión hoy.

### No-alcance

- Adoptar un motor en producción. Eso es otro cambio, con migración y con el `model-reembed` que haga falta.
- Reemplazar el resto de SQLite: es storage-ports.

## Decisiones abiertas

1. Qué motor se prueba primero, y qué problema se espera que resuelva: latencia a escala, memoria, o recuperación.
2. Si `upsert` va en el puerto, o si un candidato de evaluación se llena aparte desde `memory_vectors`. Lo segundo evita resolver el índice derivado hasta que alguien lo adopte.

## Cómo se mide

- **Test en rojo:** la batería de contrato contra `vec0`, sobre el comportamiento que hoy fijan los tests de `search.rs`.
- `rag-eval --diff` sin casos movidos con el adaptador `vec0`: el puerto no cambia nada.
- Con un candidato, el umbral fijado en la puerta, con las reglas de `docs/reference/testing.md`.

## Interacciones

- **knn-prefilter:** va antes. Decide cómo filtra `vec0`, y este puerto hereda esa decisión.
- **storage-read-side y storage-write-side:** dan `ReadTx` y `WriteTx`.
- **embedder-bakeoff y model-reembed:** otra dimensión de vector cambia el contrato del puerto.
