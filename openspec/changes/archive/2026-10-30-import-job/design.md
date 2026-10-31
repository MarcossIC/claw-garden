# import-job — Diseño

Cómo se construye lo que fijan la [propuesta](proposal.md) (decisiones 1 a 16) y la [spec](spec.md) (IJ1 a IJ19). Las citas son del código del 2026-10-29; las de dependencias, de sus fuentes en el registro local de cargo (`tokio-rusqlite-0.7.0`, `axum-0.8.9`). Nada de esto toca `memory.db`, su esquema, los ids, los estados ni el contrato de embedding: el trabajo vive en la memoria del proceso.

## Frente a la spec

La spec y el diseño se escribieron en paralelo. Donde el diseño fija algo que la spec dejó abierto, o no puede seguirla al pie de la letra, lo dice esta tabla.

| Punto | Spec | Diseño | Por qué |
|---|---|---|---|
| Un campo desconocido (IJ1) | `400`, «como `POST /v1/import/undo`» | `400`, leyendo el cuerpo como bytes (D6) | `deny_unknown_fields` con el extractor `Json` da `422`, no `400`: es `JsonDataError` (`axum-0.8.9/src/extract/rejection.rs:11-18`), y una prueba lo fija hoy para otra ruta estricta (`a_misspelt_synthesis_field_is_refused`, `crates/clawg-cli/tests/api.rs:2016-2020`). La decisión 15 se cumple, pero no «como undo» |
| El `409 import_running` frente a los `400` (IJ3, señalado) | sólo «antes de abrir el archivo» | después de todo lo que sólo lee el cuerpo y de resolver `project_map`; antes de tocar el archivo (D6) | Un cuerpo malo se entera de su falla aunque haya un trabajo, y dos `POST` nunca leen archivos a la vez |
| Un `POST` durante el apagado | no lo dice | `503 shutting_down`, `retryable: true` (D5, D7) | Un trabajo aceptado después de que empezó la espera correría contra un store cerrado |
| `report` en una parada antes de la primera escritura (IJ4, señalado) | sin `report` hasta que el diseño decida | sin `report`: `stopped_in` y los contadores; el CLI dice que no se escribió nada (D6, D8) | Un informe armado antes del primer lote nombraría proyectos y sugerencias que no se registraron (D2) |
| Valores de `stopping` (IJ4, señalado) | que nombre la causa | `cancel` o `shutdown`; la primera causa gana (D1) | — |
| `DELETE` en `snapshot` o `links`; abortar en el apagado (IJ6, IJ18, señalado) | `links` termina `done`; abortar marca `interrupted` | `snapshot`: `cancelled` con el snapshot hecho; `links`: `done`; un aborto: `interrupted` con `abandoned: true` (D7) | El estado final dice lo que se sabe; `abandoned` nombra que la llamada en vuelo pudo confirmarse |
| Una parada durante el último lote de escritura | no lo dice | corre `links` y el trabajo termina `done` (D3) | Faltan ~1 s para una importación entera; cortar ahí dejaría una parcial que hay que deshacer |
| `DELETE` de un trabajo terminado (IJ6) | su estado | `200` con su estado; `202` sólo para uno activo (D6) | Al terminado no se le pidió nada |
| `report.rows` en una parada en `writing` (IJ5) | «cuenta las filas de los lotes confirmados» | `rows` sigue siendo las filas del archivo; lo que cuentan los lotes confirmados son `inserted`, `present`, `present_elsewhere` y `merged` (D2) | `render` imprime «N de `rows` memorias del archivo» |
| Detectar un daemon anterior (IJ16, señalado) | «`404` sin el cuerpo de error» | `404` cuyo cuerpo no es un objeto JSON con `error`, y la versión de `/health` sólo para la frase (D8) | Verificado: ese `404` sale del fallback por defecto de axum, sin cuerpo (D8) |
| Fases en stderr (IJ15) | «las fases en el orden de `Step`» | las que el sondeo ve, en ese orden | Un paso más corto que el sondeo de 1 s (`load`, `decide`) puede no aparecer |
| `a_client_reads_a_health_with_an_import_block` (IJ12) | propuesta | guarda que ya pasa | `Health` ignora los campos que no conoce (`client.rs:134-159`). El cliente no gana el campo: no lo lee |
| `a_clawg_file_is_parsed_once_for_a_job` (IJ1) | con un contador de parseos | sin contador: `import_parsed` recibe `Parsed` y no tiene texto que parsear (D2) | El tipo lo garantiza. La prueba compara el informe con el de `import` sobre el texto |
| Pruebas de `import.rs` (IJ11) | «siguen verdes sin tocarse» | las de `import` e `import_foreign` no se tocan; los ayudantes `file_of` y `notes_of` pasan el control (D2) | Llaman a funciones privadas que ganan un argumento. Ninguna aserción cambia |
| Dónde viven las pruebas de parada determinista (IJ5, IJ6, IJ7, IJ18) | `crates/clawg-cli/tests/api.rs` y el registro | las que necesitan un gancho de paso, en `mod tests` de `crates/clawg-cli/src/daemon/api.rs` (D9) | Una prueba de integración no puede pasarle un control propio a un trabajo |
| `shutdown_stops_a_job_in_writing_between_whole_notes` (IJ18) | tiempo pausado | tiempo real (D9) | Con el reloj pausado, `HTTP_GRACE` vence mientras el lote está en el hilo de la conexión, que el runtime no ve |

## D1. El control: señal de parada y progreso

Hoy la importación no tiene progreso, y su única parada es `StopOnDrop` sobre un `AtomicBool` que `decide` mira por lote (`crates/clawg-core/src/memory/portability/import.rs:1566-1587`, `:2229-2235`). El tipo nuevo es del núcleo, porque lo reciben las funciones de importación. El registro de trabajos y el vocabulario HTTP son del daemon (D5, D6).

```rust
// crates/clawg-core/src/memory/portability/control.rs
/// What the caller of an import can do while it runs: ask it to stop, and
/// read how far it got.
#[derive(Clone, Default)]
pub struct ImportControl(Arc<Shared>);

struct Shared {
    stop: watch::Sender<Option<StopCause>>,
    progress: watch::Sender<Progress>,
    #[cfg(any(test, feature = "testing"))]
    on_step: OnceLock<Box<dyn Fn(Step) + Send + Sync>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopCause { Cancel, Shutdown }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub rows: usize,
    pub notes: usize,
    pub rows_to_embed: usize,
    pub rows_embedded: usize,
    pub rows_written: usize,
    pub notes_written: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    pub step: Option<Step>,
    pub step_started_at_ms: i64,
    pub counters: Counters,
    pub import_id: Option<i64>,
}
```

- **La API.**
  - `pub fn stop(&self, cause) -> bool` usa `send_if_modified`, así que la primera causa gana y vuelve `false` si ya había una. `send_if_modified` actualiza el valor aunque no haya receptores.
  - `pub fn cause(&self) -> Option<StopCause>` y `pub fn progress(&self) -> Progress` leen sin esperar.
  - Dentro del crate: `async fn raised(&self) -> StopCause`, con `subscribe()` y `wait_for(Option::is_some)`, que no pierde una señal levantada antes de esperarla; `fn begin(&self, step) -> (Instant, Timing)` (abajo); `fn count(&self, f: impl FnOnce(&mut Counters))`; `fn numbered(&self, import_id)`.
- **`stopping`.** El estado lo lleva como `cancel` o `shutdown`. Si llegan `DELETE` y después el apagado, queda `cancel` y el trabajo termina `cancelled`. Al revés, termina `interrupted`.
- **El gancho de pruebas.** `on_step(f)`, bajo `cfg(any(test, feature = "testing"))`, corre `f(step)` de forma síncrona en `begin`, después de publicar el paso. Es lo que deja levantar la señal exactamente al entrar en un paso (D9). Ninguna build de release lo compila: `testing` sólo se activa en `[dev-dependencies]` (`tests/conventions.rs` lo fija).
- **`Step::name`** (`timing.rs:36-49`) pasa a `pub`, para el `stopped_in` del daemon.

