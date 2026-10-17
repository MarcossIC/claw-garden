# embedder-idle-unload — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional. La suite completa corre una vez, sobre el último commit (regla del proyecto).

## 0. Paso 0: ¿la memoria vuelve? (D1)

- [x] Cinco cargas y descargas del modelo real en Linux, con y sin trim. Sin trim queda en ~700 MB; con trim, 41-60 MB.

## 1. Devolver la memoria del modelo (R7, D1)

- [x] `return_freed_memory` al terminar el hilo `fastembed-worker` (`malloc_trim` en Linux con glibc; nada en el resto). Sin test automático: lo mide el paso 0.
- Commit: `fix(core): give the memory a dropped model freed back to the system on glibc`

## 2. El envoltorio (R1, R2, R3, R6, D2, D3)

- [x] Los 14 tests de `idle_embedder.rs` y `an_idle_unloading_embedder_reloads_through_the_same_files_and_load` (`bootstrap.rs`): no compilan.
- [x] `IdleEmbedder`, `Embedder::resident`/`wake`, `idle_after`, `bootstrap::unloading_when_idle`.
- Commit: `feat(core): let the embedding model go after a stretch without use and load it again on demand`

## 3. El daemon (R4, R5, R6, D4)

- [x] Los 5 tests de `tests/api.rs` y `a_health_without_residency_still_parses` (`client.rs`): no compilan.
- [x] `serve` con `CLAWG_EMBEDDER_IDLE_MINUTES`, `embedder_resident` en `/health`, `wake` en `POST /v1/projects`, `503 embedder_not_ready` en búsqueda y restauración.
- Commit: `feat(cli): unload the idle model in the daemon, report it in health and wake it on project registration`

## 4. Docs

- [x] `.claude/CLAUDE.md`, el mapa de `openspec/README.md` y este cambio.
- Commit: `docs: describe how the daemon lets the idle embedding model go`

## 5. Verificación

