# ram-footprint-gates — Diseño

C (RM1 a RM3 de la [spec](spec.md)), F (RM4 y RM5) y el arnés de la puerta de C. Las citas son del código del 2026-10-29. Lo que se dice de ONNX Runtime 1.24.2 y de protobuf, por dentro, es una predicción de su código y no está medido. Lo que se dice de `memmap2` sale de su documentación: su fuente no está en el registro local, y se comprueba al aplicar. Nada de esto toca `memory.db`, su esquema, los ids, los estados ni el contrato de embedding. F lee y escribe sólo `project_doc_*`, que se deriva del disco y se puede rehacer.

## Frente a la spec

La spec y el diseño se escribieron en paralelo. La spec ya se alineó con la columna «Diseño».

| Punto | Spec | Diseño | Por qué |
|---|---|---|---|
| Dónde vive `a_repair_waits_for_a_load_to_release_the_model_file` (RM2) | `bootstrap.rs` | `model_download.rs` | El candado y el renombre viven ahí (D2). `bootstrap.rs` sólo pasa el candado |
| Texto del pico (RM3) | `private peak rose to 1.6 GiB` | `; private peak rose to 1638.4 MiB` y `; private peak stayed at 1622.0 MiB, set before this load` | La línea ya imprime la memoria con `mib` (`metrics/mod.rs:18`), en MiB con un decimal |
| La sonda del pico (RM3) | una `fn() -> Option<u64>` aparte | dos `MemoryProbe` juntas en `MemoryProbes` (D5) | Es el mismo tipo de función. Juntas, las llamadas de los tests que pasan `None` no cambian |
| Test de la línea `indexed the docs of …` (RM5) | `crates/clawg-cli/tests/api.rs` | test de unidad de `refresh_line`, una función pura de `api.rs` | La línea se arma hoy dentro de `log::info!` (`api.rs:2674-2677`), y un test de integración tendría que capturar el log |
| Test con el modelo real de F (RM5) | `eval_api.rs` | `project_docs/mod.rs`, `#[ignore]`, con `CLAWG_E5_MODEL_DIR` como `tests/e5_golden.rs:9-14` | `refresh` es del crate (`mod.rs:407`), y el modelo se carga con funciones del crate. No hace falta abrir `eval_api` |
| Sólo pasa C′ | no lo dice | queda C′ | Pasó su puerta, y es el mecanismo del cambio |

## D1. El tipo que lleva el ONNX verificado

Hoy `ModelFiles::onnx` es un `Vec<u8>` (`model_download.rs:301`), y `read_verified` lo llena con `std::fs::read` (`:605-606`). `E5Embedder::from_parts` ya toma `&[u8]` (`embedder.rs:543-549`). ort pasa el puntero y el largo, y `Session` no toma prestado el buffer: `commit_from_memory(&[u8]) -> Result<Session>` (`ort-2.0.0-rc.12/src/session/builder/impl_commit.rs:93-115`). El préstamo sólo aparece en `commit_from_memory_directly`, que devuelve `InMemorySession<'m>` (`:82-89`).

```rust
/// `model.onnx` as it was hashed: a read-only view of the file, held open.
pub struct OnnxFile(View);

enum View {
    // Field order is drop order: unmap, close the handle, and only then let a
    // waiting repair rename over the file.
    Mapped { map: memmap2::Mmap, _file: std::fs::File, _cache: OwnedRwLockReadGuard<()> },
    #[cfg(any(test, feature = "testing"))]
    InMemory(Vec<u8>),
}
impl std::ops::Deref for OnnxFile { type Target = [u8]; /* … */ }
```

