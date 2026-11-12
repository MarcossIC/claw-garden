# store-boundary — Verificación

**Resultado:** aplicado y verificado en Linux, y la suite pasa en el job de Windows del CI. La suite entera y los dos clippy pasan sobre el último commit de código. Ningún test de una respuesta ni de los contratos del agente cambió lo que comprueba. En Windows, las tres herramientas de evaluación borran su directorio temporal ([«En Windows, en local»](#en-windows-en-local)). Queda lo que se lista en [«Pendiente»](#pendiente).

## Qué se corrió

Sobre el último commit de código, en Linux, con el directorio de build limpio al empezar:

| Comando | Resultado |
|---|---|
| `cargo test --workspace` | pasa, sin ningún fallo; incluye los dos doctests de `MemoryStore` |
| `cargo clippy --workspace --all-targets -- -D warnings` | sin hallazgos |
| `cargo clippy -p clawg-cli -- -D warnings` | sin hallazgos; es el núcleo sin `testing` ni `eval`, como lo construye el release |

Además, `cargo check` no deja ningún warning en estas configuraciones del núcleo: `-p clawg-cli`, `-p clawg-eval`, `--workspace --all-targets`, `-p clawg-core --lib --profile test`, `-p clawg-core --tests` y `-p clawg-core --features testing --all-targets`.

Cada commit de la tarea 7 se revisó por separado con `cargo check --workspace --all-targets` y `cargo check -p clawg-cli`: ninguno deja un warning.

## Los tests en rojo

| Test | Qué se vio antes del cambio |
|---|---|
| `no_production_code_outside_core_reaches_the_connection` | falló con las 5 líneas del escenario de R1: `daemon/api.rs` (`locate_project`), `rag-eval/agent_view/runner.rs` (`release`) y tres de `revision-eval/run.rs` |
| `every_public_memory_module_is_named_outside_core` | falló con los 9 módulos de D10 |
| `no_memory_code_translates_a_connection_error_by_hand` | se escribió junto con la traducción única. Su rojo se contó sobre `develop` con la misma búsqueda: 47 líneas de producción en 20 archivos, las de D2 |
| `a_call_on_a_closed_store_fails_with_a_storage_error`, `memory_error_tells_apart_what_another_crate_handles` | no compilaban: `MemoryError::Storage` no existía |
| `a_runtime_closes_and_says_so` | no compilaba: `close` devolvía `()` |
| El doctest `compile_fail` de `MemoryStore` | por construcción: con `conn` público compila, y entonces falla. Un `rustdoc` estable no comprueba el `E0616`; el doctest gemelo, que sólo difiere en lo que lee, pasa |
| Los de R8 (`RunningImports`, el directorio del worker) | no compilaban hasta el cambio |

Los cinco tests de caracterización de R6 y la aserción del `detail` entero de `folder_shared` se escribieron primero, pasaron sobre el código de `develop` con los literales que dio, y siguen pasando sin haberse tocado.

## Las comprobaciones de D11

1. **Las aserciones.** En el diff de `develop...HEAD` sobre `crates/*/tests` y `crates/*/src`, las líneas de `assert` que cambian son:
   - las de `production_lines_naming` en `conventions.rs`, que pasa de una lista de nombres a un predicado;
   - `from_rusqlite_error` y `backend_error_displays_message`, que nombran `Storage`, y `row_decode_displays_table_name`, que se va con su variante;
   - las dos de los imports en curso, que leen `running.is_empty()` en lugar de `store.importing.lock().unwrap().is_empty()`, y la de `an_unknown_or_running_import_is_refused`, que recibe el `&RunningImports`;
   - la de `a_key_leaves_a_hidden_authors_holder`, que guarda con el directorio de `sharing.json` y comprueba lo mismo, más que el save no falló, como ya lo hacía el helper que usaba.

   El resto son aserciones nuevas. Fuera de las aserciones, los tests cambian en `.conn` por `.conn_for_test()` (las 47 de D1, una de ellas a `close`), en un `.expect` sobre `close` y en el argumento nuevo.
2. **Los contratos del agente.** `git diff develop...HEAD -- crates/clawg-cli/src/mcp/agent_contract.json crates/clawg-cli/src/mcp/agent_contract.es.json` no muestra nada, y los dos tests pasan sin `CLAWG_BLESS`.
3. **El daemon.** En `api.rs` cambian la consulta de `locate_project`, el argumento `&app.runtime.imports` en `import_memories`, `import_as_job` y `undo_import`, y los tests. Ningún `ApiError` ni `StatusCode` cambia: el `.map_err(ApiError::internal)` de la consulta es el mismo, ahora sobre `own_folder_count`.
4. **La línea de comandos.** `crates/clawg-cli/tests/binary.rs` pasa sin cambios.

## Lo que mostró `dead_code`

Las cuentas son de ítems de módulo y de métodos (`fn`, `struct`, `enum`, `const`, `static`, `type`, `trait`, `use`, `mod`) en el código de producción de cada archivo. Los campos de un struct público no se tocaron.

**Los 9 módulos que nadie nombra** (tarea 6):

| Módulo | Qué pasó a un `cfg` |
|---|---|
| `query_context` | el módulo entero, a `cfg(any(test, feature = "eval"))`: sólo lo usan `eval_api` y sus tests |
| `maintenance` | `ReembedMode::Normalize`, el campo `existing` de la fila que lee y `deserialize_f32_vec`, a `cfg(test)` |
| `search` | `serialize_f32_query_pub`, de `cfg(any(test, feature = "testing"))` a `cfg(test)` |
| `model_download` | `load_tokenizer`, a `cfg(any(test, feature = "eval"))`, al cerrarse `chunker` |

**Los 35 módulos que otros crates nombran** (tarea 7):

| Módulo | `pub` antes | `pub` después | a `pub(crate)` | a un `cfg` | borrados |
|---|---:|---:|---:|---|---:|
| `clock` | 3 | 3 | 0 | | 0 |
| `vector` | 2 | 2 | 0 | | 0 |
| `redact` | 6 | 2 | 4 | | 0 |
| `events` | 3 | 2 | 1 | `from_message` a `cfg(test)` | 0 |
| `spill` | 2 | 2 | 0 | | 0 |
| `language` | 4 | 3 | 1 | | 0 |
| `provenance` | 1 | 1 | 0 | | 0 |
| `sharing` | 16 | 13 | 3 | | 0 |
| `store_lock` | 6 | 6 | 0 | | 0 |
| `identity` | 25 | 15 | 10 | `resolve_project_root` a `testing` | 0 |
| `chunker` | 18 | 7 | 11 | `new_from_base_dir` a `cfg(test)`; `from_tokenizer`, `chunk` y `Source::Fixed` a `cfg(any(test, feature = "eval"))` | 0 |
| `embedder` | 20 | 14 | 6 | | 0 |
| `idle_embedder` | 10 | 8 | 2 | | 0 |
| `bootstrap` | 27 | 11 | 16 | | 0 |
| `backup` | 22 | 16 | 6 | | 0 |
| `db` | 12 | 6 | 6 | | 0 |
| `store_insert` | 9 | 4 | 5 | `insert_memory_with_vector` e `insert_noting` a `cfg(any(test, feature = "eval"))` | 0 |
| `outbox` | 11 | 6 | 5 | | 0 |
| `worker` | 9 | 5 | 4 | | 0 |
| `workspace` | 15 | 5 | 10 | | 0 |
| `prompt_builder` | 13 | 12 | 0 | | 1 |
| `inspect` | 23 | 22 | 1 | `Invariants::is_empty` a `cfg(test)` | 0 |
| `secrets` | 13 | 13 | 0 | | 0 |
| `admin` | 26 | 22 | 4 | | 0 |
| `dispute` | 13 | 13 | 0 | | 0 |
| `synthesis` | 16 | 10 | 6 | | 0 |
| `topic` | 4 | 3 | 1 | | 0 |
| `project` | 6 | 5 | 1 | | 0 |
| `project_docs` | 50 | 29 | 21 | `RefreshOutcome`, `is_due` y `try_refresh` a `testing` | 0 |
| `relocate` | 10 | 10 | 0 | | 0 |
| `merge` | 8 | 6 | 2 | | 0 |
| `subfolder` | 7 | 4 | 3 | | 0 |
| `project_delete` | 12 | 11 | 1 | | 0 |
| `portability` | 131 | 112 | 19 | `ImportTimings::steps` a `cfg(test)` | 0 |
| `registry` | 31 | 29 | 2 | | 0 |
| **Total** | **584** | **432** | **151** | 4 a `testing`, 4 a `cfg(test)`, 5 a `cfg(any(test, feature = "eval"))` | **1** |

«`testing`» es `cfg(any(test, feature = "testing"))`.

**Total de lo borrado:** `SEP_WORKSPACE_TEST` (`prompt_builder.rs`), que nada usaba, en la tarea 7; y en la tarea 2, las variantes `RowDecode` y `VecExtensionRegistration` de `MemoryError`, que nada construía, con las 30 traducciones a mano que reemplaza `failed_call`. El código muerto que la exploración temía no apareció: casi todo lo que dejó de ser `pub` lo usa el propio núcleo.

## Lo que se resolvió distinto de lo escrito

- **Tres `MemoryError::Backend` que el diseño no contó.** `fetch_last_session` y `fetch_recent_saves` (`prompt_builder.rs`) envuelven la falla entera, y en un caso un error de `serde_json`, con `<op>: <falla>`. No nombran una variante de `tokio_rusqlite::Error`, así que no están entre las 30. Usan `MemoryError::channel`, del crate, en `error.rs`, y su texto queda igual.
- **`MemoryError::into_sqlite`**, del crate. `plan_ids_in` (`project.rs`) devuelve `rusqlite::Result` y sacaba el error de SQLite de un `MemoryError::Sqlite` con un patrón. `StorageError` sigue opaco para otro crate.
- **`MemoryPool` sigue `pub`**, reexportado en `memory::` con su motivo. Sus campos sólo los leen `eval_api` y los tests. Cerrarlo obligaba a que un build sin `eval` dejara de llenarlo, y con él `KnnOutcome::saves` y `KnnOutcome::eligible`: una búsqueda habría contado su pool bajo test y no en un release. Es el caso del paso 4 de D10.
- **`EvalStore` sigue siendo una tupla**, con el directorio en el segundo campo, para que los tests que leen `store.0` no cambien.
- **`RunningImports::is_empty`**, bajo `cfg(test)`, además de `hold`, `release` y `holds`: deja las dos aserciones de los imports como estaban.
- **`public_api.rs`.** `memory_runtime_open_creates_the_db_and_returns_a_usable_store` lee una cuenta con SQL, y ahora lleva `cfg(feature = "testing")` como los demás tests del archivo que usan la conexión. Con `cargo test --workspace` corre igual que antes.
- **Los reexports de `portability` y `chunker`.** `Progress` sale del `pub use` de `portability` y `SaveSplit` pasa a `pub(crate) use` en `chunker`: ningún otro crate los nombra por esa ruta. Los dos tipos siguen `pub`, porque están en la firma de métodos que sí se usan.
- **Los commits de la tarea 7** van por grupos de módulos, en el orden de D10, y no de a uno: `redact`, `events`, `language` y `sharing`; `identity`; `chunker`; `embedder`, `idle_embedder` y `bootstrap`; `backup`, `db`, `store_insert` y `outbox`; `worker`, `workspace`, `prompt_builder` e `inspect`; `admin`, `synthesis` y `topic`; `project`, `project_docs`, `merge`, `subfolder` y `project_delete`; `portability`; `registry`. Los módulos que no cambiaron (`clock`, `vector`, `spill`, `provenance`, `store_lock`, `secrets`, `dispute`, `relocate`) no tienen commit.
- **Dos gemelas de plataforma siguen `pub`.** `keep_one_malloc_arena` (`embedder.rs`) y `sync_dir` (`backup.rs`) tienen una definición por plataforma, y los otros crates usan las dos. Linux compila una sola, así que la otra había quedado `pub(crate)`: el job de Windows del CI lo marcó como código sin uso, y se corrigió en un commit aparte.
- **`process_tracked`** pasa a ocho parámetros y lleva el mismo `#[expect(clippy::too_many_arguments)]` que `import_parsed`.

## El CI en Windows

El job `test (windows-latest)` compila el código `cfg(windows)` de los tres crates y corre la suite con finales de línea CRLF. Marcó dos cosas:

- **Las dos gemelas de plataforma** de arriba, que eran de este cambio.
- **`a_daemon_records_its_requests_and_report_reads_them_after_stop`** (`crates/clawg-cli/tests/binary.rs`) falló una vez y pasó al repetir el job con el mismo commit: `clawg metrics report`, pedido justo después de `clawg stop`, contestó `attempt to write a readonly database`. Este cambio no toca `metrics.db`. Es el texto de SQLite cuando una conexión de sólo lectura encuentra un journal junto al archivo: en Windows el borrado del journal llega al directorio recién cuando se cierra el último handle sobre él. Se reprodujo en Linux dejando el journal a mano (`a_reader_waits_for_a_journal_its_writer_is_removing`, `crates/clawg-core/src/metrics/store.rs`), y `MetricsDb::open_read_only` ahora vuelve a mirar durante 5 s. Que esa sea la causa del fallo del CI es la lectura que mejor encaja con el texto y con que no se repita; no se observó el journal en el runner.

El check `advisories` falla por un aviso sobre `rmcp` 3.5.1 que `develop` también tiene: no es de este cambio.

## Pendiente

Lo que el CI no comprueba:

- **El `E0616` del doctest**, que sólo comprueba un `rustdoc` nightly. No se corrió.

El otro punto que el CI no comprueba, que las herramientas de evaluación borren su directorio temporal en Windows, se corrió al archivar: está en [«En Windows, en local»](#en-windows-en-local).

## En Windows, en local

`rag-eval --agent-view`, `revision-eval` y `conflict-eval` cierran con `MemoryRuntime::close`, y en Windows un store que no cerró no se puede borrar. Se corrieron el 2026-11-12 en Windows 11, sobre `develop` con este cambio, con el modelo falso, un build debug en `target-test` limpiado antes de compilar, y sin ningún perfil de datos. La compilación no dejó warnings.

| Comando | Salida | Directorio temporal |
|---|---|---|
| `cargo run -p clawg-eval --bin rag-eval --target-dir target-test -- --fake --agent-view --out <dir>` | 0 | ningún `rag-eval-*` antes ni después |
| `cargo run -p clawg-eval --bin revision-eval --target-dir target-test -- chains --fake` | 0 | ningún `revision-eval-*` antes ni después |
| `cargo run -p clawg-eval --bin conflict-eval --target-dir target-test -- --fake` | 0 | ningún directorio nuevo |

`conflict-eval` crea su directorio con `tempfile::tempdir_in`, sin un prefijo propio, así que se comparó el listado entero del directorio temporal del sistema antes y después de la corrida: no quedó ningún directorio nuevo. Aparecieron tres archivos `.tmp` sueltos; `tempdir_in` crea directorios, así que se leen como de otro proceso, pero no se comprobó cuál los creó.

`revision-eval store --export` conserva su directorio a propósito hasta que se etiqueta, y no se corrió.

## Al archivar (2026-11-12)

- Los ids cambian al consolidar. R1 a R10 chocan con los ids de cada spec, así que los requisitos de este cambio pasan a SB. Mapeo: R1, R2, R4, R5, R6, R7 y R9 pasan a SB1, SB2, SB4, SB5, SB6, SB7 y SB9 en [`specs/storage`](../../../specs/storage/spec.md), en «El límite del almacenamiento». R3 pasa a SB3 en [`specs/project`](../../../specs/project/spec.md), después de PL39, que ya describe `locate_project` y el 409 `folder_shared`. R8 se parte: los imports en curso son SB8 en [`specs/portability`](../../../specs/portability/spec.md), después de IU5, y `sharing.json` es SB8 en [`specs/sharing`](../../../specs/sharing/spec.md), al final. R10 («Lo que ve un cliente no cambia») no se consolida, porque es de conservación. Este archivo conserva los ids R.
- Sale en 0.84.0.
- De «Pendiente» se corrió lo de Windows, que pasó a «En Windows, en local». Queda el `E0616` del doctest, que pide un `rustdoc` nightly.
- Citas: el `spec.md` de este cambio no cita líneas de código, y los nombres de test que cita existen en los archivos que nombra. Las de lo que se consolidó se tomaron del código de hoy. En las secciones que se tocaron se corrieron las citas cercanas: en `specs/storage`, las del cierre de `clawg serve` en «Apagado sin pérdida» (`runtime.rs` y `serve.rs`); en `specs/sharing`, las cinco de `sharing.rs` en R1; en `specs/portability`, las de `Journal` y `write` en «Dónde está». Las citas `path:línea` de `proposal.md`, `design.md` y `exploration.md` describen el código de antes de este cambio y no se corrigieron.
- Diferencias entre la spec del cambio y el código: las que ya están en «Lo que se resolvió distinto de lo escrito». En particular R9 pedía que un ítem fuera `pub` sólo si otro crate lo usa, y el código deja `pub` también lo que está en la firma de algo que otro crate usa (`MemoryPool`, `Progress`, `SaveSplit`) y las dos gemelas de plataforma; SB9 dice lo que hace el código. En lo demás que se rechequeó (los textos de SB6, la forma de `StorageError`, las variantes que quedan de `MemoryError`, `close`, `own_folder_count`, `RunningImports` y el directorio del worker) no se encontró ninguna.
