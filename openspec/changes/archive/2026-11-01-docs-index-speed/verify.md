# docs-index-speed — Verificación

## Las puertas, antes del código (2026-10-31)

Cada puerta lee los campos de las corridas D y R de docs-index-cost, con los umbrales de la [propuesta](proposal.md) («Pasos con puerta»), fijados antes de esas corridas.

- **Corridas:** sobre `0f09b2b`, una detrás de otra, con la misma `index.machine`: `windows`, `x86_64`, avx2, avx512f y fma, 16 hilos. D es debug con la memoria vacía y R es release con `--no-cache`.
- **Archivos (fuera de git):** `docs/eval/results/2026-10-31-real-docs-cost-d.json` y `docs/eval/results/2026-10-31-real-docs-cost-r.json`.
- **Campos:** los copia la [verificación de docs-index-cost](../2026-10-31-docs-index-cost/verify.md), en «Lo que dio cada corrida». Los cocientes de abajo se recalcularon con esos campos y coinciden con los de su tabla «Los cocientes que leen las puertas de `docs-index-speed`».

| Puerta | Campo | Valor | Umbral | |
|---|---|---|---|---|
| 0 | R: `phases.embed` | 1935,034 s | ≥ 120 s | cumple |
| 0 | R: `phases.embed / index_seconds` | 1935,034 / 1951,200 = 0,992 | ≥ 0,5 | cumple |
| C | D: `(scan + mask + split + lookup + write + count_seconds) / index_seconds` | 81,674 / 1601,499 = 0,051 | ≥ 0,25 | **no cumple** |
| C | esa suma en R frente a la de D | 17,815 / 81,674 = 0,218 | ≤ 0,5 | cumple |
| W | R: `phases.write / index_seconds` | 3,010 / 1951,200 = 0,0015 | ≥ 0,10 | **no cumple** |
| A1 | R: `(padded − padded_sorted_in_doc) / padded` | (8 467 287 − 7 023 477) / 8 467 287 = 0,171 | ≥ 0,10 | cumple |
| A1 | ese ahorro × `phases.embed` frente a 2 × `count_seconds` de R | 0,171 × 1935,034 = 330,0 s frente a 2 × 1,933 = 3,9 s | ≥ | cumple |
| S | R: `(read + mask + split + lookup + write) / index_seconds` (W no abre, así que se lee R) | 16,111 / 1951,200 = 0,0083 | ≥ 0,15 | **no cumple** |
| A2 | R: `(padded_sorted_in_doc − padded_sorted_all) / padded` | (7 023 477 − 5 648 371) / 8 467 287 = 0,162 | ≥ 0,15 | cumple |
| E | la puerta 0 en la última corrida | abierta | abierta | cumple, pero queda en espera |

### Qué se abre y qué se cierra

- **0 abre A1, A2 y E.** `embed` es más de la mitad del total, así que S no pasa delante de A1 y el orden de la propuesta sigue como está.
- **C se cierra sin probar la variante.** Lo que no es inferencia es el 5,1 % de una corrida debug. La segunda condición se cumple, pero la puerta pide las dos. La línea de `specs/eval` («`rag-eval --docs` se corre en release») era para una variante que se probó y no se quedó, así que no se escribe. Además, en estas corridas el `embed` de release salió un 26,8 % más lento que el de debug: correr en release no acorta una corrida de docs.
- **W se cierra.** Escribir es el 0,15 % del total. `prepare_cached` en `replace_doc` y `delete_doc` ganaría menos que el ruido.
- **S se cierra.** Leer, enmascarar, partir, consultar y escribir son el 0,83 % del total en R, y el solape esconde a lo sumo eso.
- **A1 abre y es el paso siguiente.** Es el único paso que se implementa ahora.
- **A2 abre**, pero va después de A1: su puerta mide lo que suma a A1, y su forma (la cola, su tope, el fallo de un vector) se diseña con la medición de A1 hecha.
- **E queda en espera.** La puerta abre, pero el paso necesita la huella de docs que cubre cómo se parte (el `docs_contract` de la decisión 6 de docs-search-quality). Esa huella venía en el paso 3 de docs-search-quality, que no se hizo (`../archive/2026-10-31-docs-search-quality/state.yaml:18`). Sin ella, `plan_refresh` conserva los pasajes viejos, y el experimento no tiene cómo reindexar.

### Antes de leer el tiempo de A1

En R, `phases.embed` salió un 26,8 % más lento que en D (1935,0 s frente a 1526,3 s), en contra del build. La verificación de docs-index-cost lo atribuye a la carga de la máquina, que ninguna corrida registra. Esa variación es más grande que el −10 % que pide «se queda si» de A1. Por eso:

- El tiempo de A1 se mide con corridas repetidas o alternadas de la base y del paso, en las mismas condiciones, y nunca contra este R suelto.
- Mientras se mide, no corre otra compilación, otra suite ni otra corrida.

Los tokens no dependen del reloj: el 0,171 de A1 y el 0,162 de A2 valen tal cual. A1 cumple su segunda condición aunque `phases.embed` fuera la mitad.

## A1: los bits, con el modelo real (2026-10-31)

Corridas autorizadas, una por vez y sin otra compilación en marcha. Las dos usan el modelo del perfil dev (`multilingual-e5-small`), sólo leído, en esta máquina: Windows `x86_64` con AVX2, AVX-512F y FMA, que es la huella de la fixture. El build es debug en `target-test`.

### `e5_golden`

- El bless (`CLAWG_BLESS=1`, tarea 2a) agrega el caso `a batch of 17, longest first`.
- El diff de `crates/clawg-core/tests/fixtures/e5_golden.json` sólo agrega esa línea y la coma de la anterior. «a batch of 17» y los otros 15 casos quedan byte por byte iguales: el bless en esta CPU no movió ningún vector.
- Sin `CLAWG_BLESS`, los cinco tests de `e5_golden` pasan, y la comparación es por bits («this CPU is the one that made the fixture»).
- `a_batch_longest_first_stays_within_the_tolerance_of_the_batch_in_order`: **17 de 17 vectores con los mismos bits en los dos órdenes**.
- Commit: `3d7ae94`.

### Pasajes reales de docs

`docs_passages_keep_their_bits_in_either_order` (tarea 2b) se corrió con `CLAWG_EMBED_RUNS=1` y el tope de 5000 pasajes:

- **100 docs de más de 16 pasajes: 4799 de 4799 vectores con los mismos bits en los dos órdenes, diferencia mayor 0.**
- Tiempo, una corrida de cada orden, una detrás de la otra: el orden de los pasajes tardó 460,87 s (10,4 pasajes/s) y del más largo al más corto, 421,07 s (11,4 pasajes/s). Es un 8,6 % menos.

### Qué se concluye

- **La decisión 4 se cumple con diferencia 0, en esta CPU.** No hay que reescribir vectores. Como no se movió ningún bit, la memoria de `docs/eval/cache/` sigue siendo exacta en esta máquina, y la tarea 2c no la vacía. Las corridas de 2d usan `--no-cache` de todas formas. En otra CPU, `e5_golden` compara con 1e-6, como antes.
- **El tiempo de 2b no decide «se queda si».** Es una corrida por orden, sin alternar, en debug, y la varianza entre corridas de esta máquina llegó al 26,8 %. Hay una salvedad. Todo el ahorro de tokens está en los docs de más de una tanda: `(padded − padded_sorted_in_doc) / multi_batch_padded` de R da 1 443 810 / 6 652 055 = 0,217. Este subconjunto es justo de esos docs, y aun así el reloj bajó un 8,6 %, menos que el −10 % que pide «se queda si». La prueba no imprime el relleno de su muestra, así que no se sabe si la muestra rellena como el corpus entero. Decide la tarea 2d, con corridas alternadas de `rag-eval`.

### Suite y clippy

Sobre `604dc03`, el último commit de la serie, salen con 0 la suite (`cargo test --workspace --target-dir target-test`) y los dos clippy (`--workspace --all-targets` y `--locked -p clawg-cli`). Terminaron antes de que entrara a `develop` el merge `eafcb8a` (native-memory-import y la versión 0.72.0), que no toca `project_docs`, el embedder, el chunker, `eval_api` ni `clawg-eval`.

## A1: «se queda si» (2026-10-31)

Seis corridas autorizadas de `rag-eval --real --docs --no-cache`, en release, con `CLAWG_PROFILE=dev` y el modelo real, una detrás de otra y sin otra compilación ni prueba en marcha. Alternan la base y el paso: base, paso, base, paso, base, paso.

- **Base:** `ad22dad`, el commit anterior al cambio de conducta, compilado en un worktree descartable con su propio directorio de build.
- **Paso:** `eafcb8a`, compilado en `target-test` después de `cargo clean --workspace` (la versión subió a 0.72.0).
- **Corpus:** las dos ramas indexan el mismo, congelado en un worktree de `eafcb8a`. Usan `--root`, `--golden` y `--out` explícitos, porque los valores por omisión de `rag-eval` salen de `CARGO_MANIFEST_DIR` al compilar (`crates/clawg-eval/src/bin/rag-eval/cli.rs:287-320`). Son 772 docs, 18 885 pasajes y 2 saltados, con el golden de 40 preguntas. Los dos worktrees y el build de la base se borraron al terminar.
- **Resultados (fuera de git):** `docs/eval/results/2026-10-31-real-docs-a1-{base,step}-{1,2,3}.json`.

| corrida | empieza | `index_seconds` | `phases.embed` | `phases.split` | `tokens.padded_ran` |
|---|---|---|---|---|---|
| base-1 | 13:50 | 2033,4 | 2015,76 | 13,07 | 8 765 221 |
| paso-1 | 14:24 | 1777,8 | 1760,55 | 12,64 | 7 282 905 |
| base-2 | 14:54 | 2064,0 | 2046,79 | 12,55 | 8 765 221 |
| paso-2 | 15:29 | 1454,2 | 1433,82 | 9,36 | 7 282 905 |
| base-3 | 15:53 | 1258,3 | 1246,35 | 8,53 | 8 765 221 |
| paso-3 | 16:14 | 1101,1 | 1088,82 | 8,84 | 7 282 905 |

Los tokens son iguales en las seis corridas: `real` 5 845 797, `padded` 8 765 221, `padded_sorted_in_doc` 7 282 905 y `padded_sorted_all` 5 849 005. El paso rellena un 16,9 % menos.

**La máquina cambió de régimen a mitad de la medición.** `split` es CPU pura y hace el mismo trabajo en las dos ramas. Bajó de unos 12,6 s a unos 8,7 s entre base-2 y base-3, y todas las fases bajaron con él. Paso-2 quedó a caballo: su `split` es de 9,36 s, y su `read`, de 7,88 s, unas 25 veces el de las otras corridas. Ninguna corrida registra la carga del sistema. Por eso las medianas por rama mezclarían dos máquinas, y se comparan corridas vecinas del mismo régimen:

| comparación | `phases.embed` |
|---|---|
| par 1, régimen lento (base-1 → paso-1) | **−12,7 %** |
| par 3, régimen rápido (base-3 → paso-3) | **−12,6 %** |
| paso-1 contra el promedio de base-1 y base-2 | −13,3 % |
| paso-2 contra el promedio de base-2 y base-3 | −12,9 % |
| par 2 (base-2 → paso-2), cruza el cambio | −29,9 %, no se usa |

Por área (`by_area`), en los dos pares limpios. Los conteos son iguales en las seis corridas: docs 64 docs, 2713 pasajes y 43 de más de una tanda; code 200, 8282 y 105; archive 508, 7890 y 150.

| área | par 1: `embed` base → paso | | par 3: `embed` base → paso | |
|---|---|---|---|---|
| docs | 301,97 → 220,51 s | −27,0 % | 187,08 → 137,54 s | −26,5 % |
| code | 843,96 → 761,20 s | −9,8 % | 540,27 → 472,06 s | −12,6 % |
| archive | 869,83 → 778,84 s | −10,5 % | 519,00 → 479,22 s | −7,7 % |

Donde más gana es en los docs de Markdown, que mezclan secciones de una línea con secciones largas. Los pedazos de código tienen un tope de caracteres (`CODE_CHUNK_MAX_CHARS`), así que sus largos se parecen más y rellenan menos de entrada.