| Opción | A favor | En contra |
|---|---|---|
| **`OnnxFile`: la vista, su handle y el candado de lectura, en un tipo (elegida)** | Quien tiene la vista tiene el candado, y ningún llamador puede olvidarlo. `Deref<Target = [u8]>` deja igual a `from_parts`, a `model_loader` (`bootstrap.rs:249-254`) y a `eval_api.rs:567`. Se suelta cuando `model_loader` deja caer `files`, haya armado la sesión o no | La variante `InMemory` existe sólo en las builds de test |
| Un `Mmap` suelto, con el candado en cada llamador | Un tipo menos | Son tres llamadores (`bootstrap.rs:624-632`, `:462-470` y `eval_api.rs:561`) con dos `spawn_blocking` y un `.await` en medio. Cada uno tendría que llevar el candado a mano |
| Un `enum` público con `Owned(Vec<u8>)` en producción | Sirve a los tests sin `cfg` | Deja abierta en producción la ruta de la copia que este cambio saca |

Los llamadores, uno por uno:

- La primera carga (`ensure`) y la recarga (`cached`) devuelven `ModelFiles` con la vista.
- `TestEmbedder::deps` (`bootstrap.rs:356-357`) y `empty_model_files` (`:913-915`) usan `OnnxFile::in_memory(Vec::new())`, bajo `cfg(any(test, feature = "testing"))`. Un `FakeEmbedder` no lee el ONNX.
- Los tests de `model_download.rs` comparan `&*model.onnx` con los bytes del fixture (`:1590`, `:1610`).

**La apertura y el mapeo.** `verified_onnx(path, sha, guard)` corre en el hilo bloqueante de `check_cache` (`:532-540`):

1. Abre el archivo. En Windows lo hace con `OpenOptionsExt::share_mode(FILE_SHARE_READ)`: una constante local `0x1` con el nombre de Win32. `clawg-core` no depende de `windows-sys`, y sumarlo por una constante no se justifica.
2. Lee el largo. Un largo 0 es `Altered` sin mapear: ningún archivo del modelo está vacío, y así no se depende de cómo trata `memmap2` un mapeo vacío en cada sistema.
3. Mapea con `Mmap::map`, nunca con `map_copy`.
4. Calcula el SHA-256 de `&map[..]`.

El `unsafe` lleva este comentario:

```rust
// SAFETY: read-only mapping that lives no longer than `file`. On Windows
// `file` shares neither writing nor deletion, so no handle can write,
// truncate, rename or delete it while the view exists; on Unix the data
// directory is 0700 and clawg never writes model.onnx in place, it
// downloads to `.partial` and renames.
```

Los archivos chicos siguen pasando por `read_verified`, y su copia en un `Vec` se queda.

## D2. El candado entre la carga y la reparación

La reparación es `ensure` en el mismo proceso (`bootstrap.rs:489-514`). La guarda sólo la marca un `AtomicBool` (`:453`, `:504`), que nada coordina con una carga. Con la guarda de D1, un renombre sobre un `model.onnx` abierto fallaría en Windows después de bajar 448 MiB.

**Elegida:** `CacheLock(Arc<tokio::sync::RwLock<()>>)`, en `model_download.rs`.

- **Lectura.** El candado lo toma con `read_owned().await` (tokio 1.51.6, `sync/rwlock.rs:584`) la parte async de `check_cache_off_runtime`, antes del `spawn_blocking`. El guard viaja al hilo bloqueante. Si el ONNX verifica, entra en la `OnnxFile`, y si no, se suelta ahí mismo.
- **Escritura.** La toma `download_one`, sólo para `FileKind::Onnx`. Lo hace dentro del `spawn_blocking` que ya hace `flush`, `sync_all` y renombra (`:804-814`): con `blocking_write()` (`rwlock.rs:877-879`), después del `sync_all` y alrededor de `commit_file` (`:875`). Así ninguna recarga espera mientras se sincroniza medio GB.
- **Dónde vive.** `production_deps` crea uno y lo capturan los cierres `ensure` y `cached` (`bootstrap.rs:294-297`). `serve.rs:147-162` pasa esos mismos cierres a la recarga, a la reparación y a la primera carga, así que el candado es uno por directorio de modelos y por proceso. `ensure_model` y `load_cached_model` lo reciben como parámetro, y `ModelSource` lo lleva. `eval_api` y `model_is_cached` (`:586-589`) crean uno propio.
- **Sin bloqueos mutuos.** Una corrida de `fetch` con el ONNX válido retiene su lectura mientras baja archivos chicos, cuyo renombre no toma el candado. Con el ONNX inválido, la lectura ya se soltó cuando llega su escritura.

