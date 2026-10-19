# docs-search-quality — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 | M | eval-rigor (33; todavía sin carpeta). El paso 1 no lo necesita y puede ir antes. 22 empuja la prioridad; 34 comparte `fts_match_expression` (ver «Interacciones») | exploración (borrador) |

**Tesis:** `search_project` pone el pasaje correcto entre los tres primeros en poco más de una de cada cuatro preguntas (Hit@3 0,27 en `rag-eval --real --docs`). Casi todo lo barato está en dos lugares que no necesitan un modelo nuevo: el canal léxico, que hoy une con OR todas las palabras de la pregunta (vacías incluidas) tomando las primeras 16 por posición, y cómo se etiquetan y parten los identificadores. Antes de eso hay que arreglar la regla con la que se mide, porque `rag-eval --docs` indexa un archivo que cita sus propias preguntas y lo pone primero en 7 de 11 fallas. Cada paso se mide solo; un reranker o un modelo de embedding mayor quedan para si los pasos baratos no alcanzan. Es un ajuste que sube el objetivo 3 (que la IA entienda cómo funciona cada proyecto), y sube de prioridad cuanto más empuje el protocolo de memoria a `search_project`.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| Las dos tablas FTS usan `unicode61 remove_diacritics 2` | **Cierta.** `memory_fts` en `crates/clawg-core/migrations/V8__memory_fts.sql:36-43` y `project_doc_fts` en `V12__project_docs.sql:45-48`. La de docs guarda `text` y `project_id UNINDEXED`, sin más columnas. |
| `_` parte el token y un identificador camelCase queda como uno solo | **Cierta, y el repo ya lo asume.** `holds_identifier` documenta que FTS5 parte en `_` (`search.rs:783-786`). El tokenizador pasa a minúsculas, así que `memoryType` se indexa como `memorytype`; `memory_type` como `memory` y `type`. Una consulta «memory type» no encuentra lo primero y `memoryType` no encuentra lo segundo. |
| La expresión toma hasta 16 tokens unidos con OR, por posición | **Cierta.** `fts_match_expression` (`search.rs:619-627`), `MAX_LEXICAL_TOKENS = 16` (`:599`), comentario que declara «por posición, no por rareza» (`:593-598`). No hay ninguna lista de palabras vacías en el código de recuperación; las únicas son de las herramientas de evaluación (`rag-eval/audit.rs:110`, `golden-mine/synth/vocab.rs:11`). |
| Docs reusa la misma función | **Cierta, en otra línea.** `project_docs/mod.rs:271` (no 265). La memoria la usa en `lexical_candidates` (`search.rs:716`). Cambiarla cambia los dos canales. |
| RRF_K = 60, `fuse`, 50 candidatos por canal, KNN partido por proyecto | **Cierta.** `RRF_K` `mod.rs:42`, `CANDIDATES_PER_CHANNEL` `:37`, `fuse` `:243-254`, `search` `:258-305`; partición en `V12__project_docs.sql:36-39`. |
| Pasajes con rol `Passage` y consulta con rol `Query`, y los dos prefijos son «query: » | **Cierta.** `mod.rs:405` y `:266`; `E5_QUERY_PREFIX` y `E5_PASSAGE_PREFIX` (`embedder.rs:89`, `:110`). El comentario (`:91-109`) dice que «passage: » fue peor para **memorias** con CLS (recall@10 de 0,8261 a 0,6957) y que no se re-midió con mean. Para docs nunca se midió. |
| Cambiar el prefijo es un contrato «sólo del índice de docs» | **Falsa.** El prefijo es del embedder, no del índice: `passage_prefix()` devuelve la misma constante para todo (`embedder.rs:236-238`), y el rol `Passage` también lo usan las memorias (`admin.rs:1614`, `maintenance.rs:586`, `contradiction.rs:625`). Mover la constante cambia el contrato de las memorias (y se cruza con 21). Para dejarlo en docs hace falta un rol o un prefijo propio del índice. |
| Un re-embebido de docs se dispararía solo por cambiar el prefijo | **Falsa.** `plan_refresh` compara sólo `content_hash` y `model_id` (`mod.rs:104-117`, el `110`). El `model_id` es el del embedder (`mod.rs:358`) y nombra el modelo y el pooling (`E5_MODEL_ID = "multilingual-e5-small/mean"`, `embedder.rs:115`), no el prefijo ni el texto indexado. Un cambio de prefijo, de texto léxico o de breadcrumb dejaría los vectores viejos como «al día». Hay que meterlo en la huella. |
| El nombre y la firma de una función están en el encabezado de un pasaje de código | **A medias.** El breadcrumb es `ruta > etiquetas` (`chunk.rs:109-113`). Una etiqueta (`header`, `code.rs:181-193`: sólo la primera línea del contenedor, hasta `{`, con tope de 120 bytes) se agrega sólo cuando un nodo **no entra** en el tope de 1.200 caracteres y se parte (`code.rs:156-173`). Las funciones chicas se empaquetan juntas y el pasaje lleva sólo la ruta, o las etiquetas del nodo más grande (`Packer::add`, `code.rs:104-115`). El nombre y la firma **sí** están en el cuerpo, y el cuerpo se embebe y se indexa (`indexed_text`, `chunk.rs:28-30`; `mod.rs:403` y `:485`). Lo que falta no es la presencia sino el peso y la posición. |
| Hit@3 de docs es 0,27 contra 0,70 | **Cierta, con matiz.** 0,27 es el total de 15 preguntas (10 de docs y 5 de código); docs solo da 0,20 y código 0,40 (`verify.md:190-194`). El 0,70 es el umbral de la propuesta de project-indexing, no una medición. |
| «Filtrar palabras vacías da Hit@3 0,40» | **No es exactamente eso.** El experimento filtró los tokens de **menos de 4 letras**, no una lista de palabras vacías (`verify.md:66`), sobre las 10 preguntas de docs, antes del ajuste de `RELEVANCE_RATIO`. En esa tabla, sólo vectores con mean da 0,30 y con ese filtro 0,40; la fusión de hoy da 0,20 en docs (`verify.md:54-56`, `:62-66`). Con 10 preguntas, una pregunta son 10 puntos. |
| `rag-eval --docs` se contamina con su golden set | **Cierta, pero no es el archivo de preguntas.** El golden set es `crates/clawg-eval/golden/project-docs.json` y no se indexa (el corpus sólo toma Markdown, manifiestos y código: `corpus.rs:30-39`). Lo que contamina es `verify.md` de project-indexing, que cita las preguntas textuales en una tabla y sigue en el corpus archivado: sale primero en 7 de 11 fallas (`verify.md:198`). El golden set está fijado al repo entero (`default_docs_root`, `cli.rs:214-216`), así que **cualquier** doc que cite una pregunta contamina. |
| Hay un experimento de reranker | **Cierto.** Un reranker multilingüe de tamaño medio sobre los 20 primeros: Hit@3 0,50, MRR@10 0,28, unos 10 s por pregunta (`verify.md:69`). Con el léxico filtrado y sin reranker el MRR era 0,29 (`verify.md:66`). |
| Existen 33 (eval-rigor) y 34 (memory-rank-fusion) | **No en el mapa.** `openspec/README.md` llega a la fila 32 y no hay carpetas con esos nombres. 21 (english-recall) también está sin explorar. Se trata como dependencias previstas; ninguna es necesaria para el paso 1. |

