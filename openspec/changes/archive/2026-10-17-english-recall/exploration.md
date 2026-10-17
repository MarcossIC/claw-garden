# english-recall — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | project-indexing (12, archivado); eval-rigor (33, archivado el 2026-10-15: la prueba pareada y los intervalos) | exploración cerrada; decisiones del 2026-10-16 |

> **Citas.** Las líneas de código se tomaron antes de `49bd269`. En `main` `49bd269` se movieron: los prefijos están en `embedder.rs:102` y `:123`, su comentario en `:104-122`, el de CLS en `:135-137`, `E5_MODEL_ID` en `:128` y el test que fija los prefijos en `:851-866`; `is_pooling_upgrade` y `verify_or_reembed` en `bootstrap.rs:380` y `:394`; `aggregate_by_lang` en `report.rs:535`. La propuesta vuelve a citar contra `main`.

**Tesis:** la caída de R@5 de 0,68 a 0,45 al pasar a pooling mean no es «del inglés». El set agrupa por el idioma de la **fila** esperada, no por el de la **consulta**: 15 de los 22 casos «en inglés» son consultas en español sobre memorias escritas en inglés, y con cuatro casos «es» sobre filas en inglés suman 19 cruces. Separados por pareja consulta–fila, español–español no cambia, inglés–inglés mejora, y español–inglés pierde 7 casos y gana 0 incluso con coseno puro, sin piso, decaimiento ni importancia. Así que ni el piso de coseno, ni el corte, ni el canal léxico lo explican: está en los vectores. El cambio es **primero un diagnóstico** (qué parte es el prefijo, qué parte el pooling, qué parte el contenido) y recién después, si hace falta, un arreglo. El nombre de la carpeta se conserva; la tesis lo corrige.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa (`openspec/README.md`, fila 21) | Resultado |
|---|---|
| R@5 en inglés cae de 0,68 a 0,45 con mean | **Cierta.** 0,6818 a 0,4545, o sea de 15 a 10 casos de 22 (`archive/2026-10-15-project-indexing/verify.md:129`; `docs/eval/results/2026-10-15-real-project-indexing-tip-anchor.json`, `retrieval.by_lang`). También cae R@10 (0,9091 a 0,7273) y MRR (0,4698 a 0,3733). |
| En español queda igual | **Casi.** R@5 baja de 0,8750 a 0,8333 (un caso de 24); R@1 igual, R@10 sube de 0,8750 a 0,9583 y MRR igual (`verify.md:128`). |
| «El corpus tiene 22 filas en inglés» | **Falsa tal como está dicha.** 22 es la cantidad de **casos** cuyas filas esperadas están etiquetadas `en`. Las filas por etiqueta son 22 `en` y 26 `es` (`verify.md:122`), pero por lectura de su contenido al menos cuatro filas etiquetadas `es` están escritas en inglés (c01, c08, c09, c35). |
| «No es un problema cross-lingual» | **Falsa.** `case_lang` toma el idioma de las filas esperadas y lo dice: «a case carries no `lang` of its own — only corpus rows do» (`rag-eval/main.rs:1204-1221`; `report.rs:77-81`). Leyendo las 22 consultas de los casos `en`, **15 están escritas en español** (c02, c03, c07, c11, c12, c16, c18, c25, c26, c27, c36, c41, c43, c44, c46) y 7 en inglés (c10, c15, c17, c33, c40, c42, c45). Con los cuatro casos `es` sobre filas en inglés, son **19 consultas en español sobre memorias en inglés**. |
| `rag-eval` no puede desglosar por idioma | **Falsa a medias.** Sí desglosa: `retrieval.by_lang` (`report.rs:358-377`). Pero sólo por el idioma de la fila, y el agent-view no desglosa nada por idioma (sólo `by_subset`). No hay eje por idioma de la consulta. |
| Los idiomas de las etiquetas son fiables | **Falsa en cuatro filas** (c01, c08, c09, c35), de las que al menos c09 pierde R@5 con mean. Las etiquetas de `golden.json` están fuera de git (`docs/eval/**` en `.gitignore`). |

## Problema (verificado contra el código y contra los resultados guardados)

### Lo que se ve al separar por pareja

