# model-download — Verificación

## Linux (2026-10-19)

Sobre `09fcd2f` (el último commit de código), en `target`, con rustc 1.97.0:

- `cargo test --workspace`: pasa, ningún test falla. El único ignorado es `the_vectors_are_the_ones_fastembed_gave` (`crates/clawg-core/tests/e5_golden.rs`), que pide el modelo real y no es de este cambio.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Cada commit de las [tareas](tasks.md) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` pasa sin bendecir: nada de lo que el modelo ve del servidor MCP cambió.

## Windows (2026-10-19)

Sobre `a75f34b`, el merge en `develop` de este cambio con secret-coverage (51), en `target-test` limpio de los artefactos del workspace antes de compilar, con rustc 1.99.0:

- `cargo test --workspace --target-dir target-test`: pasa, con el mismo único ignorado.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sale con 0.

## CI

El pull request de la rama corrió `ci.yml` sobre `d653bed`, el último commit del cambio: `test (ubuntu-latest)` y `test (windows-latest)` en verde. El merge en `develop` no tiene CI propio (`ci.yml` corre en pull requests y en pushes a `main`); lo cubre la corrida de Windows de arriba.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `a_body_that_stops_arriving_fails_the_attempt` y `a_server_that_never_answers_fails_the_attempt`: `ensure` no volvía y el `tokio::time::timeout` de 30 s del test vencía |
| R2 | `an_exhausted_download_names_the_last_status_and_the_host`: el texto no tenía `HTTP 503` ni el host; los dos tests de plazos no decían `timed out` |
| R3 | `a_reload_with_a_damaged_cache_is_not_ready_and_repairs_it_once_in_the_background`: no compilaba sin `CachedModelFn`; con la reparación esperada dentro de la recarga (lo de antes), la llamada no contestaba en 1 s |
| R4 | `the_cache_check_leaves_the_runtime_thread_free`: el hilo del runtime no avanzaba mientras se hasheaba; `each_file_is_hashed_once_per_run`: el archivo bajado se volvía a leer; `the_worker_resolves_the_tokenizer_off_the_runtime_thread`: el tokenizer se leía en el hilo del test; `a_file_larger_than_any_model_file_is_refused_without_holding_it`: el proceso abortaba al reservar 1 TB |
| R5 | No compilaba sin `base_url_from` |
| R6 | Refactor: los tests que ya existían pasaron sin cambiar lo que comprueban |

## Probar a mano

Los tres pasos del [diseño](design.md#probar-a-mano), en Windows, con el build de la suite (`target-test/debug/clawg.exe`, que tiene el feature `testing`) y el perfil dev. Cada paso corrió en su propio home de prueba (`CLAWG_TEST_HOME`), así que `~/.clawg-dev` no se abrió. Cada daemon arrancó con `--port 0` y terminó con `clawg stop`.

1. **Un origen que no es https ni loopback.** Con `CLAWG_HF_BASE_URL=http://example.com`, en el primer segundo `clawg status` da `embedder: "failed"` y `embedder_error: "CLAWG_HF_BASE_URL points at http://example.com; use an https:// URL, or an http:// one on localhost, 127.0.0.1 or [::1]"`, sin ninguna petición, con `embedder_retryable: true` y el reintento a los 30 s, como decide D6.
2. **Un espejo que acepta y no contesta.** Un listener en `127.0.0.1:8099` que acepta y nunca escribe (en Windows no hay `nc`: un script de Python), `CLAWG_HF_BASE_URL=http://127.0.0.1:8099` y un directorio de datos sin modelo. El listener recibió tres conexiones, a los 0, 65 y 155 s. Cada intento venció a los 60 s con su `warn!` (`model download of onnx/model.onnx from 127.0.0.1:8099 failed on attempt N of 3: error sending request: operation timed out`), y a los 3 min 35 s `embedder_error` quedó en `retry budget exhausted for onnx/model.onnx from 127.0.0.1:8099 after 3 attempts: error sending request: operation timed out`: el host y la causa, nunca la URL.
3. **Una caché dañada con el modelo suelto.** Sin `CLAWG_HF_BASE_URL` y con `CLAWG_EMBEDDER_IDLE_MINUTES=1`. El primer arranque bajó el modelo real de Hugging Face en 55 s, lo verificó y lo cargó con un solo `tokenizer.json parsed`, y lo soltó a los 60 s sin uso. Con `tokenizer_config.json` borrado, dos `POST /v1/search` seguidas respondieron `503 {"error":"embedder_not_ready"}`, en 8,7 y 9,2 s, y el log dijo `the model files in the cache do not verify: tokenizer_config.json; the model is being downloaded again in the background`. Hubo una sola reparación (`the model files were downloaded again; the next call loads them`, una vez), el archivo volvió, y la tercera búsqueda respondió `200` y dejó el modelo residente. `embedder` siguió en `ready` todo el tiempo.