- [x] Suite y clippy en Linux sobre el último commit (2.065 pasan; clippy con código 0).
- [x] El CI en verde en Linux y Windows.
- [ ] Bytes privados del daemon dev en Windows antes y después del tiempo de ocio: medidos, no cumplen R7 (ver [verify](verify.md#resultado)).

La verificación en Windows mostró que D1 no alcanza ([verify](verify.md#resultado)); las secciones 6 a 10 son el trabajo de [D5](design.md#d5-un-solo-tokenizer-por-proceso-la-inferencia-la-arma-clawg). Las secciones 1 a 5 quedan como la historia de la primera implementación. El orden está pensado para que cada commit compile y pase la suite, y para que el fixture exista antes de que cambie la inferencia.

## 6. El fixture de los vectores, antes del cambio (D5, «Probar que los vectores son los mismos»)

El test sólo puede usar una puerta que exista antes y después del cambio y no cambie de forma: `eval_api` (feature `eval`) y su `EvalEmbedder::real`, que hoy arma `FastEmbedLocal` y después armará `E5Embedder`. Así el commit del cambio no toca ni el test ni el fixture.

- [x] `EvalEmbedder::embed_batch` en `crates/clawg-core/src/eval_api.rs` (hoy sólo tiene `embed_with_role`): el lote de 17 y el corte por `EMBED_BATCH` tienen que pasar por el camino de lote. Lo usa el test, así que no es código muerto; test primero (con `FakeEmbedder`: el lote vuelve en el orden de entrada y vacío da vacío).
- [x] `crates/clawg-core/tests/e5_golden.rs`, con `required-features = ["eval"]` en un `[[test]]` de `crates/clawg-core/Cargo.toml` (en `cargo test --workspace` la feature la enciende `clawg-eval`; sola, `-p clawg-core --test e5_golden` necesita `--features eval`). El test primero, antes de que exista el fixture:
  - Textos de D5: consultas cortas, español, CJK y emoji, uno de ~2.000 tokens, tres con 509, 510 y 511 tokens de contenido, `<s> </s> <pad> <unk> <mask>` literales, uno vacío y un lote de 17.
  - `#[ignore = "<razón: necesita el modelo real en CLAWG_E5_MODEL_DIR>"]` (el primero del repo: la suite va a decir 1 ignorado, y es éste).
  - Lee `CLAWG_E5_MODEL_DIR`; con `--ignored` y sin la variable, falla (no se salta). Esa falla es el primer rojo.
  - Compara bits si la CPU coincide con la del fixture (arquitectura, AVX2, AVX-512, FMA, con `is_x86_feature_detected!`); si no, diferencia máxima de 1e-6, y el test dice cuál de las dos comparó.
  - La decisión bits/tolerancia es una función pura (`comparison_for(fixture_cpu, this_cpu)`) con su test, sin modelo.
- [x] Generar el fixture (`crates/clawg-core/tests/fixtures/e5_golden.json`) con el fastembed de hoy: `CLAWG_BLESS=1 CLAWG_E5_MODEL_DIR=<dir> cargo test -p clawg-core --features eval --test e5_golden -- --ignored`, igual que el contrato del agente (reescribe y falla; se revisa el diff y se corre otra vez sin `CLAWG_BLESS`, en verde). Lleva la huella de la CPU donde se generó (arquitectura, AVX2, AVX-512, FMA) y los vectores como bits de `f32` (`to_bits`).
- [x] Hacerlo en una máquina con el modelo. El fixture no se regenera después de este commit: si la sección 8 lo pidiera, el cambio ya no es «los mismos vectores».
- [x] `cargo test --workspace` en verde (1 ignorado) y clippy.
- Commit: `test(core): pin the vectors of the embedding model in a golden fixture`

## 7. Un solo dueño del tokenizer, lado del worker (R8, D5 «Un dueño del tokenizer»)

Antes del cambio de inferencia sólo se puede hacer la mitad del worker: la carga del modelo sigue siendo de fastembed, que parsea por su cuenta y no puede tomar la celda. La mitad del modelo y `the_worker_and_the_model_load_share_one_parse` van en la sección 8.

- [x] Test primero, en `worker.rs`: `a_tokenizer_file_with_another_sha256_is_not_parsed` (un `tokenizer.json` con otro SHA-256: no se parsea y la celda queda vacía; se ve con un contador, no con el log).
- [x] `model_download.rs`: la carga verificada de `tokenizer.json` (lee, compara con `TOKENIZER_JSON_SHA256`, parsea y deja `tokenizer.json parsed in N ms` en el log). `load_tokenizer` queda como está para las herramientas de evaluación.
- [x] `resolve_tokenizer_and_chunker` (`worker.rs`) recibe el cargador como parámetro; los dos tests que existen (`resolve_tokenizer_and_chunker_fills_the_cell_and_shares_the_identical_arc` y el de la celda vacía) pasan un cargador propio. El daemon (`serve.rs`) le pasa el verificado. La celda (`MemoryRuntime::tokenizer`, `runtime.rs`) se llena con `get_or_try_init`.
- Commit: `fix(core): check the tokenizer file against its hash before the worker parses it`

## 8. La inferencia la arma clawg (R7, R8, R9, D5)

Las piezas puras y el cambio van en **un solo commit**. Por separado, las piezas puras (ventana, relleno, salida, pooling, rechazos) no las llamaría nada de producción hasta el cambio: clippy con `-D warnings` falla por `dead_code`, y `tests/conventions.rs` prohíbe el `#[allow]` que lo taparía. Se escriben una por una, test primero, y el commit es uno. Las tareas de abajo son el orden de trabajo, no commits.

### 8.1 Las piezas puras (test primero, en `embedder.rs`)

- [x] `an_encoding_matches_the_tokenizer_configured_like_fastembed`: un tokenizer armado en código (`WordLevel` con `RobertaProcessing`) y un clon configurado como lo configura fastembed (`common.rs:98-111`); para textos bajo, en y sobre la ventana y en lotes de largos mezclados, los mismos ids, máscara y tipos. Cubre el relleno (`BatchLongest`, `pad_token_id`, tipo 0, a la derecha).
- [x] `a_long_text_is_cut_to_the_window_and_keeps_its_closing_token`: `encode(t, false)`, `Encoding::truncate(EMBED_MAX_TOKENS − added_tokens(false), 0, Right)` y `post_process(enc, None, true)`; `</s>` queda al final.
- [x] `mean_pooling_matches_fastembeds_formula_bit_for_bit`: estados ocultos al azar con una máscara con relleno contra la fórmula de fastembed escrita con ndarray; mismos `to_bits`. Pide `ndarray = { version = "0.17.2", default-features = false }` en `[dev-dependencies]` de `clawg-core`.
- [x] `the_output_is_picked_like_fastembed`: la única, o la primera de `text_embeds`, `last_hidden_state`, `sentence_embedding`; sin ninguna, `LoadFailed`.
- [x] `the_load_refuses_a_tokenizer_with_its_own_truncation_or_padding`.
- [x] `the_load_refuses_a_special_token_the_tokenizer_does_not_treat_as_special`.
- [x] `the_load_refuses_a_model_window_shorter_than_embed_max_tokens` (`model_max_length` de `tokenizer_config.json`).
- [x] Las funciones puras (ventana, relleno, pooling con L2, elección de la salida, controles de la carga) separadas de la sesión, para que los tests no necesiten el modelo; errores de carga con `thiserror`.

### 8.2 El cambio

- [x] `E5Embedder` reemplaza a `FastEmbedLocal` en `embedder.rs`: sesión con las opciones de hoy (`cpu_provider` sin arena, nivel 3, `available_parallelism` hilos intra-op, `commit_from_memory`), hilo `embedder-worker` con `encode_batch(textos, false)`, corrida y pooling; al cerrarse el canal, `drop(session)` y después `return_freed_memory`. Los tests del canal existentes no cambian.
- [x] `TokenizerFiles` propio en `model_download.rs` (los mismos cuatro campos); `bootstrap.rs` (`production_deps`, el `TokenizerFiles` de `:298` y los tests de `:741`) y `eval_api.rs` (`EvalEmbedder::real`, que parsea su propia copia) dejan de nombrar fastembed.
- [x] La celda llega a la carga: `production_deps` y `serve_deps` reciben `runtime.tokenizer` (`serve.rs`); la carga la toma o la llena con los bytes que `ensure_model` verificó, con `get_or_try_init`. Test: `the_worker_and_the_model_load_share_one_parse` (un cargador que cuenta; en cualquier orden, 1 parse y el mismo `Arc`).
- [x] La recarga arma sólo la sesión y descarta sin parsear los bytes de `tokenizer.json`; los dos JSON chicos se leen otra vez para `pad_token_id` y los rechazos. Test: `a_reload_does_not_parse_the_tokenizer_again` (`bootstrap.rs`, con el cargador que cuenta).
- [x] `Cargo.toml` y `crates/clawg-core/Cargo.toml`: fuera `fastembed`. `ort = "=2.0.0-rc.12"` con `default-features = false` y `features = ["std", "ndarray", "api-24", "download-binaries", "tls-native"]`; `tokenizers = "0.22.2"` con `default-features = false` y `features = ["onig"]`. El comentario de `ort` en `Cargo.toml` deja de decir «subirlo con fastembed» (y los de `Cargo.toml:38-41` ya no hablan de fastembed).
- [x] `cargo update --workspace` y revisar el diff del lock: pierde `fastembed` y `safetensors` y no suma ni cambia nada más.
- [x] El commit no toca `crates/clawg-core/tests/e5_golden.rs` ni el fixture (`git diff --stat` lo confirma antes de commitear).
- [x] Comentarios de `chunker/mod.rs:13` e `idle_embedder.rs:9` y el doc del módulo `embedder.rs` sin fastembed.
- [x] Suite y clippy completos antes de commitear.
- Commit: `feat(core): run the embedding model on our own session and one shared tokenizer, dropping fastembed`

## 9. Docs

- [x] `.claude/CLAUDE.md:9` dice «ONNX embeddings (fastembed)»: pasa a «ONNX embeddings (ONNX Runtime)». Lo mismo en `.cursor/rules/cursor.mdc:15` y `.gemini/GEMINI.md:9`.
- [x] `openspec/README.md`: la fila de este cambio (línea 72; la de orden, 101, sigue valiendo) y la de `:177` («Embeddings locales con fastembed-rs», que sigue siendo un descarte: sólo se corrige el nombre de la herramienta, y su cita pasa a `model_download.rs:26`).
- [x] `openspec/config.yaml:5` menciona «fastembed ONNX multilingual-e5-small».
- [x] Este cambio: `design.md` ya trae D5; completar `verify.md` con los resultados de la sección 10 cuando existan.
- Commit: `docs: describe the embedding model running on ONNX Runtime with one shared tokenizer`

## 10. Verificación (cierra el cambio)

- [x] Suite y clippy en Linux sobre el último commit (`cargo test --workspace`; `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`): los corre el CI en `ubuntu-latest` sobre `0d36f88` ([verify](verify.md#después-de-d5-un-solo-tokenizer)).
- [x] El CI en verde en Linux y Windows, sobre `0d36f88`.
- [x] Suite y clippy en Windows con `--target-dir <otro>` (el `clawg.exe` del editor bloquea `target`).
- [x] `cargo test -p clawg-core --features eval --test e5_golden -- --ignored` con `CLAWG_E5_MODEL_DIR`, en una máquina con el modelo: bits exactos si la CPU coincide con la del fixture, o diferencia máxima de 1e-6 si no, diciendo cuál; y sin la variable, falla.
- [x] `rag-eval --diff` con `--real` de `main` al cambio, y otra vez con `--agent-view`: ningún caso se mueve.
- [ ] R7 en el daemon, en Windows (hecho: [verify](verify.md#r7-en-windows)) y en Linux (medido el 2026-10-17, no cumple: [verify](verify.md#r7-en-linux)), siguiendo D5 «R7 sobre lo medido»: línea de base con `CLAWG_TEST_EMBEDDER=fake` y sólo `tokenizer.json`; con el modelo y `CLAWG_EMBEDDER_IDLE_MINUTES=1`, diez recargas con `POST /v1/search` (nunca `clawg memories search`); la mediana queda a no más de 64 MB de la línea de base, la de las recargas 6 a 10 no supera la de 1 a 5 en más de 16 MB, la `distance` de cada hit es igual en las diez vueltas, y el log tiene una sola línea `tokenizer.json parsed`.

Sólo queda la última tarea, R7 en Linux. Si cumple, el cambio se archiva sin otra revisión: [verify, «Para archivar»](verify.md#para-archivar).