Idiomas leídos por mí (consultas: los 46 textos; filas: contenido de las 48), no declarados en el set. Base: `…-long-save-context-tip-anchor.json` (CLS, piso 0,8793) y `…-project-indexing-tip-anchor.json` (mean, piso 0,79).

Ranking completo (bloque `retrieval`):

| Consulta → fila | n | R@5 CLS → mean | R@10 CLS → mean | MRR CLS → mean |
|---|---|---|---|---|
| español → español | 20 | 0,90 → 0,90 | 0,90 → 0,95 | 0,77 → 0,78 |
| inglés → inglés | 7 | 0,71 → 0,86 | 0,86 → 1,00 | 0,47 → 0,71 |
| **español → inglés** | **19** | **0,68 → 0,32** | **0,89 → 0,68** | **0,47 → 0,26** |

Coseno puro, sin piso, decaimiento ni importancia (bloque `similarity`, `rag-eval/similarity.rs:1-8`):

| Consulta → fila | n | R@1 CLS → mean | R@5 CLS → mean | R@10 CLS → mean |
|---|---|---|---|---|
| español → español | 20 | 0,70 → 0,70 | 0,90 → 0,90 | 0,90 → 0,95 |
| inglés → inglés | 7 | 0,29 → 0,71 | 0,71 → 0,86 | 0,86 → 0,86 |
| **español → inglés** | **19** | 0,26 → 0,16 | **0,68 → 0,32** | **0,89 → 0,58** |

En el cruce español–inglés, 7 casos pierden R@5 con coseno puro (c02, c03, c07, c09, c11, c18, c26) y ninguno lo gana; la prueba de signos exacta da p = 0,016. Es una lectura mía del set, no la prueba pareada de eval-rigor (33).

### Lo que ya descarta algunas explicaciones

- **El piso no es.** Barrido de `--sweep-cos-floor` por pareja, R@5 español–inglés: 0,37 con piso 0,70; 0,37 con 0,75; 0,32 con 0,79; 0,47 con 0,81 y con 0,83. Nunca se acerca al 0,68 de CLS, y a 0,83 inglés–inglés se hunde de 0,86 a 0,29 (`…-project-indexing-tip-sweep-cos*.json`; `verify.md:133-145` dice lo mismo en agregado). Con el piso casi abierto (0,70) la brecha sigue ahí.
- **El corte no es.** `RELEVANCE_RATIO` actúa sobre lo que el ranking ya devolvió. Los seis casos que ningún ratio recupera (c02, c09, c11, c18, c26 y c41, `verify.md:184`) son todos español–inglés. Del lado del agente, `search_memory` falla 10 de 46: 8 son español–inglés (encuentra 11 de 19), 1 inglés–inglés (6 de 7) y 1 español–español (19 de 20) (`…-project-indexing-relcut-agent-view.json`).
- **El canal léxico no es.** Las cuatro corridas `--sweep-lexical` dan lo mismo que el ancla (`verify.md:161`): este set no lo mide.
- **Los scores altos no discriminan.** La mediana del mejor score mostrado es 0,29 en español–español, 0,16 en español–inglés y 0,13 en inglés–inglés (n = 7). Que la más baja sea inglés–inglés, la pareja que mejoró, dice que el nivel del score no sigue al cruce de idiomas; es otra razón para no atribuirlo al piso.

### Qué queda en pie

El cambio de pooling movió los vectores. El comentario del propio código cuenta qué había antes: con CLS «fastembed pools a user-defined model by its first token, which gave every passage of a corpus nearly the same vector» (`embedder.rs:122-124`), y esa corrida tiene los cosenos de todas las parejas apretados entre 0,8625 y 0,947 (mediana 0,898), contra 0,6986 a 0,8847 (mediana 0,7866) con mean (bloque `similarity`, `cosine_*`). Esto sugiere que el 0,68 de CLS no es un objetivo a restaurar: era una geometría casi degenerada que dejaba a las parejas cruzadas arriba por poco margen. Lo que sí se puede afirmar es que **hoy** el cruce español–inglés es el punto débil de la recuperación de memorias: es el caso de uso normal de quien escribe sus saves en inglés y pregunta en español (o al revés).

Dos pistas, no conclusiones:

