# lexical-stemming — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S para medir (un censo sin código de producto y una corrida de `--lexical-only`); M si la puerta abre (una migración y una dependencia nueva) | **[memory-score-fusion](../memory-score-fusion/exploration.md) (136) para la medición de punta a punta**, no para las dos primeras etapas. Se coordina con [search-index-ports](../search-index-ports/exploration.md) (110) y con el índice de docs (V12, `project_docs/mod.rs`) | exploración (borrador) |

**Tesis:** el canal léxico de las memorias compara formas exactas de palabras, y V8 descartó `porter` con un argumento correcto: reemplazar el tokenizer por uno de un solo idioma deja sin tratar la mitad española del corpus y daña sus tokens. Esa objeción vale contra **reemplazar**, no contra **sumar**: indexar, además del texto tal cual, una segunda forma reducida a raíces de los dos idiomas, y buscar en las dos: una forma exacta que se conserva y una de raíces que se suma. La pregunta de producto no es si se puede, sino **cuánto vale**: hoy las palabras comunes de una consulta sólo proponen candidatos que después puntúa el coseno, así que mejorar el recall léxico puede no mover nada, que es lo que pasó con los tokenizers que ya se probaron. **No se decide hoy.** Hay un censo gratis que dice cuántos casos del golden fallan por palabras sólo por una inflexión, una corrida de `--lexical-only` que no depende de la fusión, y una medición de punta a punta que espera a 136.

**Qué se verificó y qué no.** El código y las migraciones se leyeron en `develop` (0.90.0, `d658c8a9`). También se miró el esquema de `docs/eval/golden.json` y `docs/eval/golden-expanded.json`, sin medir nada sobre ellos, y se comprobó con un SQLite aparte (no el del workspace) que un `CREATE VIRTUAL TABLE ... tokenize='x'` con un tokenizer desconocido falla con `no such tokenizer`, y que una consulta por prefijo (`"decisi"*`) encuentra `decisión` en una tabla con `remove_diacritics 2`. **No se corrió ninguna herramienta de evaluación, ni cargo, ni se tocó ningún perfil de datos.** **No se verificó** qué raíz da un stemmer de Snowball a cada palabra de los ejemplos (no hay ninguno instalado y agregar uno es parte de lo que la puerta decide): la lectura de abajo sobre `decisión` es una expectativa a comprobar en el censo, no un hecho.

---

## El problema, verificado contra el código

| Qué | Dónde | Efecto |
|---|---|---|
| El tokenizer descarta `porter` por escrito | `crates/clawg-core/migrations/V8__memory_fts.sql:31-35`: «stems English only and would leave the Spanish half of the corpus unstemmed while damaging its tokens» | Es la decisión que se revisa; el argumento es válido contra reemplazar el tokenizer |
| `memory_fts` es de contenido externo y con tokenizer único | `V8__memory_fts.sql:1-3`, `:36-44` (`content='memory'`, `unicode61 remove_diacritics 2`) | El texto indexado es `memory.content` tal cual; no hay una segunda copia que pueda discrepar |
| Un índice de contenido externo exige que cada columna exista en `memory` con el mismo nombre | `V8__memory_fts.sql:16-17` (dicho de las `UNINDEXED`, vale para cualquier columna) | Una columna de raíces nueva necesita su columna en `memory`, o una tabla que guarde su propio texto |
| El borrado resta postings con los tokens exactos | `V8__memory_fts.sql:48-52`, triggers `:63`, `:73`, `:84` | Lo indexado tiene que ser reproducible al borrar: un stemmer que cambie de versión entre el alta y la baja corrompe el índice |
| `content` es de una sola escritura | `V8__memory_fts.sql:90` | Una columna derivada calculada al insertar no se desactualiza |
| Hay un segundo escritor del índice además de los triggers | `RESTORE_TEXT_INDEX` (`crates/clawg-core/src/memory/revision/revival.rs:51`), el borrado de eval (`crates/clawg-core/src/eval_api.rs:1036`) | Cada uno lista las columnas de `memory_fts` a mano |
| El índice de docs usa el mismo tokenizer y guarda su propio texto | `V12__project_docs.sql:45-49`; DELETE más INSERT en la misma transacción (`crates/clawg-core/src/memory/project_docs/mod.rs:1137`, `:1190`) | No tiene triggers ni contenido externo: es el lugar más barato para ensayar |
| La consulta es un OR de tokens entre comillas, sin raíz ni prefijo | `fts_match_expression` (`crates/clawg-core/src/memory/retrieval/search.rs:643`), `fts_tokens` (`:656`), tope `MAX_LEXICAL_TOKENS = 16` (`:623`) | «decidió» no encuentra «decidido» ni «decisión»; «deployed» no encuentra «deployment» |
| Los MATCH de producción sobre `memory_fts` no nombran columna | `LEXICAL_CANDIDATE_SQL` (`search.rs:690-694`), `IDENTIFIER_MATCH_SQL` (`:852-856`), el filtro de texto del listado (`crates/clawg-core/src/memory/retrieval/listing.rs:174`) y el de `relocate` (`crates/clawg-core/src/memory/projects/relocate.rs:563`) | Con una columna más, un MATCH sin filtro de columna buscaría en las dos: hay que fijar cada uno a `content` o a las raíces a propósito |
| Los identificadores se tratan aparte | `identifier_tokens` (`search.rs:813`), `is_identifier` (`:826`), `holds_identifier` (`:837`); `identifier_matches` consulta con AND y comprueba la fila a mano (`:875-915`) | Es el único canal con bonus: `lexical_bonus = 1.0` (`crates/clawg-core/src/memory/retrieval/score.rs:181`, `:259`) |

