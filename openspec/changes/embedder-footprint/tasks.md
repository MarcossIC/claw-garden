# embedder-footprint — Tareas

Strict TDD: en cada commit el test que falla va primero, con el nombre exacto de la [spec](spec.md). Un commit convencional y atómico; se prepara por ruta explícita (nunca `git add -A`, `.` ni `-u`), sin `Co-Authored-By` ni atribución. Ni el código, ni los comentarios, ni los nombres de test, ni los mensajes de commit nombran el cambio, la spec ni fases.

Comandos (en Windows, sumar `--target-dir target-test`):

- Suite: `cargo test --workspace`.
- Clippy: `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`.
- No hay builds release, instalaciones ni acceso a `~/.clawg` ni `~/.clawg-dev` sin autorización. No se mata ningún `clawg.exe`.

Revisado: `agent_contract.json` y `agent_contract.es.json` no cambian (ninguna herramienta ni protocolo se toca).

## 1. `fix(embedder): load the model when the machine's parallelism is unknown` (EF1)

- [x] Test primero (`crates/clawg-core/src/memory/embedder.rs`): `an_unknown_parallelism_falls_back_to_two_threads` y `the_session_takes_every_thread_the_machine_reports`. Fallan por no compilar: no existe `threads_from` ni `FALLBACK_SESSION_THREADS`.
- [x] `embedder.rs`: `FALLBACK_SESSION_THREADS = 2`, `threads_from` pura y `session_threads() -> usize`, que registra un `warn` cuando cae al respaldo; `session_builder` deja de propagar el error.
- [x] `embedder.rs`: el comentario de `session_builder` dice lo medido (1, 2 y 4 hilos, mismos bits en Linux x86-64 de 4 hilos con AVX-512F) y que otras máquinas no están medidas.
- [x] `crates/clawg-core/src/eval_api.rs`: `pub fn session_threads() -> usize`; su test compara sin `unwrap`.
- [x] `crates/clawg-core/src/eval_api/vector_memo.rs`: `EvalMemo::open` sin el camino de hilos desconocidos; el archivo del memo deja de ser `Option`.
- [x] `crates/clawg-eval/src/bin/rag-eval/docs.rs`: el bloque `index` toma `session_threads()` sin `EXIT_RUNTIME_FAILURE`.
- [x] Verificación: suite y los dos clippy.

## 2. La puerta: guía de medición

Cada paso necesita el modelo real (`CLAWG_E5_MODEL_DIR=<dir>` con los cinco archivos de `HF_BASE_URL`), un `CLAWG_TEST_HOME` temporal o un perfil dev con autorización, y el mismo build en las dos variantes de cada comparación. Se corre en **Linux y en Windows**, con al menos una máquina de 8 hilos lógicos o más **con SMT**. Cada resultado anota la huella (sistema, arquitectura, AVX2, AVX-512F, FMA, hilos lógicos y físicos) y el perfil del build (debug o release). Los umbrales se fijan antes de leer los números (decisión abierta de la exploración).

- [ ] 2.1 **Hilos vivos.** Contar `/proc/<pid>/task` (Linux) o `Get-Process -Id <pid> | % Threads` (Windows) del daemon antes de cualquier guardado, después de 20 guardados y después de que el modelo se suelte (`CLAWG_EMBEDDER_IDLE_MINUTES=1`). Si el grupo de `rayon` deja hilos con el modelo suelto, abre C.
- [ ] 2.2 **CPU en reposo.** CPU de proceso (usuario + sistema) durante 60 s después de la última inferencia, con el modelo residente (`CLAWG_EMBEDDER_IDLE_MINUTES=0`). Si es medible, abre A.
- [ ] 2.3 **CPU y pared de un refresco de docs.** `rag-eval --real --docs --no-cache` sobre el mismo árbol, 5 corridas por combinación, mediana: hilos = lógicos, físicos y `min(lógicos, 4)`; espera activa sí y no; tokenizer paralelo sí y no. Las variantes se arman en una rama de medición que no se commitea.
- [ ] 2.4 **Latencia de búsqueda durante el refresco.** Un cliente de `POST /v1/search` cada 100 ms mientras corre 2.3, p50 y p99 por combinación. En Linux, además, `MALLOC_ARENA_MAX=1` contra `=2` (la arena única de embedder-idle-unload, D6).
- [ ] 2.5 **`split_save`.** Un guardado de 64 KiB y otro de 5.000 tokens: tiempo de las tres codificaciones y de la tanda, reloj en el worker. Abre E sólo sobre el umbral que se fije.
- [ ] 2.6 **Bits.** `CLAWG_E5_MODEL_DIR=<dir> cargo test -p clawg-core --features eval --test e5_golden -- --include-ignored` con cada combinación de hilos y espera activa; en Linux, `taskset -c 0-<n>` cambia lo que informa `available_parallelism()`. Cualquier combinación que mueva un bit queda fuera salvo decisión expresa.
- [ ] 2.7 Registrar todo en `verify.md` con la huella de cada máquina, y decidir cada opción contra su umbral.

## 3. Según la puerta

Cada opción que abra su puerta es una tarea con su test primero y su re-chequeo de `e5_golden`, en este orden: C, A, B, E. Si ninguna abre, el cambio se cierra con la tarea 1 y el registro de la 2 (enfoque F).
