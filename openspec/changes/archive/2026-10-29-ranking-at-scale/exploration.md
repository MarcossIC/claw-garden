# ranking-at-scale — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M (tres pasos con puerta: herramienta de medición M, peso por tipo S, puerta absoluta S–M) | el store importado con el historial (history-import, archivado); piece-boundaries (cambia las filas antes de medir); se coordina con knn-prefilter (37), docs-search-quality (20) y embed-throughput (85) | exploración (borrador) |

**Tesis:** el ranking se decidió midiendo en stores de cientos o miles de filas y los problemas aparecen con 52.000. Dos se vieron en una importación real: las piezas de `session_summary` son el 11 % de las filas y ocuparon 19 de 36 lugares del top 3 (53 %) en 12 búsquedas, y cinco preguntas sin respuesta devolvieron cinco hits con score de 0,2 a 0,33. Ninguno es un fallo del piso de coseno ni de `collapse_saves`: el piso deja pasar por construcción a una fila con coseno de 0,83 a 0,87, y `collapse_saves` hace lo suyo (un hit por save), pero las que inundan son muchas saves distintas. El código no pesa el tipo en ningún punto del score (sólo cambia la tasa de decaimiento) y no tiene una puerta de «nada relevante» que mire la cercanía real. Antes de tocar el score hay que poder **medir a escala**, y hoy `rag-eval` no puede: su corpus es de un agente, sin proyecto y sin fondo, y el golden no tiene ni un caso sin respuesta. El cambio se hace en ese orden, un cambio de ranking a la vez y con una puerta de «seguir o cerrar» entre pasos: (1) fondo y casos reales en `rag-eval`, (2) un peso de lectura para `session_summary` salvo que la consulta lo pida, (3) una puerta absoluta calibrada con consultas con y sin respuesta. La importación no deja nunca un resumen afuera; el peso es de lectura.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| La fórmula del score está en `search.rs` hacia la línea 1855 | **Falsa en el lugar, cierta en el contenido.** La fórmula es `score()` (`crates/clawg-core/src/memory/score.rs:264-308`): `relevancia × max(piso de tier, decaimiento) × importancia × estado`. `search.rs:1821` es un test (`score_formula_integration_spot_check`). El orden final lo da `hit_cmp` (`search.rs:224-230`). |
| El score no tiene ningún término por tipo | **Cierta, con un matiz.** El tipo sólo elige la tasa de decaimiento (`lambda_for`, `score.rs:228-242`): `session_summary` 0,0077 (`:132`), `semantic_fact` 0,0039 (`:133`). Un resumen además nace en tier 2 (`crates/clawg-core/src/memory/tier.rs:49`, test `:232`), piso 0,2 (`score.rs:128`), y un hecho importado en tier 3, piso 0,05 (`START_TIER`, `portability/translate.rs:224`, `:608`). A 209 días un resumen llega a su piso; un hecho, a 768. Es una hipótesis sobre la edad del historial, no un hallazgo: no se midió la edad de las filas inundadoras. |
| Los scores de 0,2 a 0,33 de una consulta sin respuesta están por encima del piso de coseno 0,79 | **Cierta, y no es un error del piso.** Como los otros factores valen a lo sumo 1, un score `s` exige un coseno reescalado ≥ `s` (`rescale_cosine`, `score.rs:219-221`), o sea coseno ≥ 0,79 + 0,21·`s`: **≥ 0,832 para 0,2 y ≥ 0,859 para 0,33**, y más alto si la fila es vieja o de poca importancia. El piso es el percentil 5 de los cosenos consulta–pasaje de la documentación del repo (`score.rs:166-168`): deja pasar al 95 % de los pares con respuesta, no mide si hay respuesta. La distribución de los cosenos de pares **sin** relación no está en el repo (lo dice también la exploración archivada de search-abstention), y con 52.000 filas el mejor de los vecinos de una consulta cualquiera crece con el tamaño del store. |
| `min_relative_score` y `RELEVANCE_RATIO` 0,35 no pueden cortar esos hits | **Cierta.** El corte es relativo al mejor hit no exacto (`shown_hits`, `crates/clawg-cli/src/relevance.rs:36-51`): si los cinco hits están entre 0,2 y 0,33 el corte (0,35 × 0,33 = 0,115, o 0,5 × 0,33 = 0,165 en la sonda) no quita ninguno, y `MIN_SHOWN_HITS` = 2 (`:23`) muestra los dos primeros con cualquier score mayor que 0. La sonda de la importación usó 0,5; el puente manda 0,35 (`client.rs:626-630`). Una fila con score 0 ya no se muestra ni rellena (`relevance.rs:44-46`, `prompt_builder.rs:237`): es lo que dejó search-abstention. |
| `collapse_saves` hace su trabajo y la inundación son saves distintas | **Cierta.** Deja el mejor hit de cada save (`search.rs:167-184`), y el pool también cuenta saves, no filas (`trim_to_saves`, `:202-210`; `knn_one`, `:642`). Que 19 de 36 lugares sean resúmenes son 19 saves de sesiones distintas. |
| El cosine floor se barrió de 0,70 a 0,83 «sin ganancia para es>en» | **Cierta sólo en lo significativo.** En english-recall (`openspec/changes/archive/2026-10-17-english-recall/verify.md:128-138`) `es>en` R@5 fue 7/19 en 0,70 y 0,75, 6/19 en 0,79, 9/19 en 0,81 (3 a 0, p = 0,25) y 9/19 en 0,83 (5 a 2, p = 0,45), y `en>en` cayó de 6/7 a 2/7 sólo en 0,83. Ningún punto fue distinguible. Además movía el reescalado de **todo** el ranking dentro de `score()`; una puerta de lectura no lo toca, así que no hereda ese resultado, pero sí su advertencia: la brecha `es>en` hace que una puerta por coseno corte primero lo cruzado de idioma. |
| Una base de 500 distractores bajó el R@5 natural de 0,78 a 0,30 | **Cierta en la cifra, vieja en las condiciones.** 0,7826 → 0,3043 (`openspec/changes/archive/2026-10-12-eval-golden-expansion/design.md:134-136`), medido el 12 de octubre con los distractores sintéticos solos, antes del pooling por media y del ancla actual (R@5 0,6522 en 46 casos, `openspec/config.yaml`). Hay que volver a medir, no citarla como línea base. |
| El set de la semilla 7 tiene 1.040 filas | **Cierta para los resultados de long-save-ranking** (`corpus_size` 1040 en `docs/eval/results/2026-10-15-real-long-save-ranking-tip-seed7-agent-view.json`); el golden expandido actual tiene 1.280 filas y 154 casos (`openspec/config.yaml`). Los archivos de `docs/eval/` no están versionados (`.gitignore:32`), así que un clon no los tiene. |
| `rag-eval` soporta casos sin respuesta | **Cierta en el código, falsa en los datos.** `CaseSubset::Unanswerable` existe (`crates/clawg-eval/src/bin/rag-eval/golden.rs:112`) y agent-view cuenta `abstained` (`agent_view/runner.rs:203`), pero ningún `docs/eval/*.json` contiene la palabra `unanswerable` y el verify de search-abstention lo dice («el golden set no tiene casos `unanswerable`»). La abstención no tiene hoy ni un caso con el que medirse. |
| `rag-eval` puede cargar un fondo de decenas de miles de filas | **Falsa.** La ruta principal siembra una fila por vez bajo un solo agente (`rag-eval/seed.rs:19`, `:71-82`), sin proyecto ni piezas, y busca con `Reach::Own` y `project_id: None` (`crates/clawg-core/src/eval_api.rs:105-111`), así que **no vería** filas de otro agente como las del agente `migrated` de una importación. Agent-view pasa por el worker real (`runner.rs:91-94`: piezas y chunker de producción) y por el daemon (que sí lee `sharing.json`), pero no aplica `age_days` (`EvalIngest::send` no lo recibe, `eval_api.rs:589-601`) y busca sin proyecto (`runner.rs:140`, `:153`). Ninguna bandera carga un fondo (las de `cli.rs:288-375`). |
| `search_memory` permite pedir resúmenes | **Falsa.** No hay filtro de tipo: `SearchMemoryArgs` (`crates/clawg-cli/src/mcp/server.rs:189-204`) y `SearchRequest` (`crates/clawg-cli/src/daemon/api.rs:400-415`) no lo tienen; `list_memories` sí lo tiene. «Salvo que la consulta lo pida» no tiene hoy ningún mecanismo. |
| El puente puede leer el coseno de un hit | **Falsa.** `RankedHit.distance` viaja en la respuesta (`search.rs:130`), pero `Hit` del cliente (`client.rs:189`) no lo declara y `shown_hits` recibe sólo `(score, exact)`. Una puerta por coseno necesita el campo o vivir en el daemon (`api.rs:1462-1465`); la de `recall_context` tiene que vivir en el núcleo (`prompt_builder.rs:195-241`). |

