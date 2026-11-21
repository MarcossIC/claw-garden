# lexical-common-terms — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 3 | S para medir (la puerta 0, sin código de producto); S–M el paso de memorias si la puerta abre; M el de docs, que además necesita un golden que hoy no existe | — para medir. **Va antes que [memory-score-fusion](../memory-score-fusion/exploration.md) (136)**, que decide cuánto pesa el BM25 en el puntaje. Se coordina con [docs-query-language](../docs-query-language/exploration.md) (135, el idioma de la consulta en `search_project`), con [lexical-stemming](../lexical-stemming/exploration.md) (139), que toca el mismo canal, y con el [docs-search-quality](../archive/2026-10-31-docs-search-quality/verify.md) archivado, que ya probó una lista fija | exploración (borrador) |

**Tesis:** el canal léxico arma un OR con los primeros 16 tokens de la pregunta, palabras funcionales incluidas, y deja que `bm25()` elija los 20 mejores. Una lista fija de palabras vacías ya se probó en docs y se revirtió. La alternativa a medir es decidir qué es «común» con los datos de clawg y no con una lista: un término que aparece en más del X % de las filas del propio índice no genera candidatos; los raros sí. Una lectura exploratoria (sin las herramientas de evaluación) dice que, en memorias, el efecto **no es sacar ruido del pool** (el 99 % de los candidatos de hoy ya trae al menos un término raro, porque el idf de BM25 hunde a las filas que sólo comparten palabras comunes) sino dos cosas más chicas: cambia cuáles 20 entran entre las filas que sí tienen un término raro, y baja el costo, porque el OR de hoy recorre las listas de los términos más frecuentes. Además, sumar los términos comunes al puntaje de los candidatos **no tiene quién lo consuma** en memorias mientras el rango BM25 no entre al puntaje (hoy no entra), y una segunda consulta restringida a rowids **no es barata** en SQLite. Por eso el primer paso es sólo la mitad candidata, y la mitad del puntaje se le deja a 136.

