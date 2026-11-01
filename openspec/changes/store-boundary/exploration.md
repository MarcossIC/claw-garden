# store-boundary — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | conviene después de retire-legacy-kinds | exploración (borrador) |

**Tesis:** `MemoryStore` deja la conexión a la vista, su error expone los tipos de `rusqlite`, y casi todos los módulos de `memory` son públicos. Por eso mover o abstraer cualquier cosa toca también a `clawg-cli` y `clawg-eval`, y por eso el compilador no ve código muerto. Este cambio cierra esa puerta. Es lo que hace baratos a los cambios que siguen.

## Problema (verificado contra el código)

**La conexión es pública.**

- `pub conn: Connection` (`crates/clawg-core/src/memory/db.rs:150`).
- `crates/clawg-cli/src/daemon/api.rs:2920-2928` la usa en producción con SQL crudo, para contar las carpetas de un proyecto.
- `clawg-eval` la usa para cerrar: `app.runtime.store.conn.clone().close()` (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:310`; `crates/clawg-eval/src/bin/revision-eval/run.rs:489`, `:633`, `:798`). Existe `MemoryRuntime::close` (`crates/clawg-core/src/runtime.rs:355`).
- Fuera de eso, el límite ya está bastante bien: `clawg-cli` no menciona `rusqlite::` en producción.
- `.conn` aparece 755 veces en 50 archivos; unas 620 están en tests. Las de `clawg-core` siguen andando con `pub(crate)`. Las de afuera son las de `crates/clawg-cli/tests/api.rs`, `mcp/server.rs`, `daemon/api.rs` y `daemon/store_probe.rs`, más `crates/clawg-core/tests/`.

**El error expone la implementación.**

- `MemoryError` tiene `Migration(refinery::Error)`, `Connection(tokio_rusqlite::Error)`, `Sqlite(rusqlite::Error)` y `RowDecode { source: rusqlite::Error }` (`crates/clawg-core/src/memory/error.rs:11`, `:14`, `:17`, `:26`), más `Backend(String)` e `Internal(String)` (`:30`, `:42`): varias formas de decir que falló la base.
- Fuera de `clawg-core` sólo se distinguen `StoreInUse`, `SchemaNewer` y `SchemaDiverged` (`crates/clawg-cli/src/daemon/serve.rs:94`, `crates/clawg-cli/src/restore.rs:25`, `crates/clawg-cli/src/secrets.rs:344`, `crates/clawg-cli/src/status.rs:57`, `:60`). Lo demás sólo se muestra.
- La traducción de `tokio_rusqlite::Error` se repite a mano: `search.rs:683-689`, `relocate.rs:617-622`, `merge.rs:375-380`, `subfolder.rs:190-193`.

**Todo es público.**

- 44 de los 51 módulos de `memory` son `pub mod` (`crates/clawg-core/src/memory/mod.rs:24-89`), y `clawg-core` tiene 447 `pub fn`. Los otros dos crates usan del orden de 73 rutas, contadas por sus dos primeros segmentos.
- `pub` apaga `dead_code`. Dos casos conocidos: `insert_episodic_trajectory` (`store_insert.rs:660`), sin llamadores de producción, y `OllamaEmbedder` y `OpenAIEmbedder`, en el backlog de `openspec/README.md`.
- `MemoryRuntime` declara «Every field is deliberately public» (`runtime.rs:115-120`).

**`MemoryStore` hace de más.** Junta conexión, lock, latencia, métricas, ruta de `sharing.json` y el conjunto de imports en curso (`db.rs:149-176`). `reader_for` lee un archivo desde el store (`db.rs:291-296`), y `importing` es estado de un caso de uso (`crates/clawg-core/src/memory/portability/undo.rs:1218`).

## Enfoques posibles

### La conexión

| Enfoque | A favor | En contra |
|---|---|---|
| **A. `pub(crate) conn`, y un acceso crudo bajo la feature `testing` para los tests de afuera** | Producción no puede llegar. Los tests que comprueban filas siguen funcionando. `testing` ya existe y sólo la habilitan las `[dev-dependencies]` | Un método más, sólo para tests |
| B. Dejarla `pub` y prohibirla con un test de convención | Sin cambio de firmas | Es una regla de texto donde el compilador puede imponerla |
| C. Pasar todos los tests a la API pública | Tests independientes de SQLite | Reescribir muchos tests, y varios comprueban justamente filas. Se discute en storage-ports |

**Recomendado: A.**

### El error

| Enfoque | A favor | En contra |
|---|---|---|
| **A. `StorageError` opaco en `MemoryError`, conservando las variantes que alguien distingue** | Sale `rusqlite` de la firma pública. Una sola traducción | Quien quiera el código de SQLite tiene que pedirlo con un método |
| B. Dejar el enum y documentarlo | Sin cambio | Un puerto no podría usarlo |

**Recomendado: A.** Por dentro, los helpers siguen devolviendo `rusqlite::Result`.

### La superficie pública

| Enfoque | A favor | En contra |
|---|---|---|
| **A. `pub(crate) mod` y `pub use` de lo que usan los otros crates, módulo por módulo** | El compilador muestra el código muerto, y mover un módulo deja de tocar a los otros crates | Cada módulo es una tanda de imports |
| B. Una fachada nueva por encima, sin cerrar nada | Rápido | Conviven dos caminos, y `dead_code` sigue sin ver nada |

**Recomendado: A.** Lo que aparezca como muerto se borra en commits aparte, salvo que algo diga por qué sigue.

## Alcance candidato

1. Mover el SQL de `api.rs:2920` a una función de core, y usar `MemoryRuntime::close` en `clawg-eval`.
2. `conn` a `pub(crate)`, con el acceso crudo para tests.
3. `StorageError` y una sola traducción de los errores de `tokio_rusqlite`.
4. Cerrar `memory/mod.rs` módulo por módulo y borrar lo que el compilador señale.
5. Sacar `importing` y la lectura de `sharing.json` de `MemoryStore`, si el diseño lo confirma.
6. Fijar en `crates/clawg-cli/tests/conventions.rs` que el código de producción de `clawg-cli` y `clawg-eval` no mencione `rusqlite` ni `.conn`.

### No-alcance

- Traits, puertos o capas de lectura y escritura.
- Mover archivos: es memory-module-layout.
- Cambiar conducta, mensajes o códigos de salida. El texto de un error de almacenamiento que hoy llega a un log puede cambiar sólo si ningún test ni contrato lo fija.

## Decisiones abiertas

1. Qué pregunta permite `StorageError`: «está ocupada» (`is_busy`) parece hacer falta y el resto no; se decide viendo quién mira `ErrorCode` hoy.
2. Dónde va `importing`: en `MemoryRuntime` o en un tipo propio del import.
3. Si `MemoryRuntime` también cierra sus campos con métodos o los deja públicos. Sus campos son `Arc` y canales, así que el daño es menor.

## Cómo se mide

- **Test en rojo:** la regla de `conventions.rs`, que hoy falla por `api.rs:2921` y por `clawg-eval`.
- La suite pasa sin cambiar lo que comprueba.
- `dead_code` encuentra lo que antes no veía; sale el conteo de lo borrado.

## Riesgos

- Cerrar módulos puede mostrar mucho más código muerto del esperado. Si es así, se borra por tandas y no todo en un commit.
- `clawg_core::eval_api` también usa internos: queda dentro de la frontera y no cambia de lugar.

## Interacciones

- **retire-legacy-kinds:** si va primero, hay menos que cerrar.
- **memory-module-layout:** este cambio es su condición: con la fachada, un movimiento no toca a los otros crates.
- **storage-ports:** `StorageError` es el error de esos puertos.