## Problema (verificado contra el código)

### 1. No hay forma de medir a escala

- El baseline son 46 casos con un corpus chico (`openspec/config.yaml`); con ellos un caso son 2,2 puntos y una conclusión necesita al menos 6 casos moviéndose en una dirección (`crates/clawg-eval/src/bin/rag-eval/stats.rs:53`, texto del veredicto en `diff.rs:55-68`). Un cambio que afecte a la inundación del top 3 mueve pocos casos de 46.
- El fondo no entra en la medición: ver las dos últimas filas de la tabla de premisas sobre agentes, proyecto, piezas y edad. Con `golden-mine synth` los distractores son sintéticos (`golden-mine/main.rs:162`, 500 por defecto): mezclan términos de dos filas del mismo tipo e idioma, no se parecen a un historial de sesiones, y no hay ninguno con la forma de un resumen largo con piezas.
- El corpus de las 12 búsquedas de validación (7 con respuesta, 5 sin) es una sola corrida a mano, no un set: 3 de 7 acertaron en el top 3, 3 a medias y 1 falló. Con ese tamaño ni el p exacto ni la partición calibración y prueba (`report.rs:198-236`) dicen nada.

### 2. El peso de un tipo no existe en el score

La inundación no tiene un lugar donde corregirla: `score()` multiplica relevancia, decaimiento, importancia y estado (`score.rs:307`). Los resúmenes son largos y cubren varios temas (Goal, Discoveries, Accomplished, Relevant files; `protocol.md:117-125`); cada pieza posterior a la primera se embebe detrás de las 48 primeras palabras del save (`SAVE_HEAD_TOKENS`, `chunker/mod.rs:23-60`), así que todas las piezas de un resumen comparten ese arranque. Es una hipótesis razonable de por qué muchos resúmenes quedan cerca de muchas consultas, **no medida**: la medición tiene que separarla del efecto del tipo (mismo tipo, otro corte de piezas, ver Interacciones con piece-boundaries). El protocolo ya trata al resumen como algo que se desvanece y no sostiene conflictos (`protocol.md:100-101`, `:130-131`), y el contexto ya abre con el último resumen del proyecto sea cual sea la consulta (`LAST_SESSION_SQL`, `prompt_builder.rs:860`): bajar su peso como hit no le quita continuidad al usuario.

