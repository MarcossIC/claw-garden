# docs-refresh-background — Verificación

## Linux (2026-10-22)

Sobre `8ac4f66`, en `target`, con el compilador de `rust-toolchain.toml`. El build no pudo bajar ONNX Runtime de `cdn.pyke.io` (la red lo rechazó), así que `ort-sys` se enlazó dinámicamente contra la misma versión, 1.24.2, sacada del wheel `onnxruntime` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1` y `LD_LIBRARY_PATH` a ese directorio). Ningún test usa el modelo real; el cambio no toca ese enlace.

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0, y `cargo test --locked -p clawg-core --lib build_info` pasa.
- Cada commit de código de las [tareas](tasks.md) (`42a7b19`, `8c2c5bf`, `277709b`) pasó la suite y clippy antes de commitearse.

## El binario real

Con `target/debug/clawg` de un build de tests, `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y `CLAWG_TEST_DEFAULT_PORT=off`, un daemon `dev` en `--port 0`:

- `POST /v1/projects` con `index_docs: true` sobre una raíz con un `README.md`: el log dice `indexed the docs of …: 1 docs, 1 reindexed`, y la búsqueda siguiente trae su pasaje con `indexing: false`.
- Se editó el `README.md`, se agregaron 30 docs y se esperaron 61 s: la búsqueda que encontró el proyecto vencido lanzó el refresco, lo vio terminar dentro del plazo (57 ms con el embedder falso) y respondió con `docs: 31`, el pasaje nuevo e `indexing: false`.
- `clawg stop` lo paró.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `a_search_answers_from_the_index_while_a_long_refresh_runs` no volvía: el `timeout` de `REFRESH_GRACE` más 5 s venció con la búsqueda esperando el refresco en línea. `the_refresh_outlives_the_search_that_started_it` vio el índice con un solo doc a los 10 s: el cliente se fue y el refresco se canceló con el handler. `begin_if_due_claims_only_a_due_idle_project` no compilaba. `a_small_edit_is_found_by_the_search_that_refreshes` ya pasaba: guarda la frescura de una edición chica |
| R2 | `a_background_refresh_that_failed_warns_the_next_search` respondía sin `warning`: el error del refresco del registro sólo iba al log. `the_last_error_lasts_until_a_refresh_succeeds` no compilaba sin `last_error` |
| R3 | `an_unusable_vector_skips_its_doc_and_the_rest_go_on` y `a_short_batch_still_aborts_the_refresh` no compilaban sin `failed` ni `VectorCount`. `a_doc_the_index_could_not_embed_fails_the_measurement` se escribió después del salto de `refresh` y falló por la aserción: `index` devolvía `Ok` con el doc salteado fuera de la medición |

## Lo que no se midió

- Cuánto tarda en release el refresco de una edición chica con el modelo real, que pediría el modelo y un perfil de datos: `REFRESH_GRACE` se eligió con la única cifra que hay (un debug), como dice el [diseño](design.md), D1.
- Ningún ranking cambia, así que no hay `rag-eval`; `rag-eval --docs` llama a `refresh` directamente y sólo gana el control de R3.
- Windows no se corrió a mano: el cambio no toca código que dependa de la plataforma, y el CI del pull request cubre la suite y clippy allí.
