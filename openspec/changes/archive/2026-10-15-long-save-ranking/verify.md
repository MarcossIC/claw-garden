# long-save-ranking — Verificación

**Código:** rama `claude/project-thread-z8r7xv`, un commit por tarea, sobre `main` en `9aa1012`.
**Modo:** Strict TDD (`openspec/config.yaml`). Los tests de R1 y R3 fallaron antes de su código (por la aserción o por no compilar). Los de R2 y R4 fijan un contrato que hoy se cumple: el de R4 falla si se baja el tope sin su commit (comprobado con el tope en 96 antes del cambio del generador); el de R2 pasa sin código nuevo, como se esperaba.
**Estado:** R1 a R4 pasan en Linux, commit por commit, y en Windows sobre `main` en `04ab3cd` (1.900 pasan, 0 fallan). Sobre el set real, U0 a U3 se cumplen: en `long`, la respuesta alcanzable sube de 10 a 12 de 12, la visible en `search_memory` de 0 a 11 y la visible en `recall_context` de 6 a 11. La prueba con el perfil dev pasa. Detalle en [Windows](#windows).

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
| R4. Set `long` fijo | `long_rows_do_not_depend_on_the_piece_cap` (`synth/mod.rs`); además, `golden-mine synth --seed 7` da un archivo idéntico con el binario de `main` y con el de la punta, sobre la base sustituta y sobre `golden.json` | ✓ |
| R5. Medición | — | ✓ U0 a U3 sobre el set real ([Windows](#windows)) |

## Set sustituto (Linux, modelo real)

El golden set local (`docs/eval/golden.json`) no está en el repo. La base sustituta (20 filas, 4 casos) y el programa de diagnóstico están en la carpeta del proyecto, `long-save-ranking/`; el set es `golden-mine synth --seed 7` sobre ella (1.012 filas, 82 casos, 12 `long`). `rag-eval --real --agent-view --golden <set>`, con el binario de `main` y con el de la punta:

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

## Windows

Todo sobre `main` en `04ab3cd` (el merge de la rama), Windows 11, binarios debug. `04ab3cd` trae además recoverable-delete, que entró a `main` en el mismo tramo. Sus propias corridas (`9aa1012` contra `1bc70b5`, sobre `golden.json`) dan un `--diff` vacío y 0 diferencias en el agent-view, caso por caso (`found`, `found_refs`, `rank`, `tokens`). Lo que cambia abajo es de este change.

### Suite

`cargo test --workspace --no-fail-fast --target-dir target-test`: **1.900 pasan, 0 fallan.** Es la base de Windows (1.864) más los 6 tests de este change y los 30 de recoverable-delete. Los 10 que fallan en Linux por el entorno pasan acá. Warnings: sólo los de `main`.

### El set

`golden-mine synth --base docs/eval/golden.json --seed 7` (`golden.json` en `d2d9d732…ed792a`): 1.040 filas y 124 casos (46 `natural`, 30 `lexical` y 12 de cada uno de `multi_ref`, `high_importance`, `low_importance` y `long`). El binario de `9aa1012` y el de `04ab3cd` generan el mismo archivo, byte a byte (`98ea356e…1dda05e80`): R4 se cumple también sobre el set real.

La base se compiló en un worktree de `9aa1012` y corrió desde ahí, con `--out` hacia `docs/eval/results` del repo, así que su `git_sha` dice `9aa1012`. La punta es el binario de `target-test`. Las dos corridas: `rag-eval --real --agent-view --golden <set>` con `--tag long-save-ranking-base-seed7` y `--tag long-save-ranking-tip-seed7`.

### U0, la base

`long` en `9aa1012`: hit rate 0,833 en búsqueda y en contexto, `search_reachable` 0,833, `search_visible` 0,000 y `context_visible` 0,500. Es la medición nueva de U2 de long-save-context: con el pooling mean y el corte de relevancia, la respuesta alcanzable ya había subido de 0,333 a 0,833 antes de este change. Lo que seguía en 0 era verla. **✓** (registrada antes de medir la punta)

### U1, el ancla

`rag-eval --real --tag long-save-ranking-tip-anchor`, y `--diff` contra `2026-10-15-real-project-indexing-tip-anchor.json` (`fc0606d`) y contra `2026-10-15-real-recoverable-delete-base.json` (`9aa1012`): 0 mejores y 0 peores en las cinco métricas de retrieval y de similarity, en los dos. El ancla queda en R@1 0,4348, R@5 0,6522, R@10 0,8478, MRR 0,5524 y nDCG@10 0,6215.

`rag-eval --real --agent-view --tag long-save-ranking-tip` sobre el mismo ancla, comparado caso por caso (`found`, `found_refs`, `rank`, `tokens`) contra `2026-10-15-real-project-indexing-relcut-agent-view.json` (`fc0606d`) y contra `2026-10-15-real-recoverable-delete-tip-agent-view.json` (`1bc70b5`): 0 diferencias. `search_memory` queda en 36 de 46 con media de 764,0 tokens (máx. 1.206) y `recall_context` en 39 de 46 con 1.086,4 (máx. 1.408). Ninguna fila de `golden.json` pasa de un pedazo, así que ni el tope nuevo ni el pozo por saves tienen nada que cambiar ahí. **✓**

### U2 y U3

| Subconjunto | Métrica | Base | Punta | Δ |
|---|---|---|---|---|
| `long` | `search.hit_rate` / `context.hit_rate` | 0,833 / 0,833 | 1,000 / 1,000 | +0,167 |
| `long` | `answers.search_reachable` | 0,833 | 1,000 | +0,167 |
| `long` | `answers.search_visible` | 0,000 | 0,917 | +0,917 |
| `long` | `answers.context_visible` | 0,500 | 0,917 | +0,417 |
| `long` | `search.mean_tokens` / `context.mean_tokens` | 585,7 / 3.967,8 | 570,9 / 3.059,4 | −14,8 / −908,4 |
| `natural` | `search.hit_rate` / `context.hit_rate` | 0,4783 / 0,4783 | 0,4783 / 0,4783 | 0 |
| `natural` | `search.mean_tokens` / `context.mean_tokens` | 861,5 / 3.446,3 | 854,7 / 3.252,5 | −6,8 / −193,9 |
| `lexical`, `multi_ref`, `high_importance`, `low_importance` | `search.hit_rate` / `context.hit_rate` | 1,0 | 1,0 | 0 |
| los seis | `repeated_rows` | 0 | 0 | 0 |
| todos (n=124) | hallados en búsqueda / en contexto | 98 / 98 | 100 / 100 | +2 |
| todos (n=124) | tokens medios (máx.) de `search_memory` | 541,5 (1.360) | 543,9 (1.360) | +2,4 |
| todos (n=124) | tokens medios (máx.) de `recall_context` | 3.114,8 (5.974) | 2.944,7 (3.754) | −170,1 |
| todos (n=124) | latencia p50 / p99 de `search_memory` (ms) | 19,8 / 28,4 | 23,8 / 31,1 | +4,0 / +2,7 |
| — | filas guardadas | 1.064 | 1.205 | +141 |

En los cuatro subconjuntos con hit rate 1,0, los tokens de búsqueda se mueven poco: `multi_ref` sube 53,8 (468,7 → 522,4), `high_importance` sube 12,8, y los demás quedan iguales. Ninguna condición pone un umbral sobre tokens ni sobre latencia; se registran.

| # | Condición | Resultado |
|---|---|---|
| U2 | `long`: `search_reachable` ≥ 0,917, `search_visible` ≥ 0,5 y `context_visible` ≥ 0,75 | ✓ 1,000, 0,917 y 0,917 |
| U3 | los otros cinco: ninguna métrica de agent-view baja más de 0,02; `repeated_rows` = 0 | ✓ ningún hit rate cambia; 0 filas repetidas en los seis |

**El caso que queda.** `s-long-06` ("how long do cached entries of cobalt-wren live") sale en el puesto 1 de `search_memory`, pero con un pedazo que no es el del hecho. Tampoco se ve en `recall_context`. Se puede alcanzar: `read_memory` con su id devuelve el save entero.

### Perfil dev

Con autorización para tocar `~/.clawg-dev`, sobre el daemon dev que ya corría: PID 19972, `target\debug\clawg.exe` compilado a las 08:21, después del merge `04ab3cd`. No hubo que pararlo ni recompilar. El último snapshot en `backups` es de la noche anterior (`…T01-38-30.827Z-pre-reembed.db`), así que el daemon abrió sin migración y sin snapshot nuevo, como se esperaba.

El bridge `clawg-dev` de la sesión respondía `the daemon rejected the token` a todo lo que lleva token, mientras `memory_status` daba `ok`. Arrancó a las 08:25:17, un segundo antes que el daemon que él mismo autoarrancó, y el `token` del disco es el de ese daemon (08:25:18). Es el problema de bridge-reconnect (fila 39 del [README](../../../README.md)), pero sin ningún reinicio en el medio. Lo más probable es una carrera en el autostart: el sondeo de 500 ms lee el `daemon.json` y el `token` que dejó el daemon anterior entre el `bind` del nuevo y su `write_token` (`crates/clawg-cli/src/daemon/serve.rs:107-118`). Como el puerto es el mismo (27381), el `/health` sin token responde igual. No se reprodujo a propósito. Para la prueba se usó un bridge nuevo, `clawg mcp --profile dev --agent claude-code --no-autostart` del binario de `target-test`, hablándole JSON-RPC por stdio.

1. `save_memory` de un texto de 3.102 caracteres que nombra `quartz-falcon` sólo en la primera oración, con el hecho (el presupuesto de reintentos) en el anteúltimo párrafo: `saved as 11 pieces: #44 … #54`. Con el tope anterior habrían sido 2.
2. `search_memory` con "what retry budget did we settle on for quartz-falcon": un solo hit para el save, `#53 [… piece=10/11 score=0.29]`, y el texto mostrado incluye la respuesta ("seven attempts per chunk…").
3. `read_memory` con `id: 53`: `#53 is piece 10 of 11 of one save: #44, …, #54`, y el texto unido es idéntico al guardado, sin solapamiento repetido.
4. `delete_memory` de `#44` a `#54`; la misma búsqueda ya no los encuentra.

**✓**