**Qué se verificó y qué no.** El código y las migraciones se leyeron en `develop` (0.90.0). Se hizo **una lectura exploratoria** con un SQLite de línea de comandos distinto del empaquetado (rusqlite con `bundled`, `Cargo.toml:35`), sobre una tabla FTS5 de prueba armada con las filas de `docs/eval/golden-expanded.json` (sha256 `855c64b9…`, el de `openspec/config.yaml:35`) y sobre un corpus sintético de 100.000 filas. Es un script descartable que no entra al repo. **No se corrió `clawg`, el daemon, `rag-eval`, cargo ni ningún build, y no se leyó ningún perfil de datos.** Los números de esa lectura son direcciones, no mediciones: la puerta de abajo las repite con la regla fijada antes.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El canal léxico es un OR de hasta 16 tokens entre comillas, sin palabras vacías | **Cierta.** `MAX_LEXICAL_TOKENS = 16` (`crates/clawg-core/src/memory/retrieval/search.rs:623`); `fts_match_expression` toma los primeros `max_tokens` por posición y los une con ` OR ` (`:643-653`); los tokens son corridas de alfanuméricos o `_` (`fts_tokens`, `:656-661`). Una pregunta de más de 16 tokens pierde los últimos: 14 de las 46 consultas naturales del golden expandido |
| `bm25()` ordena y el `LIMIT` es 20 en memorias | **Cierta.** `LEXICAL_CANDIDATE_SQL` ordena por `bm25(memory_fts)` con `LIMIT ?4` (`:690-706`); el alcance por defecto pide 20 (`SearchScope::all_default`, `:45`), con un tope de 512 (`:726`). El estado y el alcance se aplican **antes** del `LIMIT` (comentario de `:681-689`) |
| El índice de docs trae 50 candidatos por canal | **Cierta.** `CANDIDATES_PER_CHANNEL = 50` (`crates/clawg-core/src/memory/project_docs/mod.rs:39`); la consulta léxica de docs usa la misma `fts_match_expression` (`:588`) y ordena por `rank` con `LIMIT` (`:603-607`) |
| El tokenizer es `unicode61 remove_diacritics 2`, sin stemming | **Cierta** para las dos tablas: `memory_fts` (`crates/clawg-core/migrations/V8__memory_fts.sql:36-43`, con `porter` rechazado en el comentario de arriba) y `project_doc_fts` (`V12__project_docs.sql:45-48`) |
| «Sin stopwords» en el código de recuperación | **Cierta para el canal léxico, con un matiz.** Ni `fts_match_expression` ni la búsqueda de docs filtran nada. Pero `language.rs` tiene listas de palabras funcionales en inglés y español (`crates/clawg-core/src/memory/retrieval/language.rs:86-102`) que sólo sirven para detectar el idioma de un texto. No son una opción para esto: son la lista fija que ya falló, más corta |
| Una palabra funcional «entra al OR» y daña | **A medias.** Ver «Qué le hace cada palabra funcional» abajo. En memorias casi no daña el pool; sí cuesta y sí desplaza. En docs el daño depende del golden |
| Los términos comunes «sólo suman al puntaje de los candidatos» | **Falsa para memorias hoy.** El puntaje sale del coseno, la importancia, el decaimiento y el bono (`score.rs:259`); `hit_cmp` ordena por ese puntaje y desempata por id (`search.rs:245-250`), y la lista final es un único `sort_by(hit_cmp)` (`:1288-1289`). Ningún rango BM25 viaja hasta ahí. **Cierta para docs:** `fuse` suma `1/(60 + rango)` de los dos canales (`project_docs/mod.rs:560-573`, `RRF_K = 60` en `:44`), así que el orden BM25 sí pesa |
| «Segunda consulta restringida a esos rowids» como forma barata de sumar los comunes | **Falsa.** En el corpus sintético de 100.000 filas, el `MATCH` con todos los términos más `rowid IN (…)` tardó 154 ms contra 90 ms del OR solo, y el plan muestra el `MATCH` recorriendo igual (`SCAN … VIRTUAL TABLE INDEX 0:=M1`, con el `IN` como filtro posterior). Una consulta por rowid (`rowid = ?`, 20 veces) tardó 184 ms. FTS5 no usa la lista de ids para saltarse las listas largas de los términos comunes |
| `fts5vocab` sirve sobre una tabla FTS5 de contenido externo | **Cierta.** `memory_fts` es de contenido externo (`content='memory'`, `V8:36-43`) y `fts5vocab(main, memory_fts, instance)` se crea y se lee en el repo (`crates/clawg-core/src/memory/upkeep/admin.rs:1381`, dentro de `mod tests`, que empieza en `:584`): el SQLite empaquetado trae el módulo. Con una tabla de contenido externo de prueba también devolvió las frecuencias. Lee el índice invertido, no la tabla `memory` |
| El df de un token de la consulta se lee tal cual de `fts5vocab` | **Con una condición.** `fts5vocab` da el df del **término** como lo dejó el tokenizer (minúsculas, sin diacríticos, cortado en todo lo que no es alfanumérico). `fts_tokens` conserva `_` dentro del token (`:656-661`), así que `user_id` llega al `MATCH` como la frase `"user id"` y no existe como término. Hay que plegar el token como lo pliega el tokenizer (ya hay `unicode-normalization` en `crates/clawg-core/Cargo.toml:43`) y tratar «no encontrado» como raro: la falla es conservadora, vuelve al comportamiento de hoy |
| Costo de `fts5vocab` por consulta | **Proporcional al df de cada término, no constante.** Con el corpus sintético, 8 términos (6 frecuentes y 2 raros) tardaron 8,3 ms de lectura de df; el OR actual, 90 ms; los 2 raros solos, 0,2 ms; df más candidatos raros, 9,5 ms en total. Contar el df recorre la lista del término, pero sin unir con `memory`, sin ordenar por `bm25` y sin el `LIMIT` |
| El bono de identificador no se toca | **Cierta.** `identifier_matches` arma su **propia** expresión, un AND de `identifier_tokens(query_text)` (`search.rs:875`, `:886`), sobre **toda** la consulta (`:813-824`, tope de 8); no llama a `fts_match_expression`. Sus filas entran como candidatos aparte (`:1243-1250`) y el bono es `lexical_match` en `score.rs:259` (`lexical_bonus = 1.0`, `:181`). **Un acoplamiento:** el identificador también está en el OR. Si su df superara el umbral saldría del OR y, como `identifier_matches` no premia a un nombre que más de 20 filas comparten (`:915-917`), no llegaría por ningún lado. Por eso los tokens que `is_identifier` acepta (`:826-831`) quedan fuera de la regla |
| El df es el de las filas que el agente puede leer | **Falsa.** `memory_fts` indexa las filas de todos los agentes y proyectos, `superseded` y `disputed` incluidas (los triggers sólo sacan filas con `deleted_at`, `V8:63-90`), y `project_doc_fts` junta todos los proyectos en una tabla con `project_id UNINDEXED` (`V12:45-48`; el filtro corre después del `MATCH`, `project_docs/mod.rs:605`). El df es global. Es una estadística, no expone contenido, pero hace que los candidatos de un agente dependan de las filas de otro (ver riesgos) |
| Un canal que no se ve en el golden no se puede validar con él | **Cierta y central.** Con el golden de 46 casos el canal léxico es invisible: con él prendido o apagado 0 de 112 casos cambian de ranking (`archive/2026-10-12-lexical-fusion/proposal.md:15-18`), y los barridos léxicos dan lo mismo que el ancla (`archive/2026-10-17-memory-rank-fusion/exploration.md:21`). Lo que sí lo ve: `--lexical-only` (`crates/clawg-eval/src/bin/rag-eval/cli.rs:81-84`, que usa `lexical_candidates`, `search.rs:1358-1360`) y los subconjuntos `lexical` y `lexical_related` del golden expandido |

