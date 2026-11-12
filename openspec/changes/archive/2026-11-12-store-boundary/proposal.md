# store-boundary — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | — (retire-legacy-kinds, que convenía antes, ya está archivado) | ver [`state.yaml`](state.yaml) |

## Problema

`clawg-cli` y `clawg-eval` pueden llegar a la conexión de SQLite, a los errores de `rusqlite` y a casi cualquier función de `clawg_core::memory`. Por eso mover o abstraer algo del núcleo los toca también, y por eso el compilador no ve el código muerto. Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código). Las líneas que siguen son las de `develop` al escribir esto:

1. **La conexión es pública.** `pub conn: Connection` (`crates/clawg-core/src/memory/db.rs:157`). En producción la usan `locate_project` (`crates/clawg-cli/src/daemon/api.rs`), con SQL crudo para contar las carpetas de un proyecto, y `clawg-eval` para cerrar el store: `release` (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:311`) y `revision-eval` (`crates/clawg-eval/src/bin/revision-eval/run.rs:490`, `:634`, `:799`). Lo hacen a mano porque `MemoryRuntime::close` (`crates/clawg-core/src/runtime.rs:355`) registra el error y no lo devuelve. Fuera de `crates/clawg-core/src`, `.conn` aparece 52 veces: esas 5 de producción y 47 de tests.
2. **El error expone la implementación.** `MemoryError` tiene `Migration(refinery::Error)`, `Connection(tokio_rusqlite::Error)`, `Sqlite(rusqlite::Error)` y `RowDecode { source: rusqlite::Error }` (`crates/clawg-core/src/memory/error.rs:11`, `:14`, `:17`, `:23-27`), más `Backend(String)` (`:42`). `RowDecode` y `VecExtensionRegistration` (`:20`) no los construye nadie fuera de los tests de `error.rs`. Los otros crates sólo distinguen `StoreInUse`, `SchemaNewer` y `SchemaDiverged` (`crates/clawg-cli/src/daemon/serve.rs:94`, `crates/clawg-cli/src/restore.rs:34`, `crates/clawg-cli/src/secrets.rs:367`, `crates/clawg-cli/src/status.rs:80`, `:83`). Hay 30 traducciones a mano de un `tokio_rusqlite::Error` en 20 archivos de `memory`, con tres redacciones distintas para la misma conexión cerrada.
3. **Todo es público.** 44 de los 51 módulos de `memory` son `pub mod` (`crates/clawg-core/src/memory/mod.rs:26-90`), y `clawg-core` tiene 447 `pub fn`. Los otros crates nombran 35 de esos 44 módulos. Un ítem `pub` no da `dead_code`.
4. **`MemoryStore` hace de más.** Además de la conexión, guarda el directorio de donde lee `sharing.json` (`db.rs:179`, `reader_for` en `:298-303`) y los imports que el proceso está escribiendo (`importing`, `db.rs:182`), que sólo leen la importación y su deshechura (`crates/clawg-core/src/memory/portability/import.rs:967`, `crates/clawg-core/src/memory/portability/undo.rs:1208`).

## Enfoque

Los tres **A** de la exploración, con las tres decisiones abiertas tomadas en el [diseño](design.md):

1. **`StorageError` no responde ninguna pregunta** ([D5](design.md#d5-storageerror)). Nadie mira hoy el código de SQLite de un `MemoryError`; un `is_busy` no tendría llamador.
2. **`importing` va a `MemoryRuntime`**, como un tipo propio, `RunningImports`, que la importación y la deshechura reciben ([D7](design.md#d7-los-imports-en-curso)).
3. **Los campos de `MemoryRuntime` siguen públicos** ([D9](design.md#d9-los-campos-de-memoryruntime)). Siete construcciones desde otro crate usan `..runtime`, y eso exige que todos sean visibles.

El punto 5 del alcance candidato también se confirma: `sharing.json` pasa al worker de ingest ([D8](design.md#d8-sharingjson-sale-del-store)).

## Alcance

1. **Conexión.** `conn` pasa a `pub(crate)`. Los tests de otro crate la alcanzan con `MemoryStore::conn_for_test()`, bajo `cfg(any(test, feature = "testing"))`.
2. **El SQL de `locate_project`** pasa a `registry::own_folder_count`.
3. **`MemoryRuntime::close` falible:** devuelve `Result<(), StorageError>`. `clawg serve` registra el fallo con la misma línea de hoy. `rag-eval` y `revision-eval` lo usan y terminan con `RunError::Close` cuando falla.
4. **`StorageError`.** Es la única variante de almacenamiento de `MemoryError` (`Storage`), y la causa de `BackupError::Snapshot`, de `DocIndexError::Store` y de `ExportError`, que pasa de `Sqlite` a `Storage`. Ningún texto cambia.
5. **Una sola traducción** de un `call` fallido, en `memory/error.rs`.
6. **`MemoryStore` sin `importing` ni `sharing.json`.**
7. **La superficie de `memory`.** Los 9 módulos que ningún otro crate nombra pasan a `pub(crate)`. En los otros 35, cada ítem que ningún otro crate usa pasa a `pub(crate)`. Lo que `dead_code` señale se borra en commits aparte.
8. **Convenciones fijadas en `crates/clawg-cli/tests/conventions.rs`:**
   - el código de producción de `clawg-cli` y `clawg-eval` no menciona `rusqlite` ni `.conn`;
   - fuera de `memory/error.rs`, ningún código del núcleo traduce a mano un error de la conexión;
   - cada `pub mod` de `memory` lo nombra otro crate.
9. **Docs:** las dos líneas de `docs/reference/` que nombran `MemoryStore::importing` y `MemoryStore::reader_for`.

## Fuera de alcance

Lo que la exploración pone en [«No-alcance»](exploration.md#no-alcance):

- Traits, puertos o capas de lectura y escritura (storage-ports, storage-read-side, storage-write-side).
- Mover archivos (memory-module-layout). Este cambio no mueve ninguno, y ninguna ruta de los otros crates cambia ([D10](design.md#d10-la-superficie-de-memory)).
- Cambiar conducta, mensajes o códigos de salida. Acá es más estricto que en la exploración: tampoco cambia el texto de un error de almacenamiento que llega a una respuesta ([D11](design.md#d11-cómo-se-comprueba-que-nada-de-lo-que-ve-un-cliente-cambia)).

Tampoco entra:

- `MetricsError` (`crates/clawg-core/src/metrics/store.rs:31-34`), que sigue con `Sqlite(rusqlite::Error)`. Es el error de `metrics.db`, otra base con su propia conexión (`metrics/store.rs:153`, `:164`), y `clawg-cli` sólo lo muestra (`crates/clawg-cli/src/metrics.rs:84`).
- Los módulos de `clawg-core` fuera de `memory` (`metrics`, `models`, `paths`, `preferences`, `runtime`, …) y `eval_api`, que queda dentro de la frontera.

Y lo que esta tarea deja afuera:

- el código, que va en otra tarea, después de que entre a `develop` el otro cambio que toca `api.rs`;
- subir la versión y archivar;
- `openspec/README.md`, `openspec/ROADMAP.md` y las specs principales, que se consolidan al archivar.

## Datos guardados

Ninguno. No cambia el esquema, ningún identificador guardado ni ningún archivo del directorio de datos. `sharing.json` se sigue leyendo en cada save, del mismo directorio. No hace falta migración ni vuelta atrás: una versión anterior abre la misma base.

## Cómo se mide

El test en rojo de la [exploración](exploration.md#cómo-se-mide) y los que agrega este cambio, con nombre en la [spec](spec.md):

- **`no_production_code_outside_core_reaches_the_connection`** (`conventions.rs`). Hoy falla por `locate_project`, `revision-eval/run.rs` y `rag-eval/agent_view/runner.rs`.
- **`no_memory_code_translates_a_connection_error_by_hand`** (`conventions.rs`). Hoy falla por las 30 traducciones.
- **`every_public_memory_module_is_named_outside_core`** (`conventions.rs`). Hoy falla por los 9 módulos que nadie nombra.
- Desde otro crate (`crates/clawg-core/tests/public_api.rs` y el doctest de `MemoryStore`):
  - `conn` no se alcanza;
  - una falla de almacenamiento llega como `MemoryError::Storage`;
  - `MemoryError` tiene sólo las variantes de la spec;
  - `close` dice si cerró.

  Hoy ninguno de estos compila, o compila cuando no debería.

Además:

- **La suite pasa sin cambiar lo que comprueba.** Los tests que ya existen sólo cambian en la forma de la llamada (`conn_for_test()`, un `&RunningImports` más, un `.expect` sobre `close`). Ninguna aserción cambia, salvo las que nombran una variante que se va (D11).
- **Ningún texto cambia.** Lo fijan tests de caracterización que se escriben primero y pasan sobre el código de hoy: el texto de cada falla de almacenamiento ([R6](spec.md#r6-ningún-texto-cambia)) y el `detail` del 409 `folder_shared`.
- **Los contratos del agente pasan sin bendecir.**
- **`dead_code` encuentra lo que antes no veía.** `verify.md` registra cuántos ítems se borraron y cuántos pasaron a `cfg(test)`, `cfg(feature = "eval")` o `cfg(any(test, feature = "testing"))`, por módulo.
- **No hay métrica de `rag-eval`:** el cambio no toca la recuperación.