| Opción | A favor | En contra |
|---|---|---|
| **`RwLock` de tokio con guards dueños (elegida)** | Varias lecturas conviven: la de una reparación que retiene un ONNX válido mientras baja archivos chicos no frena una recarga. El guard es `Send`, así que no hay un guard de `std` cruzando un `.await` | — |
| `Mutex` de tokio (`lock_owned`, `blocking_lock_owned`, `mutex.rs:578`) | Más simple | Esa reparación frenaría cada recarga durante toda la descarga (hasta 3 intentos, con 60 s de lectura y 5 s y 30 s de espera, `:243`, `:390-393`) |
| Juntar en un `spawn_blocking` la verificación y la sesión | Sin candado entre tareas | La primera carga separa `ensure` de `load` por diseño (`bootstrap.rs:624-632`, cierres inyectados). Y no cubre la reparación |
| Un candado de archivo (`fs2`) | Cubre otros procesos | Ningún otro proceso escribe en `models/` (RM2) |

## D3. El choque al abrir

`check_cache` cuenta como faltante cualquier error de lectura (`model_download.rs:577`).

- **El clasificador.** `fn is_sharing_conflict(raw: Option<i32>) -> bool` es pura. En Windows es `matches!(raw, Some(5 | 32 | 33))` (`ERROR_ACCESS_DENIED`, `ERROR_SHARING_VIOLATION` y `ERROR_LOCK_VIOLATION`, constantes locales) y en los demás sistemas, `false`. Nunca compara `ErrorKind`.
- **El reintento.** Esperas de `[15, 30, 60, 120, 240, 480, 960]` ms (`OPEN_RETRY_WAITS_MS`): ocho intentos y 1,9 s de espera en total, con `std::thread::sleep`, en el hilo bloqueante. Un test fija que la suma no pasa de 2 s. Vale para todos los archivos de la caché, porque un archivo ocupado nunca es uno que falta.
- **Lo que queda.** Si sigue chocando, el archivo da `Unusable::Busy(io::Error)`, y `CacheCheck` lo pone en `busy`, no en `missing`. `fetch` y `load_cached_from` devuelven `DownloadError::Busy { file, source }` antes de mirar lo que falta, así que no se baja nada. `load_verified_tokenizer_with` (`:170-185`) suma un brazo para ese caso: se trocea por bytes, como hoy.
- **La recarga.** La recarga (`bootstrap.rs:462-467`) consulta `needs_repair(&DownloadError) -> bool`, que es pura y falsa sólo para `Busy`. Con `Busy` contesta `Err`, e `IdleEmbedder` lo convierte en `NotReady` con su `warn` (`idle_embedder.rs:172-175`). No llama a `repair_in_background`.
- **El arranque.** El error queda como `BootstrapError::Download`, que se reintenta (`run_embedder_bootstrap_with_retry`), y el estado es `Failed { retryable: true }`.

Un test inyecta un `cached` que devuelve `Busy`, siguiendo el patrón de `bootstrap.rs:1043-1109`, y comprueba que `ensure` corre 0 veces. Los tests de Windows sostienen el archivo con `share_mode(0)`.

## D4. La corrida que baja, sin el cuerpo

Hoy `download_one` junta `body` (`:730`, `:750`) y lo devuelve (`:822`), y `fetch` lo empuja a `found` (`:491-503`).

**Elegida:** `download_one -> Result<Option<Vec<u8>>, DownloadError>`.

