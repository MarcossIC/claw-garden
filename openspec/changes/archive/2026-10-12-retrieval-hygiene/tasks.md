# retrieval-hygiene — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda sin fallas nuevas en cada commit.

## 1. Los ceros no cuentan como acceso (R2 en parte, R3)

- [x] Tests `record_hits_keeps_only_memory_hits_that_scored` y `record_hits_with_nothing_countable_sends_nothing` en `access.rs`.
- [x] Test `recall_does_not_count_zero_score_hits_as_accesses` en `prompt_builder.rs`.
- [x] `AccessTracker::record_hits(&[RankedHit])` registra los `memory` con `score > 0`. Lo usan `build_memory_prompt` y el handler `search` en lugar de sus filtros inline.
- Commit: `fix(core): stop counting zero-score hits as accesses` (`c399565`)

## 2. Corte relativo compartido (preparación de R1)

- [x] `RELEVANCE_RATIO`, `MIN_SHOWN_HITS` y la regla del corte pasan de `mcp/tools.rs` a `crates/clawg-cli/src/relevance.rs` como `relevant_len(scores, ratio)`. Sus tests se mueven con ella; `relevant_hits` la usa.
- Commit: `refactor(cli): share the relevance cut between bridge and daemon` (`941b0b8`)

## 3. `min_relative_score` en `/v1/search` (R1, R2)

- [x] Tests HTTP `search_without_a_relative_cut_is_unchanged`, `search_applies_the_requested_relative_cut` y `search_rejects_a_relative_score_outside_zero_to_one`.
- [x] Tests unitarios de la validación en `api.rs`.
- [x] `SearchRequest.min_relative_score`; 400 fuera de `[0, 1]`; el corte va antes de registrar accesos y de responder.
- Commit: `feat(daemon): let search apply a relative relevance cut` (`59d0990`)

## 4. El cliente pide el corte (R4)

- [x] Test `search_request_carries_the_relevance_ratio` en `client.rs`.
- [x] `DaemonClient::search` manda `min_relative_score = RELEVANCE_RATIO`. El puente conserva su corte.
- Commit: `feat(cli): ask the daemon for the bridge's relevance cut` (`68b6e61`)

## Verify

- [x] `cargo test --workspace`: sin fallas nuevas contra la base (ver `verify.md`).
- [x] `rag-eval --real --diff` y `--agent-view --diff` contra el baseline: diff vacío en los dos (ver `verify.md`).
