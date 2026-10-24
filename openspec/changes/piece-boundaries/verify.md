# piece-boundaries — Verificación

Windows 11, binarios debug de `target-test`, el 2026-10-24. La base es `develop` en `7a1fd53`, y la punta la rama del cambio. Las corridas con el modelo real leen una copia de `~/.clawg-dev/models/multilingual-e5-small` en un `CLAWG_TEST_HOME` aislado, así que `~/.clawg-dev` no se abrió ni se escribió.

## Commits

| Commit | Qué |
|---|---|
| `7ebec47` | `test(chunker): add a subword tokenizer fixture` |
| `be40e7d` | `fix(chunker): cut pieces and the save head at word boundaries` (D1, la primera versión de D2/D3) |
| `db2e7c2` | `test(pieces): keep saves cut inside words readable` (R4) |
| `1921a5d` | `test(chunker): check word boundaries with the real tokenizer` (R5) |
| `4738711` | `docs(openspec): …` D3 pasa a la frontera más cercana después de que U2 no se cumpliera |
| `3341534` | `fix(chunker): open a word at a lone whitespace token` (D1) |
| `2af509b` | `fix(chunker): start the next piece at the nearest word start` (D3) |
| `0e9fe2f` | `docs(openspec): …` D1 y la segunda medición |
| `5ba7ff6` | `perf(chunker): embed each later piece behind a head of 32 tokens` (brazo b, R6b) |

## Requisitos

| Requisito | Tests | Estado |
|---|---|---|
| R1. Corte en frontera de palabra | `word_starts_marks_tokens_after_or_on_whitespace`, `word_starts_takes_an_offset_that_holds_the_leading_space`, `char_spans_count_the_characters_before_each_token`, `plan_word_windows_with_every_start_marked_keeps_a_fixed_stride`, `plan_word_windows_clamps_the_overlap_and_never_hangs`, `plan_word_windows_terminates_covers_and_never_nests`, `plan_word_windows_cuts_at_the_last_word_start_under_the_cap`, `plan_word_windows_cuts_a_word_longer_than_the_cap_by_token`, `plan_word_windows_starts_at_the_word_start_nearest_the_overlap`, `plan_word_windows_keeps_the_characters_join_pieces_needs`; *(subpalabras)* `no_piece_starts_or_ends_inside_a_word` y `a_word_longer_than_the_cap_is_cut_by_token_and_loses_no_byte`; la guarda `a_text_without_spaces_is_cut_as_before` | ✓ |
| R2. Piezas y cabecera de un save | *(subpalabras)* `the_head_ends_at_a_word_boundary`, `no_piece_of_a_long_save_starts_inside_a_word` (`chunker/chat.rs`) y `the_pieces_of_a_subword_split_save_join_back_into_its_text` (`pieces.rs`) | ✓ |
| R3. Importación e índice de docs | `a_long_note_is_not_cut_inside_a_word` (`translate.rs`), `a_long_section_is_not_cut_inside_a_word` (`project_docs/chunk.rs`) | ✓ |
| R4. Lo guardado antes no cambia | `a_save_cut_inside_words_before_is_still_found_read_and_reembedded` (`pieces.rs`); los de `split_by_bytes` y `long_rows_do_not_depend_on_the_piece_cap` sin cambios; el set de `golden-mine synth --seed 7` sale idéntico byte a byte con la base y con cada punta | ✓ |
| R5. El tokenizador real | `the_real_tokenizer_cuts_no_piece_inside_a_word` (`e5_golden.rs`, `#[ignore]`): falla en `7a1fd53` (la cabecera termina en `link_piece` · `s();`); con la regla de D3 y sin la corrección del `▁` suelto, una pieza tiene 97 tokens; pasa en `2af509b` y en `5ba7ff6` | ✓ |
| R6. Brazos | R6b con 32: `the_head_is_the_first_tokens_of_the_text` y `the_head_ends_at_a_word_boundary` leen `SAVE_HEAD_TOKENS`. R6a no entró y no dejó código | ✓ |

Antes del corte en frontera fallaban por la aserción `no_piece_starts_or_ends_inside_a_word`, `a_word_longer_than_the_cap_is_cut_by_token_and_loses_no_byte`, `the_head_ends_at_a_word_boundary`, `no_piece_of_a_long_save_starts_inside_a_word`, `a_long_note_is_not_cut_inside_a_word` y `a_long_section_is_not_cut_inside_a_word`.

**Diferencias con los documentos de antes de apply:**

- `the_pieces_of_a_subword_split_save_join_back_into_its_text` pasaba antes del cambio. La propuesta lo listaba entre los que fallan, pero `join_pieces` busca el solape en el texto y nunca dependió de dónde cae el corte: es una guarda.
- `plan_windows` quedó sin uso fuera de sus tests y se borró con ellos, como preveía D2. `plan_word_windows_with_every_start_marked_is_plan_windows` se llama `plan_word_windows_with_every_start_marked_keeps_a_fixed_stride`, y `split_ascii_byte_path_matches_plan_windows_shape` se llama `split_ascii_byte_path_cuts_windows_of_the_cap_at_its_stride`, con tablas fijas en lugar de `plan_windows`. Las propiedades de los tests de `plan_windows` (paso fijo, conteo, tope, clamp del solape) siguen probadas sobre `plan_word_windows` con todas las marcas.
- `plan_word_windows_keeps_at_least_the_overlap` fijaba la primera regla de D3 y se fue con ella.