## Problema (verificado contra el código)

### Cómo busca hoy `search_project`

`search` (`mod.rs:258-305`) hace dos consultas y las funde:

| Canal | Qué hace | Archivo |
|---|---|---|
| Denso | Vecinos del vector de la pregunta (`"query: " + pregunta`), 50, en la partición del proyecto | `mod.rs:265-283`, `V12:36-39` |
| Léxico | BM25 de FTS5 con `"t1" OR "t2" OR …` de hasta 16 tokens de la pregunta, 50 | `mod.rs:271`, `:284-293` |
| Fusión | `1/(60 + rango)` sumado; empata por id | `mod.rs:243-254` |

El texto de cada pasaje que entra a los dos canales es `breadcrumb + "\n\n" + cuerpo` (`chunk.rs:28-30`; `mod.rs:403`, `:485`).

### Qué falla, con lo medido

- **Una pregunta de lenguaje natural es casi toda palabras vacías.** Una de las del golden set tiene 19 palabras (`verify.md:79`); el recorte a 16 es por posición y descarta las últimas, que suelen ser las que nombran la cosa (`search.rs:620`). Cada «de», «el», «the» trae pasajes que sólo comparten eso (`verify.md:71`; `specs/project-docs` «Límites conocidos»).
- **Los identificadores se parten distinto según cómo estén escritos** (premisa 2): el canal léxico no cruza `snake_case` con `camelCase`, que en un repo con Rust, Python y TypeScript es el caso normal.
- **La regla de medición está sesgada.** Con el archivo que cita las preguntas en el corpus, 7 de 11 fallas tienen primero ese pasaje (`verify.md:198`). Toda mejora léxica empuja más hacia ese pasaje, porque comparte las palabras de la pregunta por construcción. Medir antes de arreglarlo puede mostrar una mejora falsa o esconder una real.
- **La medición es chica.** 15 preguntas (`verify.md:190`): un acierto son 6,7 puntos de Hit@3, y 10 en el subconjunto de docs. Ningún umbral fino se sostiene sin el set más grande de eval-rigor.
- **Prefijo.** Pasaje y pregunta usan el mismo prefijo (premisa 5). Para docs largos contra preguntas cortas la forma es asimétrica, justo lo que el comentario de `embedder.rs:102-109` dice que sería el motivo para volver a «passage: ». No está medido en docs.

