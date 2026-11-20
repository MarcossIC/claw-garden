# docs-query-language — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 | S el enfoque A; M el B | — (si toca el texto del protocolo, comparte archivos y contratos fijados con [contract-diet](../contract-diet/exploration.md) (133), que los achica) | exploración (borrador) |

**Tesis:** `search_project` busca por significado con un modelo multilingüe y por término con FTS5, pero el protocolo que lee el agente le pide buscar en español las memorias y no dice nada del idioma para `search_project`, y el aviso que propone repetir la búsqueda en el otro idioma existe sólo para memorias. Cuando una pregunta en español cae sobre pasajes en inglés (un repositorio en inglés, o el código de cualquier repositorio, cuyos identificadores son en inglés), los dos canales pierden a la vez: el denso por el cruce de idioma, ya medido en memorias con este mismo modelo (es>en, R@5 6/19 con la similitud sola), y el léxico porque las palabras de la pregunta no están en el pasaje. La prueba a mano sobre un repositorio público (0 de 3 en dos preguntas en español, 3 de 3 en una en inglés) y la partición por idioma de los resultados guardados del golden de docs (acierto en el top 3: 4 de 18 con el mismo idioma, 0 de 23 entre idiomas) apuntan a eso, pero las dos tienen una confusión que este cambio separa antes de tocar nada: la pregunta que acertó también traía los términos del código. El cambio empieza midiendo gratis; si el cruce es real, lo más barato es una línea que le diga al agente con qué vocabulario preguntar, y sólo si eso no alcanza vale guardar el idioma de los pasajes.

## Premisas que se verificaron y cuáles no eran del todo ciertas

Se leyó el código en `develop` (0.90.0, `d658c8a9`). **No se corrió nada**: ni el daemon, ni `rag-eval`, ni un build. La partición del golden sale de archivos de resultados ya guardados.