### Lo que el canal léxico vale hoy

Esto decide el orden con 136, así que se verificó primero:

- **El bonus de 1,0 es sólo de identificadores.** `lexical_match` sale de `lexical_matches` (`search.rs:1125`), y ese conjunto lo llena `identifier_matches` (`search.rs:1208-1214`), no la consulta OR. El bonus se suma al coseno en `score.rs:259`.
- **Una palabra común sólo propone candidatos.** Los ids de `lexical_candidates` (ordenados por bm25, `search.rs:773-800`) que el denso no trajo entran como «sólo léxico» (`search.rs:1250-1251`) y se puntúan con el coseno como cualquier fila. Por eso mejorar qué candidatos propone sólo sirve si el coseno de esa fila pasa el piso.
- **Ya se probaron otros tokenizers y no movieron nada.** «Todas las corridas `lex*` y `tok-trigram-*` dieron métricas idénticas» ([lexical-fusion](../archive/2026-10-12-lexical-fusion/exploration.md), «No medible hoy»). El arnés para repetirlo existe: `rag-eval --fts-tokenizer` (`crates/clawg-eval/src/bin/rag-eval/cli.rs:55`), con una lista cerrada `ALLOWED_FTS_TOKENIZERS = [migración, "trigram"]` (`crates/clawg-core/src/eval_api.rs:272`) y un test que **rechaza** `porter` (`eval_api.rs:2166-2173`).
- **Entonces: si la fusión no le da peso al rango léxico, un mejor recall léxico se mide como cero.** El índice de docs sí le da peso: `fuse` suma `1 / (RRF_K + rango)` de los dos canales (`project_docs/mod.rs:44`, `:558-572`). Allí el efecto se vería hoy.

### La respuesta posible a la objeción de V8

El mecanismo es indexar cada texto en tres formas a la vez: sin raíces, con raíces del español y con raíces del inglés. La consulta se arma por forma y las tres se unen con OR; si el ranking es por bm25, cada forma es una columna del índice y puede pesarse aparte. La forma sin raíz es la que responde a «damaging its tokens»: nada se reemplaza, las raíces se suman, y un nombre exacto sigue buscándose por la forma exacta. El mecanismo no detecta el idioma: aplica los dos stemmers a todo el texto.

### Lo que ya hay para medir