Los dos pares limpios dan lo mismo, uno en cada régimen, y los promedios de las corridas vecinas coinciden con ellos. El reloj baja alrededor de tres cuartos de lo que bajan los tokens rellenados (−12,7 % contra −16,9 %). Eso es lo esperable: parte del costo de cada tanda no depende del relleno. El −8,6 % de la prueba de bits en debug (2b), con una sola corrida por orden, quedó dentro de ese ruido.

### Las condiciones de «se queda si»

| condición | resultado | |
|---|---|---|
| `phases.embed` baja ≥ 10 % | −12,6 % y −12,7 % en los pares limpios; entre −12,9 % y −13,3 % contra los vecinos | ✅ |
| `tokens.padded_ran` = `tokens.padded_sorted_in_doc` | 7 282 905 = 7 282 905 en las tres corridas del paso, y ningún aviso (en la base, `padded_ran` = `padded` = 8 765 221) | ✅ |
| `--diff` no mueve ninguna pregunta | base-n contra paso-n, en los tres pares: 0 mejores y 0 peores en Hit@1, Hit@3, Hit@5 y MRR@10, en todas, docs y código (p = 1). Las seis corridas dan Hit@1 0,05, Hit@3 0,10, Hit@5 0,10 y MRR@10 0,08 | ✅ |
| decisión 4 de la propuesta | 0 bits movidos (arriba) | ✅ |

**A1 se queda.** Lo que sigue es A2, que se especifica y se diseña con esta corrida como base. Su puerta es (`padded_sorted_in_doc` − `padded_sorted_all`) / `padded`: 0,162 en R y (7 282 905 − 5 849 005) / 8 765 221 = 0,164 en este corpus. Su «se queda si» compara contra corridas del paso de A1, en las mismas condiciones.

## A1: verificación del código (2026-10-31)

Verifica el código de A1, los commits `ad22dad`, `e81deed`, `eebaf09`, `eedd2f7`, `f663d3b` y `3d7ae94` (`git diff 30bf545..1c806ac -- crates docs/reference`, sin los archivos del merge `eafcb8a`), contra la [spec](spec.md) (IS1 a IS5), el [diseño](design.md) (D1 a D7) y la Parte 1 de [tasks.md](tasks.md). Los resultados con el modelo real (los bits y «se queda si») son los de arriba y no se volvieron a correr.

- **El árbol.** Cuando empezó la verificación, `HEAD` ya era `67914fc`, un merge de `develop` posterior a `1c806ac` que trae agent-eval. No toca ningún archivo de A1: en `crates/clawg-core` sólo agrega `eval_api::earned_tier` y su prueba, y en `crates/clawg-eval` el binario `agent-eval`. Todo se corrió sobre `67914fc`, sin cambiar de rama. Por eso, al final, `git diff 1c806ac -- crates` no da vacío (trae ese merge), y la comprobación que vale es `git diff HEAD -- crates`, que sí da vacío.
- **El build.** Antes de la primera prueba se corrió `cargo clean --workspace --target-dir target-test`. Sólo se usó `target-test`. No hubo build release, ni modelo real, ni `CLAWG_BLESS`, ni pruebas `#[ignore]`, ni acceso a `~/.clawg*`.
- **Las mutaciones.** Cada una se aplicó sobre el árbol limpio y se revirtió con `git restore` del archivo antes de la siguiente. Casi todas se corrieron con `cargo test -p clawg-core --features eval --target-dir target-test --lib --no-fail-fast`. Las que tocan `longest_first` o el aviso también con `cargo test -p clawg-eval --target-dir target-test index_cost`. M7, la que más se esperaba que no mordiera, se corrió con `cargo test --workspace --no-fail-fast`. Las de la fixture, con `--test e5_golden`. Sólo se juntaron en una corrida mutaciones de archivos distintos cuyas pruebas no se pisan, y cada falla se atribuyó por la prueba y por el mensaje.

### Las mutaciones

La aplicación nunca vio fallar por una aserción las cinco pruebas de refresco de IS1: pasaron de no compilar a pasar. Lo que sigue prueba que muerden. Las líneas son las de `67914fc`.

| # | Mutación | Prueba que falla | Resultado |
|---|---|---|---|
| M1 | `passage_order` da siempre `None` (`… \|\| true` en `crates/clawg-core/src/memory/project_docs/mod.rs:689`) | `a_doc_over_a_batch_reaches_the_embedder_longest_first` (`mod.rs:2155`: `[(1, 0), (8, 1)] is out of order`), `passage_order_is_none_up_to_one_batch_and_longest_first_above_it` (`:2133`: «more than one batch is ordered»), `passage_order_is_the_order_a_refresh_embeds_in` (`eval_api.rs:2975`: la identidad frente a `None`) | muerde |
| M2 | `embed_passages` no llama `in_passage_order`, y los vectores quedan en el orden en que se mandaron (`mod.rs:671`) | `a_reordered_doc_keeps_each_passage_with_its_own_vector` (`:2172`: «passage 0»), `a_reordered_doc_writes_what_an_unordered_one_wrote` (`:2191`) | muerde |
| M3 | el umbral pasa a `texts.len() < EMBED_BATCH` (`:689`) | `a_doc_of_one_batch_or_less_keeps_the_passage_order` (`:2205`), `passage_order_is_none_…` (`:2129`: 16 textos dan un orden) | muerde |
| M4 | el umbral pasa a `texts.len() <= 8` | las dos de M3 y `reuse_decides_whether_a_doc_is_reordered` (`:2242`: «ten texts are one batch or less») | muerde |
| M5 | la clave sin el prefijo de pasaje: se cuentan los textos tal cual (`:692-695`) | ninguna en la biblioteca de `clawg-core` | **no muerde** (WARNING 2) |
| M6 | la clave sin la ventana: `encode_batch(…, true)` sin truncar, en lugar de `window_encodings` (`:697`) | ninguna en la biblioteca de `clawg-core` | **no muerde** (WARNING 2) |
| M7 | la clave de `TokenSplitter::count_all`: el texto crudo, sin prefijo, sin tokens especiales y sin ventana | ninguna en toda la suite (`cargo test --workspace`) | **no muerde** (WARNING 2) |
| M8 | `in_passage_order` junta (`sorted[order[j]]`) en vez de repartir (`:703-709`) | `in_passage_order_undoes_longest_first` (`:2118`: `[40, 30, 10, 0, 20]`), `a_reordered_doc_keeps_…` (`:2172`), `a_reordered_doc_writes_…` (`:2191`) | muerde |
| M9 | una llamada al embedder por tanda en lugar de una por doc (`:663-666`) | `a_doc_over_a_batch_…` (`:2150`: «one call for the doc», 2 contra 1), `without_a_tokenizer_a_doc_keeps_the_passage_order` (`:2218`), `an_empty_memo_embeds_in_the_batches_production_does` (`eval_api/vector_memo.rs:903`), `a_changed_doc_embeds_only_its_new_texts` (`vector_memo.rs:925`) | muerde |
| M10 | `longest_first` ascendente (`crates/clawg-core/src/memory/embedder.rs:191`) | `longest_first_puts_the_most_tokens_first_and_keeps_ties_in_order` (`embedder.rs:998`), `longest_first_is_stable_and_descending` (`eval_api.rs:2983`), `passage_order_is_the_order_a_refresh_embeds_in`, `a_doc_over_a_batch_…`, `passage_order_is_none_…`, dos de la importación (`import.rs:5289`) y tres de `index_cost` (`a_doc_over_embed_batch_runs_several_batches`, `sorting_can_pad_more_when_the_last_batch_is_short`, `sorting_keeps_ties_in_their_order`) | muerde |
| M11 | `longest_first` con `sort_unstable_by_key` | sólo `an_import_of_another_tool_embeds_its_rows_longest_first` (`import.rs:5308`: «equal lengths keep the order of the file») | muerde, pero ninguna prueba de A1 (SUGGESTION 1) |
| M12 | `longest_first` desempata al revés (`(Reverse(len), Reverse(k))`) | `longest_first_puts_…`, `longest_first_is_stable_and_descending`, `passage_order_is_the_order_a_refresh_embeds_in`, `a_doc_over_a_batch_…`, `passage_order_is_none_…`, `an_import_of_another_tool_…`, `sorting_keeps_ties_in_their_order` | muerde |
| M13 | `padding_warning` compara `padded` otra vez (`crates/clawg-eval/src/bin/rag-eval/index_cost.rs:155`) | `the_padding_warning_compares_with_the_sorted_in_doc_padding` (`index_cost.rs:420`), `a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo` (`:442`) | muerde |
| M14 | el mensaje del aviso nombra `padded` (`:158`) | `the_padding_warning_names_both_numbers` (`:430`: nombra 1200 y 1000), `a_mismatch_…` (`:440`) | muerde |
| M15 | `eval_api::passage_order` da siempre la identidad (`crates/clawg-core/src/eval_api.rs:1201`) | `passage_order_is_the_order_a_refresh_embeds_in` (`eval_api.rs:2975`) | muerde |
| M16 | `TokenSplitter::tokenizer()` da siempre `None` (`crates/clawg-core/src/memory/chunker/token_splitter.rs:31`) | `a_doc_over_a_batch_…` (`mod.rs:2155`) | muerde |
| M17 | la fixture sin el caso `a batch of 17, longest first` (renombrado) | `a_batch_longest_first_stays_within_the_tolerance_of_the_batch_in_order` (`crates/clawg-core/tests/e5_golden.rs:137`: «the fixture has no case named …») | muerde |
| M18 | la fixture con un vector del caso nuevo movido: su primera componente, a 0 | la misma (`e5_golden.rs:146`): nombra el texto y «moved its vector by 0.041131318, over 0.000001» | muerde |
| M19 | un vector inutilizable corta el refresco: el error de `to_unit` pasa a `DocIndexError::Embed` (`mod.rs:678`) | `an_unusable_vector_in_a_reordered_doc_skips_only_that_doc` (`:2102`), y también las anteriores `an_unusable_vector_skips_its_doc_and_the_rest_go_on` y `a_refresh_times_a_doc_whose_vector_is_unusable` | muerde |

De las 19 mutaciones, 16 muerden. Las cinco pruebas de refresco de IS1 muerden todas: `a_doc_over_a_batch_…` (M1, M9, M10, M12, M16), `a_reordered_doc_keeps_…` y `a_reordered_doc_writes_…` (M2, M8), `reuse_decides_…` (M4) y `an_unusable_vector_in_a_reordered_doc_…` (M19). M19 no es propia del orden: es la misma regla que ya fijaba `an_unusable_vector_skips_its_doc_and_the_rest_go_on`, y la prueba nueva comprueba que el orden no la rompe.

**Por qué M5, M6 y M7 no muerden, y cuánto importa.** El modelo rellena según K = min(L + c, W): L son los tokens del texto, c el prefijo y los tokens especiales, W la ventana. Con un tokenizer que cuenta el prefijo siempre igual, K es una función no decreciente de L. Ordenar por L, por L + c o por K da entonces la misma secuencia de valores de K, y el relleno de cada tanda depende sólo de esa secuencia. Así, ninguna de las tres claves mutadas cambia el relleno. Lo único que cambia es el orden entre textos que empatan en K, que son los que la ventana corta. Eso puede cambiar qué textos comparten tanda, y con eso los últimos bits (IS4 e IS5 midieron 0 bits movidos con la clave de producción). Con el tokenizer de palabras de las pruebas, c es constante, y ningún texto de las pruebas de refresco se acerca a la ventana: ninguna prueba puede ver la diferencia. Lo que la ve, y lo que importa para la puerta, es la igualdad que el modelo real dio en las tres corridas del paso: `padded_ran` = `padded_sorted_in_doc` = 7 282 905. `padded_sorted_in_doc` se cuenta con prefijo y ventana (`EvalDocIndex::passage_lengths`, `eval_api.rs:1464-1503`), y `padding_warning` avisaría si no coincidieran. Una clave que con el tokenizer real contara el prefijo distinto según el texto sí podría cambiar el relleno, y la corrida real lo habría mostrado. El hueco es de la suite, no del código.

### Cumplimiento de la spec

Cada escenario contra la prueba que lo sostiene. Todas salieron `ok` en la corrida final, salvo las ignoradas, que se toman de las corridas con el modelo real anotadas arriba. ✅ cumple; ⚠️ parcial.