- **Prefijos.** Ambos roles envían `"query: "` (`embedder.rs:89`, `:110`). El comentario (`:91-109`) cita la recomendación de la ficha del modelo para tareas simétricas y registra que `"passage: "` bajó `sim recall@10` de 0,8261 a 0,6957, «measured with CLS pooling and has not been re-measured under mean». Esa medición fue sobre vectores casi idénticos entre sí, así que es evidencia débil para mean. Los resultados guardados lo confirman: `2026-08-16-real-pre-prefix.json` (sin prefijos) 0,8261; `…-e5-prefix.json` (`query:` y `passage:`) 0,6957; `…-symmetric.json` (`query:` en ambos) 0,8913.
- **Contenido.** Cuatro de las siete filas perdidas llevan código entre comillas invertidas, contra dos de las otras doce del cruce. Son números chicos (p cercano a 0,1), pero señalan una prueba barata: mean promedia todos los tokens, y una fila con identificadores diluye su parte en lenguaje natural. Por longitud no se separan (mediana de 209 caracteres las perdidas, cerca de 190 el resto).

## Lo que ya existe y se reutiliza

- `retrieval.by_lang` y `similarity` en el reporte, `aggregate_by_lang` (`report.rs:358-377`) y `case_lang` (`main.rs:1204-1221`).
- El bloque `similarity` de coseno puro, que sólo necesita los vectores ya capturados al sembrar, sin inferencia extra (`main.rs:1099-1145`): sirve para probar cualquier variante de prefijo sin tocar el piso ni el ranking.
- `--sweep-cos-floor`, `--sweep-lexical`, `--audit`, `--agent-view`, `--diff`, `--golden <archivo>` (`rag-eval/cli.rs:286-334`).
- El método de la verificación de project-indexing para variar una constante: recompilar `rag-eval` por valor en un worktree descartable con `--tag` propio (`verify.md:169`).
- `EvalTokenCounter::real` para contar tokens con el tokenizador del modelo (`eval_api.rs:595-625`).
- Los stores de evaluación son temporales y el modelo sólo se lee del caché: nada de esto toca `~/.clawg*`.

## Hipótesis, una por vez, y cómo se mide cada una

Todas se corren sobre una copia armada por `rag-eval` (store temporal, modelo del caché), nunca sobre el store del usuario, y comparan por pareja consulta–fila.

| | Hipótesis | Estado con lo que ya hay | Cómo se mide |
|---|---|---|---|
| a | El piso global de 0,79 corta más al cruce español–inglés | **Descartada con datos propios** (barrido por pareja arriba, y coseno puro con la misma caída) | Agregar al reporte el coseno de las filas esperadas por pareja, para ver si el cruce queda sistemáticamente por debajo del piso |
| b | El prefijo simétrico (`query:` en ambos lados) rinde peor con mean | **Abierta, la principal.** Nunca se midió con mean | Tres brazos sobre el mismo set, midiendo con el bloque `similarity`: hoy (`query:`/`query:`), `query:`/`passage:`, y sin prefijos. Se compara R@5, R@10 y MRR por pareja |
| c | Longitud o composición de tokens (código, identificadores) de las filas | **Abierta, con una pista** (comillas invertidas) | Contar tokens de las filas esperadas por pareja con el tokenizador del modelo; separar las filas con código del resto en el cruce |
| d | El lado léxico para consultas en inglés (sin palabras vacías, 16 primeros tokens por posición, `unicode61`) | **Descartada para esta brecha:** no tiene efecto medible en este set | Sólo reabrir con un subconjunto léxico en inglés; la semilla 7 tiene un subconjunto `lexical`, pero no verifiqué en qué idioma está |
| e | Control de traducción: ¿el cruce de idiomas explica toda la brecha? | Abierta | Variante del set (`--golden`, fuera de git) con las 19 consultas escritas en inglés a mano, y otra con las 19 filas en español; medir los dos brazos con mean |
| f | La etiqueta de idioma del set es ruidosa y el n es chico | **Cierta** (cuatro filas mal etiquetadas, n = 19) | Etiquetar consulta y fila por separado (`query_lang`, `row_lang`), y reportar por pareja |

