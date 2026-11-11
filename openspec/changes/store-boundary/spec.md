# store-boundary — Especificación

Requisitos sobre [`specs/quality`](../../specs/quality/spec.md) (las convenciones de `conventions.rs`), [`specs/project`](../../specs/project/spec.md) (el conteo de carpetas de `locate`), [`specs/storage`](../../specs/storage/spec.md) (el cierre y los errores del store) y [`specs/eval`](../../specs/eval/spec.md) (el cierre de `rag-eval` y `revision-eval`). Se consolidan al archivar. Lo que no se nombra acá sigue como está; en particular, ningún cambio de este documento cambia una respuesta del daemon, del bridge ni de la línea de comandos (SB8).

Términos:

- **Código de producción** de un archivo: sus líneas anteriores al primer `#[cfg(test)]` seguido de una línea `mod <nombre> {`.
- **Fuera del núcleo:** `crates/clawg-cli/src` y `crates/clawg-eval/src`.
- **Fallo de almacenamiento:** un error que viene de SQLite, de la conexión de tokio-rusqlite o de refinery.

## SB1. Fuera del núcleo nada llega a SQLite

Ninguna línea de código de producción fuera del núcleo MUST nombrar `rusqlite` (también `tokio_rusqlite`) ni `.conn` seguido de algo que no sea un carácter de identificador. Una declaración `#[cfg(test)] mod <nombre>;` MUST NOT terminar el código de producción.

#### Escenario: hoy
- GIVEN el código de `develop`
- WHEN corre la regla
- THEN falla y nombra `crates/clawg-cli/src/daemon/api.rs` (en `locate_project`), `crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs` (en `release`) y tres líneas de `crates/clawg-eval/src/bin/revision-eval/run.rs`

#### Escenario: un módulo de tests declarado sin cuerpo
- GIVEN un archivo con `#[cfg(test)]` y `mod t;`, después una línea con `.conn`, y después `#[cfg(test)]` y `mod tests {` con otra línea con `.conn`
- WHEN se calcula su código de producción
- THEN la primera línea con `.conn` está en él y la segunda no

**Test:** `no_production_code_outside_the_core_reaches_sqlite` y el puro `the_production_code_runs_past_a_test_module_declared_without_a_body` (`crates/clawg-cli/tests/conventions.rs`). `no_production_code_names_the_retired_tables`, que usa el mismo helper, MUST seguir pasando.

## SB2. Las carpetas de un proyecto se cuentan en el núcleo

`own_folder_count` (`crates/clawg-core/src/memory/registry.rs`) MUST devolver cuántas carpetas de `project_path` pertenecen al proyecto, las mismas filas que lee `own_folders_in`. `locate_project` MUST usarla, y su respuesta MUST seguir siendo la de hoy: `409 folder_shared` con el mismo `detail` cuando el proyecto tiene más de una carpeta.

#### Escenario: un proyecto con dos carpetas
- GIVEN un proyecto con una carpeta propia y otra que se le asignó, y otro proyecto sin carpetas
- WHEN se cuentan las carpetas de cada uno
- THEN el primero tiene 2 y el segundo 0

**Test:** `own_folders_are_counted_as_the_registry_reads_them` (`registry.rs`); `a_folder_shared_by_its_project_is_not_located` (`crates/clawg-cli/tests/api.rs`), sin cambios en lo que comprueba.

## SB3. Cerrar el runtime devuelve su error

`MemoryRuntime::close` MUST devolver `Result<(), MemoryError>`, con el error que da cerrar la conexión del store. `clawg serve` y los cierres de `eval_api` que hoy cuentan con el log MUST registrar un cierre que falla con el texto de hoy y seguir como hoy. `rag-eval` y `revision-eval` MUST cerrar con `MemoryRuntime::close` y terminar con `RunError::Close` si falla.

#### Escenario: SQLite rechaza el cierre
- GIVEN un runtime de prueba con una sentencia que quedó sin finalizar
- WHEN se cierra
- THEN `close` devuelve un error

**Test:** `a_close_that_fails_is_returned` (`crates/clawg-core/src/runtime.rs`).

## SB4. La conexión del store no es pública

`MemoryStore::conn` MUST ser `pub(crate)`. Un test de otro crate que lee o siembra filas MUST hacerlo con `MemoryStore::raw_connection()`, que MUST existir sólo con `cfg(test)` o con la feature `testing`. Ninguna sección que no sea `[dev-dependencies]` MUST encender `testing`.

**Test:** el compilador (con `pub(crate)`, una línea que la use fuera de `clawg-core` no compila); `no_production_code_outside_the_core_reaches_sqlite` para el código de producción; `only_dev_dependencies_turn_on_the_test_stand_ins` (`conventions.rs`) para la dependencia de desarrollo que suma `clawg-eval`.

## SB5. Un fallo de almacenamiento es una sola variante opaca

Todo fallo de almacenamiento MUST ser `MemoryError::Storage(StorageError)`. Ninguna variante pública de `MemoryError` ni de `StorageError` MUST llevar un tipo de `rusqlite`, `tokio_rusqlite` ni `refinery`, y `StorageError` MUST NOT ofrecer métodos públicos para preguntar por la causa. El `Display` y el `source()` de cada fallo MUST ser los de la variante que reemplaza. `MemoryError` MUST NOT tener `VecExtensionRegistration` ni `RowDecode`.