- `None` para `FileKind::Onnx`, que no acumula nada. `Some(body)` para los archivos chicos, que se siguen entregando desde memoria (RM1).
- El chequeo del tope (`:738`; la constante `MAX_FILE_BYTES`, `:836`) se hace con un contador de bytes recibidos, no con `body.len()`.
- Después de bajar, `fetch` verifica el ONNX con el mismo `check_cache_off_runtime` sobre la especificación del ONNX. Así toma la lectura y mapea como una recarga.
- Si esa segunda verificación falla, da `NotCached` para el ONNX: el arranque reintenta, y la caché pide bajarlo de nuevo.
- `each_file_is_hashed_once_per_run` (`:1575-1599`) pasa a esperar el ONNX bajado una vez en `READS` y los chicos bajados ninguna.

| Opción | A favor | En contra |
|---|---|---|
| **Sólo el ONNX sin cuerpo (elegida)** | Es lo que pide RM1. Los chicos no se releen | `download_one` devuelve dos formas |
| No guardar el cuerpo de ninguno y verificar todo desde la caché | Una sola forma | Hashea dos veces también los chicos, y RM1 dice que no se releen |

Una reparación del ONNX lo hashea tres veces: al bajar, al verificar desde la caché (y suelta lo verificado) y en la recarga siguiente. Una reparación es rara, y no vale la pena una ruta más.

## D5. El pico de cada carga

`IdleEmbedder` lee la memoria con `Option<MemoryProbe>` (`idle_embedder.rs:125`, `:167-180`), y `clawg serve` le pasa `private_bytes` (`serve.rs:153`, `process.rs:33-37`).

| Opción | A favor | En contra |
|---|---|---|
| **`pub struct MemoryProbes { pub private: MemoryProbe, pub private_peak: MemoryProbe }` en `metrics/mod.rs`, en lugar de `Option<MemoryProbe>` (elegida)** | Los diez `None` de los tests de `idle_embedder.rs`, los cuatro de `unloading_when_idle` en `bootstrap.rs` y el de `crates/clawg-cli/tests/api.rs:275` no cambian. Las dos lecturas van juntas | Un tipo más |
| Un segundo parámetro `Option<MemoryProbe>` | Sin tipo nuevo | Cambian `IdleEmbedder::new`, `unloading_when_idle` y todas sus llamadas |
| Cambiar `MemoryProbe` a un par | — | `ImportTimings::with_memory` también lo usa (`api.rs:2011`, `portability/timing.rs:152`) |

- **La lectura.** `process.rs` suma `pub fn private_peak_bytes() -> Option<u64>`, que es `os_sample().private_peak_bytes`, `PeakPagefileUsage` en `:114`. En Linux es `None` (`:16-18`).
- **La línea.** `load_line(cause, took, memory: Option<(u64, u64)>, peak: Option<(u64, u64)>)` sigue pura y agrega un final después del de hoy:
  - `; private peak rose to {mib(after)}` si `after > before`;
  - `; private peak stayed at {mib(before)}, set before this load` si no subió;
  - nada, sin sonda o con `None`.
- **Lo que nombra.** El valor es el pico del proceso en la ventana de la carga. Otra tarea que corra a la vez, como el conteo de tokens de un import, puede ser la que lo subió.
- **El alcance.** Va sólo al log: `metrics.db` no cambia. La primera carga no pasa por `load_line`, y su pico es el `process.private_peak_bytes` de su minuto, como hoy.

## D6. F, la reutilización

**La lectura.** Va en la conexión (`conn.call`), una vez por cada doc de `plan.index` que ya tiene una huella (los docs nuevos no consultan):

```sql
SELECT c.breadcrumb, c.body, v.embedding
FROM project_doc_chunk c JOIN project_doc_vectors v ON v.rowid = c.id
WHERE c.project_id = ?1 AND c.path = ?2
  AND EXISTS (SELECT 1 FROM project_doc_file f
              WHERE f.project_id = ?1 AND f.path = ?2 AND f.model_id = ?3)
ORDER BY c.ordinal
```

