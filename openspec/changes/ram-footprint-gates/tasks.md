# ram-footprint-gates — Tareas

Strict TDD: en cada commit el test que falla va primero, con el nombre exacto de la [spec](spec.md), y falla por la aserción o por no compilar. Un commit convencional y atómico, que compila y pasa la suite; se prepara por ruta explícita (nunca `git add -A`, `.` ni `-u`), sin `Co-Authored-By` ni atribución. Ni el código, ni los comentarios, ni los nombres de test, ni los mensajes de commit nombran el cambio, la spec ni fases. El orden es el de D9 del [diseño](design.md).

Comandos de cada commit, en Windows y en `target-test`:

- Suite: `cargo test --workspace --target-dir target-test`.
- Clippy: `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`.
- Dentro del ciclo de un commit se reusa el directorio. Antes de empezar la tanda: `cargo clean --workspace --target-dir target-test`. Después de un cambio de dependencias en `Cargo.lock`: `cargo clean --target-dir target-test`. No se crea otro directorio de build.
- No hay builds release, instalaciones ni acceso a `~/.clawg` ni `~/.clawg-dev` sin autorización. No se mata ningún `clawg.exe`.

Revisado: ni `agent_contract.json` ni `agent_contract.es.json` cambian (ninguna herramienta, argumento ni protocolo se toca). `crates/clawg-cli/tests/conventions.rs` no fija nada de los binarios de `clawg-eval`, de su `Cargo.toml` ni de `windows-sys`; sólo vigila `testing` fuera de `[dev-dependencies]` (el `eval` de `clawg-eval` no es `testing`), los workflows y los metadatos del manifiesto. Comprobar al aplicar el commit 3 y el 5 que sigue en verde.

## 1. `feat(embedder): each load names the private peak it reached` (RM3, D5)

- [x] Test primero (`crates/clawg-core/src/memory/idle_embedder.rs`): `a_load_line_names_the_peak_when_the_load_raised_it`. Falla: no existe `MemoryProbes` ni el final `; private peak rose to …` (no compila).
- [x] Test primero (`idle_embedder.rs`): `a_load_line_says_when_the_load_did_not_pass_the_peak`. Falla por lo mismo.
- [x] Test primero (`idle_embedder.rs`): `a_load_line_without_a_peak_probe_is_unchanged`. Falla por no compilar; fija la línea de hoy sin sonda o con `None`.
- [x] Test primero (`crates/clawg-cli/src/daemon/metrics.rs`): `the_peak_is_not_written_to_the_metrics_file`. Falla por no compilar contra la sonda nueva; fija que `metrics.db` no cambia.
- [x] `crates/clawg-core/src/metrics/mod.rs`: `pub struct MemoryProbes { pub private: MemoryProbe, pub private_peak: MemoryProbe }`.
- [x] `idle_embedder.rs`: `IdleEmbedder` guarda `Option<MemoryProbes>` en lugar de `Option<MemoryProbe>`; lee el pico antes y después de la carga. `load_line(cause, took, memory: Option<(u64, u64)>, peak: Option<(u64, u64)>)` sigue pura y suma el final: `; private peak rose to {mib(after)}` si `after > before`, `; private peak stayed at {mib(before)}, set before this load` si no, nada sin sonda o con `None`.
- [x] `crates/clawg-core/src/memory/bootstrap.rs`: `unloading_when_idle` y sus llamadas pasan `Option<MemoryProbes>`; los `None` de los tests (diez en `idle_embedder.rs`, cuatro en `bootstrap.rs` y uno en `crates/clawg-cli/tests/api.rs:275`) no cambian.
- [x] `crates/clawg-cli/src/daemon/process.rs`: `pub fn private_peak_bytes() -> Option<u64>` como `os_sample().private_peak_bytes` (`PeakPagefileUsage`); `None` en Linux.
- [x] `crates/clawg-cli/src/daemon/serve.rs:153`: pasa `MemoryProbes { private: process::private_bytes, private_peak: process::private_peak_bytes }`.
- [x] `.claude/CLAUDE.md`: la frase de la recarga (`idle_embedder.rs`, «Each such load logs who asked for it …») suma que, en Windows, la línea dice el pico privado de esa carga, o que sigue donde estaba, y que no va a `metrics.db`.
- [x] Verificación: suite y clippy.