Los brazos de (b) necesitan una forma de variar el prefijo: hoy no hay eje en `rag-eval` (los prefijos son constantes, `embedder.rs:89` y `:110`; el reporte sólo los registra, `main.rs:268-269`). O se recompila en un worktree descartable, que no suma código al repo, o se agrega un constructor sólo de evaluación, que sí lo suma.

## Enfoques posibles

### A. Diagnosticar y dejar registrado (recomendado como primer paso)

Eje por idioma de la consulta en el reporte, brazos (a) a (f) y un registro de lo que cada uno explica. Sin cambio de comportamiento: ranking, piso, prefijos y esquema quedan como están.

- A favor: no toca datos del usuario ni el ranking; decide con números si hace falta B o C o D; arregla la medición que hoy engaña (el reporte dice «inglés» para casos en español).
- En contra: puede terminar sin arreglo. Es el caso en que se pasa a D.

### B. Umbrales por idioma (se descarta)

Un piso de coseno para cada idioma o para cada pareja.

- El idioma que importa es el de la pareja consulta–fila, no el de la fila; el store no guarda el idioma ni de la consulta (que no existe al guardar) y detectarlo agregaría una dependencia o una abstracción sin otro uso (regla de `config.yaml`).
- Los datos propios lo descartan: con el piso casi abierto la brecha sigue, y ningún piso la cierra (arriba).

### C. Cambiar el prefijo, si (b) se confirma

Pasar `E5_PASSAGE_PREFIX` a `"passage: "`, o los dos, según qué brazo gane. Lo que arrastra, verificado:

- El contrato de embedding es `multilingual-e5-small/mean` (`embedder.rs:115`) más los dos prefijos. Al arrancar sólo se repara solo una mejora de pooling: `is_pooling_upgrade` exige el mismo modelo viejo, la misma dimensión y **los mismos prefijos** (`bootstrap.rs:255-261`), y `verify_or_reembed` devuelve cualquier otro desajuste tal cual (`:278-282`). Un cambio de prefijo deja la memoria **apagada** (contrato `Mismatch`, estado `failed` no reintentable) hasta que haya un camino de re-embebido.
- Hay dos caminos: model-reembed (17), opción B (`clawg reembed`, offline, con el snapshot `pre-reembed` que ya existe, `backup.rs:127-139`), o generalizar `is_pooling_upgrade` a «cambio de prefijo» con el mismo snapshot. El snapshot previo y la regla de no perder datos de `config.yaml` se cumplen con cualquiera; hace falta el test de base sembrada de la versión anterior.
- El prefijo está fijado por un test (`embedder.rs:765-772`) que habría que cambiar a propósito.
- **Se mueven todos los umbrales absolutos**: el piso 0,79 (su comentario lo ata a los prefijos, `score.rs:161-182`), `CONTRADICTION_THRESHOLD = 0,42` y `NEAR_DUPLICATE_FLOOR = 0,10` (`contradiction.rs:58`, `:66`) y `RELEVANCE_RATIO = 0,35` (`relevance.rs:15-19`). Hay que re-medirlos con el barrido de piso, la curva de contradicciones (28 pares) y el agent-view.
- El índice de docs se rehace sólo cuando cambia el hash del archivo o el `model_id` (`project_docs/mod.rs:104-110`); un cambio de prefijo con el mismo `model_id` dejaría los pasajes con vectores del prefijo viejo. Hay que decidir si el id del modelo incluye el prefijo o se invalida el índice.
- Por todo esto **no se hace junto con memory-rank-fusion (34)**: los dos mueven la escala de los scores, y una medición con ambos no dice cuál produjo cada diferencia.

### D. Aceptar la brecha

Si los brazos muestran que es una propiedad del modelo con este contenido y que ningún prefijo la cierra, se registra y se sigue. Criterio (decidido): un brazo cuenta como mejora del cruce si McNemar exacto sobre los casos de esa pareja da p < 0,05, lo que con 0 pérdidas pide 6 casos ganados (p = 0,031); con pérdidas, la cuenta neta sube. Con n = 19, cada caso vale 5 puntos.

**Recomendación:** A. Y de ahí D o C, según lo que midan (b) y (e). B queda descartada.

## Descartado

