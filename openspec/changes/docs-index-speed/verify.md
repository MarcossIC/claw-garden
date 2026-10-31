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
