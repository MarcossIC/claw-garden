# long-save-ranking — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | long-save-context (vínculo, colapso, `read_memory`), eval-golden-expansion (subconjunto `long`) | ver [`state.yaml`](state.yaml) |

## Problema

El pedazo que guarda la respuesta de un save largo pierde contra filas cortas del mismo tema. Detalle y medición en la [exploración](exploration.md#medición-2026-10-15-linux-modelo-real):

1. **La respuesta se diluye en el vector.** Un save que no entra en 480 tokens se parte en pedazos de 424 (`crates/clawg-core/src/memory/chunker/mod.rs:37`; `chunker/chat.rs:88-94`), y el vector de pooling mean promedia el pedazo entero. Con el modelo real, la frase con la respuesta sola, detrás de la cabecera del save, supera al primer resultado en los 12 casos `long`; el pedazo que la contiene queda en el puesto 20, 26 y 43 en tres de ellos.
2. **La respuesta no se ve en `search_memory`.** Muestra los primeros 500 caracteres de cada hit (`crates/clawg-cli/src/mcp/tools.rs:29`), y la respuesta está a mitad de un pedazo de 424 tokens: `search_visible` es 0,00 en los 12 casos.
3. **El pozo de KNN cuenta filas, no saves.** La búsqueda junta 30 vecinos de `memory` (`crates/clawg-core/src/memory/search.rs:39`) y recién después deja un pedazo por save (`search.rs:1181`). Un save con más pedazos parecidos que el pozo lo llena solo y deja afuera otras memorias. Hoy hace falta un save de unos 11.000 tokens; con pedazos más chicos, mucho menos.

U2 de long-save-context (0,333 contra 0,75) se midió antes del pooling mean y del corte de relevancia de 0,35; sobre `main` y el set sustituto da 0,75. Hay que medirlo de nuevo sobre el set real (U0).

## Decisión

**Pedazos chicos (a de la exploración).** Tarjeta en el hilo, opción recomendada; mientras no se elija otra, se sigue con ésta.

- **¿Por qué no ventanas aparte (b)?** Daría el mismo ranking con una tabla nueva, migración y re-embed de ventanas. a lo consigue con dos constantes y deja la vista previa con el pedazo que ganó.
- **¿Por qué no un puntaje por save (c)?** Los hermanos no tienen la respuesta; subir el save no hace ganar al pedazo correcto.
- **Tope y solapamiento:** 96 tokens con 24 de solapamiento, el mejor del barrido (12 de 12 hallados, respuesta visible en `search_memory` en 11). 128/32 perdió un caso.
- **Saves ya guardados:** no se vuelven a partir. Reescribir filas guardadas por una ganancia de ranking no vale el riesgo; un save que se guarda de nuevo queda partido con el tope nuevo y sus pedazos viejos siguen como estaban.

## Alcance

1. **Tope de los pedazos** (`chunker`): un texto que no entra en un pedazo de 480 tokens se parte en pedazos de 96 con 24 de solapamiento. Un texto que entra en 480 no cambia.
2. **El pozo cuenta saves** (`search.rs`): la consulta KNN de `memory` amplía su ventana cuando los pedazos hermanos le dejan menos saves distintos que el pozo pedido.
3. **La medición no depende del tope** (`golden-mine synth`): las filas `long` se generan con el largo de hoy, medido contra el tope de 424, así base y punta miden el mismo set.

## Fuera de alcance

- **Volver a partir saves guardados** (ver arriba).
- **`list_memories` y el workspace** siguen mostrando una fila por pedazo; un save largo ocupa más filas que hoy. Colapsarlos es otro cambio.
- **Textos de un pedazo de hasta 480 tokens.** También diluyen una frase, pero no los mide ningún caso y el ancla de 46 casos no se mueve.
- **Claves con guion como identificadores léxicos** (`is_identifier`, `search.rs:712-717`): cambia todo el canal léxico.
- **El corte de 500 caracteres** de `search_memory`.

## Datos guardados

No cambia el esquema ni ningún dato guardado. Las filas, vínculos y vectores de saves partidos con 424 se siguen buscando, colapsando, leyendo con `read_memory` y re-embebiendo como hoy (R2 de la [spec](spec.md)). Rollback: el binario anterior lee la base sin cambios; los saves partidos con 96 quedan como pedazos más chicos, ya vinculados, que la versión anterior también colapsa y lee.

## Éxito

**Tests que fallan hoy** (detalle en la [spec](spec.md)):

- un save largo se parte en pedazos de a lo sumo 96 tokens con 24 de solapamiento, y uno que entra en 480 no cambia;
- un save con más pedazos que el pozo deja lugar a otras memorias en el resultado;
- las filas `long` de `golden-mine synth` no cambian al bajar el tope.

**`rag-eval`, con umbrales fijados antes de medir** (golden set local, `docs/eval/golden.json`, fuera de git):

| # | Condición | Corrida |
|---|---|---|
| U0 | registrar `long` de `main`: hit rate, `search_reachable`, `search_visible`, `context_visible` | `rag-eval --real --agent-view --golden <synth seed 7>` con el binario de `main` |
| U1 | ancla: 0 peores en las cinco métricas | `rag-eval --real` y `--diff` contra la base |
| U2 | `long`: `search_reachable` ≥ 0,917 (11 de 12), `search_visible` ≥ 0,5 y `context_visible` ≥ 0,75 | agent-view de la punta sobre el mismo set |
| U3 | los otros cinco subconjuntos: ninguna métrica de agent-view baja más de 0,02; `repeated_rows` = 0 | idem |