## Suite

`cargo test --workspace --no-fail-fast --target-dir target-test`, en `2af509b` y en `5ba7ff6`: 3.337 pasan, 0 fallan, 3 ignorados (los que necesitan el modelo). `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sin warnings. Linux queda para el CI del pull request. Al terminar quedan sólo `target` y `target-test`.

## Medición

Las condiciones son las de la [propuesta](proposal.md#éxito). Los JSON fechados están en `docs/eval/results/` (fuera de git), con el prefijo `2026-10-24-real-piece-boundaries-`. El recuento de filas sale de `clawg import --from engram --dry-run` de un `engram export --all` fresco (3.814 notas, 52.080 filas en el archivo), contra una copia del snapshot `pre-import` del perfil dev, porque el store del perfil ya tiene la importación hecha y un dry run contra él encontraría todo presente. El export se guardó fuera del repositorio y se borra al cerrar el cambio, porque tiene el texto sin enmascarar.

### U0, la base (`7a1fd53`)

| Corrida | Archivo | Resultado |
|---|---|---|
| ancla | `base.json` | R@5 0,6522 |
| ancla, agent-view | `base-agent-view.json` | `search_memory` 36/46 (0,7826), `recall_context` 37/46 (0,8043) |
| seed 7, agent-view | `base-seed7-agent-view.json` | `long`: hit rate 1,0 en búsqueda y contexto, `search_reachable` 1,0, `search_visible` 0,917, `context_visible` 0,917 |
| importación en seco | — | 51.996 filas (en 14.569, es 20.261, und 17.166) |

### U1 y U2, la primera versión de D3 (`be40e7d`)

- U1 ✓: `rag-eval --diff --fail-on-regression` contra U0 sale con 0 y 0 casos se mueven, sin `--agent-view` y con él. En seed 7, ningún caso `long` pierde visibilidad ni hallazgo, uno gana `search_visible` y `context_visible`, y ningún otro subconjunto baja.
- U2 ✗: 56.514 filas, un 8,7 % más que U0, por encima del techo del 5 %. Por idioma: en 19.135, es 27.549, und 9.830.

El cambio volvió a diseño. Una simulación de las reglas sobre el mismo export y el tokenizador real reproduce la medición (+8,85 %) y está en [D3](design.md#d3-el-solape-va-a-la-frontera-más-cercana). Con el corte de antes, 57.921 de unos 97.000 cortes caían dentro de una palabra.

### U1 y U2, la regla de D3 (`2af509b`)

- U1 ✓: igual que la primera versión. 0 casos se mueven en el ancla, sin `--agent-view` y con él; en seed 7 el mismo caso `long` gana visibilidad, ninguno pierde y ningún otro subconjunto baja.
- U2 ✓: 54.321 filas, un 4,47 % más que U0. Por idioma: en 18.808, es 26.868, und 8.645. La simulación daba +4,50 %.

El idioma indeterminado baja de 17.166 a 8.645 filas: una pieza que empieza en una palabra entera le da al detector más palabras que reconoce.

### U3 y U4, los brazos

Cada valor es un build propio de una rama de medición que salió de `2af509b`, medido contra U1 (`tip2`). Los JSON llevan los sufijos `b32`, `b24`, `b16`, `a4` y `a12`. Ningún valor mueve un caso del ancla, sin `--agent-view` ni con él: ninguna de sus filas pasa de una pieza, así que `by_split.calibration` y `by_split.test` quedan iguales y la elección sale del set seed 7.

| Valor | Filas en la importación en seco | Casos `long` | Otros subconjuntos | Veredicto |
|---|---|---|---|---|
| (b) cabecera 32 | sin cambio (la cabecera no cambia las filas) | ninguno pierde | nada baja | **se adopta** |
| (b) cabecera 24 | sin cambio | ninguno pierde | `multi_ref`: `all_hit_rate` baja 0,083 en búsqueda y en contexto (un caso de 12) | sin regresión según la propuesta; no se toma por D5 |
| (b) cabecera 16 | sin cambio | uno pierde `search_visible` y `context_visible` | `multi_ref`: `context.all_hit_rate` baja 0,083 | no pasa |
| (a) retroceso 4 | 53.186 (−2,1 % contra U1) | dos pierden `context_visible` | nada baja | no pasa |
| (a) retroceso 8 | — | — | — | sin medir: el build falló por falta de memoria de la máquina (`link.exe` 0xc0000142) |
| (a) retroceso 12 | 52.241 (−3,8 % contra U1) | uno pierde `search_visible` y `context_visible` | nada baja | no pasa |

- **U4:** al pie de la letra la menor cabecera sin regresión es 24. Se adoptó 32 por la advertencia de D5 y porque el único retroceso del brazo, fuera de 16, aparece en 24 ([design D5](design.md#d5-brazo-b-una-cabecera-más-corta)).
- **U3:** ningún valor deja un 5 % menos de filas que U1, y los dos medidos pierden casos `long`. El brazo (a) no se adopta y no dejó código. 8 no se repitió: queda entre dos valores que no cumplen.
- Como entró un solo brazo, no hubo combinación que medir.

La rama de medición y su worktree se borraron, y el export de engram también, porque tenía el texto sin enmascarar.

### Advertencia de D5

Los 12 casos `long` abren con la clave del save en la primera oración. Eso favorece a una cabecera corta, y la medición del brazo (b) puede sobrestimarla.