#### Escenario: un fallo de SQLite
- GIVEN un `rusqlite::Error` convertido con `?`
- WHEN se mira el `MemoryError` que resulta
- THEN es `MemoryError::Storage`, su texto es «SQLite error: » seguido del texto del error de SQLite, y su `source()` es ese error

**Test:** `a_storage_failure_is_one_opaque_variant` y la fijación `a_storage_failure_reads_as_before` (`crates/clawg-core/src/memory/error.rs`).

## SB6. Una sola traducción de los errores de la conexión

Lo que devuelve un `Connection::call` fallido MUST traducirlo `call_failed` (`error.rs`): el error de la operación pasa tal cual, y una conexión cerrada se vuelve un `StorageError` que dice «connection closed during» y qué se estaba haciendo. Ninguna línea de código de producción de `crates/clawg-core/src` fuera de `memory/error.rs` MUST nombrar `tokio_rusqlite::Error::`.

#### Escenario: una llamada sobre una conexión cerrada
- GIVEN un store cuya conexión se cerró
- WHEN una operación del núcleo hace un `call`
- THEN el error dice «connection closed during» y el nombre de esa operación

**Test:** `a_call_on_a_closed_connection_names_what_it_was_doing` (`error.rs`); `only_the_storage_error_matches_the_connection_errors` (`conventions.rs`).

## SB7. Las importaciones en curso son un tipo de la importación

`MemoryStore` MUST guardar las importaciones en curso en un `ImportsRunning` (`crates/clawg-core/src/memory/portability/import.rs`), que cada clon comparte. Una importación MUST tener su número desde que se numera hasta que termina, también cuando termina con un error. `undo_import` MUST seguir rechazando una importación en curso con `UndoError::Running`.

#### Escenario: una importación que termina mal
- GIVEN una importación que se numera y después falla
- WHEN termina
- THEN mientras corría `holds` decía que sí, y después dice que no

**Test:** `an_import_holds_its_number_until_it_ends` (`import.rs`); `an_unknown_or_running_import_is_refused` (`crates/clawg-core/src/memory/portability/undo.rs`), con lo mismo comprobado por los métodos del tipo.

## SB8. Ninguna respuesta cambia

Ninguna respuesta del daemon (estado HTTP y cuerpo, incluido el `error` de un 500), del bridge (esquemas, descripciones y textos de las herramientas) ni de la línea de comandos (stdout, stderr y código de salida) MUST cambiar. La única excepción es el `error` del 500 de `locate_project` cuando SQLite falla justo en el conteo de carpetas, que pasa a tener el texto que ya da el mismo handler cuando falla `find_project` ([D9](design.md#d9-cómo-se-comprueba-que-ninguna-respuesta-cambia)).

#### Escenario: una consulta que falla
- GIVEN la tabla `memory` rota a propósito
- WHEN se pide `GET /v1/memories`
- THEN el daemon responde 500 con el `error` exacto que responde hoy

#### Escenario: una búsqueda de proyecto que falla
- GIVEN la tabla `project` rota a propósito
- WHEN se pide `POST /v1/projects/locate`
- THEN el daemon responde 500 con el `error` exacto que responde hoy

**Test:** las fijaciones `a_failed_query_answers_with_the_sqlite_text` y `a_failed_lookup_answers_with_the_connection_text` (`crates/clawg-cli/tests/api.rs`) y `a_storage_failure_reads_as_before` (`error.rs`), escritas y en verde antes de tocar el código; `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` sin `CLAWG_BLESS`; las dos comprobaciones de `git diff` de [D9](design.md#d9-cómo-se-comprueba-que-ninguna-respuesta-cambia).

## SB9. `memory` no declara módulos públicos

`crates/clawg-core/src/memory/mod.rs` MUST NOT declarar ningún `pub mod`. Lo que usan `clawg-cli` y `clawg-eval` MUST estar reexportado en `clawg_core::memory`, y un nombre que sólo usan los tests de otro crate MUST reexportarse bajo `#[cfg(feature = "testing")]`. Un nombre que exportan dos módulos MUST tomar en la fachada el alias de [D7](design.md#d7-la-fachada-de-memory). Ningún `module_path!` MUST cambiar.

**Test:** `memory_declares_no_public_module` (`conventions.rs`); que lo de afuera alcance lo que necesita lo prueba el compilador, sobre la suite entera.

## SB10. `dead_code` ve lo que nadie usa

`cargo clippy --locked -p clawg-core -- -D warnings`, sin features, MUST terminar con código 0. Un ítem que sólo usan los tests MUST estar detrás de `cfg(test)` o de `cfg(any(test, feature = "testing"))`; uno que sólo usa `eval_api`, detrás de `cfg(feature = "eval")`; uno que nada usa MUST borrarse. Ninguno MUST quedar con un `allow` ni un `expect` de `dead_code`.

**Test:** el comando de arriba y el de `.github/workflows/ci.yml` (`cargo clippy --locked -p clawg-cli -- -D warnings`); `no_source_allows_dead_code` (`conventions.rs`).