### 3. Con 52.000 filas el piso no abstiene

Con la fórmula de la tabla de premisas, 0,2 a 0,33 equivale a coseno de 0,83 a 0,87 o más. Una puerta de abstención hoy es imposible de construir con lo que hay: no hay consultas sin respuesta en el golden, el cliente no ve el coseno, y la exploración de search-abstention descartó el criterio relativo por consulta sólo con scores redondeados de 46 consultas con respuesta (17 de 46 con hueco menor a 0,02) y dejó dicho que «se reabre con scores crudos». Esos scores crudos ya se pueden obtener (el daemon devuelve `distance`).

### 4. El alcance de la búsqueda cambia lo que se mide

Una fila sin proyecto la lee cualquier proyecto (`memory_project_predicate!`, `search.rs:107-114`), y una importación deja sin proyecto las notas que `--project-map` no nombra (`.claude/CLAUDE.md`, `clawg import`). Esas 52.000 filas compiten entonces en **todas** las búsquedas, y una búsqueda con proyecto pasa por la ventana de 30 de vec0 antes de filtrar (`SearchScope::all_default`, `search.rs:36-46`; `knn_one`, `:514-653`), de modo que parte de lo que se vea como «menos resultados» es crowding (knn-prefilter) y no ranking. La medición debe registrar `passes` y la línea de supervivencia del pool (`format_pool_line`, `search.rs:664`; `RUST_LOG=info,clawg_core::memory::search=debug`).

