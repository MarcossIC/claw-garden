# pipeline-metrics — Verificación

## Suite y clippy

- Linux, al cerrar el apply (lo registró `state.yaml` en `6d3f89d`): `cargo test --workspace` y `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` pasan.
- CI, `test (ubuntu-latest)` y `test (windows-latest)` en verde (la suite, clippy y lo que construye el release, en los dos sistemas):
  - tramo 1, pull request #100, sobre `db2a5f9`;
  - tramos 2 a 7, pull request #101, sobre `6d3f89d` (run 40984497420, 2026-10-30), el último commit del cambio.

## El spec contra el código

Sobre `10cf418`, el 2026-10-30:

- Los 34 tests que nombra el [spec](spec.md) existen, cada uno en el fichero que dice.
- Toda etiqueta sale de un conjunto cerrado del código: `arm` y `table` (`record_arm`, `crates/clawg-core/src/memory/search.rs:1467`; `:1360`), `stage` (`record_stage`, `crates/clawg-core/src/memory/worker.rs:747`), `role` (`embedder.rs:441`, `:509`, `:644`), `phase` (`bootstrap.rs:404`), `state` y `block` (`retrieval.name()`, `block.as_str()`, `crates/clawg-cli/src/daemon/api.rs:1655-1658`; `record_refresh`, `:3016`), `file` (`crates/clawg-cli/src/daemon/store_probe.rs:73`). Los detalles de los eventos son números y `&'static str` (`DetailValue`, `crates/clawg-core/src/metrics/event.rs`).
- `memory.db` no cambia: ninguna migración nueva, y las tablas nuevas son de `metrics.db` (`event`, `crates/clawg-core/src/metrics/store.rs:420`, `SCHEMA_VERSION = 2` en `:19`). `agent_contract.json` y `agent_contract.es.json` no cambian.
- D8: el evento `shutdown` se registra después del drain y antes de `writer.close(METRICS_CLOSE)` (`crates/clawg-cli/src/daemon/serve.rs:316-321`); los plazos de las fases no cambian.
- Los eventos se podan con las horas (`HOURS_KEPT_MS`, `prune`, `store.rs:258`), y una ejecución con eventos no se borra mientras los tenga.

## Lo que no se midió

- El sobrecoste con el modelo real (p95 de `http.duration{route=/v1/search}` con y sin la sonda) pide el perfil dev con el modelo y un build sin la sonda para comparar, porque nada la apaga: `CLAWG_METRICS=off` sólo deja de escribir `metrics.db`, y `serve` la arranca siempre (`serve.rs:242`). No se usó ese perfil sin autorización, como en 71. La sonda es una llamada sin SQL cada 10 s en la cola de la conexión, y una vez por minuto una lectura de `sqlite3_db_status`, tres `metadata` y la consulta del outbox de `/health`, frente a los milisegundos de un embebido.
- `embedder.batch_tokens` sólo lo da la sesión del modelo real; la suite cubre `embedder.batch` y `embedder.wait` con el canal de inferencia y no carga el modelo.
- Ningún ranking cambia: no hay `rag-eval`.