- **El modelo.** Se compara en la misma sentencia que lee los vectores, con el `model_id` que se lee una vez (`mod.rs:420`).
- **La lectura por rowid.** Una tabla vec0 se lee por rowid aunque tenga clave de partición. sqlite-vec 0.1.9 toma una igualdad sobre el rowid como plan `POINT`, de costo 10 contra 3·10⁶ del recorrido completo (`sqlite-vec.c:5517-5528`, `:5827-5839`), y `vec0Filter_point` lee el vector por rowid sin mirar la partición (`:7376-7395`). La forma `JOIN … ON v.rowid = p.id` ya corre en producción sobre `memory_vectors` (`maintenance.rs:741-747`).
- **El esquema.** No tiene un `UNIQUE` sobre el texto (`V12__project_docs.sql:22-31`).

**La decisión.** Es pura y corre en la tarea async del refresco. La conexión es compartida, y el reparto es O(n) sobre un `HashMap`.

- La clave es `Passage { ordinal, breadcrumb, body }.indexed_text()` (`chunk.rs:28-30`): la misma fórmula que se embebe, con el texto entero y sin hash. Con `entry().or_insert`, gana el primer ordinal.
- `fn reuse_plan(passages: &[Passage], stored: &HashMap<String, Vec<u8>>) -> Reuse { vectors: Vec<Option<Vec<u8>>>, to_embed: Vec<usize> }`. Un texto repetido toma el blob en cada uno de sus pasajes.

**El embebido y la escritura.**

- `embed_passages` (`:464-489`) pasa a recibir los textos de `to_embed`. Sin textos, vuelve sin llamar al embebedor, como ya hace (`:469-471`). Su manejo de `Vector` y `VectorCount` no cambia.
- Una función pura `fill(reuse, new)` arma el `Vec<Vec<u8>>` en el orden de los pasajes. `replace_doc` no cambia (`:544-577`).
- `IndexedDoc.vectors` ya lleva bytes LE después de `to_unit` (`:486-493`, `:539`). El blob guardado entra tal cual, byte a byte.

| Opción | A favor | En contra |
|---|---|---|
| **Llevar el blob guardado (elegida)** | Es idéntico byte a byte sin código de conversión | — |
| Decodificar el f32 LE y volver a codificar | El tipo de `vectors` sería `f32` | La ida y vuelta es exacta, pero es código de más. Y si el vector volviera a pasar por `to_unit`, que divide por una norma en f64 (`vector.rs:53-61`), cambiarían bits de un vector ya unitario |
| La decisión en la conexión | No cruza el mapa | Ocupa el hilo que comparten las saves y las búsquedas |

**La cuenta.** `RefreshReport` suma `reused: usize`, en pasajes. Se suma junto a `report.passages`, sólo cuando el doc se escribe: un doc que falla no cuenta (`mod.rs:446-457`). `refresh_line(project_id, &RefreshReport) -> String`, en `api.rs`, deja la línea igual y le agrega `, {} reused` al final, así lo que ya leía sus campos sigue leyéndolos.

**Las pruebas.**

- Con el embebedor falso, que vuelve `FakeEmbedder` con otro `model_id` y anota textos, como `Counting` (`mod.rs:632-656`), se comprueba que el doc es idéntico al de un reindexado completo.
- `a_reused_vector_is_the_stored_blob` usa ese embebedor con una «sal» que cambia entre refrescos y no cambia el id. Un pasaje reutilizado guarda el blob de la sal vieja, cosa que un re-embebido determinista no podría distinguir.

## D7. El test del contrato

`the_embedding_contract_is_pinned` va en `embedder.rs`, en `mod tests`, y es una tabla de literales.