### Por qué ahora

El protocolo de memoria (22) empuja a los agentes a llamar `search_project` antes de abrir archivos (`memory-protocol/exploration.md:99`; `INSTRUCTIONS` en `server.rs:108`). Una herramienta que acierta una de cuatro cuesta tokens cada vez que el agente confía en ella y se equivoca, y lo más probable es que abra los archivos de todos modos.

## Lo que ya existe y se reutiliza

- `rag-eval --real --docs` y su golden set de 15 preguntas, con Hit@1/3/5 y MRR@10 separados por docs y código (`docs.rs:96-151`). Usa la misma `project_docs::search` que producción (`eval_api.rs:708`), así que mide la ruta real.
- `check_corpus` (`docs.rs:76-84`): ya falla si un doc esperado falta; es el lugar para que también falle por un doc que cita la pregunta.
- El patrón de barrido «recompilar con cada valor en un worktree descartable» usado para `RELEVANCE_RATIO` (`verify.md:169`).
- `fts_tokens` y `identifier_tokens`/`is_identifier` (`search.rs:632`, `:763`, `:776`): extracción pura de identificadores, ya con tests.
- `plan_refresh` (`mod.rs:104`) y `model_id` en la huella (`project_doc_file.model_id`, `V12:10-18`): un cambio de la cadena reindexa todo sin código extra (`another_model_reindexes_every_doc`, `mod.rs:726`). Es el mecanismo para cualquier cambio de contrato del índice de docs.
- El índice de docs se deriva del disco y se puede reconstruir (`specs/project-docs`, «Invariantes»): ningún paso toca memorias.
- `chunk_code` y `header` ya producen etiquetas por contenedor (`code.rs:63`, `:181`).

## Enfoques posibles

Escalera ordenada. Cada paso se mide **solo**, contra el anterior, con `rag-eval --real --docs`; no se juntan dos cambios en una medición.

### 1. Arreglar la medición (recomendado, primero)

`rag-eval --docs` deja fuera del corpus los docs que citan una pregunta del golden set. Dos formas:

- **a. Excluir por contenido:** una función pura `quotes_a_question(text, cases) -> bool` que descarta un doc si contiene una pregunta textual. Cubre también docs futuros.
- **b. Excluir por ruta:** una lista de rutas en el golden set. Más simple, pero hay que acordarse de mantenerla.

Tiene que aplicarse al recorrido que usa `rag-eval` (`eval_api.rs:722`), sin tocar `corpus::walk` de producción. El `check_corpus` sigue fallando si el pasaje esperado fue excluido.

- A favor: sin código de producción, sin migración, sin riesgo para los datos. Cambia la base contra la que se miden todos los demás pasos, así que se hace **primero** y se vuelve a medir la línea base.
- En contra: baja o sube el 0,27 sin que cambie el producto. Hay que escribirlo en el reporte para que nadie lo lea como una regresión.