## Lo que ya existe y se reutiliza

- `ScoringConfig` con una constante `DEFAULT` y un `EvalScoringConfig` espejo para los barridos (`score.rs:106-192`, `eval_api.rs:124-144`): el patrón para un peso barrible con `--sweep-*` y comparable con `--diff`.
- `rag-eval --diff` con McNemar exacto, intervalo bootstrap, corte por pareja de idioma y `--fail-on-regression` (`diff.rs:46-68`), y la partición calibración y prueba por hash de la consulta (`report.rs:198-236`): se elige en calibración y se informa en prueba.
- El subconjunto `unanswerable` con su validación, su bloque y la tasa con intervalo de Wilson (`golden.rs:112`; spec `eval`): falta sólo tener casos.
- `EvalIngest` (`eval_api.rs:546-641`): el pipeline real de ingesta sobre un store aislado, con proyecto, agente y `topic_key` por save (`EvalSave`, `:477-484`), usado por `revision-eval`.
- `clawg export` en JSONL y `clawg backup` (un snapshot con vectores, ver Enfoques): dos maneras de sacar un fondo del perfil dev sin tocar el perfil.
- `golden-mine` ya minaba candidatos desde una extracción del historial (`golden-mine/main.rs:157-170`) y `synth` ya genera distractores deterministas por semilla.
- `RankedHit` con `distance`, `exact`, `memory_type` y `project_id` (`search.rs:122-149`): la entrada de cualquier peso o puerta, sin tocar la consulta SQL.
- El texto «no relevant memories for this query: N searched, none close enough to show» y el conteo `withheld` (search-abstention, archivado): el destino de una abstención real, sin frase nueva.

## Enfoques posibles

### Paso 1. Medir a escala (condición de los otros dos)

**A1. Un fondo copiado de un snapshot.** `rag-eval --background DIR` abre una copia, en el directorio temporal, de un store ya poblado (un `clawg backup` del perfil dev), le suma encima el corpus del golden con `EvalIngest` y busca con el alcance de producción (todos los agentes legibles, proyecto del caso). No se vuelve a embeber nada del fondo (el snapshot trae los vectores). Condición: el store tiene que coincidir con el contrato de embeddings, o se rehúsa sin tocarlo; el texto personal queda en el directorio temporal y en `docs/eval/` (ignorado), nunca versionado.

**A2. Un fondo desde un export.** `--background FILE.jsonl` lee las líneas `memory` de un `clawg export`, las guarda con `EvalIngest` y las vuelve a embeber. Es portable (no hace falta un snapshot ni el mismo perfil) pero el export no trae vectores, así que cada corrida paga la re-embebida del fondo (en la importación real fueron unos 40 filas/s con una compilación de depuración: ~22 minutos para 52.000, en el orden de lo que baja embed-throughput).

**A3. Un fondo sintético con la forma del historial.** Extender `synth` con resúmenes largos y proporciones por tipo. Reproducible por semilla y sin texto personal, pero es el fondo que se parece menos a lo medido, y la inundación (una propiedad de cómo el modelo ve esos textos) puede no aparecer en texto de plantilla.

**Casos.** Se minan del historial: las 12 búsquedas de validación como semilla, más las que haga falta hasta tener el poder deseado, con la fila esperada identificada por su origen (`created_by_ref`, indexado para las filas de `source = 'import'`, `V23__origin_project.sql:18`) en lugar de insertarla, porque en A1 ya está en el fondo. Hace falta un formato de caso que cite una fila existente además de `corpus_refs` (`golden.rs:40-56` exige que cada ref esté en el corpus del golden). Entre los casos, al menos 20 sin respuesta (con 0 errores de 20 la cota inferior de Wilson es 0,839; con 16 de 20 es 0,584) y consultas en las dos lenguas y en las parejas `es>es`, `en>en`, `es>en`. También se mide el costo de escala **del mismo golden de 46 casos**, con y sin fondo, con `--diff`: es la primera cifra del cambio.