## Qué le hace cada palabra funcional («que», «el», «de», «la», «how», «the»)

| Dónde | Efecto | Evidencia |
|---|---|---|
| El tope de 16 tokens | Ocupa un lugar. En una pregunta larga, las últimas palabras, que suelen nombrar la cosa, quedan afuera | `search.rs:643-653`; 14 de 46 consultas naturales pasan de 16 tokens; `archive/2026-10-31-docs-search-quality/exploration.md:44` (una pregunta del golden de docs tiene 19) |
| El costo del `MATCH` | **Daño.** Para ordenar por `bm25()` antes del `LIMIT`, SQLite recorre y puntúa **todas** las filas que calzan con alguno de los términos (`search.rs:703-704`). «de» y «the» están en el 33 % y el 41 % de las filas del golden expandido; el OR de la consulta calza en una mediana de 607 de 1.280 filas contra 175 con sólo los raros (máximo 848 contra 300) | Lectura exploratoria; mismo orden de magnitud que el corpus sintético (90 ms contra 9,5 ms) |
| El pool de memorias | **Casi inocuo, con un desplazamiento.** El idf de BM25 hunde a las filas que sólo comparten palabras comunes: de 920 candidatos de las 46 consultas naturales, sólo 13 (1 %) no traen ningún término raro. Pero entre las que sí lo traen, los comunes reordenan: el conjunto de 20 cambia en 43 de 46 consultas (media de 8,6 candidatos distintos). Un término en más de la mitad de las filas pesa 1e-6 en BM25; uno en el 41 %, poco pero no cero, y se acumula con diez palabras | Lectura exploratoria |
| El puntaje final de memorias | Un candidato léxico que el KNN no trajo se puntúa con el mismo coseno y compite por los mismos lugares (`search.rs:1250`, `:1288`). Si sólo calza por palabras comunes, su coseno suele estar bajo el piso y puntúa 0. Por eso el daño al ranking final es chico y **no lo ve el golden de 46** | `archive/2026-10-17-memory-rank-fusion/exploration.md:18` |
| El pool de docs | **Daño potencial real, y también la única señal.** El orden BM25 entra al RRF (`mod.rs:560-573`): un pasaje que sólo comparte «de» y «el» con la pregunta recibe puntos de rango. Pero en el golden de docs **cada pregunta se escribió sin las palabras del pasaje que la responde** (`verify.md:112`): lo único que comparte con su respuesta son palabras comunes | Ver la sección siguiente |
| El bono de identificador | Ninguno. Los identificadores casi siempre son raros y su ruta es otra (premisas) | `search.rs:813-831`, `:875-886` |

## Lo que ya se probó