| Premisa | Resultado |
|---|---|
| El protocolo le dice al agente que busque en español | **Cierta, con matiz.** La línea está en la lista «Cómo escribir un save» y habla de memorias: «Escríbela en español, y busca en español también. Una memoria guardada en un idioma y buscada en otro suele perderse» (`crates/clawg-cli/src/mcp/protocol.es.md:86-88`; en inglés, `protocol.md:17-18`). El párrafo de `search_project` (`protocol.es.md:5-8`, `protocol.md:5-8`) no nombra ningún idioma. Que el agente generalice la línea de memorias a los docs es una hipótesis: una persona que escribe en español probablemente pregunta en español sin que nada se lo pida |
| El aviso `other_language` existe sólo para memorias | **Cierta.** Lo arma `other_language_line` (`crates/clawg-cli/src/mcp/tools.rs:139`) y lo agrega `with_other_language` (`:128`, llamado en `:119`, dentro de `format_search`); el daemon cuenta las memorias en el otro idioma (`other_language`, `crates/clawg-cli/src/daemon/api/memories.rs:489`; campo en `:452` y `:466`). `format_docs` (`tools.rs:690`) no lo tiene, y `DocsSearchRequest` no lleva idioma (`crates/clawg-cli/src/daemon/api/docs.rs:148-154`). Su umbral, `FEW_SHOWN = 3` (`tools.rs:125`), está documentado como «Not measured yet» |
| El detector de idioma no usa modelo | **Cierta.** Cuenta palabras de función y marcas del español, sin código ni identificadores (`crates/clawg-core/src/memory/retrieval/language.rs:76-130`: `MIN_EVIDENCE = 3`, `TERSE_PROSE = 8`, listas `ENGLISH` y `SPANISH`, `detect` en `:106`). Responde `Some(Lang::Es)`, `Some(Lang::En)` o `None`: **sólo conoce inglés y español**, y un texto corto, con menos de 3 puntos de evidencia, da `None` |
| Las memorias guardan `lang` | **Cierta.** `ALTER TABLE memory ADD COLUMN lang TEXT CHECK (lang IN ('en', 'es', 'und'))` (`crates/clawg-core/migrations/V22__memory_lang.sql:15`); se llena al insertar con `language::detect` (`crates/clawg-core/src/memory/ingest/store_insert.rs:485`) |
| El índice de docs no guarda el idioma | **Cierta.** `project_doc_file` y `project_doc_chunk` tienen `path`, `breadcrumb`, `body`, `content_hash`, `model_id`, `chunks`, `indexed_at` y nada más (`crates/clawg-core/migrations/V12__project_docs.sql:11-31`), y ninguna otra migración nombra las tablas `project_doc_*`. Un pasaje se podría clasificar con `detect`, pero hoy no se hace |
| El ranking es RRF k=60 sobre KNN 50 y FTS5 50, y siempre devuelve `limit` pasajes | **Cierta.** `RRF_K = 60.0` (`crates/clawg-core/src/memory/project_docs/mod.rs:44`), `CANDIDATES_PER_CHANNEL = 50` (`:39`), `fuse` suma `1 / (RRF_K + rank)` por canal y corta en `limit` (`:560-571`). El canal denso pide `k = 50` vecinos del proyecto sin piso de distancia, y ni siquiera selecciona la distancia (`SELECT rowid … ORDER BY distance`, `:597-600`), así que en cuanto el índice tiene `limit` pasajes devuelve `limit`, relevantes o no. `limit` es 5 por defecto y 20 como máximo (`crates/clawg-cli/src/daemon/api/docs.rs:136-144`). `openspec/ideas.md:82` ya lo anota como «sin piso de relevancia» |
| El puntaje RRF no da una señal absoluta de «resultados flojos» | **Cierta.** Es una suma de dos rangos: con la respuesta en el primer lugar de los dos canales el puntaje es el mismo `2/61` haya o no una respuesta buena (`mod.rs:564-566`), y nada compara el pasaje con la pregunta. Lo único absoluto que el canal denso podría dar es la distancia del primer vecino, que hoy se descarta |
| Los identificadores del código son en inglés aunque el repositorio sea en español | **Cierta para los identificadores y a medias para el resto.** El detector parte de que «in this kind of store they are English whatever the prose around them is» (`language.rs:4-6`). Los comentarios y los literales de un repositorio en español sí estarán en español, y el troceo por definiciones los conserva dentro del pasaje (`crates/clawg-core/src/memory/project_docs/code.rs:137`, `:216`). Un pasaje de código es **mixto**, no «inglés» a secas |
| El cruce es→en ya se midió en memorias con este modelo: R@5 6/19 | **Cierta, pero es la similitud sola.** `openspec/config.yaml` (bloque `context`, «By query>row language pair») da, con `multilingual-e5-small`: similitud R@5 es>es 18/20, en>en 6/7, **es>en 6/19**; y `search_memory`, que suma el canal léxico, **es>en 11/19**. Con un lado traducido llega a 19/19 y 18/19, y «no E5 prefix, cosine floor or lexical channel closes it». Los bloques `by_pair` están en `docs/eval/results/2026-10-17-real-english-recall-base.json` (fuera de git, `docs/eval/**` en `.gitignore`); el diseño de la pareja, en `openspec/specs/eval/spec.md:41`. Para docs, que también tiene canal léxico, el número comparable es el segundo, no el primero |
| El cambio archivado `docs-search-quality` concluyó que falla el canal denso | **A medias.** Cerró con Hit@3 ≈ 0,10 y concluyó otra cosa: que el golden sólo puede medir el canal denso, porque cada pregunta se escribió sin las palabras del pasaje que la responde y «el canal léxico no puede encontrar la respuesta de ninguna pregunta por sus palabras de contenido» (`openspec/changes/archive/2026-10-31-docs-search-quality/verify.md`, «Por qué el paso 2 no puede pasar su puerta con este golden»). No evaluó el idioma: el prefijo `passage: ` tampoco movió ninguna pregunta de docs (`verify.md`, «`E5_PASSAGE_PREFIX`»). La hipótesis del cruce de idioma es nueva |
| El golden de docs mezcla preguntas en español y en inglés, con destinos en los dos idiomas | **Cierta.** 40 casos (`crates/clawg-eval/golden/project-docs.json`): 22 preguntas en español y 18 en inglés; los destinos son `README.md` y `openspec/**` (español), `docs/reference/*` (inglés) y `crates/**` (código). **No tiene el campo `query_lang`** que el golden de memorias sí lleva, así que `rag-eval --diff` no corta docs por pareja de idiomas |
| Los resultados guardados permiten ver el acierto por caso | **Cierta.** Cada archivo `*-real-docs-*.json` de `docs/eval/results/` trae `per_case` con `id`, `code`, `rank` (puesto dentro de los 10 primeros, `null` si no está) y `top`. El de `2026-11-13-real-docs-memory-module-layout.json` corresponde al golden de hoy (sha256 `64a8973181ae931d…`, 40 casos). Los de octubre y principios de noviembre son de otra versión del golden (`0749d71b…`, `d7a573b8…`, `20701930…`), y sus preguntas no se guardan en el resultado |