- Una prueba del tipo que se necesita: el test que cambia el tokenizer de una tienda de eval (`eval_api.rs:2147-2160`, `:2176-2203`) arma el `CREATE` a partir de una plantilla idéntica a V8 (`FTS_CREATE_SQL_TEMPLATE`, `:285`). El que está en `:2166` no cambia ningún tokenizer: comprueba que `porter` se rechaza. Sirve para dos cosas: como plantilla para probar una forma nueva del índice, y como lo que hay que tocar para que el arnés acepte otra.
- `rag-eval --lexical-only` ordena con la misma expresión y el mismo orden que producción, sin leer un vector ni puntuar (`search.rs:1354-1358`; `crates/clawg-eval/src/bin/rag-eval/main.rs:671-676`). **No pasa por la fusión**, por lo que su resultado no depende de 136.
- `rag-eval --docs` mide el índice de docs con un set que **está en git** (`crates/clawg-eval/golden/project-docs.json`): Hit@1/3/5 y MRR@10, y `--diff` lo compara.
- Los sets: `docs/eval/golden.json` (46 casos, 48 filas, `query_lang` en todos: 39 `es`, 7 `en`) y `docs/eval/golden-expanded.json` (154 casos, 1.280 filas, 30 `lexical_related`; 78 casos sin `query_lang`). Ambos fuera de git.

## Qué se puede hacer, en SQLite y FTS5

El tokenizer es de la tabla: no hay un tokenizer por fila ni por idioma.

| Enfoque | A favor | En contra | Qué lo decide |
|---|---|---|---|
| **A. Una columna de raíces en `memory_fts`, calculada en Rust, además de `content`** | Suma, no reemplaza: la forma exacta queda como está (la respuesta a V8). Es la indexación en tres formas, con la forma sin raíz en `content`. El stemmer corre en Rust, donde ya está el resto de la lógica pura. Un cambio por consulta: cada canal elige qué columna lee | Una columna nueva en `memory` (por el contenido externo), una migración con backfill, triggers y `RESTORE_TEXT_INDEX` reescritos, cada MATCH fijado a una columna (tabla de arriba), una **dependencia nueva** (no hay ningún stemmer en `Cargo.lock`: el único `unicode-normalization` es de normalización, no de raíces) y un índice más grande | El censo y la corrida léxica de la puerta, y que el costo de la migración se pague con un recall medido |
| **B. Un tokenizer FTS5 propio, registrado desde Rust** | El más limpio de usar: una sola tabla, el tokenizer emite la forma exacta y la raíz en la misma posición (FTS5 lo permite con `FTS5_TOKEN_COLOCATED`) y ningún trigger cambia | `rusqlite` 0.42 no trae API de tokenizers de FTS5 (`src/vtab` sólo tiene `array`, `csvtab`, `series`, `vtablog`, y la feature `vtab` no está activada en `Cargo.toml:35`). `libsqlite3-sys` 0.40 sí compila FTS5 (`build.rs:177`) y trae `fts5_tokenizer` y `FTS5_TOKEN_COLOCATED` en sus bindings, así que se haría con FFI y `unsafe`, como el registro de sqlite-vec (`crates/clawg-core/src/memory/store/db.rs:105-126`). El tokenizer queda **escrito en el esquema**: una conexión que no lo registre (un binario anterior, una herramienta) no puede ni crear la tabla (`no such tokenizer`, comprobado), y los triggers de `memory` la tocan en cada alta: la vuelta atrás deja de ser un `DROP COLUMN`. El stemmer tiene que ser determinista para siempre, por el borrado sustractivo | Sólo si A resulta demasiado caro de mantener y la puerta ya abrió: es la opción de más riesgo para la base del usuario |
| **C. Sin tocar el índice: reducir la consulta a raíces y buscar por prefijo (`decisi*`)** | Cero migración, cero cambio de esquema; se vuelve atrás con el código. Es lo más barato de **medir**: `--lexical-only` lo ejercita sin tocar nada. FTS5 ya hace consultas por prefijo (el visor las usa: `viewer_match_expression`, `search.rs:667`) | La raíz de un stemmer **no es siempre prefijo** de las formas (irregulares, y el prefijo de una raíz corta se expande a miles de términos: el prefijo de dos letras es inútil), y la raíz de «decidió» no es prefijo de «decisión». Sigue necesitando un stemmer para sacar la raíz de la consulta, o una heurística de truncado, que mete falsos positivos (`memor*` junta `memoria`, `memory` y `memorándum`). Sin raíces en el índice, sólo cubre las formas que comparten prefijo | Si el censo muestra que las inflexiones perdidas son sobre todo de sufijo (inglés: `deploy*`; español regular: `decid*`) |
| **D. Dejarlo como está** | Los vectores ya cubren las inflexiones: en el golden de 46, `search_memory` acierta es>es 19/20 y en>en 6/7 (`openspec/config.yaml`, línea base de `rag-eval`). Cero código, cero riesgo para los datos | Si hay casos que fallan por palabras sólo por una inflexión, el recall léxico que podría rescatarlos se pierde; y en el índice de docs, que sí fusiona por rango, el costo se ve | El censo de la puerta |

