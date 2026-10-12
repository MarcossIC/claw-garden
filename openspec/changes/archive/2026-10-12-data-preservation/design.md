# data-preservation — Diseño (partes A + F)

Hay design porque A tiene una decisión real con alternativas: qué hacer con los eventos que el apagado no alcanza a ingerir. F es mecánico y sólo lleva notas.

## D1. Eventos que no se pueden ingerir al apagar

El caso que importa es el embedder sin cargar: el modelo se descarga o se carga en segundo plano (`serve.rs:82-92`), y el worker retiene los eventos mientras tanto (`worker.rs:191-206`). Un apagado en esa ventana no tiene cómo embeberlos.

| Opción | A favor | En contra |
|---|---|---|
| **a. Esperar al embedder** sin límite | No agrega nada | Una descarga de 470 MB puede tardar minutos. `clawg stop` vence a los 15 s y el instalador sigue igual. Si la descarga falló, no termina nunca |
| **b. Insertar sin vector** y re-embeber al arrancar | Queda en la base | La memoria, el vector y el FTS se insertan juntos en una transacción (`store_insert.rs:329-372`). Una fila sin vector necesita un estado nuevo, cambios en el esquema o en las consultas, y un re-embed que hoy no tiene llamador (`maintenance.rs:442`). Es la parte D entera |
| **c. Volcado a archivo** (`ingest-pending.jsonl`) y reinyección al arrancar | Sin esquema nuevo. `MemoryEvent` ya es `Serialize`/`Deserialize` (`events.rs:12`). El próximo arranque pasa por el mismo camino de siempre: chunker, dedup, contradicciones, insert | Un archivo más en el directorio de datos. No cubre un kill |
| **d. Outbox en una tabla** | Cubre también el kill | Es la opción C de ingest-durability, que la descarta por ahora: migración, estados y más complejidad que el problema |

**Decisión: c.** Es lo mínimo que cumple R1 sin tocar el esquema. Si ingest-durability elige más adelante el outbox (d), el volcado se reemplaza por esa tabla y R1 sigue valiendo.

El volcado está en texto plano, igual que la base. No agrega exposición: es el mismo contenido, en el mismo directorio y con los mismos permisos.

## D2. Quién orquesta el apagado

Hoy `MemoryRuntime` devuelve futuros y el host decide dónde lanzarlos (`runtime.rs:111-143`). Se mantiene esa forma:

- `MemoryRuntime::ingest_worker(rx)` sigue devolviendo un futuro, que ahora recibe también la señal de apagado como `tokio::sync::watch::Receiver<bool>`, igual que el shutdown de axum (`serve.rs:120`). No se usa `CancellationToken` porque `tokio-util` no está entre las dependencias de `clawg-core`.
- El worker deja de esperar sólo a que se cierren todos los `Sender` (`worker.rs:118`, `:137`). Cuando el flag se levanta, llama `rx.close()`, que impide nuevos envíos pero deja leer lo que ya está en el buffer. Esto evita depender de que se suelten todas las copias de `memory_tx` (`AppCtx` vive dentro del router).
- Un método nuevo, `MemoryRuntime::shutdown(self, handles)`, hace la secuencia: señal al worker, espera con `DRAIN_DEADLINE`, volcado del resto, flush del tracker, abort del mantenimiento y checkpoint. `serve.rs` guarda los `JoinHandle` y lo llama después de `axum::serve`. Así la secuencia vive en `clawg-core` y se testea con `new_for_test`, y `serve.rs` sigue sin nada que valga la pena testear (`serve.rs:1-8`).

**Alternativa descartada:** hacer el apagado desde `Drop` de `MemoryRuntime`. `Drop` no puede ser async, y el drenaje necesita await.

### Qué se vuelca cuando vence el plazo

El worker procesa evento por evento. Al vencer el plazo:

- El evento en curso **termina**: cortar un embed o un insert no ahorra nada que importe, y un evento a medio insertar (algunos chunks sí, otros no) se reinyectaría duplicando chunks. El insert de cada chunk ya es atómico (`store_insert.rs:329-372`).
- Los eventos que no empezaron se vuelcan en orden.