## La evidencia que hay

**1. La partición del golden (hecha al escribir esto, a mano).** Con `per_case` de `2026-11-13-real-docs-memory-module-layout.json` y el idioma de cada pregunta y de cada destino (README y openspec, español; `docs/reference` y `crates`, inglés; un destino de cada idioma, «mixto»):

| Pregunta > destino | Casos | Hit@3 | Hit@10 |
|---|---|---|---|
| en > en | 9 | 3 | 3 |
| es > es | 9 | 1 | 2 |
| en > es | 8 | 0 | 0 |
| es > en | 12 | 0 | 0 |
| mixto (1 en, 1 es) | 2 | 0 | 0 |

Mismo idioma: 4 de 18 en el top 3 (5 de 18 en el top 10). Idiomas distintos o mixto: 0 de 23. Un test exacto de Fisher sobre esas dos filas da p ≈ 0,03 para el top 3 y p ≈ 0,01 para el top 10. **Es una lectura, no una medición:** el idioma de cada pregunta lo asignó una regla simple (signos `¿`, acentos, palabras de función) y se miró el resultado al armar la tabla, así que no vale como umbral fijado antes. Y deja a la vista que **el idioma no es todo**: con el mismo idioma sólo aciertan 4 de 18; el resto cae por lo que ya dijo `docs-search-quality` (el golden evita las palabras del pasaje).

**2. Una prueba a mano sobre un repositorio público (no se puede reproducir desde este repositorio).** El 2026-11-20 se indexó con `clawg projects add` un repositorio público en inglés, de código y docs, y se hicieron tres preguntas con respuesta conocida:

- La fusión de los resultados densos y BM25 en la búsqueda híbrida, en español: 0 de 3 pasajes útiles. Trajo un script de ejemplos de rendimiento, el módulo de puntaje léxico (sólo imports) y una página de ejemplos; lo que coincidió fueron términos sueltos como BM25, por el canal léxico.
- Cómo se les pone nombre a los tópicos del grafo semántico, en español: 0 de 3.
- «How are dense and sparse scores fused in hybrid search? convex combination rrf», en inglés: 3 de 3 (las dos piezas del módulo de búsqueda híbrida y el método de búsqueda de la clase base).

**La confusión.** La pregunta que acertó difiere de las que fallaron en **dos** cosas: el idioma y que agrega términos exactos del código (`convex combination`, `rrf`). El canal léxico pudo haberla resuelto solo, sin que el denso cruzara nada. Con tres preguntas no se separan. Es lo primero que mide la puerta.

## Enfoques

