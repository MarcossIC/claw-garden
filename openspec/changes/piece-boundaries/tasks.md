# piece-boundaries — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla por la aserción o por no compilar. Una tarea es un commit convencional que compila y pasa la suite. Los tests que sólo guardan lo que ya hace el código se marcan como guardas y pasan desde el principio.

## 0. Base (U0)

Requisitos: el modelo real del perfil dev, el golden set local (`docs/eval/golden.json`, fuera de git) y un export de `engram export --all` fuera del repositorio. Tocar el perfil dev necesita autorización explícita.

- [x] Con el binario de `develop`, `rag-eval --real` y `rag-eval --real --agent-view` sobre el ancla de 46 casos, y `rag-eval --real --agent-view --golden <set>` con el set de `golden-mine synth --seed 7`. Se guardan los JSON fechados en `docs/eval/results/`.
- [x] Con un daemon de ese binario en el perfil dev, `clawg import --from engram --dry-run` del export: filas que insertaría, en total y por idioma. Sólo conteos. El store del perfil dev ya tiene la importación hecha, así que el daemon corre con `CLAWG_TEST_HOME` sobre una copia del modelo y del snapshot `pre-import`, y `~/.clawg-dev` no se abre.

## 1. El fixture de subpalabras

- [x] `subword_tokenizer()` en `cfg(test)`, junto a `word_level_tokenizer` (`token_splitter.rs:716`): un `WordPiece` sobre `WhitespaceSplit` con raíces y continuaciones `##`, y un generador de texto cuyas palabras tienen dos o tres subtokens. Un test del propio fixture fija que parte `retrievies` en tres tokens y que sus offsets caen dentro de la palabra.
- [x] Commit: `test(chunker): add a subword tokenizer fixture`.

## 2. Las marcas y el plan, puros (R1)