| Requisito | Escenario | Prueba | |
|---|---|---|---|
| IS1 | un doc largo llega ordenado | `a_doc_over_a_batch_reaches_the_embedder_longest_first` (`mod.rs:2140`): una llamada de 20 textos, que no crecen, con los empates por sección | ✅ |
| IS1 | cada pasaje conserva su vector | `a_reordered_doc_keeps_each_passage_with_its_own_vector` (`:2161`) y `a_reordered_doc_writes_what_an_unordered_one_wrote` (`:2177`): `RefreshReport` y `dump` iguales a los de un refresco sin tokenizer, con docs de 20, 30 y 2 pasajes | ✅ |
| IS1 | la reutilización decide | `reuse_decides_whether_a_doc_is_reordered` (`:2223`): 10 reusados y 10 en el orden de los pasajes. Que no se cuenten tokens es estructural: `passage_order` vuelve antes de tokenizar (`:689-691`), y no hay costura para observarlo | ✅ |
| IS1 | una tanda o menos | `a_doc_of_one_batch_or_less_keeps_the_passage_order` (`:2195`), `passage_order_is_none_up_to_one_batch_and_longest_first_above_it` (`:2124`): 16 dan `None` y 17, un orden. Lo de no tokenizar, como en el escenario anterior | ✅ |
| IS1 | un vector inutilizable | `an_unusable_vector_in_a_reordered_doc_skips_only_that_doc` (`:2246`) | ✅ |
| IS1 | relleno menor | Ninguna prueba mide el relleno de las tandas que manda el refresco. Se sostiene por composición: el 1640 frente a 2000 es la fórmula (`a_doc_over_embed_batch_runs_several_batches`, `index_cost.rs:351`), y que el refresco mande un orden que no crece lo prueba `a_doc_over_a_batch_…`. Fuera de la suite lo cierra la corrida real, con `padded_ran` = `padded_sorted_in_doc` (WARNING 1) | ⚠️ |
| IS1 | (no es un escenario) las piezas puras que nombra el requisito | `in_passage_order_undoes_longest_first` (`:2113`: vacío, empates, identidad), `passage_order_is_none_…` | ✅ |
| IS2 | sin largos | `without_a_tokenizer_a_doc_keeps_the_passage_order` (`:2209`) y `passage_order(None, …)` en `:2136`. «Si contar falla» no tiene prueba, porque el tokenizer de palabras no falla. Es el `.ok()?` de `:697` | ✅ |
| IS3 | coinciden | `the_padding_warning_compares_with_the_sorted_in_doc_padding` (`index_cost.rs:416`) | ✅ |
| IS3 | difieren | `the_padding_warning_names_both_numbers` (`:425`) | ✅ |
| IS3 | algo salió de la memoria | `a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo` (`:434`), con `from_memo = 30` | ✅ |
| IS4 | el caso falta | `a_batch_longest_first_stays_within_the_tolerance_of_the_batch_in_order` (`e5_golden.rs:131`), con M17. La spec nombra `the_vectors_are_the_ones_fastembed_gave`, pero esa prueba lee los textos de la fixture (`e5_golden.rs:345`), así que no falla si falta un caso (SUGGESTION 2) | ✅ |
| IS4 | el orden mueve un vector más de la cota | la misma prueba, con M18: nombra el texto y la diferencia. Con la fixture de `3d7ae94` imprime «17 of 17 vectors have the same bits in either order» | ✅ |
| IS5 | el informe | `docs_passages_keep_their_bits_in_either_order` (`mod.rs:2267`, ignorada): imprime los vectores con los mismos bits, la diferencia mayor y la mediana de cada orden. La corrida de arriba dio 4799 de 4799 y diferencia 0 | ✅ |

**12 de 13 escenarios cumplen enteros, y 1 en parte** (el relleno menor). Ninguno queda sin prueba.

Hay dos reglas del texto de IS1 que ninguna prueba muerde: contar «con el prefijo de pasaje del embedder, cortados a su ventana» (M5 a M7, WARNING 2) y «MUST NOT contar sus tokens» con una tanda o menos, que es estructural. IS3 pide que los campos no cambien de nombre ni de tipo, y eso también es estructural: el diff de `Tokens` sólo cambia comentarios. La fixture de IS4 sí reordena de verdad. En `a batch of 17, longest first` (`tests/fixtures/e5_golden.json:20`) van primero los cuatro textos largos y después las frases cortas, así que el caso no es la identidad.

### El diseño

| Decisión | ¿Se siguió? | Notas |
|---|---|---|
| D1. El orden vive en el refresco | ✅ | `E5Embedder::embed_batch` no cambia. El orden está en `embed_passages` (`mod.rs:648-679`) |
| D2. La clave y el tokenizer | ✅ | Prefijo de pasaje, `window_encodings` y `Encoding::len` (`:692-697`). El tokenizer sale de `TokenSplitter::tokenizer()` (`:543`). `refresh` y sus llamadores no cambian de firma |
| D3. Contar sólo cuando importa | ✅ | Se mide `to_embed` (`:540`, `:543`), no los pasajes del doc |
| D4. Las piezas puras | ✅ | `longest_first` está en `embedder.rs:189`, con su prueba, y la usan la importación y `eval_api::longest_first`. `in_passage_order` es privada, pero quedó junto a `passage_order` (`:703`) y no junto a `fill` (`:603`). `passage_order` es `pub(crate)` |
| D5. El costo de contar | ✅ | Va entre el `lap` de `lookup` y el de `embed` (`:542-544`) |
| D6. El aviso | ✅ | Compara contra `padded_sorted_in_doc` (`index_cost.rs:155`). Se corrigieron los comentarios de `Tokens`, de `EvalDocLengths` y de `passage_lengths`, y se renombró la prueba a «ordinal» |
| D7. Los bits | ✅ | Están el caso nuevo, la prueba sobre la fixture y la prueba ignorada, que no abre store ni temporal. Un desvío: la prueba del orden usa un embedder nuevo, `Batches` (`mod.rs:2034`), y no `Recording` (`:1466`). `Recording` anota textos y no llamadas, y no da un vector con NaN. El desvío está justificado, aunque repite la regla `BROKEN` de `Unusable` (`:1235`) |

### `.claude/CLAUDE.md` y las reglas

- **Nada prohibido en el rango.** No hay `#[allow]`, ni código comentado, ni `TODO`. Los comentarios nuevos son de documentación y explican un porqué. Por ejemplo, el de `passage_order` dice por qué da `None`: una tanda rellena igual en cualquier orden, sin tokenizer no hay con qué contar, y un texto que no se puede contar falla en el embedder.
- **Sin referencias al plan.** Ninguna prueba, comentario, mensaje o commit nombra un plan, una tarea, el cambio ni los números IS o D. «Phase» sólo aparece en `docs/reference/daemon.md`, como la fase `embed` de los tiempos del refresco (`DocPhases`).
- **Commits.** Los seis son convencionales y no tienen atribución. `3d7ae94` también marca tareas en `tasks.md` (SUGGESTION 6).
- **Visibilidad.**
  - `TokenSplitter::tokenizer()` es `pub` en un tipo `pub(crate)` de un módulo `pub(crate)`, así que su alcance real es el crate, como el de sus vecinos `with_tokenizer`, `fallback` y `count`. La usa producción (`mod.rs:543`).
  - `project_docs::passage_order` es `pub(crate)` porque la llama `eval_api`.
  - `eval_api::passage_order` es `pub` y sólo la usa `e5_golden`, que como prueba de integración sólo ve lo público. D7 lo eligió así para que la fixture fije el orden de producción, y la función sólo existe con `eval`, que el release no enlaza.
- **Puro e I/O.** `passage_order` e `in_passage_order` son puras. No hay dependencias nuevas: no cambian `Cargo.toml` ni `Cargo.lock`, y `tokenizers` ya era una dependencia de `clawg-core`.
- **Temporales.**
  - Las pruebas de refresco nuevas usan `crate::test_dir::test_dir()` y un store en memoria (`MemoryStore::new_for_test`, `crates/clawg-core/src/memory/db.rs:264`), así que no escriben ningún `memory.db` en el temporal.
  - Antes de la primera corrida, el temporal del sistema ya tenía ocho `.tmp*` con `memory.db` y `memory.lock`, un `.tmp*` con `clawg-copy.exe` y un `clawg-loader-lets-go-*` con `model.onnx`. Ninguno es `rag-eval-*`.
  - Al terminar hay uno más: `.tmpUFd8Mh`, con `memory.db` y un `memory.lock` vacío. Se escribió durante la corrida de `cargo test --workspace` con M7, que pasó entera, y la corrida final de la suite no dejó otro. Tiene la misma forma que los ocho anteriores: un store en disco, que las pruebas de A1 no abren. Parece una fuga intermitente de alguna prueba que abre un store en disco, anterior a este cambio y fuera de su alcance.
  - No se borró nada. Que lo haya creado esta verificación es probable pero no seguro, y la única herramienta para borrarlo queda fuera de lo que esta verificación podía usar.
  - `docs/eval/cache/` no existe.
- **`docs/reference/daemon.md`** describe lo que hace el código: el umbral, que haga falta un tokenizer, el orden con empates, la clave, la vuelta de cada vector a su pasaje y el conteo dentro de `embed`. Tiene dos detalles (SUGGESTION 4).

### Lo que encontró la verificación

**CRITICAL:** ninguno.

**WARNING:**

1. **Ninguna prueba de la suite mide el relleno de las tandas que manda el refresco** (escenario «relleno menor» de IS1). El 1640 frente a 2000 lo prueba la fórmula de `rag-eval` (`index_cost.rs:351`). Que el refresco mande ese orden lo prueba `a_doc_over_a_batch_…`. Que una secuencia que no crece rellene `padded_sorted` es aritmética. La medición la dio la corrida real (`padded_ran` = `padded_sorted_in_doc`). **Arreglo:** un `Batches` que, con el tokenizer de palabras, sume filas × el largo mayor de cada tanda de `EMBED_BATCH` que recibe, y un doc de 20 pasajes que alternan 10 y 100 palabras, que tiene que dar 1640.

   **Cerrado en `b97d97a`:** `a_doc_reaches_the_embedder_with_less_padding_than_in_its_passage_order` suma filas × el largo mayor de cada tanda de 16 que recibe el embedder, con un doc de 20 pasajes que alternan 10 y 100 palabras: da 16 × L100 + 4 × L10 (con L medido con el tokenizer, breadcrumb incluido) y menos que 20 × L100, lo que rellenaría el orden de los pasajes. Con `passage_order` siempre `None` (M1) falla.
2. **La clave «con el prefijo de pasaje, cortada a la ventana» de IS1 no la sostiene ninguna prueba** (`mod.rs:692-697`; mutaciones M5, M6 y M7). Por lo de arriba, no cambia el relleno con ningún tokenizer que cuente el prefijo siempre igual, y la corrida real mostró la igualdad. Pero la regla está en el requisito y se podría perder sin que nada fallara. **Arreglo:** un caso puro en `passage_order_is_none_up_to_one_batch_and_longest_first_above_it`, con el tokenizer de palabras, que ya tiene ventana de 512 y no agrega tokens especiales.

   **Cerrado en `b97d97a`:** `the_order_counts_the_passage_prefix` (511 y 512 palabras, con prefijo de una palabra empatan y quedan en el orden de los pasajes) y `the_order_counts_no_more_than_the_window` (600 y 700 empatan en 512). Mutaciones: M5 (sin prefijo) falla la primera (y `passage_lengths_follow_the_stored_passages_in_ordinal_order`); M6 (sin ventana) falla las dos y `a_passage_longer_than_the_window_counts_the_window`; M7 (`count_all` crudo) falla las dos. Lo único que cambia a la verificación original: el conteo ahora es una sola función, así que M5 y M6 también muerden en `eval_api`.
   - Con un embedder de prefijo de una palabra (el `Prefixed` de `eval_api.rs:2885` u otro igual), dos textos de 511 y 512 palabras empatan en la ventana y tienen que quedar en el orden de los pasajes. Sin prefijo, el de 512 pasaría delante.
   - Dos textos de 600 y 700 palabras también empatan en 512. Sin ventana, el de 700 pasaría delante.

**SUGGESTION:**