### 2. Canal léxico

Una expresión propia del índice de docs, sin tocar `fts_match_expression`:

- Palabras vacías en español e inglés y tokens muy cortos fuera, **salvo** los que son identificadores (`is_identifier`).
- Los tokens más raros primero en lugar de los primeros 16. La rareza real necesita la frecuencia de documento del token: SQLite ofrece la tabla virtual `fts5vocab` para leerla, pero crearla es una tabla nueva (migración) o una consulta aparte por token. Sin eso, una heurística pura (identificadores primero, luego los más largos) es una aproximación sin costo.
- Intentar con AND sobre los más raros y caer a OR si no hay resultados. Con preguntas largas, el AND sobre todo casi nunca coincide; sobre dos o tres tokens raros puede ser muy preciso.

- Cada token una sola vez. `fts_match_expression` toma los primeros 16 por posición y no quita repetidos (`crates/clawg-core/src/memory/search.rs:619-620`), así que una palabra repetida en la pregunta ocupa varios lugares del tope y deja afuera otras.
- Una ruta en la pregunta se busca como frase. `crates/clawg-core/src/memory/search.rs` se parte en siete palabras comunes (`fts_tokens`, `search.rs:629`) que, unidas con OR, coinciden con cualquier pasaje que diga «memory» o «src». Como frase FTS5 (`"crates clawg core src memory search rs"`), coincide con el breadcrumb del archivo, que empieza por su ruta y encabeza el texto que va a `project_doc_fts` (`indexed_text`, `crates/clawg-core/src/memory/project_docs/chunk.rs:28-30`). Es la pregunta típica de «qué hace este archivo».

Piezas separables (se miden por separado): (2a) palabras vacías y cortos, (2b) orden por rareza, (2c) AND con respaldo OR, (2d) tokens repetidos, (2e) rutas como frase.

- A favor: es lo que el experimento del verify ya mostró (0,30 a 0,40 con una versión burda). Pura y testeable sin base. Sin migración, sin re-embebido: el texto indexado no cambia.
- En contra: dos idiomas y una lista que puede sesgar a uno (el corpus es ES + EN: `V8__memory_fts.sql:30-34`; el cambio de conflict-candidates descartó una lista de un solo idioma por esa razón). Los nombres de código en inglés dentro de prosa en español no son palabras vacías pero sí son cortos (`id`, `db`): por eso la excepción de identificadores.
- **Alcance (decisión del usuario):** solo docs (recomendado) o compartida con la memoria. Compartirla cambia el canal léxico de las memorias, cuya ancla (R@5 0,6522) no está para perder (`openspec/config.yaml`). Si se comparte, se mide con `rag-eval --real --diff` y hay que coordinar con 34.

### 3. Identificadores partidos

Indexar también las formas partidas de camelCase/PascalCase (`memoryType` → `memory type`) y ofrecer la forma unida a la consulta. FTS5 con `unicode61` no parte camelCase por sí solo, así que hay que hacerlo del lado nuestro. Dos formas:

- **a. Texto preprocesado:** el texto que va a `project_doc_fts` (`mod.rs:485`) lleva el cuerpo y, al final, las formas partidas de los identificadores. El cuerpo mostrado (`project_doc_chunk.body`) no cambia. Sin migración de esquema.
- **b. Columna FTS propia** para las formas partidas, con su peso en BM25. Es otro esquema: FTS5 no permite agregar columnas, hay que descartar y recrear `project_doc_fts` (migración V13; los datos son derivados).

Del lado de la consulta: partir los identificadores de la pregunta por la misma regla, o consultar ambas formas.

- A favor: cruza `snake_case` con `camelCase` y con la prosa («memory type»). Sin modelo nuevo.
- En contra: **cambia el contrato del índice** y por lo tanto hay que forzar el re-embebido/reindexado (ver «Contrato del índice de docs»); la forma (a) infla un poco el texto, que se embebe también y comparte el tope de 480 tokens (`CHUNK_MAX_TOKENS`, `chunker/mod.rs:15`), así que o se agrega sólo al texto léxico (no al que se embebe) o se descuenta del presupuesto. Se recomienda **solo al texto léxico**: el vector no gana nada con ello.