| Opción | A favor | En contra |
|---|---|---|
| **`watch` para la señal y para el progreso (elegida)** | La señal se mira sin esperar (`borrow`, también desde el hilo bloqueante) y se espera sin carrera (`wait_for`). El progreso se lee entero de una vez, así que `rows_written` y `notes_written` nunca se ven a medias. Es el patrón del repo (`AppCtx::shutdown`, `api.rs:102-105`; el drenaje, `runtime.rs:103-110`). Está en la feature `sync` de tokio, que ya está (`Cargo.toml:18`) | Un `RwLock` adentro. Se escribe una vez por lote, y nadie compite |
| Atómicos para los contadores y un `Notify` para la señal | Sin lock | Un lector puede ver un contador actualizado y el de al lado no. `Notify` exige el orden `notified()` antes de mirar la bandera, fácil de romper |
| `CancellationToken` | Hecho para esto | Es de `tokio-util`, que no es dependencia. No lleva causa |
| Reusar el `AtomicBool` de `StopOnDrop` como señal del trabajo | Sin tipo nuevo | `StopOnDrop` lo levanta cuando `write` termina, también en una importación que salió bien (`import.rs:2230`). Un trabajo terminado mostraría `stopping` |

**El paso tiene una sola fuente.** Hoy cada paso arma su par a mano: `let (clock, mut timing) = (Instant::now(), Timing::new(Step::Load))` (`import.rs:2224`, `:2251`, `:2281`, `:2307`). Ahora lo arma `control.begin(step)`: publica `step` y `step_started_at_ms` en el progreso y devuelve ese mismo par. El `Timing` que `ImportTimings::record` registra al terminar (`timing.rs:157-161`, sin cambios) lleva entonces el paso que se publicó al empezar. El progreso dice qué paso corre; `ImportTimings` guarda los que terminaron. No hay dos valores del paso que puedan diferir.

| Opción | A favor | En contra |
|---|---|---|
| **`begin` en el control, `ImportTimings` intacto (elegida)** | Un solo lugar nombra el paso. `ImportTimings` no cambia de rol ni de API | El control se pasa por parámetro a `plan`, `prepare_parsed`, `prepare_export` y `write`, que son privadas |
| El control dentro de `ImportTimings` (`with_control`, como `with_memory`, `timing.rs:152-154`) | Ninguna firma cambia, porque `&mut ImportTimings` ya llega a todos lados | Un objeto que mide pasaría a llevar también una orden de parar. Nadie buscaría la parada ahí |

**Los contadores** se publican así:

- `rows` y `notes` al terminar `prepare` o `translate`. `notes` es `prepared.units().len()` (`import.rs:369-382`): en un archivo de clawg, una por fila.
- `rows_to_embed` al empezar `embed` (`picked.len()`, `:2250`).
- `rows_embedded`, sumando las filas de cada lote que vuelve.
- `rows_written` y `notes_written` después de cada lote confirmado, con el `import_id` del `Journal`. Las notas de cada lote las cuenta una función pura, `batch_notes(units, &batches) -> Vec<usize>`, al lado de `note_batches` (`:2183-2197`), que no cambia.

Nada depende de `EMBED_BATCH` ni de `WRITE_BATCH`.

## D2. La entrada parseada y el resultado de una parada

```rust
pub enum ImportInput { Clawg(Parsed), Foreign(ForeignExport) }

pub enum Imported { Done(ImportReport), Stopped(Stopped) }

pub struct Stopped {
    pub cause: StopCause,
    pub step: Step,
    /// The report of the confirmed write batches; only after one was.
    pub report: Option<ImportReport>,
}

pub async fn import_parsed(
    store: &MemoryStore,
    embedder: Option<&dyn Embedder>,
    chunker: Arc<ChatChunker>,
    backups: &Path,
    input: ImportInput,
    options: &ImportOptions,
    ground: Arc<dyn Ground>,
    timings: &mut ImportTimings,
    control: &ImportControl,
) -> Result<Imported, ImportError>
```

- **Un solo parseo.** `prepare_file` (`import.rs:2071-2096`) pasa a llamarse `prepare_parsed` y recibe el `Parsed`, sin `parse` adentro (hoy, `:2079`). Un archivo de clawg se parsea sólo en el handler (D6). El paso `prepare` ya no cuenta un segundo parseo.
- **Las entradas viejas.** `import` (`:2055-2068`) e `import_foreign` (`:2103-2117`) conservan su firma y sus pruebas. Pasan a ser envoltorios `#[cfg(test)]` de `import_parsed` con un `ImportControl::default()`, y salen del `pub use` de `portability/mod.rs:16-19`, porque sus dos llamadas de producción (`api.rs:2059`, `:2061`) pasan a `import_parsed` (D6). Con un control que nadie levanta no hay `Stopped`. Por totalidad, el envoltorio convierte uno en el error de hoy (`MemoryError::Internal("the import was stopped")`, `:2235`).
- **Dentro del módulo**, cada paso devuelve `Result<T, Cut>`:
  - `enum Cut { Stopped(Step, StopCause), Failed(ImportError) }`;
  - `From<ImportError>` y `From<MemoryError>`, así `?` sigue sirviendo.
  - `import_parsed` traduce `Cut::Stopped` a `Imported::Stopped { report: None }` y `Cut::Failed(e)` a `Err(e)`.
- **La parada en `write`** no es un `Cut`. `Written` (`:2200-2208`) suma `cut: Option<StopCause>`, y `write` devuelve `Ok(Written { ledger, import_id: journal.id, cut: Some(cause), .. })` con el `Ledger` de los lotes confirmados. `import_parsed` arma el informe con eso, `report(&prepared, &written, false)` (`:2363`) y, para otra herramienta, `with_foreign` (`:2423`), y devuelve `Imported::Stopped { step: Step::Write, report: Some(..) }`.

**Qué cuenta un informe parcial.** `write` empieza con un `Ledger` vacío (`:2272`), así que `placed` tiene sólo las filas de los lotes confirmados:

- `inserted` es el número de filas de `memory_import_row` de su `import_id`;
- `present`, `present_elsewhere` y `merged` son de esos lotes;
- `projects`, `suggestions` y `subfolders` son verdad, porque los escribió el lote 0 (`record_projects`, `:2288-2297`);
- `dropped_links` queda vacío, porque `links` no corrió;
- `rows` sigue siendo las filas del archivo (`:2367`), así `render` dice «se importaron N de `rows` memorias del archivo».

`with_foreign` no nombra una nota que el `Ledger` no tiene (`:2466-2477`), y los recuentos de la traducción (`unplaced`, `discarded_sessions`) son del archivo, como siempre.

**Antes del primer lote no hay informe.** Hasta el lote 0 no se registró ningún proyecto ni sugerencia. Un `report(...)` armado con el `Ledger` vacío nombraría en `projects` y `suggestions` lo que se planeó y no se escribió, y `render` lo imprimiría como hecho (`crates/clawg-cli/src/import.rs:136`). La parada lleva `stopped_in` y los contadores, todos en cero salvo los de las fases que corrieron.

| Opción | A favor | En contra |
|---|---|---|
| **`Imported::Stopped` con `report` sólo tras un lote confirmado (elegida)** | Lo que el estado dice escrito está escrito. Coincide con IJ4 | El CLI tiene dos formas de contar una parada (D8) |
| Siempre un `report`, vacío antes del primer lote | Una forma | Un informe que nombra proyectos colocados que no existen |
| Una parada como `Err(ImportError::Stopped { report })` | La firma de hoy no cambia | La decisión 13 dice «en vez de un error», y `import_error` la convertiría en un 500 |

## D3. Dónde se mira la señal, paso por paso

Una función, en `import.rs`:

```rust
/// `work`, or the stop if it was raised first: dropping `work` is how a step
/// stops at its next `.await`.
async fn or_stop<T>(control: &ImportControl, step: Step, work: impl Future<Output = T>) -> Result<T, Cut> {
    tokio::select! {
        biased;
        cause = control.raised() => Err(Cut::Stopped(step, cause)),
        out = work => Ok(out),
    }
}
```

