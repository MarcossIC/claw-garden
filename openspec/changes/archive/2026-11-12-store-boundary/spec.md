# store-boundary — Especificación

Son requisitos sobre [`specs/storage`](../../../specs/storage/spec.md), [`specs/portability`](../../../specs/portability/spec.md) y [`specs/sharing`](../../../specs/sharing/spec.md), y se consolidan al archivar. Lo que no se nombra acá sigue como está.

Términos:

- **Otro crate:** `clawg-cli`, `clawg-eval` o un test de integración de `crates/*/tests/`.
- **Código de producción** de un archivo de `src/`: sus líneas antes del primer `#[cfg(test)]` seguido de una línea `mod <nombre> {`.
- **Una falla de almacenamiento:** un error de SQLite, de la conexión de `tokio-rusqlite` o de una migración de refinery.
- **Una falla de la conexión:** un `call` cuya clausura no corrió o no contestó, porque la conexión está cerrada o su hilo terminó.

## R1. El código de los otros crates no llega a la conexión

El código de producción de `crates/clawg-cli/src` y de `crates/clawg-eval/src` MUST NOT contener `rusqlite` ni `.conn` seguido de un carácter que no pueda seguir en un identificador. Así, `.connect_timeout` no cuenta. Un `#[cfg(test)]` seguido de `mod <nombre>;`, que declara un módulo en otro archivo, MUST NOT cortar el código de producción del archivo que lo declara.

#### Escenario: hoy
- GIVEN el código de `develop`
- WHEN corre la regla
- THEN nombra `crates/clawg-cli/src/daemon/api.rs` (`locate_project`), `crates/clawg-eval/src/bin/revision-eval/run.rs` (tres líneas) y `crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs` (`release`)

#### Escenario: un módulo de tests en otro archivo
- GIVEN un fuente con `#[cfg(test)]` y `mod helpers;` en sus primeras líneas, y una línea que nombra `rusqlite` más abajo, fuera de todo módulo de tests
- WHEN se separa su código de producción
- THEN esa línea es de producción, con fin de línea LF y con CRLF

**Tests:** `no_production_code_outside_core_reaches_the_connection` y `a_test_module_in_another_file_does_not_end_the_production_code` (`crates/clawg-cli/tests/conventions.rs`).

## R2. La conexión es del núcleo

`MemoryStore::conn` MUST ser `pub(crate)`. Desde otro crate, la conexión MUST alcanzarse sólo con `MemoryStore::conn_for_test()`, compilado sólo con `cfg(any(test, feature = "testing"))`. Sólo un `[dev-dependencies]` enciende `testing`, y `only_dev_dependencies_turn_on_the_test_stand_ins` lo comprueba.

#### Escenario
- GIVEN un crate que usa `clawg_core` sin `testing`
- WHEN lee `store.conn`, y en otra función igual llama a `store.probe_wait()`
- THEN la primera no compila (E0616, campo privado) y la segunda sí