- **Lista fija de palabras vacías y tokens cortos, sólo en docs (2a, `875d7b0`, revertido en `51c3a6b`).** Empeoró. `rag-eval --diff dq-base dq-2a` sobre 40 preguntas: Hit@1 0 mejor y 2 peor, Hit@3 1 mejor y 2 peor (neto −1), Hit@5 +2, MRR@10 3 y 3 con media −0,029. La puerta pedía que ninguna pregunta saliera del top 3 y que Hit@3 ganara 2; salieron d01 y c14 (`archive/2026-10-31-docs-search-quality/verify.md:65-86`).
- **Por qué empeoró, con lo que dicen sus resultados (`verify.md:110-120`).** El golden de docs se escribió, por construcción, sin las palabras del pasaje que responde cada pregunta; con la respuesta sólo comparte palabras vacías. «2a quitó lo único que la respuesta compartía con la pregunta», y lo que quedó llevó a otros pasajes: el primer hit de d01 pasó a una bitácora de otro cambio, el de d02 a un `mod tests` y el de c14 a otro `mod tests`, pasajes densos en palabras de contenido sueltas. **El golden de docs mide el canal denso, no el léxico** (`:120`): con 4 de 40 en el top 3, lo que falla ahí es el denso (`state.yaml` de ese cambio, `after_archive`). La lista no falló por ser una lista sino porque se midió con un golden que sólo le dejaba quitar señal. **La misma trampa vale para este cambio:** B también quita palabras de la generación de candidatos, así que sobre ese golden tampoco puede ganar.
- **Orden por rareza con un sustituto de la rareza (2b, `459f024`, revertido).** Identificadores primero, después los más largos, desempate por posición: Hit@3 0 mejor y 1 peor; sacó a d01 del top 3 por la misma razón (`verify.md:92-102`). Usó la longitud como sustituto del df. **No se probó con el df real**, que es lo que propone este cambio.
- **AND de los 3 más raros con respaldo OR (2c):** ninguna pregunta se movió (`verify.md:104-108`).
- **Un tokenizer distinto (`trigram`) y el canal léxico prendido o apagado.** Las corridas `--fts-tokenizer trigram` y los barridos de presupuesto léxico dieron las mismas métricas que el ancla en el golden de 46 (`cli.rs:55-61`; `docs/eval/results/2026-08-17-real-tok-trigram-lex20.json` y `…-lexoff.json`; `archive/2026-10-12-lexical-fusion/proposal.md:15-18`).
- **Los candidatos léxicos ya se puntúan por coseno, no por rango** (`archive/2026-10-12-lexical-fusion/exploration.md:23`, `proposal.md:11`); el bono por identificador reemplazó a una fusión por rango, descartada entonces porque el arnés no veía el canal (`archive/2026-10-17-memory-rank-fusion/exploration.md:57-60`).
- **Lo que sirve del arnés para este cambio:** `--lexical-only` (el piso léxico: R@1 0,3478, R@5 0,5000, R@10 0,5000, MRR 0,4040, `openspec/config.yaml:25`), `--diff` con la regla de 6 casos (`config.yaml:27`), `--fts-tokenizer` (ya sabe recrear `memory_fts` con otro tokenizer, `crates/clawg-core/src/eval_api.rs:476-489`) y `drop_superseded_text`, cuyo comentario ya registra que BM25 pesa un término por cuántas filas lo tienen y que las `superseded` siguen indexadas (`eval_api.rs:1019-1022`).

## Enfoques

Cada uno se mide solo, con `rag-eval --diff`, contra el anterior.

### A. No hacer nada
- A favor: el idf de BM25 ya hunde lo común; el golden de 46 no se mueve; sin riesgo.
- En contra: el costo del OR crece con el corpus y con el df de las palabras de la pregunta, y la ventana de 16 se gasta en palabras que no discriminan. Además condiciona a 136: si el BM25 entra al puntaje, el ruido de los términos comunes (idf chico pero no cero) pesa más.

### B. Los términos raros generan candidatos, sin segunda etapa (recomendado)
Un paso puro `split_by_df(tokens, df, filas, umbral) -> (raros, comunes)` y uno de E/S que lee el df de `fts5vocab` para los tokens de la consulta. El `MATCH` lleva sólo los raros. Si todos los términos son comunes, o el índice tiene pocas filas, o un df no se pudo leer, se usa la expresión de hoy. Los tokens que `is_identifier` acepta no se clasifican nunca como comunes.
- A favor: un cambio en `fts_match_expression` y su llamador; sin migración, sin re-embebido, sin tabla nueva (`fts5vocab` se crea como tabla virtual temporal, como ya hace `admin.rs:1381`); el índice de docs puede adoptarlo después con la misma función; es independiente del idioma, porque decide el corpus y no una lista; una palabra rara **en este corpus** nunca se descarta aunque una lista genérica la tendría por vacía; el costo baja en la dirección medida.
- En contra: es un cambio de ranking de candidatos; el df tiene su propio costo (≈ 8 ms por 8 términos en 100.000 filas sintéticas, sin caché); decide con una estadística global (ver riesgos).