## 2. `fix(model): a model file another program holds answers not ready without a download` (D3)

- [x] Test primero (`crates/clawg-core/src/memory/model_download.rs`): `a_persistent_sharing_violation_is_not_a_missing_file_at_start`. Falla: `check_cache` cuenta cualquier error de lectura como `missing` (`Err(_) => missing`, `:577`) y baja de nuevo.
- [x] Test primero (`model_download.rs`, `#[cfg(windows)]`): `a_sharing_violation_on_open_is_retried_within_the_wait`. Sostiene el archivo con `share_mode(0)` y lo suelta antes de 2 s. Falla: no hay reintento.
- [x] Test primero (`model_download.rs`): un test puro de que la suma de `OPEN_RETRY_WAITS_MS` no pasa de 2 s y de que `is_sharing_conflict` acepta 5, 32 y 33 en Windows y nada en los demás sistemas. Falla por no compilar.
- [x] Test primero (`crates/clawg-core/src/memory/bootstrap.rs`): `a_persistent_sharing_violation_answers_not_ready_without_a_repair`, con un `cached` inyectado que devuelve `Busy`, siguiendo el patrón de `bootstrap.rs:1043-1109`; comprueba que `ensure` corre 0 veces y que no se lanza `repair_in_background`. Falla: la recarga repara ante cualquier error.
- [x] `model_download.rs`: `fn is_sharing_conflict(raw: Option<i32>) -> bool` pura (en Windows `matches!(raw, Some(5 | 32 | 33))` con constantes locales; `false` en los demás). Nunca `ErrorKind`.
- [x] `model_download.rs`: `OPEN_RETRY_WAITS_MS = [15, 30, 60, 120, 240, 480, 960]` con `std::thread::sleep` en el hilo bloqueante, para todos los archivos de la caché. `Unusable::Busy(io::Error)`, `CacheCheck::busy` y `DownloadError::Busy { file, source }`, que `fetch` y `load_cached_from` devuelven antes de mirar lo que falta.
- [x] `load_verified_tokenizer_with` (`:170-185`): un brazo para `Busy`, que trocea por bytes como hoy.
- [x] `bootstrap.rs`: `needs_repair(&DownloadError) -> bool`, pura y falsa sólo para `Busy`; la recarga (`:462-467`) contesta `Err` ante `Busy` sin llamar a `repair_in_background`. `IdleEmbedder` lo vuelve `NotReady` con su `warn` (`idle_embedder.rs:172-175`). En el arranque queda `BootstrapError::Download`, reintentable (`Failed { retryable: true }`).
- [x] `.claude/CLAUDE.md`: la frase de la descarga y la de la recarga suman que un archivo del modelo ocupado por otro programa (errores 5, 32 o 33 de Windows, tras unos 2 s de reintentos) no cuenta como faltante, no baja nada y contesta no listo.
- [x] Verificación: suite y clippy.

## 3. `perf(model): load model.onnx from the read-only view its hash was taken on` (D1, D2; RM1, RM2)