**Recomendado:** **D hasta que el censo hable**; si abre, **medir con C** en la etapa léxica (no cuesta migración) y **construir A** si la medición de punta a punta lo pide, **empezando por el índice de docs**, que guarda su propio texto y no necesita columna en `memory` ni triggers. B queda como una nota: sólo si A resulta caro de mantener. Un detalle de A que hay que decidir en la propuesta: la columna de raíces lleva las dos raíces (la española y la inglesa) de cada token, porque `memory.lang` no sirve de entrada: nace `NULL` y lo llena el mantenimiento después (`V22__memory_lang.sql:1-9`).

### Los identificadores no se tocan

`project_id`, `save_memory`, `getUser4821`, `E_AUTH_4821` no se reducen. La regla, para que valga en cualquiera de los tres enfoques:

1. **Al indexar y al consultar, un token para el que `is_identifier` da verdadero** (letra y dígito, guion bajo o salto de camelCase, `search.rs:826`) **no se reduce**: se copia tal cual a las raíces, o se deja fuera de ellas.
2. **`identifier_matches` sigue consultando sólo la forma exacta** (`content`), y `holds_identifier` sigue comprobando la fila a mano (`search.rs:837`). Es el único canal con bonus de 1,0, y un identificador reducido le daría ese bonus a filas que sólo comparten raíz.
3. **Un MATCH sin columna es un error nuevo**: cada uno de los cuatro sitios de producción (tabla de arriba) se fija a una columna con un test que falla hoy porque aún no hay dos.

## Orden con memory-score-fusion (136)

La hipótesis es que la fusión va primero y esto después, midiendo contra la línea base que deje la fusión. **Se confirma**, con dos matices:

- **A favor de que la fusión vaya primero.** Hoy una palabra común no pesa en el ranking (arriba), así que un mejor canal léxico se mediría casi en cero, como los tokenizers anteriores. Medirlo ahora llevaría a cerrar una idea que quizá sólo espera a que el canal valga algo. Y si los dos cambios corren a la vez, no hay forma de atribuir un movimiento de `--diff`: la regla de [testing.md](../../../docs/reference/testing.md) pide 6 casos en un sentido y ninguno en el otro, y dos cambios sobre los mismos casos la rompen. La línea de `config.yaml` sobre la validación con `rag-eval --diff` contra una línea base vale para cada cambio por separado.
- **Matiz 1: no todo espera.** El censo (etapa 0) no usa ningún ranking, y `--lexical-only` (etapa 1) no pasa por la fusión (`main.rs:671-676`). Esas dos etapas son válidas hoy y dicen si el canal léxico mejora *en sí*; sólo la etapa 2, la de punta a punta, espera la línea base de 136.
- **Matiz 2: el índice de docs no espera.** Su fusión es otra (`project_docs/mod.rs:558`), y su golden está en git. 136 deja la fusión de docs como un paso aparte, que espera a un golden de docs con preguntas que nombren lo que dice el pasaje (su «Parte B»), así que `--docs --diff` puede medir las raíces en docs antes.
- **El riesgo de ir al revés.** Si 136 ajusta pesos sobre un canal sin raíces y las raíces entran después, la distribución de bm25 cambia: las raíces mueven N, df y `avgdl`. Su brazo por posición sólo mira el orden y debería aguantar; su forma convexa usa la magnitud anclada al índice, y aunque un ancla calculada a pedido se recalcula sola, el peso elegido hay que volver a leerlo en la etapa 2.