**Lo que se informa por caso**, sin cambiar el ranking: tipo de cada hit del top 5, coseno crudo (1 − d²/2) del mejor hit no exacto, hueco entre el primero y el décimo, parejas de idioma, `passes` del pool, y la edad de la fila.

### Paso 2. Un peso de lectura para `session_summary`

**B1. Factor multiplicativo en `score()`** (un `type_weights` en `ScoringConfig`, constante `DEFAULT` y espejo para el barrido). Un solo lugar; el corte relativo (`shown_hits`) y `recall_context` lo ven sin cambios porque operan sobre el score ya pesado; barrible con `--diff`. Toca también a quien ordena dentro de `knn_one` antes de recortar al pool (`search.rs:641-642`), incluido el detector de contradicciones, que ya excluye los resúmenes por tipo (el protocolo: «nunca entra en un conflicto»); hay que comprobar que su pool de 5 no cambia.
**B2. Tope por tipo en la lista mostrada** (por ejemplo a lo sumo N resúmenes entre los 5 primeros, sin tocar scores). No mueve el score ni el corte, pero decide entre hits de scores dispares con una regla que el score no explica, y hay que repetirla en `recall_context`.
**B3. Sólo parámetros existentes** (tasa de decaimiento y piso de tier de `session_summary`): sin código nuevo y barrible hoy, pero actúa por edad y no por tipo, y el piso de 0,2 (`score.rs:128`) lo sostiene aunque la tasa suba. Sirve de brazo de control, no de solución.

**La excepción «salvo que la consulta lo pida».** No tiene mecanismo hoy (ver premisas). Tres caminos: (i) un argumento explícito de tipos en `search_memory` y `POST /v1/search`, determinista y sin depender del idioma, pero cambia el contrato que el modelo ve (`agent_contract.json` y su versión en español se vuelven a bendecir, `protocol.md`/`protocol.es.md`); (ii) detectarlo en el texto de la consulta con una lista de palabras en inglés y español, sin cambio de contrato pero frágil (`resumen`, `la última vez`, `summary`) y con falsos positivos; (iii) ninguna excepción: el último resumen ya abre cada contexto (LAST SESSION), y los anteriores se piden con `list_memories` y su filtro de tipo. La medición del paso 1 decide si existen consultas del historial que pidan resúmenes viejos.

### Paso 3. Una puerta absoluta para las consultas sin respuesta

**C1. Umbral sobre el coseno crudo del mejor hit no exacto**, aplicado donde hoy se corta (el daemon, `api.rs:1462-1465`, y el núcleo para el contexto). No mezcla decaimiento ni importancia (la objeción a la variante A1 de search-abstention) y se calibra en consultas con y sin respuesta. Queda atada al embedding y se invalida con el prefijo, el pooling o la cuantización (`score.rs:179-186`), igual que el piso.
**C2. Un criterio relativo a la propia consulta con scores crudos** (hueco entre el primero y el décimo, o el primero contra la cola del pool). A escala, un pool de 30 saves planos es la firma de «nada responde», y no depende de la escala absoluta. Se reabre porque ahora hay scores crudos; se descartó con 46 consultas redondeadas.
**C3. Combinar C1 y C2** sólo si ninguna basta sola en la partición de calibración.

La puerta decide qué se **muestra**: el ranking y los scores del núcleo siguen siendo la lista completa, como en `relevance.rs:1-5`, y lo no mostrado no cuenta como acceso (el orden del daemon, `api.rs:1462-1466`).

### Recomendación