1. **Las pruebas de empates de A1 no ven un `sort_unstable`** (M11). Con este toolchain, el orden inestable deja los empates en su lugar con 20 elementos o menos, y todas las pruebas de A1 ordenan 20 o menos. Lo detecta una prueba de la importación con más filas, y como `longest_first` es compartida, la regla queda cubierta. Si se quiere que lo vea el refresco, `a_doc_over_a_batch_…` puede usar un doc de más de 20 pasajes con empates.

   **Cerrado en `b97d97a`:** `a_doc_over_a_batch_reaches_the_embedder_longest_first` usa un doc de 40 pasajes con empates. Con `sort_unstable_by_key` en `longest_first` falla (junto a la de la importación).
2. **El escenario «el caso falta» de IS4 nombra una prueba que no falla por eso.** `the_vectors_are_the_ones_fastembed_gave` lee los textos de la fixture (`e5_golden.rs:328-366`), así que un caso que falta no lo hace fallar. La que falla es `a_batch_longest_first_stays_within_the_tolerance_of_the_batch_in_order` (`:137`). Al archivar, conviene que la spec nombre esa. Por la misma razón, la fixture fija el orden de producción del momento del bless. Ninguna prueba de la suite compara ese orden con el `passage_order` de hoy, porque hace falta el tokenizer real.
3. **`passage_order` arma las copias con prefijo antes de mirar si hay tokenizer** (`mod.rs:692-697`). Sin tokenizer y con más de 16 textos, copia todos para nada. Basta con poner `let tokenizer = tokenizer?;` antes de la línea 692. Además, `passage_order` y `EvalDocIndex::passage_lengths` (`eval_api.rs:1490-1502`) arman la clave cada uno por su lado, y la igualdad de D2 depende de que coincidan. Una sola función para las dos haría que `a_passage_longer_than_the_window_counts_the_window` (`eval_api.rs:2955`) cubriera también la de producción, y cerraría el WARNING 2 de paso.

   **Cerrado en `102feda`:** `embedder::passage_token_counts` (prefijo, `window_encodings`, `Encoding::len`) la usan `passage_order` y `EvalDocIndex::passage_lengths`; `passage_order` vuelve antes de armar copias si no hay tokenizer. `count_seconds` ahora incluye el armado del prefijo, que es parte de contar.
4. **`docs/reference/daemon.md`** dice «so the order changes no row, id or report». Con el modelo real, los últimos bits de un vector pueden moverse según con qué textos comparte tanda (IS1). En esta CPU se movieron 0, pero en otra pueden moverse. Tampoco dice que un conteo fallido deja el orden de los pasajes. Son dos frases.

   **Cerrado en `02355b4`:** el párrafo dice que los últimos bits de un vector pueden moverse y que, si el conteo falla, los textos van en el orden de los pasajes.
5. **Detalles de forma.** El comentario de `passage_lengths` quedó mal partido después de la edición («`None` without a» / «tokenizer, which only the real model brings», `eval_api.rs:1468-1469`). También falta una línea en blanco entre ítems de prueba en `mod.rs:2031-2032` y en `:2259-2260`.

   **Cerrado en `102feda` (el comentario de `passage_lengths`) y `b97d97a` (las líneas en blanco de `mod.rs`).**
6. **`3d7ae94` mezcla.** Es un commit `test(embedder)` que también marca casillas de `tasks.md`. Ese cambio era su propio commit de `docs(openspec)`.

### Resultado de la suite y de clippy

Sobre `67914fc` (`HEAD`), con el árbol limpio y después de revertir las mutaciones M1 a M18:

- `cargo test --workspace --no-fail-fast --target-dir target-test`: pasa (salida 0). Las ignoradas son las de antes más `docs_passages_keep_their_bits_in_either_order`, que es la medición de IS5.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: salida 0.
- `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings`: salida 0.
- M19 se aplicó y se revirtió después de esa corrida. Al terminar, `git status` está limpio y `git diff HEAD -- crates` no da nada, así que la suite corrió sobre los mismos bytes que quedan.
- Ninguna métrica de `rag-eval` se volvió a medir. Las de A1 son las de «se queda si», arriba.

### Veredicto

**PASS WITH WARNINGS.** La suite y los dos clippy pasan. De las 19 mutaciones, 16 muerden, y entre ellas las cinco pruebas de refresco de IS1, que la aplicación sólo había visto pasar de no compilar a pasar. Muerden también el umbral (16 y 8), la vuelta al orden de los pasajes (quitada o invertida), la llamada única por doc, el sentido y los empates de `longest_first`, el aviso y su mensaje, la puerta de `eval_api`, el tokenizer del partidor y la fixture (el caso que falta y el vector que se mueve). No muerden las tres que cambian la clave (M5, M6 y M7, WARNING 2). Por construcción, no pueden cambiar el relleno con un tokenizer que cuente el prefijo igual para todos los textos, y la igualdad `padded_ran` = `padded_sorted_in_doc` del modelo real cubre el caso que la suite no ve. 12 de 13 escenarios cumplen enteros, y el del relleno menor cumple en parte, cerrado por la corrida real (WARNING 1). Ningún hallazgo cambia la conducta del daemon ni un número de las puertas.

## A2: el techo, antes del código (2026-10-31)

### Por qué se midió antes de diseñar

A1 bajó los tokens rellenados un 16,9 % y `phases.embed` un 12,6–12,7 %: el reloj bajó unos 0,75 de lo que bajaron los tokens. Con esa conversión, ni un orden perfecto entre docs llegaba a «se queda si» de A2. Desde la corrida de A1, los tokens bajan un 19,7 % (7 282 905 → 5 849 005), lo que proyectaba −14,7 % frente al −15 % pedido. Antes de escribir código de producción se midió ese techo.

### La medición

`sorting_across_docs_against_sorting_in_each_doc` (`crates/clawg-core/src/memory/project_docs/mod.rs`, commit `0c9d49a`) es una prueba ignorada.

- **Corrida:** autorizada, con el modelo real del perfil dev en esta máquina, en build debug en `target-test` y sin otra compilación en marcha. Variables: `CLAWG_EMBED_PASSAGES=5000` y `CLAWG_EMBED_RUNS=3`.
- **Muestra:** uno de cada 4 docs de este repositorio, ordenados por ruta y de todos los tamaños. Son 197 docs y 5261 pasajes, 77 de esos docs con más de 16 pasajes.
- **Dos órdenes, alternados en el mismo proceso.** El de hoy, que ordena cada doc (`embed_passages`). Y el techo de A2: toda la muestra ordenada una vez por `passage_token_counts` y mandada junta. Cada uno cuenta sus propios tokens dentro de su tiempo.

| | en cada doc | entre docs | entre / en cada doc |
|---|---|---|---|
| tokens reales | 1 611 676 | 1 611 676 | |
| tokens rellenados | 1 998 461 | 1 614 977 | 0,808 |
| par 0 | 308,95 s | 244,05 s | 0,790 |
| par 1 | 265,53 s | 217,56 s | 0,819 |
| par 2 | 269,38 s | 212,84 s | 0,790 |
| mediana de los pares | | | **0,790** |

En el último par, 5261 de 5261 vectores dan los mismos bits en los dos órdenes, con diferencia mayor 0.

### Qué se concluye

- **La muestra es representativa en tokens.** Baja un 19,2 %, y el corpus entero un 19,7 %.
- **El techo de A2 es −21 % de tiempo de embeber**, por encima del −15 % de «se queda si». La proyección de arriba era falsa. Suponía una conversión fija de tokens a tiempo, y acá el tiempo bajó más que los tokens (21,0 % frente a 19,2 %).
- **La explicación más probable no está medida.** Ordenar entre docs no sólo rellena menos: también llena las tandas. Hoy cada doc de 16 pasajes o menos corre su propia tanda incompleta, y entre docs las tandas son de 16. Así el modelo corre menos veces, y cada corrida tiene un costo que no depende del relleno. El diseño de A2 tiene que contar las tandas además del relleno.
- **Una cola acotada recupera menos que este techo.** El diseño de A2 elige el tope con números que no necesitan el modelo: el relleno y las tandas que daría cada tope sobre el corpus. «Se queda si» de A2 se decide después con corridas de `rag-eval` como las de A1.
- **La decisión 4 se cumple también para A2 en esta CPU:** 0 bits movidos.

## A2: el tope de la cola (2026-10-31)

`padding_and_batches_by_queue_cap` (commit `e8169b4`, prueba ignorada) se corrió con autorización.

- **Qué lee:** sólo el tokenizer verificado del perfil dev, sin sesión de ONNX ni inferencia.
- **Dos corridas:**
  - Sobre este repositorio en `478f59f`.
  - Con `CLAWG_DOCS_ROOT` apuntando a un worktree descartable de `0c9d49a`, el commit de la corrida del techo. Corrió el mismo binario, porque el partidor no cambió entre los dos commits. El worktree se borró al terminar.

### La validación de la muestra del techo

Sobre el árbol de `0c9d49a`, la muestra tiene 197 docs y 5261 pasajes, como la del techo.

| | P | B | Q |
|---|---|---|---|
| en cada doc | **1 998 461** | 423 | 850 335 781 |
| todo junto | **1 614 977** | 329 | 606 730 453 |

Los dos P son los de la corrida del techo, así que la cuenta reproduce esa muestra. Las 423 tandas en cada doc son el número que esa corrida no imprimió. Las 329 de todo junto coinciden con ⌈5261/16⌉.

### La cuenta sobre el corpus de `478f59f`

Son 19 218 pasajes. P son los tokens rellenados, B las tandas, Q las filas × el largo al cuadrado, y H los bytes que retiene el vaciado más grande.

| orden | P | B | Q | llamada mayor | H |
|---|---|---|---|---|---|
| pasajes | 8 915 741 | 1571 | 4 196 844 517 | 278 | 1,2 MiB |
| en cada doc | 7 414 472 | 1571 | 3 171 559 180 | 278 | 1,2 MiB |
| cola 64 | 6 736 330 | 1322 | 2 722 211 078 | 278 | 1,2 MiB |
| cola 128 | 6 435 252 | 1276 | 2 531 996 006 | 278 | 1,2 MiB |
| cola 256 | 6 225 527 | 1241 | 2 405 565 215 | 278 | 1,3 MiB |
| cola 512 | 6 093 004 | 1220 | 2 328 582 506 | 512 | 2,2 MiB |
| cola 1024 | 6 029 953 | 1213 | 2 292 949 921 | 1023 | 4,4 MiB |
| cola 2048 | 6 002 502 | 1211 | 2 277 673 308 | 2041 | 8,6 MiB |
| cola 4096 | 5 990 423 | 1208 | 2 270 940 879 | 4088 | 17,2 MiB |
| todo junto | 5 957 676 | 1202 | 2 252 185 608 | 19 218 | 78,4 MiB |

### La regla de D9

- **Las reducciones ρ_X.**
  - Corpus de `478f59f`: P 0,1965, B 0,2349 y Q 0,2899.
  - Muestra del techo (árbol de `0c9d49a`): P 0,1919, B 0,2222 y Q 0,2865.
  - Cocientes corpus/muestra: 1,024, 1,057 y 1,012. El factor min(1, …) vale **1**.

| c | R_P | R_B | R_Q | previsto(c) | H ≤ 16 MiB | |
|---|---|---|---|---|---|---|
| 64 | 0,4655 | 0,6748 | 0,4888 | 0,0978 | sí | no |
| 128 | 0,6722 | 0,7995 | 0,6957 | 0,1412 | sí | no |
| 256 | 0,8161 | 0,8943 | 0,8332 | 0,1714 | sí | no |
| **512** | 0,9071 | 0,9512 | 0,9169 | **0,1905** | sí (2,2 MiB) | **el menor que cumple** |
| 1024 | 0,9504 | 0,9702 | 0,9557 | 0,1996 | sí | |
| 2048 | 0,9692 | 0,9756 | 0,9723 | 0,2035 | sí | |
| 4096 | 0,9775 | 0,9837 | 0,9796 | 0,2053 | no (17,2 MiB) | |

**`QUEUE_PASSAGES` = 512.** Es el menor tope con previsto ≥ 0,18 y H ≤ 16 MiB. El corpus de `0c9d49a` elige el mismo: previsto 0,1905 con c = 512 y 0,1713 con c = 256. A2 sigue a la Parte 6, el código de producción. Su «se queda si» lo deciden las corridas alternadas de la Parte 7, no esta cuenta.

## A2: verificación del código (2026-10-31)

