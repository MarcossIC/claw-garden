# docs-index-speed — Verificación

## Las puertas, antes del código (2026-10-31)

Cada puerta lee los campos de las corridas D y R de docs-index-cost, con los umbrales de la [propuesta](proposal.md) («Pasos con puerta»), fijados antes de esas corridas.

- **Corridas:** sobre `0f09b2b`, una detrás de otra, con la misma `index.machine`: `windows`, `x86_64`, avx2, avx512f y fma, 16 hilos. D es debug con la memoria vacía y R es release con `--no-cache`.
- **Archivos (fuera de git):** `docs/eval/results/2026-10-31-real-docs-cost-d.json` y `docs/eval/results/2026-10-31-real-docs-cost-r.json`.
- **Campos:** los copia la [verificación de docs-index-cost](../archive/2026-10-31-docs-index-cost/verify.md), en «Lo que dio cada corrida». Los cocientes de abajo se recalcularon con esos campos y coinciden con los de su tabla «Los cocientes que leen las puertas de `docs-index-speed`».

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
