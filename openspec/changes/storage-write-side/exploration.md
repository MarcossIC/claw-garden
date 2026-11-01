# storage-write-side — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L–XL, por entregas | store-boundary, memory-module-layout, domain-types; conviene después de retire-legacy-kinds | exploración (borrador) |

**Tesis:** hoy cualquier módulo escribe cualquier tabla: `memory` se escribe desde al menos 11 módulos y `memory_conflict` desde 8, así que las reglas de cada tabla no tienen dueño. Este cambio le da un dueño a cada grupo de tablas (un agregado) y una unidad de trabajo para lo que cruza grupos. Los agregados solos no alcanzan: un agregado es un límite de consistencia, de modo que lo transversal es justamente lo que no cabe en uno.

## Censo

Aproximación estática sobre el código de producción de `clawg-core`, sin `eval_api` ni lo que está bajo `#[cfg(test)]`. Una función cuenta las tablas que escribe ella y lo que llama, resuelto por nombre. El SQL armado con `format!` no se ve, y un nombre repetido puede sumar de más.

- 41 funciones abren una transacción. En 7 no se detecta ninguna tabla: las de `metrics/store.rs`, y las que reciben el trabajo como parámetro o arman el SQL (`retrying`, `merge.rs:385`).
- De las 34 restantes, **18 escriben un solo grupo de tablas y 16 cruzan grupos**. Es una cota superior: `declare_subfolder` cuenta como una, y son dos transacciones (`crates/clawg-core/src/memory/subfolder.rs:184-187` y `:197`).

Escritores por tabla, directos:

| Tabla | Módulos |
|---|---|
| `memory` | 11: `access`, `admin`, `dispute`, `maintenance`, `portability/import`, `portability/undo`, `relocate`, `revival`, `secrets`, `store_insert`, `topic` |
| `memory_conflict` | 8: `admin`, `dispute`, `maintenance`, `portability/import`, `portability/undo`, `secrets`, `store_insert`, `topic` |
| `project` | 6 |
| `project_path`, `project_suggestion` | 5 cada una |
| `memory_vectors`, `memory_piece`, `memory_derivation`, `project_remote` | 4 cada una |

Esto es lo que más duele, más que la transacción: una regla como «una versión viva por tema» no tiene un solo lugar donde cumplirse.

## Grupos candidatos

| Agregado | Tablas | Observación |
|---|---|---|
| Memoria | `memory`, `memory_vectors`, `memory_fts`, `memory_piece`, `memory_derivation`, y probablemente `memory_conflict` | La raíz natural. Una fila sin su vector o su texto no es una memoria válida |
| Outbox | `ingest_outbox` | Ya está casi contenido: 6 de sus transacciones sólo la tocan a ella (`outbox.rs`) |
| Proyecto | `project`, `project_path`, `project_remote`, `project_suggestion`, `project_subfolder` | |
| Docs | `project_doc_file`, `project_doc_chunk`, `project_doc_vectors`, `project_doc_fts` | El caso más limpio: sólo `project_docs` las escribe, salvo la purga |
| Diarios | `project_merge*`, `project_delete*`, `memory_move*`, `memory_import*` | No son agregados: son el registro de una operación, y pertenecen a ella |

Con `memory_conflict` dentro de Memoria, `judge_pair`, `restore_memory_core` y `finish_save` dejan de cruzar. Sin los tipos viejos de retire-legacy-kinds, tampoco cruzan `update_memory_core` ni `delete_memory_core`. Eso deja **unas diez operaciones realmente transversales**, y son tres familias.

## Las tres familias de lo transversal (verificado leyendo cada una)

### 1. Guardado durable: memoria y progreso juntos

`insert_noting` (`crates/clawg-core/src/memory/store_insert.rs:427-446`), `insert_unless_twin` (`:454-492`), `reinforce_noting` (`:373`) y `outbox_finish` (`crates/clawg-core/src/memory/outbox.rs:481-508`).

La garantía es de producto: «a process that dies after it leaves both or neither» (`store_insert.rs:424-426`). Sin ella, un reintento después de una caída reforzaría o insertaría dos veces.

