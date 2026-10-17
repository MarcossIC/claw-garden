# memory-rank-fusion — Verificación

## Linux

En cada commit de las tareas 1 a 6, `cargo test --workspace` pasa y `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` sale con 0. `agent_contract.json` no cambia: el texto de `search_memory` es el mismo.

Lo que fallaba antes del cambio y pasa después:

- `search_shows_the_rows_an_identifier_match_used_to_hide` (`crates/clawg-cli/tests/api.rs`): con el corte de hoy aplicado a los mismos hits, el daemon devuelve la fila del identificador (score 1,18) y una relacionada (0,39), y corta las otras dos (0,35 × 1,18 = 0,41 contra 0,36 y 0,33). Con el corte nuevo vuelven las cuatro.
- En puro, `exact_hits_do_not_set_the_bar_for_the_others`: el exacto en 1,6 y 0,30 / 0,25 / 0,20 / 0,05 dejan 4 hits, contra 2.

`golden-mine synth --seed 7` sobre una base con `query_lang` (que antes rechazaba) agrega los 30 casos `lexical_related`; sin ellos, el set es byte a byte el de antes (sha256 fijado en `the_other_subsets_do_not_change_when_lexical_related_is_added`).

## CI

El CI (`test (ubuntu-latest)` y `test (windows-latest)`, clippy y la suite) pasa sobre la punta de la rama, `bb8f60d`.

## Medición con el modelo real

**Veredicto:** U2 a U6 cumplen; **U1 no** (0,70 contra 0,90). El corte nuevo ya no esconde ninguna fila en `lexical_related`: lo que falta para 0,90 lo decide el ranking con el límite de 10 hits.

### Cómo se midió

- **Qué necesita:** el modelo en el caché del perfil `dev` (`rag-eval --real --check-only` sale con 0 en los dos commits), el golden set fuera de git (`docs/eval/golden.json` y `docs/eval/golden-expanded.json`) y un build debug de `rag-eval` y `golden-mine`. `rag-eval` siembra un store temporal y no abre ningún store real.
- **Commits:** la base es `e6184c9` (`feat(eval): synthesize cases that expect an identifier row and its related rows`), en un worktree descartable fuera del repositorio; el después, `31420de` de `main`. Entre los dos sólo cambian `crates/clawg-cli` (el corte, `1b34231`) y documentación: `clawg-eval` y `Cargo.lock` son los mismos. Cada uno se compiló en `target-test` después de `cargo clean --workspace`.
- **Comandos:** `rag-eval --real --agent-view --golden docs/eval/golden-expanded.json --tag <prefijo>-expanded`, `rag-eval --real --tag <prefijo>` y `rag-eval --real --agent-view --tag <prefijo>`, con los prefijos `rank-fusion-base` y `rank-fusion-tip`; `rag-eval --diff <base> <después>` para U5 y U6. El `git_sha` y el `golden_set.content_sha256` de cada archivo confirman su commit y su golden.

| Archivo (`docs/eval/results/`) | Commit | Golden |
|---|---|---|
| `2026-10-17-real-rank-fusion-base-expanded-agent-view.json` | `e6184c9` | `golden-expanded.json` |
| `2026-10-17-real-rank-fusion-base.json`, `-base-agent-view.json` | `e6184c9` | `golden.json` |
| `2026-10-17-real-rank-fusion-tip-expanded-agent-view.json` | `31420de` | `golden-expanded.json` |
| `2026-10-17-real-rank-fusion-tip.json`, `-tip-agent-view.json` | `31420de` | `golden.json` |
| `2026-10-17-real-rank-fusion-tip-expanded.json` | `31420de` | `golden-expanded.json`, sin `--agent-view`: sólo para leer el ranking de U1, fuera de los umbrales |

### El golden