`biased`, con la señal primero, hace que una parada ya levantada gane aunque el trabajo esté listo en el mismo sondeo. Así no se escapa una parada por un paso que terminó a la vez.

| Paso | Cómo se mira | Dónde en el código | Qué deja |
|---|---|---|---|
| `plan` | `or_stop` alrededor de `plan(...)` | `prepare_parsed` y `prepare_export` (hoy `import.rs:2085`, `:2141`) | nada. La llamada en vuelo (`plan_ids_in`, sólo lee) termina en el hilo de la conexión y su respuesta se descarta. La lectura del disco en `spawn_blocking` (`:494-502`) termina sola |
| `prepare` | `control.cause()` al terminar `prepare` | después de `prepare(parsed, …)` (hoy `:2089`) | nada. No tiene `await` adentro |
| `translate` | `or_stop` sobre el `JoinHandle` del `spawn_blocking` | `:2166-2174` | nada. El hilo termina de traducir y su resultado se suelta |
| `load` | `or_stop` alrededor de `load(...)` | `write`, `:2225` | nada. No se manda otra llamada de `LOAD_BATCH` (`:1463`, `:1528`) |
| `decide` | `or_stop` sobre el `spawn_blocking`. Al volver, el `StopOnDrop` de hoy se suelta y levanta su `AtomicBool`, y `decide` para en su lote siguiente (`:1569-1572`) | `:2229-2235` | nada. Una pasada en seco termina acá (`:2238-2247`) |
| `embed` | `or_stop` sobre `embedding_order` y sobre cada `embed_batch`, y `control.cause()` al salir del bucle | `:2252`, `:2258`, antes de `:2268` | nada. Los vectores en memoria se sueltan (D4) |
| `snapshot` | `control.cause()` al terminar. No se compite | después de `:2268` | el snapshot `pre-import`, sin filas |
| `write` | `control.cause()` antes de cada lote salvo el primero. La llamada del lote nunca compite | el bucle de `:2282-2304` | notas enteras, con su diario y `finished_at` nulo |
| `links` | no se mira | `:2307-2319` | la importación entera |

- **Por qué `write` no usa `or_stop`.** La transacción de un lote se confirma en el hilo de la conexión, haga lo que haga el futuro (`:2286-2301`). Soltar la espera de un lote ya mandado perdería el `Ledger` y el `Journal` que vuelven con él, aunque sus filas estén escritas. El informe contaría de menos y el estado no tendría el `import_id`. Por eso la señal se mira sólo entre lotes. Lo mismo vale para `links` y `snapshot`.
- **Antes del lote 0 no se mira.** La última mirada antes de él es la de después del snapshot. Así `stopped_in: write` implica al menos un lote confirmado, y con él un `report`.
- **Una parada durante el último lote** no encuentra otro lote antes del cual mirarse: corre `links` y el trabajo termina `done`. Lo mismo un `DELETE` en `links`.
- **El número.** Al volver con `cut`, el `Journal` local de `write` (`:2275`) se suelta al terminar la función, y su `Drop` quita el número de `store.importing` (`:1015-1021`). Antes, `Written` copió `journal.id`. `clawg import --undo N` deja de ser `UndoError::Running` (`undo.rs:1218-1219`).

## D4. Abandonar el lote de embedding en vuelo

El `or_stop` de cada lote suelta el futuro de `embed_batch`, y con él lo que espera:

- **En producción**, el embedder es `IdleEmbedder`.
  - Soltar `with_model` suelta su `InFlight`, que estampa el uso también en una llamada cancelada (`crates/clawg-core/src/memory/idle_embedder.rs:146-157`).
  - El `E5Embedder` de adentro pierde su `oneshot`. Su hilo salta una petición cuyo receptor ya se cerró (`embedder.rs:614`) o termina la inferencia en curso y descarta la respuesta (`:620`).
  - Si el futuro estaba esperando una recarga dentro de `lease` (`idle_embedder.rs:160-201`), la recarga se suelta con él, y el próximo uso vuelve a cargar.
- **Sin API nueva en `Embedder`.** Lo único que queda corriendo es la inferencia que el hilo del modelo ya tenía, hasta ~3,5 s en la medición de 85. No escribe nada.

`wake()` se llama al aceptar un trabajo real (D6), como en un registro (`api.rs:2207-2211`). Despierta un modelo que la inactividad soltó (`idle_embedder.rs:357-366`) mientras corren `plan`, `translate`, `load` y `decide`. Si la recarga falla, el primer `embed_batch` devuelve `EmbedError::NotReady` (`:184-191`). Eso es `ImportError::Embed`, y `import_error` lo convierte en `embedder_not_ready` (`api.rs:1996`, `:1431-1436`): el trabajo termina `failed` sin snapshot ni filas, porque el snapshot viene después de `embed`.

## D5. El registro de trabajos

Va en un módulo nuevo, `crates/clawg-cli/src/daemon/imports.rs` (`pub mod imports` en `daemon/mod.rs`). Es un campo de `AppCtx` (`api.rs:97-114`), al lado de `sessions`, y como `SessionRegistry` (`daemon/sessions.rs:32-47`) es `Default` y usa un `std::sync::Mutex` que nunca cruza un `await`.

```rust
pub struct ImportJobs {
    state: Mutex<Jobs>,
    /// `true` from a reservation until its job ends or it is released.
    busy: watch::Sender<bool>,
}

/// The registry as data: every transition is a method, tested without a runtime.
struct Jobs { next_id: u64, slot: Slot, last: Option<Arc<JobView>>, closing: bool }
enum Slot { Idle, Reserved(u64), Running(Running) }
struct Running { id: u64, meta: Meta, control: ImportControl, task: AbortHandle }
struct Meta { dry_run: bool, from: &'static str, started_at_ms: i64 }

pub enum End {
    Done(ImportReport),
    Stopped(Stopped),
    Failed { error: String, detail: Option<String> },
}
pub enum Busy { Running(u64), Closing }
```

**Las transiciones**, todas bajo el lock:

| Método | Desde | Hacia | Si no |
|---|---|---|---|
| `reserve()` → `Reservation` | `Idle` y sin `closing` | `Reserved(next_id++)`, `busy = true` | `Busy::Running(id)` con el id reservado o corriendo. `Busy::Closing` si `closing` |
| `Drop for Reservation` | `Reserved(id)` sin aceptar | `Idle`, `busy = false` | nada, si la ranura ya no es la suya |
| `run(reservation, meta, control, spawn)` | `Reserved(id)` sin `closing` | `Running`; `spawn()` se llama bajo el lock y devuelve el `AbortHandle` | `Busy::Closing`: la reserva se suelta y no se lanza nada |
| `finish(id, end)` | `Running(id)` | `Idle`; `last` = la vista final; `busy = false` | nada: el trabajo ya se abandonó |
| `cancel(id)` | `Running(id)` | igual, con `control.stop(Cancel)` | vuelve la vista de `last` si es `id`; si no, `None` |
| `close()` | cualquiera | `closing = true`; `stop(Shutdown)` en el que corre | — |
| `abandon()` | `Running` | `task.abort()`; `Idle`; `last` = `interrupted`, `stopped_in` = `control.progress().step`, `abandoned: true` | `Reserved` → `Idle`; `Idle` → nada |
| `async settled()` | — | espera `busy == false` (`wait_for`) | — |
| `view(id)` / `active()` | — | la vista del que corre, armada con `meta`, `control.progress()` y `control.cause()`, o la de `last` | `None` |

- **La carrera de dos `POST`.** La reserva y la comprobación son una sola operación bajo el lock: gana uno y el otro recibe `409 import_running` con el id reservado. Como la reserva viene antes de leer el archivo (D6), dos `POST` nunca leen y parsean archivos de 256 MiB a la vez.
- **El id de una reserva que no llega a aceptarse.** El id se toma al reservar, así el `409` siempre nombra uno. Si la reserva falla después (un `400` del archivo), ese id no existe nunca: su `GET` es `404` y el contador lo saltea.
- **El handle.** Lanzar la tarea bajo el lock y guardar su `AbortHandle` en la misma sección cierra la carrera con una tarea que termina antes de que el registro sepa de ella: su `finish` espera el lock. `tokio::spawn` no espera, así que el lock se tiene microsegundos. El registro no conoce `AppCtx`: `spawn` es un cierre que arma `api.rs`.