## Puerta (umbrales fijados antes de medir)

**Pregunta que contesta:** ¿cuántos casos del golden fallan por palabras **sólo por una inflexión**, y mover ese canal mueve el ranking de verdad?

**Quién la corre:** la persona, en local. El censo y `--lexical-only` no necesitan el modelo real; la etapa 2 sí (`--real`) y el golden ampliado, fuera de git. **No se corrió aquí.**

### Etapa 0: censo (gratis, un script fuera del producto)

Un script en un directorio ignorado, con un stemmer de Snowball (español e inglés) **fuera del producto**: así la dependencia no entra en `Cargo.lock` hasta que la puerta abra.

1. Para cada caso de `golden.json` y de `golden-expanded.json`, tomar los tokens de contenido de la consulta (los de `fts_tokens`, sin acentos, en minúscula, sin las palabras de parada del español y del inglés, con una lista fija escrita en el script antes de leer los datos) y los de las filas esperadas (`expected_refs`).
2. Clasificar cada caso: **(a)** algún token de la consulta aparece igual en una fila esperada; **(b)** ninguno aparece igual, pero uno comparte raíz (la del español o la del inglés) con una palabra de una fila esperada; **(c)** ni eso.
3. Cruzar con la línea base de `rag-eval --real` (el archivo `docs/eval/results/2026-10-16-real-eval-rigor-tip-anchor.json` que cita `config.yaml` trae los aciertos por caso): **M = casos de clase (b) que esa línea base no acierta en el top 5.** Son los únicos que el stemming puede mover. Informar también cuántos de (b) son `es` y cuántos `en`, y cuántos de los 30 `lexical_related` caen en (b).
4. Revisar a mano 20 casos de (b) tomados al azar **antes de ver el recuento**: ¿la raíz común es de verdad la misma palabra, o un falso amigo (`memoria`, `memory`)?

**Umbral:**

- Si **M < 6 en el golden ampliado** (y también en el de 46), nada puede dar una conclusión según las reglas de `--diff` (hacen falta al menos 6 casos en un sentido): **cierra, queda D**, y se archiva en «Ideas evaluadas» del README.
- Si **M ≥ 6 en el ampliado**, sigue la etapa 1. Con el de 46 sólo se informa.
- **Un tope por el otro lado:** si más de la mitad de la muestra a mano de (b) son falsos amigos, el censo sobreestima y el umbral sube a **M ≥ 12**.

### Etapa 1: el canal léxico solo (no depende de 136)

`rag-eval --real --lexical-only` sobre el ampliado, dos corridas: la línea base (hoy: R@1 0,3478, R@5 0,5000, R@10 0,5000, MRR 0,4040 en el set de 46, `docs/eval/results/2026-10-16-real-lexical-only.json`) y la variante, comparadas con `--diff`. La variante de esta etapa es **C** (la consulta con raíces y prefijo sobre el índice actual), que no pide migración, más una corrida de **A** sobre una tienda de eval si el arnés puede armar la columna (el arnés cambia el DDL con una plantilla; sería otra plantilla y otro rechazo en `ALLOWED_FTS_TOKENIZERS`).

**Umbral:** **recall@5 sube con al menos 6 casos entrando al top 5 y ninguno saliendo** (la regla de `--diff`: cada caso que va al revés sube el piso, y el veredicto dice cuántos más hacen falta). Con menos, «no distinguible», nunca «sin diferencia»: no se pasa a la etapa 2.

### Etapa 2: punta a punta, **después de 136**, contra la línea base que deje 136

`rag-eval --real` y `rag-eval --real --agent-view` sobre el ampliado y el de 46, variante contra la línea base de 136 con la **misma** semilla y el mismo golden (sha256 anotado). El índice de docs: `rag-eval --docs --real --diff`, que ya puede correr antes.

