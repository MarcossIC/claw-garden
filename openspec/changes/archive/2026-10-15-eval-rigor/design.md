# eval-rigor — Diseño

Las decisiones de la [propuesta](proposal.md), con las alternativas que se descartaron. Las alternativas más amplias (sólo ampliar el set, una dependencia de estadística, una prueba t, Bonferroni) están en la [exploración](exploration.md#descartado).

Todo lo nuevo vive en `crates/clawg-eval`, salvo dos funciones de `clawg_core::eval_api` (D6, D7), que sólo existe con la feature `eval` y nunca entra en un build de producción.

## D1. Las funciones estadísticas

**Elegido:** un módulo `crates/clawg-eval/src/bin/rag-eval/stats.rs` con funciones puras, sin dependencias nuevas:

| Función | Qué devuelve | Bordes |
|---|---|---|
| `wilson(k, n)` | El intervalo de Wilson al 95 % (`z = 1,959964`) de `k` aciertos en `n` | `n = 0` da `(0, 1)`: sin casos no se sabe nada, y no se divide por cero |
| `mcnemar_exact(b, c)` | El p de dos colas de la binomial exacta con `p = 0,5` sobre `b + c` casos que se movieron | `(0, 0)` da 1; nunca pasa de 1. Se suma en logaritmos, así que no se desborda con miles de casos |
| `paired_bootstrap_ci(deltas, resamples, seed)` | Media de las diferencias por caso y su intervalo percentil al 95 % | Sin diferencias da `(0, 0, 0)`; con la misma semilla, el mismo intervalo |
| `min_one_way_significant()` | El menor número de casos movidos en una sola dirección con p < 0,05: 6 | — |

`mcnemar_exact` se usa para **toda** métrica por caso, también para MRR, nDCG y tokens: cuenta cuántos casos mejoraron y cuántos empeoraron, sin mirar cuánto, que es la prueba del signo. En una métrica binaria (`found`, la abstención, la frescura) es exactamente McNemar. Las métricas continuas suman el intervalo bootstrap de la diferencia media, que dice cuánto se movió.

| Alternativa | Por qué no |
|---|---|
| McNemar sólo para las binarias y Wilcoxon para las continuas | Dos pruebas que leer; Wilcoxon pide rangos con empates y una aproximación normal con pocos casos. El bootstrap ya da el tamaño del efecto |
| Prueba con la aproximación normal o chi cuadrado | Con 1 a 10 casos discordantes la aproximación es mala, que es justo el rango donde importa |
| Bootstrap con sesgo corregido (BCa) | Más código para un intervalo que sólo se lee como «incluye 0 o no» |

## D2. La forma del diff

`MetricDiff` suma `ci: Option<Interval>` (media, límite inferior y superior de la diferencia), sólo en las continuas, y un método `p_value()` que llama a `mcnemar_exact(better, worse)`. `better`, `worse` y `net` no cambian de significado.

`RunDiff` suma tres bloques opcionales, cada uno `None` si a alguno de los dos archivos le falta:

- `agent_view`: `search_found`, `context_found` (binarias), `search_tokens`, `context_tokens` (continuas; **menos tokens cuenta como mejor**) y `search_abstained` sobre los casos `unanswerable`. Se une por `query` sobre `agent_view.per_case`, como hoy se une `retrieval`. Sólo entran los casos activos.
- `abstention`: `retrieval.unanswerable[].abstained`.
- `freshness`: `retrieval.per_case[].fresh_first` de los casos `update`.

`print_stdout` imprime por métrica `better`, `worse`, `net`, `p` y, si hay, el intervalo. Al final, una línea por métrica primaria (R@5 de `retrieval` y `search_found` de `agent_view`): «distinguible (p = …)» o «no distinguible con N casos pareados: hacen falta 6 en una dirección». Nunca «sin diferencia».

| Alternativa | Por qué no |
|---|---|
| Recalcular `better`/`worse` de R@k como acierto binario | Cambiaría lo que imprime hoy `--diff` para los casos con varias esperadas; el signo del cambio fraccional ya es la entrada correcta de la prueba |
| Un JSON del diff | El diff se deriva de dos archivos que ya están en disco (`diff.rs:157-161`); `--fail-on-regression` da la señal que una máquina necesita |

### `--fail-on-regression`

Sólo con `--diff`, en cualquier orden. Devuelve el código nuevo **6** cuando una métrica primaria presente tiene `worse > better` y p < 0,05; si no, 0, como hoy. Las informativas no lo disparan: es lo que evita que muchas comparaciones produzcan un falso positivo.

## D3. Generador del bootstrap

**Elegido:** copiar en `stats.rs` las dos funciones de SplitMix64 que el bootstrap usa (`new`, `next_u64`), con el test de los tres primeros valores de semilla 0 que ya tiene `golden-mine/synth/rng.rs:39-45`. La semilla es la constante `BOOTSTRAP_SEED`.

| Alternativa | Por qué no |
|---|---|
| `#[path]` al `rng.rs` de `golden-mine` | `rag-eval` no usaría `below` ni `pick`, que darían `dead_code`; y la convención prohíbe el `allow` |
| Mover el generador a una biblioteca del crate | `clawg-eval` no tiene biblioteca a propósito (`Cargo.toml:11-12`); crear una para doce líneas es la abstracción sin uso real que pide evitar `config.yaml` |
| `rand` | No garantiza la misma secuencia entre versiones, la misma razón por la que `golden-mine` lo evitó |

## D4. Partición por hash

`split_of(query)` toma los primeros 8 bytes del sha256 de la consulta (`clawg_core::eval_api::sha256_hex`, `crates/clawg-core/src/eval_api.rs:732`) como `u64`: módulo 10 menor que 7 es `calibration`, si no `test`. Sin semilla ni estado, y agregar casos no mueve a los que ya están.

El reporte suma `retrieval.by_split`, con el mismo `Aggregate` y los mismos aciertos que `by_subset`. No hay un flag de barrido nuevo: cada punto de un barrido ya escribe su archivo con `by_split`, y la regla, escrita en `specs/eval`, es elegir una constante nueva leyendo `calibration` y reportar `test`.

| Alternativa | Por qué no |
|---|---|
| Un flag que mide los barridos sólo sobre calibración | Esconde la mitad de los datos de cada punto, y el informe de prueba igual hay que leerlo aparte |
| Partición aleatoria con semilla | Depende de la semilla y del orden de los casos; agregar un caso puede cambiar de lado a otros |
| 50/50 | Con 46 casos deja 23 y 23, con intervalos más anchos que los de hoy ([exploración, B](exploration.md#b-partición-calibración-y-prueba)) |

## D5. Aciertos estrictos y parciales, e intervalo de Wilson

Un `Rate { hits, n_cases, rate, lo, hi }` y un `Hits { strict, partial }` por cada k de 1, 5 y 10. Estricto es `recall@k == 1` (todas las esperadas en el top-k); parcial, `recall@k > 0`. Con una sola esperada son iguales. Se calculan de los `recall@k` que `CaseResult` ya guarda, así que también salen de un archivo viejo.

Van en `retrieval.hits` y dentro de cada bloque de `by_lang`, `by_subset` y `by_split`. El `Aggregate` fraccional no cambia.

`GoldenSetProvenance` suma `min_one_way_cases` (el 6 de D1) y la nota lo dice: «a paired comparison needs at least 6 cases moving one way (13.0 points of 46)». El flag `provisional` y sus umbrales no cambian: dicen si el set tiene forma de set de dominio, no si tiene poder para una diferencia chica.

## D6. Casos `unanswerable` y `update`

### Esquema

| Clave | Dónde | Regla |
|---|---|---|
| `subset: "unanswerable"` | `RetrievalCase` | `expected_refs` MUST estar vacía; y una lista vacía sólo vale en esta clase (hoy `validate` la deja pasar y `recall_at_k` la cuenta como 0) |
| `subset: "update"` y `stale_refs` | `RetrievalCase` | Exactamente una esperada; `stale_refs` no vacía, dentro de `corpus_refs`, sin la esperada; y `stale_refs` sólo vale en esta clase |
| `age_days` | `CorpusRow` | Opcional, por defecto 0, no negativo. En un caso `update`, la esperada MUST ser más nueva que cada vieja |

### Medición

- **Abstención.** Un caso `unanswerable` se responde igual que los otros, pero su resultado va a `retrieval.unanswerable` (fuera de `per_case`, de R@k, MRR, nDCG, del bloque `similarity` y de `--follow-up`) con `abstained = ranked_refs.is_empty()`. `retrieval.abstention` es el `Rate`. Con el ranking de hoy siempre hay resultados, así que la tasa esperada es 0: es la línea de base de search-abstention (35).
- **Frescura.** Un caso `update` queda en `per_case` y en R@k. `fresh_first` es verdadero cuando la esperada aparece en el ranking y ninguna vieja aparece antes. `retrieval.freshness` es el `Rate`.
- **Agent-view.** Los `unanswerable` quedan fuera de `hit_rate` y su `CaseView` lleva `abstained` (la búsqueda no imprimió ninguna línea); `agent_view.abstention` es el `Rate`. Los `update` se miden como cualquier caso: el corpus entra por la ingesta real, que pone la hora actual, así que `age_days` no aplica ahí y la frescura sólo se mide en la recuperación.

### Sembrar con edad

`EvalStore::insert` (`eval_api.rs:269-301`) suma `age_ms: u64`, que se resta de la hora actual al armar `created_at_ms`. `seed.rs` pasa `age_days` convertido; los demás llamadores pasan 0.

| Alternativa | Por qué no |
|---|---|
| Un `created_at` absoluto en la fila | Un golden escrito hoy envejecería: el decaimiento por edad cambiaría los números de un mes al otro sin tocar el código |
| Los `unanswerable` dentro de `per_case` y filtrados en `aggregate` | `aggregate` lo llaman el reporte, el audit, los barridos y el diff; cada uno tendría que acordarse del filtro. Una lista aparte es la forma que ya tiene `blocked` |
| La clase como un campo nuevo `kind` en lugar de `subset` | Serían dos ejes para la misma pregunta («qué clase de caso es») y el reporte ya parte por `subset` |

## D7. Brazo sólo léxico

`--lexical-only` responde cada caso con `EvalStore::lexical_search(query_text, agent_id, k)`, una función nueva de `eval_api` que llama a `lexical_candidates` (`crates/clawg-core/src/memory/search.rs:705-737`): la misma expresión `MATCH` y el mismo orden por `bm25` que el canal léxico de producción, sin vectores ni puntaje. `k` es `K_FINAL`. El archivo lleva el tag `lexical-only` y, como cualquier otro, se compara con `--diff` contra el ancla.

Es excluyente con `--agent-view`, `--follow-up`, `--docs`, los barridos y `--fts-tokenizer`. El bloque `similarity` se sigue calculando: es coseno puro y no depende del ranking.

| Alternativa | Por qué no |
|---|---|
| `--sweep-lexical` con el pool denso en 0 | El puntaje final sigue usando el coseno (`score_lexical_candidates`), así que no sería «sin vectores» |
| Una consulta FTS propia del arnés | Mediría otra cosa que el canal de producción; el piso tiene que ser el mismo canal sin el denso |

## D8. Los tests de `--diff` sin archivos ignorados

Los tres tests de `diff.rs` que leen `docs/eval/results/2026-08-16-real-symmetric.json` pasan a un fixture en el código con dos casos de `retrieval` y dos de `similarity`, como ya hace `a_results_file_without_the_block_still_diffs_against_one_with_it` (`report.rs:1003`). `diff_files_reads_both_paths…` escribe el fixture a un archivo temporal.