| Enfoque | Qué da a clawg y a quien lo usa | Contrapartidas |
|---|---|---|
| **A. Una línea de protocolo, o de la descripción de la herramienta:** buscar en `search_project` con el vocabulario de los archivos (el idioma de la documentación y los nombres del código); si una pregunta en español no trae nada útil, repetirla en inglés con los términos del código | Costo de código casi cero (texto en `protocol.md`, `protocol.es.md` y, si va en la descripción, `server.rs:2033-2042` y `tools_es.rs`, más los contratos fijados con `CLAWG_BLESS=1`). Quien pregunta en español en un repositorio en inglés recupera, en una segunda llamada, lo que hoy no ve. Redactada como «con los términos que usa el código», no como «en inglés», sirve también al repositorio en español con identificadores en inglés | Más texto. El bloque del protocolo viaja en cada pedido (11.271 caracteres en español, [contract-diet](../contract-diet/exploration.md), «Inventario medido») y 133 trabaja para achicarlo; la descripción de la herramienta sólo se paga al cargarla (`search_project`, 1.619 caracteres), pero una herramienta cargada se queda en cada pedido. Depende de que el agente la siga, y no hay dato de cuánto. Cuesta una segunda llamada completa de `search_project` cuando falla |
| **B. Un aviso como `other_language`, para docs:** guardar el idioma de cada pasaje o archivo al indexar (con `detect`) y, si la consulta está en otro idioma que la mayor parte del corpus y los resultados son flojos, agregar la línea al resultado | La línea llega **sólo** cuando hace falta, y el agente no tiene que acordarse de nada. Reutiliza el detector, la forma del aviso (`OtherLanguage`, `other_language_line`) y el patrón de la V22 | Es el más caro: una migración (columna en `project_doc_chunk` o en `project_doc_file`), reindexar o rellenar lo ya indexado, y la prueba de datos previos que exige `openspec/config.yaml`. **La señal de «flojos» no existe:** ver la decisión D2. `detect` sólo distingue dos idiomas y responde `None` con poca evidencia, que es una consulta corta como un nombre de función. Un repositorio de código tiene casi todos los pasajes mixtos (`und`), y «la mayor parte del corpus» pierde sentido. Sin la señal de flojedad, el aviso saldría en toda consulta en español sobre un corpus en inglés, también en las que acertaron, y se vuelve ruido |
| **C. Que el daemon traduzca la consulta** | Ninguno que valga: sería la corrección más completa del cruce (con un lado traducido, memorias llega a 19/19 y 18/19) | **Descartado.** clawg no llama a ningún modelo de lenguaje y no debe: es local, sin costo por uso y sin enviar texto a un tercero, y eso es lo que lo hace un almacén que la persona controla. Un traductor local es un modelo nuevo (otro contrato de embeddings, memoria y tiempo de arranque) para un problema que A resuelve con una línea |
| **D1. Hacer visible lo que el canal denso ya sabe:** seleccionar la distancia del primer vecino en `search` (`mod.rs:597`) y devolverla | Es la única señal absoluta que hay para decidir en B si un resultado es flojo, y sirve a cualquier otro uso (un piso de relevancia, `ideas.md:82`) | Cambia la forma de `DocHit` y del cuerpo de la respuesta. Con el modelo de memorias el coseno se amontona (el piso 0,79 de `openspec/config.yaml` lo muestra), así que el umbral hay que calibrarlo, y un pasaje de código mixto puede dar distancias engañosas. Va después de la puerta, no antes |
| **D2. Detectar «sin acuerdo» entre canales:** en `fuse` (`mod.rs:560`), contar cuántos ids del top `limit` aparecen en las dos listas; con cero, avisar | No necesita columna ni migración. Las listas ya existen dentro de la función | Es una heurística que nadie midió. Con cero ids en común tampoco se prueba que la causa sea el idioma. Se mediría con el mismo resultado por caso, pero `per_case` no guarda las listas, así que pide un cambio en `rag-eval --docs`, que ya no es gratis |
| **D3. Sólo medir y documentar** | Cero costo | Deja sin resolver lo que vive hoy quien tiene un repositorio en inglés |

## Recomendación

**A**, redactada por vocabulario y no por idioma, **si la puerta muestra que el cruce existe**; B y las dos D no se tocan hasta que A se pruebe y no alcance. Razones, por lo que le dan a clawg:

1. El problema, si es el del cruce, ya tiene un remedio medido en memorias: tocar el idioma de un lado (19/19 y 18/19). Pedirle al agente que lo haga es lo único que está a su alcance sin un modelo nuevo.
2. B pide una señal (D1 o D2) que hay que construir y calibrar antes de saber si discrimina; A pide sólo texto. Si A no alcanza, esa medición es el paso siguiente, con la distancia del primer vecino como candidata.
3. Si la puerta dice que el cruce no pesa, A no se escribe y el protocolo no crece: el resultado es «cerrado, no hacer», y se anota en «Ideas evaluadas».

## Puerta de medición (gratis: sin código de producto, sin API de pago, sin store real)

**Qué decide:** si el cruce de idioma entre pregunta y pasaje explica fallas de `search_project` por sí solo, o si lo que falla es el vocabulario que la pregunta comparte con el pasaje. No toca el store de la persona: el índice del repositorio de prueba lo arma `clawg projects add` en un perfil de desarrollo, y la partición sólo lee archivos de `docs/eval/results/`.

### Paso 1 — partir el golden por pareja de idiomas

1. Escribir, en un script fuera del producto, una tabla `id → (idioma de la pregunta, idioma del destino)` para los 40 casos de `crates/clawg-eval/golden/project-docs.json`, con la regla de la sección anterior, y revisarla a mano. Fijarla **antes** de mirar los puestos.
2. Leer `per_case` de `docs/eval/results/2026-11-13-real-docs-memory-module-layout.json` y contar Hit@3 y Hit@10 por celda (como la tabla de arriba).
3. Repetir con las corridas `2026-11-01-real-docs-a2-base-*.json` y `2026-10-31-real-docs-a1-base-*.json` (golden de sha `0749d71b…`), recuperando ese golden con `git log -- crates/clawg-eval/golden/project-docs.json`. Sirve para ver si la diferencia se repite y no depende de una corrida.

**Qué dice el paso 1 por sí solo:** nada decisivo. La partición está confundida con el tipo de destino (el inglés es código y `docs/reference`; el español es `README.md` y `openspec`) y con que el golden evita las palabras del pasaje. Sólo puede **descartar**: si en las corridas del paso 3 el acierto entre idiomas **no** es menor que con el mismo idioma, la hipótesis cae y el cambio se cierra sin más.

### Paso 2 — la tanda pareada sobre el mismo repositorio de prueba

Un repositorio público en inglés, de código y docs, fijado antes de preguntar (puede ser el de la prueba a mano de arriba), indexado una vez en un perfil de desarrollo.

- **16 preguntas** sobre cosas cuya ubicación se conoce, anotada **antes de preguntar** como una lista de rutas y encabezados o definiciones que cuentan como útiles. Un tercio sobre docs, dos tercios sobre código.
- **Tres formas por pregunta**, escritas antes de ejecutar ninguna: **(es)** en español natural; **(en)** la misma pregunta en inglés natural, sin agregar nombres del código; **(en+t)** la (en) con los términos del código que la persona sabría (nombres de clases, siglas).
- Se corre `search_project` con `limit` 5 (el valor por defecto) por cada forma y se anota si hay un pasaje útil entre los **3 primeros** y entre los 5. Cada forma se evalúa sin mirar el resultado de las otras.
- Ningún archivo del repositorio de prueba cambia entre las 48 búsquedas.

**Métrica:** Hit@3 por forma, y los pares discordantes entre (es) y (en): casos donde (en) acierta y (es) no, contra los que van al revés.

**Umbrales, fijados antes de medir:**

