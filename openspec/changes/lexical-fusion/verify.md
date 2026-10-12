# lexical-fusion — Verificación

**Código:** rama `claude/lexical-fusion-sdd-vapx8s` sobre `main` (`197186d`): `a43862a` y `6b2ead6`.
**Modo:** Strict TDD (`openspec/config.yaml`: `strict_tdd: true`).
**Estado:** verificado con una medición pendiente. R1 a R6 tienen tests en verde. La medición `--real` con el golden set del usuario (la que decide si el bono queda como default) **está pendiente**: este entorno no tiene `docs/eval/`. Se midió `--real` sobre un set sustituto, ver abajo.

## Suite

`cargo test --workspace --no-fail-fast` en Linux:

| Commit | Pasan | Fallan |
|---|---|---|
| `197186d` (base, `main`) | 1551 | 10 |
| `a43862a` feat(core): let an exact lexical match add a bonus to the score | 1554 | 10 |
| `6b2ead6` feat(core): rank rows that carry the query's exact identifier first | 1565 | 10 |

- +14 tests: 3 en `score.rs`, 11 en `search.rs`. Ninguna falla nueva.
- Las 10 fallas son las mismas en los tres commits y vienen del entorno, no del código:
  - 7 de `doctor::tests` (`crates/clawg-cli/src/doctor.rs`): modelan un PATH de Windows con `clawg.exe`.
  - 3 de `diff::tests` en `rag-eval`: leen `docs/eval/results/`, que está fuera de git.
- El 1566 de `CLAUDE.md` es de Windows; en Linux la base da 1561 tests en total.
- **Warnings:** los mismos de la base. Ninguno nuevo en los archivos tocados.
- Cada commit compila y pasa el suite por separado (tabla de arriba).

## Strict TDD

| Chequeo | Resultado |
|---|---|
| RED de la tarea 1 | Los tests de `score.rs` no compilaban: no existían `RawHit.lexical_match` ni `ScoringConfig.lexical_bonus` |
| RED de la tarea 2 | Con `identifier_matches` devolviendo siempre vacío, fallan `an_exact_identifier_outranks_its_closer_siblings`, `an_identifier_past_the_sixteenth_token_still_reaches_its_row`, `an_identifier_shared_by_more_rows_than_the_budget_gives_no_bonus` y `rows_the_caller_cannot_see_neither_match_nor_count_against_the_budget` |
| Guardas | `no_bonus_without_the_lexical_channel_or_query_text`, `a_query_without_identifiers_scores_exactly_as_without_the_bonus` y `a_failing_identifier_query_leaves_the_search_unboosted` pasan antes y después: fijan que el bono no aparece donde no debe. Fallarían con un bono incondicional |
| Triangulación | El gate del presupuesto se prueba con 20 filas (hay bono) y 21 (no hay). El scope, con filas de otro agente y una `superseded` que harían pasar el presupuesto si contaran |

## Matriz de cumplimiento

| Requisito | Escenario | Test | Resultado |
|---|---|---|---|
| R1 Tokens identificadores | tipos de identificador | `identifier_tokens_keep_mixed_snake_and_camel_tokens_in_order`, `identifier_tokens_skip_bare_numbers_and_plain_words` | Cumple |
| R1 | identificador al final | `identifier_tokens_read_the_whole_query` | Cumple |
| R1 | sin identificadores; tope de 8 | `identifier_tokens_skip_bare_numbers_and_plain_words`, `identifier_tokens_are_distinct_and_capped` | Cumple |
| R2 Bono en el score | después del piso | `a_lexical_match_adds_the_bonus_after_the_floor`, `a_match_outranks_a_closer_row_with_the_same_decay_and_importance` | Cumple |
| R2 | sin match, sin cambio | `without_a_match_the_score_is_unchanged`; literal fijado en `scoring_config_default_pins_the_shipped_ranking_literals` | Cumple |
| R3 Búsqueda | le gana a los hermanos | `an_exact_identifier_outranks_its_closer_siblings` (con control sin bono) | Cumple |
| R3 | fuera del top denso | `an_identifier_past_the_sixteenth_token_still_reaches_its_row` | Cumple |
| R3 | canal apagado | `no_bonus_without_the_lexical_channel_or_query_text` | Cumple |
| R4 Gate | identificador repetido | `an_identifier_shared_by_more_rows_than_the_budget_gives_no_bonus` | Cumple |
| R4 | consulta natural | `a_query_without_identifiers_scores_exactly_as_without_the_bonus` | Cumple |
| R5 Scope | fila de otro agente | `rows_the_caller_cannot_see_neither_match_nor_count_against_the_budget` | Cumple |
| R6 Fail-soft | FTS ausente | `a_failing_identifier_query_leaves_the_search_unboosted` | Cumple |

