# ranking-at-scale — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M (paso 1 M; pasos 2 y 3, S y S–M, sólo si su puerta los abre) | 83 (archivado); se coordina con 37, 20 y 85 (archivado) | ver [`state.yaml`](state.yaml) |

## Problema

En las 12 búsquedas de validación sobre las ~52.000 filas de una importación real (7 con respuesta, 5 sin), las piezas de `session_summary`, el 11 % de las filas, ocuparon 19 de los 36 lugares del top 3 (53 %), y las cinco preguntas sin respuesta devolvieron cinco hits con score de 0,2 a 0,33 ([exploración](exploration.md)). Lo que explica el código:

- **El score no pesa el tipo.** `score()` es `relevancia × max(piso de tier, decaimiento) × importancia × estado` (`crates/clawg-core/src/memory/score.rs:264-308`). El tipo sólo elige la tasa de decaimiento (`lambda_for`, `score.rs:228-242`), y un resumen nace en tier 2, con piso 0,2 (`crates/clawg-core/src/memory/tier.rs:49`, `score.rs:128`). `collapse_saves` deja un hit por save (`crates/clawg-core/src/memory/search.rs:167-184`): los 19 lugares son 19 saves distintas.
- **Nada mira la cercanía real.** Un score `s` exige un coseno ≥ 0,79 + 0,21·`s` (`rescale_cosine`, `score.rs:219-221`), o sea ≥ 0,832 para 0,2 y ≥ 0,859 para 0,33. El piso 0,79 es el percentil 5 de los cosenos con respuesta (`score.rs:166-168`) y no dice si hay respuesta. El corte relativo no quita esos hits, y `MIN_SHOWN_HITS` = 2 muestra los dos primeros con cualquier score mayor que 0 (`shown_hits`, `crates/clawg-cli/src/relevance.rs:23`, `:36-51`).
- **`rag-eval` no puede medirlo.** La ruta principal siembra bajo un solo agente (`crates/clawg-eval/src/bin/rag-eval/seed.rs:19`, `:71-82`) y busca con `Reach::Own` y `project_id: None` (`crates/clawg-core/src/eval_api.rs:105-111`), así que no ve las filas del agente `migrated` de una importación. Agent-view pasa por el daemon, pero `EvalIngest::send` no recibe la edad (`eval_api.rs:589-601`) y busca sin proyecto (`rag-eval/agent_view/runner.rs:140`, `:153`). Ninguna bandera carga un fondo (`rag-eval/cli.rs:289-377`), un caso sólo cita filas del corpus del golden (`rag-eval/golden.rs:328-346`) y ningún golden tiene un caso `unanswerable`, aunque la clase existe (`golden.rs:113`). El ancla son 46 casos (`openspec/config.yaml`): un caso son 2,2 puntos.

### Premisas que cambiaron desde la exploración (2026-10-23)

| Premisa | Hoy |
|---|---|
| Cada pieza se embebe detrás de las 48 primeras palabras de su save | 32 tokens desde piece-boundaries (`crates/clawg-core/src/memory/chunker/mod.rs:40`). El store dev, importado antes, conserva la cabecera de 48 y los cortes a mitad de palabra; prod tiene los nuevos. |
| B3 (tasa y piso de `session_summary`) es «barrible hoy» | **Falsa.** `EvalScoringConfig` sólo espeja importancia y piso de coseno (`eval_api.rs:124-130`), y las banderas de barrido son `--sweep-imp`, `--sweep-cos-floor` y `--sweep-lexical` (`cli.rs:344-370`). Barrer B3 pide campos y bandera nuevos. |
| El informe por caso sale de lo que `rag-eval` ya ve | `EvalHit` lleva `distance` pero no `memory_type`, `project_id` ni la edad (`eval_api.rs:203-213`); `passes` sólo sale en la línea de debug (`KnnOutcome`, `search.rs:427-444`; `format_pool_line`, `:664`). |
| Un snapshot se abre con el contrato comprobado | `EvalIngest::open` pone el embedder sin comprobarlo (`eval_api.rs:558-581`); lo comprueba el daemon (`verify_or_reembed`, `crates/clawg-core/src/memory/bootstrap.rs:534-572`), que además re-embebería un store CLS. |
| Edad de un fondo | La búsqueda lee la hora al correr (`search.rs:1248`) y el decaimiento mide desde `last_accessed_at` (`score.rs:285-289`): un fondo con fechas reales envejece entre corridas, a diferencia del `age_days` relativo del golden. |
| Ubicación por proyecto de una reimportación | Sin `--project-map`, todas las filas quedan sin proyecto (verify de embed-throughput). |
| Citas que se movieron | `client.rs:189` → `:194`; `client.rs:626-630` → `:671-675`; `api.rs:400-415` → `:411-426`; `api.rs:1462-1466` → `:1484-1490`; `server.rs:189-204` → `:200-219`; `golden.rs:40-56` → `:328-346` (la validación); `golden.rs:112` → `:113`; `translate.rs:224`, `:608` → `:225`, `:609`; `cli.rs:288-375` → `:289-377`. Lo demás sigue donde la exploración lo cita. |