- Fija `E5_MODEL_ID` (`:135`), `E5_QUERY_PREFIX` (`:109`), `E5_PASSAGE_PREFIX` (`:130`), `EMBED_MAX_TOKENS` (`:166`), `EMBEDDING_DIM` (`vector.rs:8`) y `MODEL_SHA256`, `TOKENIZER_JSON_SHA256`, `CONFIG_JSON_SHA256`, `SPECIAL_TOKENS_MAP_SHA256` y `TOKENIZER_CONFIG_SHA256` (`model_download.rs:62-71`).
- El mensaje de cada aserción pide revisar si `E5_MODEL_ID` sube.
- **El pooling.** El pooling es código, y el test lo fija de dos maneras. Por el nombre: el sufijo `/mean` del id. Por el resultado: los bits de `pool_and_normalize` (`:290`) sobre una entrada chica y fija, de forma `[1, 3, 2]` con máscara `[1, 1, 0]`, escritos como literales desde el código de hoy. La fórmula ya la contrasta con la de fastembed `mean_pooling_matches_fastembeds_formula_bit_for_bit` (`:986`). La sonda fija lo que el id nombra.

| Opción | A favor | En contra |
|---|---|---|
| **Tabla de literales (elegida)** | Un fallo nombra el campo, y quien edita ve `E5_MODEL_ID` al lado | Repite literales que ya fijan `:1119`, `:1176` y `:1492`, que se quedan |
| Un digest de todos los valores | Una línea | Un fallo no dice qué cambió, y hay que recalcular el digest a mano |

**Lo que no fija.**

- `EMBED_BATCH`, la versión de ort u ONNX Runtime y las opciones de sesión. Mueven los bits del orden de 1e-7: lo midió la revisión en un codificador sintético, sin medirlo con este modelo.
- Con el modelo real lo detecta `tests/e5_golden.rs`, ignorado.

## D8. El arnés de la puerta de C

| Opción | A favor | En contra |
|---|---|---|
| **Un binario `load-eval` en `clawg-eval` (elegida)** | Es la casa de las mediciones, y enlaza `clawg-core` sólo por `eval`, que ninguna build de release enlaza. La decisión es de tres salidas, como el `decision` de `revision-eval`, y no un pasa o falla. Las reglas de la puerta quedan en una función pura con tests, commiteada antes de medir. El hijo lee sus contadores con `clawg_cli::daemon::process::sample()` (`process.rs:27`), porque `clawg-eval` ya depende de `clawg-cli` (`crates/clawg-eval/Cargo.toml:38-50`) | Suma `windows-sys` a `clawg-eval`, para el job object |
| Un test `#[ignore]` como `transport_bar.rs` (`:11`, `:308`) | Tiene precedente | Volver a ejecutar un binario de libtest como hijo exige `--exact` y mezcla su salida. El brazo C necesitaría `ort` directo, fuera de la función de carga real |
| Un arnés temporal, sin commitear | No se mantiene | Las reglas no quedan fijadas antes de medir, y el seguimiento de datos externos o `.ort` tendría que rehacer la misma puerta |

**Los brazos, sin `commit_from_file` en producción.**

- `build_session` (`embedder.rs:513-526`) se parte en `pub(crate) fn session_builder()`, con las opciones de hoy (CPU sin arena, `Level3`, un hilo por CPU), y en el commit.
- `eval_api` suma, bajo `eval`, `pub fn load_once(model_dir, LoadArm) -> Result<LoadTiming { verify_ms, session_ms }, EvalError>`:
  - `Vec`: `std::fs::read` y SHA-256, luego `commit_from_memory`;
  - `File`: el handle con `share_mode(FILE_SHARE_READ)`, un hash por bloques de 1 MiB y `commit_from_file(path)` (`impl_commit.rs:45-56`) con el handle abierto;
  - `View`: `verified_onnx` de producción y luego `build_session`.
- También suma `pub fn fetch_model(dir)`.
- La build de release (`-p clawg-cli`) no compila nada de esto.

**La corrida.**