### C. B más una segunda etapa que suma los comunes
Por una consulta restringida a rowids (descartada: 154 ms contra 90) o recalculando BM25 en Rust sobre el texto de los 20 candidatos.
- En contra: en memorias **no hay quién lo consuma** hasta que 136 deje entrar el BM25 al puntaje, y recalcular en Rust obliga a duplicar la tokenización y el idf de FTS5. En docs sí tendría efecto, pero el golden de hoy no lo puede medir. **Se difiere a 136**, que decide con qué BM25 (todos los términos o sólo los raros) normaliza.

### D. Los k más raros por df, sin umbral
Ordena los tokens de **toda** la pregunta por df y toma los k más raros (k = 8, por ejemplo). Es 2b con el df real.
- A favor: no necesita un X % ni un piso de filas; rescata un nombre clave en la posición 17 de una pregunta larga.
- En contra: leer el df de todos los tokens cuesta más (cada término frecuente recorre su lista); cambia **dos** cosas a la vez (cuáles entran y cuántos), y las reglas piden de a una. Se mide después de B, como un paso aparte.

### E. Lista fija de palabras funcionales
Descartada: ya se probó en docs y se revirtió, es de dos idiomas y se desfasa, y la de `language.rs` está hecha para detectar idioma, no para buscar.

### F. Tokenizer con stemming
Fuera de alcance: es [lexical-stemming](../lexical-stemming/exploration.md).

## Puerta de medición

**Puerta 0 (gratis, sin código de producto).** Se hace sobre los datos que ya están en `docs/eval/` (fuera de git; los sha256 están en `openspec/config.yaml`) con un SQLite de línea de comandos y una tabla FTS5 con el mismo tokenizer, sin tocar `clawg`. Para cada consulta: leer el df con `fts5vocab`, partir los tokens con X = 10 %, y comparar los 20 candidatos de hoy con los 20 de sólo los raros por el puesto de la fila esperada.

Se fija **antes de la corrida confirmatoria**, con X = 10 % y un piso de 200 filas (con menos, la fracción no significa nada):

| Condición | Umbral |
|---|---|
| La regla parte de verdad los tokens | entre el 15 % y el 60 % de los tokens de las consultas naturales caen sobre el umbral, y ninguna consulta queda toda común (si no, la regla no hace nada o vacía el OR) |
| Efecto en el recall léxico (el equivalente de `--lexical-only`) | la fila esperada entre los 5 primeros: **al menos 6 casos mejoran y a lo sumo 1 empeora**, sobre el conjunto confirmatorio (la regla de 6 de `config.yaml:27`) |
| Efecto en el costo | las filas que calza el OR bajan **al menos a la mitad** en la mediana |

Si pasan las tres, se escribe la propuesta. Si sólo pasa el costo, se escribe como un cambio de rendimiento con la regla de «ningún caso peor». Si no pasa ninguna, el cambio se cierra sin código.

**La lectura exploratoria ya existe y no cuenta.** Se miró antes de fijar el umbral, sobre las 46 consultas naturales de `golden-expanded.json` (1.280 filas: las del golden de 46 más distractores sintéticos y plantillados, así que sus df no son los de un store real):

| X | Tokens comunes | Consultas con el conjunto de 20 distinto | Fila esperada en el top 5, hoy → sólo raros | Mejoran / empeoran |
|---|---|---|---|---|
| 5 % | 42 % | 44 de 46 | 16 → 18 | 5 / 3 |
| 10 % | 32 % | 43 de 46 | 16 → 21 | 5 / 0 |
| 20 % | 23 % | 42 de 46 | 16 → 17 | 1 / 0 |