| Opción | A favor | En contra |
|---|---|---|
| **Una ranura con reserva, tomada antes de leer el archivo (elegida)** | Nunca hay dos lecturas a la vez. El `409` es atómico. El soltado por `Drop` cubre un handler que se corta mientras lee (cliente que se va, `HTTP_GRACE`) | Un id puede saltearse |
| Mirar si hay un trabajo y registrarlo al final, antes del `202` | Sin estado `Reserved` | Dos `POST` a la vez leen y parsean dos archivos, y uno se tira |
| Sin reserva: el `409` sólo con un trabajo corriendo | Lo más simple | Lo mismo, con más memoria |
| Un `tokio::sync::Semaphore` de un permiso | Primitiva hecha | No da el id para el `409` ni el último terminado. Igual hace falta el estado |

**El último terminado** se guarda como `Arc<JobView>`, así un `GET` lo clona sin copiar el informe bajo el lock. Se reemplaza al terminar el siguiente. Un id que no es el activo ni el último es `404`.

**Las fases** las da una función pura del módulo, `phase_of(Option<Step>) -> &'static str`:

- `None`, `Read` y `Plan` dan `planning`. Todo trabajo empieza en `plan` (`import.rs:2085`, `:2141`), y el `202` sale antes de que la tarea corra. `Read` no llega a un control: es del `POST`.
- `Prepare` → `preparing`, `Translate` → `translating`, `Load` → `loading`, `Decide` → `deciding`, `Embed` → `embedding`, `Snapshot` → `snapshot`, `Write` → `writing` y `Links` → `linking`.
- Las finales salen del `End`:
  - `Done` → `done`;
  - `Stopped` con `cause` `Cancel` → `cancelled` y con `Shutdown` → `interrupted`;
  - `Failed` → `failed`;
  - un aborto → `interrupted`.

El `match` es exhaustivo, así un `Step` nuevo no compila sin su fase.

## D6. Las rutas, el cuerpo y lo que comparten con `POST /v1/import`

```rust
.route("/imports", post(start_import_job))
.route("/imports/{id}", get(import_job).delete(cancel_import_job))
```

Las dos rutas se leen bien con `routes_in_router` (`crates/clawg-cli/tests/api.rs:4563-4583`): ningún nombre de handler contiene `get(`, `post(` ni `delete(`. Quedan detrás de `require_bearer`, `admit_loopback` y `time_request` (`api.rs:1075-1086`), que miden por la plantilla.

**El cuerpo** es `ImportJobRequest { path, from, dry_run, agent, map_roots, project_map }`, con `#[serde(deny_unknown_fields)]`.

- **Se lee como `Bytes`** y se deserializa con `serde_json::from_slice`. Cualquier falla es un `ApiError::bad_request` con el texto de serde, que nombra el campo («unknown field `text`, expected one of `path`, …»). Con el extractor `Json` sería un `422` en texto plano (Frente a la spec).
- **El tope** es el `DefaultBodyLimit` de axum, que también aplica a `Bytes`. No lleva el `IMPORT_BODY_LIMIT` de `/import` (`api.rs:1063`, `:1939-1941`): el cuerpo ya no trae el archivo.
- **La parte común.** `ImportRequest` (`:1943-1965`) y `ImportJobRequest` comparten todo salvo `text` y `path`. `#[serde(flatten)]` no se puede combinar con `deny_unknown_fields`, así que son dos structs que dan un `ImportFields` común.

**Las piezas comunes**, en `api.rs`:

| Pieza | Qué hace | Hoy en |
|---|---|---|
| `too_large(len) -> ApiError` | la frase del tamaño, idéntica en las dos rutas | `:2013-2018` |
| `checked_fields(&app, ImportFields) -> ApiResult<Asked>` | agente en blanco, `map_roots` sin raíz, `resolve_map` (404, 409), `from` desconocido, el mapa de la otra fuente. Devuelve `Asked { source, options }` | `:2019-2033`, `:2037-2042`, `:2049` |
| `parse_input(source, &str) -> ApiResult<ImportInput>`, pura | `format::parse` o `engram::read`, con sus errores como 400 | `:2034`, `:2043` |
| `tokenizer_for(&app, &input) -> ApiResult<()>` | `503 tokenizer_not_ready` para otra herramienta | `:2044-2046` |
| `embedder_for(&app, dry_run) -> ApiResult<Option<Arc<dyn Embedder>>>` | `ready_embedder` salvo en seco | `:2052` |

**`POST /v1/import`** las usa en su orden de hoy, sin cambiar respuestas: `too_large`, `checked_fields`, `parse_input`, `tokenizer_for`, el registro de `read` y `embedder_for`. Después corre `import_parsed` con `ImportControl::default()`. `body.text` sigue vivo hasta el final del handler, como hoy (`api.rs:2009`), así la medición por cuerpo mide lo de siempre. Lo único que cambia adentro es que el archivo se parsea una vez. `StopOnDrop` sigue cortando `decide` cuando el cliente se va: está dentro de `write`, que esta ruta sigue usando.

**`POST /v1/imports`** (`start_import_job`, que delega en `accept_job(app, request, control)` para que las pruebas pasen su control, D9) sigue este orden:

1. El cuerpo estricto (`400`).
2. `path` absoluta (`Path::is_absolute`, pura; `400`).
3. `checked_fields` (`400`, `404`, `409 project_ambiguous`, `409 project_deleted`).
4. `reserve()`: `409 import_running` con `job_id` o `503 shutting_down` con `retryable: true`.
5. Un `spawn_blocking` que:
   - lee `std::fs::metadata(path)`, que sigue los enlaces. Un archivo que falta es «no such file». Uno que no es regular es «not a regular file»: se mira antes de abrir, así un FIFO de Unix nunca bloquea la lectura;
   - aplica `too_large` con el largo;
   - abre y lee con `take(MAX_IMPORT_BYTES + 1)`, así un archivo que crece entre la medida y la lectura también es un `400`;
   - convierte a UTF-8;
   - corre `parse_input`.

   El `String` del archivo nace y muere dentro del cierre: al volver, sólo existe el `Parsed` o el `ForeignExport`.
6. `tokenizer_for`, el registro de `read` en el `ImportTimings` del trabajo (con `with_memory`, como `:2011`) y `embedder_for`.
7. `run(reservation, …)`, que lanza `run_job`. `wake()` sólo si la importación es real.
8. `202` con la vista.

Cualquier falla entre 5 y 7 suelta la reserva por `Drop`.

| Opción para el `409` | A favor | En contra |
|---|---|---|
| **Después de lo que sólo lee el cuerpo y de `resolve_map` (elegida)** | Un pedido mal armado se entera de eso aunque haya un trabajo. El archivo nunca se toca con un trabajo activo | `resolve_map` consulta el store para un pedido que después es `409`: unas pocas búsquedas por índice |
| Antes de todo, salvo la forma del cuerpo | Una sola comprobación al principio | Un `agent` en blanco recibe `409`, y su `400` recién cuando el otro trabajo termina |

Cada caso de una sola falla da la misma respuesta en las dos rutas (IJ2). Con dos fallas a la vez, la del archivo y la de un campo, la ruta síncrona nombra primero el tamaño (`:2013`) y la de trabajos nombra primero el campo.

**`run_job(app, checked, control) -> End`** corre `import_parsed`:

- `Ok(Done)`: `timings.finish()` (`timing.rs:173-175`) y `End::Done`.
- `Ok(Stopped)`: una línea de log y `End::Stopped`.
- `Err(e)`: `import_error(e)` (`api.rs:1992-2000`) da `End::Failed` con su `message` como `error` y su `detail`. Son los códigos de hoy: `embedder_not_ready`, `project_deleted`, o el texto de un 500.

Las líneas de log del trabajo (`import job N accepted`, `import job N ended …`) llevan números y nombres de paso, nunca la ruta del archivo, como manda `timing.rs:1-4`.