1. **Paso 1 primero y entero**, con A1 (fondo copiado de un snapshot, que da la distribución real sin pagar 22 minutos por corrida) y A2 como la forma portable de construir el snapshot. A3 queda fuera salvo que A1 resulte inusable. Tomar la línea base a escala **después** de piece-boundaries, y repetirla sobre el store anterior para atribuir la inundación al corte de piezas o al tipo.
2. **Puerta 1:** si a escala el top 3 no está inundado por resúmenes en los casos que no los piden, o el peso no los mueve más de lo que mueve ruido, cerrar el paso 2 sin código.
3. **Paso 2 con B1**, B3 como control y la excepción (iii) por defecto, (i) sólo si el historial muestra consultas que piden resúmenes viejos. Un peso barrido, no elegido a ojo.
4. **Puerta 2:** el paso 3 sólo avanza si C1 o C2 separan consultas con y sin respuesta con la cota de Wilson pre-comprometida; si no, se cierra dejando los casos y las distribuciones escritas.
5. **Paso 3 con C1**, o C2 si el coseno crudo no separa; cada paso en sus propios commits, y partir el paso 3 en su propio cambio si el 1 y el 2 tardan.

## Descartado

| Opción | Razón técnica |
|---|---|
| Dejar los resúmenes fuera del índice o de la importación | Pierde datos del usuario; el cambio es de lectura (`openspec/config.yaml`: nada de lo guardado se pierde ni se oculta). |
| Cambiar el tier inicial de `session_summary` | Es un dato guardado (`tier` en `memory`): pide migración y relleno, y no distingue entre un resumen que se pide y uno que no. El peso de lectura no toca filas. |
| Subir el piso de coseno 0,79 como puerta | Es el percentil 5 de los cosenos con respuesta (`score.rs:166-168`) y reescala todo el ranking; english-recall lo barrió de 0,70 a 0,83 sin un punto distinguible y con costo en `en>en`. |
| Puerta sobre el score final | Mezcla decaimiento, importancia y estado (`score.rs:279-307`): una memoria relevante, vieja y poco importante quedaría bajo una irrelevante reciente. |
| Reranker o clasificador aparte | Un modelo y una dependencia nuevos sin una necesidad medida; ya estaba fuera de alcance en search-abstention. |
| Aumentar el pool de 30 o el `limit` por defecto | Mueve las métricas de todos los llamadores y no toca la causa (muchas saves cerca); además la ventana de vec0 es de knn-prefilter. |
| Medir sólo con el store dev vivo | Cada corrida de `rag-eval` insertaría el corpus del golden en `~/.clawg-dev` y mezclaría la medición con el uso; el fondo es una copia en el directorio temporal. |

## Interacciones

- **knn-prefilter (37).** A escala el crowding por proyecto, por agente y por revisiones es real (`openspec/changes/knn-prefilter/exploration.md`). Es otra causa de «pocos o malos resultados» y confunde la medición del ranking: conviene que su puerta de medición corra antes, o registrar `passes` y la supervivencia del pool en cada caso y descartar los casos con pool corto de las conclusiones de peso y de puerta.
- **docs-search-quality (20).** Comparte `fts_match_expression` (`search.rs:711`) con el canal léxico de memorias, que este cambio no toca, y propone también un piso de relevancia para `search_project` (`fuse` devuelve siempre `limit` pasajes). Las constantes no se comparten (otro score, RRF sin unidad absoluta); lo que sí se comparte es el método: calibrar con consultas con y sin respuesta, en el orden que cada cambio decida.
- **piece-boundaries (83).** Cambia las filas antes de esta medición: una nota ya presente no se vuelve a partir al reimportar, así que el store dev actual (importado antes) tiene el corte viejo y el de producción tendrá el nuevo. Si las piezas que arrancan a mitad de palabra o el arranque compartido de 48 tokens causan parte de la inundación, el peso calibrado sobre el store viejo sobrecorrige en el nuevo. De ahí la repetición de la línea base sobre los dos cortes.
- **embed-throughput (85).** Baja el costo de reconstruir un fondo (A2) y, si adopta INT8, mueve la distribución de cosenos y con ella el piso y cualquier puerta absoluta: la puerta se guarda con el contrato de embeddings con el que se calibró, y se vuelve a medir tras cualquier cambio de ese contrato.
- **context-hit-room (75, archivado).** Los hits de un contexto con consulta reservan un cuarto del presupuesto (`budget / 4`); los resúmenes largos consumen esa reserva. Un peso mueve qué hits caben y, con ello, `context.found`; se mide en agent-view, no sólo en `search`.
- **memory-rank-fusion (34) y search-abstention (35), archivados.** No hay fusión por rangos: el score sigue siendo un producto, por lo que un peso lo compone sin cambiar la forma del corte. La abstención real (la parte A de 35) se entrega aquí.
- **english-recall (21).** Toda cifra se corta por pareja de idioma; una puerta por coseno tiene que no cortar `es>en` de más.
- **Contrato del modelo.** Un argumento nuevo o una frase nueva cambian `agent_contract.json` y `agent_contract.es.json` (`.claude/CLAUDE.md`, Testing) y las descripciones de `tools_es.rs`: el diff tiene que ser intencional.