En `lexical` y `lexical_related` (30 casos cada una) la fila esperada ya estaba entre los 20 candidatos de hoy en 30 de 30, antes y después. A X = 10 %, 5 contra 0 da p = 0,0625 por la prueba exacta de signos: **por debajo** de los 6 casos que pide la regla. Dice que la dirección es plausible y que la puerta puede fallar. La confirmatoria usa otro conjunto: el golden de cruce de idioma (`docs/eval/golden-cross.json`, 48 casos en las dos direcciones, descrito en `openspec/specs/eval/spec.md:1328`) ampliado con `golden-mine synth --base docs/eval/golden-cross.json --seed 7`, que además mide si el idioma minoritario sale perjudicado (riesgos). X se elige entre 5, 10 y 20 % **sólo** con la lectura exploratoria; en la confirmatoria no se toca.

**Puerta 1 (el producto, sólo si la 0 abre).** Con el cambio de B detrás de un valor fijo, con el modelo real y una corrida por vez:

| Medida | Umbral |
|---|---|
| `rag-eval --real --diff` sobre `golden.json` (46) | **ningún caso se mueve** en R@1, R@5 ni R@10 (el canal léxico no se ve ahí; si se mueve alguno hay que explicarlo antes de seguir) |
| `rag-eval --real --diff` sobre `golden-expanded.json` (154) y sobre el expandido de cruce | peor ≤ mejor y peor ≤ 2 en cada métrica; `lexical_related` `all_hit_rate` no baja de 0,70 |
| Pares de idioma de `--diff` (`es>es`, `en>en`, `es>en`, `en>es`) | **ninguna fila de par pierde un caso**; `es>en` es la que más se mira (`config.yaml:21-24`) |
| `--lexical-only --diff` sobre el expandido | por la ruta de recall, mejoran al menos 6 y empeoran a lo sumo 1; por la de costo (la puerta 0 pasó sólo por costo), ninguno empeora, aunque ninguno mejore |
| `--agent-view` | `search_memory` no baja de 36/46 ni sube sus tokens medios (761,4, `config.yaml:28-30`) |
| Costo | el tiempo del brazo `lexical` (`record_arm`, `search.rs:1269`) sobre una copia de un store real (`--background`) no sube, y baja si la puerta 0 pasó por costo |

**Puerta de docs (aparte).** No se puede evaluar con el golden de hoy, por la razón de arriba. Antes hay que escribir un golden de docs con preguntas que nombren lo que el pasaje dice (un identificador, una ruta, un término propio), que es el pendiente que dejó el cambio archivado (`archive/2026-10-31-docs-search-quality/state.yaml`, `after_archive`), y una forma de leer el df del índice de docs: `--keep-store` no se combina con `--docs` (`cli.rs:150`, `:192-195`), así que hace falta un modo pequeño del arnés. Sin eso, extender B a docs repetiría 2a: una prueba que sólo sabe quitar señal.

## Decisiones abiertas

- **D1. Umbral X.** 10 % a priori; se elige entre 5, 10 y 20 % con la lectura exploratoria y se fija antes de la confirmatoria. Recomendado: 10 %.
- **D2. Un umbral por índice.** Una fila de memorias es corta (decenas de palabras) y un pasaje de docs es un trozo largo de código o prosa: el mismo X % significa otra cosa. Recomendado: **memorias primero, con su propio X**; el de docs se mide aparte con su golden, sin heredar el número.
- **D3. Piso de filas.** Con pocas filas la fracción no dice nada (un store nuevo, un proyecto nuevo). Recomendado: 200, y por debajo el comportamiento de hoy.
- **D4. Qué tokens entran al cálculo.** Los 16 de hoy (cambio mínimo, recomendado para B) o los de toda la pregunta (rescata el nombre clave en la posición 17, pero es el enfoque D y es otra variable).
- **D5. Cuántos raros pasan.** Todos los que haya dentro de los 16 (recomendado) o un tope menor.
- **D6. Caché del df.** Sin caché, el df cuesta lo que cuesta su lista; con caché hay que invalidarla cuando el índice cambia. Recomendado: **sin caché en B**, y sólo si la puerta de costo muestra que el df se come la ganancia, una caché por proceso con un contador de escrituras como llave.
- **D7. df global o por alcance.** El df sale de todo el índice (premisas). El del agente costaría una consulta por término con un join y no se justifica. Se acepta el global y la propuesta escribe qué implica: ninguna fuga de contenido, sólo la estadística.
- **D8. Cuándo se lee el df.** En cada búsqueda (recomendado, es lo que mide la lectura exploratoria) o una vez por conexión.
- **D9. Identificadores exentos.** Recomendado que sí (`is_identifier`): su ruta ya es otra y el df de una frase como `user_id` no se lee de `fts5vocab`.
- **D10. Qué hace 136 con la segunda etapa.** Si el BM25 normalizado se calcula sobre todos los términos o sólo sobre los raros. Es de 136, no de este cambio.