Verifica el código de A2: `e8169b4` (la cuenta ignorada y la pieza de la cola, entonces dentro de `tests`) y `17678e0`, `efadede` y `7be1fad` (producción y evaluación), es decir `git show e8169b4` más `git diff 2f92e17..HEAD -- crates docs/reference`. Se compara con la [spec](spec.md) («Paso A2», IS6 a IS17), el [diseño](design.md) («Paso A2», D8 a D15), la Parte 6 de [tasks.md](tasks.md) con sus cinco desviaciones y `QUEUE_PASSAGES` = 512 de «A2: el tope de la cola». No se corrió nada con el modelo real: IS16 e IS17 son de la Parte 7.

- **El árbol.** `HEAD` es `f3de23f` en `develop`, con el árbol limpio. Los cuatro commits de código no tocan `openspec/`, y ninguno cambia `Cargo.toml`, `Cargo.lock`, `tests/e5_golden.rs` ni su fixture.
- **El build.** Sólo `target-test`, sin limpiar: es el mismo cambio que acaba de compilar la aplicación. No hubo build release, ni modelo real, ni `CLAWG_BLESS`, ni pruebas `#[ignore]`, ni acceso a `~/.clawg*`.
- **Las mutaciones.** Cada una se aplicó sobre el árbol limpio y se revirtió con `git restore` antes de la siguiente. Se corrieron con `cargo test -p clawg-core --features eval --target-dir target-test --lib --no-fail-fast`. Las que tocan la cola o la evaluación se corrieron también con `cargo test -p clawg-eval --target-dir target-test --bin rag-eval --no-fail-fast`, porque `index_cost` usa `queue_groups` de `clawg-core`. Sólo se juntaron mutaciones de archivos distintos (`embed_shares` con `index_cost.rs` o con `eval_api.rs`, y M6 con M7, que no muerden ninguna de las dos), y cada falla se atribuyó por la prueba y por el mensaje. M18 y M19 tocan la misma función: se corrieron juntas y después M19 sola.

### Las mutaciones

Las líneas son las de `f3de23f`, en `crates/clawg-core/src/memory/project_docs/mod.rs` salvo donde se indica.