- **El modelo.** `load-eval gate [--model-dir D] [--loads 10] [--tag T]` baja el modelo a `load-eval-<uuid>` en el temporal, como exige `revision-eval` para sus directorios (`main.rs:44-52`). Sólo acepta un `--model-dir` de ese tipo, así que nunca lee un perfil. Al terminar lo borra, haya fallado la corrida o no.
- **El orden.** `schedule(loads)` es pura: once rondas, cada una con los tres brazos en orden rotado. La ronda 0 es el calentamiento y se descarta.
- **Cada carga.** Corre en `load-eval child --arm A --model-dir D`:
  - en Windows, el padre crea un job con `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, le asigna el hijo y recién entonces le escribe `go` por stdin;
  - el hijo carga, lee `sample()` (`PeakPagefileUsage`, `PeakWorkingSetSize`, `PrivateUsage`, y `VmHWM` y `RssAnon` en Linux) y escribe una línea JSON;
  - el padre lee `PeakProcessMemoryUsed` con `QueryInformationJobObject(JobObjectExtendedLimitInformation)`.
  - Hacen falta `windows-sys` 0.61 con `Win32_Foundation`, `Win32_Security` (`CreateJobObjectW`), `Win32_System_JobObjects` y `Win32_System_Threading` (para `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`), en `[target.'cfg(windows)'.dependencies]` de `clawg-eval` (`windows-sys-0.61.2/src/Windows/Win32/System/JobObjects/mod.rs:2-5`, `:113-121`, `:421`, `:481`). Es la versión que ya usa `clawg-cli` (`Cargo.toml:62-63`), así que `Cargo.lock` no suma ningún paquete.
  - En Linux, un proceso por carga deja `VmHWM` limpio, sin `clear_refs`.
- **El veredicto.** `verdict(samples)` es pura y aplica las tres reglas de la puerta sobre el pico del job y el tiempo de verificar más armar la sesión. Si pasan los dos brazos, o sólo `View`, devuelve `View`. Si pasa sólo `File`, devuelve `File`. Si no pasa ninguno, `Revert`. En Linux informa y no decide.
- **El resultado.** Sale por stdout y en `docs/eval/results/{date}-real-load-eval[-T].json`, que nunca reemplaza otro, fuera de git.
- **Lo que hace falta para correrla.** Windows, autorización para el build release y para la descarga (~470 MB), y `cargo run -p clawg-eval --release --target-dir target-test --bin load-eval -- gate`.

## D9. El orden de los commits

Cada commit escribe primero su test que falla, compila y pasa la suite. C y F son independientes. Las líneas de `.claude/CLAUDE.md` que describen un comportamiento cambian en el mismo commit que lo cambia.

1. `feat(embedder): each load names the private peak it reached` (RM3, D5).
2. `fix(model): a model file another program holds answers not ready without a download` (D3). Va antes que la guarda, porque abrir sin compartir escritura ni borrado hace más probable un 32 frente a un antivirus que tiene el archivo con acceso de borrado. Hoy vale solo, porque `std::fs::read` también puede chocar.
3. `perf(model): load model.onnx from the read-only view its hash was taken on` (D1, D2; `memmap2` en `[workspace.dependencies]`, en `clawg-core` y en `Cargo.lock`). Desde este commit, `fetch` suelta el cuerpo del ONNX que bajó y lo verifica desde la caché (la segunda verificación de D4). Así, ningún `OnnxFile` de producción sale de bytes en memoria, y el commit compila sin D4. Después: `cargo clean --target-dir target-test`.
4. `perf(model): the download run no longer holds model.onnx in memory` (D4: `download_one` deja de juntar el cuerpo del ONNX).
5. `feat(eval): measure the model load by arm, one process per load` (D8). Después viene la corrida, con autorización. Si da `File`, un commit cambia el mecanismo y RM1 se reescribe. Si da `Revert`, se revierten 4 y 3. El arnés queda para el seguimiento de datos externos.
6. `test(embedder): pin the embedding contract together with its model id` (D7). Tiene que entrar antes que el 7.
7. `perf(docs): a refresh keeps the vector of every passage whose text the index holds` (D6, con `reused` y la línea).

La fila 92 de `openspec/README.md` se actualiza al archivar, junto con la subida de la versión.

## Archivos que cambian

| Archivo | Qué |
|---|---|
| `Cargo.toml`, `crates/clawg-core/Cargo.toml`, `Cargo.lock` | `memmap2 = "0.9"` |
| `crates/clawg-core/src/memory/model_download.rs` | `OnnxFile`, `CacheLock`, `verified_onnx`, `is_sharing_conflict`, el reintento, `Unusable::Busy` y `DownloadError::Busy`, `download_one` sin cuerpo para el ONNX, el candado alrededor del renombre, tests |
| `crates/clawg-core/src/memory/bootstrap.rs` | el candado en `production_deps`, `needs_repair` en la recarga, `OnnxFile::in_memory` en `TestEmbedder`, `MemoryProbes`, tests |
| `crates/clawg-core/src/memory/embedder.rs` | `session_builder`, `build_session` como `pub(crate)`, el test del contrato y el de la sesión desde la vista |
| `crates/clawg-core/src/memory/idle_embedder.rs`, `crates/clawg-core/src/metrics/mod.rs` | `MemoryProbes` y el final del pico en `load_line` |
| `crates/clawg-cli/src/daemon/process.rs`, `crates/clawg-cli/src/daemon/serve.rs` | `private_peak_bytes` y cómo se conecta |
| `crates/clawg-core/src/memory/project_docs/mod.rs` | la lectura por doc, `reuse_plan`, `fill`, `reused`, tests y el `#[ignore]` real |
| `crates/clawg-cli/src/daemon/api.rs` | `refresh_line` y su test |
| `crates/clawg-core/src/eval_api.rs` | el candado en `real` y `model_is_cached`, `load_once`, `fetch_model` |
| `crates/clawg-eval/Cargo.toml`, `crates/clawg-eval/src/bin/load-eval/` | el arnés y `windows-sys` para Windows |
| `.claude/CLAUDE.md` | la descarga, la recarga, el log de cada carga y el índice de docs |

