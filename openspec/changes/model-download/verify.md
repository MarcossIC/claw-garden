# model-download — Verificación

## Linux (2026-10-19)

Sobre `09fcd2f` (el último commit de código), en `target`, con rustc 1.97.0:

- `cargo test --workspace`: pasa, ningún test falla. El único ignorado es `the_vectors_are_the_ones_fastembed_gave` (`crates/clawg-core/tests/e5_golden.rs`), que pide el modelo real y no es de este cambio.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Cada commit de las [tareas](tasks.md) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` pasa sin bendecir: nada de lo que el modelo ve del servidor MCP cambió.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `a_body_that_stops_arriving_fails_the_attempt` y `a_server_that_never_answers_fails_the_attempt`: `ensure` no volvía y el `tokio::time::timeout` de 30 s del test vencía |
| R2 | `an_exhausted_download_names_the_last_status_and_the_host`: el texto no tenía `HTTP 503` ni el host; los dos tests de plazos no decían `timed out` |
| R3 | `a_reload_with_a_damaged_cache_is_not_ready_and_repairs_it_once_in_the_background`: no compilaba sin `CachedModelFn`; con la reparación esperada dentro de la recarga (lo de antes), la llamada no contestaba en 1 s |
| R4 | `the_cache_check_leaves_the_runtime_thread_free`: el hilo del runtime no avanzaba mientras se hasheaba; `each_file_is_hashed_once_per_run`: el archivo bajado se volvía a leer; `the_worker_resolves_the_tokenizer_off_the_runtime_thread`: el tokenizer se leía en el hilo del test; `a_file_larger_than_any_model_file_is_refused_without_holding_it`: el proceso abortaba al reservar 1 TB |
| R5 | No compilaba sin `base_url_from` |
| R6 | Refactor: los tests que ya existían pasaron sin cambiar lo que comprueban |

## Pendiente

- El CI de la rama en Linux y Windows (corre con el pull request).
- «Probar a mano» del [diseño](design.md#probar-a-mano) sobre el perfil dev: pide autorización para tocar `~/.clawg-dev`.
- Al archivar: consolidar R3 en `openspec/specs/storage/spec.md` («Recargar a demanda, una sola vez», «Una recarga que falla no deja nada roto»), R2 en `openspec/specs/ingest/spec.md` («Estado en `/health` y `clawg status`») y R1, R4 y R5 en un apartado de la descarga del modelo; subir el minor.
