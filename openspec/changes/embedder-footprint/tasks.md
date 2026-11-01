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

- [x] 2.1 **Hilos vivos.** Contar `/proc/<pid>/task` (Linux) o `Get-Process -Id <pid> | % Threads` (Windows) del daemon antes de cualquier guardado, después de 20 guardados y después de que el modelo se suelte (`CLAWG_EMBEDDER_IDLE_MINUTES=1`). Si el grupo de `rayon` deja hilos con el modelo suelto, abre C. Windows y Linux hechos, en [verify.md](verify.md).
- [x] 2.2 **CPU en reposo.** CPU de proceso (usuario + sistema) durante 60 s después de la última inferencia, con el modelo residente (`CLAWG_EMBEDDER_IDLE_MINUTES=0`). Si es medible, abre A. Windows y Linux hechos, en [verify.md](verify.md).
- [x] 2.3 **CPU y pared de un refresco de docs.** `rag-eval --real --docs --no-cache` en release sobre el mismo árbol congelado. Un factor por vez contra la configuración de hoy, 5 brazos: base, hilos = físicos, hilos = `min(lógicos, 4)`, sin espera activa y tokenizer en serie. 3 rondas con el orden rotado; cada brazo se compara con la base de su ronda, mediana de los 3 cocientes, porque esta máquina ya mostró cambios de régimen de hasta 27 % entre corridas. Un brazo que en su primera corrida pase de +15 % de tiempo frente a la base de su ronda, con `split` parejo, no se repite. Los umbrales están en [verify.md](verify.md). Si un brazo gana, recién ahí se prueban combinaciones. Las variantes se arman en una rama de medición que no se commitea. Cada corrida, por brazo y por ronda, escribe su propio archivo de resultados con `--tag <brazo>-r<ronda>` o un `--out` propio, sin `--force`: el nombre sale de la fecha, el modo y el tag (`results_path`, `crates/clawg-eval/src/bin/rag-eval/docs.rs:389`), así que sin eso la segunda corrida del día termina con código 4, y con `--force` pisaría la anterior. La ronda 1 en Windows está hecha: descartó los dos brazos de hilos y mostró que correr brazos seguidos calienta la máquina y favorece al primero. **Lo que queda se mide con una prueba corta:** una muestra del corpus (cada séptimo archivo de cada tipo), brazos base, sin espera activa y tokenizer en serie, 3 rondas con el orden rotado. Cada brazo arranca con el sensor térmico en 340 K o menos, y se registran temperatura y frecuencia durante la corrida. Lo que gane se confirma con un refresco completo frente a la base, porque una corrida corta no muestra el calor sostenido. La prueba corta está hecha en Windows, en [verify.md](verify.md): sin espera activa se descarta y tokenizer en serie no entra, porque su ganancia es marginal. Sin ningún brazo en pie, no hacen falta el refresco completo ni medirlo en Linux.
- [x] 2.4 **Latencia de búsqueda durante el refresco.** Un cliente de `POST /v1/search` cada 100 ms mientras un daemon refresca docs, p50 y p99 por brazo, sólo de los brazos que sigan en pie después de 2.3. Son corridas propias: `rag-eval` no sirve HTTP, así que no se pueden hacer durante 2.3. En Linux, además, una arena contra dos (la arena única de embedder-idle-unload, D6). `MALLOC_ARENA_MAX=2` solo no da dos: `clawg serve` llama a `mallopt(M_ARENA_MAX, 1)` al arrancar (`keep_one_malloc_arena`, `crates/clawg-cli/src/main.rs:72`, `crates/clawg-core/src/memory/embedder.rs:747`), y eso pisa la variable. Se mide con un build de la rama de medición sin esa llamada, el mismo para las dos variantes, con `MALLOC_ARENA_MAX=1` y `=2`. Después de 2.3 no queda ningún brazo: sólo falta la comparación de arenas. Hecho en Linux, en [verify.md](verify.md): con 2 arenas el p99 no baja, así que queda una arena.
- [ ] 2.5 **`split_save`.** Un guardado de 64 KiB y otro de 5.000 tokens: tiempo de las tres codificaciones y de la tanda, reloj en el worker. Abre E sólo sobre el umbral de [verify.md](verify.md). Linux hecho, en [verify.md](verify.md): las dos codificaciones de más pesan 1,07 % de un guardado de 64 KiB, así que E no abre en Linux. Falta Windows.
- [x] 2.6 **Bits.** `CLAWG_E5_MODEL_DIR=<dir> cargo test -p clawg-core --features eval --test e5_golden -- --include-ignored` con cada combinación de hilos y espera activa; en Linux, `taskset -c 0-<n>` cambia lo que informa `available_parallelism()`. Cualquier combinación que mueva un bit queda fuera salvo decisión expresa. No se corre: no entra ningún cambio de hilos, de espera activa ni de tokenizer.
- [ ] 2.7 Registrar todo en `verify.md` con la huella de cada máquina, y decidir cada opción contra su umbral.

## 3. Según la puerta

Cada opción que abra su puerta es una tarea con su test primero y su re-chequeo de `e5_golden`, en este orden: C, A, B, E. Si ninguna abre, el cambio se cierra con la tarea 1 y el registro de la 2 (enfoque F).