- [x] Tests primero: `word_starts_marks_tokens_after_or_on_whitespace`, `word_starts_takes_an_offset_that_holds_the_leading_space`, `plan_word_windows_with_every_start_marked_is_plan_windows`, `plan_word_windows_terminates_covers_and_never_nests` (barrido exhaustivo de `n`, `max`, `overlap` y patrones de marcas), `plan_word_windows_cuts_at_the_last_word_start_under_the_cap`, `plan_word_windows_cuts_a_word_longer_than_the_cap_by_token` y `plan_word_windows_keeps_at_least_the_overlap`.
- [x] `word_starts` y `plan_word_windows` en `token_splitter.rs` ([design D1 y D2](design.md#d1-la-frontera-sale-de-los-bytes-no-de-word_ids)). Fuera de sus tests todavía no los usa nadie, y `dead_code` es un error en este repositorio, así que esta tarea va en el commit de la tarea 3.

## 3. El divisor corta en frontera (R1, R2, R3)

- [x] Tests primero *(subpalabras)*: `no_piece_starts_or_ends_inside_a_word` y `a_word_longer_than_the_cap_is_cut_by_token_and_loses_no_byte` (`token_splitter.rs`); `the_head_ends_at_a_word_boundary` y `no_piece_of_a_long_save_starts_inside_a_word` (`chunker/chat.rs`); `the_pieces_of_a_subword_split_save_join_back_into_its_text` (`pieces.rs`); `a_long_note_is_not_cut_inside_a_word` (`translate.rs`); `a_long_section_is_not_cut_inside_a_word` (`project_docs/chunk.rs`).
- [x] Guarda: `a_text_without_spaces_is_cut_as_before` (`token_splitter.rs`).
- [x] `split_by_tokens` usa `word_starts` y `plan_word_windows`. Si `plan_windows` queda sin más uso que sus tests, se borra con ellos, y el test de equivalencia de la tarea 2 compara contra una tabla fija de casos en lugar de contra `plan_windows`.
- [x] Commit: `fix(chunker): cut pieces and the save head at word boundaries`.

## 4. Lo guardado antes no cambia (R4)

- [x] Guarda: `a_save_cut_inside_words_before_is_still_found_read_and_reembedded` (`pieces.rs`). Inserta y vincula un save con piezas cortadas a mitad de palabra, como las dejaba el divisor anterior, y comprueba la búsqueda, `read_save_core` y el texto embebido del re-embebido.
- [x] Comprobar que `long_rows_do_not_depend_on_the_piece_cap` y los tests de `split_by_bytes` pasan sin cambios.
- [x] Commit: `test(pieces): keep saves cut inside words readable`.

## 5. El tokenizador real (R5)

- [x] `the_real_tokenizer_cuts_no_piece_inside_a_word` en `crates/clawg-core/tests/e5_golden.rs`, `#[ignore]`, con el `tokenizer.json` de `CLAWG_E5_MODEL_DIR`. Lo que el test necesita de `clawg-core` pasa por `eval_api`, sin abrir visibilidad nueva en producción. Se corre con el modelo antes y después de la tarea 3: antes falla, después pasa.
- [x] Commit: `test(chunker): check word boundaries with the real tokenizer`.

## 6. Comentario desfasado

- [x] `pieces.rs:58-59`: el chunker repite 24 tokens entre piezas de un save (64 entre chunks) y, desde este cambio, hasta la palabra. Va en el commit de la tarea 3 si se toca el mismo archivo; si no, como `docs(pieces): …`.

## 7. Medición del corte (U1, U2)

Mismos requisitos que la tarea 0.

- [x] Con la punta, las mismas corridas de la tarea 0, y `rag-eval --diff --fail-on-regression` contra U0, con y sin `--agent-view`. Se lee `long` caso por caso.
- [x] La importación en seco con un daemon de la punta: filas contra U0, con el techo del 5 %.
- [x] Si U1 o U2 no se cumplen, se detiene el cambio y se vuelve al diseño. Con la primera versión de D3 (el solape crece hasta la frontera anterior), U1 se cumplió y U2 no: 56.514 filas contra 51.996, un 8,7 % más. D3 pasa a la frontera más cercana con un piso de 16 caracteres (tarea 7b).

## 7b. El solape a la frontera más cercana (D3)

- [ ] Tests primero: `char_spans_count_the_characters_before_each_token`, `plan_word_windows_starts_at_the_word_start_nearest_the_overlap` y `plan_word_windows_keeps_the_characters_join_pieces_needs` (puros); el barrido de `plan_word_windows_terminates_covers_and_never_nests` con la regla nueva del paso 2, y `no_piece_starts_or_ends_inside_a_word` con el solape de 24 tokens, uno más o uno menos, y 16 caracteres.
- [ ] `char_spans`, `MIN_OVERLAP_CHARS` en `pub(crate)` y el paso 2 de D2. `plan_word_windows_keeps_at_least_the_overlap` se va con la regla que fijaba.
- [ ] Commit: `fix(chunker): start the next piece at the nearest word start`.
- [ ] Las corridas de la tarea 7 con la punta nueva, contra U0. Si U1 o U2 no se cumplen, se vuelve al diseño.

## 8. Brazos en la rama de medición (U3, U4)

[Design D6](design.md#d6-la-rama-de-medición). Un worktree aparte desde la punta, que se borra con su build al terminar.

- [ ] Brazo (a), TDD: `sentence_starts_marks_a_word_after_a_full_stop_or_a_blank_line`, `a_cut_at_a_sentence_end_keeps_the_shortest_overlap_that_joins` y `the_pieces_cut_at_sentence_ends_join_back_into_their_text` *(subpalabras)*; `sentence_starts` y la regla de D4, con la aserción de compilación del retroceso. Se mide con `SENTENCE_LOOKBACK_TOKENS` en 4, 8 y 12: las corridas de la tarea 7 y la importación en seco, contra U1.
- [ ] Brazo (b): `SAVE_HEAD_TOKENS` en 32, 24 y 16, con las corridas de `rag-eval` de la tarea 7, contra U1.
- [ ] Si los dos pasan, la combinación de los valores elegidos se mide una vez más contra U1.
- [ ] Cada valor se elige leyendo `by_split.calibration` y se informa `by_split.test`.
- [ ] El brazo que pasa se lleva a la rama del cambio como un commit con sus tests: `perf(chunker): …` para (a) y para (b). El que no pasa no deja código.

## 9. Docs

- [ ] `.claude/CLAUDE.md`: el párrafo de los saves largos dice que los bordes caen en frontera de palabra con el tokenizador cargado, y el brazo adoptado si lo hay.
- [ ] El mapa de `openspec/README.md`.

## 10. Verificación

- [ ] Suite y clippy de `.claude/CLAUDE.md` en Linux y en Windows (`target-test`), y el CI en verde.
- [ ] `verify.md`: el resultado de U0-U4 con las rutas de los JSON fechados, el recuento de filas de cada build, los brazos que no se adoptaron y por qué, y la advertencia de D5 sobre los casos `long`. Sin textos ni ids de filas.
- [ ] Ningún directorio de build fuera de `target` y `target-test`.

## 11. Archive

- [ ] `specs/pieces` con R1, R2, R4 y el brazo adoptado; `specs/project-docs` con R3; el cambio a `changes/archive/AAAA-MM-DD-piece-boundaries/`.
- [ ] Subir el minor de `[workspace.package] version`, `cargo update --workspace`, y el commit `chore: bump version to X.Y.Z`.