**Hoy la unidad de trabajo existe disfrazada de callback:** `note: impl FnOnce(i64) -> Option<OutboxNote>` (`store_insert.rs:380`, `:431`, `:459-460`) le pasa a la memoria el progreso que tiene que escribir, y `store_insert` termina llamando a `apply_note`. La memoria conoce al outbox sólo porque no hay otra forma de compartir la transacción.

### 2. Operaciones de proyecto con pasada de prueba

`register_project` (`crates/clawg-core/src/memory/project.rs:941-969`), `run_batch` (`crates/clawg-core/src/memory/relocate.rs:585-615`) y lo que corre por `retrying` (`crates/clawg-core/src/memory/merge.rs:385-404`): unión y su deshacer (`merge.rs:423`, `:602`, `:612`) y borrado y su deshacer (`crates/clawg-core/src/memory/project_delete.rs:628`, `:960`, `:979`).

Tienen una forma propia: **corren una vez en una transacción que se suelta, para averiguar, y otra que confirma**, con un snapshot en el medio (`relocate.rs:590-612`). Soltar una transacción es parte del diseño, y cualquier contrato tiene que admitirlo. Los helpers reciben `&Connection` (`relocate.rs:587`, `merge.rs:389`), así que el tipo no dice si hay una transacción abierta.

### 3. Procesos largos con diario, y reescrituras con el store parado

- **Varias transacciones con diario:** `import::write` (`crates/clawg-core/src/memory/portability/import.rs:2360`, lotes en `:2442`) y `undo_import` (`crates/clawg-core/src/memory/portability/undo.rs:1217`, lotes en `:1261`). No son una transacción: las tablas de diario son lo que permite deshacer y retomar.
- **Con el store parado:** `purge` y `purge_delete` (`crates/clawg-core/src/memory/secrets.rs:189`, `:325`) abren su propia conexión (`open_to_rewrite`, `:475`), arman SQL por nombre de tabla (`:370`, `:388`) y leen nombres de tabla guardados como dato en `project_delete_row.tbl` (`:352`, `:359-368`).

## Lo que la escritura da por hecho

- **Un solo escritor.** Ni `idx_memory_dedup` ni `idx_memory_topic` son `UNIQUE`. La unicidad depende de que el worker sea el único consumidor (`store_insert.rs:356-369`, `crates/clawg-core/migrations/V6__content_hash.sql:9-13`, `crates/clawg-core/src/runtime.rs:212-220`).
- **Lógica en la base.** Seis triggers: tres mantienen el índice de texto (`V8__memory_fts.sql:63`, `:73`, `:84`) y tres son regla de negocio: adoptan un proyecto al guardar, con el nombre derivado por `GLOB`, y reviven uno borrado (`V29__project_delete.sql:55-85`). Tres UDF: `content_hash`, `compute_tier` y `demote_tier`.
- **Sin `await` en la transacción.** El trabajo caro va antes (`PreparedRow::new`, `store_insert.rs:433`), y la clausura de `.call` es síncrona, así que no se puede esperar adentro. Hay que conservar esta propiedad.

## Forma candidata

```rust
store.write(|tx: &WriteTx| {                       // una unidad de trabajo
    let id = memories::insert(tx, &row)?;          // agregado Memoria
    outbox::note_inserted(tx, outbox_row, id)?;    // agregado Outbox, misma transacción
    Ok(id)
}).await?
```

Reglas:

1. **Un agregado es un módulo, y es el único que escribe sus tablas.** Se fija en `crates/clawg-cli/tests/conventions.rs`.
2. **Un agregado no importa a otro.** `memories` no conoce a `outbox`: sale el callback `note`.
3. **Lo transversal es un caso de uso que compone agregados dentro de un `WriteTx`.** Es explícito y se puede leer de corrido.
4. **Las funciones de un agregado son síncronas.** Lo asíncrono queda en el borde.
5. **`WriteTx` no tiene `Deref` a `Connection`** y deja a la vista que hay una transacción abierta. Abandonarla es explícito.
6. **Todavía sin traits.** Salen en storage-ports, si hay una segunda implementación.

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Agregados con unidad de trabajo** | Cada tabla tiene dueño, sin perder ninguna garantía. Lo transversal se escribe como lo que es | Mucho código por mover |
| B. Agregados estrictos, una transacción por agregado, y eventos entre ellos | Es la forma clásica de DDD | Pierde «both or neither» y obliga a compensar. Se paga para resolver un problema de sistemas distribuidos que una base embebida no tiene. Rechazada |
| C. Un solo agregado que contenga todo | Sin transversales | Vuelve al módulo que hace todo. Rechazada |
| D. Un repositorio por tabla | Lo conocido | Hay más de 30 tablas, y ninguno expresa una transacción que cruza. Rechazada |
| E. Mover más reglas a triggers | Atomicidad gratis | Más lógica atada a SQLite y más difícil de probar. Rechazada |

**Recomendado: A.**

## Alcance candidato, en entregas

1. **Medir con exactitud:** leer cada función del censo y dejar la lista real de lo que escribe y de lo transversal. Fijar la regla de escritores en `conventions.rs`, con una lista de excepciones que se achica en cada entrega.
2. **`WriteTx` y `MemoryStore::write`.**
3. **Docs:** el caso más limpio, para probar la forma.
4. **Outbox:** casi contenido, y saca el callback `note` junto con la entrega de Memoria.
5. **Proyecto:** con las pasadas de prueba como concepto propio.
6. **Memoria y conflictos:** el grande, al final y con la forma ya probada.
7. **Operaciones:** import, undo, purga, unión, mudanza y borrado, sobre los agregados.

### No-alcance

- Cambiar esquema, conducta o garantías.
- Traits (storage-ports) o un motor distinto (search-index-ports).
- Paralelizar el worker. La garantía de un solo escritor sigue en pie, ahora escrita.

## Decisiones abiertas

1. Si `memory_conflict` va dentro de Memoria. Parece que sí: un par no existe sin sus dos filas.
2. Los triggers de `V29`: si la regla de adopción se queda en SQL o baja al agregado Proyecto. Bajarla le ahorra la regla a un adaptador futuro, pero una migración sobre triggers existentes es delicada. El índice de texto de `V8` puede quedar en la base, como infraestructura.
3. Dónde viven las reglas que hoy dependen del único escritor, y si se reemplazan por `UNIQUE` reales. `store_insert.rs:364-369` ya describe ese camino, y es lo que se necesitaría para una base concurrente.
4. Cómo modela `WriteTx` la pasada de prueba y el reintento de `retrying`.
5. Si `open_to_rewrite` da el mismo `WriteTx`. Debería, para que la purga use los mismos agregados.
6. Si el nombre de tabla guardado en `project_delete_row.tbl` se queda como está: un cambio de esquema acá pide test sembrado.
7. Si cada entrega sale en su propia versión o el cambio se publica entero.

## Cómo se mide

- **Test en rojo:** la regla de escritores en `conventions.rs`, que hoy falla por 11 módulos sobre `memory`.
- **Conducta:** la suite pasa, y los tests de caída y de reintento del outbox siguen igual.
- `perf-bench` y `ingest.stage` sin regresión.
- `doctor` limpio sobre una base con datos.

## Riesgos

- Es el cambio más grande de la serie y toca la ruta de guardado, la más sensible de datos. Por eso va en entregas y con la forma probada primero en docs.
- Choca con todo lo que escriba en la base mientras dure. Conviene no tener abiertos cambios de ingest.
- El censo es una aproximación. La entrega 1 existe para no planificar sobre un número equivocado.

## Interacciones

- **storage-read-side:** es la otra mitad y comparte la forma. Las lecturas que deciden una escritura, como el dedup, quedan de este lado.
- **domain-types:** da los tipos de estas firmas.
- **core-use-cases:** los casos de uso son los que abren `store.write`.
- **read-connections:** mantiene un solo escritor, así que no se pisa con esto.
- **storage-ports:** extrae los traits de estos módulos.