- `golden.json`: `363d3e08854bfa222ccf9944508938e0e58a8dab9808696eca7a10525da0a1fb`, el de english-recall, sin cambios.
- El `golden-expanded.json` anterior queda como `docs/eval/golden-expanded.pre-rank-fusion.json` (`6312586c…`, 1.028 filas, 112 casos, sin `long`).
- `golden-mine synth --base docs/eval/golden.json --out docs/eval/golden-expanded.json --seed 7` (500 distractores, el valor por defecto) escribe `855c64b98e5403e55777c87873bcf006299502b857dcfbb7920acb48d9d6f2b5`: 1.280 filas (48 de la base), 154 casos y 28 pares de contradicción. Valida con `rag-eval --fake --golden <archivo> --out <dir temporal>` (salida 0).

| `natural` | `lexical` | `multi_ref` | `high_importance` | `low_importance` | `long` | `lexical_related` |
|---|---|---|---|---|---|---|
| 46 | 30 | 12 | 12 | 12 | 12 | 30 (16 `en`, 14 `es`) |

Contra el archivo anterior cambian también los distractores: `synth` agrupa las filas de la base por tipo e idioma, y english-recall pasó el `lang` de c01, c08, c09 y c35 a `en`. Base y después corren sobre el mismo archivo nuevo, así que la comparación no cambia.

### La base reproduce la propuesta

Ancla en `e6184c9`: R@1 0,4348, R@5 0,6522, R@10 0,8478, MRR 0,5524, nDCG@10 0,6215; agent-view `search_memory` 36/46 a 764,0 tokens y `recall_context` 39/46 a 1.086,4. Contra `2026-10-17-real-english-recall-base.json` y `-base-agent-view.json`, `--diff` da 0 casos movidos en todas las métricas (p = 1,0000).

### Umbrales

| # | Métrica | Base | Después | Umbral | Veredicto |
|---|---|---|---|---|---|
| U1 | `lexical_related`: `search.all_hit_rate` | 0,0000 (0/30) | 0,7000 (21/30) | `>= 0,90` | **No cumple** |
| U2 | `lexical`: la fila con el identificador primera (`rank = 1`) | 30/30 | 30/30 | 30/30 | Cumple |
| U3 | `lexical`: tokens medios de `search_memory` | 151,9 (`natural` 899,3) | 478,1 (`natural` 899,3) | no más que `natural` | Cumple |
| U4 | `natural`, `multi_ref`, `high_importance`, `low_importance`, `long`: salida de `search_memory` | — | idéntica en 46/46, 12/12, 12/12, 12/12 y 12/12 | idéntica caso por caso | Cumple |
| U5 | Ancla, `--diff` | R@1 0,4348, R@5 0,6522, R@10 0,8478, MRR 0,5524, nDCG@10 0,6215 | las mismas | ningún caso se mueve | Cumple |
| U6 | Agent-view del ancla, `--diff` | 36/46 a 764,0; 39/46 a 1.086,4 | las mismas | ningún caso se mueve | Cumple |

- **U4.** Apareados por consulta, se compara todo el registro `search` de cada caso salvo la latencia: `found`, `found_refs`, `rank`, `tokens`, `truncated`, `repeated_rows` y cada línea impresa (`score`, `tokens`, `expected`). El archivo no guarda el texto, así que eso es lo que se compara. `context` también es idéntico en los 154 casos, como corresponde: `recall_context` no tiene corte.
- **U5.** En `retrieval` y en `similarity`, recall@1, @5 y @10: 0 / 0, p = 1,0000; MRR y nDCG@10: 0 / 0, p = 1,0000, cambio medio +0,0000 [+0,0000, +0,0000]. Lo mismo en cada pareja (`en>en` 7, `es>en` 19, `es>es` 20). Las 46 listas rankeadas son idénticas en los dos bloques, y `--fail-on-regression` sale con 0.
- **U6.** `search.found` y `context.found`: 0 / 0, p = 1,0000; tokens de las dos herramientas: 0 / 0, cambio medio +0,0000 [+0,0000, +0,0000]; lo mismo en cada pareja. Los 46 registros `search` y `context` (sin latencia) son idénticos, y `--fail-on-regression` sale con 0.
- **U3.** `lexical_related` sube de 105,1 a 494,4 tokens. Los dos subconjuntos con identificador muestran 10 líneas en los 30 casos, contra 2 en 24 casos de `lexical` y en 29 de `lexical_related` en la base.

