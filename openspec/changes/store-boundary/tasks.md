# store-boundary — Tareas

Strict TDD: cada tarea empieza por sus tests, que fallan por la aserción o porque no compilan. Cada tarea es un commit convencional que compila y pasa los tests que nombra; las de la fachada y el código muerto (6) son una serie de commits, cada uno igual de verde. La suite entera y los tres clippy corren una vez, sobre el último commit (7).

**Antes de la tarea 2:** el código empieza cuando ya está en `develop` el otro cambio que toca `crates/clawg-cli/src/daemon/api.rs`. Se parte de ese `develop`, se corre `cargo clean --workspace --target-dir target-test` y se vuelven a ubicar por nombre las funciones de `api.rs` que cita el diseño (`locate_project`, `list_memories`, `ApiError::internal`, `import_memories`, `import_as_job`, `undo_import` y los tests `count`, `notes_stored`, `daemon_embedding_with` y `an_abandoned_job_sends_nothing_to_the_connection`).

En Windows, cada `cargo test` y `cargo clippy` de abajo lleva `--target-dir target-test`.

## 1. Artefactos

- [x] `proposal.md`, `spec.md`, `design.md`, `tasks.md` y `state.yaml`.
- Commit: `docs(openspec): propose store-boundary`

## 2. Lo que mide el cambio, y la conexión fuera de `api.rs` y de `clawg-eval` (SB1, SB2, SB3, SB8)

