# long-save-ranking — Verificación

**Código:** rama `claude/project-thread-z8r7xv`, un commit por tarea, sobre `main` en `9aa1012`.
**Modo:** Strict TDD (`openspec/config.yaml`). Los tests de R1 y R3 fallaron antes de su código (por la aserción o por no compilar). Los de R2 y R4 fijan un contrato que hoy se cumple: el de R4 falla si se baja el tope sin su commit (comprobado con el tope en 96 antes del cambio del generador); el de R2 pasa sin código nuevo, como se esperaba.
**Estado:** R1 a R4 pasan en Linux, commit por commit. Con el modelo real sobre el set sustituto, `long` pasa de 9 a 12 saves hallados y la respuesta visible en `search_memory` de 0 a 11 de 12. **Falta lo que corre en la máquina de Marcos:** la suite en Windows y U0 a U3 sobre su set real ([pasos](#pendiente-máquina-de-marcos)).

## Suite

`cargo test --workspace --no-fail-fast` en Linux:

| Commit | Pasan | Fallan |
|---|---|---|
| `9aa1012` (base, `main`) | 1848 | 10 |
| `2aacb02` docs(openspec): propose smaller pieces so a long save's answer ranks | 1848 | 10 |
| `0c04e0c` refactor(eval): generate long rows against a cap of their own | 1849 | 10 |
| `571ff7b` feat(core): split a long save into small pieces so its answer is not diluted | 1850 | 10 |
| `43145e8` test(core): keep saves split at the old cap searchable and readable | 1851 | 10 |
| `6e4d2e3` fix(core): fill the search pool with distinct saves, not sibling pieces | 1854 | 10 |
| `9e162b6` docs: describe the smaller pieces of a long save | 1854 | 10 |

- **+6 tests.** `571ff7b` reemplaza `a_long_text_is_split_below_the_cap_to_leave_room_for_the_head` por dos tests; el conteo no baja en ningún commit.
- **Los 10 que fallan son los de `main`** (por nombre): `doctor::tests::*` (7, sólo pasan en Windows) y `rag-eval` `diff::tests::*` (3, leen `docs/eval/results/`, que `.gitignore` deja afuera).
- **Warnings:** los de `main` (`insert_row_with_embedding`, `search_all_memories_with_scoring`); ninguno nuevo.
- **Fixtures ajustados en `571ff7b`.** Un save largo ya no puede tener 2 o 3 pedazos: el más corto que se parte (más de 480 tokens) da al menos 7 de 96. Los tests que fijaban "dos pedazos" o "tres pedazos" (worker, `/v1/memories`, `read_memory`) ahora piden "varios" o el número que da el fixture; los de pedazos idénticos usan 7 copias de 384 bytes. `a_chunk_still_disputes_an_older_row` usa un texto más corto: con 1.600 palabras daba 39 pedazos girados hasta 0,38 rad, y los últimos ya no caían en la banda de disputa con la fila vieja, que es otra cosa que lo que el test mide.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. Pedazos chicos | `a_long_text_is_split_into_small_pieces`, `small_pieces_share_their_own_overlap` (`chunker/chat.rs`); `a_text_that_fits_one_piece_is_split_as_before` sin cambios; las aserciones de compilación en `chunker/mod.rs` | ✓ |
| R2. Saves guardados | `a_save_split_at_the_old_cap_is_still_found_and_read` (`pieces.rs`); `the_pieces_of_a_split_save_join_back_into_its_text` con el tope nuevo | ✓ |
| R3. El pozo cuenta saves | `distinct_saves_counts_unlinked_rows_as_their_own`, `trim_to_saves_keeps_the_rows_up_to_k_saves`, `a_save_with_more_pieces_than_the_pool_leaves_room_for_other_memories` (`search.rs`). Antes del cambio, el último devolvía 1 hit en lugar de 10 | ✓ |
| R4. Set `long` fijo | `long_rows_do_not_depend_on_the_piece_cap` (`synth/mod.rs`); además, `golden-mine synth --seed 7` sobre la base sustituta da un archivo idéntico con el binario de `main` y con el de la punta | ✓ |
| R5. Medición | — | parcial: set sustituto abajo; el real queda pendiente |

## Set sustituto (Linux, modelo real)

El `golden.json` de Marcos no está en el repo. La base sustituta (20 filas, 4 casos) y el programa de diagnóstico están en la carpeta del proyecto, `long-save-ranking/`; el set es `golden-mine synth --seed 7` sobre ella (1.012 filas, 82 casos, 12 `long`). `rag-eval --real --agent-view --golden <set>`, con el binario de `main` y con el de la punta:

| Subconjunto | Métrica | `main` | Punta |
|---|---|---|---|
| `long` | `search.hit_rate` / `context.hit_rate` | 0,75 / 0,75 | 1,0 / 1,0 |
| `long` | `answers.search_reachable` | 0,75 | 1,0 |
| `long` | `answers.search_visible` | 0,00 | 0,917 |
| `long` | `answers.context_visible` | 0,583 | 0,917 |
| `long` | `repeated_rows` | 0 | 0 |
| los otros cinco | hit rates | 1,0 | 1,0 |
| todos (n=82) | tokens medios de `search_memory` / `recall_context` | 366,0 / 4.177,5 | 359,3 / 4.051,3 |
| — | filas guardadas | 1.035 | 1.175 |

Con los umbrales de la propuesta, sobre este set: U2 ✓ (1,0 ≥ 0,917; 0,917 ≥ 0,5; 0,917 ≥ 0,75) y U3 ✓ (nada baja). U0 y U1 no se pueden correr sin el set real.

## Pendiente (máquina de Marcos)

1. **Suite en Windows:** `cargo test --workspace --target-dir <dir>`. Espero 1.870 (la base de 1.864 más 6) y 0 fallas.
2. **U0, la base:** con el binario de `main` (`9aa1012`), `golden-mine synth --base docs/eval/golden.json --seed 7 --out <set>` y `rag-eval --real --agent-view --golden <set> --tag long-save-ranking-base-seed7`. Es la medición nueva de U2 de long-save-context con pooling mean.
3. **U1, el ancla:** con el binario de la punta, `rag-eval --real --tag long-save-ranking-tip-anchor` y `--diff` contra `2026-10-15-real-project-indexing-tip-anchor.json`. El ancla no tiene filas que se partan: se espera un diff vacío.
4. **U2 y U3:** con el binario de la punta y el mismo set del paso 2, `rag-eval --real --agent-view --golden <set> --tag long-save-ranking-tip-seed7`, y comparar `by_subset` contra la base a mano (`--diff` no mira agent-view).
5. **Perfil dev** (opcional, con autorización): guardar un texto de unos 3.000 caracteres que nombre su tema sólo en la primera oración, buscar por un hecho del final y leerlo con `read_memory`. No hay migración: el perfil abre sin snapshot nuevo.