**El estado (`JobView`)**:

```
job_id, phase, dry_run, from, started_at_ms, phase_started_at_ms,
rows, notes, rows_to_embed, rows_embedded, rows_written, notes_written,   // Counters, #[serde(flatten)]
import_id?, stopping?, stopped_in?, abandoned? (sólo true), report?, error?, detail?
```

- `phase_started_at_ms`: en un final, el momento en que terminó.
- `stopping`: sólo mientras el trabajo está activo.
- `stopped_in`: `Step::name` (`plan`, `prepare`, `translate`, `load`, `decide`, `embed`, `snapshot`, `write`, y `links` sólo en un aborto).
- `report`: con `done`, y con `stopped_in: write`.

| Ruta | Respuesta |
|---|---|
| `GET /v1/imports/{id}` | `200` con la vista; `404` `no import job N` |
| `DELETE /v1/imports/{id}` | activo: `stop(Cancel)` y `202` con la vista, que ya trae `stopping`; terminado: `200` con su vista, sin cambios; `404` |
| `GET /health` | `import`: la vista del activo, sin `report` (un activo no lo tiene), `skip_serializing_if` vacío. Sale de `ImportJobs::active()`, que no toca la conexión, al lado del bloque `ingest` que sí espera hasta `HEALTH_INGEST_WAIT` (`api.rs:1165-1180`) |

`ErrorBody` y `ApiError` (`api.rs:123-148`) suman `job_id: Option<u64>`, como `memories` para un save fallido.

## D7. El apagado

**Dónde.**

1. La tarea que observa la señal (`serve.rs:249-253`) llama a `ctx.imports.close()` antes de `stopping_tx.send(true)`: la causa `shutdown` se levanta en el mismo instante en que empieza la gracia.
2. La espera corre como un tercer futuro dentro del mismo `try_join!` que los dos servidores (`:272`). Así queda dentro de la ventana que `within_grace` cuenta desde esa señal (`:273`, `:405-412`):

   ```rust
   let import_job = async { stopped(stopping.clone()).await; ctx.imports.settled().await; Ok(()) };
   ```

   Espera la señal antes que el registro. Sin eso terminaría al acabar un trabajo en plena operación, y no esperaría al que empiece después.
3. Si `within_grace` da `None`, `ctx.imports.abandon()` aborta la tarea y marca el trabajo antes de `store_probe.abort()` (`:286`). La advertencia de `:276-279` pasa a decir «requests or an import job were still running», y `abandon` deja su propia línea con el paso.
4. Después siguen, sin cambios, `writer.close`, `runtime.shutdown` y `runtime.close` (`:289-298`).

La suma que fija `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` (`stop.rs:67-76`) no gana ningún término.

**Por qué abortar no escribe de más.**

- **El orden de la conexión.** En `tokio-rusqlite` 0.7.0, `call_raw` manda el cierre por un canal `crossbeam_channel::unbounded` antes de esperar la respuesta (`tokio-rusqlite-0.7.0/src/lib.rs:290-304`; el canal en `:376` y `:387`). El hilo de la conexión ejecuta los mensajes de a uno, en el orden en que llegaron (`event_loop`, `:412-427`). `close` es un mensaje más (`:347-361`), y tras él el bucle termina (`:422`), así que lo que se mande después nunca corre.
- **Lo que deja el aborto.** Abortar suelta el futuro del trabajo en un `await`. Si es la espera de una llamada, esa llamada ya está en la cola y corre antes que el drenaje, el checkpoint y el `close` que el apagado manda después. Nada más se manda, porque sólo se manda mientras el futuro se sondea.
- **Un lote de escritura** es una transacción que se confirma entera dentro de su cierre (`import.rs:2286-2301`). Si la respuesta no encuentra receptor, el valor (el `Ledger` y el `Journal`) se suelta en el hilo de la conexión, y el `Drop` del `Journal` libera el número (`:1015-1021`).
- **El snapshot** de un store en disco no pasa por esa cola: corre en un hilo bloqueante con su propia conexión (`db.rs:308-311`). Escribe en `.db.tmp` y renombra sólo cuando terminó (`backup.rs:370-379`). Si `RUNTIME_TEARDOWN` corta el proceso antes, no queda un snapshot con su nombre final, y ninguna fila depende de él.

**Lo que dice el estado final (Frente a la spec, punto 5).**

- **`DELETE` en `snapshot`:** el snapshot termina, la mirada de después lo ve, y el trabajo queda `cancelled` con `stopped_in: snapshot`. No hay filas ni número. El CLI dice que no se escribió nada y que el snapshot `pre-import` quedó en `backups` (D8).
- **`DELETE` en `links`** (o durante el último lote): `done`. La respuesta del `DELETE` mostró `stopping: cancel`. El final dice lo que pasó, y el CLI, que mandó el `DELETE`, lo aclara (D8).
- **El aborto** marca `interrupted` con `abandoned: true` y `stopped_in` en el paso que corría. `interrupted` no afirma que nada se confirmó: dice que el proceso se fue antes de que el trabajo informara su final. `abandoned` nombra que la llamada que ese paso ya había mandado puede haberse confirmado después, y que `memory_import` es el registro. Cuando aborta, los servidores ya se cortaron (es el mismo `within_grace`), así que ese estado sólo lo leen la línea de log y las pruebas. El cliente se entera por la desaparición del daemon (D8).

| Opción | A favor | En contra |
|---|---|---|
| **`interrupted` con `abandoned` y el paso; el log nombra la duda (elegida)** | No afirma lo que no sabe. Ningún estado final nuevo | Un campo que el CLI nunca ve |
| `interrupted` a secas | Sin campo | En `writing` o `linking` afirma una parada limpia que no hubo |
| Esperar la llamada en vuelo más allá de la gracia | El final sería exacto | Rompe la cota de `HTTP_GRACE` que fija la decisión 7 |
| Un final `unknown` | Explícito | Un quinto estado final que la spec no tiene, para un estado que nadie lee por HTTP |

## D8. El CLI

**El envío.**

- `clawg import FILE` manda `std::path::absolute(FILE)`, que no toca el disco, sin leer el archivo.
- Las comprobaciones de `--project-map` sobre directorios siguen como hoy (`crates/clawg-cli/src/import.rs:51-57`). Las de tamaño y lectura del cliente (`:58-63`) se van: las hace el daemon, con un `400` que nombra la causa. También se van `ImportError::Read` y `TooLarge` (`:16-23`), y `DaemonClient::import` (`client.rs:1410-1423`), que se queda sin llamadas.
- `--dry-run` también va como trabajo. `--undo` sigue síncrono e intacto.

**El cliente** (`client.rs`) suma tres métodos sin plazo, porque `only_health_sets_a_timeout` (`:1962-1970`) no cambia:

- `start_import(&ImportJobRequest) -> Result<Option<ImportJob>, ClientError>`: `None` es «este daemon no tiene la ruta»;
- `import_job(id)`;
- `cancel_import(id)`.

`ImportJob` es un tipo propio del cliente con `Deserialize`, como manda el comentario de `client.rs:127-132`. Reusa `Counters` e `ImportReport` del núcleo, y deja `phase`, `stopping` y `stopped_in` como texto, así una fase que no conoce se imprime igual. `ErrorDetails` (`:101-109`) suma `job_id`.

**Un daemon anterior.** Verificado en esta build:

- `router` no declara `fallback` (`api.rs:1041-1087`).
- El `Router` de axum 0.8.9 tiene como red el servicio `NotFound` (`routing/mod.rs:150-152`), que contesta `StatusCode::NOT_FOUND.into_response()`, sin cuerpo (`routing/not_found.rs:31-33`).
- Al anidar `/v1`, el fallback por defecto del router de adentro no pasa al de afuera (`routing/mod.rs:216-229`). Por eso `require_bearer` ni corre para `/v1/imports` en un daemon que no la tiene, y el `404` sale sin cuerpo, con token o sin él. `admit_loopback` y `time_request` lo dejan pasar.
- Todo `404` que el daemon contesta por su cuenta es `ApiError::not_found`, con `{"error": …}` (`api.rs:167-169`, `:182-191`).

