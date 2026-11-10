# store-boundary — Diseño

Los enfoques de fondo (A en la conexión, en el error y en la superficie) están en la [exploración](exploration.md#enfoques-posibles). Acá van sus tres decisiones abiertas (D5, D7 y D9), el punto 5 del alcance candidato (D8) y las decisiones de implementación.

`crates/clawg-cli/src/daemon/api.rs` se cita por función y no por línea, porque otro cambio que lo toca entra antes a `develop`. El resto, con `ruta:línea` de `develop` al escribir esto.

## D1. La conexión

`conn` pasa a `pub(crate)` (`crates/clawg-core/src/memory/db.rs:157`). Las 693 apariciones de `.conn` en `crates/clawg-core/src` siguen iguales. Los tests de otro crate la piden con:

```rust
#[cfg(any(test, feature = "testing"))]
pub fn conn_for_test(&self) -> &Connection
```

**Qué cambia en los tests de otros crates:**

| Lugar | `.conn` hoy |
|---|---|
| `crates/clawg-cli/tests/api.rs` | 26 |
| tests de `crates/clawg-cli/src/mcp/server.rs` | 10 |
| tests de `crates/clawg-cli/src/daemon/api.rs` | 3 |
| tests de `crates/clawg-cli/src/daemon/store_probe.rs` | 1 |
| `crates/clawg-core/tests/public_api.rs` | 3 |
| `crates/clawg-core/tests/shutdown.rs` | 2 |
| tests de `crates/clawg-eval/src/bin/rag-eval/main.rs` | 2 |

Cada `.conn` pasa a `.conn_for_test()`, con dos detalles:

- Un `.conn.clone().close()` que cierra un runtime pasa a `close()` ([D4](#d4-close-dice-si-cerró)). Es el caso de `rag-eval/main.rs:1768`.
- El otro test de `rag-eval` (`background_with`, `main.rs:1749`) escribe filas con SQL. `clawg-eval` no tiene `[dev-dependencies]`, y con `cargo test -p clawg-eval` no recibe `testing`. Por eso se le agrega `clawg-core = { workspace = true, features = ["eval", "testing"] }` en `[dev-dependencies]`, que es lo único que `only_dev_dependencies_turn_on_the_test_stand_ins` permite.

**Producción.** Los dos usos de producción fuera de core se van: el de `locate_project` ([D3](#d3-las-carpetas-de-un-proyecto)) y los cierres de `clawg-eval` ([D4](#d4-close-dice-si-cerró)).

**El test en rojo** es un par de doctests en la documentación de `MemoryStore`. Los doctests compilan como otro crate:

```rust
/// ```
/// fn reach(store: &clawg_core::memory::MemoryStore) { let _ = store.probe_wait(); }
/// ```
///
/// ```compile_fail,E0616
/// fn reach(store: &clawg_core::memory::MemoryStore) { let _ = &store.conn; }
/// ```
```

El segundo hoy compila, así que falla. Es el único test que compila como otro crate sin agregar una dependencia:

- Un test de `crates/clawg-core/tests/` no puede afirmar que algo no compila.
- `trybuild` lo haría, pero es una dependencia nueva para un test.

Un `rustdoc` estable no comprueba el código de error de un `compile_fail`; sólo uno nightly lo hace. Por eso el primer doctest, que compila y difiere del segundo sólo en lo que lee (`probe_wait`, que usa `crates/clawg-cli/src/daemon/store_probe.rs:40`), prueba que la ruta y la firma resuelven. Así, lo único que puede romper el segundo es leer `conn`. El `E0616` (campo privado) queda escrito para quien lo corra con nightly. Los doctests de `clawg-core` ya corren con `cargo test --workspace`, aunque hoy no hay ninguno.

**Descartado:**

- **B** de la exploración: dejarla `pub` y prohibirla con un test de convención.
- **C**: pasar todos los tests a la API pública.
- Un `MemoryStore::call` público que reciba una clausura sobre `rusqlite::Connection`. Devolvería `rusqlite` a la firma pública.

## D2. Las reglas de `conventions.rs`

Las tres reglas nuevas (R1, R7, R9) se escriben con `production_lines_naming` y `rust_files`, que ya están en `crates/clawg-cli/tests/conventions.rs:116` y `:26`.

**El corte del código de producción.** Hoy `production_lines_naming` termina el código de producción en el primer `#[cfg(test)]` seguido de una línea que empieza con `mod ` (`conventions.rs:120-123`). Eso incluye `mod test_temp;`, que declara un módulo en otro archivo, así que hoy se saltea casi todo `crates/clawg-eval/src/bin/rag-eval/main.rs` (corta en `:15-16`) y `crates/clawg-cli/src/update/mod.rs` (corta en `:13-14`). Con el cambio, la segunda línea tiene que ser `mod <nombre> {`.

`no_production_code_names_the_retired_tables` lee entonces más líneas, y ninguna nombra las tablas retiradas. Los archivos que son un módulo de tests entero (`test_temp.rs`, `update/testkit.rs`, `memory/crowding.rs`, `test_dir.rs`) se leen como producción, y hoy ninguno nombra lo que buscan las reglas nuevas. El helper pasa a recibir un predicado en lugar de una lista de nombres.

**Las tres reglas:**

| Test | Lee | Busca |
|---|---|---|
| `no_production_code_outside_core_reaches_the_connection` (R1) | `crates/clawg-cli/src`, `crates/clawg-eval/src` | `rusqlite`, o `.conn` seguido de fin de línea o de un carácter que no sea `[A-Za-z0-9_]`. Así no cuentan `.connect_timeout` (`crates/clawg-cli/src/update/client.rs:63`) ni `.conn_for_test()` |
| `no_memory_code_translates_a_connection_error_by_hand` (R7) | `crates/clawg-core/src`, menos `memory/error.rs` | `tokio_rusqlite::Error::`. Con `::` al final, así `eval_api.rs`, que anota el tipo (`\|e: tokio_rusqlite::Error\|`), no cuenta. Hoy da 47 líneas |
| `every_public_memory_module_is_named_outside_core` (R9) | los `pub mod <m>;` de `memory/mod.rs` contra `crates/clawg-cli/{src,tests}`, `crates/clawg-eval/src` y `crates/clawg-core/tests` | un módulo cuenta como nombrado con `clawg_core::memory::<m>` seguido de `::`, `;`, `,` o `}`, o con un grupo `use clawg_core::memory::{…}` en el que una ruta empieza con `<m>` |

## D3. Las carpetas de un proyecto

`locate_project` cuenta hoy las carpetas del proyecto propio de la raíz con SQL crudo sobre `store.conn`. Es el mismo `WHERE` que `own_folders_dated_in` (`crates/clawg-core/src/memory/registry.rs:826-833`), el helper de `own_folders_in` (`:822-824`). La consulta pasa a `registry.rs`, junto a ellos:

```rust
pub async fn own_folder_count(store: &MemoryStore, project: &str) -> Result<usize, StorageError>
```

Usa `own_folders_in(c, project)?.len()`, así que no se agrega SQL. Un proyecto tiene pocas carpetas y la cuenta en Rust no cuesta nada.

El error es `StorageError` y no `MemoryError` porque es la única falla posible, y porque así el 500 de una consulta fallida lleva el texto de hoy. Hoy `locate_project` pasa a `ApiError::internal` el `tokio_rusqlite::Error` tal cual, y `StorageError` muestra la causa como la da `tokio-rusqlite` ([D5](#d5-storageerror)).

`locate_project` sigue decidiendo: con más de una carpeta responde el 409 `folder_shared`, con el mismo `detail`. Bajar esa decisión al núcleo es de core-use-cases. `usize` y el `i64` de hoy se escriben con los mismos dígitos en el `detail`.

## D4. `close` dice si cerró

```rust
pub async fn close(&self) -> Result<(), StorageError>
```

`tokio-rusqlite` falla al cerrar sólo si falla el `close` de SQLite, y da `Ok` si la conexión ya estaba cerrada (`Connection::close`, `tokio-rusqlite-0.7.0/src/lib.rs:347-364`). Por eso el error es `StorageError`. Su texto es el de `tokio_rusqlite::Error` de hoy, y así la línea del log y `RunError::Close` siguen iguales.

| Llamador | Hoy | Con el cambio |
|---|---|---|
| `clawg serve` (`crates/clawg-cli/src/daemon/serve.rs:327`) | `close` registra y traga el error (`crates/clawg-core/src/runtime.rs:356-358`) | La línea `closing memory.db failed: {e}; SQLite replays what is left on the next open` pasa a `serve.rs`, igual, sólo cuando `close` devuelve `Err`. El proceso sale como hoy |
| `release` (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:311`) y `revision-eval` (`crates/clawg-eval/src/bin/revision-eval/run.rs:490`, `:634`, `:799`) | `.store.conn.clone().close()` | `.close().await.map_err(\|e\| RunError::Close(e.to_string()))?` |
| `conflict-eval` (`crates/clawg-eval/src/bin/conflict-eval/main.rs:216`, `:234`) | traga el error | propaga la falla como el `String` de su error |
| Caminos de error de `runner.rs:178` y `:196` | | El error que se devuelve sigue siendo el de hoy (`OutsideTemp`, `Serve`). La falla del cierre se descarta con `let _ =` y un comentario que dice por qué: ahí se cierra sólo para que se pueda borrar el directorio |
| Los tests que cierran un runtime (`public_api.rs:131`, `:179`; `shutdown.rs:283`, `:305`, `:332`) | `close().await;` | `.expect(...)`. `tests_leaving_a_store_open` busca `.close().await`, y lo sigue encontrando |

`docs/reference/testing.md:9` y la spec de storage («El daemon cierra `memory.db` antes de salir») nombran `MemoryRuntime::close`. Sigue siendo cierto: la falla se registra y nada más, ahora desde el daemon.

## D5. `StorageError`

**Decisión abierta 1: qué pregunta permite. Ninguna.** Nadie mira hoy el código de SQLite de un `MemoryError`:

- `sqlite_error_code` y `ErrorCode::` aparecen sólo en `crates/clawg-core/src/metrics/store.rs:578-579`, sobre el `rusqlite::Error` propio de `metrics.db`.
- `SqliteFailure` aparece sólo donde se construye uno (`db.rs:486-489`).
- Fuera del núcleo se distinguen `StoreInUse`, `SchemaNewer` y `SchemaDiverged`, que no son de SQLite.
- Ninguna ruta trata aparte una conexión cerrada.
- La única ocupación que se trata hoy, la del checkpoint de `clawg restore`, se lee de la columna que devuelve `PRAGMA wal_checkpoint` (`crates/clawg-core/src/memory/backup.rs:452-454`), no de un código de error.

Un `is_busy` hoy no tendría llamador. Si read-connections da a las lecturas una conexión propia y aparece quién pregunte, ese cambio agrega la pregunta.

**La forma:**

```rust
/// A failure of the store's backend, shown as the backend gave it: the error
/// that wraps it puts the heading.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StorageError(Cause);

#[derive(Debug, thiserror::Error)]
enum Cause {
    #[error("{0}")] Sqlite(#[source] rusqlite::Error),
    #[error("{0}")] Connection(#[source] tokio_rusqlite::Error),
    #[error("{0}")] Migration(#[source] refinery::Error),
    #[error("{0}")] Channel(String),
}
```

`StorageError` muestra sólo la causa, y cada error que lo envuelve pone el encabezado que ya ponía:

| Envoltorio | Encabezado |
|---|---|
| `MemoryError::Storage` | por tipo de causa, el de la variante que reemplaza (`SQLite error: `, `Memory DB connection error: `, `Migration failed: `, `memory backend channel error: `), con `#[error("{}{}", heading(.0), .0)]` y una función privada de `error.rs` |
| `BackupError::Snapshot` (`crates/clawg-core/src/memory/backup.rs:202-207`) | `could not write the snapshot {path}: {source}` |
| `DocIndexError::Store` (`crates/clawg-core/src/memory/project_docs/mod.rs:70-71`) | `documentation index store error: {0}` |
| `ExportError::Storage`, antes `Sqlite` (`crates/clawg-core/src/memory/portability/export.rs:64-65`) | `sqlite error: {0}` |

Se descartó que `StorageError` mostrara el encabezado de `MemoryError`: los otros tres envoltorios habrían agregado `SQLite error: ` a su texto.

**Qué se va de `MemoryError`:**

- `Migration`, `Connection`, `Sqlite` y `Backend` (`crates/clawg-core/src/memory/error.rs:11`, `:14`, `:17`, `:42`) pasan a `Storage`.
- `RowDecode` y `VecExtensionRegistration` (`:20`, `:23-27`) se borran, porque nada los construye.
- `Internal` queda: no es de almacenamiento, y los tests de `restore.rs:64`, `secrets.rs:496` y `serve.rs:550` lo usan.

Se reexporta `StorageError` en `memory::`, junto a `MemoryError`, porque está en firmas públicas (`close`, `own_folder_count`).

**Las conversiones** son los `From` que el código ya usa con `?`:

- `rusqlite::Error`, `tokio_rusqlite::Error` y `refinery::Error` hacia `StorageError`;
- esos mismos hacia `MemoryError`;
- `rusqlite::Error` hacia `ExportError`, y `tokio_rusqlite::Error` hacia `DocIndexError`, escritos a mano.

Un `impl From` es visible desde otro crate, pero no deja leer el error de SQLite que lleva adentro. Además, otro crate no puede producir uno sin depender de `rusqlite`, y R1 se lo prohíbe en producción. La alternativa era una función de conversión del crate en cada `?` que hoy convierte. Toca cientos de líneas sin dar nada a otro crate, así que se descartó.

**Fuera:**

- `MetricsError` ([propuesta](proposal.md#fuera-de-alcance)).
- `MemoryStoreError` (`crates/clawg-core/src/memory/store_insert.rs:284-296`). Sólo lo nombra `store_insert.rs`, y con [D10](#d10-la-superficie-de-memory) queda `pub(crate)`: ningún otro crate lo alcanza.

**Los tests en rojo**, en `crates/clawg-core/tests/public_api.rs`, que mira el núcleo como otro crate:

- `a_call_on_a_closed_store_fails_with_a_storage_error`.
- `memory_error_tells_apart_what_another_crate_handles`: un `match` sin comodín que nombra las diez variantes que quedan (`Storage`, `Internal`, `WorkspaceIo`, `StoreInUse`, `SchemaNewer`, `SchemaDiverged`, `Backup`, `Purge`, `DataDir`, `StoreLockIo`). Compila sólo si son exactamente esas. Una variante nueva obliga a nombrarla ahí, y eso es lo que se busca: es el contrato con los otros crates.

## D6. Una sola traducción

Las 30 traducciones a mano están en 20 archivos de `memory`. Todas mandan `Error(e)` a lo mismo: el error de la clausura tal cual, o `MemoryError::Sqlite`. Para la conexión cerrada hay tres redacciones:

| Redacción | Texto | Sitios |
|---|---|---|
| Primera | `connection closed during <op>`, y cualquier otra falla como la da `tokio-rusqlite` | 15: `admin.rs` (6, `backend` en `:693` incluido), `tier.rs` (2), `dedup.rs`, `dispute.rs:413`, `maintenance.rs`, `prompt_builder.rs`, `search.rs`, `topic.rs`, `workspace.rs` |
| Segunda | `<op>: <causa>` | 5: `language.rs:71`, `dispute.rs:506`, y los helpers de `synthesis.rs:243`, `outbox.rs:215` y `pieces.rs:190` |
| Tercera | `<causa>` | 9: `merge.rs:366`, `project.rs:998`, `project_delete.rs:109`, `registry.rs:433`, `:560`, `:651`, `:926`, `relocate.rs:624`, `subfolder.rs:192` |

La trigésima, `run_migrations` (`db.rs:478-507`), traduce un error de refinery y no de SQLite.

Todas pasan a `memory/error.rs`:

```rust
pub(crate) enum ClosedAs { During(&'static str), Named(&'static str), AsIs }

pub(crate) fn failed_call<E, Out>(closed: ClosedAs) -> impl Fn(tokio_rusqlite::Error<E>) -> Out
where
    Out: From<E> + From<MemoryError>;

pub(crate) fn failed_migration(e: tokio_rusqlite::Error<refinery::Error>) -> MemoryError;
```

- **`failed_call`.** `Error(e)` da `Out::from(e)`. Una falla de la conexión da un `StorageError` de causa `Channel` con la redacción de `closed`. Todos los errores de dominio que hoy traducen a mano ya tienen `Store(#[from] MemoryError)` (`relocate.rs:133`, `registry.rs:494`, `:585`, `:781`, `project.rs:92`, `subfolder.rs:150`, …), así que no se agrega ningún `From`.
- **`failed_migration`.** Lleva el `match` de `run_migrations` como está, con sus textos.
- **Los 21 `map_err(MemoryError::Connection)`** pasan a `map_err(MemoryError::from)`, que da la causa `Connection`, como hoy.

Se descartó una sola redacción para todos. Cambiaría el texto de un 500 en un caso que existe: `tokio-rusqlite` corre cada clausura sin `catch_unwind` (`event_loop`, `tokio-rusqlite-0.7.0/src/lib.rs:412-415`). Un pánico dentro de un `call` termina el hilo, cada llamada siguiente es `ConnectionClosed`, y el daemon responde esas llamadas con `ApiError::internal`, que pone el texto en `error`. Con `ClosedAs`, la diferencia queda escrita en un solo lugar. El cambio que pueda cambiar esos textos los unifica tocando sólo `failed_call`.

## D7. Los imports en curso

**Decisión abierta 2: dónde va `importing`. En `MemoryRuntime`, como un tipo propio.**

**Hechos:**

- El conjunto lo escribe el `Journal` de la importación al tomar su número, dentro de la transacción de su primera escritura (`crates/clawg-core/src/memory/portability/import.rs:972-984`), y lo suelta al soltarse (`:1022-1028`).
- Lo lee `undo_import` antes de leer nada más (`crates/clawg-core/src/memory/portability/undo.rs:1208`).
- En producción, a las dos funciones las llama sólo el daemon, con `app.runtime.store`: `import_memories` e `import_as_job` llaman a `import_parsed`, y el handler `undo_import` a `undo_import`.

**La forma:**

```rust
/// The imports this process is writing, by number: an undo refuses them until
/// they end.
#[derive(Clone, Default)]
pub struct RunningImports(Arc<Mutex<HashSet<i64>>>);
```

- Va en `memory/portability` y se reexporta, porque está en la firma de `import_parsed` y `undo_import`.
- Sus métodos (`hold`, `release`, `holds`) son del crate y tratan una sola vez el lock envenenado, que hoy se trata tres veces (`import.rs:981`, `:1025`, `undo.rs:1208`).
- `MemoryRuntime` gana `pub imports: RunningImports`, que crean sus constructores.
- `import_parsed` y `undo_import` lo reciben después de `store`. El `Journal` guarda un clon.
- `import_parsed` pasa a tener diez parámetros, bajo el `#[expect(clippy::too_many_arguments)]` que ya tiene (`import.rs:2154`).

**Descartado:**

- **En `AppCtx`, junto a `ImportJobs`.** `AppCtx` se arma a mano en varios lugares (el `Harness` de `crates/clawg-cli/tests/api.rs`, los tests de `api.rs`, `crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:183`), y la regla dependería de que cada uno pase el mismo conjunto. Un runtime, en cambio, sale siempre de un constructor del núcleo, y una construcción con `..runtime` se lleva el mismo conjunto. Además, `ImportJobs` conoce su trabajo pero no el número del diario en el instante en que el import lo toma.
- **Dejarlo en el store.** El adaptador de storage-ports cargaría estado de un caso de uso.

## D8. `sharing.json` sale del store

**El punto 5 del alcance candidato se confirma.**

`reader_for` (`db.rs:298-303`) tiene dos lectores de producción:

- `process_tracked`, una vez por save, antes del dedup (`crates/clawg-core/src/memory/worker.rs:702-704`);
- `EvalStore::search` (`crates/clawg-core/src/eval_api.rs:363`).

`sharing_dir` vale el directorio de datos en `open` (`db.rs:260`) y `None` en el store en memoria (`db.rs:284`). `with_sharing_dir` (`db.rs:291-294`, `testing`) lo usan `runtime.rs:428` y un test del worker (`worker.rs:4727`).

**El cambio:**

- El cuerpo de `reader_for` pasa a `sharing::reader_from(dir: Option<&Path>, agent_id)`, junto a `reader_in` (`crates/clawg-core/src/memory/sharing.rs:85`).
- `memory_worker` recibe `sharing: Option<PathBuf>` y se lo pasa a `process_tracked`.
- `ingest_worker_future`, que ya recibe `base_dir` (`runtime.rs:485-486`), pasa `Some(base_dir)`. `EvalIngest` pasa su directorio, y `EvalStore` guarda el directorio del que abrió.
- Los tests que llaman a `memory_worker` pasan `None`, que es lo que da hoy un store en memoria.

Es el mismo archivo, leído en el mismo momento. Nada de esto sale de `clawg-core`.

**Descartado: dejarlo.** Ningún otro crate lo ve, pero el adaptador de storage-ports llevaría una política que no es de almacenamiento.

## D9. Los campos de `MemoryRuntime`

**Decisión abierta 3: siguen públicos.**

Siete construcciones desde otro crate usan `..runtime`:

- `crates/clawg-cli/tests/api.rs:167` (`with_empty_embedder`);
- un test de `crates/clawg-cli/src/daemon/api.rs` (`daemon_embedding_with`);
- `crates/clawg-cli/src/mcp/server.rs:3640`;
- `crates/clawg-core/tests/shutdown.rs:128`, `:151`, `:215`, `:227`.

Rust exige que todos los campos sean visibles para esa sintaxis (E0451). Cerrar uno solo rompe las siete, y reemplazarlas pediría un constructor de test por cada cambio (la celda del embedder, su estado, el canal).

En producción, el daemon pasa las celdas al bootstrap (`serve.rs:136`, `:164-165`, `:225`) y lee `embedder`, `tokenizer`, `access_tracker`, `project_docs` y `metrics`. Un getter devolvería el mismo `Arc`, sin ninguna regla que cuidar.

Con `conn` cerrada, `store` deja ver sólo los métodos de `MemoryStore`, así que el campo público no deja ver SQLite. `imports` ([D7](#d7-los-imports-en-curso)) también es público. El comentario de `runtime.rs:113-120` sigue siendo cierto.

## D10. La superficie de `memory`

El A de la exploración dice `pub(crate) mod` más un `pub use` de lo que usan los otros crates. Tal cual, no se puede sin cambiar rutas de los otros crates:

- **Una fachada plana en `memory::`** choca con nombres repetidos: `Candidate` (`contradiction.rs:77`, `project_delete.rs:165`, `registry.rs:744`), `Refusal` (`relocate.rs:31`, `project_delete.rs:118`, `registry.rs:726`), `ResolveError` (`dispute.rs:74`, `registry.rs:775`), `KeepReason` (`project_delete.rs:128`, `portability/undo.rs:39`) y `Resolution` (`dispute.rs:112`, `registry.rs:756`). Habría que renombrar en `clawg-cli` y `clawg-eval`.
- **Una fachada con el mismo nombre de módulo** (`pub(crate) mod registry;` y `pub mod registry { pub use … }`) no compila: los dos viven en el espacio de tipos (E0428).
- **Renombrar los módulos de implementación** es mover archivos, que es de memory-module-layout.

**Lo que se hace da al compilador lo mismo que A**, porque un ítem `pub(crate)` sin uso es `dead_code`, igual que un `pub` en un módulo `pub(crate)` sin reexportar:

- Los 9 módulos que ningún otro crate nombra (`access`, `conn_probe`, `error`, `maintenance`, `migrations`, `model_download`, `query_context`, `record`, `search`) pasan a `pub(crate) mod`. Los `pub use` de la raíz (`MemoryError`, `MemoryStore`, `search_all_memories`, …) siguen andando desde un módulo `pub(crate)`.
- Los otros 35 siguen `pub mod`, y cada ítem suyo que ningún otro crate usa pasa a `pub(crate)`.

Ninguna ruta de otro crate cambia. memory-module-layout puede mover un archivo y dejar su ruta con `pub use self::<carpeta>::<módulo>;`, que vale para un módulo `pub` dentro de una carpeta privada.

**Cada módulo, en este orden:**

1. Todo ítem `pub` del módulo pasa a `pub(crate)`. Los campos de un struct público quedan como están: cerrarlos cambia cómo otro crate construye y lee el tipo.
2. `cargo check --workspace --all-targets` dice qué usan los otros crates.
   - Lo que usa su producción vuelve a `pub`.
   - Lo que usan sólo sus tests vuelve a `pub` con `cfg(any(test, feature = "testing"))`, como `new_for_test`.
3. `cargo clippy -p clawg-cli -- -D warnings` construye el núcleo sin `testing` ni `eval`, como el release, y señala lo que sólo usan los tests del núcleo, que pasa a `cfg(test)`. También señala lo que sólo usa `eval_api`, que pasa a `cfg(feature = "eval")`, como ya lo hacen otros ítems del núcleo.
4. Lo que queda señalado no lo usa nadie y se borra, en un commit aparte del que cierra el módulo. El mensaje dice qué se borró. Si algo explica por qué un ítem tiene que quedar, ese motivo va en un comentario y el ítem se queda.

`MemoryStoreError` ([D5](#d5-storageerror)) y los demás tipos que sólo nombra su módulo quedan `pub(crate)` por el paso 1.

**El orden de los módulos:** primero los 9 sin nombre de afuera, en un commit. Después los 35, de a uno o en grupos de módulos chicos, empezando por los que menos dependen de otros (`clock`, `vector`, `redact`, `events`, …) y terminando por `portability` y `registry`. Si un módulo muestra más código muerto del esperado (el riesgo de la exploración), se borra por tandas.

`verify.md` registra, por módulo, cuántos ítems quedaron `pub`, cuántos pasaron a cada `cfg` y cuántos se borraron.

## D11. Cómo se comprueba que nada de lo que ve un cliente cambia

Un cliente ve tres cosas: las respuestas HTTP del daemon, lo que el puente MCP le da al agente y lo que imprime la línea de comandos. El puente es un cliente HTTP del daemon, así que ningún tipo del núcleo cruza hasta él salvo como texto de una respuesta. Lo que este cambio podría mover es el texto de un error y el estado de una respuesta. Se comprueba así:

1. **Caracterización primero.** La tarea que puede mover un texto empieza por fijarlo, antes de tocar el código:
   - la tarea 2, con los tests de R6: los textos de cada falla de almacenamiento, en las tres redacciones y en los cuatro envoltorios;
   - la tarea 3, con la aserción del `detail` entero de `folder_shared` (R3).

   Se escriben con los literales que da el código de ese momento y pasan sobre él. Después tienen que seguir pasando sin tocarlos.
2. **Los tests que ya existen no cambian lo que comprueban.** Las únicas ediciones permitidas son de forma:
   - `.conn` por `.conn_for_test()`;
   - un `.expect` sobre `close`;
   - el `&RunningImports` o el directorio de `sharing.json` como argumento;
   - en `error.rs`, `from_rusqlite_error` y `backend_error_displays_message` pasan a nombrar `Storage`, y `row_decode_displays_table_name` se va con su variante.

   Se revisa con:

   ```bash
   git diff --word-diff=porcelain develop...HEAD -- 'crates/*/tests/*.rs' 'crates/*/src/**/*.rs'
   ```

   En una línea de `assert` sólo pueden cambiar esas formas. `verify.md` lo deja escrito.
3. **El contrato del agente, sin bendecir.** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin `CLAWG_BLESS`, y este comando no muestra nada:

   ```bash
   git diff develop...HEAD -- crates/clawg-cli/src/mcp/agent_contract.json crates/clawg-cli/src/mcp/agent_contract.es.json
   ```

4. **El daemon.** En `api.rs` cambian sólo:
   - la consulta de `locate_project`;
   - un argumento en `import_memories`, `import_as_job` y `undo_import`;
   - los tests.

   Ningún `ApiError` ni `StatusCode` cambia. Se revisa en el diff de `api.rs`, y los tests de `crates/clawg-cli/tests/api.rs` comprueban estados y cuerpos.
5. **La línea de comandos.** Los mensajes de `restore`, `secrets`, `status` y `serve` dependen de `StoreInUse`, `SchemaNewer` y `SchemaDiverged`, que no cambian, y del texto de los demás errores, que fija R6. `crates/clawg-cli/tests/binary.rs` corre el binario de punta a punta sin cambiar lo que comprueba.
6. **Lo que no es de un cliente**, también igual. La línea de log de `clawg serve` al fallar el cierre, y `RunError::Close` de `rag-eval` y `revision-eval`, llevan el texto de `StorageError`, que es el de `tokio_rusqlite::Error` de hoy.