### 4. Prefijo «passage: » para los pasajes de docs

Probar en un worktree descartable (como el barrido de `RELEVANCE_RATIO`) cambiando el prefijo de pasaje, y medir solo docs. Si gana, productivizarlo **sólo para docs**.

- A favor: el modelo se entrenó con los dos prefijos para recuperación asimétrica, y docs largos contra preguntas cortas es esa forma. Es una línea en el experimento.
- En contra, verificado: no es una constante de docs (premisa 5). Productivizarlo exige un rol o un prefijo propio del índice (`EmbedRole` más una variante, o un método nuevo del trait `Embedder`), que toca todos los implementadores (`embedder.rs:146-155`), y meter el prefijo en la huella de `project_doc_file` (premisa 6), si no, un índice viejo queda con vectores del otro prefijo. Durante el re-embebido el refresco cambia **doc por doc** (`replace_doc`, `mod.rs:464`), así que por un rato el índice mezcla dos espacios; el mismo riesgo existe hoy con un cambio de modelo. El reindexado completo de este repo tomó 645 s en debug (`verify.md:188`).
- **Memorias aparte.** Mover la constante global cambia el contrato de las memorias y la ancla; ese ajuste vive en 21 (english-recall) y no entra aquí.

### 5. Nombre y firma en el encabezado de los pasajes de código

Poner en el breadcrumb el nombre y la firma de cada definición que el pasaje contiene (`path > fn search`), aun cuando varias funciones chicas se empaqueten juntas.

- A favor: el breadcrumb es lo que se muestra al agente y lo que el canal léxico ve primero.
- En contra, verificado: el nombre ya está en el cuerpo (premisa 7), así que la ganancia es de peso y no de presencia, y es la menos probable de mover Hit@3. Un pasaje con varias funciones tendría un breadcrumb largo (hay un tope de 256 bytes: `MAX_BREADCRUMB_BYTES`, `chunk.rs:12`). Cambia el texto embebido y el indexado: es un cambio de contrato igual que el 3 y el 4. **Se mide el último**, y sólo si 1 a 4 dejan preguntas de código sin acertar.

### 6. Reranker local sobre los 20 primeros (descartado por ahora)

Un modelo pequeño que reordene los 20 primeros con un presupuesto de latencia (propuesta: 300 ms p95 en CPU).

- **Lo medido en este repo:** Hit@3 0,50 y MRR@10 0,28 con unos 10 s por pregunta (`verify.md:69`). Es 30 veces el presupuesto de latencia propuesto y el MRR no mejora respecto de sólo arreglar el canal léxico (0,29, `verify.md:66`).
- Un modelo así es una dependencia nueva y un contrato de modelo nuevo (descarga, caché en el directorio de datos, licencia, estado del cargador). Va contra «no agregar dependencias sin un uso real» (`CLAUDE.md`).
- Se reabre sólo si 1 a 5 medidos no llegan al objetivo **y** existe un reranker que cumpla la latencia con una licencia compatible. Hoy no hay medición que lo apoye.

### 7. Un modelo de embedding mayor o uno orientado a código (descartado por ahora)

El verify midió e5-base int8 con mean y léxico filtrado: Hit@3 0,40 al doble del costo; e5-large int8 sólo vectores: Hit@3 0,40 con 745 s para embeber (`verify.md:67-68`). Es el mismo 0,40 que da el modelo actual con el léxico arreglado, a más costo. Cambiar de modelo es un cambio del contrato de embedding que alcanza a las memorias (17, model-reembed) y no tiene urgencia (`openspec/README.md`, fila 17). No hay ninguna medición en este repo de un modelo orientado a código.

### 8. Un piso de relevancia para lo que se muestra