## Orden respecto de memory-score-fusion (136) y los otros

**Recomendado: 137 primero y solo, después 136.** Razones, todas de clawg:

1. **Hoy el rango BM25 no entra al puntaje** (premisas), así que 137 en memorias sólo cambia **qué 20 filas se proponen** y no toca la fórmula; 136 es quien lo haría pesar. Si 136 va primero, su normalización contra un ancla del corpus se calibra sobre una distribución de BM25 que 137 va a mover, y habría que recalibrar.
2. **El ruido de los comunes pesa más cuando BM25 pesa.** Hoy un candidato léxico sólo cambia el resultado si el KNN lo perdió y su coseno supera el piso (`search.rs:1250`, `score.rs:259`). Con una combinación convexa, un BM25 inflado por diez palabras de idf 0,4 sube un puntaje que antes no subía. Es un argumento para limpiar el insumo antes de pesarlo.
3. **La segunda etapa (sumar los comunes) sólo tiene sentido con 136.** Por eso 137 entrega la mitad candidata; si 136 abre, la mitad del puntaje se discute ahí.
4. **Se mide solo con lo que existe** (`--lexical-only`, `lexical`, `lexical_related`, `--diff`), y las reglas piden un cambio de ranking por vez.

El contra-argumento es real: 137 casi no mueve el ranking final (el golden de 46 no lo ve y el 99 % de los candidatos de hoy ya traen un término raro), y 136 podría absorber parte de la ganancia con el idf. Por eso 137 se mide primero **y solo**: lo que no se vea de punta a punta es lo que 136 tiene que explicar. Si la puerta 0 cierra sin código, 136 sigue sin cambios y su dependencia desaparece.

**Con [docs-query-language](../docs-query-language/exploration.md) (135):** la parte de docs de 137 espera a 135. Si 135 cambia el texto de la consulta o le dice al agente con qué vocabulario preguntar, cambia **qué tokens** ve el df; medir 137 sobre docs antes de eso mediría una consulta que va a cambiar. La parte de memorias no depende de 135. **Con [lexical-stemming](../lexical-stemming/exploration.md) (139) y el bono por posición, un brazo de 136:** los dos tocan el canal léxico de memorias; si una agrega una forma indexada o un bono por posición, el df y el idf que ven cambian. Un cambio de ranking por vez: 137 va antes porque no necesita migración ni dependencia.

## Cómo se mide

| Qué | Cómo | Quién | Puerta |
|---|---|---|---|
| Puerta 0: df, partición y candidatos | SQLite de línea de comandos sobre los golden de `docs/eval/`, sin `clawg` | La persona, en local; sin costo | Las tres condiciones de la tabla de arriba, con X = 10 % |
| Puerta 1: ranking completo y pares de idioma | `rag-eval --real --diff` y `--lexical-only --diff`, un cambio por vez | La persona, con el modelo real | Tabla de la puerta 1 |
| Costo | `record_arm` del brazo `lexical` sobre un snapshot real (`--background`) y la lectura de df por separado | La persona | No sube; baja si la puerta 0 pasó por costo |
| Docs | Golden nuevo con preguntas que nombran su respuesta, y un modo del arnés que lea el df | Escritura del golden: la persona; modo del arnés: código de evaluación | Se fija al escribir ese golden, con la regla de 6 casos |

Pruebas rojas primero, si la puerta abre: la función pura con su tabla de casos (todo común vuelve a la expresión de hoy; un identificador no se parte; un token sin df queda como raro; un df no legible vuelve a la expresión de hoy; con menos de 200 filas la expresión es la de hoy; un acento plegado calza con su término), y un test con una base creada con el esquema anterior que compruebe que `fts5vocab` lee los mismos términos que el `MATCH` (`memory_fts` no cambia de esquema).

## Riesgos