## Alcance candidato

1. `rag-eval` con un fondo (A1, y A2 para construirlo): alcance de producción (todos los agentes legibles, proyecto del caso, edad de las filas), informe por tipo, coseno crudo, hueco y `passes`, y la medición del mismo golden con y sin fondo.
2. Un formato de caso que cite una fila del fondo por su origen, y un set minado del historial con una porción sin respuesta, con la partición calibración y prueba.
3. Si el paso 1 lo pide: un peso por tipo en `ScoringConfig` (con su espejo para el barrido) y la excepción elegida.
4. Si el paso 1 lo pide: una puerta de abstención en el daemon y en el núcleo para el contexto, con su dato de calibración (el contrato de embeddings) y la frase que ya existe.
5. Specs: deltas de `specs/retrieval`, `specs/eval` y, si cambia el contrato, `specs/mcp`; `.claude/CLAUDE.md` sólo si cambia algo visible.

### No-alcance

- El corte de piezas (83), la cuantización y la velocidad del embedder (85), la ventana de vec0 (37) y el canal léxico de docs (20).
- Cambiar el piso 0,79, `RELEVANCE_RATIO` 0,35 o las tasas de decaimiento por defecto (sólo como brazos de un barrido, no como resultado).
- Dejar algo fuera de una importación o borrar filas.
- Un reranker, otro modelo o un índice aproximado.
- Versionar el fondo, los casos minados o los resultados: contienen texto personal y `docs/eval/**` está ignorado (`.gitignore:32`).

## Decisiones abiertas

Son del usuario.

- [ ] **Qué store hace de fondo.** El dev actual (importado antes de piece-boundaries) o uno reimportado después; recomendado: los dos, para atribuir el efecto del corte de piezas.
- [ ] **Forma del fondo.** Copia de un snapshot (recomendado: sin re-embebido), export re-embebido (portable) o sintético.
- [ ] **Tamaño del set minado.** Con 6 casos moviéndose en una dirección como mínimo y una partición a la mitad, un set de ~60 casos solo distingue un cambio que mueva ~20 % de los casos; propuesta: al menos 80 con respuesta y 20 sin respuesta.
- [ ] **Si los casos llevan proyecto.** Con proyecto reproducen al agente real (y su crowding); sin él, al peor caso `all_projects`.
- [ ] **La excepción de los resúmenes.** Argumento explícito (i), lista de palabras (ii) o ninguna (iii, recomendada por defecto).
- [ ] **Forma del peso.** Factor en `score()` (recomendado) o tope por tipo en la lista mostrada; y si el brazo B3 entra al barrido.
- [ ] **Dónde se decide la puerta.** Daemon y núcleo (recomendado), o también el puente; sobre el coseno crudo (C1), el criterio relativo (C2) o los dos.
- [ ] **Si la puerta puede bajar `MIN_SHOWN_HITS`.** Cada respuesta perdida cuesta 2,2 puntos con 46 casos (search-abstention); con más casos se mide.
- [ ] **Umbrales de aceptación**, fijados antes de medir (propuestas abajo).
- [ ] **Uno o tres cambios.** Un cambio con puertas entre pasos (recomendado) o partir el paso 3.

## Cómo se mide

**Tests RED (fallan hoy):**