- [ ] **Tests en rojo:**
  - `no_production_code_outside_the_core_reaches_sqlite` y el puro `the_production_code_runs_past_a_test_module_declared_without_a_body` (`crates/clawg-cli/tests/conventions.rs`), con `production_lines_naming` corregido como dice [D10](design.md#d10-las-reglas-en-conventionsrs);
  - `own_folders_are_counted_as_the_registry_reads_them` (`crates/clawg-core/src/memory/registry.rs`);
  - `a_close_that_fails_is_returned` (`crates/clawg-core/src/runtime.rs`).
- [ ] **Fijaciones de [D9](design.md#d9-cómo-se-comprueba-que-ninguna-respuesta-cambia), antes de tocar código de producción:** `a_storage_failure_reads_as_before` (`crates/clawg-core/src/memory/error.rs`), `a_failed_query_answers_with_the_sqlite_text` y `a_failed_lookup_answers_with_the_connection_text` (`crates/clawg-cli/tests/api.rs`). Se escriben con el texto que da hoy el código, se corren y pasan. De acá al final no se tocan, salvo `.conn` en la tarea 3.
- [ ] `own_folder_count` en `registry.rs`, y `locate_project` la usa ([D1](design.md#d1-lo-que-hoy-usa-la-conexión-desde-afuera)).
- [ ] `MemoryRuntime::close` devuelve `Result<(), MemoryError>`. El `if finished` de `serve.rs` y los tres llamadores de `eval_api.rs` registran el error con el texto de hoy. `release` (`runner.rs`) y los tres cierres de `revision-eval/run.rs` usan `runtime.close()` con el mismo `RunError::Close`. Los tests que cierran un runtime pasan a `.close().await.expect("the store closes")`.
- [ ] **Al cerrar, se corre:**
  ```bash
  cargo test -p clawg-cli --test conventions
  ```
  ```bash
  cargo test -p clawg-cli --test api
  ```
  ```bash
  cargo test -p clawg-cli --lib -- daemon::
  ```
  ```bash
  cargo test -p clawg-core --features eval,testing --lib --test public_api --test shutdown
  ```
  ```bash
  cargo test -p clawg-eval --bin rag-eval --bin revision-eval
  ```
- Commit: `refactor: count a project's folders in the core and return a failed close`

## 3. La conexión cerrada (SB4)

- [ ] **Tests en rojo:** los tests de afuera que leen o siembran filas pasan de `.conn` a `.raw_connection()`, que todavía no existe, y no compilan. Son los de [D5](design.md#d5-la-conexión-cerrada): `crates/clawg-cli/tests/api.rs` (las dos fijaciones incluidas), `crates/clawg-cli/src/mcp/server.rs`, `crates/clawg-cli/src/daemon/api.rs`, `crates/clawg-cli/src/daemon/store_probe.rs`, `crates/clawg-core/tests/public_api.rs`, `crates/clawg-core/tests/shutdown.rs` y `crates/clawg-eval/src/bin/rag-eval/main.rs`.
- [ ] `conn` pasa a `pub(crate)` y `raw_connection()` queda detrás de `cfg(any(test, feature = "testing"))`.
- [ ] `[dev-dependencies]` de `crates/clawg-eval/Cargo.toml` con `clawg-core = { workspace = true, features = ["eval", "testing"] }`. `Cargo.lock` no cambia, porque `clawg-core` ya es dependencia de `clawg-eval`.
- [ ] La comprobación de `vec_version()` de `memory_runtime_open_creates_the_db_and_returns_a_usable_store` pasa a un bloque `#[cfg(feature = "testing")]`.
- [ ] `docs/reference/testing.md`: un test de otro crate lee filas con `MemoryStore::raw_connection()`, que sólo existe con `testing`.
- [ ] **Al cerrar, se corre:**
  ```bash
  cargo test -p clawg-cli --lib --test api --test conventions
  ```
  ```bash
  cargo test -p clawg-core --features testing --test public_api --test shutdown
  ```
  ```bash
  cargo test -p clawg-eval --bin rag-eval
  ```
- Commit: `refactor(core): keep the store's connection inside the core`

## 4. `StorageError` y una sola traducción (SB5, SB6)

- [ ] **Tests en rojo:** `a_storage_failure_is_one_opaque_variant` y `a_call_on_a_closed_connection_names_what_it_was_doing` (`error.rs`); `only_the_storage_error_matches_the_connection_errors` (`conventions.rs`).
- [ ] `StorageError`, `Fault`, `MemoryError::Storage`, `call_failed` e `into_sqlite`, como dicen [D2](design.md#d2-storageerror-no-responde-ninguna-pregunta) y [D3](design.md#d3-una-sola-traducción-de-los-errores-de-la-conexión).
- [ ] Cada traducción a mano de `crates/clawg-core/src` pasa a `call_failed`: los unos 20 archivos, más `run_migrations` y `Retried::connection`. `plan_ids_in` pasa a `into_sqlite`.
- [ ] Salen `VecExtensionRegistration`, `RowDecode` y `row_decode_displays_table_name`. `from_rusqlite_error` y `backend_error_displays_message` (`error.rs`) comprueban lo mismo con la forma nueva.
- [ ] **Al cerrar, se corre:**
  ```bash
  cargo test -p clawg-core --features eval,testing --lib --test public_api --test shutdown
  ```
  ```bash
  cargo test -p clawg-cli --test conventions --test api
  ```
  ```bash
  cargo test -p clawg-cli --lib -- secrets restore daemon::serve
  ```
- Commit: `refactor(core): report storage failures as one opaque error`

## 5. Las importaciones en curso (SB7)

- [ ] **Test en rojo:** `an_import_holds_its_number_until_it_ends` (`crates/clawg-core/src/memory/portability/import.rs`).
- [ ] `ImportsRunning` en `import.rs`. `MemoryStore` guarda un `pub(crate) imports: ImportsRunning`. `Journal` usa `start` y `end`, y `undo_import` usa `holds` ([D4](design.md#d4-las-importaciones-en-curso)).
- [ ] `an_import_is_numbered_and_journaled`, `a_stop_between_write_batches_returns_the_committed_ledger` y `an_unknown_or_running_import_is_refused` cambian el lock por los métodos del tipo, y comprueban lo mismo.
- [ ] **Al cerrar, se corre:**
  ```bash
  cargo test -p clawg-core --lib -- memory::portability memory::db
  ```
- Commit: `refactor(core): keep the running imports in a type of the import`

## 6. La fachada y el código muerto (SB9, SB10)

- [ ] **Test en rojo:** `memory_declares_no_public_module` (`conventions.rs`). Se escribe primero y queda en rojo hasta la última tanda, con la que se commitea; así cada commit intermedio pasa los tests de los archivos que cambia.
- [ ] **Por cada tanda** ([D7](design.md#d7-la-fachada-de-memory)): primero los módulos que ningún otro crate nombra, después los demás, de a uno o de a pocos.
  1. Los módulos de la tanda pasan a `pub(crate) mod`.
  2. `cargo check --workspace --all-targets` dice qué usan los otros crates. Eso va a la fachada de `memory/mod.rs`, con los alias de D7 y bajo `cfg(feature = "testing")` lo que sólo usan tests, y `clawg-cli` y `clawg-eval` pasan a importar desde `clawg_core::memory`.
  3. `cargo clippy --locked -p clawg-core -- -D warnings` dice qué quedó muerto.
- [ ] **Lo que aparece muerto** ([D8](design.md#d8-el-código-muerto)) se resuelve antes de commitear la tanda. Lo que no usa nada se borra con sus tests, en un commit propio y anterior al de la tanda: con el módulo todavía abierto, ese commit compila igual. Lo que sólo usan tests o `eval_api` queda detrás de su `cfg`, en el commit de la tanda. La cuenta de cada caso va anotándose para `verify.md`.
- [ ] **Al cerrar cada tanda, se corre:**
  ```bash
  cargo clippy --locked -p clawg-core -- -D warnings
  ```
  ```bash
  cargo test -p clawg-cli --lib --test api --test binary
  ```
  ```bash
  cargo test -p clawg-eval
  ```
  Y, en la última, también:
  ```bash
  cargo test -p clawg-cli --test conventions
  ```
- Commits: `refactor(core): drop <ítem> nobody calls`, uno por borrado, y `refactor(core): close <módulos> behind the memory facade`, uno por tanda.

## 7. Verificación

- [ ] Sobre el último commit de código, en Linux:
  ```bash
  cargo test --workspace
  ```
  ```bash
  cargo clippy --workspace --all-targets -- -D warnings
  ```
  ```bash
  cargo clippy --locked -p clawg-cli -- -D warnings
  ```
  ```bash
  cargo clippy --locked -p clawg-core -- -D warnings
  ```
- [ ] Las comprobaciones de [D9](design.md#d9-cómo-se-comprueba-que-ninguna-respuesta-cambia): los dos contratos sin `CLAWG_BLESS` y sin diferencias contra `origin/develop`, y el `git diff … | rg` de las aserciones, con cada línea que liste explicada en `verify.md`.
- [ ] El CI del PR, que también corre Windows.
- [ ] `verify.md` con lo que se comprobó; cuántos ítems se borraron y cuántos quedaron detrás de cada `cfg`; el texto de `locate_project` que puede cambiar y por qué ningún test lo alcanza; y los dos textos de log que cambian.
- Commit: `docs(openspec): record store-boundary applied`

## 8. Archivo

En su propia tarea, después de la verificación.

- [ ] Las specs:
  - SB1, SB6, SB9 y SB10 en [`quality`](../../specs/quality/spec.md);
  - SB2 en [`project`](../../specs/project/spec.md);
  - SB3, SB4, SB5 y SB7 en [`storage`](../../specs/storage/spec.md), donde también se corrige la cita de `MemoryRuntime::close` en «El daemon cierra `memory.db` antes de salir»;
  - SB3, para las herramientas, en [`eval`](../../specs/eval/spec.md).
- [ ] La carpeta pasa a `changes/archive/AAAA-MM-DD-store-boundary`; `openspec/README.md` y `openspec/ROADMAP.md`; y la versión sube un minor (`chore: bump version to X.Y.Z`).