- **El cruce existe** si (en) acierta y (es) no en **al menos 6** de las 16 preguntas y ninguna va al revés (es la regla de `docs/reference/testing.md`: con menos de 6 casos que se mueven para un lado, «no distinguible», nunca «no hay diferencia»). Se escribe A.
- **El cruce no existe** si hay 2 o menos pares con (en) acertando y (es) no. Se cierra el cambio sin código ni texto.
- Entre 3 y 5 pares: **no concluyente**. Se amplía a 30 preguntas antes de decidir, no se escribe A.
- **El vocabulario pesa más que el idioma** si (en+t) acierta en al menos 4 preguntas más que (en). Entonces A se redacta por **términos del código** («pasá los nombres que usa el código») y no por idioma; si además (en) no gana a (es), vale sólo esa redacción.
- **Mínimo de datos:** que al menos 8 de las 16 preguntas tengan un pasaje útil entre los 5 primeros en alguna de las tres formas. Si ninguna forma encuentra nada en la mayoría, la tanda mide un índice incapaz y no el idioma, y el resultado es «no concluyente».

**Límites:** un solo repositorio y un solo modelo (`multilingual-e5-small`), con preguntas escritas por quien conoce el repositorio; 16 preguntas dan pocos pares; y el golden de docs se escribió para evitar las palabras del pasaje, así que lo que muestra sobre el canal léxico no se traslada a una pregunta real.

**Costo:** unas horas de una persona. No cuesta créditos ni toca el store de la persona. El script del paso 1 son unas 40 líneas y no entra al producto.

## Decisiones abiertas

| # | Decisión | Opciones | Qué la decide |
|---|---|---|---|
| D1 | ¿Dónde va la línea de A? | (a) El párrafo de `search_project` del protocolo, que se paga en cada pedido; (b) la descripción de la herramienta (`server.rs:2033-2042`), que se paga al cargarla; (c) el aviso en el resultado, que sólo se paga cuando sale (es B) | Los caracteres de cada lugar contra la regla de 133, y si el agente lee la descripción de una herramienta diferida (contract-diet midió el costo de cargarla, no la conducta) |
| D2 | Si hay B, ¿cómo se decide «flojos»? | (a) Distancia del primer vecino con un umbral calibrado (hay que seleccionarla en `search`, `mod.rs:597`); (b) cero ids en común entre los dos canales en el top `limit` (`fuse`); (c) ninguna: avisar siempre que la consulta esté en otro idioma que la mayor parte del corpus. El puntaje RRF no sirve: no es absoluto | Una medición propia que no existe: qué señal separa los aciertos de los fallos. Con 4 aciertos y 36 fallos del golden de hoy casi no hay positivos; la tanda del paso 2 da más |
| D3 | ¿En qué se guarda el idioma, si hay B? | (a) `project_doc_chunk` (un valor por pasaje, un código mixto queda `und`); (b) `project_doc_file` (uno por archivo, más barato, pero un archivo de código mezcla); (c) no guardarlo y decidirlo al buscar, sobre los pasajes devueltos | Lo que cueste reindexar: el memo de vectores no cambia, pero la columna nueva obliga a una migración con la prueba de datos previos de `openspec/config.yaml` |
| D4 | La consulta que `detect` no sabe clasificar (poca evidencia, o un tercer idioma) | Sin aviso; o tratarla como inglés | Cuántas consultas reales de `search_project` dan `None`; hoy no se cuentan |
| D5 | ¿Redactar por idioma o por vocabulario? | «Repetí en inglés» o «repetí con los términos que usa el código» | El resultado de (en+t) contra (en) en la tanda: es el único dato que separa las dos |

## Cómo se mide

- **Métrica:** Hit@3 de `search_project` con `limit` 5 sobre la tanda de 16 preguntas, por forma (es, en, en+t), con el pareo de arriba.
- **Umbral:** el de la sección anterior: 6 o más pares en un sentido y ninguno en el otro para escribir A; 2 o menos para cerrar; 3 a 5 no concluyente.
- **Si se escribe A:** el efecto de la línea se mide con la misma tanda corrida por el agente real con el protocolo nuevo y con el viejo, que sí cuesta crédito de API, la autoriza quien lo corre y **no** forma parte de esta puerta. Un cambio de texto no mueve el golden de docs: `rag-eval --docs` no lo ve.
- **Si se llega a B:** `rag-eval --docs` comparado con `--diff` contra la línea de base antes de que sea la conducta por defecto (`openspec/config.yaml`, `rules.design`), un test con datos previos para la migración, y un conteo de cuántas veces sale el aviso cuando el resultado ya era bueno.