| # | Mutación | Prueba que falla | Resultado |
|---|---|---|---|
| M1 | el límite no se duplica: `EMBED_BATCH.min(cap)` (`Queue::limit`, `:589`) | `the_queue_limit_starts_at_one_batch_and_doubles_up_to_the_cap` (`:2884`), `a_doc_over_the_limit_is_a_flush_of_its_own` (`:2899`: «the limit grew once per flush»), `small_docs_share_one_call_to_the_embedder` (`:2975`), `each_passage_of_a_shared_call_keeps_its_own_vector` (`:3027`), `no_call_holds_more_passages_than_the_cap_unless_it_is_one_doc` (`:2997`: todas las llamadas de 30), `queue_groups_are_the_calls_a_refresh_makes` (`eval_api.rs:2947`) | muerde |
| M2 | el límite se duplica sin tope (`:589` sin `.min(cap)`) | sólo las puras: `the_queue_limit_starts_…` (`:2882`), `the_queue_flushes_before_…` (`:2891`), `a_doc_over_the_limit_…` (`:2904`: `[[0], [1], [2, 3], [4, 5, 6]]`) | muerde, pero ninguna prueba de refresco (WARNING 2) |
| M3 | nunca vacía antes de un doc que pasaría el límite (`:601`) | `the_queue_flushes_before_…` (`:2891`), `a_doc_over_the_limit_…` (`:2898`), `a_doc_over_the_cap_goes_alone` (`:3012`), `queue_groups_are_the_calls_…` | muerde |
| M4 | sólo vacía al terminar el plan (`:601` y `:606` siempre falsos) | once: las cuatro puras, `small_docs_share_…`, `no_call_holds_…`, `a_doc_over_the_cap_…`, `each_passage_…`, `an_embedder_error_keeps_the_docs_of_its_call_as_they_were`, `queue_groups_are_the_calls_…` y `what_a_run_embedded_before_the_index_failed_is_kept` (`eval_api/vector_memo.rs:969`) | muerde |
| M5 | un doc más grande que el límite se suma a los de la cola (`:601` con `&& passages <= limit`) | `a_doc_over_the_limit_…` (`:2898`), `a_doc_over_the_cap_…` (`:3012`), `queue_groups_are_the_calls_…` | muerde |
| M6 | sin la vuelta temprana de un doc sin pasajes (`:598-600`) | ninguna | **no muerde**: mutante equivalente |
| M7 | el límite sin `.min(16)` en el corrimiento (`:589`) | ninguna | **no muerde** (SUGGESTION 1) |
| M8 | `refresh` arma la cola con `Queue::new(usize::MAX)` (`:516`) | ninguna | **no muerde** (WARNING 2) |
| M9 | una llamada de varios docs va sin ordenar (`:821`, `None` con más de un dueño) | `small_docs_share_…` (`:2977`: «longest first») | muerde |
| M10 | se ordena dentro de cada doc y no sobre la lista entera (`:821`) | `small_docs_share_…` (`:2977`) | muerde |
| M11 | los vectores de una llamada compartida, corridos en uno (`rotate_left(1)` antes de `:840`) | `each_passage_…` (`:3032`), `an_unusable_vector_in_a_shared_call_skips_only_its_doc`, `an_unusable_vector_skips_its_doc_and_the_rest_go_on`, `a_refresh_times_a_doc_whose_vector_is_unusable`, `search_finds_a_passage_by_meaning`, `search_finds_an_identifier_lexically` y `a_doc_the_index_could_not_embed_fails_the_measurement` (`eval_api.rs`) | muerde |
| M12 | el reparto por dueño, al revés (`:841-851` sobre `owners.iter().rev()`) | `each_passage_…` (`:3032`) y las demás de M11 salvo `an_unusable_vector_in_a_shared_call_…`, que sólo mira rutas y cuentas | muerde |
| M13 | un vector inutilizable hace fallar a todos los docs de la llamada (`:840`) | `an_unusable_vector_in_a_shared_call_…` (`:3048`), `an_unusable_vector_skips_its_doc_…`, `a_refresh_times_a_doc_whose_vector_is_unusable` | muerde |
| M14 | el doc que sigue a un vector inutilizable toma el resto de los vectores de ese doc: `take(n).collect::<Result<…>>()` sin la colección intermedia (`:846-849`), el riesgo que nombra el comentario de `:844-845` | ninguna | **no muerde** (WARNING 3) |
| M15 | las escrituras esperan al final del refresco (`replace_doc` de `:778` diferido hasta después del último vaciado) | `an_embedder_error_keeps_…` (`:3097`: «only the docs of the earlier call») | muerde |
| M16 | ante un error del embedder, los docs de esa llamada se escriben igual, con su huella y sin pasajes (`:752`) | `an_embedder_error_keeps_…`, `a_failed_embedding_keeps_the_doc_as_it_was`, `a_failed_embedding_of_the_rest_keeps_the_doc_as_it_was`, `a_short_batch_still_aborts_the_refresh` | muerde |
| M17 | los docs de un vaciado se escriben al revés (`:756` con `.rev()`) | `a_shared_call_writes_its_docs_in_plan_order` (`:3080`: «row ids grow with the path and the ordinal») | muerde |
| M18 | `Embed` y `VectorCount` nombran al último dueño (`:819`) | `an_embedder_error_keeps_…` (`:3094`: `d.md`), `a_short_batch_still_aborts_…` (`:1496`: `b.md`) | muerde |
| M19 | `VectorCount.passages` cuenta los textos del primer doc y no los de la llamada (`:834`) | ninguna: la única aserción sobre `VectorCount` (`:1496`) usa `..` | **no muerde** (SUGGESTION 2) |
| M20 | la cola cuenta sólo los textos a embeber y no los pasajes reutilizados (`:546`, `to_embed.len()`) | ninguna | **no muerde** (WARNING 4) |
| M21 | `embed_shares` pierde el resto (`:799`) | `embed_shares_split_the_time_by_texts_and_add_up_to_it` (`:3107`: «the rest goes to the last») | muerde |
| M22 | `embed_shares` reparte en partes iguales (`:796`) | `embed_shares_split_…` (`:3106`) | muerde |
| M23 | `embed_shares` le da tiempo a un doc sin textos (peso n + 1, `:796`) | `embed_shares_split_…` (`:3108`: «a doc with no texts gets none») | muerde |
| M24 | el resto va al último doc y no al último con textos (`:798`) | ninguna | **no muerde** (SUGGESTION 3) |
| M25 | `eval_api::queue_groups` con la mitad del tope (`eval_api.rs:1208`) | ninguna | **no muerde** (SUGGESTION 4) |
| M26 | `eval_api::queue_groups` con tope `EMBED_BATCH` (`eval_api.rs:1208`) | `queue_groups_are_the_calls_…` (`eval_api.rs:2947`) | muerde |
| M27 | `project_docs::queue_groups` no usa la decisión de la cola: cada doc es su grupo (`:620`) | `queue_groups_are_the_calls_…`, `a_doc_with_no_passages_moves_no_flush` (`:2912`), `padded_sorted_queue_sorts_each_group_of_docs_together` (`crates/clawg-eval/src/bin/rag-eval/index_cost.rs:432`) | muerde |
| M28 | `padding_warning` vuelve a comparar `padded_sorted_in_doc` (`index_cost.rs:171`) | `the_padding_warning_compares_with_the_queue_padding` (`index_cost.rs:455`: «the model ran the queue's padding»), `a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo` (`:480`) | muerde |
| M29 | `padded_sorted_queue` se calcula doc por doc (`index_cost.rs:132`) | `padded_sorted_queue_sorts_each_group_…` (`:432`) | muerde |
| M30 | el mensaje del aviso nombra `padded_sorted_in_doc` (`index_cost.rs:174`) | `the_padding_warning_names_both_numbers` (`:468`), `the_padding_warning_compares_…` (`:456`), `a_mismatch_…` (`:478`) | muerde |

De las 30 mutaciones, 22 muerden. Todas las pruebas nuevas de refresco muerden al menos una: `small_docs_share_…` (M1, M4, M9, M10), `each_passage_…` (M1, M4, M11, M12), `an_unusable_vector_in_a_shared_call_…` (M11, M13), `a_shared_call_writes_…` (M17), `an_embedder_error_keeps_…` (M4, M15, M16, M18), `a_doc_over_the_cap_goes_alone` (M3, M4, M5) y `no_call_holds_…` (M1, M4). La de `rag-eval`, `padded_sorted_queue_sorts_each_group_…`, no ve M1, M4 ni M26: sus tres docs dan los mismos grupos con límite 16 o con una sola llamada. Esas mutaciones las ve `queue_groups_are_the_calls_…` del lado de `clawg-core`.

**Las que no muerden, y cuánto importa.**
- **M6 es equivalente.** Después de cada `admit` la cola retiene menos que su límite, porque si llega a él se vacía. Un doc de 0 pasajes no puede entonces disparar ni el vaciado de antes ni el de después. La vuelta temprana sólo documenta la intención.
- **M7 importa con muchos vaciados.** `EMBED_BATCH << 60` da 0 en `usize`. Sin el `.min(16)`, del vaciado 60 al 63 el límite valdría 0, y cada doc iría solo. En debug, el vaciado 64 entraría en pánico. Con 512 pasajes por vaciado, este repositorio queda en unas decenas de vaciados (19 218 pasajes, «A2: el tope de la cola»), y uno algo más grande pasa de 60. Ninguna prueba hace más de unos pocos vaciados.
- **M8, M14 y M20** son WARNING 2, 3 y 4. **M19, M24 y M25** son SUGGESTION 2, 3 y 4.

### La conducta que cuida el diseño

- **`RefreshReport`, igual para las mismas entradas.** Sus campos son sumas. `failed` sigue el orden del plan, porque los vaciados van en ese orden y `write_queued` recorre sus docs en el orden en que entraron (`:756`). Lo fijan `a_shared_call_writes_…` (`:3068`, contra el informe literal de un refresco doc por doc), `timing_does_not_change_what_a_refresh_writes` y `a_reordered_doc_writes_what_an_unordered_one_wrote` (`:2422`). Ninguna prueba tiene dos docs en `failed` en una llamada, así que el orden de `failed` dentro de un vaciado sólo lo cubre, de paso, M17, por los ids.
- **Los ids de fila y los empates del ranking (D11).** Cada doc se escribe con su propio `replace_doc`, en el orden del plan (`:778`), así que la secuencia de `INSERT` es la de antes de la cola, y `fuse` desempata por id (`:392`). Lo fija `a_shared_call_writes_…` (M17). Un doc de un vaciado que falla por el embedder ya no queda escrito aunque vaya antes del texto que falló. Es lo que elige D10 y no cambia el informe, porque el resultado es un `Err`.
- **Los tiempos siguen sumando `index_seconds`.** Cada `lap` empieza donde terminó el anterior. `embed_shares` reparte el `lap` del vaciado entero (M21 a M23). Un vaciado sin textos deja el tiempo en la marca, y lo toma el `lap` siguiente: la escritura del primer doc, la lectura del doc siguiente o `remove`. Es la desviación (2). Un doc que no se pudo leer anota su tiempo en el momento (`:529`). La línea del daemon (`refresh_timing_line`, `crates/clawg-cli/src/daemon/api.rs:3029-3042`, IC2) no cambió, y `the_docs_timing_line_names_every_phase` (`api.rs:3207`) pasa.
- **`timing.docs`, ahora en el orden en que terminan los docs.** Ningún consumidor depende del orden del plan:
  - `api.rs:3030` sólo suma, con `RefreshTiming::phases`.
  - `eval_api.rs:1519-1536` copia la lista en el mismo orden a `EvalIndexRun::docs`.
  - En `rag-eval`, `Phases::of` suma (`index_cost.rs:22-35`) y `ByArea::of` agrupa por ruta y suma (`:261-300`). El bloque `index` no guarda una lista por doc (`IndexBlock::new`, `:324-342`).
  - Las pruebas ordenan antes de comparar (`mod.rs:1296`, `:1478`, `eval_api.rs:2876`, `:2941`).
- **La memoria (D13), por inspección.**
  - Un doc en cola retiene lo que dice D13 (`QueuedDoc`, `:721-732`): ruta, tipo y hash, sus pasajes, el plan de reutilización con los blobs reutilizados, los textos a embeber y sus fases. El texto leído y el enmascarado se sueltan al terminar la vuelta del bucle.
  - Los textos se mueven sin copiarse: `mem::take` en `:750` y `slots[k].take()` en `:824-825`.
  - Lo que H no cuenta es el WARNING 5.

### Cumplimiento de la spec

Cada escenario contra la prueba que lo sostiene, en la corrida final. ✅ cumple; ⚠️ parcial; ➖ no aplica; ⏳ pendiente de la Parte 7.

| Requisito | Escenario | Prueba | |
|---|---|---|---|
| IS6 | un tope cumple | `padding_and_batches_by_queue_cap` (`mod.rs:2803`, ignorada; se corrió con autorización, «A2: el tope de la cola»): c = 512. Desde `17678e0` usa la pieza de producción (`queue_calls` llama a `queue_groups`, que pasó a producción) | ✅ |
| IS6 | ninguno cumple | no aplica: c = 512 cumple (Parte 5d) | ➖ |
| IS7 | el límite crece | `the_queue_limit_starts_…` (`:2877`): 16, 32, 64, 64 (M1, M2, M4) | ✅ |
| IS7 | vaciar antes y al llegar | `the_queue_flushes_before_…` (`:2888`, con tope 16: M2, M3, M4) | ✅ |
| IS7 | un doc vacío | `a_doc_with_no_passages_moves_no_flush` (`:2908`, M4 y M27; M6 es equivalente) | ✅ |
| IS7 | (texto) los pasajes reutilizados cuentan contra el límite | ninguna (M20, WARNING 4) | ⚠️ |
| IS8 | un doc grande | `a_doc_over_the_limit_is_a_flush_of_its_own` (`:2896`, 3 y 40 con límite 16) y `a_doc_over_the_cap_goes_alone` (`:3001`) (M3, M5). El escenario usa un doc de 40, más del doble del límite. Uno de entre 17 y 31 no va solo (WARNING 1) | ✅ |
| IS8 | el tope se respeta | `no_call_holds_…` (`:2982`): con 20 docs de 30, las llamadas son 30, 30, 60, 120, 240 y 120. El límite nunca llega a 512, y la prueba pasa sin tope (M8, WARNING 2) | ⚠️ |
| IS9 | docs chicos | `small_docs_share_…` (`:2957`, con la desviación (1): un doc de 16 antes) (M1, M9, M10) | ✅ |
| IS10 | una llamada, tres docs | `each_passage_…` (`:3016`: 18 textos reordenados entre tres docs, y cada blob es el de su texto) (M11, M12) | ✅ |
| IS11 | el del medio | `an_unusable_vector_in_a_shared_call_…` (`:3037`) (M13). No mira los vectores del tercer doc, que con M14 se escribe con el vector del segundo (WARNING 3) | ⚠️ |
| IS12 | falla el segundo vaciado | `an_embedder_error_keeps_…` (`:3085`) (M15, M16, M18). `VectorCount.passages` no se afirma (M19, SUGGESTION 2) | ✅ |
| IS13 | ids por ruta | `a_shared_call_writes_…` (`:3057`, también el informe) (M17) | ✅ |
| IS14 | el reparto suma | `embed_shares_split_…` (`:3104`) (M21 a M23; M24 no, SUGGESTION 3) | ✅ |
| IS14 | todo doc con textos tiene `embed` | `every_doc_of_a_shared_call_has_embed_time` (`:3123`) | ✅ |
| IS14 | (texto) la línea del daemon no cambia de forma | `the_docs_timing_line_names_every_phase` (`api.rs:3207`); `api.rs` no cambió | ✅ |
| IS15 | coinciden | `the_padding_warning_compares_with_the_queue_padding` (`index_cost.rs:447`) (M28) | ✅ |
| IS15 | difieren | la misma y `the_padding_warning_names_both_numbers` (`:463`) (M28, M30) | ✅ |
| IS15 | (texto) sin aviso con `from_memo > 0` | la misma (`from_memo = 30`) y `a_mismatch_…` (`:472`) | ✅ |
| IS15 | los grupos son las llamadas | `queue_groups_are_the_calls_…` (`eval_api.rs:2924`, `[3, 20, 5, 0, 5]`) (M1, M3, M4, M5, M26, M27; M25 no, SUGGESTION 4) | ✅ |
| IS15 | (texto) `padded_sorted_queue` por grupo; los archivos viejos se leen con `--diff` | `padded_sorted_queue_sorts_each_group_…` (`index_cost.rs:432`) (M27, M29); `a_results_file_with_an_index_block_still_diffs_against_an_old_one` y `…without_an_index_block…` (`docs.rs:1117`, `:1130`). La comparación sólo lee `golden_sha256`, `corpus.left_out` y `per_case` (`docs.rs:410-420`), y `padded` y `padded_sorted_in_doc` no cambian de nombre | ✅ |
| IS16 | el informe | Parte 7a y 7b, sin escribir | ⏳ |
| IS17 | el pico | Parte 7d | ⏳ |

**De los 15 escenarios de producción (IS7 a IS15), 13 cumplen enteros y 2 en parte** («el tope se respeta» y «el del medio»). Ninguno queda sin prueba. Hay además una regla del texto de IS7 que no sostiene ninguna prueba (los pasajes reutilizados cuentan). IS6 cumple con su corrida. IS16 e IS17 esperan a la Parte 7.

**TDD.** No hay un artefacto `apply-progress`: la evidencia de TDD es la de `tasks.md` (RED, GREEN y comprobación por grupo), y cada commit lleva su código con sus pruebas, como en A1. Las pruebas que `tasks.md` nombra existen y pasan. Que muerden lo prueban las mutaciones de arriba.

### El diseño

| Decisión | ¿Se siguió? | Notas |
|---|---|---|
| D8. La cola | ⚠️ | Una pieza pura (`Queue`, `:573-612`) que recorren `refresh` (`:546`) y `queue_groups` (`:616-633`). El tope cuenta los pasajes retenidos, y los vectores vuelven por posición. El caso de un doc más grande que el límite difiere en la rampa (WARNING 1), y la regla de los reutilizados no tiene prueba (WARNING 4) |
| D9. El tope | ✅ | La cuenta de `e8169b4` usa la misma pieza, que desde `17678e0` es la de producción. `QUEUE_PASSAGES` = 512 (`:564`) |
| D10. Fallos | ✅ | Un vector inutilizable devuelve el error de su doc (`:841-851`), que va a `failed` (`:760-764`). `Embed` y `VectorCount` nombran al primer doc, con `passages` = textos del vaciado (`:819`, `:832-835`). Un fallo del store en `replace_doc` corta con los anteriores escritos (`:778`) |
| D11. Escritura y latencia | ✅ | Una transacción por doc, en el orden del plan. El límite empieza en 16 y se duplica |
| D12. Tiempos | ✅ | `embed_shares` (`:790-802`) en nanosegundos, con el resto al último doc con textos. Hay una desviación justificada, la (2) |
| D13. Memoria | ⚠️ | Lo retenido por doc y los textos movidos, como dice. La cota H no cubre todo lo que tiene el vaciado (WARNING 5) |
| D14. La evaluación | ✅ | `eval_api::queue_groups` (`eval_api.rs:1207-1209`) delega en la pieza con el tope de producción. Están `padded_sorted_queue`, el aviso (`index_cost.rs:170-177`), `Tokens::line` y los comentarios. Hay un detalle en el de `padded` (SUGGESTION 6) |
| D15. Bits | ➖ | No hay caso nuevo en `e5_golden` ni cambió su tolerancia. El tercer orden de `sorting_across_docs_…` es la Parte 7a |

Las desviaciones de `tasks.md` se sostienen. (1) Con límite 16, cuatro docs de 5 no caben en una llamada. (2) Ver arriba. (3) `what_a_run_embedded_before_the_index_failed_is_kept` necesita que el primer doc vaya solo, y M4 la hace fallar. (4) `queue_groups` tras `cfg(any(test, feature = "eval"))`. (5) Los tamaños 16 y 6 de la llamada que falla.

### `.claude/CLAUDE.md` y las reglas

- **Nada prohibido en el rango.** No hay `#[allow]`, ni código comentado, ni `TODO`. Los comentarios nuevos dicen un porqué:
  - por qué un vaciado sin textos no reparte (`:753-754`);
  - por qué se junta cada doc antes de recolectar sus resultados (`:844-845`);
  - por qué el primer doc va solo en las dos pruebas que lo necesitan (`vector_memo.rs:972-973`, `index_cost.rs:433-434`).
- **El comentario de «one changed doc in memory at a time» ya no está.** El de `:521-524` dice ahora que el refresco retiene sólo los docs de su cola, que acota `QUEUE_PASSAGES` («a doc past it goes alone»), y no el árbol. Es cierto: un doc con más pasajes que el tope siempre va solo.
- **Sin referencias al plan.** En las líneas agregadas no aparecen IS, D, A1, A2, paso, `openspec` ni spec. Los cinco commits (`e8169b4`, `17678e0`, `efadede`, `7be1fad` y `f3de23f`) son convencionales, sin cuerpo y sin atribución. Los de código no tocan `openspec/`.
- **Visibilidad.**
  - `QUEUE_PASSAGES` es `pub(crate)`: lo usan `refresh` y `eval_api`.
  - `project_docs::queue_groups` es `pub(crate)` tras `cfg(any(test, feature = "eval"))`. Lo llaman `eval_api::queue_groups` (con `eval`) y la cuenta ignorada (con `test`). Sin ninguno de los dos no se compila, así que no es código muerto, y el clippy de `clawg-cli` sin `eval` pasa.
  - `eval_api::queue_groups` es `pub`. La usa `rag-eval` (`index_cost.rs:132`), y `eval_api` sólo existe con `eval` (`crates/clawg-core/src/lib.rs:22-23`).
  - `Queue`, `Flush`, `Target`, `QueuedDoc`, `write_queued` y `embed_shares` son privados. `Target` agrupa las cinco referencias que necesita `write_queued`.
- **Puro e I/O.** `Queue`, `queue_groups` y `embed_shares` son puras. No hay dependencias nuevas.
- **Temporales.**
  - Al terminar apareció un `.tmp*` nuevo en el temporal del sistema, con un `memory.db` de 2 MiB y un `memory.lock` vacío. Se creó durante la corrida de `cargo test --workspace`. Tiene la forma de los que encontró la verificación de A1: un store en disco de alguna prueba que no lo suelta.
  - Las pruebas de refresco de A2 usan `MemoryStore::new_for_test`, en memoria, y la de `eval_api` usa `test_dir()` y `close`. Así que no parece de este cambio.
  - Se borró, porque lo creó esta verificación. Las demás entradas nuevas del temporal (nombres UUID `.tmp`, fuentes, video, un log) son de otros programas.
  - `docs/eval/cache/` ya tenía un `docs-vectors-*.bin` de antes. No se tocó: es la Parte 7f.
- **`docs/reference/daemon.md`** describe la cola, el tope, el límite que crece, la escritura en el orden del plan, los fallos y el reparto del tiempo. Tiene dos detalles: WARNING 1 y SUGGESTION 5.

### Lo que encontró la verificación

**CRITICAL:** ninguno.

**WARNING:**

1. **Un doc de más pasajes que el límite no siempre va solo.** IS8, D8, el comentario de `QUEUE_PASSAGES` (`mod.rs:562-563`) y `daemon.md` dicen que va solo.
   - **Qué hace el código.** `Queue::admit` (`:597-611`) vacía antes, y con eso duplica el límite. Después compara al doc con el límite nuevo. Con límite 16 y 3 pasajes en cola, un doc de 20 vacía los 3. Queda solo con límite 32, no llega, y el doc siguiente se le suma.
   - **Dónde pasa.** Sólo durante la rampa, con la cola no vacía, para docs de entre límite + 1 y 2 × límite − 1 pasajes. En el tope el límite ya no crece, así que un doc de más de 512 siempre va solo. Lo de IS8 «el tope se respeta» vale siempre, porque cada doc que se suma cabe en un límite que no pasa del tope.
   - **Qué no cambia.** La cuenta de IS6 y `queue_groups` usan la misma pieza, así que la tabla, la evaluación y el refresco coinciden.
   - **Arreglo:** corregir el texto, que es lo más barato y deja válida la tabla de IS6: «un doc con más pasajes que el límite que rige después del vaciado anterior va solo», en IS8, D8, `:562-563` y `daemon.md`. La alternativa es que `admit` recuerde `passages > limit()` antes de vaciar y vacíe también después. Eso cambia levemente los grupos de la rampa que midió la tabla.
   - **Cerrado en `a0ec285` (código y `daemon.md`) y en el commit de openspec que cierra esta lista (spec IS8, diseño D8).** Se corrigió el texto, no el código. La rampa la fija `a_doc_between_the_limit_and_twice_it_joins_the_docs_after_it_while_the_limit_grows`, que no se mutó: fija la regla que ya cumplía el código (`[[0], [1, 2]]` con límite 16 y docs de 3, 20 y 5).
2. **Ninguna prueba de refresco llega al tope** (M2 y M8). En `no_call_holds_…` (`:2982`), los 600 pasajes reparten las llamadas en 30, 30, 60, 120, 240 y 120: el límite no llega a 512, y la prueba pasa aunque `refresh` arme la cola sin tope. Sólo las pruebas puras, con tope 64 o 16, ven el tope, y el valor 512 lo sostiene sólo `assert!(QUEUE_PASSAGES < 600)`. **Arreglo:** que los docs sumen bastante más que 16 + 32 + … + 512 = 1008 (por ejemplo, 60 docs de 30 pasajes) y que la prueba afirme también que alguna llamada pasa de `QUEUE_PASSAGES / 2`. Con el tope, las llamadas llegan a 510. Con M8, la última llevaría 810 pasajes de 27 docs, y la prueba fallaría.
   - **Cerrado en `5357e85`.** `no_call_holds_…` usa 60 docs de 30 pasajes y afirma que alguna llamada pasa de `QUEUE_PASSAGES / 2`. M8 reaplicada: falla `no_call_holds_more_passages_than_the_cap_unless_it_is_one_doc`.
3. **Un vector inutilizable no debe correr los vectores del doc siguiente** (M14). Si el doc con el vector malo tiene más textos después de él, cortar la colección en el primer error deja el resto al doc que sigue. Ese doc se escribiría con vectores ajenos, sin error. Hoy el código lo evita (`:846-849`), y el comentario lo dice, pero `an_unusable_vector_in_a_shared_call_…` sólo mira rutas y cuentas. **Arreglo:** que esa prueba compare el blob de `c.md` con el de `FakeEmbedder` para su texto, como `each_passage_…` (`:3030-3033`). El vector malo de `b.md` va primero en su doc, así que con M14 fallaría.
   - **Cerrado en `5357e85`.** `an_unusable_vector_in_a_shared_call_…` compara el blob de cada pasaje guardado con el de `FakeEmbedder` para su texto. M14 reaplicada: falla esa prueba.
4. **Que los pasajes reutilizados cuenten contra el límite no lo sostiene ninguna prueba** (M20; IS7, D8 «qué cuenta el tope»). Es la regla que acota la memoria de un refresco incremental que reutiliza casi todo. **Arreglo:** una prueba que indexe dos docs de 16 pasajes y después cambie uno de cada uno. El segundo refresco tiene que hacer dos llamadas de un texto, porque cada doc llena el límite de 16 con sus pasajes. Contando sólo los textos, haría una de dos.
   - **Cerrado en `5357e85`.** `the_passages_a_doc_reuses_count_against_the_limit` espera dos llamadas de un texto. M20 reaplicada: falla esa prueba.
5. **La cota H de D13 no acota lo que retiene un vaciado.** Es por inspección, sin medir. H = Σ (3 × bytes del texto + 1536) deja fuera dos cosas:
   - **Las codificaciones del tokenizer.** `passage_order` cuenta toda la llamada de una vez. `passage_token_counts` → `window_encodings` (`crates/clawg-core/src/memory/embedder.rs:249-279`) arma un `tokenizers::Encoding` por texto, y todos coexisten. Cada uno guarda por token ids, type_ids, el token como `String`, words, offsets y dos máscaras: unos 70 a 80 B por token. Con 512 pasajes de unos cientos de tokens son del orden de 10 MiB, varias veces los 2,2 MiB de H. En el peor caso, 512 pasajes de 512 tokens, unos 20 MB, más que los 16 MiB de D13. A1 ya hacía lo mismo con el doc más grande (278 pasajes en este corpus). A2 lo lleva al mayor entre 512 y ese doc.
   - **Los vectores en dos copias.** En `embed_passages` los vectores f32 viven hasta el final, mientras se arman los blobs (`:836-851`): son 3072 B por texto, no 1536.

   IS17 mide el pico real y sigue siendo la puerta. **Arreglo:** contar en tandas de `EMBED_BATCH` dentro de `passage_token_counts`, guardando sólo los largos, como ya hace `lengths_by_doc` (`eval_api.rs:1426-1433`), y recorrer `vectors` con `into_iter` al armar los blobs. Así H vuelve a ser una cota.
   - **Cerrado en `d003faf`.** `passage_token_counts` cuenta en tandas de `EMBED_BATCH` y `embed_passages` consume los vectores con `into_iter`. No cambia nada observable: las pruebas existentes (96 de `project_docs`, entre ellas las de largos idénticos) siguen en verde. D13 dice lo que queda fuera de H (unos 640 KB por tanda de codificaciones, y el blob en armado).

**SUGGESTION:**

1. **El `.min(16)` de `Queue::limit` (`:589`) no lo fija ninguna prueba** (M7). Basta una prueba pura con más de 64 vaciados, por ejemplo `queue_groups(QUEUE_PASSAGES, &[QUEUE_PASSAGES; 80])`: tiene que dar 80 grupos de uno, sin pánico.
   - **Cerrado en `38f38ac`** (`the_limit_stays_at_the_cap_through_many_flushes`). M7 reaplicada: falla.
2. **`VectorCount.passages` = textos del vaciado** (IS12, MUST) no se afirma (M19). Basta afirmar `passages: 2` en `a_short_batch_still_aborts_the_refresh` (`:1496`), cuyos dos docs comparten la llamada.
   - **Cerrado en `38f38ac`.** M19 reaplicada: falla `a_short_batch_still_aborts_the_refresh`.
3. **«El resto al último doc con textos» no se prueba** (M24). Ningún caso de `embed_shares_split_…` termina en un doc sin textos con resto. Basta un caso como `embed_shares(ns(10), &[1, 1, 1, 0])`, que tiene que dar `[3, 3, 4, 0]`.
   - **Cerrado en `38f38ac`.** M24 reaplicada: falla `embed_shares_split_the_time_by_texts_and_add_up_to_it`.
4. **`queue_groups_are_the_calls_…` no distingue 512 de 256 ni de 1024** (M25). Sus 33 pasajes no pasan del límite 32. Con el arreglo del WARNING 2 aplicado a esa prueba (un árbol que llegue al tope), la igualdad con las llamadas de `refresh` fijaría también que la evaluación usa el tope de producción.
   - **No se hizo:** pide armar en `eval_api` un árbol que llegue al tope y compararlo con las llamadas de `refresh`; no es barato, y M25 (la evaluación con otro tope) cambia sólo lo que mide la Parte 7.
5. **`daemon.md`, una frase vieja.** «With that many or fewer, without a tokenizer, or when counting fails, they go in the passages' order» quedó repetida después de la frase de los fallos. Su versión nueva («…in the order of the plan») ya está antes. Hay que borrar la vieja.
   - **Cerrado en `a0ec285`.**
6. **El comentario de `Tokens::padded`** (`index_cost.rs:106-107`) dice «what the model would run if a refresh did not sort them». Con la cola, un refresco sin tokenizer junta docs en una llamada, así que eso ya no es `padded`, que sigue siendo doc por doc. Basta decir «doc by doc, in the order of each doc's passages, as a refresh ran before it sorted or queued».
   - **Cerrado en `38f38ac`** (con el texto «what a refresh ran before it sorted or queued them»).

### Resultado de la suite y de clippy

Sobre `f3de23f` (`HEAD`), con el árbol limpio, después de revertir M1 a M30:

- `cargo test --workspace --no-fail-fast --target-dir target-test`: pasa (salida 0). Las ignoradas son las de antes más `padding_and_batches_by_queue_cap`.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: salida 0.
- `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings`: salida 0.
- M20 se aplicó y se revirtió después de esa corrida. Al terminar, `git status` sólo muestra este archivo y `git diff HEAD -- crates` no da nada, así que la suite corrió sobre los mismos bytes que quedan.
- Sólo quedan `target` y `target-test`. No se volvió a medir ninguna métrica de `rag-eval`: es la Parte 7.

### Veredicto

**PASS WITH WARNINGS.** La suite y los dos clippy pasan. De las 30 mutaciones muerden 22, y cada prueba nueva de refresco muerde al menos una. Muerden el límite que crece y su tope en las pruebas puras, las tres causas de vaciado, el doc grande, el orden sobre la llamada entera, la vuelta de cada vector a su doc y a su pasaje, el vector inutilizable que omite sólo su doc, el error del embedder (qué queda escrito, qué se vuelve a planear y a quién nombra), el orden de escritura, el reparto del tiempo y la evaluación (los grupos, el campo, el aviso y su mensaje).

No muerden ocho:
- una equivalente (M6);
- tres que dejan sin prueba reglas de la spec: el tope en un refresco real (M8), los vectores del doc que sigue a uno inutilizable (M14) y los pasajes reutilizados que cuentan (M20), que son los WARNING 2 a 4;
- cuatro de detalle (M7, M19, M24 y M25), que son SUGGESTION.

13 de los 15 escenarios de producción cumplen enteros y 2 en parte.

El código no lo contradice ningún escenario, pero hay dos puntos donde el texto y el código difieren. Un doc de más pasajes que el límite se suma a los siguientes durante la rampa (WARNING 1, que se arregla en el texto). La cota de memoria de D13 no cuenta las codificaciones del tokenizer ni la doble copia de los vectores (WARNING 5), y así el pico de IS17 es la única puerta de memoria.

IS16 e IS17 esperan a la Parte 7. Ningún hallazgo cambia lo que el refresco escribe ni un número de la tabla de IS6.

## A2: «se queda si» (2026-11-01)

Corridas autorizadas de la Parte 7, una por vez y sin otra compilación ni prueba en marcha. Usan el modelo del perfil dev (`multilingual-e5-small`), sólo leído, en esta máquina: Windows `x86_64` con AVX2, AVX-512F y FMA, 16 hilos. Durante todas corrió también el daemon `prod` del editor, que ninguna corrida registra.

- **Base:** `2f92e17`, el commit anterior al código de A2, que ya incluye A1 (0.72.0). Se compiló en release en un worktree descartable, con su propio directorio de build.
- **Paso:** `30b549f` (0.72.2), compilado en `target-test` después de `cargo clean --workspace`, porque la versión subió. Entre la base y el paso, el daemon sólo cambia en `crates/clawg-core/src/memory/project_docs/mod.rs` y `embedder.rs`. Lo demás que entró (agent-eval y las subidas de versión) no toca el índice de docs.
- **Corpus:** congelado en un worktree de `30b549f`. Tiene 787 docs, 19 278 pasajes, 2 saltados y 1 dejado fuera (`openspec/changes/archive/2026-10-15-project-indexing/verify.md`), con el golden de 40 preguntas.
  - Las dos ramas lo leen con `--root`, `--golden` y `--out` explícitos, y se llaman los binarios ya compilados.
  - En los JSON, `git_sha` es el del corpus y no el del binario.
  - El checkout del worktree tiene fin de línea CRLF. Por eso sus tokens no se comparan con los de A1.

### Los bits (7b)

`sorting_across_docs_against_sorting_in_each_doc` con el tercer orden (`bb6a565`) corrió en debug, con `CLAWG_EMBED_PASSAGES=5000` y `CLAWG_EMBED_RUNS=3`.

- **Muestra:** 197 docs y 5293 pasajes, de los que 77 docs tienen más de 16 pasajes. La cola los manda en 16 llamadas.
- **Bits:** la cola da **5293 de 5293 vectores con los mismos bits** que ordenando en cada doc, con diferencia mayor 0. Todo junto da lo mismo.
- **Tokens rellenados:** 2 003 961 en cada doc, 1 664 643 en la cola (0,831) y 1 620 417 todo junto (0,809).
- **Tiempo:** los tres órdenes se turnan en cada ronda. Contra «en cada doc», la cola da 0,798, 0,702 y 0,905 (mediana 0,798), y todo junto 0,791, 0,674 y 0,701. Es vista previa en debug y no decide nada.

La decisión 4 se cumple con diferencia 0 en esta CPU. Nada guardado cambia, y `docs/eval/cache/` no se vacía.

### El tiempo (7c)

Seis corridas de `rag-eval --real --docs --no-cache` en release, con `CLAWG_PROFILE=dev`, alternando base, paso, base, paso, base y paso. Los resultados, fuera de git, están en `docs/eval/results/2026-11-01-real-docs-a2-{base,step}-{1,2,3}.json`.

| corrida | empieza | `index_seconds` | `phases.embed` | `phases.split` | `tokens.padded_ran` |
|---|---|---|---|---|---|
| base-1 | 23:25 (31/10) | 1601,6 | 1585,43 | 11,99 | 7 435 637 |
| paso-1 | 23:52 | 1187,6 | 1172,47 | 11,24 | 6 114 552 |
| base-2 | 00:12 | 1580,4 | 1562,66 | 12,98 | 7 435 637 |
| paso-2 | 00:38 | 1184,3 | 1169,48 | 10,85 | 6 114 552 |
| base-3 | 00:58 | 1569,4 | 1552,32 | 12,74 | 7 435 637 |
| paso-3 | 01:24 | 1175,3 | 1161,04 | 10,70 | 6 114 552 |

**Las otras fases** son chicas y parecidas en las seis: `scan` 0,05–0,06, `read` 0,21–0,29, `mask` 0,82–0,95, `lookup` 0,01–0,02 y `write` 2,47–3,41 s. `count_seconds` va de 1,96 a 2,00 s.

**Los tokens** son iguales en las seis: `real` 5 976 339, `padded` 8 944 685, `padded_sorted_in_doc` 7 435 637 y `padded_sorted_all` 5 979 576. El paso agrega `padded_sorted_queue` 6 114 552. En la base, `padded_ran` = `padded_sorted_in_doc`, como dejó A1.

**La cola recupera 0,9073 del ahorro de tokens del techo:** (7 435 637 − 6 114 552) / (7 435 637 − 5 979 576). La cuenta del tope, sin modelo y sobre el corpus de `478f59f`, había dado R_P(512) = 0,9071.

**Esta vez la máquina no cambió de régimen.** Dentro de cada rama, `embed` varía menos del 2 %. `split` es más bajo en el paso (10,7–11,2 s frente a 12,0–13,0 s) en las tres corridas, no en una sola. Esa diferencia sería propia de la rama y no de la máquina, aunque no está medido por qué. Una explicación probable: el paso parte varios docs seguidos antes de cada llamada al modelo, y la base parte uno por llamada. Por eso `split` no sirve acá para comparar entre ramas, y se comparan los tres pares y las vecinas:

| comparación | `phases.embed` |
|---|---|
| par 1 (base-1 → paso-1) | **−26,1 %** |
| par 2 (base-2 → paso-2) | **−25,2 %** |
| par 3 (base-3 → paso-3) | **−25,2 %** |
| paso-1 contra el promedio de base-1 y base-2 | −25,5 % |
| paso-2 contra el promedio de base-2 y base-3 | −24,9 % |

**Por área (`by_area`).** Los conteos son iguales en las seis corridas: docs 68 docs, 2903 pasajes y 45 de más de una tanda; code 211, 8485 y 109; archive 508, 7890 y 150. En el paso, el tiempo de una llamada compartida se reparte entre sus docs por textos (`embed_shares`). Por eso el `embed` por área del paso es un reparto, no una medición por doc.

| área | par 1 | par 2 | par 3 |
|---|---|---|---|
| docs | 213,51 → 158,31 s (−25,9 %) | 205,46 → 158,52 s (−22,8 %) | 202,75 → 156,57 s (−22,8 %) |
| code | 671,30 → 584,92 s (−12,9 %) | 677,64 → 583,45 s (−13,9 %) | 673,85 → 577,29 s (−14,3 %) |
| archive | 700,62 → 429,24 s (−38,7 %) | 679,56 → 427,51 s (−37,1 %) | 675,73 → 427,18 s (−36,8 %) |

**Dónde gana.** Gana más en `archive`, que tiene muchos docs chicos. Antes, cada uno corría su propia tanda incompleta, y en la cola comparten tandas. En `code` gana menos, porque sus pedazos tienen un tope de caracteres y ya rellenaban poco.

**Por qué el reloj baja más que los tokens.** Los tokens rellenados bajan un 17,8 % y el reloj un 25 %. Pasa porque la cola, además de rellenar menos, llena las tandas: el modelo corre menos veces, y cada corrida tiene un costo que no depende del relleno. En A1 era al revés: tokens −16,9 % y reloj −12,7 %.

**Por qué supera el techo de debug.** El techo de debug había dado −21,0 % («A2: el techo, antes del código»). La explicación más probable no está medida. En debug, la parte en Rust (tokenizar y armar los tensores) es lenta y no se achica con la cola, así que diluye la mejora. En release pesa poco.

### El pico de memoria (7d)

Cuatro daemons `clawg serve --profile dev` de release, uno por corrida, alternando base, paso, base y paso.

- **Preparación.** Antes de la primera corrida hubo un arranque del paso sin proyecto. Aplicó las migraciones V25 a V31 al store dev, que estaba en la 24, y escribió su snapshot `pre-migration`. Sin ese arranque, la migración habría caído dentro de base-1.
- **Las copias del corpus.** Cada daemon indexó una copia propia del worktree congelado, sin `.git` y en una ruta nueva: 968 archivos idénticos, registrados con `clawg projects add`. Un worktree resuelve a su repositorio principal, y el índice de docs va por raíz. Con la misma ruta, el segundo daemon habría reutilizado los vectores del primero.
- **El store dev** tenía 51 998 memorias vivas.
- **La lectura.** `clawg metrics report --metric process.private_peak_bytes` y `process.private_bytes`, `--by minute`, tomando de cada daemon sólo los minutos en que muestreó él solo.
- **El tiempo de más.** Tres de los daemons siguieron vivos después de su refresco más de lo previsto: base-1 y paso-1 unas 2 h, y base-2 media hora. El script que esperaba el fin del refresco no encontraba `rg`, una función de la shell que un `bash` hijo no hereda, y esperó hasta su tope. Eso no cambia el pico. En las cuatro corridas, el pico al terminar el refresco es el mismo que el de toda la vida del proceso. Después del refresco el modelo se descarga a los 10 minutos sin uso, y `private` baja a unos 320 MiB.

| corrida | empieza | refresco del daemon | `embed` del daemon | pico, primer minuto | pico de vida del proceso | `private` máx. en el refresco |
|---|---|---|---|---|---|---|
| base-1 | 01:49 | 1539,0 s | 1521,0 s | 1188,5 MiB | **1448,5 MiB** | 1349,9 MiB |
| paso-1 | 04:22 | 868,4 s | 854,0 s | 1189,0 MiB | **1312,6 MiB** | 1193,8 MiB |
| base-2 | 06:55 | 1170,7 s | 1154,2 s | 1188,3 MiB | **1448,5 MiB** | 1399,1 MiB |
| paso-2 | 07:46 | 926,3 s | 911,0 s | 1300,1 MiB | **1312,6 MiB** | 1176,9 MiB |

**El pico lo fija el refresco en las dos ramas.** Arranca en unos 1188 MiB y crece mientras indexa, así que vale `private_peak_bytes` y no hace falta el máximo de `private_bytes`. Las dos corridas de cada rama dan el mismo pico, sin diferencia de un MiB. **El paso tiene un pico un 9,4 % más bajo que la base** (1312,6 frente a 1448,5 MiB).

**Por qué baja el pico.** La explicación más probable no está medida por separado. La base cuenta los tokens de todos los textos de un doc de una vez, con todas sus codificaciones en memoria. El paso los cuenta de a 16 y consume los vectores con `into_iter` (`d003faf`, el WARNING 5 de la verificación del código).

**Los tiempos del daemon son informativos.** Van en la misma dirección que la 7c: paso-1 contra el promedio de las dos bases −36,2 %, y paso-2 contra base-2 −21,1 %. No deciden nada, porque las dos bases difieren un 24 % entre sí.

### Las condiciones de «se queda si»

| condición | resultado | |
|---|---|---|
| `phases.embed` baja ≥ 15 % | −26,1 %, −25,2 % y −25,2 % en los tres pares; −25,5 % y −24,9 % contra las vecinas | ✅ |
| `tokens.padded_ran` = `tokens.padded_sorted_queue`, sin aviso, en cada corrida del paso | 6 114 552 = 6 114 552 en las tres. `padding_warning` sólo avisa si difieren | ✅ |
| `--diff` base-n contra paso-n no mueve ninguna pregunta | en los tres pares: 0 mejores y 0 peores en Hit@1, Hit@3, Hit@5 y MRR@10, en todas, docs y código (p = 1). Las seis corridas dan Hit@1 0,05, Hit@3 0,10, Hit@5 0,10 y MRR@10 0,079 | ✅ |
| decisión 4 de la propuesta | 0 bits movidos (7b) | ✅ |
| IS17: el pico sube ≤ 5 % | baja un 9,4 % (1312,6 frente a 1448,5 MiB, en las dos corridas de cada rama) | ✅ |

**A2 se queda.** IS16 e IS17 quedan cumplidos: IS16 por la 7b e IS17 por la 7d. No se revierte ningún commit. Como no se movió ningún bit, `docs/eval/cache/` sigue siendo exacta.

## Archivo (2026-11-01)

El cambio se archiva con A1 y A2 adoptados, y sale en 0.73.0. El comportamiento (IS1, IS2 e IS7 a IS14) pasó a [`specs/project-docs`](../../../specs/project-docs/spec.md), «Refresco incremental» y «La cola del refresco»; IS3 e IS15, a [`specs/eval`](../../../specs/eval/spec.md), IC6, con lo medido en «Índice de docs»; IS4, a [`specs/storage`](../../../specs/storage/spec.md), «Los mismos vectores que fastembed». IS5, IS6, IS16 e IS17 son requisitos de medición y quedan sólo en este archivo, con las puertas y sus resultados. El test del caso de `e5_golden` de IS4 es `a_batch_longest_first_stays_within_the_tolerance_of_the_batch_in_order`, no el que nombraba la spec. El paso E sigue en espera.