`fn route_missing(status, body: &[u8]) -> bool` es pura: un `404` cuyo cuerpo no es un objeto JSON con `error` de texto. Lee la `Answer` entera (`client.rs:835-840`) antes de `read_response`. Un `404` del daemon nuevo, por ejemplo `no project "x"` en un `project_map`, lleva `error` y sigue siendo un `ClientError::Daemon`.

| Opción | A favor | En contra |
|---|---|---|
| **La forma del cuerpo decide, y `/health` da la versión para la frase (elegida)** | Sale de la respuesta misma. No depende de una versión fija: un daemon de desarrollo de la misma versión, anterior a la ruta, también se detecta | Si el daemon un día sumara un `fallback` con JSON, habría que revisarlo (lo cubre la prueba de IJ16) |
| Comparar con `/health` una versión mínima | Antes de mandar nada | La versión que trae la ruta recién se fija al archivar. Y no distingue dos builds de una misma versión |
| `message == "404 Not Found"` (lo que `daemon_error` pone sin cuerpo, `client.rs:113-120`) | Sin función nueva | Compara un texto de reqwest |

La frase sale de `version_warning(lang, daemon, cli)` (`memories.rs:378-389`), con la versión de `/health`. Si las versiones coinciden, `version_warning` da `None`, y una frase hermana dice que el daemon que corre es una build anterior de la misma versión y que `clawg stop` hace que el próximo arranque use esta. Sale con 1 y no manda nada del archivo, ni la ruta, salvo en ese primer `POST`.

**El seguimiento.** Todo lo que decide es puro; el bucle de E/S sólo lo maneja.

```rust
pub struct Follow { asked_to_stop: bool, last: ImportJob }
pub enum Event { Polled(ImportJob), Interrupt, Lost(ClientError) }
pub enum Next { Poll, Cancel, Leave(String), End(Finished), Fail(String) }
impl Follow { pub fn on(&mut self, lang: Lang, event: Event) -> Next }
pub struct Finished { pub out: String, pub note: Option<String>, pub ok: bool }
```

- **El primer Ctrl-C** devuelve `Cancel` (el bucle manda `DELETE` y sigue sondeando). El segundo devuelve `Leave`, con la frase de que el trabajo N quedó parando y, si hay `import_id`, el `--undo N`.
- **Un estado final** devuelve `End`. `Lost` devuelve `Fail` con `gone(lang, &last)`.
- **Los Ctrl-C** los reenvía a un canal una tarea que se lanza antes del `POST`. Uno que llega durante el `POST` se atiende al llegar el `202`.
- **El sondeo** es cada `POLL_EVERY` = 1 s. Un `DELETE` contesta la vista, y esa vista también pasa por `on`.
- **`shows_progress(follow, stderr_is_terminal) -> bool`** es `follow || terminal`, con `std::io::IsTerminal` sobre stderr.
  - En una terminal, `progress_line(lang, &job)` redibuja la línea en el lugar (`\r`) y pasa a una línea nueva cuando cambia la fase.
  - Fuera de una terminal, con `--follow`, imprime una línea por fase que ve y una al final con los contadores.
  - Sin ninguna de las dos, stderr no lleva progreso.

**Lo que imprime**:

| Final | stdout | stderr | Código |
|---|---|---|---|
| `done` | `render(lang, report)` (`import.rs:104`) | si se mandó `DELETE`: que el trabajo estaba en su último paso y terminó | 0 |
| `cancelled` o `interrupted` con `report` | `render` y una línea: paró en `write` tras R de `rows` filas (N de `notes` notas); `clawg import --undo K`, o importar otra vez el archivo para terminarlo | — | 1 |
| `cancelled` o `interrupted` sin `report` | paró en `{stopped_in}`: no se escribió nada; en `snapshot`, además, que el snapshot `pre-import` quedó en `backups` | — | 1 |
| `failed` | — | `clawg: the import failed: {error}: {detail}`, por el `Err` de `dispatch` (`main.rs:82-91`) | 1 |
| el daemon desaparece | — | `gone`: con la última fase y el último `import_id` vistos | 1 |
| `409 import_running` | — | que otra importación (trabajo N) corre en el daemon y que se reintente cuando termine | 1 |
| daemon sin la ruta | — | la frase de arriba | 1 |

`gone` decide por la última fase, porque el estado de un trabajo abandonado no se puede leer (D7):

- antes de `snapshot`: no se escribió nada;
- en `snapshot`: no se escribió nada, y puede haber quedado un snapshot;
- en `writing` o `linking` con `import_id` K: la importación K quedó escrita en parte, o entera si estaba en `linking`; `clawg import --undo K`, o importar otra vez el archivo;
- en `writing` sin `import_id`: su primer lote pudo escribirse, y una reimportación termina el resto, porque halla presentes las notas escritas.

Un `GET` que da `404` mientras se sigue un trabajo cubre otro caso raro: otro cliente lanzó un trabajo justo después del final de este, y el último terminado ya es otro. Lo trata `gone`, con la última vista.

**El idioma.** Cada frase nueva es un `match` sobre `Lang`, como `render` (`import.rs:106-111`). Quedan como vienen `phase`, `stopped_in`, los códigos de error, `job_id`, `import_id`, los comandos y la causa técnica.

**`main.rs`.** El brazo `Command::Import` (`main.rs:237-260`) pasa `follow` y convierte `Finished { ok }` en el código de salida. `cli.rs` suma `#[arg(long, conflicts_with = "undo")] follow: bool` a `Command::Import` (`cli.rs:112-145`).

## D9. Pruebas

Cada prueba de la spec, con su costura. Las del núcleo toman su directorio de `crate::test_dir::test_dir()`. Las de `clawg-cli` usan `tempfile`, como el `Harness` (`tests/api.rs:241`) y `stop.rs:116`. Ninguna deja nada en el temporal: el store de prueba de `MemoryRuntime::new_for_test` vive en memoria (`runtime.rs:418-422`), y su snapshot va por la conexión (`db.rs:300-306`). Una prueba que deja el snapshot en vuelo espera una llamada posterior en la conexión, que por el orden FIFO corre después, antes de soltar su directorio.

**Un embedder que retiene.** Va en `embedder.rs`, al lado de `FakeEmbedder` (`:796-799`), bajo `cfg(any(test, feature = "testing"))`: `GatedEmbedder`, con un `Gate`.

- `embed_batch` avisa por `Gate::entered()` (un `Notify`) y toma un permiso de un `Semaphore`. Sin permisos, no vuelve nunca. `Gate::release(n)` suelta `n` lotes.
- Responde como `FakeEmbedder`.
- Sirve a las pruebas del núcleo y de `clawg-cli`, unitarias y de integración. El `Harness` suma `EmbedderState::Gated`, como `Docs` y `Fixed` (`tests/api.rs:310-321`).

**El gancho de paso** (`ImportControl::on_step`, D1) hace determinista una parada al entrar en cualquier paso:

- en uno con `await`, `or_stop` lo corta en su primer `await`;
- en `prepare`, la mirada del final;
- en `snapshot`, el snapshot se hace y la mirada del final para;
- en `write`, se confirma el lote 0 y la parada llega antes del lote 1;
- en `links`, termina `done`.

Una prueba de integración no puede inyectar un control. Las que lo necesitan van en `mod tests` de `daemon/api.rs` (`:2802`), que ya arma un `AppCtx` (`:3058`) y llama a `accept_job(app, request, control)`.

