# lexical-fusion — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 3 | M | eval-golden-expansion (archivado el 2026-10-12) | apply |

## Problema

Un match exacto por identificador no cambia el ranking. El canal léxico sólo propone candidatos; después todos se puntúan con el coseno.

- BM25 decide **cuáles** filas entran (`crates/clawg-core/src/memory/search.rs:550-557`, `ORDER BY bm25(memory_fts) LIMIT`) y después se descarta: el candidato léxico se puntúa con el mismo `compute_score` que el denso (`search.rs:711-721`, `:822-834`).
- Bajo el piso de coseno 0,8793 el score es 0 (`crates/clawg-core/src/memory/score.rs:172`, `:201-203`, `:256-260`). Entre ceros, `hit_cmp` desempata por `(kind, id ASC)` (`search.rs:78-84`): un candidato léxico bajo el piso sólo llega arriba por ser más viejo.
- La consulta léxica toma los primeros 16 tokens (`MAX_LEXICAL_TOKENS`, `search.rs:482`, `:503`). Un identificador al final de una consulta larga no llega al MATCH.

**Medido** (baseline de eval-golden-expansion, set de la semilla 7, [`specs/eval`](../../specs/eval/spec.md#set-de-la-semilla-7)):

- subconjunto `lexical`: R@5 **0,4333**, MRR 0,3825, con R@10 1,0. La fila esperada siempre está en el top 10, pero sus 7 hermanos (mismo marco que la consulta, identificador casi igual) le ganan por coseno.
- con el canal léxico en 20 frente a 0, **0 de 112** casos cambian de ranking. El canal hoy no influye en nada.

## Decisiones (cierran las abiertas de la exploración)

**Enfoque: b, generalizado a un bono aditivo por identificador exacto.** Se descartan a (RRF) y c (BM25 normalizado). El detalle y las alternativas están en `design.md`.

- Un **token identificador** es un token de la consulta que mezcla letras y dígitos (`getUser4821`), lleva `_` (`E_AUTH_4821`, `snake_case`) o tiene una minúscula seguida de una mayúscula (`getEstadoLabel`). Se toman de **toda** la consulta, no sólo de los primeros 16 tokens.
- Una fila es un **match de identificador** si su contenido tiene todos los tokens identificadores de la consulta (FTS5, `AND`), y el conjunto de filas que matchean no pasa del presupuesto léxico (20). Si lo pasa, el identificador no discrimina y no hay bono para nadie.
- El score de un match pasa de `cos' × decaimiento × importancia` a `(cos' + 1) × decaimiento × importancia`. Un identificador exacto vale lo mismo que un coseno perfecto. Una consulta sin tokens identificadores puntúa exactamente igual que hoy, bit a bit.
- Los matches entran también como candidatos léxicos, así que un identificador fuera del top 30 denso igual llega a puntuarse.

**¿Se mueve el piso de coseno?** No. El bono se suma después del re-escalado, así que no depende del piso; 0,8793 sigue atado a los prefijos e5.

**¿FTS para otras tablas?** No en este cambio. `episodic` y `procedural` siguen sin canal léxico; depende de knowledge-synthesis.

**Empates en 0 (venía de retrieval-hygiene).** Se resuelven por la fusión: el caso que justificaba el desempate por id viejo (un match léxico bajo el piso) ahora puntúa > 0. El resto de los ceros no tiene ninguna señal que los ordene, y su orden queda como hoy.

## Alcance

1. `identifier_tokens(query)` puro en `search.rs`.
2. Consulta FTS5 de matches de identificador, con los mismos predicados de scope que el candidato léxico de hoy, y la regla de que más matches que el presupuesto no dan bono.
3. `ScoringConfig.lexical_bonus` (1,0) y un campo de match en `RawHit`; `score()` suma el bono a `cos'`.
4. La búsqueda marca los matches en el canal denso y en el léxico, y los suma a los candidatos léxicos.

## Fuera de alcance

- Partir camelCase o snake_case en el tokenizer (enfoque d). `getEstadoLabel` sigue siendo un token.
- Números solos (`4821`, `2026`) como identificadores: son demasiado comunes para valer un bono fijo.
- Un bono parcial cuando la fila tiene sólo algunos de los identificadores de la consulta.
- Mover el piso de coseno, los pesos de importancia o el decaimiento.
- Un eje de sweep para el bono en `rag-eval`. La medición compara contra el baseline guardado; si no alcanza, se agrega al iterar.
- FTS para `episodic` y `procedural`.

## Éxito (umbrales comprometidos antes de medir)

`cargo test --workspace` sin fallas nuevas contra la base. Tests nuevos que fallan hoy:

- una fila con el identificador exacto de la consulta, de coseno más bajo que sus hermanas, sale primera;
- un identificador que matchea más filas que el presupuesto no cambia ningún score;
- una consulta sin tokens identificadores da los mismos scores, bit a bit, que con el bono en 0;
- un identificador más allá del token 16 sigue proponiendo y bonificando su fila.

`rag-eval --real --diff` contra el baseline de eval-golden-expansion (mismo `seed7.json`, sha256 `6312586c…d228c`):

| Métrica | Hoy | Umbral |
|---|---|---|
| `lexical` R@5 | 0,4333 | >= 0,90 |
| `lexical` MRR | 0,3825 | >= 0,80 |
| `natural`: casos que empeoran en R@5 | — | 0 |
| `multi_ref`, `high_importance`, `low_importance` R@5 | 0,7778 / 1,0 / 0,75 | sin bajar |
| `importance_rank_dominance` | +0,0256 | sin subir |
| léxico 20 frente a 0: casos con ranking distinto | 0 de 112 | > 0 |

`rag-eval --real --diff` sobre el ancla (`docs/eval/golden.json`, n = 46): R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979 y nDCG@10 0,6693 sin bajar ninguna.

`rag-eval --real --agent-view --diff`: hit rate natural sin bajar, tokens de `search_memory` sin crecer.

Si un umbral falla, se registra tal cual y no se mueve después de medir. El bono queda como default sólo si todos se cumplen.

## Datos guardados

- No cambia el esquema, ni identificadores, ni estados, ni el contrato de embeddings. No hay migración ni vuelta atrás que preparar: el cambio es sólo de lectura y se revierte con el código.