- [x] Comprobar al aplicar, antes de escribir código: la versión exacta de `memmap2` en el registro (el diseño dice `0.9`), su manejo de un mapeo vacío (el diseño no depende de él: el largo 0 es `Altered` sin mapear) y que `Mmap::map` usa `PAGE_READONLY` y `FILE_MAP_READ` en Windows. Su fuente no estaba en el registro local.
- [x] Test primero (`model_download.rs`): `the_cache_check_hands_the_onnx_over_as_a_read_only_view`. Falla: `ModelFiles::onnx` es un `Vec<u8>` (`:301`).
- [x] Test primero (`model_download.rs`): `a_view_of_a_changed_onnx_is_refused`. Falla por no compilar contra `OnnxFile`.
- [x] Test primero (`crates/clawg-core/src/memory/embedder.rs`): `a_session_is_built_from_the_verified_view`, con el ONNX mínimo y `commit_from_memory`. Falla por no compilar.
- [x] Test primero (`model_download.rs`, `#[cfg(windows)]`): `a_load_holds_the_model_file_against_writes_until_the_session_returns`, también en un directorio temporal con «ñ». Falla: la carga no retiene el archivo.
- [x] Test primero (`model_download.rs`, `#[cfg(windows)]`): `the_model_file_is_free_again_after_a_failed_load`. Falla por no compilar.
- [x] Test primero (`model_download.rs`): `a_repair_waits_for_a_load_to_release_the_model_file`. Falla: no existe `CacheLock`; nada coordina el renombre con una carga.
- [x] Dependencia: `memmap2` en `[workspace.dependencies]` del `Cargo.toml` raíz y en `crates/clawg-core/Cargo.toml`; `cargo update --workspace` sólo si hace falta escribir `Cargo.lock` sin tocar otras dependencias, y después `cargo clean --target-dir target-test`.
- [x] `model_download.rs`: `OnnxFile(View)` con `enum View { Mapped { map, _file, _cache }, InMemory(Vec<u8>) }` (la segunda bajo `cfg(any(test, feature = "testing"))`; el orden de campos es el orden de drop), `Deref<Target = [u8]>` y `OnnxFile::in_memory`. `ModelFiles::onnx: OnnxFile`.
- [x] `model_download.rs`: `verified_onnx(path, sha, guard)` en el hilo bloqueante de `check_cache`: abre (en Windows con `share_mode(FILE_SHARE_READ)`, constante local `0x1`), lee el largo (0 es `Altered`), mapea con `Mmap::map` (nunca `map_copy`) y hashea `&map[..]`. El `unsafe` lleva el comentario `SAFETY` de D1. Los archivos chicos siguen por `read_verified`.
- [x] `model_download.rs`: `CacheLock(Arc<tokio::sync::RwLock<()>>)`. Lectura con `read_owned().await` en la parte async de `check_cache_off_runtime`, antes del `spawn_blocking`; el guard entra en `OnnxFile` si el ONNX verifica y se suelta si no. Escritura con `blocking_write()` en `download_one`, sólo para `FileKind::Onnx`, después de `sync_all` y alrededor de `commit_file`.
- [x] `ensure_model`, `load_cached_model` y `ModelSource` reciben el candado como parámetro; `production_deps` (`bootstrap.rs:294-297`) crea uno y lo capturan `ensure` y `cached`; `serve.rs:147-162` ya comparte esos cierres. `eval_api.rs` (`real`, `:561`, y `model_is_cached`, `:586-589`) crean uno propio.
- [x] `bootstrap.rs`: `TestEmbedder::deps` (`:356-357`) y `empty_model_files` (`:913-915`) usan `OnnxFile::in_memory(Vec::new())`; los tests de `model_download.rs` comparan `&*model.onnx` con los bytes del fixture (`:1590`, `:1610`).
- [x] `embedder.rs`: `from_parts` ya toma `&[u8]`; comprobar que `model_loader` (`bootstrap.rs:249-254`) y `eval_api.rs:567` compilan por el `Deref` sin cambios.
- [x] Test que cambia (`model_download.rs`): `each_file_is_hashed_once_per_run` (`:1575-1599`) pasa a esperar el ONNX bajado dos veces en `READS`: una por la descarga y otra por la verificación desde la caché. Los chicos bajados quedan en una sola. Falla con el comportamiento de hoy.
- [x] `fetch`: después de bajar y renombrar, suelta el cuerpo del ONNX que devolvió `download_one` y lo verifica con `check_cache_off_runtime` sobre su especificación. Así toma el candado de lectura y mapea como una recarga, y nunca arma un `OnnxFile` desde bytes en memoria, porque `InMemory` no existe en producción. Si esa verificación falla, da `NotCached` para el ONNX. En este commit `download_one` todavía junta el cuerpo, que el 4 saca.
- [x] Se conservan `the_cache_check_leaves_the_runtime_thread_free`, `a_file_larger_than_any_model_file_is_refused_without_holding_it` y `the_worker_resolves_the_tokenizer_off_the_runtime_thread` (`runtime.rs`).
- [x] `.claude/CLAUDE.md`: la frase de la descarga (hash una vez) y la de la recarga cambian: el ONNX verificado llega a la carga como la vista de sólo lectura sobre la que se hasheó, el archivo queda abierto sin compartir escritura ni borrado en Windows hasta que `commit_from_memory` vuelve, y una reparación espera esa carga.
- [x] Verificación: suite y clippy; `cargo clean --target-dir target-test` por el cambio de `Cargo.lock`.

