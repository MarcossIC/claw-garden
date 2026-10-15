# long-save-ranking — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. El set `long` no depende del tope (R4, D3)

- [ ] Test `long_rows_do_not_depend_on_the_piece_cap` (`crates/clawg-eval/src/bin/golden-mine/synth/mod.rs`): fija el digest de las 12 filas `long` de la base de los tests con `--seed 7`, calculado con el código de hoy.
- [ ] `eval_api::fallback_pieces` recibe tope y solapamiento; el generador pasa 424/64 como constantes propias.
- Commit: `refactor(eval): generate long rows against a cap of their own`

Va primero para que el commit del tope no mueva el set.

## 2. Pedazos chicos (R1, D1)

- [ ] Tests `a_long_text_is_split_into_small_pieces` y `small_pieces_share_their_own_overlap` (`chunker/chat.rs`).
- [ ] `PIECE_MAX_TOKENS` = 96 y `PIECE_OVERLAP_TOKENS` = 24 (`chunker/mod.rs`), con sus aserciones de compilación; `split_save` corta con ellos.
- [ ] Ajustar en el mismo commit los fixtures que contaban pedazos de 424 (worker, pieces, search, MCP), sin bajar el conteo de tests.
- Commit: `feat(core): split a long save into small pieces so its answer is not diluted`

## 3. Los saves guardados siguen igual (R2, D4)

- [ ] Test `a_save_split_at_the_old_cap_is_still_found_and_read` (`pieces.rs`): siembra con SQL un save de 3 pedazos de 424 con su vínculo y contexto, y otro guardado por el worker con el tope nuevo; los dos salen en un hit y se leen enteros.
- Commit: `test(core): keep saves split at the old cap searchable and readable`

Si el test pasa sin código nuevo (lo esperado), el commit es sólo el test: fija el contrato.

## 4. El pozo cuenta saves (R3, D2)

- [ ] Tests puros `distinct_saves_counts_unlinked_rows_as_their_own` y `trim_to_saves_keeps_the_rows_up_to_k_saves`.
- [ ] Test `a_save_with_more_pieces_than_the_pool_leaves_room_for_other_memories` (falla hoy: el pozo se llena de hermanos).
- [ ] Extraer la lectura de vínculos de `save_links` a una función síncrona; en `knn_one` de `memory`, segunda pasada cuando faltan saves y recorte por saves.
- Commit: `fix(core): fill the search pool with distinct saves, not sibling pieces`

## 5. Docs

- [ ] `CLAUDE.md` (el párrafo de pedazos: "split at a smaller cap") y la descripción de `save_memory` si nombra el tamaño.
- [ ] `openspec/README.md`: fila 19.
- Commit: `docs: describe the smaller pieces of a long save`

## 6. Verificación

- [ ] `cargo test --workspace` en cada commit; conteo contra la base.
- [ ] `verify.md` con los pasos de Marcos: U0 a U3 sobre su set real, y la suite en Windows.