## Riesgos

- **Ruido de protocolo.** Cada línea que se suma al bloque viaja en cada pedido y trabaja en contra de 133. Si A va en la descripción de la herramienta, el riesgo es que el agente no la relea después de cargarla.
- **Una conclusión sobre un solo repositorio.** El resultado puede ser de ese repositorio (nombres largos y en inglés, mucho código) y no de cualquiera. Se repite sobre un segundo repositorio, uno en español, si el primero abre la puerta.
- **La confusión entre idioma y vocabulario**, que es la principal: sin la forma (en+t) se escribiría una línea que dice «en inglés» cuando lo que falta son los nombres del código.
- **Un aviso que acierta poco.** El de memorias tiene el umbral `FEW_SHOWN` sin calibrar (`tools.rs:121-125`); repetirlo en docs hereda ese defecto y, con `limit` siempre completo, no hay «pocos resultados» que lo dispare.
- **Un repositorio en español con identificadores en inglés** cae justo en el caso mixto: las preguntas en español encuentran la prosa y fallan en el código. La línea de A tiene que cubrir las dos cosas.
- **La partición del paso 1 se leyó antes de fijar los umbrales** (aquí, al escribir). Por eso decide sólo para **descartar**, no para aprobar.

## Esfuerzo e impacto

- **La puerta:** S. Un script de unas 40 líneas, un índice del repositorio de prueba (el modelo ya está en el perfil de desarrollo) y 48 búsquedas.
- **A:** S. Una línea en `protocol.md` y `protocol.es.md` o en la descripción (`server.rs` y `tools_es.rs`), los contratos fijados (`agent_contract*.json`) con `CLAWG_BLESS=1`, y un test de que el texto sale en los dos idiomas. Sin migración.
- **B:** M. Una migración con su prueba de datos previos, relleno de lo ya indexado, una señal calibrada (D2) y las dos redacciones del aviso.
- **Impacto:** alto para quien trabaja en español sobre repositorios en inglés, que es el uso que el protocolo en español asume, y nulo para quien pregunta en el idioma del repositorio. En el objetivo 3, mejora que el agente reciba del índice lo que clawg ya tiene guardado y hoy no le devuelve.

## Fuentes

- Código: `crates/clawg-cli/src/mcp/protocol.es.md:5-8,86-88`, `crates/clawg-cli/src/mcp/protocol.md:5-8,17-18`; `crates/clawg-cli/src/mcp/tools.rs:119,125,128,139,690`; `crates/clawg-cli/src/mcp/server.rs:2033-2042`; `crates/clawg-cli/src/daemon/api/memories.rs:452,466,489`; `crates/clawg-cli/src/daemon/api/docs.rs:136-154,253`; `crates/clawg-core/src/memory/retrieval/language.rs:4-6,76-130`; `crates/clawg-core/src/memory/ingest/store_insert.rs:485`; `crates/clawg-core/src/memory/project_docs/mod.rs:34-44,560-623`, `code.rs:137,216`; `crates/clawg-core/migrations/V12__project_docs.sql`, `V22__memory_lang.sql`.
- Mediciones guardadas: `docs/eval/results/2026-11-13-real-docs-memory-module-layout.json` (`per_case`); `docs/eval/results/2026-10-17-real-english-recall-base.json` (`by_pair`, fuera de git) y su resumen en `openspec/config.yaml`.
- Golden: `crates/clawg-eval/golden/project-docs.json`.
- Cambios y notas: [docs-search-quality](../archive/2026-10-31-docs-search-quality/verify.md); [contract-diet](../contract-diet/exploration.md); `openspec/ideas.md:82`; `openspec/specs/eval/spec.md:41`; `docs/reference/testing.md` (regla de los 6 casos).
- La prueba a mano sobre un repositorio público en inglés se hizo fuera de este repositorio y no se reprodujo aquí.