### U1

En la base, `lexical_related` muestra 2 líneas en 29 casos y 3 en uno: la fila del identificador y la mejor sin bono, que está por `MIN_SHOWN_HITS`. Después muestra 10 en los 30, el `limit` por defecto del pedido (`DEFAULT_SEARCH_LIMIT`), así que el corte ya no quita ninguna fila de este subconjunto. Filas esperadas a la vista por caso: en la base, 1 en 6 casos, 2 en 23 y 3 en 1; después, 4 en 21, 3 en 6, 2 en 2 y 1 en 1.

Los 9 casos que fallan pierden filas relacionadas por el ranking, no por el corte. En los 30 casos, las esperadas entre los 10 primeros del store (`retrieval` de `2026-10-17-real-rank-fusion-tip-expanded.json`) son exactamente las que muestra agent-view, y las 13 que faltan están más abajo. En esos 9 casos, los 67 lugares que no ocupa una esperada son de:

- 26 filas de `lexical` (23 con el mismo tipo de identificador y el mismo idioma que la consulta), las hermanas que se leen como la pregunta;
- 17 hermanas de la propia familia, con identificadores vecinos;
- 14 filas del identificador o relacionadas de otras familias de `lexical_related` (9 de familias con la misma plantilla de tema, que sólo cambia el nombre clave y el identificador);
- 10 hermanas de otras familias de `lexical_related`.

El umbral se anota tal cual: no se mueve.

### El riesgo: el coseno de las filas relacionadas

Agent-view guarda el score de cada línea con dos decimales. Una fila recién sembrada, de importancia 5 y sin bono, puntúa `c' × 0,9167` (decaimiento 1 y `0,8333 + 5/60`), con `c' = (cos − 0,79) / 0,21`; con bono, `(c' + 1) × 0,9167`. De ahí sale el coseno con ±0,0012.

- **Base:** la segunda línea es una relacionada en 24 de 30 casos (25 líneas, contando el caso de 3). Su coseno va de 0,852 a 0,921, con mediana 0,891; 17 de 25 llegan a 0,88 y 23 a 0,8635.
- **Después:** las 77 relacionadas a la vista van de 0,838 a 0,921, con mediana 0,879; 34 llegan a 0,88 y 59 a 0,8635. Las 13 que no se ven están por debajo de la décima línea de su caso.
- La fila del identificador va de 0,894 a 0,940 (mediana 0,917). La mejor línea sin bono de cada caso tiene el mismo score en las dos corridas: el corte no cambia los vectores.

La fila del identificador puntúa de 1,37 a 1,57 (mediana 1,47), así que el corte de antes le pedía a una fila sin bono un score de 0,48 a 0,55: un coseno de 0,900 a 0,916 (0,908 en la mediana). **Veredicto: las relacionadas llegan a un coseno cercano a 0,88, así que U1 mide la tesis.** Con el corte de antes, ningún caso las muestra todas; con el nuevo, 21 de 30.

### Diferencias con la propuesta

- `2026-10-17-real-rank-fusion-tip-expanded.json` es una corrida que la propuesta no pedía: lee el ranking de U1 y no entra en ningún umbral.
- En la base, `lexical` muestra 2 líneas en 24 de 30 casos y entre 4 y 10 en los otros 6. La exploración contaba 2 en los 30, pero medía antes del pooling mean, con piso 0,8793.
- El worktree de la base se armó con una ruta corta, y su build fue al `target-test` del repositorio. `git worktree remove --force` lo borró entero; al terminar, `git worktree list` sólo muestra `main` y no queda otro directorio de build que `target` y `target-test`.

Archivado el 2026-10-17 con U1 anotado como no cumplido (0,70 contra 0,90). El ranking de las filas relacionadas por debajo del límite de 10 hits queda para un cambio aparte.
