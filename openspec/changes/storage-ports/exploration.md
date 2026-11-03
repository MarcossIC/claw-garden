# storage-ports — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L, más el adaptador; con puerta | storage-read-side, storage-write-side | exploración (borrador) |

**Tesis:** el patrón repository sirve acá, pero no como un trait por conexión ni uno por tabla. Un trait gigante aparece cuando se le pide a una interfaz lo que necesita todo el sistema. Si lectura y escritura ya están separadas y cada grupo de tablas tiene su dueño, ningún caso de uso necesita todo: cada uno pide lo suyo, y el trait gigante deja de existir. Este cambio extrae esos traits del código concreto que dejaron los dos cambios anteriores, sólo si hay una segunda implementación que los justifique.

## La puerta

`openspec/config.yaml` lo dice: «No new dependency, visibility or abstraction without a real use». Un trait con una sola implementación no da nada que el tipo concreto no dé ya.

**Antes de la propuesta hay que nombrar qué base se quiere probar y qué problema tiene que resolver.** Si no hay ninguna, el cambio cierra sin código, y lo que queda de storage-read-side y storage-write-side ya vale por sí mismo: cada tabla con un dueño, la lectura separada, y un adaptador futuro con un lugar definido.

Si lo que se quiere probar es sólo la búsqueda, el camino es [search-index-ports](../search-index-ports/exploration.md), que es mucho más barato.

## Cómo se evita el trait gigante

Hoy hay 94 funciones que llaman a la conexión (54 que leen y 40 que escriben, ver [storage-read-side](../storage-read-side/exploration.md)). Un solo trait tendría del orden de cien métodos, y cualquier adaptador nuevo tendría que implementarlos todos antes de responder la primera consulta.

La solución es la segregación de interfaces, por capacidad:

```rust
// Cada uno es chico; el número exacto sale de los dos cambios anteriores.
pub trait MemoryWrites  { fn insert(&self, row: &NewRow) -> Result<MemoryId, StorageError>; /* … */ }
pub trait OutboxWrites  { fn note(&self, row: OutboxRow, note: &OutboxNote) -> Result<(), StorageError>; /* … */ }
pub trait ProjectWrites { /* … */ }
pub trait RetrievalReads { fn nearest(&self, q: &KnnQuery) -> Result<Vec<RawHit>, StorageError>; /* … */ }

// Un caso de uso pide exactamente lo que usa. Lo transversal se expresa solo:
fn finish_save(tx: &(impl MemoryWrites + OutboxWrites), save: &Save) -> Result<Stored, StorageError> { /* … */ }
```

```rust
// El adaptador pone el límite de la transacción.
pub trait Store: Send + Sync + 'static {
    type Read<'t>: RetrievalReads + ProjectReads + AdminReads;
    type Write<'t>: MemoryWrites + OutboxWrites + ProjectWrites + DocsWrites;

    async fn read<T, F>(&self, f: F) -> Result<T, StorageError>
    where F: for<'t> FnOnce(&Self::Read<'t>) -> Result<T, StorageError> + Send + 'static, T: Send + 'static;

    async fn write<T, F>(&self, f: F) -> Result<T, StorageError>
    where F: for<'t> FnOnce(&Self::Write<'t>) -> Result<T, StorageError> + Send + 'static, T: Send + 'static;
}
```

Lo transversal, que es lo que un repositorio por entidad no puede expresar, queda como una función que pide dos capacidades dentro de una misma unidad de trabajo.

## Lo que el contrato tiene que decir

Un adaptador no se valida porque compile. Lo que hoy se da por hecho (verificado contra el código):

| Garantía | Dónde se apoya hoy |
|---|---|
| Las escrituras se serializan: dedup y `topic_key` no tienen restricción única | `crates/clawg-core/src/memory/store_insert.rs:356-369`, `crates/clawg-core/migrations/V6__content_hash.sql:9-13` |
| Una transacción se puede abandonar: una pasada de prueba corre y se suelta | `crates/clawg-core/src/memory/relocate.rs:590-595`, `merge.rs:399-401` |
| Memoria, vector, texto y progreso del outbox entran juntos o no entra ninguno | `store_insert.rs:424-426`, `outbox.rs:478-508` |
| Adoptar un proyecto al guardar es automático | Triggers `crates/clawg-core/migrations/V29__project_delete.sql:55-85` |
| Índice de texto sincronizado | Triggers `V8__memory_fts.sql:63`, `:73`, `:84` |
| Búsqueda vectorial filtrada | `vec0`, `search.rs:245-252` |
| Una copia coherente con el sistema andando | `VACUUM INTO`, `db.rs:298-331` |
| Abrir sin migrar, inspeccionar con el daemon parado, reescribir el archivo | `backup.rs` (`open_for_probe`), `secrets.rs:475` (`open_to_rewrite`) |
| Un solo dueño del directorio de datos | `store_lock`, `db.rs:200` |