**12 de 12 escenarios con test en verde.**

## Coherencia con el diseño

| Decisión | ¿Se siguió? | Nota |
|---|---|---|
| D1 bono aditivo `cos' + 1` | Sí | `score.rs`, `relevance` antes del producto |
| D2 qué es identificador | Sí | `is_identifier` en `search.rs` |
| D3 gate por presupuesto | Sí | `LIMIT presupuesto + 1`; si vuelven más, conjunto vacío |
| D4 `AND` | Sí | `match_expr` une con `AND` |
| D5 antes del `try_join!` | Sí | `knn_one` marca al mapear, antes del recorte a k |
| D6 matches como candidatos | Sí | Se suman a `lex_ids`, ordenados por id para que el orden no dependa del `HashSet` |
| D7 medición | Parcial | Ver abajo |

## Medición `rag-eval --real`

### Set sustituto (hecha en este entorno)

No es la medición de la propuesta: el golden set real está en la máquina del usuario. Sirve para ver el efecto con el modelo real (multilingual-e5-small) antes de esa corrida.

- Base: las 4 filas y 2 casos inline de `crates/clawg-eval/tests/synth_roundtrip.rs` (sha256 `f74ea12e…fae898`).
- `golden-mine synth --seed 7`, 500 distractores: 984 filas, 68 casos (sha256 `eaa0aaa5…3b84a2`).
- El modelo se descargó a un `HOME` temporal; no se tocó ningún `~/.clawg*`.
- Binarios debug de `197186d` (base) y `6b2ead6` (cambio).

| Subconjunto | n | R@1 | R@5 | R@10 | MRR | nDCG@10 |
|---|---|---|---|---|---|---|
| lexical (base) | 30 | 0,3667 | 0,7667 | 0,9667 | 0,5315 | 0,6342 |
| **lexical (cambio)** | 30 | **1,0000** | **1,0000** | **1,0000** | **1,0000** | **1,0000** |
| multi_ref | 12 | 0,2639 | 0,7500 | 0,9722 | 0,7361 | 0,7551 |
| high_importance | 12 | 0,5833 | 0,8333 | 1,0000 | 0,7028 | 0,7730 |
| low_importance | 12 | 0,1667 | 0,6667 | 1,0000 | 0,3758 | 0,5224 |
| natural | 2 | 0,0000 | 0,5000 | 0,5000 | 0,1667 | 0,2500 |

`multi_ref`, `high_importance`, `low_importance` y `natural` dan lo mismo con y sin el cambio.

- `rag-eval --diff` base contra cambio: R@1 19 mejoran, R@5 7, R@10 1, MRR 19, nDCG@10 19; **0 empeoran** en todas.
- Léxico 20 frente a 0: **19 de 68** casos cambian de ranking con el cambio (todos `lexical`); en la base, 0 de 68.
- `importance_rank_dominance`: 0,0077 en la base, 0,0064 con el cambio.
- El audit da `unusable` en las dos corridas: con 2 casos naturales el set sustituto no pasa el chequeo de saturación. No cambia la lectura del diff pareado.
- Dos corridas del cambio dan las mismas cifras.

### Pendiente (máquina del usuario)

Con los binarios de este cambio, contra los resultados guardados de eval-golden-expansion:

- [ ] `rag-eval --real --audit --golden <seed7.json> --sweep-lexical 20 --sweep-lexical 0` y `--diff` contra el baseline de la semilla 7 (sha256 `6312586c…d228c`). Umbrales: `lexical` R@5 >= 0,90 y MRR >= 0,80; 0 casos `natural` que empeoran en R@5; `multi_ref`, `high_importance` y `low_importance` sin bajar R@5; `importance_rank_dominance` sin subir de +0,0256; léxico 20 frente a 0 con más de 0 casos distintos.
- [ ] `rag-eval --real --diff` sobre `docs/eval/golden.json` (ancla): ninguna de R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979 ni nDCG@10 0,6693 baja.
- [ ] `rag-eval --real --agent-view --diff`: hit rate natural sin bajar, tokens de `search_memory` sin crecer.

Si un umbral falla, se anota tal cual y el bono no queda como default.