**Test:** los dos doctests de `MemoryStore` (`crates/clawg-core/src/memory/db.rs`), uno normal y uno `compile_fail,E0616` ([D1](design.md#d1-la-conexión)). Hoy el segundo compila, así que falla. Los tests de otro crate que hoy leen `.conn` pasan a `conn_for_test()` sin cambiar lo que comprueban.

## R3. Las carpetas de un proyecto se cuentan en el núcleo

`registry::own_folder_count(store, project)` MUST devolver cuántas carpetas de `project_path` llevan a `project`: las que tienen su `project_id`, y una de antes de V27 cuyo `id` es el del proyecto. Es el mismo `WHERE` que hoy escribe `locate_project`. Su error MUST ser `StorageError`.

`locate_project` MUST decidir con ese número:

- el 409 `folder_shared` MUST llevar el mismo `detail`, con el número de carpetas;
- el 500 de una consulta que falla MUST llevar el mismo texto que hoy.

#### Escenario
- GIVEN el proyecto A con dos carpetas, el proyecto B cuya única carpeta es de antes de V27 (con el `id` de B y sin `project_id`), y el proyecto C sin carpetas
- WHEN se cuentan las carpetas de cada uno
- THEN A tiene 2, B 1 y C 0

**Tests:**
- `own_folder_count_counts_the_folders_that_lead_to_a_project` (`crates/clawg-core/src/memory/registry.rs`).
- `a_folder_shared_by_its_project_is_not_located` (`crates/clawg-cli/tests/api.rs`). Antes del cambio se le agrega la aserción del `detail` entero, que pasa sobre el código de hoy.

## R4. Cerrar dice si cerró

`MemoryRuntime::close` MUST devolver `Result<(), StorageError>`: `Ok` cuando la conexión cerró o ya estaba cerrada, y la falla de SQLite al cerrar cuando no.

- `clawg serve` MUST registrar esa falla con la misma línea `warn` de hoy, que dice que SQLite reproduce lo que quede al abrir, y seguir saliendo como hoy.
- `rag-eval` (`release`) y `revision-eval` MUST cerrar con `close` y terminar con `RunError::Close` cuando falla, con el mismo texto que hoy.
- `conflict-eval` MUST devolver esa falla como su error.
- Un camino que ya devuelve otro error MUST seguir devolviendo ese error.

#### Escenario
- GIVEN un runtime abierto en un directorio temporal
- WHEN se cierra dos veces
- THEN las dos veces es `Ok`, y una llamada al store después es `Err(MemoryError::Storage(_))`

**Tests:**
- `a_runtime_closes_and_says_so` (`crates/clawg-core/tests/public_api.rs`). Hoy no compila: `close` devuelve `()`.
- `closing_after_the_shutdown_leaves_no_wal_beside_the_store` (`crates/clawg-core/tests/shutdown.rs`), que sigue comprobando lo mismo.

## R5. Una sola variante de almacenamiento

Una falla de almacenamiento que llega a otro crate dentro de un `MemoryError` MUST ser `MemoryError::Storage(StorageError)`. Una función que sólo puede fallar por el almacenamiento (`close`, `own_folder_count`) MAY devolver el `StorageError` solo.

- `MemoryError` MUST NOT tener una variante con un tipo de `rusqlite`, `tokio_rusqlite` o `refinery`, ni directo ni dentro de un tipo que envuelva.
- Se van `Migration`, `Connection`, `Sqlite`, `Backend`, `RowDecode` y `VecExtensionRegistration`. Quedan las demás, `Internal` incluida.
- `StorageError` MUST ser opaco: sin campos públicos y sin métodos públicos más allá de `Display`, `Debug` y `std::error::Error`.
- Los otros errores de `memory` que otro crate alcanza y hoy llevan un tipo de esos crates MUST llevar un `StorageError`:
  - la causa (`source`) de `BackupError::Snapshot`;
  - `DocIndexError::Store`;
  - `ExportError::Sqlite`, que pasa a `ExportError::Storage`.

#### Escenario
- GIVEN un store cerrado
- WHEN otro crate le hace una llamada
- THEN recibe `MemoryError::Storage(_)`, y un `match` sin comodín sobre `MemoryError` nombra sólo las variantes que quedan

**Tests:** `a_call_on_a_closed_store_fails_with_a_storage_error` y `memory_error_tells_apart_what_another_crate_handles` (`crates/clawg-core/tests/public_api.rs`). Hoy ninguno compila.

## R6. Ningún texto cambia

Este requisito es de conservación. Sus tests se escriben primero y pasan sobre el código de `develop`. Lo que comprueban es que siguen pasando.

Cada falla MUST mostrarse con el texto que muestra hoy, en estos casos:

| Falla | Texto de hoy |
|---|---|
| Un error de SQLite como `MemoryError` | `SQLite error: <causa>` |
| Una falla de la conexión que hoy pasa por `MemoryError::Connection` (por ejemplo `probe_wait`) | `Memory DB connection error: <causa>` |
| Una conexión cerrada en `get_memory_core` (primera redacción) | `memory backend channel error: connection closed during get_memory_core` |
| Una conexión cerrada en `count_saves_in` (segunda redacción) | `memory backend channel error: language count: ConnectionClosed` |
| Una conexión cerrada en `rename_project` (tercera redacción) | `memory backend channel error: ConnectionClosed` |
| Una migración fallida | `Migration failed: <causa>`, o `SQLite error: connection closed during migration` con la conexión cerrada |
| `BackupError::Snapshot` | `could not write the snapshot <ruta>: <causa>` |
| `DocIndexError::Store` | `documentation index store error: <causa>` |
| `ExportError` de SQLite | `sqlite error: <causa>` |

La causa MUST ser la que da hoy `rusqlite`, `tokio-rusqlite` o refinery, sin un encabezado más.

**Tests:**
- `each_storage_failure_keeps_its_text` (`crates/clawg-core/src/memory/error.rs`), sobre un store cerrado y sobre errores construidos.
- `a_failed_migration_keeps_its_text` (`crates/clawg-core/src/memory/db.rs`): una migración rota, como la de `a_failing_migration_rolls_back_every_migration_of_its_run`, y `run_migrations` sobre una conexión cerrada.
- `a_snapshot_in_a_transaction_says_why_it_failed` (`crates/clawg-core/src/memory/backup.rs`).
- `a_store_error_of_the_docs_index_keeps_its_text` (`crates/clawg-core/src/memory/project_docs/mod.rs`).
- `a_sqlite_error_of_an_export_keeps_its_text` (`crates/clawg-core/src/memory/portability/export.rs`).

Cada uno con el literal que da `develop`.

## R7. Una sola traducción

Un `call` fallido sobre la conexión MUST traducirse con las conversiones de `memory/error.rs`: los `From` de los errores de los tres crates y `failed_call`. Una migración fallida se traduce con `failed_migration`. Ningún archivo de `crates/clawg-core/src` MUST nombrar una variante de `tokio_rusqlite::Error` (`tokio_rusqlite::Error::`) en su código de producción, salvo `memory/error.rs`.

#### Escenario: hoy
- GIVEN el código de `develop`
- WHEN corre la regla
- THEN nombra las líneas de las 30 traducciones a mano, en 20 archivos de `memory`

**Test:** `no_memory_code_translates_a_connection_error_by_hand` (`crates/clawg-cli/tests/conventions.rs`).

## R8. El store guarda sólo lo del almacenamiento

`MemoryStore` MUST NOT guardar los imports en curso ni de dónde se lee `sharing.json`.

- **Imports en curso.** Los guarda `MemoryRuntime::imports`, de tipo `RunningImports`. `import_parsed` y `undo_import` MUST recibirlo. Deshacer un import que este proceso está escribiendo MUST seguir siendo `UndoError::Running` (`409`), y su número MUST soltarse cuando el `Journal` se suelta, como hoy.
- **`sharing.json`.** El worker de ingest MUST leerlo del directorio de datos que le da el runtime, una vez por save, como hoy. Un worker sin directorio, como el de un test sobre un store en memoria, MUST compartir todo, como hoy un store sin directorio. `MemoryStore::with_sharing_dir` y `MemoryStore::reader_for` MUST NOT existir.

#### Escenario
- GIVEN un import que escribe en un store, con el `RunningImports` del runtime
- WHEN se pide deshacerlo antes de que termine
- THEN es `UndoError::Running`, y cuando termina se deshace

**Tests:** estos tests pasan a usar `RunningImports` o el directorio del worker, y hasta el cambio no compilan:
- `an_unknown_or_running_import_is_refused` (`undo.rs`);
- `an_import_is_numbered_and_journaled` y `a_stop_between_write_batches_returns_the_committed_ledger` (`import.rs`);
- `importing_again_finishes_a_stopped_job` (`crates/clawg-cli/src/daemon/api.rs`);
- `a_key_leaves_a_hidden_authors_holder` (`crates/clawg-core/src/memory/worker.rs`).

## R9. La superficie de `memory` es la que usan los otros crates

- Cada `pub mod` de `crates/clawg-core/src/memory/mod.rs` MUST estar nombrado por otro crate, en una ruta `clawg_core::memory::<módulo>` o en un grupo `clawg_core::memory::{…}`. Los demás MUST ser `pub(crate)`.
- Dentro de un módulo `pub`, un ítem MUST ser `pub` sólo si otro crate lo usa.
- Un ítem que sólo usan los tests de otro crate MUST compilarse sólo con `cfg(any(test, feature = "testing"))`.
- Un ítem que sólo usa `eval_api` MUST compilarse sólo con `cfg(feature = "eval")`, o con `cfg(any(test, feature = "eval"))` si también lo usan los tests del núcleo.
- Un ítem que sólo usan los tests del núcleo MUST compilarse sólo con `cfg(test)`.
- Lo que nada usa MUST borrarse.
- Ninguna ruta que hoy usa otro crate MUST cambiar.

Los dos clippy de `[workspace.lints.clippy]` MUST pasar con `-D warnings`:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo clippy -p clawg-cli -- -D warnings
```

El segundo es lo que construye el release, sin `testing` ni `eval`.

**Tests:** `every_public_memory_module_is_named_outside_core` (`crates/clawg-cli/tests/conventions.rs`). Hoy falla por `access`, `conn_probe`, `error`, `maintenance`, `migrations`, `model_download`, `query_context`, `record` y `search`. Además, los dos clippy y `no_source_allows_dead_code`.

## R10. Lo que ve un cliente no cambia

Ninguna respuesta del daemon (estado, cuerpo, encabezados), del puente MCP (texto de cada herramienta, esquema, descripción) ni de la línea de comandos (stdout, stderr, código de salida) MUST cambiar. Como R6, es de conservación: sus tests pasan antes y después. Cómo se comprueba está en [D11](design.md#d11-cómo-se-comprueba-que-nada-de-lo-que-ve-un-cliente-cambia).

**Tests:**
- La suite entera, sin cambiar lo que comprueba.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`crates/clawg-cli/src/mcp/server.rs`), sin `CLAWG_BLESS`.
- Los de R6.