`fuse` (`mod.rs:247-258`) devuelve siempre `limit` pasajes, aunque la pregunta no tenga nada que ver con el proyecto, y la `score` que ve el modelo es la suma de recíprocos de RRF (`mod.rs:251`), que con `RRF_K` = 60 (`mod.rs:42`) es como mucho 2/61 (unos 0,033) y no dice nada en términos absolutos. La búsqueda de memorias sí se abstiene: corta por `RELEVANCE_RATIO` y no muestra un hit de puntaje 0 (`openspec/specs/retrieval/spec.md`; `openspec/changes/archive/2026-10-18-search-abstention/`), pero ese corte no se aplica a `search_project`. Hay dos formas baratas: llevar en cada hit el coseno del canal denso y no mostrar lo que queda bajo el `cosine_floor` de las memorias, o exigir que un pasaje de rango bajo aparezca en los dos canales. Cualquiera de las dos cambia lo que ve el modelo, así que se mide con preguntas sin respuesta en el set de docs (hoy no tiene ninguna) y no mueve Hit@3 en las que sí la tienen.

**Recomendación:** 1, 2 y 3 en ese orden, cada uno con su medición. El 4 se **prueba** en un worktree y se productiviza sólo si gana por un margen pre-comprometido. El 5 queda condicionado a lo que dejen los anteriores. 6 y 7 fuera. El 8 va después de 2, porque el canal léxico filtrado cambia qué pasajes aparecen en los dos canales, y necesita preguntas sin respuesta en el golden set de docs.

## Contrato del índice de docs

Los pasos 3, 4 y 5 cambian lo que se embebe o se indexa sin cambiar el modelo, y hoy eso no reindexa nada (premisa 6). La solución más chica es una función pura `docs_contract(model_id) -> String` que arme la cadena que se guarda en `project_doc_file.model_id`, con una versión del formato del índice y del prefijo (por ejemplo `multilingual-e5-small/mean+docs2`). `plan_refresh` ya reindexa un doc cuando esa cadena difiere (`mod.rs:110`), sin esquema nuevo.

Reglas de `config.yaml` que aplican: un cambio del contrato de embedding lleva un test con datos sembrados en la versión anterior. Aquí: un índice armado con la cadena vieja se reindexa entero en el siguiente refresco, sin perder pasajes (`a_failed_embedding_keeps_the_doc_as_it_was` ya prueba que un fallo deja el doc como estaba), y las memorias no cambian (`indexing_docs_leaves_memory_search_unchanged`, `mod.rs:987`).

## Descartado

| Opción | Razón técnica |
|---|---|
| Cambiar `fts_match_expression` para todos | Cambia el canal léxico de las memorias, que tiene su ancla (`search.rs:716`). Se acota a docs o se mide con `--diff` en las dos; ver «Interacciones». |
| Subir `MAX_LEXICAL_TOKENS` sin más | Más tokens por posición agregan más palabras vacías al OR. El problema es cuáles, no cuántas. |
| Cambiar `RRF_K` o `CANDIDATES_PER_CHANNEL` | No hay evidencia de que la fusión sea la causa: con 10 preguntas de docs nada distingue un cambio de pesos de un azar. Se reabre con el set de eval-rigor. |
| Mover `E5_PASSAGE_PREFIX` globalmente | Cambia el contrato de las memorias y la ancla (R@5 0,6522) por una medición que sólo existe en docs. Es de 21. |
| Reranker y modelos mayores | Ver pasos 6 y 7 con lo medido. |
| Medir con el set de 15 preguntas y declarar umbral fino | Un acierto son 6,7 puntos. Sin eval-rigor el umbral es ruido. |

## Interacciones

- **memory-protocol (22).** Empuja más llamadas a `search_project`: es lo que sube la prioridad de este cambio. No comparten código.
- **memory-rank-fusion (34, sin carpeta).** Comparte `fts_match_expression`. Si 34 cambia la expresión de memorias y este cambio la copia para docs, las dos quedan separadas y se evita medir dos cambios a la vez. Hay que acordar quién es dueño de la función compartida; la recomendación es dejarla como está y crear la de docs aparte.
- **eval-rigor (33, sin carpeta).** Aporta pruebas pareadas y un golden set de docs más grande. Sin eso, los pasos 2 a 5 se miden sobre 15 preguntas y sólo sirven para descartar lo que empeora o para ver ganancias grandes.
- **english-recall (21).** El prefijo de las **memorias** es suyo. Si 21 decide cambiar `E5_PASSAGE_PREFIX`, el paso 4 queda subsumido para docs, que heredaría el prefijo, o se separa la decisión.
- **model-reembed (17).** Cambiar de modelo reindexa docs sin código extra (`mod.rs:110`), pero el paso 7 lo descartó.
- **project-root-guard (29).** Decide qué raíces se indexan; este cambio no toca eso.