| Opción | Razón técnica |
|---|---|
| Volver a CLS | CLS daba a todas las filas vectores casi iguales (`embedder.rs:122-124`); con mean mejoran inglés–inglés y el R@10 español–español, y la separación entre la esperada y sus distractoras casi se duplica (`mean_corpus_distractor_gap` de 0,063 a 0,117). Además los stores ya migraron (snapshot `pre-reembed`). |
| Traducir la consulta o la fila al guardar | Necesita un traductor, o sea un modelo o proveedor nuevo y una dependencia sin otro uso. La traducción a mano de (e) es sólo un control, fuera del producto. |
| Cambiar de modelo | Es model-reembed (17), de urgencia baja, y recién se justifica si los brazos de prefijo no alcanzan. |
| Arreglar el piso o el corte | Ya se vio que no explican la brecha; el piso se recalibró con el barrido y el ratio con el agent-view (`verify.md:133-184`). |
| Cambiar el set para que la brecha desaparezca | Las filas mal etiquetadas se corrigen; las consultas no se reescriben para ocultar un caso real. |

## Interacciones

- **eval-rigor (33).** Archivado el 2026-10-15. `--diff` ya da McNemar exacto caso por caso sobre `retrieval` y `similarity` (`diff.rs:96-203`), pero sobre los 46 casos juntos: no corta por pareja consulta–fila. Para probar el criterio sobre los 19 del cruce hace falta ese corte, en el reporte y en el diff, o un golden que sólo tenga esa pareja; lo decide la propuesta. Esta exploración usó conteos y la prueba de signos exacta.
- **memory-rank-fusion (34).** Se miden por separado. Si 34 aterriza antes, esta base es la nueva.
- **model-reembed (17).** Es el camino de C. Su urgencia sube sólo si (b) gana.
- **docs-search-quality (20).** La búsqueda de docs usa el mismo embedder: un cambio de prefijo la afecta (ver C), aunque sus preguntas se midieron por separado.
- **memory-protocol (22).** Cuánto pesa el cruce de idiomas depende de si el protocolo recomienda un idioma para los saves. Va en las decisiones.
- **contradiction/conflict-candidates (24).** La banda `[0,10, 0,42)` está calibrada sobre este espacio; un prefijo nuevo la mueve.

## Alcance candidato

1. Eje por idioma de la consulta en el set y el reporte (`query_lang`, `row_lang`, por pareja), con compatibilidad para sets viejos (campo opcional).
2. Corregir las etiquetas del set propio (c01, c08, c09, c35), que viven fuera de git.
3. Un golden aparte para el cruce (`--golden`, fuera de git como el resto de `docs/eval`): casos inglés→español sobre filas que ya existen (hoy n = 0) y más casos español→inglés. `golden.json`, el ancla de 46 casos, no cambia.
4. Los brazos (a) a (f), con el bloque `similarity` y el agent-view. Los de prefijo (b) se compilan en un worktree descartable por brazo, con `--tag` propio; no se agrega un constructor ni un flag de prefijo.
5. El registro del diagnóstico (en el `verify.md` del cambio o su `proposal.md`).
6. Sólo si (b) o (c) lo piden: la propuesta de un cambio de prefijo, como cambio aparte.

### No-alcance

- Cambiar el prefijo, el pooling, el modelo, el piso, el corte o el canal léxico dentro de la fase de diagnóstico.
- Detección de idioma en el store.
- Agregar casos al set más allá de etiquetar mejor y de sumar un subconjunto cruzado.
- Mezclar el trabajo con 34.

## Decisiones (2026-10-16)

- [x] **Alcance.** Sólo A, el diagnóstico. Si (b) gana, el cambio de prefijo (C) se propone como cambio aparte: arrastra re-embebido (17) y volver a medir el piso, el corte y la banda de contradicciones.
- [x] **¿Cómo se varía el prefijo?** Worktree descartable, una compilación de `rag-eval` por brazo con `--tag` propio. No se agrega un constructor sólo de evaluación.
- [x] **Criterio de mejora.** McNemar exacto p < 0,05 sobre los casos de la pareja (6 ganados con 0 perdidos), sin que bajen las otras parejas ni el ancla global. Reemplaza el «ganar 4 casos» de la tabla de umbrales, que daba p = 0,125.
- [x] **Subconjunto cruzado.** Se amplía en un golden aparte, sin tocar `golden.json`.
- [x] **Dirección inversa.** Entra en ese golden aparte: consultas en inglés sobre filas en español que ya existen.
- [x] **Etiquetas.** El `lang` de la fila pasa a ser el idioma real del contenido; se corrigen c01, c08, c09 y c35.
- [x] **Qué es «escribir bien» en cada idioma.** No se decide acá: este cambio le pasa los números a memory-protocol (22).