- **Repetir 2a.** Es el riesgo principal. Si la medición se hace sobre un golden cuyas respuestas sólo comparten palabras comunes con la pregunta, B también va a quitar la única señal. Por eso docs no entra hasta tener un golden que nombre su respuesta, y la puerta de memorias usa `--lexical-only` y `lexical_related`, donde el canal se ve.
- **El idioma minoritario.** El df es del corpus, no de una lista: en un store casi todo en inglés, «que» y «para» están en pocas filas, no pasan el umbral y **siguen generando candidatos**, que son justo las filas en español. Puede empeorar el cruce `es>en` (6 de 19 con similitud sola, `config.yaml:21-24`) o favorecer un idioma. Se mira en los pares de `--diff` y en el expandido de cruce. Mitigación posible: df por idioma (`memory.lang`, `V22__memory_lang.sql`; la columna no está en `memory_fts`), a un costo que hoy no se justifica.
- **Fracción con pocas filas.** Un store o un proyecto chico hace que cualquier palabra sea «común». Piso de 200 filas (D3).
- **Plegado distinto al del tokenizer.** Un token que se pliega mal da df 0 y se trata como raro: vuelve al comportamiento de hoy y no se pierde nada. Se prueba con acentos, mayúsculas y `ñ`.
- **Estadística global entre agentes.** El df cuenta filas de agentes y proyectos que quien busca no puede leer (premisas); no expone texto, pero los candidatos de un agente dependen de las filas de otro. Se documenta, no se esconde.
- **El df se come la ganancia.** Cada término frecuente recorre su lista para contarse. La puerta de costo mide lectura de df más candidatos contra el OR de hoy, no el `MATCH` solo.
- **Filas `superseded` en el df.** Siguen indexadas e inflan el df de los temas con muchas revisiones (`eval_api.rs:1019-1022`). Efecto chico.
- **Que el golden no lo vea.** Es probable: el resultado de punta a punta en el golden de 46 será «nada se mueve». Es lo que se le pide a esa fila; la ganancia se lee en `--lexical-only`, `lexical_related` y el costo.
- **Cambios de ranking cerca.** 136 (que ya mide el bono por posición), 37 ([knn-prefilter](../knn-prefilter/exploration.md)) y `lexical-stemming` (139) tocan `search.rs` y la misma ruta. Quien entra segundo vuelve a medir su línea base.

## Esfuerzo e impacto

| Paso | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|
| Puerta 0 | Decide si hay cambio | Ninguno | Horas, sin código de producto |
| B en memorias (si abre) | Menos filas recorridas por consulta; candidatos más centrados en lo raro; despeja el camino de 136 | Bajo a medio (ranking de candidatos; sin migración) | S–M: una función pura, la lectura de df y sus pruebas |
| Golden de docs que nombre su respuesta, y modo para leer el df | Hace medible el canal léxico de docs por primera vez | Bajo (sólo evaluación) | M, con trabajo de la persona para escribir las preguntas |
| B en docs | Incierto hasta tener el golden | Medio (el canal entra al RRF) | S una vez que el golden exista |
| Segunda etapa de puntaje | Sólo con 136 | — | — |

## Fuentes

- Código y migraciones de `develop` (0.90.0), con las rutas y líneas citadas. Contratos del arnés y bases: `openspec/config.yaml`.
- [`2026-10-31-docs-search-quality`](../archive/2026-10-31-docs-search-quality/verify.md) (exploración, propuesta, `verify.md:65-120` y `state.yaml`): 2a, 2b, 2c y el motivo de que el golden de docs no mida el canal léxico.
- [`2026-10-12-lexical-fusion`](../archive/2026-10-12-lexical-fusion/proposal.md) y [`2026-10-17-memory-rank-fusion`](../archive/2026-10-17-memory-rank-fusion/exploration.md): BM25 sólo propone; el canal léxico es invisible en el golden de 46; el bono por identificador.
- [`memory-score-fusion`](../memory-score-fusion/exploration.md), [`docs-query-language`](../docs-query-language/exploration.md), [`lexical-stemming`](../lexical-stemming/exploration.md): los cambios que tocan el mismo canal. Se leyó su tesis y su coordinación, no más.
- Lectura exploratoria (SQLite de línea de comandos, tabla FTS5 de prueba con `unicode61 remove_diacritics 2`, `fts5vocab` y un corpus sintético de 100.000 filas con palabras de frecuencia tipo Zipf): descartable, no entra al repo y no sustituye a la puerta 0. Sus límites: el corpus del golden expandido tiene distractores plantillados, el SQLite no es el empaquetado, y la tokenización es aproximada (`\w+` y plegado NFD).
