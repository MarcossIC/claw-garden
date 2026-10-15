# long-save-ranking — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | long-save-context (vínculo y `read_memory`), eval-golden-expansion (subconjunto `long`) | cerrada: sigue en la [propuesta](proposal.md) |

**Tesis:** el pedazo que guarda la respuesta de un save largo pierde contra filas cortas del mismo tema porque su vector promedia 424 tokens donde la respuesta es una frase. No es un problema del puntaje sino de la unidad que se embebe: la frase sola, con la cabecera del save, sale primera en los 12 casos medidos.

## De dónde sale

U2 de long-save-context no se cumplió: la respuesta de un save `long` era alcanzable en 0,333 de los casos, contra un umbral de 0,75 ([verify](../archive/2026-10-14-long-save-context/verify.md#u2-u3-y-u4)). Cuando el save aparecía, lo hacía en el puesto 7 a 9; en 8 de 12 casos quedaba fuera de los 10.

**Esa medición ya no describe `main`.** Se hizo con vectores de pooling CLS y `RELEVANCE_RATIO` 0,6. Después, project-indexing pasó el embedder a pooling mean, re-embebió toda la base, bajó el piso del coseno a 0,79 (`crates/clawg-core/src/memory/score.rs:161-182`) y el corte de relevancia a 0,35 (`crates/clawg-cli/src/relevance.rs:19`). U2 hay que volver a medirlo sobre `main` antes de dar por bueno cualquier número.

## Estado actual (verificado en `9aa1012`)

- **Cómo se parte.** Un texto que no entra en un pedazo de 480 tokens se parte en pedazos de `PIECE_MAX_TOKENS` = 424 (`crates/clawg-core/src/memory/chunker/mod.rs:37`), con 64 de solapamiento (`chunker/mod.rs:21`), en `ChatChunker::split_save` (`chunker/chat.rs:88-94`). Desde el segundo, cada pedazo se embebe detrás de los primeros 48 tokens del save (`chunker/mod.rs:29`).
- **Cómo se puntúa.** Cada fila compite sola: `score = coseno reescalado (+ bono léxico) × decaimiento × importancia × estado` (`score.rs:264-305`). Nada en la fórmula sabe que la fila es un pedazo.
- **Qué entra al ranking.** La búsqueda junta 30 vecinos de `memory` por KNN (`crates/clawg-core/src/memory/search.rs:39`) y 20 candidatos léxicos (`search.rs:43`), ordena, deja el mejor pedazo de cada save (`collapse_saves`, `search.rs:1181`) y corta en `k` (`search.rs:1187`). `search_memory` además descarta lo que puntúa debajo de 0,35 del primero (`relevant_len`, `crates/clawg-cli/src/relevance.rs:25-34`).
- **Qué ve el agente.** `search_memory` muestra los primeros 500 caracteres de cada hit (`CONTENT_PREVIEW_LIMIT`, `crates/clawg-cli/src/mcp/tools.rs:29`). Una respuesta en la mitad de un pedazo de 424 tokens no entra nunca en esos 500 caracteres: hay que pedir `read_memory`. `recall_context` muestra el pedazo entero.
- **Lo que mide el eval.** Los casos `long` de `golden-mine synth` (`crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:417-460`) nombran la clave (`scarlet-stoat`) sólo en la primera oración y entierran el hecho a dos tercios del texto (`synth/templates.rs:259-279`). La pregunta nombra la clave y el tema del hecho. Las filas que compiten son las de `multi_ref`, `high_importance` y `low_importance`: una o dos oraciones sobre el mismo tema ("define la política de reintentos", "expiran a los 30 segundos") con otra clave del mismo estilo (`scarlet-heron`).

## Medición (2026-10-15, Linux, modelo real)

**El `golden.json` de Marcos no está en el repo** (`.gitignore`: `docs/**/golden.json`). Se armó una base sustituta de 20 filas en inglés y español con 4 casos, y sobre ella `golden-mine synth --seed 7` (1.012 filas, 82 casos, 12 `long`). Los números son de ese set: sirven para ver el mecanismo, no reemplazan la medición sobre el set real. La base, el programa de diagnóstico y las salidas quedan en la carpeta del proyecto, `long-save-ranking/` (`stand-in-base.json`, `lsr_diag.rs`, `diag-main.txt`, `sweep.txt`).

### `main` tal cual

`rag-eval --real --agent-view --golden <set>`:

| `long` | Valor |
|---|---|
| `search.hit_rate` / `context.hit_rate` | 0,75 / 0,75 |
| `answers.search_reachable` | 0,75 |
| `answers.search_visible` | 0,00 |
| `answers.context_visible` | 0,583 |

Los otros cinco subconjuntos dan 1,0 en búsqueda y contexto.

El diagnóstico (búsqueda con `k = 50`, coseno de cada pedazo y de la frase con la respuesta) muestra dónde se pierde:

| Caso | Puesto del save | Pedazo que gana | Coseno del pedazo con la respuesta | Coseno de cabecera + frase | Coseno del 1.º |
|---|---|---|---|---|---|
| s-long-00 | 1 | el de la respuesta | 0,826 | 0,853 | 0,826 |
| s-long-01 | 8 | otro (3/3) | 0,845 | 0,888 | 0,866 |
| s-long-02 | 5 | el de la respuesta | 0,834 | 0,879 | 0,848 |
| s-long-03 | **20** | el de la respuesta | 0,845 | 0,895 | 0,875 |
| s-long-04 | **43** | el de la respuesta | 0,784 | 0,836 | 0,820 |
| s-long-05 | **26** | el de la respuesta | 0,802 | 0,877 | 0,819 |
| s-long-06 a 09 | 1 | el de la respuesta | 0,820–0,866 | 0,875–0,922 | el mismo |
| s-long-10 | 4 | el de la respuesta | 0,810 | 0,833 | 0,817 |
| s-long-11 | 1 | otro (3/3) | 0,841 | 0,888 | 0,843 |

- **La cabecera hace su parte:** el pedazo con la respuesta es casi siempre el mejor de su save, y la clave le llega por la cabecera.
- **Lo que falla es la dilución.** El pedazo tiene 424 tokens de relleno y una frase con la respuesta; el vector mean promedia todo. La misma frase, sola con la cabecera, supera al primer resultado en los 12 casos, por 0,01 a 0,06 de coseno.
- **Por qué ganan las filas cortas:** son una o dos oraciones sobre el mismo tema, así que su vector es casi todo tema. Comparten además la segunda mitad de la clave (`violet-lynx` contra `ivory-lynx`).
- **En 2 de 12 casos el pedazo que se muestra no tiene la respuesta** (gana el 3/3), y en `search_memory` la respuesta no se ve en ningún caso por el corte de 500 caracteres.

### Prototipo: pedazos más chicos

Sólo cambiando el tope y el solapamiento de los pedazos de un save largo (prototipo descartado, no está en la rama):

| Tope / solapamiento | Filas guardadas | `long` hallado | alcanzable | visible en `search_memory` | visible en contexto | Otros subconjuntos |
|---|---|---|---|---|---|---|
| 424 / 64 (`main`) | 1.035 | 0,75 | 0,75 | 0,00 | 0,583 | 1,0 |
| 192 / 64 | 1.095 | 0,917 | 0,917 | 0,333 | 0,583 | 1,0 |
| 128 / 32 | 1.131 | 0,917 | 0,917 | 0,750 | 0,917 | 1,0 |
| 128 / 64 | 1.189 | 1,0 | 1,0 | 0,750 | 0,833 | 1,0 |
| 96 / 24 | 1.175 | 1,0 | 1,0 | 0,917 | 0,917 | 1,0 |

- Con 96 o 128 tokens aparecen los 12 saves, y la respuesta se ve en `search_memory` sin pedir `read_memory` en 9 a 11 casos, porque el pedazo entero entra en los 500 caracteres.
- Los otros subconjuntos no se mueven: sus filas entran en un pedazo de 480 y no se parten distinto (R1 de [`specs/pieces`](../../specs/pieces/spec.md)).
- Tokens medios de `search_memory` y `recall_context`: iguales o un poco menores (358–364 y 4.042–4.093 contra 366 y 4.178).
- El costo son filas: un save de unos 1.200 tokens pasa de 3 a unas 13 filas con 96/24.

## Enfoques

| Opción | Qué hace | A favor | En contra |
|---|---|---|---|
| a. Pedazos chicos | Baja el tope de los pedazos de un save largo (96 o 128 tokens, con solapamiento proporcional) | Una constante y un test; sin esquema ni migración. Medido: 12 de 12. El pedazo ganador entra en la vista previa de `search_memory` | Unas 4 veces más filas por save largo, y se ven en `list_memories` y, si es de tier 1, en el workspace (`crates/clawg-core/src/memory/workspace.rs:255-292`), que no colapsan pedazos. Los saves ya guardados no cambian |
| b. Ventanas aparte | Las filas quedan como están; una tabla nueva (V12) guarda vectores de ventanas chicas de cada pedazo, y la distancia de la fila es la de su mejor ventana | Filas, listados y workspace iguales. Podría mostrar la ventana que coincidió | Esquema nuevo, rollback, test de supervivencia, re-embed de las ventanas y un brazo KNN más. Sin prototipo: el ranking esperado es el de a |
| c. Puntaje del save | Sube el puntaje de un pedazo según sus hermanos (bono por save, máximo o suma) | Sin esquema | Ataca el síntoma: los hermanos de los casos medidos no tienen la respuesta, así que sumar puntaje sube el save sin que el pedazo correcto gane. Sin base para elegir el bono |
| d. Medir primero | No tocar el ranking hasta correr U2 sobre `main` con el set real | Cero riesgo | Con el set sustituto el problema sigue (3 de 12 fuera, respuesta nunca visible en búsqueda) |

**Riesgos de a, a medir:**

- **Hermanos en el pozo de KNN.** El colapso corre después de juntar los 30 vecinos (`search.rs:39, 1181`). Un save de 40 pedazos podría ocupar el pozo entero con sus propios pedazos y dejar afuera otras memorias. Con saves de 1.200 tokens no pasó; con uno de 5.000 tokens hay que probarlo.
- **El ancla de 46 casos** no tiene filas que se partan, así que no debería moverse; `rag-eval --real --diff` lo confirma.

## Relación con otros cambios

- **long-save-context:** reutiliza el vínculo, el colapso y `read_memory`, que ya unen los pedazos sin importar cuántos sean.
- **recoverable-delete** (en curso): no se tocan sus archivos. Borrar un save largo sigue siendo borrar filas sueltas; con a hay más filas por save.
- **lexical-fusion:** las claves con guion (`scarlet-stoat`, también los nombres de crates como `clawg-ledger`) no son identificadores para el bono léxico (`is_identifier`, `search.rs:712-717`). Tratarlas como tales subiría el pedazo 1 (el que nombra la clave) pero no el de la respuesta, y cambia todo el canal léxico: queda fuera.
- **docs-search-quality** (20) e **english-recall** (21): no se tocan.

## Decisiones abiertas

- [x] **¿a, b, c o d?** a, pedazos chicos (tarjeta en el hilo, opción recomendada; ver la [propuesta](proposal.md#decisión)).
- [x] **Si a: ¿qué tope?** 96/24, el mejor del barrido sobre el set sustituto; el set real lo confirma en U2.
- [x] **¿Se vuelven a partir los saves ya guardados?** No: reescribir filas guardadas es tocar datos del usuario por una ganancia de ranking. Un save que se guarda de nuevo queda partido con el tope nuevo.

## Cómo se mide

- `rag-eval --real --agent-view` sobre `golden-mine synth --seed 7` del set real: `long` `search_reachable` y `context_visible`, base contra punta.
- `rag-eval --real` sobre el ancla: `--diff` vacío.
- Los otros cinco subconjuntos de agent-view: ninguna métrica baja más de 0,02.
