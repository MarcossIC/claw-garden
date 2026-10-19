# conclusion-upkeep — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. La regla y la lista (C1, C2)

- [ ] Tests `source_state_cases`, `review_needs_one_lost_source` y los de la lista en `synthesis.rs`.
- [ ] `SourceState`, `LOST_SOURCE`, `to_review` y `to_review_total` en `synthesis_core`.
- Commit: `feat(core): list the conclusions whose sources were lost`

## 2. Purge (C6)

- [ ] Tests `purging_keeps_the_links_that_name_the_row_as_a_source` (`secrets.rs`) y `a_purged_source_leaves_the_conclusion_to_review` (`synthesis.rs`).
- [ ] `purge` borra sólo los vínculos de la fila como conclusión.
- Commit: `fix(core): keep the link to a purged source`

## 3. El conteo (C4, núcleo)

- [ ] Tests `the_review_count_follows_losses_after_the_last_review` y `a_purged_source_counts_until_the_first_review`.
- [ ] `conclusions_to_review_count`.
- Commit: `feat(core): count the conclusions that lost a source since the last review`

## 4. Fuentes reemplazadas (C5)

- [ ] Tests `replaced_ids_name_their_replacement` y `a_save_from_a_replaced_source_is_refused`.
- [ ] `replaced_memory_ids` y el 400.
- Commit: `feat(cli): refuse a conclusion drawn from a replaced memory`

## 5. HTTP, puente y hook (C2, C3, C4)

- [ ] Tests en `tests/api.rs`, `tools.rs`, `hook.rs`, `client.rs`.
- [ ] `ContextBody.conclusions_to_review`, cliente, `format_synthesis`, `review_nudge`, hook.
- Commit: `feat(cli): show the conclusions to review and remind of them`

## 6. Docs

- [ ] `.claude/CLAUDE.md`.
- Commit: `docs: describe the review of conclusions`

## 7. Verificación

- [ ] `cargo test --workspace` y el clippy de `.claude/CLAUDE.md` en Linux.
- [ ] `verify.md`.

## 8. Archive

- [ ] Specs `synthesis` y `secrets`; `git mv` al archivo; subir el minor.