La decisión "¿sigo drenando o vuelco?" es una función pura de (instante actual, plazo, embedder listo). Se testea sin runtime.

### `DRAIN_DEADLINE`

`STOP_TIMEOUT` es 15 s (`stop.rs:15`). `DRAIN_DEADLINE` = 10 s deja 5 s para el evento en curso, el volcado, el flush y el checkpoint. Vive en `clawg-core`, y el test de R6 está en `clawg-cli`, que ve las dos constantes. Es un valor inicial elegido por esa relación, no medido; por eso lo fija un test.

## D3. Reinyección

- Al arrancar, el worker lee el volcado **antes** del primer `rx.recv()`. Las líneas se parsean con la función pura de D1; las que fallan se mueven a `ingest-pending.rejected.jsonl`.
- El volcado se borra recién cuando se procesaron todas sus líneas. Si el proceso muere antes, el próximo arranque las reinyecta otra vez. Un evento repetido pasa por el gate de dedup (`absorb_duplicate`, `worker.rs:225`) y refuerza la fila existente en vez de crear otra. El costo es un `access_count` de más, contra la pérdida del save.
- Un apagado durante la reinyección escribe primero las líneas no procesadas del volcado anterior y después lo encolado (R2).

**Alternativa descartada:** cargar el volcado en el canal desde `MemoryRuntime::open`. Mezcla I/O de archivos en la apertura del store, y el canal no distingue "viene del volcado" para saber cuándo borrarlo.

## D4. Flush final del `AccessTracker`

Hoy el tracker lanza su worker dentro de `new` y no guarda el handle (`access.rs:49-89`).

- El tracker guarda el `JoinHandle` y gana un `async fn shutdown(&self)`. Envía un mensaje de cierre por el mismo canal, y el loop hace el flush del buffer y termina; después espera el handle.
- El canal pasa de `Vec<i64>` a un enum privado (`Hit(Vec<i64>)` / `Close`). `record` sigue igual hacia afuera.
- Cuando se sueltan todos los `Sender` (`None` en `access.rs:68`), el loop también hace flush antes de salir. Así una ruta que no llama `shutdown` igual pierde menos.

`inert()` (`access.rs:95`) no lanza worker: su `shutdown` no hace nada.

## D5. Checkpoint

`PRAGMA wal_checkpoint(TRUNCATE)` sobre `store.conn`, después del flush del tracker, que es la última escritura. No cierra la conexión: el lock se suelta cuando se suelta el store (`store_lock.rs:192`), como hoy. Si el checkpoint falla (por ejemplo, un lector externo con la base abierta), se registra y se sigue: los datos ya están commiteados en el WAL.

## D6. Notas de F

- **`data_dir`:** la decisión pasa a `fn data_dir_in(home: Option<PathBuf>, profile) -> Result<PathBuf, PathsError>`, pura, y `data_dir` la llama con `dirs::home_dir()`. Mismo patrón que `Profile::resolve`. El error se agrega a los tipos de `paths.rs` con `thiserror`.
- **Migraciones agrupadas:** la configuración del runner se separa en `fn configured(runner: Runner) -> Runner` (hoy sólo `set_grouped(true)`). La usan `run_migrations` y el test de R8, que arma un runner con `runner().get_migrations()` más una V9 rota. Los tests de `db.rs` que llaman `runner().run` directamente (`db.rs:475`, `:627`, …) prueban el SQL de cada migración y quedan como están.
- **Checksums:** `runner().get_migrations()` da versión, nombre y checksum. El test compara contra una tabla literal. Cuando se agregue V9, el mismo commit agrega su fila.
- **`workspace.md`:** escribe a `workspace.md.tmp` en el mismo directorio, y `tokio::fs::rename`. En Windows, `rename` reemplaza el destino si existe (`MoveFileExW` con `REPLACE_EXISTING`, que es lo que usa `std::fs::rename`). El test de R10 provoca el fallo creando un directorio con el nombre del temporal; si la escritura falla, el temporal se borra.

## Sin dependencias nuevas

Todo se hace con `tokio` (watch, `JoinHandle`, `timeout`), `serde_json` y `refinery`, que ya están.