1. `score.rs`: dos hits idénticos salvo el tipo (`session_summary` y `semantic_fact`, mismo coseno, edad, importancia y tier) puntúan **igual** hoy; con el peso, el resumen puntúa menos y el hecho no cambia.
2. `search.rs` con un store de prueba: 12 resúmenes a coseno `c` y un hecho a `c − ε`; hoy el hecho queda 13º, con el peso queda primero. Y con la excepción elegida, una consulta que pide resúmenes los ordena como hoy.
3. `api.rs` (`POST /v1/search`, `limit` 5): con 10 resúmenes y 3 hechos igual de cercanos, a lo sumo los lugares acordados del top 3 son resúmenes.
4. `relevance.rs` (o la función pura equivalente): una lista cuyo mejor coseno crudo está bajo el umbral muestra 0 hits; hoy muestra 2 por `MIN_SHOWN_HITS`. Una lista con un hit sobre el umbral y otro bajo conserva el comportamiento actual.
5. `recall_context` sobre un store con todos los hits bajo el umbral: dice la frase de abstención en vez de listar; con un daemon anterior se lee como hoy.
6. `eval_api.rs`: una búsqueda de producción alcanza filas de otro agente (hoy `Reach::Own` no las ve).
7. `rag-eval`: `--background` se parsea (hoy `UnknownFlag`), el corpus del golden queda resoluble sobre un fondo de 200 filas con el embedder `--fake`, los hits del fondo no cuentan como esperados, y el informe suma el recuento por tipo del top 5 y la ruta del fondo.
8. `golden.rs`: un caso que cita una fila del fondo por su origen valida; uno que cita una fila inexistente falla con salida 2.
9. Un test de migración **no hace falta**: no hay cambio de esquema; si el paso 2 eligiera tocar `tier`, sí (siembra de la versión anterior con vectores).

**Medición.** Con el modelo real y el fondo del perfil dev (`rag-eval --real --agent-view --background …`, un perfil y un modelo descargado):

- **Línea base a escala**: el golden de 46 con y sin fondo (`--diff`); el set minado entero; por tipo, por pareja de idioma y por partición. Recuento de lugares del top 3 por tipo contra la proporción de filas del tipo.
- **Distribuciones**: coseno crudo del mejor hit y hueco, en consultas con y sin respuesta, por idioma.
- **Cada brazo contra la línea base** con `rag-eval --diff … --fail-on-regression` y los resultados en `docs/eval/results/`.

**Aceptación propuesta (a fijar antes de medir):** U1, la parte de los lugares del top 3 que ocupan los resúmenes en las consultas que no los piden baja de ≥ 50 % a ≤ 25 %; U2, `search.found` y `context.found` del set minado y del ancla no empeoran de forma distinguible (`--fail-on-regression` sale con 0) y el ancla no mueve ningún caso sin explicación; U3, la puerta abstiene bien en al menos 16 de 20 sin respuesta (cota inferior de Wilson publicada) sin que `found` de las consultas con respuesta baje más de 2 casos en prueba; U4, `es>en` no baja de forma distinguible (informativo, pero se informa).

## Riesgos

- **Sobreajustar un set chico.** Con ~100 casos y una partición, un peso o un umbral elegidos en calibración pueden no sostenerse en prueba. Se informa la prueba y se dice «no distinguible» cuando corresponde.
- **El fondo es de una persona.** Las conclusiones (proporción de resúmenes, edad, idioma) son las de ese historial; el peso se llama por su medición, no por «lo que le pasa a todos». La proporción de 11 % y los 53 % de lugares salen de una sola importación.
- **Calibrar sobre el corte de piezas equivocado** (ver piece-boundaries) y heredar la inundación del chunker en un peso por tipo.
- **Una puerta atada al embedding** que se invalida con el prefijo, el pooling o INT8 sin aviso. Se mitiga guardando el contrato de embeddings con el dato y repitiendo la medición tras cualquier cambio.
- **Cortar lo cruzado de idioma.** `es>en` ya es la pareja más débil (6/19 por coseno puro); una puerta por coseno la corta primero.
- **Perder una respuesta real** con el peso o la puerta: el costo principal; lo mide el set minado, no el ojo.
- **Costo de la medición.** Reembeber 52.000 filas por corrida no es viable (A2); por eso A1 y una copia. Si el snapshot no coincide con el contrato de embeddings, el fondo no abre y hay que reconstruirlo.
- **Contrato del modelo.** Cualquier argumento o frase nueva cambia lo que ve el agente en cada sesión.