**Umbrales, todos juntos:**

1. **recall@5 sube** de modo distinguible (al menos 6 casos en un sentido, ninguno al revés), y entre las dos corridas de `--agent-view`, `search.found` sube igual (las dos métricas primarias de `docs/reference/testing.md`).
2. **`--fail-on-regression` sale con 0** (nada empeora con p < 0,05 en las métricas primarias).
3. **Los identificadores no se mueven:** los casos `lexical_related` de las corridas `--agent-view` no bajan de su `all_hit_rate` de la línea base, y ninguno que acertaba por identificador deja de acertar.
4. **La brecha entre idiomas no se toca:** las filas por par de idioma de `--diff` (`es>en` 6/19 en similitud) quedan informativas, pero un cambio ahí, para bien o para mal, se investiga antes de seguir: dentro de un idioma, las raíces no deberían cruzar al otro.
5. **Los tokens de `search_memory` no suben más de 5 %** (línea base 761,4; `config.yaml`). Más candidatos «sólo léxicos» no deben llenar la respuesta.
6. **Docs:** Hit@3 y MRR@10 de `--docs` no bajan.

Si 1 no se cumple, no se hace default; sin el cambio queda D. Si 1 se cumple y falla otro, se vuelve al diseño con ese dato.

## Migración y vuelta atrás (sólo si A abre)

Por `config.yaml`, un cambio de esquema lleva un test que siembra datos de la versión anterior y prueba que siguen alcanzables, más migración y vuelta atrás. Para A:

- **Migración.** Un V nuevo: `ADD COLUMN` nullable en `memory` para las raíces; recrear `memory_fts` con la columna y los tres triggers y `RESTORE_TEXT_INDEX` con la lista nueva; backfill en Rust. Hay precedente: V6 usa un UDF, `content_hash`, registrado antes de las migraciones (`V6__content_hash.sql:1-7`, `crates/clawg-core/src/memory/store/dedup.rs:57`); un UDF `stem_text` seguiría ese camino. Abrir una tienda con migraciones pendientes ya escribe un snapshot `pre-migration` (`docs/reference/data-profiles.md`).
- **Una fila sin raíces sigue alcanzable.** La columna nace `NULL` y `content` no cambia: toda fila anterior se sigue encontrando por su forma exacta. El test siembra una tienda en la versión anterior, migra y prueba que cada fila se encuentra por una palabra exacta **y** por una inflexión.
- **Vuelta atrás, escrita en el comentario de la migración** (como V22): recrear `memory_fts` con la forma de V8 y sus triggers, y `ALTER TABLE memory DROP COLUMN` de las raíces. El binario anterior no necesita nada: no conoce la columna y `content` sigue entero.
- **Un cambio de versión del stemmer** invalida lo indexado, porque el borrado resta con los tokens exactos. Eso obliga a decidir en la propuesta: la columna guarda el texto reducido (la raíz de entonces) y el borrado resta lo guardado, no lo recalculado. Es una razón más a favor de una columna real en `memory` frente a recalcular en el trigger.

## Coordinación

| Cambio | Cómo se toca |
|---|---|
| [memory-score-fusion](../memory-score-fusion/exploration.md) (136) | Va primero para la medición de punta a punta; ver arriba. Las etapas 0 y 1 no esperan. Ya mide el bono por la posición en bm25, sólo en consultas con identificador |
| [lexical-common-terms](../lexical-common-terms/exploration.md) (137) | Decide qué términos de la consulta generan candidatos por su frecuencia en el índice; este cambio decide en qué forma se indexan y se consultan. Los dos cambian qué propone el canal léxico, así que no se miden juntos, y quien entre segundo vuelve a medir su línea base. Comparten una pieza: la frecuencia de documento por término se lee del índice con raíces, si éste entra primero |
| [search-index-ports](../search-index-ports/exploration.md) (110) | Su puerto `LexicalIndex` es justo el lugar donde viviría «qué texto se indexa y cómo se consulta» (hoy repartido entre `search.rs`, los triggers de V8 y `fts_match_expression`). Este cambio sería el **primer candidato concreto** para esa mitad del puerto (la puerta de 110 cierra sin código si no hay uno): conviene escribir la reducción a raíces como una función pura y sin I/O, para que el adaptador de 110 la envuelva sin reescribirla. No depende de 110 |
| Índice de docs (V12, `project_docs/mod.rs`) | Es el banco de pruebas natural (guarda su propio texto, fusiona por rango, su golden está en git). Si la puerta abre, la propuesta decide si las raíces entran primero ahí o en los dos índices a la vez |
| [knn-prefilter](../knn-prefilter/exploration.md) (37), [storage-read-side](../storage-read-side/exploration.md), [storage-write-side](../storage-write-side/exploration.md) | Tocan `search.rs` y las escrituras a `memory`: un cambio de esquema aquí choca con ellos si corren a la vez; se ordena en la propuesta |
| [model-reembed](../model-reembed/exploration.md) | No se cruza: las raíces no dependen del modelo de embeddings |

