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