## 4. `perf(model): the download run no longer holds model.onnx in memory` (D4; RM1)

- [x] Test primero (`model_download.rs`): `a_downloaded_onnx_is_not_kept_in_memory_and_is_loaded_from_the_cache`. Falla: `download_one` junta `body` con `Vec::with_capacity(content_length)` (`:730`) y `fetch` lo empuja a `found` (`:491-503`).
- [x] `download_one -> Result<Option<Vec<u8>>, DownloadError>`: `None` para `FileKind::Onnx`, que no acumula nada, y `Some(body)` para los chicos. El chequeo del tope (`:738`; la constante `MAX_FILE_BYTES`, `:836`) se hace con un contador de bytes recibidos, no con `body.len()`. `fetch` ya verifica el ONNX desde la caché (commit 3).
- [x] `.claude/CLAUDE.md`: la frase de la descarga dice que la corrida que baja el ONNX lo escribe a `.partial` con su hash, lo renombra y lo carga desde la caché, y que sólo en esa corrida se hashea dos veces.
- [x] Verificación: suite y clippy.

## 5. `feat(eval): measure the model load by arm, one process per load` (D8)

- [x] Test primero (`crates/clawg-eval/src/bin/load-eval/`, pura): `schedule(loads)`: once rondas, la ronda 0 de calentamiento descartada, los tres brazos en orden rotado en cada ronda. Falla por no compilar.
- [x] Test primero (misma carpeta): `verdict(samples)` con las tres reglas de la puerta (mediana del pico de commit baja al menos 300 MiB, peor carga bajo la mejor de hoy, mediana del tiempo total con caché tibia sin subir más de max(150 ms, 15 %)): pasan los dos brazos o sólo `View` da `View`; sólo `File` da `File`; ninguno da `Revert`; en Linux informa y no decide. Falla por no compilar.
- [x] Test primero: `load-eval gate` rechaza un `--model-dir` que no sea un `load-eval-<uuid>` del temporal (nunca lee un perfil) y borra su directorio aunque la corrida falle.
- [x] `crates/clawg-eval/Cargo.toml`: `[[bin]] name = "load-eval"` con `path = "src/bin/load-eval/main.rs"` (el crate tiene `autobins = false`); en `[target.'cfg(windows)'.dependencies]`, `windows-sys` 0.61 con `Win32_Foundation`, `Win32_Security`, `Win32_System_JobObjects` y `Win32_System_Threading`. Comprobar al aplicar que `Cargo.lock` no suma ningún paquete (es la versión de `clawg-cli`) y, si cambia, `cargo clean --target-dir target-test`.
- [x] `crates/clawg-core/src/memory/embedder.rs`: partir `build_session` (`:513-526`) en `pub(crate) fn session_builder()` (CPU sin arena, `Level3`, un hilo por CPU) y el commit; `build_session` queda `pub(crate)`.
- [x] `crates/clawg-core/src/eval_api.rs` (bajo `eval`): `pub fn load_once(model_dir, LoadArm) -> Result<LoadTiming { verify_ms, session_ms }, EvalError>` con los brazos `Vec` (`std::fs::read`, SHA-256, `commit_from_memory`), `File` (handle con `share_mode(FILE_SHARE_READ)`, hash por bloques de 1 MiB, `commit_from_file(path)` con el handle abierto) y `View` (`verified_onnx` de producción y `build_session`); y `pub fn fetch_model(dir)`. Nada de esto entra en `-p clawg-cli`.
- [x] `crates/clawg-eval/src/bin/load-eval/`: `load-eval gate [--model-dir D] [--loads 10] [--tag T]` y `load-eval child --arm A --model-dir D`. En Windows el padre crea un job con `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, le asigna el hijo y recién entonces le escribe `go` por stdin; el hijo carga, lee `clawg_cli::daemon::process::sample()` y escribe una línea JSON; el padre lee `PeakProcessMemoryUsed` con `QueryInformationJobObject(JobObjectExtendedLimitInformation)`. En Linux, un proceso por carga y `VmHWM`/`RssAnon`.
- [x] Salida por stdout y en `docs/eval/results/{date}-real-load-eval[-T].json`, que nunca reemplaza otro y queda fuera de git.
- [x] `.claude/CLAUDE.md`, en el mismo commit:
  - la fila de `crates/clawg-eval` de la tabla del workspace suma `load-eval` (el pico de memoria y el tiempo de cargar el modelo, por brazo, una carga por proceso);
  - en «Testing», un párrafo como los de `rag-eval` y `revision-eval` dice:
    - qué mide, y qué lee de cada carga;
    - qué necesita: Windows para decidir, un build release y el modelo real bajado a un `load-eval-*` del temporal que se borra al terminar, nunca un perfil;
    - las tres salidas del veredicto;
    - dónde escribe el resultado.
- [x] Verificación: suite y clippy; comprobar que `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` no compila nada de lo nuevo.

### 5b. Puerta: PARAR y pedir autorización

- [x] **STOP.** Preguntar al usuario y esperar. Hace falta autorización para: un build release de `clawg-eval` (`cargo run -p clawg-eval --release --target-dir target-test --bin load-eval -- gate`), la descarga del modelo (~470 MB) a un `load-eval-<uuid>` del temporal, y usar Windows. No toca ningún perfil. No continuar ni suponer la respuesta. No matar ningún `clawg.exe`.
- [x] Correr la puerta con la autorización, leer `decision` y guardar el JSON en `docs/eval/results/` (fuera de git). Limpiar los builds release de `target-test` al terminar.
- [x] Resultado `View` (queda C′, lo previsto): seguir con el commit 6. Anotar lo medido en `exploration.md`.
- [x] No aplica (salió `View`). Resultado `File`: un commit propio cambia el mecanismo a `commit_from_file`, con la ruta que sale del handle y la comparación de `FILE_ID_INFO`, y RM1 se reescribe en `spec.md` y en la línea de `.claude/CLAUDE.md` del commit 3 (tests nuevos primero, como arriba).
- [x] No aplica (salió `View`). Resultado `Revert`: revertir los commits 4 y 3 (en ese orden, con `git revert`, cada uno atómico). El 2 y el 1 se quedan. El arnés queda para el seguimiento de datos externos. El cambio se cierra con lo medido.

## 6. `test(embedder): pin the embedding contract together with its model id` (D7; RM4)

- [x] Test primero (`crates/clawg-core/src/memory/embedder.rs`, `mod tests`): `the_embedding_contract_is_pinned`, una tabla de literales: `E5_MODEL_ID` (`:135`), `E5_QUERY_PREFIX` (`:109`), `E5_PASSAGE_PREFIX` (`:130`), `EMBED_MAX_TOKENS` (`:166`), `EMBEDDING_DIM` (`vector.rs:8`), `MODEL_SHA256`, `TOKENIZER_JSON_SHA256`, `CONFIG_JSON_SHA256`, `SPECIAL_TOKENS_MAP_SHA256` y `TOKENIZER_CONFIG_SHA256` (`model_download.rs:62-71`), el sufijo `/mean` del id y los bits de `pool_and_normalize` (`:290`) sobre la entrada `[1, 3, 2]` con máscara `[1, 1, 0]`, escritos como literales desde el código de hoy.
- [x] Cada aserción lleva un mensaje que pregunta si `E5_MODEL_ID` tiene que subir. Como este test pasa de entrada (fija lo que existe), comprobar a mano que falla: cambiar un literal, ver el mensaje y deshacerlo, antes de commitear.
- [x] Los literales de `:1119`, `:1176` y `:1492` se quedan.
- [x] Sin cambios en `.claude/CLAUDE.md`. Entra antes que el 7.
- [x] Verificación: suite y clippy.

## 7. `perf(docs): a refresh keeps the vector of every passage whose text the index holds` (D6; RM5)

- [ ] Test primero (`crates/clawg-core/src/memory/project_docs/mod.rs`), con un embebedor falso que anota los textos como `Counting` (`:632-656`): `an_edited_doc_embeds_only_the_passages_that_changed`. Falla: `embed_passages` (`:464-489`) embebe todos.
- [ ] Test primero (`mod.rs`): `a_partly_reused_doc_equals_a_full_reindex` (byte a byte), `a_reused_vector_is_the_stored_blob` (con la «sal» que cambia entre refrescos sin cambiar el id), `a_repeated_text_is_reused_in_all_its_passages`, `a_doc_of_another_model_reuses_nothing`, `a_moved_file_reuses_nothing`, `a_failed_embedding_of_the_rest_keeps_the_doc_as_it_was` y `reused_counts_passages_not_distinct_texts`. Fallan por no compilar (`RefreshReport::reused`) o por la aserción.
- [ ] Test primero (`mod.rs`, `#[ignore]`, con `CLAWG_E5_MODEL_DIR` como `tests/e5_golden.rs:9-14`): `a_partly_reused_doc_matches_a_full_reindex_within_tolerance` (1e-5 por componente o coseno ≥ 1 − 1e-6). Se ejecuta a mano con el modelo real; necesita autorización si usa un directorio de un perfil.
- [ ] Test primero (`crates/clawg-cli/src/daemon/api.rs`): `the_docs_log_line_shows_the_reused_passages`, de unidad sobre `refresh_line`. Falla: la línea se arma dentro de `log::info!` (`:2674-2677`), sin función.
- [ ] Se conservan `an_unchanged_project_embeds_nothing`, `another_model_reindexes_every_doc` y `a_second_refresh_does_not_wait_for_the_first`; `plan_refresh` no cambia.
- [ ] `mod.rs`: la lectura de D6 por cada doc de `plan.index` con huella, en `conn.call`, comparando el `model_id` (`:420`) en la misma sentencia; los docs nuevos no consultan.
- [ ] `mod.rs`: `fn reuse_plan(passages: &[Passage], stored: &HashMap<String, Vec<u8>>) -> Reuse { vectors: Vec<Option<Vec<u8>>>, to_embed: Vec<usize> }`, con clave `Passage::indexed_text()` (`chunk.rs:28-30`) y `entry().or_insert`. Corre en la tarea async. `embed_passages` recibe los textos de `to_embed` y vuelve sin llamar al embebedor si no hay; su manejo de `Vector` y `VectorCount` no cambia. Una función pura `fill(reuse, new)` arma el `Vec<Vec<u8>>` en el orden de los pasajes; `replace_doc` (`:544-577`) no cambia.
- [ ] `RefreshReport::reused: usize`, sumado junto a `report.passages` sólo cuando el doc se escribe (`:446-457`).
- [ ] `api.rs`: `refresh_line(project_id, &RefreshReport) -> String` deja la línea igual y le suma `, {} reused` al final.
- [ ] Comprobar al aplicar: que el plan de la lectura es el `POINT` por rowid de sqlite-vec 0.1.9 (`EXPLAIN QUERY PLAN` o la misma forma que `maintenance.rs:741-747`).
- [ ] `.claude/CLAUDE.md`: el párrafo del índice de docs («a file is re-read when its sha256 or the model changes») suma que un doc cambiado reutiliza el vector guardado de cada pasaje de texto idéntico del mismo doc y mismo `model_id`, que sólo embebe el resto en la misma transacción, y que la línea `indexed the docs of …` lleva `reused`.
- [ ] Verificación: suite y clippy.