## No alcance

- Cambiar `unicode61 remove_diacritics 2` en la columna `content` ni en `project_doc_fts`: el texto exacto no se toca.
- Elegir un idioma por fila o por consulta (la detección de idioma vive en otro lado, y `memory.lang` no es fiable al insertar).
- Búsqueda difusa, sinónimos, lematización con diccionario, errores de tipeo.
- Cruzar idiomas por palabras (la brecha es>en): `config.yaml` la atribuye al cambio de lengua, no al canal léxico, y aquí se vigila que no se mueva.
- Cambiar cómo se fusionan los canales (es 136) ni el bonus de los identificadores.
- Otro motor de búsqueda (es 110).

## Decisiones abiertas

- **D1. ¿Las raíces van primero en docs o en los dos índices a la vez?** Recomendado: primero docs, si la etapa 2 lo mide allí antes de 136.
- **D2. ¿Una columna con las dos raíces o dos columnas, una por idioma?** Dos columnas permiten pesar cada una en bm25 y ver cuál aporta; una sola es más simple. Lo decide la propuesta con el censo a la vista.
- **D3. ¿Qué crate de stemming?** No hay ninguno en `Cargo.lock`. Hay que revisar licencia, tamaño y mantenimiento antes de proponerlo; el censo no depende de la respuesta porque corre fuera del producto.
- **D4. ¿Se guarda el texto reducido en `memory` o se recalcula en el trigger con un UDF?** Recomendado: guardarlo (el borrado sustractivo necesita los mismos tokens).
- **D5. El umbral de falsos amigos del censo (más de la mitad de la muestra) es una conjetura**; se ajusta con la muestra a mano antes de leer los totales, no después.
- **D6. El tamaño del índice no tiene herramienta de medición hoy.** Si la puerta abre, la propuesta dice cómo se mide (una copia de un snapshot con `--background`, por ejemplo) y qué crecimiento es aceptable.

## Riesgos

- **Medir demasiado pronto.** Antes de 136, el canal léxico casi no pesa: un resultado cero no cierra la idea.
- **Falsos amigos entre idiomas.** Las dos raíces sobre todo el texto dejan que la raíz inglesa de una palabra choque con la española de otra. Aquí lo mide el punto 4 de la etapa 2 y la muestra a mano del censo.
- **Un MATCH que busca en una columna de más.** Es el fallo silencioso de A: ninguno de los cuatro sitios de producción da error, sólo cambia qué encuentran.
- **La base del usuario.** Una migración que toca el índice de todas las memorias es el cambio más delicado de este cambio: por eso la puerta va antes, el snapshot `pre-migration` y la vuelta atrás se escriben y se prueban, y B (tokenizer en el esquema) queda al final.

## Fuentes

- Código y migraciones de `develop` en `d658c8a9` (0.90.0), con las rutas y líneas citadas.
- [`2026-10-12-lexical-fusion/exploration.md`](../archive/2026-10-12-lexical-fusion/exploration.md) (las corridas `lex*` y `tok-trigram-*`) y [`docs/reference/testing.md`](../../../docs/reference/testing.md) (las reglas de `rag-eval --diff`).