## Riesgos

- **Unix.** Quien escriba en el directorio de datos puede cambiar el archivo en el lugar durante la carga, o truncarlo (`SIGBUS`). La guarda sigue siendo el `0700`, y clawg nunca escribe en el lugar.
- **Otro proceso.** El candado es del proceso. Una herramienta de evaluación que mapea el `model.onnx` de un perfil (`e5_golden`, `model_is_cached`) durante ~1 s hace fallar el renombre de una reparación de ese daemon en Windows. La reparación queda en un `warn`, y la siguiente recarga vuelve a bajar el archivo. `load-eval` rechaza los perfiles.
- **Un antivirus.** Un antivirus que revisa el ONNX recién bajado puede pasar de los 2 s. Entonces la primera carga falla reintentable y vuelve a probar a los 30 s, sin bajar nada.
- **Un error 5 que dura.** Un error 5 persistente, por ejemplo una ACL rota, pasa a ser «ocupado» y nunca se repara. Cada recarga nombra la causa en el log. Hoy bajaría 448 MiB y casi seguro fallaría al escribir.
- **El working set.** Durante el hash y el parse entran hasta 448 MiB de páginas del archivo, que se pueden compartir y no cuentan en el commit ni en «Memoria» del Administrador de tareas. La puerta lee los dos contadores.
- **La deriva que F vuelve permanente.** Si el contrato cambia sin que cambie el id, los pasajes sin cambios conservan el vector viejo. D7 cubre lo que fija. La versión de ort y el nivel de optimización quedan para `e5_golden`.
- **El ahorro de C.** Se predice del código de ONNX Runtime, sin medir: −272 MiB para C y −448 para C′. Sólo la corrida de D8 decide.
- **`memmap2`.** Lo que se dice de sus banderas en Windows y de un mapeo vacío no se pudo leer en su fuente. Ningún paso depende del mapeo vacío.

## Migración

No hace falta. Ni el esquema ni los datos cambian. El índice de docs que deja F es equivalente al de un reindexado, y un reindexado lo rehace. Volver atrás es revertir los commits.
