# store-boundary — Tareas

## Cómo se trabaja cada tarea

- **TDD estricto.** Los tests de cada tarea van primero y fallan, por la aserción o porque no compilan. Las tareas de código empiezan por los tests en rojo de [«Cómo se mide»](proposal.md#cómo-se-mide).
- **Caracterización.** Un test de caracterización (R6, y el `detail` de R3) se escribe antes de tocar el código y pasa sobre el código de ese momento ([D11](design.md#d11-cómo-se-comprueba-que-nada-de-lo-que-ve-un-cliente-cambia)).
- **Commits.** Cada tarea es un commit convencional que compila y pasa los tests de los archivos que cambia. Esos tests se nombran al final de la tarea. La suite entera y los dos clippy corren una vez, sobre el último commit (tarea 9).
- **Windows.** Cada `cargo` lleva `--target-dir target-test`.

**Antes de la tarea 2:**

- El código parte de `develop` después de que entre el otro cambio que toca `crates/clawg-cli/src/daemon/api.rs`. Las funciones que nombran estas tareas (`locate_project`, `import_memories`, `import_as_job`, `undo_import`) se buscan por nombre.
- Es una versión nueva, así que antes de probar se limpia el directorio de build (`cargo clean --workspace`, o con `--target-dir target-test` en Windows).

**Por qué hay dos commits grandes:**

- La tarea 2 es un commit: sacar `Sqlite` y `Backend` de `MemoryError` obliga a tocar las 30 traducciones a mano en el mismo commit, y esas 30 son las que reemplaza `failed_call`.
- La tarea 3 también: su regla (R1) tiene que nacer en rojo, y lo que la pone en verde (D3, D4) y lo que la vuelve una garantía del compilador (D1) van con ella.

## 1. Artefactos

- [x] `proposal.md`, `spec.md`, `design.md`, `tasks.md` y `state.yaml`.
- Commit: `docs(openspec): propose store-boundary`

## 2. `StorageError` y una sola traducción (R5, R6, R7)

- [x] **Caracterización**, sobre el código de `develop` y con los literales que da:
  - `each_storage_failure_keeps_its_text` (`memory/error.rs`);
  - `a_failed_migration_keeps_its_text` (`memory/db.rs`);
  - `a_snapshot_in_a_transaction_says_why_it_failed` (`memory/backup.rs`);
  - `a_store_error_of_the_docs_index_keeps_its_text` (`memory/project_docs/mod.rs`);
  - `a_sqlite_error_of_an_export_keeps_its_text` (`memory/portability/export.rs`).

  Pasan antes de seguir.
- [x] **Tests en rojo:**
  - `a_call_on_a_closed_store_fails_with_a_storage_error` y `memory_error_tells_apart_what_another_crate_handles` (`crates/clawg-core/tests/public_api.rs`);
  - `no_memory_code_translates_a_connection_error_by_hand` (`crates/clawg-cli/tests/conventions.rs`), que nombra las 47 líneas de [D2](design.md#d2-las-reglas-de-conventionsrs).
- [x] `StorageError` y su `Cause` en `memory/error.rs`, reexportado en `memory::`. `MemoryError::Storage` lleva el encabezado de cada causa. Se van `Migration`, `Connection`, `Sqlite`, `Backend`, `RowDecode` y `VecExtensionRegistration` ([D5](design.md#d5-storageerror)).
- [x] Los `From` hacia `StorageError` y `MemoryError` que el código usa con `?`. `ExportError` y `DocIndexError` reciben los suyos a mano.
- [x] `BackupError::Snapshot` lleva su causa como `StorageError`, `DocIndexError::Store` lleva un `StorageError` y `ExportError::Sqlite` pasa a `ExportError::Storage`.
- [x] `failed_call`, `ClosedAs` y `failed_migration` en `error.rs` ([D6](design.md#d6-una-sola-traducción)):
  - las 15 de la primera redacción pasan a `ClosedAs::During`, las 5 de la segunda a `Named` y las 9 de la tercera a `AsIs`;
  - `run_migrations` pasa a `failed_migration`;
  - los 21 `map_err(MemoryError::Connection)` pasan a `map_err(MemoryError::from)`;
  - los `MemoryError::Sqlite(…)` que quedan pasan a `MemoryError::from(…)`.
- [x] Los tests de `error.rs` que nombran una variante que se va: `from_rusqlite_error` y `backend_error_displays_message` pasan a `Storage`, y `row_decode_displays_table_name` se borra con su variante.
- Commit: `refactor(core): give storage failures one opaque error and one translation`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-core --lib
  ```

  ```bash
  cargo test -p clawg-core --features testing --test public_api
  ```

  ```bash
  cargo test -p clawg-cli --test conventions
  ```

  ```bash
  cargo check --workspace --all-targets
  ```

  La caracterización tiene que pasar sin tocarla.

## 3. La conexión queda en el núcleo (R1, R2, R3, R4)

- [x] **Caracterización:** en `a_folder_shared_by_its_project_is_not_located` (`crates/clawg-cli/tests/api.rs`), la aserción del `detail` entero, con el número de carpetas. Pasa antes de seguir.
- [x] **Tests en rojo:**
  - `no_production_code_outside_core_reaches_the_connection` y `a_test_module_in_another_file_does_not_end_the_production_code` (`crates/clawg-cli/tests/conventions.rs`);
  - los dos doctests de `MemoryStore` (`memory/db.rs`, [D1](design.md#d1-la-conexión));
  - `own_folder_count_counts_the_folders_that_lead_to_a_project` (`memory/registry.rs`);
  - `a_runtime_closes_and_says_so` (`crates/clawg-core/tests/public_api.rs`).
- [x] `production_lines_naming` corta en `#[cfg(test)]` seguido de `mod <nombre> {` y recibe un predicado ([D2](design.md#d2-las-reglas-de-conventionsrs)). `no_production_code_names_the_retired_tables` sigue pasando.
- [x] `registry::own_folder_count` y su uso en `locate_project` ([D3](design.md#d3-las-carpetas-de-un-proyecto)).
- [x] `MemoryRuntime::close` devuelve `Result<(), StorageError>` ([D4](design.md#d4-close-dice-si-cerró)):
  - la línea `warn` pasa a `serve.rs`;
  - `release`, las tres llamadas de `revision-eval` y las dos de `conflict-eval` cierran con `close`;
  - los caminos de error de `runner.rs` siguen devolviendo su error;
  - los tests que cierran un runtime llevan `.expect`.
- [x] `conn` pasa a `pub(crate)` y se agrega `conn_for_test()`. Los 47 `.conn` de los tests de otros crates ([D1](design.md#d1-la-conexión)) pasan a `conn_for_test()`, salvo los que cierran un runtime, que pasan a `close` (`rag-eval/main.rs:1768`).
- [x] `clawg-eval`: `[dev-dependencies]` con `clawg-core` y las features `eval` y `testing`.
- Commit: `refactor: keep the store's connection inside clawg-core`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-cli --test conventions
  ```

  ```bash
  cargo test -p clawg-core --doc
  ```

  ```bash
  cargo test -p clawg-core --lib
  ```

  ```bash
  cargo test -p clawg-core --features testing --test public_api --test shutdown
  ```

  ```bash
  cargo test -p clawg-cli --lib
  ```

  ```bash
  cargo test -p clawg-cli --test api
  ```

  ```bash
  cargo test -p clawg-eval
  ```

## 4. Los imports en curso, en el runtime (R8)

- [x] **Tests en rojo**, que pasan a usar `RunningImports` y hasta el cambio no compilan:
  - `an_unknown_or_running_import_is_refused` (`portability/undo.rs`);
  - `an_import_is_numbered_and_journaled` y `a_stop_between_write_batches_returns_the_committed_ledger` (`portability/import.rs`);
  - `importing_again_finishes_a_stopped_job` (`crates/clawg-cli/src/daemon/api.rs`).
- [x] `RunningImports` en `memory/portability`, con `hold`, `release` y `holds`, y `MemoryRuntime::imports` ([D7](design.md#d7-los-imports-en-curso)).
- [x] `import_parsed` y `undo_import` lo reciben, y el `Journal` guarda un clon. `MemoryStore::importing` se va.
- [x] En el daemon, `import_memories`, `import_as_job` y el handler `undo_import` pasan `&app.runtime.imports`.
- Commit: `refactor(core): keep the imports in progress in the runtime, not the store`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-core --lib
  ```

  ```bash
  cargo test -p clawg-core --features testing --test public_api --test shutdown
  ```

  ```bash
  cargo test -p clawg-cli --lib daemon::api
  ```

  ```bash
  cargo test -p clawg-cli --test api
  ```

## 5. `sharing.json`, en el worker (R8)

- [x] **Test en rojo:** `a_key_leaves_a_hidden_authors_holder` (`memory/worker.rs`) le pasa el directorio al worker en lugar de usar `with_sharing_dir`. Hasta el cambio no compila.
- [x] `sharing::reader_from` ([D8](design.md#d8-sharingjson-sale-del-store)).
- [x] `memory_worker` y `process_tracked` reciben el directorio, e `ingest_worker_future` les pasa `base_dir`.
- [x] `EvalIngest` pasa su directorio, y `EvalStore` guarda el directorio del que abrió.
- [x] `MemoryRuntime::new_for_test_with_capacity` deja de llamar a `with_sharing_dir`.
- [x] Se van `sharing_dir`, `reader_for` y `with_sharing_dir` de `MemoryStore`.
- Commit: `refactor(core): read sharing.json in the ingest worker, not the store`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-core --features eval,testing --lib
  ```

  ```bash
  cargo test -p clawg-cli --test api
  ```

  ```bash
  cargo test -p clawg-eval
  ```

## 6. Los módulos que nadie nombra (R9)

- [x] **Test en rojo:** `every_public_memory_module_is_named_outside_core` (`crates/clawg-cli/tests/conventions.rs`). Hoy nombra los 9 de [D10](design.md#d10-la-superficie-de-memory).
- [x] `access`, `conn_probe`, `error`, `maintenance`, `migrations`, `model_download`, `query_context`, `record` y `search` pasan a `pub(crate) mod`. Los `pub use` de la raíz quedan.
- [x] Lo que `dead_code` señale en esos módulos se borra en un commit anterior a este, `refactor(core): remove <ítem>, which nothing calls`, así ningún commit deja un warning. Lo que sólo usan tests o `eval_api` pasa a su `cfg`, según los pasos 2 y 3 de [D10](design.md#d10-la-superficie-de-memory).
- Commit: `refactor(core): keep the memory modules no other crate names inside clawg-core`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-cli --test conventions
  ```

  ```bash
  cargo check --workspace --all-targets
  ```

  Además, los tests de los archivos de cada borrado.

## 7. Los 35 módulos que otros crates nombran (R9)

Esta tarea se repite por módulo, o por grupo de módulos chicos, en el orden de [D10](design.md#d10-la-superficie-de-memory): de `clock`, `vector`, `redact`, `events`, … a `portability` y `registry`.

No tiene un test en rojo propio: su rojo es el del compilador. El paso 1 rompe la compilación de cada otro crate que usa el módulo, y los pasos 2 y 3 la arreglan. Después, `dead_code` y `private_interfaces` señalan lo que sobra, y los dos clippy de la tarea 9 lo comprueban. La regla de la tarea 6 cubre los módulos.

- [x] Por cada módulo, los pasos 1 a 3 de D10: todo a `pub(crate)`; de vuelta a `pub` lo que usa otro crate; a su `cfg` lo que sólo usan tests o `eval_api`.
- [x] Lo que queda sin uso se borra en un commit anterior al cierre del módulo: `refactor(core): remove <ítem>, which nothing calls`. Si algo dice por qué tiene que quedar, se queda con ese comentario.
- [x] Por módulo se anota cuántos ítems quedaron `pub`, cuántos pasaron a cada `cfg` y cuántos se borraron. Va a `verify.md` en la tarea 9.
- Commit por módulo: `refactor(core): narrow what <módulo> shows to the other crates`
- Tests al cerrar cada uno:

  ```bash
  cargo test -p clawg-core --lib memory::<módulo>
  ```

  ```bash
  cargo check --workspace --all-targets
  ```

  ```bash
  cargo check -p clawg-cli
  ```

  El último construye el núcleo sin `testing` ni `eval`, como el release, y no puede dejar un warning. Además, los tests de los archivos donde un ítem pasó a un `cfg` o se borró.

## 8. Docs

- [x] `docs/reference/commands.md`: los imports en curso guardan su número en `MemoryRuntime::imports`, no en `MemoryStore::importing`.
- [x] `docs/reference/data-profiles.md`: el worker lee `sharing.json` en cada save con `sharing::reader_from`, no con `MemoryStore::reader_for`.
- Commit: `docs: say where the imports in progress and the sharing policy are kept`
- Tests al cerrar: ninguno; ningún test lee `docs/reference/`.

## 9. Verificación

- [x] Sobre el último commit de código, en Linux:

  ```bash
  cargo test --workspace
  ```

  ```bash
  cargo clippy --workspace --all-targets -- -D warnings
  ```

  ```bash
  cargo clippy -p clawg-cli -- -D warnings
  ```

- [ ] El CI del PR, que corre también Windows.
- [x] Las comprobaciones de [D11](design.md#d11-cómo-se-comprueba-que-nada-de-lo-que-ve-un-cliente-cambia):
  - el `git diff --word-diff` de los tests, en el que una línea de `assert` sólo cambia en las formas permitidas;
  - el diff vacío de los dos contratos del agente;
  - el diff de `api.rs` sin ningún `ApiError` ni `StatusCode` tocado.
- [x] `verify.md`: la suite y los dos clippy, las comprobaciones de D11, y el conteo por módulo de la tarea 7 con el total de lo borrado.
- Commit: `docs(openspec): record store-boundary applied`

El archivo (specs principales, `openspec/README.md`, `openspec/ROADMAP.md`) y la subida de versión no son parte de estas tareas.