## Objetivo

1: que lo que se recupera sea bueno a la escala real. Primero la medición; después, sólo si la medición lo pide, un peso de lectura para `session_summary` y una puerta de abstención. Nada de lo guardado cambia.

## Decisiones

Cierran las [decisiones abiertas de la exploración](exploration.md#decisiones-abiertas), tomadas el 2026-10-28. Las elecciones de los pasos 2 y 3 se toman en su puerta, con los datos del paso 1: acá quedan su opción por defecto y el dato que la decide.

1. **Fondo: un snapshot de cada corte de piezas (A1).**
   - *Corte viejo:* una copia del store dev actual (`clawg backup --profile dev`), importado antes de piece-boundaries.
   - *Corte nuevo:* un snapshot de una reimportación del export completo en un `CLAWG_TEST_HOME` aislado, con el corte que tiene prod. Se construye una vez (~16 min en debug: 947,6 s sumando las fases del verify de 85, 825,8 de `embed`).
   - Sirven para atribuir la inundación al corte de piezas o al tipo. A2 (el export re-embebido) es sólo cómo se construye el segundo; A3 (sintético) queda fuera.
   - El fondo vive en una copia en el directorio temporal del sistema, nunca versionada. Un snapshot cuyo contrato de embeddings no es el del embedder de la corrida se rehúsa sin tocarlo: `rag-eval` no repara ni re-embebe.
   - Los dos snapshots ubican las filas en los mismos proyectos (el mismo `--project-map`, o los mismos `projects move` después). Si no, la comparación mezcla el corte con el alcance.
2. **Casos: al menos 80 con respuesta y 20 sin respuesta**, minados del historial por el agente, con las 12 búsquedas de validación como semilla, y validados uno por uno por el usuario. Cada caso cita su fila esperada por su origen (`created_by_ref`, `engram:<sync_id>`, indexado en `crates/clawg-core/migrations/V23__origin_project.sql:18`), una forma de caso nueva al lado de `corpus_refs`. El origen es lo único igual en los dos snapshots: los ids y las piezas cambian con el corte. Por eso el caso cita la save entera, y un hit en cualquiera de sus piezas cuenta. Viven en `docs/eval/` (fuera de git, `.gitignore:32`), con la partición calibración y prueba por hash de la consulta de hoy (`split_of`, `rag-eval/report.rs:180`) y consultas en las parejas `es>es`, `en>en` y `es>en`.
3. **Alcance de la búsqueda: el de producción, con el proyecto del caso.** Todos los agentes legibles (`Reach::AllBut` sin ocultos, lo que lee el daemon sin `sharing.json`) y el proyecto del caso bajo el predicado de producción (`memory_project_predicate!`, `search.rs:107-114`). Cada caso registra `passes` y la supervivencia del pool. Un caso con el pool de memorias corto (menos saves que las 30 que pide `SearchScope::all_default`, `search.rs:36-46`) queda fuera de las conclusiones de peso y de puerta, y se cuenta aparte.
4. **Umbrales de aceptación, fijados ahora, antes de medir**, tal como los propuso la exploración:

   | # | Umbral | Lo mide |
   |---|---|---|
   | U1 | la parte de los lugares del top 3 que ocupan los resúmenes en las consultas que no los piden baja de ≥ 50 % a ≤ 25 % | paso 2 |
   | U2 | `search.found` y `context.found` del set minado y del ancla no empeoran de forma distinguible (`--fail-on-regression` sale con 0) y el ancla no mueve ningún caso sin explicación | pasos 2 y 3 |
   | U3 | la puerta abstiene bien en al menos 16 de 20 sin respuesta (cota inferior de Wilson publicada) sin que `found` de las consultas con respuesta baje más de 2 casos en prueba | paso 3 |
   | U4 | `es>en` no baja de forma distinguible (informativo, pero se informa) | pasos 2 y 3 |

5. **Un cambio, con una puerta entre cada paso.** Un cambio de ranking por vez, cada paso en sus propios commits. El paso 3 se parte en un cambio propio si el 1 y el 2 tardan.
6. **Paso 2, a decidir en la puerta 1:**

   | Elección | Por defecto | Lo decide |
   |---|---|---|
   | La excepción «salvo que la consulta lo pida» | (iii), ninguna: el último resumen ya abre cada contexto (`LAST_SESSION_SQL`, `crates/clawg-core/src/memory/prompt_builder.rs:860`) y los demás se listan con `list_memories` por tipo. (i), un argumento de tipos, sólo si hace falta; (ii), una lista de palabras, no se recomienda | cuántos casos del set minado esperan un `session_summary` que no es el último de su proyecto |
   | Forma del peso | B1, un factor en `score()` (`type_weights` en `ScoringConfig`, espejo en `EvalScoringConfig` y un `--sweep-*`) | si algún punto de B1 llega a U1 en calibración sin romper U2 ni U4; si no, B2 (tope por tipo en la lista mostrada) |
   | Si B3 entra al barrido | sí, como brazo de control | la edad de las saves que inundan: si casi todas ya están en el piso de tier 2 (a partir de los 209 días), subir la tasa no las mueve y B3 sólo puede actuar por el piso |

7. **Paso 3, a decidir en la puerta 2:**

   | Elección | Por defecto | Lo decide |
   |---|---|---|
   | Criterio | C1, umbral sobre el coseno crudo del mejor hit no exacto; C2 (hueco entre el 1º y el 10º) si el coseno no separa; C3 sólo si ninguno basta solo | las distribuciones de los dos, con y sin respuesta, por pareja de idioma, en calibración |
   | Dónde vive | daemon (`crates/clawg-cli/src/daemon/api.rs:1484-1490`) y núcleo para el contexto (`prompt_builder.rs:232-241`) | lo que lee el criterio: `distance` y los scores ya están en el daemon; el puente sólo si el criterio necesitara algo que el daemon no tiene, y hoy su `Hit` no trae `distance` (`crates/clawg-cli/src/client.rs:194`) |
   | Si la puerta puede bajar `MIN_SHOWN_HITS` | sí, a 0 sólo en la consulta en que se dispara; la constante sigue en 2 | cuántas consultas con respuesta pierden su fila en prueba (U3: a lo sumo 2) |

## Alcance

### Paso 1: medir a escala (entero)

1. **`rag-eval --background SNAPSHOT`**: la ruta de un snapshot de `clawg backup`. Lo copia como `memory.db` a un directorio temporal y nunca abre el original. Rehúsa un archivo con un `-wal` no vacío al lado, como `restore`, y un contrato de embeddings distinto del de la corrida. Siembra encima el corpus del golden. El informe nombra la ruta, el sha256 y el contrato del fondo, y su composición: filas por tipo, agente, idioma y con o sin proyecto.
2. **Alcance de producción en la búsqueda de evaluación**: `EvalScope` lleva el alcance y el proyecto (hoy fijos, `eval_api.rs:99-114`), y agent-view manda el proyecto del caso a `search` y `context` (`runner.rs:140`, `:153`).
3. **Edad en agent-view**: `EvalIngest::send` toma la edad de la fila, como ya la toma `EvalStore::insert` (`eval_api.rs:273-308`).
4. **Forma de caso por origen**: la save esperada se nombra por su `created_by_ref` y se resuelve contra el fondo. Un origen sin fila, o un caso por origen sin `--background`, invalida el golden (salida 2, `EXIT_GOLDEN_INVALID`, `rag-eval/main.rs:51`). Un caso nombra su proyecto de modo que se resuelva en los dos snapshots; la spec fija cómo.
5. **Informe por caso, sin cambiar el ranking**:
   - el tipo de cada hit del top 5;
   - el coseno crudo 1 − d²/2 del mejor hit no exacto (`cosine_from_l2_unit`, `score.rs:209-211`);
   - el hueco de score entre el 1º y el 10º;
   - la pareja de idioma (`query_lang` del caso y `memory.lang` de la fila);
   - `passes` y la supervivencia del pool;
   - la edad que lee el decaimiento.

   Agregados: los lugares del top 3 por tipo contra la proporción de filas del tipo en los casos que no piden resúmenes, y las distribuciones de coseno crudo y hueco con y sin respuesta, por idioma y por partición.
6. **El set minado y las corridas**: el ancla de 46 casos con y sin fondo (`--diff`). Sin proyecto, porque su corpus no tiene, así que compite todo el fondo. El set minado, en los dos cortes.

### Paso 2, si lo abre la puerta 1

Un peso de lectura para `session_summary` en la forma elegida, barrido, elegido en calibración e informado en prueba. Entra en `ScoringConfig::DEFAULT` sólo si cumple U1, U2 y U4. Lleva la excepción elegida y, si entra B3, sus campos de barrido.

### Paso 3, si lo abre la puerta 2

Una puerta de abstención con el criterio elegido, en el daemon y en el núcleo, que usa la frase que ya existe (`no relevant memories for this query: …`, search-abstention). Su umbral queda fijado por un test que nombra el contrato de embeddings con que se calibró, como el del piso (`score.rs:411-416`).

### Documentos

El delta de `specs/eval` en el paso 1; `specs/retrieval` en los pasos 2 y 3, y `specs/mcp` si cambia el contrato que ve el modelo. En `.claude/CLAUDE.md`, la sección Testing (`--background`, la forma de caso) y lo que cambie de la búsqueda. La fila 86 de `openspec/README.md`.

### No-alcance

- El corte de piezas (83), la velocidad y la cuantización del embedder (85), la ventana de vec0 (37) y el canal léxico de docs (20).
- Cambiar el piso 0,79, `RELEVANCE_RATIO` 0,35 o las tasas por defecto, salvo como brazos de un barrido.
- Dejar algo fuera de una importación, borrar filas o cambiar el `tier` guardado.
- Un reranker, otro modelo o un índice aproximado.
- Fijar la hora de la búsqueda para un fondo: las dos corridas de una comparación se hacen el mismo día.
- Versionar los fondos, los casos o los resultados: tienen texto personal.
- Ubicar las filas sin proyecto del import en prod (abajo).

## Precondición fuera del cambio

El [orden de ejecución](../../../README.md#orden-de-ejecución-de-lo-pendiente) deja un paso operativo, que no es código: ubicar las ~15.709 filas sin proyecto del import en prod con `clawg projects move --origin-project NOMBRE --to-project PROYECTO`, con 0.65.0 instalado y con autorización. Es condición de leer en prod lo que este cambio mida. No lo es de la medición, que corre sobre los snapshots del perfil dev y del home aislado. Este cambio no lo ejecuta.

## Cómo se mide

### Tests que fallan hoy (paso 1)

1. `eval_api.rs`: una búsqueda con el alcance de producción alcanza una fila de otro agente y una sin agente, y con un proyecto lee las suyas, las sin proyecto y toda `user_preference`, pero no las de otro proyecto. Hoy `Reach::Own` no ve la de otro agente.
2. `rag-eval`: `--background` se parsea (hoy `UnknownFlag`, `cli.rs:377`). Sobre un fondo de 200 filas hecho con el embedder `--fake`, el corpus del golden queda resoluble, los hits del fondo nunca cuentan como esperados y el informe suma el recuento por tipo del top 5, la ruta y la composición del fondo.
3. Después de una corrida, el snapshot original tiene los mismos bytes. Un snapshot con otro contrato, o con un `-wal` no vacío al lado, se rehúsa sin escribir nada.
4. `golden.rs`: un caso que cita su save por origen valida contra el fondo, y un hit en cualquiera de sus piezas cuenta. Uno que cita un origen sin fila, o que cita por origen sin `--background`, sale con 2. Hoy el campo no parsea (`deny_unknown_fields`).
5. Agent-view: una fila del corpus con `age_days` 300 queda con esa edad (hoy, con la de la corrida), y el caso manda su proyecto a `search` y `context` (hoy `None`).
6. Informe por caso: el tipo de cada hit del top 5; el coseno crudo del mejor no exacto, que con una fila igual a la consulta da 1; el hueco; la pareja; `passes`; la edad. Un caso con el pool corto queda marcado. Hoy ninguno de esos campos existe.

No hace falta test de migración: no hay cambio de esquema, identificadores, estados ni contrato.

### Si entra el paso 2

1. `score.rs`: dos hits iguales salvo el tipo (`session_summary` y `semantic_fact`) puntúan igual hoy. Con el peso, el resumen puntúa menos y el hecho no cambia.
2. `search.rs`, con 12 resúmenes a coseno `c` y un hecho a `c − ε`: hoy el hecho queda 13º, con el peso queda primero. Si la excepción es (i) o (ii), una consulta que pide resúmenes los ordena como hoy.
3. `POST /v1/search` con `limit` 5, con 10 resúmenes y 3 hechos igual de cercanos: a lo sumo los lugares acordados del top 3 son resúmenes.
4. Red de seguridad, que pasa hoy: el pool de 5 del detector de contradicciones no cambia con el peso. Con (i), `agent_contract.json` y `agent_contract.es.json` cambian a propósito y se vuelven a bendecir.

### Si entra el paso 3

1. `relevance.rs`, o la función pura que lo reemplace: una lista cuyo mejor coseno crudo está bajo el umbral muestra 0 hits, cuando hoy muestra 2 por `MIN_SHOWN_HITS`. Una lista con un hit sobre el umbral y otro bajo conserva lo de hoy.
2. `recall_context` con todos los hits bajo el umbral dice la frase de abstención en vez de listarlos. Con un daemon anterior, se lee como hoy.

### Medición con el modelo real

Necesita el modelo, los dos snapshots, el ancla y el set minado en `docs/eval/`, fuera de git. Leer el perfil dev (el backup, el modelo) pide autorización.

- **Línea base en los dos cortes.** Primero, el ancla de 46 con y sin fondo (`--diff`): es la primera cifra del cambio, el costo de la escala. Después, el set minado entero, por tipo, por pareja de idioma y por partición, con los lugares del top 3 por tipo y las distribuciones.
- **Cada brazo contra la línea base**, con `rag-eval --diff … --fail-on-regression` y los resultados en `docs/eval/results/`. Las dos corridas de una comparación sobre un mismo fondo se hacen el mismo día.
- **Puerta 1, al cerrar el paso 1.** Si a escala el top 3 no está inundado por resúmenes en los casos que no los piden, el paso 2 se cierra sin código. Se lee contra U1 sobre el corte nuevo, que es el de prod: con la línea base ya en ≤ 25 % no hay nada que bajar. El corte viejo sirve para atribuir. Si en cambio el peso barrido no mueve los resúmenes más que el ruido (`--diff` no distinguible), el paso 2 se cierra sin cambiar `DEFAULT`.
- **Puerta 2, al cerrar el paso 2.** El paso 3 sólo avanza si C1 o C2 separan, en calibración, las consultas con respuesta de las sin respuesta con la cota de Wilson comprometida en U3. Si no, se cierra dejando escritos los casos y las distribuciones.

## Migración y vuelta atrás

No cambian el esquema, los identificadores guardados, los estados ni el contrato de embeddings. El paso 1 no toca el ranking de producción. Su código es de evaluación (`eval_api.rs`, tras la feature `eval`), y lo que exponga de `search.rs` no cambia el orden. Los pasos 2 y 3 cambian un valor por defecto cada uno, en su propio commit, así que volver atrás es revertirlo. `rag-eval` escribe sólo en su copia: ni el snapshot original ni los perfiles se escriben.

## Riesgos

- **Sobreajustar un set chico.** Con ~100 casos y una partición, lo elegido en calibración puede no sostenerse en prueba. Se informa la prueba y se dice «no distinguible» cuando corresponde.
- **Pocos casos útiles.** Si el crowding (37) deja muchos casos con el pool corto, quedan pocos para concluir. Se cuentan, y una conclusión que no llega a 6 casos en una dirección no se toma.
- **Dos snapshots que no se comparan.** Uno con otra ubicación por proyecto, o con otro contrato, mezcla causas. Se mitiga con la decisión 1 y con la composición del fondo en el informe.
- **Un fondo que envejece.** Corridas de días distintos mueven el decaimiento de las filas que todavía no llegaron a su piso. Las dos corridas de una comparación van el mismo día, y el informe guarda la fecha.
- **El fondo es de una persona.** Las proporciones (11 %, 53 %) salen de una sola importación. El peso se justifica con su medición, no con «lo que le pasa a todos».
- **Una puerta atada al embedding.** Se invalida con el prefijo, el pooling o INT8 sin aviso. Por eso se guarda con su contrato y se vuelve a medir tras cualquier cambio de contrato.
- **Cortar lo cruzado de idioma.** `es>en` ya es la pareja más débil, y una puerta por coseno la corta primero (U4).
- **Perder una respuesta real** con el peso o la puerta: es el costo principal, y lo mide el set minado.
- **El contrato del modelo.** Con la excepción (i), o con una frase nueva, cambia lo que ve el agente en cada sesión.
