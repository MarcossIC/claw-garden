# store-boundary — Diseño

Los enfoques están elegidos en la [exploración](exploration.md) y en la [propuesta](proposal.md). Acá van las decisiones de implementación, las tres abiertas entre ellas (D2, D4, D6), y cómo se comprueba que ninguna respuesta cambia (D9).

En `crates/clawg-cli/src/daemon/api.rs` se nombran funciones y no líneas: otro cambio que entra antes a `develop` las mueve. Las demás citas son del código de hoy.

## D1. Lo que hoy usa la conexión desde afuera

**El conteo de carpetas.** `locate_project` (`api.rs`) cuenta con SQL crudo las filas de `project_path` de su proyecto, para responder `folder_shared` cuando hay más de una. La consulta es la misma que ya lee `own_folders_dated_in` (`crates/clawg-core/src/memory/registry.rs:826-832`, con el mismo `WHERE` en `:829`). `registry` suma:

```rust
pub async fn own_folder_count(store: &MemoryStore, project: &str) -> Result<usize, MemoryError>
```

que cuenta lo que devuelve `own_folders_in` (`registry.rs:822-824`), y `locate_project` la llama donde hoy hace el `call`. Se descartó:

- **Un `count(sql)` genérico en el store:** vuelve a abrir la puerta que este cambio cierra.
- **Bajar a core toda la decisión de `folder_shared`:** es lo que core-use-cases anota para esta consulta (`openspec/changes/core-use-cases/exploration.md:18`), con sus propios casos de uso.

**El cierre.** `MemoryRuntime::close` (`crates/clawg-core/src/runtime.rs:355-359`) pasa a devolver `Result<(), MemoryError>`, con el error que da cerrar la conexión del store:

- **El daemon** registra el error con el texto de hoy («closing memory.db failed: {e}; SQLite replays what is left on the next open»), en el `if finished` de `crates/clawg-cli/src/daemon/serve.rs:326-328`, y sigue: [`specs/storage`](../../specs/storage/spec.md), «El daemon cierra `memory.db` antes de salir», pide que un cierre que falla se registre y nada más.
- **`eval_api`** tiene tres llamadores que hoy cuentan con ese log: dos cierran en un camino que ya devuelve otro error (`crates/clawg-core/src/eval_api.rs:756`, `:888`) y uno es un cierre que no devuelve nada (`EvalIngest::close`, `eval_api.rs:898-902`). Los tres registran el error con el mismo texto y siguen como hoy.
- **`rag-eval` y `revision-eval`** cambian `store.conn.clone().close()` por `runtime.close()` con el mismo `map_err(|e| RunError::Close(e.to_string()))`: `release` (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:311`) y `crates/clawg-eval/src/bin/revision-eval/run.rs:490`, `:634`, `:799`.
- **Los tests** que hoy cierran con `rt.close().await;` pasan a `.expect("the store closes")`, como ya hace `crates/clawg-core/src/memory/crowding.rs:479`. `tests_leaving_a_store_open` (`crates/clawg-cli/tests/conventions.rs`) busca `.close().await` en el texto del test, así que sigue reconociéndolos.

`a_close_that_fails_is_returned` (`runtime.rs`) provoca un cierre que falla de verdad: deja sin finalizar un `Statement` (`std::mem::forget` dentro de un `call`), y `sqlite3_close` responde `SQLITE_BUSY`. rusqlite 0.37 cierra con `sqlite3_close` y no con `_v2` (`src/inner_connection.rs:156`), y tokio-rusqlite 0.7.0 devuelve ese fallo como `Error::Close` (`src/lib.rs:347-364`). Usa `MemoryRuntime::new_for_test`, que es en memoria: no queda nada en disco.

## D2. `StorageError` no responde ninguna pregunta

**Decisión abierta 1.** `StorageError` no tiene `is_busy` ni ningún otro predicado público. Nadie lo pediría:

- **Fuera de `clawg-core`** sólo se distinguen `StoreInUse`, `SchemaNewer` y `SchemaDiverged` (`crates/clawg-cli/src/daemon/serve.rs:94`, `crates/clawg-cli/src/restore.rs:34`, `crates/clawg-cli/src/secrets.rs:367`, `crates/clawg-cli/src/status.rs:80`, `:83`), y ninguna es de almacenamiento.
- **Dentro de `clawg-core`** ningún código lee un `ErrorCode` a través de `MemoryError`. El único `sqlite_error_code()` es el del almacén de métricas, sobre su propia conexión y con errores de `rusqlite` directos (`crates/clawg-core/src/metrics/store.rs:579`); `db.rs:486` construye uno, no lo lee.
- **«Ocupada» no le llega a nadie:** otro proceso se queda afuera por el lock del data dir antes de tocar SQLite (`StoreLock::acquire`, `crates/clawg-core/src/memory/db.rs:207`, que da `StoreInUse`), y dentro del proceso la espera la absorbe el busy handler de rusqlite ([`specs/storage`](../../specs/storage/spec.md), «Cada fase del apagado tiene tope», los 5 s de `AFTER_DRAIN`).

Un predicado sin llamador es una abstracción sin uso (`openspec/config.yaml`, reglas de diseño). Se descartó también un `sqlite_code() -> Option<i32>`: le pasaría a quien llama la semántica de SQLite, y storage-ports quiere este error como el de cualquier adaptador (`openspec/changes/storage-ports/exploration.md:25-44`).

**La forma:**

```rust
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StorageError(Fault);

#[derive(Debug, thiserror::Error)]
enum Fault {
    #[error("Migration failed: {0}")]
    Migration(#[source] refinery::Error),
    #[error("Memory DB connection error: {0}")]
    Connection(#[source] tokio_rusqlite::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[source] rusqlite::Error),
    #[error("memory backend channel error: {0}")]
    Channel(String),
}
```

y `MemoryError::Storage(#[from] StorageError)` con `#[error(transparent)]` reemplaza `Migration`, `Connection`, `Sqlite` y `Backend` (`crates/clawg-core/src/memory/error.rs:11`, `:14`, `:17`, `:42`). Cada `#[error]` es el de la variante que reemplaza, palabra por palabra, y `transparent` pasa `Display` y `source()` sin agregar un nivel: el texto y la cadena de causas de cada error quedan como hoy. Lo que sí cambia es el `Debug` (`Storage(StorageError(Sqlite(…)))`). Ninguna línea de producción de `clawg-cli` ni de `clawg-eval` formatea un error con `{:?}`.

**Lo que queda sin cambio, y por qué:**

- **Los `From` de `rusqlite::Error`, `tokio_rusqlite::Error` y `refinery::Error`** siguen en `MemoryError`. Un `impl` de un trait no puede ser `pub(crate)`, y sacarlos obligaría a convertir a mano cada `?` del núcleo sin que nadie de afuera gane nada. Nombran `rusqlite` sólo como entrada: `clawg-cli` tiene `rusqlite` sólo entre sus `[dev-dependencies]` (`crates/clawg-cli/Cargo.toml:99-101`), `clawg-eval` no lo tiene, y SB1 impide que su código de producción lo nombre.
- **`Internal(String)` (`error.rs:30`)** no es de almacenamiento: son tareas que no terminaron (por ejemplo `undo.rs:1217`). Además, tres tests de `clawg-cli` lo construyen (`crates/clawg-cli/src/secrets.rs:496`, `crates/clawg-cli/src/restore.rs:64`, `crates/clawg-cli/src/daemon/serve.rs:550`).

**Lo que se borra:** `VecExtensionRegistration` y `RowDecode` (`error.rs:20`, `:23`), que nada construye, junto con el test que muestra el texto de `RowDecode` (`row_decode_displays_table_name`, `error.rs:170-182`).

**Por dentro** queda una sola pregunta, `pub(crate) fn into_sqlite(self) -> Result<rusqlite::Error, Self>`, para `plan_ids_in` (`crates/clawg-core/src/memory/project.rs:566`). Esa función propaga un fallo de SQL y se traga el rechazo de un registro; hoy lo hace desarmando `RegisterError::Store(MemoryError::Sqlite(e))`.

## D3. Una sola traducción de los errores de la conexión

`Connection::call` de tokio-rusqlite 0.7.0 sólo devuelve `ConnectionClosed` o `Error(E)` (`src/lib.rs:272-282`). `Close` sale únicamente de `close` (`src/lib.rs:347-364`). Por eso el brazo `other =>` de cada traducción escrita a mano no se alcanza nunca. `error.rs` suma la única:

```rust
pub(crate) fn call_failed<E, T>(doing: &str) -> impl FnOnce(tokio_rusqlite::Error<E>) -> T + '_
where
    T: From<E> + From<StorageError>,
```

- **`Error(e)`** se vuelve `T::from(e)`. Con un `rusqlite::Error` es `MemoryError::Storage` en su forma `Sqlite`, con el texto «SQLite error: …» que ya dan las traducciones a mano (por ejemplo `crates/clawg-core/src/memory/admin.rs:894`). Con un error propio de la operación (`MoveError`, `MergeError`, `SubfolderError`) pasa tal cual, como en `relocate.rs:622-627`, `merge.rs:364-369` y `subfolder.rs:191-194`.
- **`ConnectionClosed`** se vuelve `Channel("connection closed during {doing}")`.
- **Una variante que agregue una versión futura** (el enum es `non_exhaustive`) se vuelve `Channel(other.to_string())`.

**Los textos.** El de `Error(e)` queda igual en todos los sitios. El de `ConnectionClosed` se unifica: hoy unos sitios dicen `ConnectionClosed` a secas (los que usan `other.to_string()`, como `relocate.rs:625`) y otros `<contexto>: ConnectionClosed` (`crates/clawg-core/src/memory/synthesis.rs:244`). Ese texto no llega a ninguna respuesta:

- la conexión sólo se cierra en el apagado, después de que paran los servidores (`within_grace` y después `close`, `serve.rs:295-328`) y de que `finish` abortó el mantenimiento (`runtime.rs:373`);
- el bridge no tiene store: habla con el daemon por HTTP.

Ningún test lo fija: el único texto «connection closed during …» que aparece fuera de las traducciones es el que produce `run_migrations` (`db.rs:491`).

`run_migrations` (`db.rs:479-502`), `Retried::connection` (`merge.rs:356`) y cada `map_err` con un `match` a mano pasan a `call_failed`. En `run_migrations` el brazo de conexión cerrada (`db.rs:485-493`) tampoco se alcanza: durante las migraciones la conexión es de `open`, y nadie la cerró.

**Lo que queda separado: el `From<tokio_rusqlite::Error>`.** Lo usan los `map_err(MemoryError::Connection)` (21 en `crates/clawg-core/src`) y los `?` implícitos, y su texto («Memory DB connection error: Error("…")») llega hoy al `error` de un 500: por ejemplo cuando falla `find_project` (`registry.rs:944-961`). Unificarlo con el de `call_failed` cambiaría esas respuestas, así que no es de este cambio. Las dos traducciones viven en `error.rs`, y `only_the_storage_error_matches_the_connection_errors` impide que vuelva a aparecer un `match` sobre `tokio_rusqlite::Error::` en otro archivo de producción del núcleo.

## D4. Las importaciones en curso

**Decisión abierta 2.** Van a un tipo propio de la importación, que el store sigue guardando, y no a `MemoryRuntime`.

`ImportsRunning`, en `crates/clawg-core/src/memory/portability/import.rs`, envuelve el `Arc<Mutex<HashSet<i64>>>` de hoy (`db.rs:182`) con tres métodos: `start(id)`, `end(id)` y `holds(id)`. `Journal` llama a `start` al numerarse (`import.rs:981`) y a `end` en su `Drop` (`import.rs:1022-1028`); `undo_import` pregunta `holds` (`undo.rs:1208`). `MemoryStore` guarda un `pub(crate) imports: ImportsRunning`, que cada clon comparte como hoy.

Se descartó:

- **`MemoryRuntime`.** La garantía es por base: un undo tiene que ver cada importación que corre sobre el mismo `memory.db`. El objeto que tiene esa identidad es el store, porque hay uno por data dir por proceso y un segundo `open` se rechaza (`a_second_runtime_open_on_the_same_dir_is_refused_naming_the_first_holder`, `crates/clawg-core/tests/public_api.rs:111`). En el runtime, `import_parsed`, `import`, `import_foreign` y `undo_import` (`import.rs:2064`, `:2155`, `:2236`; `undo.rs:1207`) tendrían que recibir el conjunto además del store. Eso cambia cuatro firmas públicas que usan los handlers de `api.rs` (`import_memories`, `import_as_job`, `undo_import`) y los tests del núcleo que importan a un `MemoryStore::new_for_test` sin runtime. Y un conjunto que pasa quien llama puede no ser el de esa base: el undo no vería la importación.
- **Dejarlo como está.** `db.rs` seguiría sabiendo cómo se marca una importación, y tres tests seguirían tomando el lock a mano: `an_import_is_numbered_and_journaled` y `a_stop_between_write_batches_returns_the_committed_ledger` (`import.rs`), y `an_unknown_or_running_import_is_refused` (`undo.rs`). Con el tipo, esos tests piden lo mismo con sus métodos.

**`sharing.json` se queda donde está.** Con el mismo criterio, `reader_for` (`db.rs:298-303`) sigue en el store: lee la política del data dir del propio store (`open` la toma de `base_dir`, `db.rs:260`). Sacarla haría que `process_event` (`crates/clawg-core/src/memory/worker.rs:704`) y `eval_api` (`crates/clawg-core/src/eval_api.rs:363`) recibieran un directorio que siempre es el del store. Le toca a core-use-cases, que le da al guardado su punto de entrada.

## D5. La conexión cerrada

- **`conn` pasa a `pub(crate)`** (`db.rs:157`). Lo de adentro no cambia: `AccessTracker::new(store.conn.clone())` (`runtime.rs:189`, `:431`), los cierres de `eval_api` (`eval_api.rs:448`, `:1540`) y los tests del núcleo.
- **`raw_connection(&self) -> &Connection`**, con `#[cfg(any(test, feature = "testing"))]` junto a `new_for_test` (`db.rs:270-271`), para los tests de los otros crates que leen o siembran filas. El nombre dice que se saltea la API. Lo usan:
  - `crates/clawg-cli/tests/api.rs`;
  - los tests de `crates/clawg-cli/src/mcp/server.rs`;
  - en `crates/clawg-cli/src/daemon/api.rs`, los de `count`, `notes_stored` y `an_abandoned_job_sends_nothing_to_the_connection`;
  - el test de `crates/clawg-cli/src/daemon/store_probe.rs`;
  - `crates/clawg-core/tests/public_api.rs` y `shutdown.rs`;
  - el test de `crates/clawg-eval/src/bin/rag-eval/main.rs` que siembra una copia.
- **`clawg-eval`** suma `clawg-core = { workspace = true, features = ["eval", "testing"] }` a unas `[dev-dependencies]` que hoy no tiene. `only_dev_dependencies_turn_on_the_test_stand_ins` (`conventions.rs:176`) lo permite.
- **`memory_runtime_open_creates_the_db_and_returns_a_usable_store`** (`public_api.rs:139`) mira `vec_version()` por la conexión sin estar detrás de `testing`. Esa comprobación pasa a un bloque `#[cfg(feature = "testing")]`. Sigue corriendo en `cargo test --workspace`, porque la dependencia de desarrollo de `clawg-cli` enciende `testing` en `clawg-core` (`crates/clawg-cli/Cargo.toml:88`), de lo que ya depende `shutdown.rs` (`#![cfg(feature = "testing")]`, `shutdown.rs:4`). Además, la misma comprobación corre sin condición dentro del núcleo (`open_creates_memory_db_and_a_usable_store`, `runtime.rs:604`).

Los enfoques B y C de la exploración (una regla de texto sola, o reescribir los tests contra la API) ya están descartados allí.

## D6. `MemoryRuntime` conserva sus campos públicos

**Decisión abierta 3.** No se cierran en este cambio.

- **Tres tests construyen un runtime desde otro crate** con la sintaxis de actualización (`MemoryRuntime { embedder: cell, ..runtime }`): `with_empty_embedder` (`crates/clawg-cli/tests/api.rs:166-167`), `daemon_embedding_with` (tests de `crates/clawg-cli/src/daemon/api.rs`) y `save_memory_is_an_error_when_the_embedder_failed` (tests de `crates/clawg-cli/src/mcp/server.rs`). Con un solo campo privado ninguno compila (E0451), y cada uno pediría un constructor más bajo `testing`.
- **Ningún campo lleva SQLite.** `store` es un `Arc<MemoryStore>`, cuya conexión cierra D5; los demás son `Arc`, canales y un `PathBuf` (`runtime.rs:121-163`).
- **Los métodos que los reemplazarían son puntos de entrada de casos de uso.** `clawg-cli` lee `store`, `base_dir`, `metrics`, `embedder`, `embedder_status` y `tokenizer`. Esos métodos son de core-use-cases, que ya toma `MemoryRuntime::accept` (`runtime.rs:245`) como la entrada del guardado (`openspec/changes/core-use-cases/exploration.md:19`). Cerrarlos ahora sería inventar accesores que ese cambio rehace.

El comentario que lo dice (`runtime.rs:115-120`) sigue siendo cierto y no se toca.

## D7. La fachada de `memory`

- **Cada módulo de `memory` deja de ser `pub`** (`crates/clawg-core/src/memory/mod.rs:26-92`). Los submódulos (`portability`, `chunker`, `project_docs`) quedan como están: con el padre privado, nada de afuera los alcanza.
- **La fachada son los `pub use` de `memory/mod.rs`,** planos en `clawg_core::memory`, de lo que usan `clawg-cli` y `clawg-eval`, en producción o en tests. Es la forma que ya tienen los reexports de hoy (`mod.rs:59-70`), con el motivo que da su comentario (`mod.rs:64-68`). Hoy son unos 250 nombres, y la lista sale de compilar, no de buscar texto.
- **Un nombre que sólo usan los tests de otro crate** se reexporta bajo `#[cfg(feature = "testing")]`.
- **Lo que no cambia:** la visibilidad no cambia `module_path!`, así que los targets de log siguen siendo los de hoy y `RUST_LOG=info,clawg_core::memory::search=debug` (`docs/reference/commands.md:5`) sigue andando. `eval_api` queda dentro de la frontera y no cambia de lugar.

**Los nombres que chocan.** Cinco nombres los exportan dos o tres módulos. La fachada les da un alias con `as`: el tipo conserva su nombre, así que ni `serde` ni `module_path!` cambian. El alias es el que `clawg-cli` ya usa donde renombra al importar:

| Hoy | En la fachada | Por qué |
|---|---|---|
| `registry::Resolution` | `ProjectResolution` | así lo importan `mcp/server.rs`, `mcp/tools.rs`, `projects.rs`, `client.rs` y `daemon/api.rs` |
| `registry::ResolveError` | `ProjectResolveError` | así lo importa `daemon/api.rs` |
| `registry::Refusal` | `ResolveRefusal` | así lo importa `projects.rs` |
| `registry::Candidate` | `ProjectCandidate` | un proyecto que una referencia puede nombrar (`registry.rs:742-744`) |
| `project_delete::Candidate` | `DeleteCandidate` | el proyecto al que vaciar otro (`project_delete.rs:163-165`) |
| `project_delete::KeepReason` | `DeleteKeepReason` | por qué una fila queda borrada al deshacer (`project_delete.rs:125-128`) |
| `project_delete::Refusal` | `DeleteRefusal` | |
| `relocate::Refusal` | `MoveRefusal` | por qué una unidad no se mueve (`relocate.rs:28-31`) |
| `portability::KeepReason` | `UndoKeepReason` | es de `undo` (`portability/undo.rs:36-39`) |

`contradiction::Candidate`, `dispute::Resolution` y `dispute::ResolveError` conservan su nombre: el primero ya está en la raíz (`mod.rs:59`), y `clawg-cli` importa los otros dos sin renombrarlos. Los demás nombres también conservan el suyo, aun los genéricos (`probe`, `restore`, `export`, `import`): en la fachada no chocan, y renombrarlos no es tarea de este cambio.

**El orden.** Primero se cierran los módulos que ningún otro crate nombra; después, los demás, de a uno o de a tandas chicas, con un commit por tanda. En cada tanda, `clawg-cli` y `clawg-eval` pasan a importar desde la raíz.

**Se descartó:**

- **Una fachada encima, sin cerrar nada** (B de la exploración).
- **Dejar los `pub mod` y bajar a `pub(crate)` cada ítem que nadie usa afuera.** No cambia ningún import de afuera, pero la puerta sigue abierta: un `pub` nuevo en un módulo abierto vuelve a ser invisible para `dead_code`; mover un módulo cambia las rutas de afuera, salvo que memory-module-layout reexporte cada módulo; y la lista de lo que sale del crate quedaría repartida en 44 archivos y no en uno.
- **Módulos de fachada con los mismos nombres.** No se puede tener un módulo privado y uno público con el mismo nombre (E0428), y renombrar los de implementación es mover archivos: es memory-module-layout.
- **Una fachada por dominio** (`clawg_core::memory::projects::…`). Decidiría las carpetas de memory-module-layout, que ese cambio tiene abiertas (su decisión 3).

Con la fachada plana, mover un archivo cambia sólo el camino de un `pub use` en `mod.rs`, quede `memory::` como nivel o no.

## D8. El código muerto

- **Lo que cuenta es el conjunto de features de producción:** `clawg-core` sin `testing` ni `eval`. Con `--workspace --all-targets` las features se unifican y un ítem que sólo usa un test o `eval_api` parece vivo. Por eso cada tanda corre `cargo clippy --locked -p clawg-core -- -D warnings`. El CI ya corre `cargo clippy --locked -p clawg-cli -- -D warnings` (`.github/workflows/ci.yml:71`), que compila `clawg-core` con esas features.
- **Cada ítem señalado:**
  - si sólo lo usan los tests del núcleo, va a `#[cfg(test)]`;
  - si lo usan tests de otro crate, va a `#[cfg(any(test, feature = "testing"))]`, como `new_for_test` (`db.rs:270`);
  - si sólo lo usa `eval_api`, va a `#[cfg(feature = "eval")]`, como los 9 ítems que ya lo están;
  - si no lo usa nada, se borra con sus tests, en un commit propio (`refactor(core): drop … nobody calls`).
- **Nunca un `allow` ni un `expect` de `dead_code`:** `no_source_allows_dead_code` (`conventions.rs:64`) lo impide.
- **Si aparece mucho más de lo esperado,** se borra por módulo, en tandas, y no en un solo commit.
- **`verify.md` registra** cuántos ítems se borraron y cuántos quedaron detrás de cada `cfg`.

## D9. Cómo se comprueba que ninguna respuesta cambia

**Qué responden las tres superficies:**

- **El daemon:** estado HTTP y cuerpo JSON, incluido el `error` de un 500, que `ApiError::internal` (`api.rs`) reenvía tal cual desde el `Display` del error.
- **El bridge:** el esquema y la descripción de cada herramienta, y los textos de sus respuestas.
- **La línea de comandos:** stdout, stderr y código de salida. Varios mensajes salen del `Display` de `MemoryError` (`restore::explain`, `crates/clawg-cli/src/restore.rs:33-41`; `secrets.rs:366-367`; `status.rs:77-84`).

**Lo que el cambio podría moverles:** el texto de un error (D2, D3), un nombre de tipo (D7: los alias no renombran nada, y `serde` serializa campos, no nombres de tipo), el conteo de `locate_project` (D1) y el cierre (D1).

**Las comprobaciones:**

1. **Fijaciones.** Se escriben primero, se corren sobre el código sin cambiar y pasan. Después no se tocan, salvo el cambio de `.conn` por `.raw_connection()` en su tarea. Comparan contra texto literal, el que da hoy el código:
   - `a_storage_failure_reads_as_before` (`crates/clawg-core/src/memory/error.rs`): el `Display` y el `source()` de un `MemoryError` que llega por `?` desde un `rusqlite::Error` y desde un `tokio_rusqlite::Error`. Usa sólo los `From`, así que compila antes y después.
   - `a_failed_query_answers_with_the_sqlite_text` (`crates/clawg-cli/tests/api.rs`): rompe a propósito la tabla `memory` con SQL crudo y pide `GET /v1/memories` (`list_memories`). Fija el 500 y su `error` exacto, que hoy sale de la traducción a mano de `list_memories_core` (`admin.rs:894`) y después de `call_failed`.
   - `a_failed_lookup_answers_with_the_connection_text` (`crates/clawg-cli/tests/api.rs`): rompe la tabla `project` y pide `POST /v1/projects/locate` (`locate_project`, por `find_project`). Fija el 500 y su `error` exacto, que sale del `From<tokio_rusqlite::Error>`.
2. **Los contratos del agente.** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin `CLAWG_BLESS`, y esto sale vacío:
   ```bash
   git diff --exit-code origin/develop -- crates/clawg-cli/src/mcp/agent_contract.json crates/clawg-cli/src/mcp/agent_contract.es.json
   ```
3. **Ninguna aserción cambia.** Sobre toda la serie, este comando lista sólo las aserciones de los tests nuevos que nombra la spec y las de los tests que se van con el código muerto (D2, D8), cada uno nombrado en `verify.md`:
   ```bash
   git diff origin/develop..HEAD -U0 -- crates | rg '^[-+]\s*(assert|prop_assert)'
   ```
   El resto de lo que cambia en los tests es mecánico: imports, `.conn` por `.raw_connection()`, `.close().await` con `.expect(…)`, y el lock de `importing` por los métodos de `ImportsRunning`.
4. **La suite.** `crates/clawg-cli/tests/api.rs` (daemon), los tests de `crates/clawg-cli/src/mcp/` (bridge) y `crates/clawg-cli/tests/binary.rs` (línea de comandos) pasan sin otros cambios que los del punto 3.

**El único texto de respuesta que puede cambiar** es el `error` del 500 de `locate_project` cuando SQLite falla justo en el conteo de carpetas. Hoy es el `Display` de tokio-rusqlite (`Error("…")`, `src/lib.rs:134`); después, «Memory DB connection error: Error("…")», que es lo que el mismo handler ya responde cuando falla `find_project`. Ningún test puede provocarlo solo: el handler lee `project_path` antes, en `register_root`, y una tabla rota hace fallar esa lectura primero. Queda escrito en `verify.md`.

**Fuera de las respuestas cambian dos textos:** la línea de log de un cierre que falla y el texto de `RunError::Close` en las herramientas de eval. Los dos ganan el prefijo «Memory DB connection error: », porque el error pasa por `MemoryError`. Ningún test ni contrato los fija, que es lo que la exploración pone como condición (`exploration.md`, «No-alcance»).

## D10. Las reglas en `conventions.rs`

- **`no_production_code_outside_the_core_reaches_sqlite`:** ninguna línea de producción de `crates/clawg-cli/src` ni de `crates/clawg-eval/src` nombra `rusqlite` ni `.conn`. `.conn` cuenta sólo si no le sigue un carácter de identificador; si no, `.connect(` también contaría.
- **El fin del código de producción.** `production_lines_naming` (`conventions.rs:116-128`) termina el código de producción en el primer `#[cfg(test)]` seguido de una línea que empieza con `mod `. Eso incluye una declaración sin cuerpo, como `#[cfg(test)] mod test_temp;` (`crates/clawg-eval/src/bin/rag-eval/main.rs:15-16`), que esconde de la regla todo el `main.rs` de `rag-eval`. Pasa a ser `#[cfg(test)]` seguido de `mod <nombre> {`; lo fija el puro `the_production_code_runs_past_a_test_module_declared_without_a_body`. `no_production_code_names_the_retired_tables`, que usa el mismo helper, se vuelve más estricta en los cuatro archivos con una declaración así (`crates/clawg-cli/src/update/mod.rs:13-14`, `crates/clawg-core/src/memory/mod.rs`, `crates/clawg-core/src/lib.rs`, el `main.rs` de `rag-eval`), y sigue en verde: ninguno nombra una tabla retirada.
- **`only_the_storage_error_matches_the_connection_errors`:** ninguna línea de producción de `crates/clawg-core/src` fuera de `memory/error.rs` nombra `tokio_rusqlite::Error::`.
- **`memory_declares_no_public_module`:** ninguna línea de `crates/clawg-core/src/memory/mod.rs` empieza con `pub mod`.

Un archivo que sólo existe para los tests y no está bajo un `mod tests { … }` (como `test_temp.rs`) cuenta como producción para estas reglas. Hoy ninguno nombra lo que prohíben. Si alguno lo hace, el test nombra el archivo y se decide ahí.
