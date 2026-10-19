# model-download — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. Una entrada y un lector (R6, D1)

- [x] Los tests de `model_download.rs` pasan a `ModelSource` y `ensure_model_from` sin cambiar lo que comprueban.
- [x] `ModelSource`, `ensure_model_from`, `read_verified`; fuera las cuatro capas y los tres lectores.
- Commit: `refactor(core): take the model source as one value and read the cache through one checked reader`

## 2. Plazos de red (R1, D2)

- [x] Tests primero: `a_body_that_stops_arriving_fails_the_attempt`, `a_server_that_never_answers_fails_the_attempt`, `the_download_waits_30_s_to_connect_and_60_s_per_read`.
- [x] `Timeouts` en `ModelSource`, el cliente por corrida, `DownloadError::Client`.
- Commit: `fix(core): give the model download a connect and a per-read deadline`

## 3. La causa del fallo (R2, D3)

- [x] Tests primero: `an_exhausted_download_names_the_last_status_and_the_host`, `the_cause_never_carries_the_credentials_of_the_url`.
- [x] `RetryExhausted { host, cause }`, `describe`, un `warn!` por intento; fuera `Http`.
- Commit: `fix(core): say why the model download failed and from which host`

## 4. Un hash por archivo, fuera del runtime (R4, D5)

- [x] Tests primero: `the_cache_check_leaves_the_runtime_thread_free`, `each_file_is_hashed_once_per_run`, `the_worker_resolves_the_tokenizer_off_the_runtime_thread`.
- [x] `check_cache` en `spawn_blocking`, el hash mientras se baja, el cierre en `spawn_blocking`; el tokenizer del worker en `spawn_blocking`.
- Commit: `perf(core): hash each model file once and off the async workers`

## 5. La recarga sólo lee la caché (R3, D4)

- [x] Tests primero: `the_cached_model_never_touches_the_network`, `a_reload_with_a_damaged_cache_is_not_ready_and_repairs_it_once_in_the_background`; `an_idle_unloading_embedder_reloads_through_the_same_files_and_load` cuenta la lectura de caché y falla si la recarga descarga.
- [x] `load_cached_model`, `EmbedderDeps`, la recarga y la reparación en `unloading_when_idle`, el cableado de `serve.rs`.
- Commit: `fix(core): reload the idle model from its cache and repair the cache in the background`

## 6. `CLAWG_HF_BASE_URL` (R5, D6)

- [x] Tests primero: `the_base_url_defaults_to_hugging_face`, `the_base_url_takes_https_and_loopback_http`, `a_base_url_that_is_not_https_or_loopback_is_refused`.
- [x] `base_url_from`, `DownloadError::BaseUrl`, el comentario del módulo.
- Commit: `fix(core): accept only https or a loopback mirror in CLAWG_HF_BASE_URL`

## 6b. Un tope al tamaño de un archivo (R4, D5)

Hallado al releer el diff: el cuerpo se acumula en memoria y la reserva sale de `Content-Length`.

- [x] Test primero: `a_file_larger_than_any_model_file_is_refused_without_holding_it` (sin el tope, el proceso aborta al reservar 1 TB).
- [x] `MAX_FILE_BYTES`, `DownloadError::TooLarge`.
- Commit: `fix(core): refuse a model file larger than the disk pre-flight before holding it in memory`

## 7. Docs

- [x] `.claude/CLAUDE.md`, el mapa de `openspec/README.md`, `verify.md` y `state.yaml`. `openspec/specs/storage/spec.md` («Recargar a demanda», «Una recarga que falla») y `openspec/specs/ingest/spec.md` («Estado en `/health`») se consolidan al archivar.
- Commit: `docs: describe the model download deadlines, the cache-only reload and the mirror variable`

## 8. Verificación

- [x] Suite y clippy en Linux sobre el último commit.
- [x] El CI en verde en Linux y Windows (`d653bed`), y la suite y clippy en Windows sobre el merge en `develop`.
- [x] «Probar a mano» del diseño con el perfil dev en homes de prueba, sin tocar `~/.clawg-dev`; los tres pasos en [`verify.md`](verify.md).
