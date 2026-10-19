# outbox-lifecycle — Verificación

## Linux (2026-10-19)

Sobre `4d9f83b` (el último commit de código), en `target`, con el compilador de `rust-toolchain.toml`:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0, y `cargo test --locked -p clawg-core --lib build_info` pasa.
- Cada commit de código de las [tareas](tasks.md) (`a1428be`, `624a02b`, `4b9be90`, `4d9f83b`) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` pasa sin bendecir: ninguna descripción ni esquema de herramienta cambió; sólo los textos que devuelven `save_memory` y `memory_status`.

## El binario real

Con `target/debug/clawg`, `CLAWG_TEST_HOME` en un directorio temporal y `CLAWG_TEST_EMBEDDER=fake`: un daemon `dev` en `--port 0`, dos filas `failed` sembradas en su `ingest_outbox`, y después:

- `clawg status` las muestra en `ingest.problems`;
- `clawg outbox retry op-retry` contesta `operation op-retry (agent smoke) is pending again; the daemon is storing it` y la memoria aparece en `clawg memories list`;
- `clawg outbox discard op-discard` contesta `discarded operation op-discard (agent smoke); it had stored nothing`;
- `clawg outbox discard op-nope` sale con 1 y `daemon returned 404: no save waits under operation op-nope`;
- al final `ingest` queda en `pending: 0`, `failed: 0`.

## CI

`ci.yml` corre en el pull request de la rama, sobre Linux y Windows. Windows no se corrió a mano: el cambio no toca código que dependa de la plataforma, y el CI cubre la suite y clippy allí.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `retry_delay_doubles_up_to_a_minute` y los tests del worker no compilaban sin `retry_delay`; `a_transient_failure_is_retried_in_the_same_run` describe una fila que antes quedaba `pending` hasta otro arranque. `a_save_the_model_cannot_take_answers_202_retrying_and_keeps_its_row`: el handler respondía `503 embedder_unavailable` y borraba la fila |
| R2 | `a_partial_save_answers_202_and_keeps_its_row` fallaba contra el handler anterior: `500 ingest_failed` y el outbox vacío. Los casos nuevos de `save_outcome_text_covers_every_daemon_answer` no compilaban sin `memories` y `detail` en `SaveOutcome::Queued` |
| R3 | `a_pause_hands_back_its_attempt` y `a_drain_pauses_a_long_save_between_pieces` no compilaban sin `outbox_pause` y `pause_due` |
| R4 | Sin cambio de código: lo prueban los tests que ya había de `claim_step` y `after_failure`, y `a_pause_hands_back_its_attempt` |
| R5 | Los tests de `outbox.rs` y `cli.rs` no compilaban sin `outbox_find_failed`, `outbox_requeue` y `clawg outbox`. Los de `tests/api.rs` se escribieron antes que las rutas pero no se corrieron en rojo por separado: sin las rutas, axum responde `404` y fallan en la primera aserción de estado |
| R6 | `the_summary_counts_rows_and_lists_the_latest_causes` y `format_status_shows_the_ingest_block` no compilaban sin `OutboxProblem::pieces`, y la línea de `memory_status` no existía |

## Lo que no se midió

- Ningún ranking ni recuperación cambia, así que no hay `rag-eval`.
- El caso en que una fila de la lectura de arranque se reintenta y su llamante ve `202 unconfirmed` ([diseño](design.md), D1) no tiene test propio: es la conducta del `202 unconfirmed` que ya cubren los tests del plazo de confirmación.