## Al cerrar

- [ ] `sdd-verify`: contra RM1 a RM5 y la puerta, con la suite y clippy de Windows; Linux queda para el CI del pull request (RM1 en Windows, RM3 y la parte de Windows de RM2 sólo corren allá).
- [ ] Consolidar la spec al archivar en `openspec/specs/storage/spec.md` (RM1, RM2, RM3 y RM4) y en `openspec/specs/project-docs/spec.md` (RM5), y realinear las citas que este cambio movió, incluidas las que la spec notó viejas: `E5_MODEL_ID` figura como `embedder.rs:133` en el `storage` actual y el código está en `:135`, y `serve.rs:153` (la sonda de memoria privada, línea 708 de ese spec).
- [ ] Actualizar la fila 92 de `openspec/README.md` (y las filas que citan este cambio) al archivar, y mover el cambio con `git mv` a `changes/archive/`.
- [ ] Subir el minor de `[workspace.package] version` del `Cargo.toml` raíz, `cargo update --workspace`, y commitear ambos como `chore: bump version to X.Y.Z`. Nunca tag ni release a mano.
- [ ] La segunda pasada, con los hooks instalados, decide B y E. Necesita autorización explícita (instalar los hooks y leer `metrics.db` y `serve.log` de prod). PARAR y preguntar; no se hace en este cambio.
- [ ] La revisión externa que adoptó la propuesta no está en el repositorio: ningún documento ni commit la enlaza.
