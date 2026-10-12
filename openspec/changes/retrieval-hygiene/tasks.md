# retrieval-hygiene — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda en verde en cada commit.

## 1. Desempate por distancia (R1)

- [ ] Test `hit_cmp_breaks_score_ties_by_distance` en `search.rs` (unitario).
- [ ] Test `ties_at_zero_follow_the_closest_row_not_the_oldest` en `search.rs` (contra la base en memoria).
- [ ] `hit_cmp`: después del score, `distance` ascendente; después `(kind, id)`.
- Commit: `fix(core): break score ties by distance instead of age`

## 2. Los ceros no cuentan como acceso (R3 en parte, R4)

- [ ] Test `access_ids_keep_only_memory_hits_that_scored` en `access.rs`.
- [ ] Test `recall_does_not_count_zero_score_hits_as_accesses` en `prompt_builder.rs`.
- [ ] Función pura `access_ids(&[RankedHit]) -> Vec<i64>` en `access.rs` (memory y `score > 0`). La usan `build_memory_prompt` y el handler `search` del daemon en lugar de sus filtros inline.
- Commit: `fix(core): stop counting zero-score hits as accesses`

## 3. Corte relativo compartido (preparación de R2)

- [ ] Mover `RELEVANCE_RATIO`, `MIN_SHOWN_HITS` y la regla del corte de `mcp/tools.rs` a `crates/clawg-cli/src/relevance.rs`, como `relevant_len(scores, ratio)`. Los tests del corte se mueven con ella; `relevant_hits` la usa.
- Commit: `refactor(cli): share the relevance cut between bridge and daemon`

## 4. `min_relative_score` en `/v1/search` (R2, R3)

- [ ] Tests `search_applies_the_requested_relative_cut`, `search_without_a_relative_cut_is_unchanged` y `search_rejects_a_relative_score_outside_zero_to_one` en `crates/clawg-cli/tests/api.rs`.
- [ ] Tests unitarios de la validación en `api.rs`.
- [ ] `SearchRequest.min_relative_score: Option<f64>`; 400 si no es finito o está fuera de `[0, 1]`; el corte va antes de registrar accesos y de responder.
- Commit: `feat(daemon): let search apply a relative relevance cut`

## 5. El cliente pide el corte (R5)

- [ ] Test `search_request_carries_the_relevance_ratio` en `client.rs`.
- [ ] `DaemonClient::search` manda `min_relative_score = RELEVANCE_RATIO`. El puente conserva su corte.
- Commit: `feat(cli): ask the daemon for the bridge's relevance cut`

## Verify

- [ ] `cargo test --workspace`: el conteo contra la base, y ninguna falla nueva.
- [ ] `rag-eval --real --diff` y `--agent-view --diff` contra el baseline (necesita el modelo real; si no se puede correr en el entorno, queda pendiente y se dice).