## Lo que encontró la verificación

- **El «no listo» de una recarga espera el hash de la caché entera.** El diseño esperaba el `503` «enseguida». No espera la red, que es lo que pide R3, pero `check_cache` (`crates/clawg-core/src/memory/model_download.rs:572`) lee y verifica los cinco archivos aunque el que no pasa sea chico, así que el `503` llega después de hashear el ONNX con el cerrojo del modelo tomado: 8,7 y 9,2 s en debug, lo que tarda una recarga buena en leer la caché (9,5 s). D4 ya aceptaba ocupar el cerrojo lo que tarda esa lectura. Queda en el backlog de `openspec/README.md` que la recarga corte en el primer archivo que no pasa.
- **Un fallo del bootstrap se loguea dos veces** (`embedder bootstrap failed: …`, dos líneas iguales en los pasos 1 y 2): la descarga manda su evento `Error` y el bootstrap emite otro al terminar. Ya pasaba antes de este cambio (`48971bf`). Queda en el backlog.
- **Un origen inválido falla aunque la caché esté completa.** `ensure_model` valida la variable antes de mirar la caché (D6: «antes de chequear el disco o pedir nada»), así que un valor mal escrito deja el embedder `failed` también en un arranque que no necesitaba bajar nada. Se acepta: el error aparece en cuanto arranca el daemon y no recién cuando haga falta una descarga, y la recarga del modelo suelto no lee la variable. Queda escrito en `specs/storage`.

## Diferencias entre el diseño y el código

No hay diferencias de comportamiento con la spec. Quedan tres de redacción en el diseño, y en ellas manda el código:

- **`ModelSource::production`** (D1, D6) no existe: `ensure_model` (`model_download.rs:396`) arma en línea el `ModelSource` de producción.
- **`base_url_from`** devuelve `Result<String, String>`, no `Result<Url, String>` (D6), y su error no repite el valor (D6 decía `CLAWG_HF_BASE_URL=<valor> …`): dice `points at <esquema>://<host>` o `is not a URL`, porque el valor puede llevar credenciales. Lo fija `a_base_url_that_is_not_https_or_loopback_is_refused`.
- **La espera entre intentos:** `RETRY_BACKOFF_SECS` (`model_download.rs:243`) y el comentario de `download_one` listan 5, 30 y 120 s; con 3 intentos sólo se usan las dos primeras, como dice R6.

## No aplica

- `rag-eval`: el ranking no cambia.

## Citas realineadas al consolidar

Este cambio movió líneas de `bootstrap.rs`, `model_download.rs`, `runtime.rs` y `crates/clawg-cli/src/daemon/serve.rs`; secret-coverage (51), que entró en `develop` en el mismo merge, no tocó ninguno de los cuatro.

- **Todas las specs y el `README.md` de openspec:** cada cita a uno de esos archivos se comparó con la línea que citaba en `48971bf` (el código antes de este cambio) y se movió a la única línea de `HEAD` con el mismo texto. Así se realinearon 26 en `ingest`, `secrets`, `storage` y el backlog. La de `serve_deps` en `storage` tenía dos líneas con el mismo texto, una por variante de `cfg`; queda la del build con `testing` (`bootstrap.rs:373`), la misma que cita `install` para lo que arma `clawg serve`.
- **Las que ya estaban corridas antes de este cambio** apuntaban a una línea en blanco o a otro ítem, así que se corrigieron por el símbolo que nombran: en `install`, `TEST_EMBEDDER_VAR`, `TestEmbedder`, `serve_deps` (las dos variantes), `production_deps`, `TestEmbedder::parse` y `TestEmbedder::deps`; en `ingest`, `run_embedder_bootstrap_with_retry`; en `storage` y el backlog, el `spawn_blocking` de la carga del bootstrap (`bootstrap.rs:592`); en el backlog, el uso de producción de `load_verified_tokenizer` (`runtime.rs:208`).
- Las citas nuevas de «Descarga del modelo», de la recarga y de la causa del fallo se tomaron de `HEAD`.