## Cómo se mide

### Pre-compuerta (antes de la propuesta)

Estas son las cifras de base; ninguna requiere código nuevo, se leen de los resultados guardados con la separación por pareja:

| Pareja | n | R@5 mean | R@10 mean | R@5 sin piso (`similarity`) |
|---|---|---|---|---|
| español → español | 20 | 0,90 | 0,95 | 0,90 |
| inglés → inglés | 7 | 0,86 | 1,00 | 0,86 |
| español → inglés | 19 | 0,32 | 0,68 | 0,32 |

### Umbrales propuestos, a fijar antes de medir (aplican a cualquier brazo que se proponga como cambio)

| Métrica | Base | Umbral |
|---|---|---|
| español → inglés, R@5 (n = 19) | 0,32 | McNemar exacto p < 0,05 sobre los casos de la pareja (con 0 pérdidas, 6 ganados: `>= 0,63`) |
| español → inglés, R@10 | 0,68 | sin bajar |
| español → español, R@5 y R@10 | 0,90 / 0,95 | sin bajar |
| inglés → inglés, R@5 y R@10 | 0,86 / 1,00 | sin bajar |
| Ancla global, R@1 / R@5 / R@10 / MRR / nDCG@10 | 0,4348 / 0,6522 / 0,8478 / 0,5524 / 0,6215 | ninguna baja |
| Agent-view | `search_memory` 36/46 a 764,0 tokens; `recall_context` 39/46 a 1.086,4 | sin bajar los aciertos, sin subir los tokens |
| Contradicciones al corte actual | recall 0,8 a 0,9 con `fpr` 0 (28 pares) | recall igual o mejor y `fpr` 0, o el corte se re-mide |

Si un umbral falla se anota tal cual y no se mueve después de medir. Un arreglo sólo se propone si los umbrales se cumplen en el brazo ganador y la brecha no se explica por (e) o (f).

### Tests RED (sólo si hay cambio)

1. El reporte tiene un eje por pareja consulta–fila y, para un set sin esos campos, se comporta como hoy (un resultado viejo se sigue leyendo).
2. Con dos casos donde sólo cambia el idioma de la consulta, el reporte los pone en parejas distintas.
3. Si (b) gana y hay cambio de prefijo: una base sembrada con vectores del prefijo viejo abre, queda alcanzable y se re-embebe con snapshot previo; un binario anterior la abre o la rechaza sin tocarla (regla de `config.yaml` para el contrato de embedding). El test de contrato de producción cambia a propósito.

`rag-eval --diff` contra la base: obligatorio si cambia un prefijo, porque toca el ranking. El reporte de agent-view se compara a mano mientras 33 no lo cubra (`--diff` hoy no lo mira).

## Riesgos

- **Conclusión sobre n chico.** 19 casos son poco; la prueba de signos da p = 0,016 sobre las pérdidas, pero es una sola muestra de un set con dos dominios. Hay que ampliar el cruce antes de apostar a un cambio.
- **Idiomas leídos por mí.** Las parejas de esta exploración salen de mi lectura de las consultas y del contenido; el paso 1 del alcance es justamente fijarlas en el set.
- **Volver a un número que no era sano.** El 0,68 de CLS salió de una geometría casi degenerada; no es un objetivo de recuperación.
- **Un prefijo cambia todo.** Si (b) gana, se mueven el piso, el corte, los umbrales de contradicción y la indexación de docs. Hay que medir esos umbrales de nuevo y no mezclarlo con otros cambios de ranking.
- **Memoria apagada.** Un cambio de prefijo sin camino de re-embebido apaga la memoria al arrancar (ver C). Por eso C depende de 17 o de generalizar la reparación existente, con snapshot previo.
- **Corpus de eval chico.** El set de 48 filas cubre dos dominios de trabajo; la base real de un usuario es otra cosa. Se mide sobre una copia, y el resultado orienta, no demuestra.
