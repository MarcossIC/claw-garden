# docs-index-status — Verificación

## Linux (2026-11-12)

En Linux x86_64, build debug, toolchain 1.99.1, en `target`, sobre `f4fbb83`, el último commit de código de la rama (los que siguen sólo tocan documentos):

- `cargo test --workspace` pasa entero.
- `cargo clippy --workspace --all-targets -- -D warnings` y `cargo clippy -p clawg-cli -- -D warnings` salen con 0, sin ningún warning.

Cada requisito tiene sus tests:

| Requisito | Tests |
|---|---|
| IS1, IS6 | `a_refresh_reports_its_plan_and_each_file_it_finishes`, `the_status_follows_a_refresh_from_its_plan_to_its_end` y `a_claimed_refresh_shows_as_running_until_it_is_released` (`crates/clawg-core/src/memory/project_docs/mod.rs`) |
| IS2 | `the_status_follows_a_refresh_from_its_plan_to_its_end`, `a_failed_refresh_leaves_its_cause_until_one_succeeds` y, sin cambios, `the_last_error_lasts_until_a_refresh_succeeds` (`mod.rs`) |
| IS3 | `an_index_status_serializes_what_health_shows` (`mod.rs`); `health_names_the_project_whose_refresh_runs_and_how_far_it_is`, `health_shows_a_running_refresh_without_the_connection`, `health_says_when_a_refresh_ended_and_what_it_did`, `health_keeps_the_cause_of_a_failed_refresh` y `health_lists_no_docs_index_before_any_refresh` (`crates/clawg-cli/tests/api.rs`) |
| IS4 | `the_docs_start_line_names_the_project_the_trigger_and_the_plan` (`crates/clawg-cli/src/daemon/api.rs`) y `status_shows_the_docs_index_and_the_log_says_when_a_refresh_starts` (`crates/clawg-cli/tests/binary.rs`) |
| IS5 | `the_status_report_shows_the_docs_index` (`crates/clawg-cli/src/status.rs`), `a_health_without_a_docs_index_still_parses` (`crates/clawg-cli/src/client.rs`) y `status_shows_the_docs_index_and_the_log_says_when_a_refresh_starts` (`binary.rs`) |
| IS7 | sin cambios: `search_project_warns_while_indexing` (`crates/clawg-cli/src/mcp/tools.rs`), `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`crates/clawg-cli/src/mcp/server.rs`), y los de la búsqueda de docs en `tests/api.rs` |

Los seis tests de «Cómo se mide» (los cinco de `/health` en `tests/api.rs` y el del binario) se vieron fallar antes del código, por su aserción: `/health` no traía `docs_index`, `clawg status` no lo mostraba y el log no tenía la línea. Están en un commit propio, anterior al código. Los tests de unidad de `mod.rs` se escribieron junto con el código que prueban, no antes.

El contrato fijado del agente pasa sin regenerar sus archivos: `agent_contract.json` y `agent_contract.es.json` no cambiaron. No hay migración nueva.

`rag-eval` no aplica: no cambian el ranking, los vectores ni lo que se indexa.

Lo que aplicar el cambio movió respecto del primer diseño, ya anotado en [design.md](design.md#d5-un-tipo-para-health-el-cliente-pasa-cada-entrada-como-llegó): el cliente no comparte el tipo del daemon para `docs_index`, pasa cada entrada como llegó.

## Lo que estos tests no cubren

- El refresco «a mitad» de los tests se detiene con un embedder de prueba que retiene tandas enteras, sobre dos archivos. Ningún test mira el avance de un refresco de cientos de archivos con el modelo real.
- `files_done` avanza de a tandas de la cola del refresco, no archivo por archivo: los archivos de una tanda se cuentan cuando la tanda se embebió y se escribió. Con el modelo real, entre dos tandas el número puede quedarse quieto varios segundos. Está en el diseño (D2), y no se midió cuánto.
- El disparador `search` se prueba en `clawg-core` (`a_failed_refresh_leaves_its_cause_until_one_succeeds`); por HTTP y por el binario sólo se prueba `registration`.

## CI (2026-11-12)

El CI del pull request pasó en Linux y en Windows sobre `8300329`: clippy con todos los targets, la suite entera y el clippy de `clawg-cli` sin tests. El cambio no toca código `cfg(windows)`; los tests nuevos de `tests/api.rs` y `tests/binary.rs` corrieron también en Windows.

## Pendiente en local

Linux con el embedder falso no comprueba esto. Con un build debug y el perfil `dev`, el modelo real y un repositorio grande (cientos de archivos indexables):

1. **Mirar `clawg status` mientras corre un refresco real.** Registrar el repositorio (`clawg projects add <ruta> --profile dev`, o abrir una sesión del cliente en él) y correr `clawg status --profile dev` varias veces mientras indexa. Esperado en `health.docs_index`: una entrada con el proyecto, `indexing` `true`, `current.trigger` `registration`, `current.files_total` con los archivos por indexar y `current.files_done` que sube hasta igualarlo; al terminar, `indexing` `false`, sin `current`, y `last` con `finished_at_ms` y un `report` que coincide con la línea `indexed the docs of …` del log.
2. **El total falta mientras se recorre el árbol.** En ese mismo repositorio, el primer `clawg status` después de registrar puede mostrar `current` sin `files_total`. Anotar cuánto dura esa ventana.
3. **El disparador `search`.** Con el índice ya armado, cambiar varios archivos, esperar más de un minuto y llamar a `search_project`: el log tiene `started by search` con la cantidad de archivos cambiados, y `clawg status` muestra `current.trigger` `search` mientras corre.
4. **`/health` con el escritor ocupado.** Durante el refresco real, `clawg status` contesta sin demora perceptible y con `docs_index`, aunque `ingest` falte.
5. **Windows.** Repetir el punto 1 en Windows, donde el daemon atiende por su socket AF_UNIX nativo: nada del cambio depende del sistema, pero ahí no se miró.
6. **Después de reiniciar el daemon,** `docs_index` vuelve vacío y el proyecto reaparece cuando el cliente lo registra de nuevo o una búsqueda lo refresca.

## Al archivar (2026-11-12)

- Los ids cambian al consolidar. IS1 a IS15 ya nombran a docs-index-speed en el `Origen` de [`specs/project-docs`](../../../specs/project-docs/spec.md), así que los requisitos de este cambio pasan a ST para no chocar con ellos. Mapeo: IS1, IS2, IS4 e IS6 pasan a ST1, ST2, ST4 y ST6 en `specs/project-docs`; IS3 e IS5 pasan a ST3 y ST5 en [`specs/daemon-access`](../../../specs/daemon-access/spec.md). IS7 («Lo que no cambia») no se consolida. Este archivo conserva los ids IS.
- Sale en 0.83.0.
- «Pendiente en local» no se corrió y queda pendiente. Necesita un build debug, el perfil `dev`, el modelo real y un repositorio grande; ninguno de los seis puntos se midió.
- Citas: el `spec.md` de este cambio no cita líneas de código, y los nombres de test que cita existen en los archivos que nombra. Las citas `path:línea` de `proposal.md`, `design.md` y `exploration.md` describen el código de antes de este cambio y no se corrigieron. En `specs/project-docs`, IC2 tenía cinco líneas de `crates/clawg-cli/src/daemon/api.rs` corridas: se corrigieron al consolidar.
- Diferencias entre la spec del cambio y el código: no se encontró ninguna en lo rechequeado (el log al conocer el plan, `files_total` que viene del plan, `last_error` que dice lo mismo que antes, y el `Drop` del guardia que deja de mostrar el refresco como en curso).
