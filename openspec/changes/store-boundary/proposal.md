# store-boundary — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | — (retire-legacy-kinds ya está archivado) | ver [`state.yaml`](state.yaml) |

## Problema

`clawg-core` no tiene una frontera: la conexión a SQLite, los errores de `rusqlite` y casi todos los módulos de `memory` se ven desde `clawg-cli` y `clawg-eval`. Por eso mover o abstraer algo del núcleo toca a los otros dos crates, y por eso el compilador no ve código muerto. Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código); las citas de acá están releídas contra el código de hoy:

1. **La conexión es pública.** `pub conn: Connection` (`crates/clawg-core/src/memory/db.rs:157`). En producción la usan `locate_project` (`crates/clawg-cli/src/daemon/api.rs`), que cuenta con SQL crudo las carpetas de un proyecto, y `clawg-eval` para cerrar el store: `release` (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:311`) y tres cierres de `revision-eval` (`crates/clawg-eval/src/bin/revision-eval/run.rs:490`, `:634`, `:799`). Lo hacen porque `MemoryRuntime::close` registra el error y no lo devuelve (`crates/clawg-core/src/runtime.rs:355-359`), y esas herramientas terminan con `RunError::Close`.
2. **El error expone la implementación.** `MemoryError` tiene `Migration(refinery::Error)`, `Connection(tokio_rusqlite::Error)` y `Sqlite(rusqlite::Error)` (`crates/clawg-core/src/memory/error.rs:11`, `:14`, `:17`), además de `Backend(String)` (`:42`), y dos variantes que nada construye: `VecExtensionRegistration` y `RowDecode` (`:20`, `:23`). Fuera de `clawg-core` sólo se distinguen `StoreInUse`, `SchemaNewer` y `SchemaDiverged` (`crates/clawg-cli/src/daemon/serve.rs:94`, `crates/clawg-cli/src/restore.rs:34`, `crates/clawg-cli/src/secrets.rs:367`, `crates/clawg-cli/src/status.rs:80`, `:83`). La traducción de `tokio_rusqlite::Error` está escrita a mano en unos 20 archivos de `crates/clawg-core/src` (por ejemplo `search.rs:595-601`, `relocate.rs:622-627`, `merge.rs:364-369`, `subfolder.rs:191-194`).
3. **Todo es público.** 44 de los 52 módulos de `memory` son `pub mod` (`crates/clawg-core/src/memory/mod.rs:26-92`; 7 son `pub(crate)` y `crowding` sólo existe en los tests). `pub` apaga `dead_code`: `VecExtensionRegistration` y `RowDecode` son dos casos que se ven a simple vista.
4. **`MemoryStore` guarda estado de un caso de uso.** El conjunto de importaciones en curso es un `HashSet` en el store (`db.rs:182`), que llenan y vacían `Journal` (`crates/clawg-core/src/memory/portability/import.rs:981`, `:1022-1028`) y lee `undo_import` (`crates/clawg-core/src/memory/portability/undo.rs:1208`).

## Enfoque

Los tres **A** de la exploración: la conexión pasa a `pub(crate)` con un acceso crudo sólo para los tests de afuera; un `StorageError` opaco reemplaza las variantes de almacenamiento de `MemoryError`; y los módulos de `memory` se cierran con una fachada de `pub use`. Las tres decisiones abiertas están tomadas en el [diseño](design.md):

1. `StorageError` **no responde ninguna pregunta**: nadie lee hoy un código de SQLite a través de `MemoryError` ([D2](design.md#d2-storageerror-no-responde-ninguna-pregunta)).
2. Las importaciones en curso pasan a **un tipo propio de la importación**, que el store sigue guardando: no van a `MemoryRuntime` ([D4](design.md#d4-las-importaciones-en-curso)).
3. `MemoryRuntime` **conserva sus campos públicos** ([D6](design.md#d6-memoryruntime-conserva-sus-campos-públicos)).

## Alcance

1. **Lo que hoy usa la conexión desde afuera.** Una función de `registry` cuenta las carpetas de un proyecto, y `locate_project` la usa. `MemoryRuntime::close` devuelve `Result<(), MemoryError>`: el daemon registra el error con el mismo texto de hoy, y `rag-eval` y `revision-eval` lo propagan como `RunError::Close`.
2. **La conexión cerrada.** `conn` pasa a `pub(crate)`; los tests de los otros crates leen filas con `MemoryStore::raw_connection()`, que sólo existe con `cfg(test)` o la feature `testing`. `clawg-eval` suma `clawg-core` con `testing` a sus `[dev-dependencies]` para los tests de `rag-eval` que siembran filas.
3. **`StorageError`.** `MemoryError::Storage(StorageError)` reemplaza `Migration`, `Connection`, `Sqlite` y `Backend`; `VecExtensionRegistration` y `RowDecode` se borran. El texto y la fuente de cada error quedan como hoy. Una sola función, en `error.rs`, traduce lo que devuelve `Connection::call`.
4. **Las importaciones en curso** como tipo propio de `portability`, guardado por el store.
5. **La fachada.** `memory/mod.rs` no declara ningún módulo público; reexporta lo que usan `clawg-cli` y `clawg-eval`, y `clawg-cli` y `clawg-eval` lo importan de ahí. Lo que el compilador señale como muerto se borra en commits aparte.
6. **Las reglas en `crates/clawg-cli/tests/conventions.rs`**: el código de producción de `clawg-cli` y `clawg-eval` no nombra `rusqlite` ni `.conn`; sólo `error.rs` mira las variantes de `tokio_rusqlite::Error`; `memory` no declara módulos públicos.
7. **Docs:** `docs/reference/testing.md`, para el acceso crudo de los tests.

## Fuera de alcance

Lo que la exploración pone en [«No-alcance»](exploration.md#no-alcance):

- Traits, puertos o capas de lectura y escritura (storage-ports, storage-read-side, storage-write-side).
- Mover archivos: es memory-module-layout. Este cambio no mueve ningún archivo ni cambia ningún `module_path!`.
- Cambiar conducta, respuestas o códigos de salida.

Y lo que deja afuera esta tarea: el código (va en otra, cuando entre a `develop` el cambio que también toca `crates/clawg-cli/src/daemon/api.rs`), subir la versión, archivar, `openspec/README.md`, `openspec/ROADMAP.md` y las specs principales (se consolidan al archivar).

## Datos guardados

Ninguno cambia: no hay migración, ni cambia el esquema, ningún identificador guardado ni el contrato de embedding. El rollback es volver al binario anterior, que abre la misma base.

## Cómo se mide

Tests que fallan hoy, con nombre en la [spec](spec.md):

- **La regla de la exploración:** `no_production_code_outside_the_core_reaches_sqlite` (`conventions.rs`) falla hoy por `locate_project`, por `release` de `rag-eval` y por los tres cierres de `revision-eval`.
- `only_the_storage_error_matches_the_connection_errors` (`conventions.rs`) falla por los unos 20 archivos que traducen a mano.
- `memory_declares_no_public_module` (`conventions.rs`) falla por los 44 `pub mod`.
- `a_close_that_fails_is_returned` (`runtime.rs`), `own_folders_are_counted_as_the_registry_reads_them` (`registry.rs`), `a_storage_failure_is_one_opaque_variant` y `a_call_on_a_closed_connection_names_what_it_was_doing` (`error.rs`), `an_import_holds_its_number_until_it_ends` (`import.rs`): no compilan hoy.

Además:

- **Ninguna respuesta cambia** ([D9](design.md#d9-cómo-se-comprueba-que-ninguna-respuesta-cambia)): tres fijaciones que pasan antes de tocar el código y no se tocan después, los dos contratos del agente sin bendecir, y ninguna aserción existente cambiada.
- La suite y los dos clippy pasan, y `cargo clippy --locked -p clawg-core -- -D warnings` también, sin features.
- `dead_code` encuentra lo que antes no veía: `verify.md` dice cuántos ítems se borraron y cuáles quedaron detrás de `cfg`.
