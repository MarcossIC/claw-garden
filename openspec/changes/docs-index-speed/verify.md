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