El contrato se escribe como una batería de tests que corre contra cada adaptador, por ejemplo una `macro_rules!` que recibe el constructor. Esa batería es lo que permite «ir probando adaptadores». Probablemente dé más valor que los traits.

## Lo que probablemente no se pueda portar

- `clawg restore`, `purge` y los snapshots asumen que la base es un archivo. Un servidor necesita otro camino: `Snapshots` tendría que ser una capacidad, y el comando cambiaría de significado.
- Las migraciones (31 archivos con sintaxis de SQLite, `refinery`) son del adaptador. Otra base tiene las suyas, y llevar datos de una a otra es trabajo aparte.
- Unas 620 apariciones de `.conn` en tests comprueban filas con SQL de SQLite. Quedan como tests del adaptador SQLite: no hay que reescribirlas todas, pero no prueban el contrato.
- El carácter local del producto. Un servidor cambia la instalación, el arranque y `doctor`. Un candidato embebido cambia mucho menos que uno con servidor, y eso pesa en la decisión de la puerta.

## Enfoques posibles para el despacho

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Tipo concreto en la composición (`type ActiveStore = SqliteStore`), y los casos de uso genéricos sobre capacidades** | Los genéricos no llegan a `AppCtx` ni a los handlers. El candidato se compila sólo en `clawg-eval` | Cambiar de base en producción es recompilar. Para un binario local, eso alcanza |
| B. `MemoryRuntime<S: Store>` | Todo verificado por tipos | El parámetro se contagia a `clawg-cli` entero, y más código monomorfizado |
| C. `dyn Store` con clausuras en caja | Elegir base al arrancar | `read` y `write` genéricos no admiten `dyn`, así que hay que encajar la clausura y borrar el tipo del resultado. Una complejidad que sólo se justifica si hace falta elegir al correr |
| D. Un `enum` de adaptadores | Sin `dyn` | Cada método hace `match`. Sirve con dos adaptadores y envejece mal |

**Recomendado: A.** Elegir la base al arrancar no es un requisito de nadie.

## Descartado

- **Un repositorio por tabla o por conexión.** No expresa transacciones que cruzan tablas, como `insert_noting` (`store_insert.rs:427-446`), y una consulta de lectura une tablas.
- **Un único trait `Store` con todos los métodos.** Es el trait gigante.
- **Un ORM o un constructor de consultas.** Es una dependencia que no resuelve `vec0` ni FTS5, y aleja el SQL de los planes que el proyecto cuida.
- **SQL portable.** `vec0`, FTS5, `json_each`, `ON CONFLICT` y los triggers son justamente lo que se usa. Rebajarlo perdería conducta medida.

## Alcance candidato

1. Con la puerta abierta: extraer los traits de capacidad de los módulos de lectura y de los agregados, sin cambiar conducta.
2. `Store` y `SqliteStore`.
3. La batería de contrato, corriendo sobre `SqliteStore`.
4. El adaptador candidato, sólo en `clawg-eval`, hasta que una medición justifique otra cosa.

### No-alcance

- Adoptar otra base en producción: otro cambio, con migración de datos y con la regla de `config.yaml` sobre la base del usuario.
- Elegir la base al arrancar.

## Decisiones abiertas

1. **Qué base se prueba, y qué problema tiene que resolver.** Es la puerta del cambio.
2. Si `Snapshots` y `Migrations` son capacidades del mismo `Store` o van aparte.
3. Si los triggers de `V29` se mueven a Rust en storage-write-side, lo que le ahorra esa regla a cada adaptador.

## Cómo se mide

- **Test en rojo:** la batería de contrato, antes de que exista el adaptador candidato.
- La suite y `rag-eval --diff` no cambian con `SqliteStore` detrás de los traits.
- `perf-bench` sin regresión.
- Con un candidato, el umbral fijado en la puerta.

## Interacciones

- **storage-read-side y storage-write-side:** este cambio extrae de ellos, sin rediseñar.
- **store-boundary:** su `StorageError` es el error de estos traits.
- **search-index-ports:** puede resolver el caso de uso sin llegar a este cambio.