| Prueba (spec) | Dónde | Costura |
|---|---|---|
| `the_synchronous_import_still_cancels_when_the_client_goes` (IJ8, guarda) | `tests/api.rs` | `Gated`; la tarea del cliente se aborta tras `entered()`. `http.in_flight` vuelve a 0 sin soltar el lote, y no hay filas ni `memory_import` |
| `a_clawg_file_is_parsed_once_for_a_job`, `the_counters_do_not_depend_on_the_batch_sizes`, `the_counters_of_a_finished_import_match_its_report` | `import.rs` | `import_parsed` contra `import`. Archivos de 31, 32, 33, 255, 256 y 257 filas |
| `each_step_of_an_import_stops_where_the_table_says` | `import.rs` | `on_step` levanta `Cancel` al entrar en cada paso. Se mira `Stopped.step`, las filas, `memory_import` y los archivos de `backups` |
| `an_abandoned_embedding_batch_does_not_block_the_stop` | `import.rs` | `GatedEmbedder` sin permisos; `stop` tras `entered()` |
| `a_stop_between_write_batches_returns_the_committed_ledger` | `import.rs` | `on_step(Write)`; `inserted` contra `memory_import_row`; `finished_at` nulo; `store.importing` vacío |
| `an_abandoned_import_sends_nothing_after_its_cut` | `import.rs` | `on_step(Write)` encola, con `FutureExt::now_or_never` (`futures` ya es dependencia del núcleo), una llamada que retiene la conexión hasta que la prueba la suelta. La importación corre en una tarea que se aborta. Al soltar, se confirman los lotes que ya estaban en la cola y no otros; una llamada marcador después lo comprueba |
| IJ1, IJ2, IJ3, IJ4, IJ5 al terminar (`a_finished_jobs_counters_match_its_report`), IJ9, IJ10, IJ11, IJ12, IJ14 | `tests/api.rs` | HTTP sobre el `Harness`. IJ2 es una tabla que manda cada caso a las dos rutas, con el archivo en `tempfile`. IJ3 e IJ9 con `start_unloaded` (`:227-229`) y su contador `reloads`. El `GET` en `embedding` con `Gated`, de a un permiso |
| `deleting_a_job_in_embedding_leaves_no_row_no_journal_and_no_snapshot`, `a_delete_on_a_finished_job_changes_nothing`, `a_job_outlives_the_client_that_accepted_it` | `tests/api.rs` | `Gated`: se manda `DELETE`, o se corta el cliente, tras `entered()` |
| `a_delete_during_links_ends_done`, `a_job_stopped_in_writing_counts_exactly_the_committed_batches`, `a_job_stopped_in_writing_leaves_whole_notes_and_frees_its_number`, `importing_again_finishes_a_stopped_job`, `undoing_a_stopped_job_works` | `daemon/api.rs`, `mod tests` | `accept_job` con un `on_step` que levanta `Cancel` en el control (en `Links` o en `Write`). La lectura, por el router en un puerto efímero (`reqwest` es dependencia normal) |
| `shutdown_stops_a_job_in_embedding_within_the_grace` | `daemon/api.rs`, `mod tests` | `Gated` sin permisos; `close()` tras `entered()`; `timeout(HTTP_GRACE, settled())` |
| `shutdown_stops_a_job_in_writing_between_whole_notes` | ídem, con tiempo real | `on_step(Write)` llama a `close()` |
| `an_abandoned_job_sends_nothing_to_the_connection` | ídem, `start_paused = true` | `Gated` retiene `embed`. La prueba encola una llamada que retiene la conexión y espera a que entre. Suelta el embedder: el trabajo queda en `snapshot`, que en el store de prueba va por la conexión. `close()`, y `settled()` vence la gracia, porque el reloj avanza con el runtime ocioso. `abandon()` deja `interrupted`, `abandoned`, `stopped_in: snapshot`. Se suelta la llamada, y un marcador muestra que no hay filas ni `memory_import` |
| registro puro | `daemon/imports.rs` | `Jobs` sin runtime: reserva, carrera, soltado por `Drop`, `run` con `closing`, `finish` después de `abandon`, último terminado, `phase_of` exhaustivo |
| `the_job_messages_speak_both_languages`, `phases_go_to_stderr_only_on_a_terminal_or_with_follow`, `any_final_state_but_done_exits_1`, `a_second_ctrl_c_leaves_without_waiting`, `a_vanished_daemon_names_the_last_import_id`, `route_missing` | `crates/clawg-cli/src/import.rs` y `client.rs` | Funciones puras: `Follow::on`, `progress_line`, `gone`, `shows_progress`, `route_missing`. En Windows, Ctrl-C se prueba así |
| `a_job_import_ends_done_with_the_report_of_the_synchronous_import`, `follow_prints_the_phases_in_order_on_stderr_and_the_same_report_on_stdout`, `killing_clawg_import_does_not_stop_the_job`, `ctrl_c_sends_the_delete_and_prints_what_was_written` (Unix, `SIGINT`) | `tests/binary.rs` | `CLAWG_TEST_EMBEDDER=fake` (`:69`). El proceso se mata por su propio handle |
| `import_against_a_daemon_without_the_job_route_exits_1_without_sending_the_file` | `tests/binary.rs` | Un servidor axum de la prueba: publica `daemon.json` y `token` en el home de prueba, contesta `/health` con una versión y deja el resto al `404` por defecto, anotando cada ruta pedida. El cuerpo del `POST` lleva una ruta, y nada más llega |

`every_route_says_which_project_it_takes` (`tests/api.rs:4587-4591`) gana tres filas en `PROJECT_INPUTS` (`:4520-4560`): `("POST", "/imports", Some("body.project_map[].project"))`, `("GET", "/imports/{id}", None)` y `("DELETE", "/imports/{id}", None)`. Los dos recorridos ganan su brazo `("POST", "/imports")`, junto al de `/import` (`:4744-4751`, `:5171-5179`):

- **tras una fusión** (`:4614`): escribe el export de otra herramienta en un `tempfile` y manda su `path` con `project_map` a `app`. Espera `done` con `GET` y comprueba que la fila quedó en `web`;
- **con un proyecto borrado** (`:5042`): por cada `dry_run`, `refused(...)` (un `409 project_deleted` antes del `202`) y que `/health` no tiene bloque `import`.

## D10. La medición de memoria

Va en un archivo de pruebas propio, `crates/clawg-cli/tests/import_memory.rs`, con dos pruebas `#[ignore]` que se corren de a una, cada una en su proceso, porque un pico sólo sube:

```
cargo test -p clawg-cli --test import_memory --target-dir target-test -- --ignored --exact by_body --nocapture
cargo test -p clawg-cli --test import_memory --target-dir target-test -- --ignored --exact by_path --nocapture
```

Cada brazo:

1. Genera en el home de prueba un export de clawg y uno de otra herramienta, de unos 100 MiB cada uno, con texto determinista.
2. Por cada uno, arranca un `clawg serve --port 0` nuevo, con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake`. Es el binario de `cargo test`, la build con `testing`.
3. Importa:
   - **`by_body`**: el proceso de la prueba hace lo que hacía el cliente anterior: `read_to_string`, `json!` con `text`, `to_vec` y `POST /v1/import` con reqwest;
   - **`by_path`**: llama a `clawg_cli::import::run`, el mismo código que `clawg import`, sin terminal.
4. El pico del cliente es el del proceso de la prueba: `clawg_cli::daemon::process::sample()` (`process.rs:27`), su `private_peak_bytes` en Windows y su `rss_peak_bytes`, que en Linux es `VmHWM` (`process.rs:58`).
5. El del daemon es su `process.private_peak_bytes` (Windows) o `process.rss_peak_bytes` (Linux), leído en `GET /v1/metrics` en el primer minuto que se escribió después de terminar la importación. El pico es monótono, y el daemon es nuevo por brazo.
6. Lee además la memoria privada al terminar `embed`, de la línea `import: embed` del log del daemon (`timing.rs:157-159`).
7. Imprime una línea JSON por formato y brazo, y la prueba no afirma nada más: los números van a mano a `verify.md`.

No suma dependencias. Ninguna build de release la compila.

| Opción | A favor | En contra |
|---|---|---|
| **Un archivo de pruebas `#[ignore]` (elegida)** | Precedente: `transport_bar.rs`. Usa el binario de la suite y la biblioteca `clawg_cli` | Hay que correrlo a mano, una prueba por proceso |
| Un binario en `clawg-eval`, como `load-eval` | Separa la medición de la suite | Más código para una medición que se hace una vez, y no hay una puerta que decidir |
| Medir el cliente por cuerpo con el binario anterior | Es el cliente real | La propuesta pide la misma build |

## D11. Orden de los commits