## Alcance candidato

1. Paso 1: el recorrido de `rag-eval --docs` excluye los docs que citan una pregunta; `check_corpus` falla si eso deja sin pasaje esperado a una pregunta; el reporte dice cuántos docs excluyó. Línea base nueva.
2. Función pura de la expresión léxica de docs (palabras vacías ES y EN, cortos, identificadores exceptuados, orden de tokens, AND con respaldo OR) con su tabla de casos, usada por `search` de `project_docs` y no por la memoria.
3. Formas partidas de identificadores en el texto léxico (y en la consulta), con una función pura `split_identifier`.
4. `docs_contract` y el test de reindexado con una huella vieja.
5. (Condicional) rol o prefijo propio del índice de docs, si el experimento del paso 4 gana.
6. Actualizar `specs/project-docs` (búsqueda y límites conocidos, que hoy dicen que filtrar las vacías «no cambia este contrato») y `specs/eval`, y reportar las mediciones antes y después.
7. (Condicional) el piso de relevancia del paso 8, con preguntas sin respuesta en el golden set de docs.

### No-alcance

- Reranker, modelos de embedding distintos o código-específicos.
- Cambiar la expresión léxica, el prefijo o el ranking de las **memorias** (21 y 34).
- Cambiar el troceo de Markdown o el tope de tokens de los pasajes.
- Cambiar `RRF_K`, `CANDIDATES_PER_CHANNEL` o el intervalo de refresco.
- Un watcher del filesystem o una herramienta `index_project`.
- Subir el umbral del golden set de 0,70: ver decisiones.

## Decisiones abiertas

Son del usuario.

- [ ] **Alcance de la expresión léxica.** ¿Sólo docs (recomendado) o compartida con las memorias, midiendo las dos y coordinando con 34?
- [ ] **Forma del paso 1.** ¿Excluir por contenido (cubre docs futuros) o por lista de rutas (más simple)?
- [ ] **Umbral de salida.** Hoy: Hit@3 0,27 total, 0,20 en docs, 0,40 en código. El 0,70 y el MRR 0,50 salieron de la propuesta de project-indexing y no se cumplen ni con lo medido hasta ahora (0,40). ¿Se mantienen como objetivo, o se fija uno intermedio (por ejemplo Hit@3 0,50) sobre el set ampliado?
- [ ] **Esperar a eval-rigor.** ¿Se hace el paso 1 ya y se esperan 2 a 5 hasta tener el set grande, o se miden con 15 preguntas aceptando que sólo se detectan diferencias grandes?
- [ ] **Rareza.** ¿Heurística pura (identificadores y largos primero) o la frecuencia real con `fts5vocab` (una tabla virtual más, o una consulta por token)?
- [ ] **Formas partidas.** ¿Texto preprocesado (sin migración de esquema, recomendado) o columna FTS propia con su peso (migración V13)?
- [ ] **Prefijo de docs.** ¿Se prueba en un worktree y se productiviza sólo si gana por un margen fijado de antemano, o se descarta el paso?
- [ ] **Listas de palabras vacías.** ¿Quién las mantiene y dónde viven? Una constante en `project_docs` es lo más simple.
- [ ] **Reranker.** ¿Descartado en este cambio y reabierto sólo con una medición que cumpla la latencia?
- [ ] **Piso de relevancia.** ¿Coseno del canal denso contra el mismo `cosine_floor` de las memorias (un número calibrado sobre otro corpus), presencia en los dos canales para los rangos bajos, o ninguno hasta tener preguntas sin respuesta en el set de docs?

## Cómo se mide

Tests RED primero, uno por paso:

1. **Paso 1.** Un corpus de prueba con un doc que cita una pregunta textual: hoy sale primero; con la exclusión no está en el corpus. Un golden set cuyo pasaje esperado es el único doc que cita la pregunta falla en `check_corpus`. El reporte cuenta los excluidos.
2. **Paso 2.** Función pura, con tabla de casos: pregunta de 19 palabras conserva las raras y los identificadores y descarta «de», «el», «the»; una consulta toda de palabras vacías cae a la expresión de hoy y no a `None` (hoy una consulta así igual busca); `CHUNK_MAX_TOKENS` sobrevive al filtro; AND sin resultados cae a OR. En `search`: un pasaje con las palabras raras de la pregunta gana a uno que sólo comparte palabras vacías.
3. **Paso 3.** `memoryType` y «memory type» y `memory_type` se encuentran entre sí (hoy `memoryType` no encuentra `memory_type`); el cuerpo mostrado no cambia; el texto que se embebe no cambia.
4. **Contrato.** Un índice armado con la cadena de `model_id` vieja se reindexa entero con la nueva, y las memorias quedan igual.
5. **Base sembrada en la versión anterior** (regla de `config.yaml`, aplica a 3, 4 y 5): un índice con la huella vieja abre, sigue contestando búsquedas mientras se reindexa y no pierde pasajes; si hubo migración V13, también el test de checksum de migraciones fijados (`published_migration_checksums_are_pinned`).

`rag-eval`: **obligatorio**, un paso a la vez, nunca dos juntos.

- Paso 1: se corre `rag-eval --real --docs` antes y después y se reporta la línea base nueva con la cantidad de docs excluidos. Esa línea es la que se usa para el resto.
- Pasos 2 a 5: una corrida por paso contra la anterior, con **umbrales fijados antes de correr**. Propuesta: no empeora ninguna pregunta que ya acertaba en el top 3 y gana al menos 2 preguntas en el total (con 15 preguntas es la mínima diferencia que un azar no explica fácil; con el set de eval-rigor se reemplaza por una prueba pareada).
- Si el alcance de 2 es compartido: `rag-eval --real --diff` contra el ancla de memorias debe dar cero filas distintas en R@5 y MRR; un empeoramiento descarta compartirlo.
- Se reporta Hit@1/3/5 y MRR@10 por separado en docs y código, y el tiempo de indexado (645 s en debug es la referencia, `verify.md:188`) y de una búsqueda.
- No hay archivo de resultados de `--docs` (imprime sin escribir: `verify.md:108`); poner uno es parte del paso 1 para poder compararlos con `--diff`.

## Riesgos

- **Mejora falsa por una regla de medición sesgada.** Si se mide un paso antes de arreglar el 1, el pasaje que cita las preguntas distorsiona el resultado. Por eso el 1 va primero.
- **Medir con muy pocas preguntas.** 15 preguntas, una pregunta son 6,7 puntos. Un cambio real de dos preguntas y un azar se parecen. Sin eval-rigor sólo se pueden afirmar diferencias grandes.
- **Sobreajustar a los idiomas y a este repo.** La lista de palabras vacías y las preguntas salen del mismo repo y están escritas por la misma persona. Una mejora en 15 preguntas puede no moverse en otro proyecto.
- **Excluir palabras que importan.** Un token corto que es un nombre real (`id`, `db`, `rs`) o un término técnico igual a una palabra vacía. Por eso la excepción de identificadores y la prueba de que un identificador corto sobrevive.
- **Reindexado a todos los proyectos.** El contrato nuevo reindexa cada proyecto registrado (645 s para este repo en debug), y durante el refresco por doc un índice mezcla dos formas. Es el mismo mecanismo y el mismo riesgo que un cambio de modelo; se acota con la prueba 5.
- **Tocar el canal léxico de las memorias sin querer.** `fts_match_expression` es de las dos búsquedas. Dejar la función como está y crear la de docs aparte evita mover la ancla.
- **El prefijo puede empeorar.** Con CLS el prefijo asimétrico bajó recall@10 de 0,8261 a 0,6957 en memorias (`embedder.rs:102-105`). Para docs puede pasar lo mismo; se mide antes de productivizar.
- **Ninguna garantía sobre el objetivo 0,70.** Lo medido hasta ahora (0,40 con el léxico burdo, 0,50 con un reranker de 10 s por pregunta) no llega. Si 1 a 3 dejan Hit@3 por debajo del objetivo que se acepte, el cambio puede cerrar con una mejora parcial y la decisión sobre un modelo mayor queda para después.