Cada commit escribe primero su prueba que falla, compila y pasa la suite. Las líneas de `.claude/CLAUDE.md` que describen un comportamiento cambian en el mismo commit que lo cambia. El CLI pasa a la ruta nueva recién en el commit 8, así que ningún commit intermedio expone el trabajo a `clawg import`.

1. `test(import): a synchronous import is cut when its client goes`.
   - `GatedEmbedder` y `EmbedderState::Gated`.
   - Es la guarda de IJ8: pasa con el código de hoy.
2. `feat(import): an import reports its step and its counters as it runs`.
   - `control.rs` con el progreso y `begin`, `ImportInput` e `import_parsed`.
   - `import` e `import_foreign` pasan a envolver `import_parsed`, todavía públicas.
   - `batch_notes` y `Step::name` pública.
   - Las pruebas de contadores y de un solo parseo.
3. `feat(import): an import stops at the safe point of its step`.
   - La señal, `or_stop`, `Cut`, el `cut` de `Written`, `Imported::Stopped` y `on_step`.
   - Las pruebas de parada del núcleo.
4. `refactor(daemon): both import routes share their checks`.
   - `ImportFields`, las piezas de D6, y `POST /v1/import` sobre `import_parsed`.
   - `import` e `import_foreign` pasan a `#[cfg(test)]` y salen de `pub use`.
   - Lo guardan las pruebas de importación de `tests/api.rs`, que no cambian.
5. `feat(daemon): run an import as a job`.
   - `daemon/imports.rs` y `imports: Default::default()` en los doce sitios que arman un `AppCtx`: `serve.rs:227`, `stop.rs:118`, `socket/mod.rs:148`, `client.rs:1875` y `:2114`, `mcp/mod.rs:203`, `mcp/server.rs:1831` y `:4689`, `api.rs:3058`, `tests/api.rs:343`, `clawg-eval/src/bin/revision-eval/run.rs:438` y `clawg-eval/src/bin/rag-eval/agent_view/runner.rs:182`.
   - `POST /v1/imports`, `GET /v1/imports/{id}`, `job_id` en `ErrorBody`, sus dos filas de `PROJECT_INPUTS` y los brazos.
   - Las pruebas de IJ1 a IJ5, IJ7 (con un `on_step` que levanta `Cancel` en el control), IJ9 a IJ11, IJ13 e IJ14.
6. `feat(daemon): a shutdown stops the import job within the grace`.
   - `close`, `settled`, `abandon`, la espera en `serve.rs` y `503 shutting_down`.
   - Las pruebas de IJ18. `stop.rs` no cambia.
   - Va justo después del 5, así ningún commit deja un trabajo que el apagado no espere.
7. `feat(daemon): cancel an import job, and name it in health`.
   - `DELETE /v1/imports/{id}` y su fila de `PROJECT_INPUTS`, y el bloque `import` de `/health`.
   - Las pruebas de IJ6, IJ8 e IJ12.
8. `feat(cli): clawg import follows its job and Ctrl-C cancels it`.
   - `client.rs`, `cli.rs`, `import.rs`, `main.rs`, `--follow` y la detección del daemon anterior.
   - Se van `DaemonClient::import`, `ImportError::Read` y `TooLarge`.
   - Las pruebas de IJ15 a IJ17 y las del binario.
9. `test(import): measure an import's memory by body and by path`: D10.

Al archivar: los deltas de `specs/portability`, `specs/daemon-access` y `specs/storage`, la fila 87 de `openspec/README.md` y la subida de la versión.

## Archivos que cambian

| Archivo | Qué |
|---|---|
| `crates/clawg-core/src/memory/portability/control.rs` (nuevo), `portability/mod.rs` | `ImportControl`, `StopCause`, `Counters`, `Progress` y el gancho de pruebas. Los `pub use` |
| `crates/clawg-core/src/memory/portability/import.rs` | `ImportInput`, `import_parsed`, `Imported`, `Stopped`, `Cut`, `or_stop`, `prepare_parsed`, el control en `plan`, `prepare_export` y `write`, el `cut` de `Written`, `batch_notes`, `import` e `import_foreign` como `#[cfg(test)]`, los ayudantes `file_of` y `notes_of`, y pruebas |
| `crates/clawg-core/src/memory/portability/timing.rs` | `Step::name` pública |
| `crates/clawg-core/src/memory/embedder.rs` | `GatedEmbedder` y `Gate`, bajo `cfg(any(test, feature = "testing"))` |
| `crates/clawg-cli/src/daemon/imports.rs` (nuevo), `daemon/mod.rs` | el registro y sus pruebas |
| `crates/clawg-cli/src/daemon/api.rs` | `imports` en `AppCtx`, `job_id` en el error, las piezas comunes, las rutas, `accept_job`, `run_job`, el bloque de `/health`, y pruebas |
| `crates/clawg-cli/src/daemon/serve.rs` | `close` al observar la señal, la espera dentro de la gracia, `abandon` y la advertencia |
| `crates/clawg-cli/src/client.rs` | `start_import`, `import_job`, `cancel_import`, `ImportJob`, `route_missing`, `job_id` en `ErrorDetails`; se va `import` |
| `crates/clawg-cli/src/import.rs`, `cli.rs`, `main.rs` | `--follow`, el seguimiento, Ctrl-C y las frases en los dos idiomas |
| `stop.rs`, `socket/mod.rs`, `mcp/mod.rs`, `mcp/server.rs`, `tests/api.rs`, y en `clawg-eval` `revision-eval/run.rs` y `rag-eval/agent_view/runner.rs` | el campo `imports` |
| `crates/clawg-cli/tests/api.rs`, `tests/binary.rs`, `tests/import_memory.rs` (nuevo) | pruebas, filas de `PROJECT_INPUTS` y la medición |
| `.claude/CLAUDE.md` | `clawg import`, el apagado de `clawg serve` y la API del daemon |

## Riesgos

- **Un archivo que otro nombra.** El daemon lee una ruta con la confianza de `projects add` (decisión 1). Los motivos de un `400` de parseo citan el error del parser, que para un archivo que no es un export puede incluir un fragmento del texto, como hoy por cuerpo. La ruta no se escribe en el log.
- **Un `POST` lento.** Leer y parsear 256 MiB corre dentro del `POST`, en un hilo bloqueante y no en el runtime. Un Ctrl-C del CLI durante ese tiempo se atiende recién con el `202`, porque el `POST` no compite con la señal (D8).
- **Ids que se saltean.** Una reserva que termina en `400` gasta su id. El `409` que la nombró apunta a un id cuyo `GET` es `404`.
- **Un trabajo terminado que se pierde.** Si otro trabajo empieza y termina entre dos sondeos de un CLI, el último terminado ya es otro. El CLI lo trata como desaparición (D8). Con un trabajo a la vez y sondeos de 1 s, hace falta otro cliente justo en ese segundo.
- **Lo que queda corriendo tras una parada.** La inferencia del lote abandonado (hasta ~3,5 s), la traducción u orden en su hilo bloqueante y, en el apagado, lo que `RUNTIME_TEARDOWN` corta. Nada de eso escribe.
- **El reloj pausado.** Avanza cuando ninguna tarea de tokio puede correr, aunque el hilo de la conexión esté trabajando: tokio 1.51.6 frena el avance sólo mientras corre un `spawn_blocking` (`runtime/blocking/schedule.rs:25`), y la conexión es un `thread::spawn` (`tokio-rusqlite-0.7.0/src/lib.rs:377`). Sólo las pruebas que quieren que la gracia venza lo usan (D9).
- **El fallback de axum.** La detección del daemon anterior depende de que esta build no declare un `fallback` que conteste JSON. La prueba de IJ16 corre contra un servidor que usa el de axum, y `route_missing` tiene su prueba.

## Migración

No hace falta. Ni el esquema, ni los ids, ni los estados, ni `metrics.db`, `daemon.json` o `preferences.json` cambian. El trabajo vive en la memoria del proceso. Una importación cortada se deshace con `clawg import --undo N`, que existe desde V26 en las dos versiones, o se termina importando otra vez. Volver atrás es revertir los commits.
