# ranking-at-scale — Especificación

Suma RS1 a RS12 (paso 1, entero) a [`specs/eval`](../../../specs/eval/spec.md), y RS13 y RS14 (condicionales) a [`specs/retrieval`](../../../specs/retrieval/spec.md). Origen: [proposal](proposal.md), decisiones del 2026-10-28.

Las salidas de `rag-eval` que se citan son las de `main.rs:49-59`: 1 fallo de ejecución o de uso (`EXIT_RUNTIME_FAILURE`), 2 golden inválido (`EXIT_GOLDEN_INVALID`), 3 sin modelo, 4 archivo de resultados existente, 5 golden no apto, 6 regresión. Un fondo inservible es una entrada que no se puede leer, así que sale con 1: no se suma un código nuevo.

## Paso 1: medir a escala (entra entero)

### RS1. `--background SNAPSHOT`

`rag-eval` MUST aceptar `--background <archivo>`, la ruta de un snapshot de `clawg backup`. MUST copiarlo como `memory.db` a un directorio nuevo del temporal del sistema y trabajar sólo sobre la copia: el original MUST NOT abrirse ni escribirse, y la copia MUST borrarse al terminar, también ante un error. Siembra encima el corpus del golden (si lo hay). `--background` junto a `--docs`, `--lang-detect`, `--fts-tokenizer` (vacía `memory_fts` sin rellenarla: las filas del fondo saldrían del canal léxico), `--follow-up`, `--audit` o `--lexical-only` MUST ser un error de uso (salida 1); los `--sweep-*` MUST aceptarse, porque el paso 2 barre sobre el fondo. Un hit del fondo MUST conservar su puesto en la lista ordenada, escrito `bg` y sin su id: hoy un id fuera del golden se descarta (`filter_map`, `main.rs:445-446`) y eso subiría la esperada por encima de cada hit del fondo. Sin `--background`, todo MUST medir igual que antes.

#### Escenario: el original queda intacto
- GIVEN un snapshot de 200 filas hecho con el embedder `--fake`
- WHEN corre `rag-eval --fake --background SNAP`
- THEN el sha256 de SNAP es el de antes, y el directorio temporal ya no existe al salir

#### Escenario: el corpus del golden se resuelve sobre el fondo
- GIVEN el mismo fondo y un golden de corpus chico
- WHEN corre la medición
- THEN cada `expected_refs` se resuelve y los hits que vienen del fondo nunca cuentan como esperados

#### Escenario: sin la bandera
- GIVEN un golden sin las claves nuevas
- WHEN corre sin `--background`
- THEN `per_case` y `aggregate` son idénticos a los de antes y ningún bloque nuevo aparece

#### Escenario: un hit del fondo conserva su puesto
- GIVEN un fondo cuya fila más cercana a la consulta no es la esperada
- WHEN se ordena el caso
- THEN el primer puesto es `bg` y la esperada queda segunda, no primera

**Tests:** `background_parses` (hoy `UnknownFlag`, `cli.rs:377`), `background_with_docs_is_a_usage_error` (y los otros modos), `a_background_run_leaves_the_snapshot_byte_identical`, `the_background_copy_is_removed_even_when_the_run_fails`, `a_run_without_background_measures_as_before`, `a_background_hit_ranked_first_keeps_its_place`.

### RS2. Un fondo que no se puede usar se rehúsa sin tocarlo

`rag-eval` MUST rehusar, con salida 1 y la causa nombrada, sin escribir nada ni en el original ni en la copia, un snapshot con un `-wal` no vacío al lado (como `restore`), uno dañado, uno sin historia de clawg, uno de un esquema más nuevo que el del binario o divergente (la regla de `clawg restore`), uno sin contrato de embeddings y uno cuyo contrato no es el del embedder de la corrida. MUST NOT repararlo ni re-embeberlo: un store CLS se rehúsa nombrando que hay que abrirlo una vez con el daemon y respaldarlo de nuevo. Todo eso se decide en sólo lectura, antes de abrir la copia. Un snapshot de un esquema más viejo MUST migrarse en la copia, como lo abre el daemon, y el informe MUST decir de qué esquema a cuál.

#### Escenario: `-wal` no vacío
- GIVEN un snapshot con un `-wal` de bytes al lado
- WHEN corre con `--background`
- THEN sale con 1 nombrando el `-wal`, y SNAP y su `-wal` conservan sus bytes

#### Escenario: otro contrato
- GIVEN un snapshot cuyo contrato es el de CLS
- WHEN corre con el embedder de la corrida (media)
- THEN sale con 1 nombrando los dos contratos, sin re-embeber

#### Escenario: esquema más viejo
- GIVEN un snapshot de un esquema anterior al del binario
- WHEN corre con `--background`
- THEN la copia se migra, el original conserva sus bytes y el bloque `background` dice `schema` de y a

**Tests:** `a_snapshot_with_a_nonempty_wal_is_refused_untouched`, `a_snapshot_of_another_embedding_contract_is_refused_untouched`, `a_newer_schema_snapshot_is_refused`, `an_older_schema_snapshot_is_migrated_in_the_copy`.

### RS3. El informe nombra el fondo

El informe MUST llevar un bloque `background` con la ruta dada, el sha256 del archivo, el contrato de embeddings, el esquema de y a (`schema`), la hora de la búsqueda (`searched_at_ms`) y la composición de las filas vivas: por tipo, por agente, por idioma (`memory.lang`, con `null`), por `source`, con o sin proyecto, por proyecto (`by_project`, por nombre) y su ubicación (`placement`: cada `origin_project` con el nombre del proyecto en que quedó, o `null`, y sus filas). Sin `placement`, la ubicación de los dos cortes no se puede comparar. MUST listar además en `golden_twins` las refs del golden cuyo `content_hash` también está en el fondo. MUST NOT llevar texto, ids de fila ni `topic_key` del fondo. Sin `--background` MUST NOT aparecer.

#### Escenario: composición
- GIVEN un fondo de 200 filas con 3 tipos y 2 agentes
- WHEN termina la corrida
- THEN `background` suma 200 por tipo, por agente, por idioma y por proyecto, y no contiene ningún contenido de fila

**Tests:** `the_report_describes_the_background_without_text`.

### RS4. Alcance de producción en la búsqueda de evaluación

La búsqueda de evaluación (`EvalScope`) MUST usar el alcance del daemon sin `sharing.json`: todos los agentes legibles (`Reach::AllBut` sin ocultos) y, si el caso lo nombra (RS7), su proyecto bajo el predicado de producción. Con proyecto MUST leer las filas de ese proyecto, las sin proyecto y toda `user_preference`, y MUST NOT leer las de otro proyecto. Agent-view MUST mandar el proyecto del caso a `search` y a `context`. Sin `project` en el caso, la búsqueda MUST ser la de antes de este cambio salvo por el alcance de agentes.

#### Escenario: otro agente
- GIVEN filas de dos agentes y una sin agente
- WHEN se busca con el alcance de producción
- THEN se alcanzan las tres (con `Reach::Own` no se alcanza la de otro agente)

#### Escenario: proyecto
- GIVEN filas del proyecto P, de Q, sin proyecto y una `user_preference` de Q
- WHEN se busca con P
- THEN se leen las de P, las sin proyecto y la `user_preference`, y no las otras de Q

#### Escenario: agent-view manda el proyecto
- GIVEN un caso con `project`
- WHEN corre agent-view
- THEN `search` y `context` reciben su id (hoy `None`)

**Tests:** `eval_api.rs`: `production_reach_reads_other_agents_and_unowned_rows`, `a_project_search_reads_its_own_unowned_and_every_preference`; `agent_view/runner.rs`: `agent_view_sends_the_case_project_to_search_and_context`.

### RS5. Edad de la fila en agent-view

`EvalIngest::send` MUST tomar la edad de la fila (`age_days`) como `EvalStore::insert`. Una fila con `age_days` 300 MUST quedar con esa edad y no con la de la corrida.

#### Escenario: fila vieja
- GIVEN una fila del corpus con `age_days` 300
- WHEN agent-view la siembra
- THEN su `created_at` es la de la corrida menos 300 días

**Tests:** `eval_ingest_backdates_created_at`.

### RS6. Caso que cita su save por origen

`RetrievalCase` MUST aceptar `expected_origins` opcional, una lista de `created_by_ref` (`engram:<sync_id>`), resuelta sólo entre las filas vivas con `source = 'import'`: una save nativa guarda en `created_by_ref` el id de su agente (`store_insert.rs:231`). La resolución MUST correr una vez por fondo, antes de medir. Un hit en cualquiera de las piezas de la save citada MUST contar como esa esperada; un hit del fondo que no está citado MUST NOT contar nunca. Un caso con `expected_origins` MUST tener `corpus_refs` y `expected_refs` vacías o ausentes, y MUST NOT tener `stale_refs`. Un caso `unanswerable` MUST tener `expected_origins` vacía; uno de otra clase MUST tener `expected_refs` o `expected_origins` no vacía. La validación MUST fallar con salida 2 (`EXIT_GOLDEN_INVALID`) cuando un caso cita un origen y no hay `--background`, cuando un origen no tiene fila viva en el fondo (también si una reimportación dejó su save `superseded`), y cuando la lista mezcla esperadas por `expected_refs` y por origen. Una clave desconocida MUST seguir siendo error de parseo. Un golden sin `expected_origins` MUST valer igual que hoy (su `content_sha256` no cambia).

#### Escenario: un hit en cualquier pieza
- GIVEN una save de 3 piezas citada por su origen
- WHEN sólo la segunda pieza sale en el top 5
- THEN el caso acierta, con el puesto de esa pieza

#### Escenario: origen sin fila
- GIVEN un caso que cita `engram:no-existe`
- WHEN valida contra el fondo
- THEN sale con 2 nombrando el caso

#### Escenario: sin fondo
- GIVEN un caso por origen
- WHEN corre sin `--background`
- THEN sale con 2

**Tests** (`golden.rs`, `main.rs`): `a_case_may_cite_its_expected_save_by_origin`, `an_origin_without_a_row_fails_validation`, `an_origin_case_without_background_fails_validation`, `an_origin_case_cannot_also_carry_corpus_refs`, `a_hit_on_any_piece_of_the_cited_save_counts`, `a_background_hit_not_cited_never_counts`, `a_golden_without_origins_measures_exactly_as_before`.

### RS7. Cómo un caso nombra su proyecto

`RetrievalCase` MUST aceptar `project` opcional: el nombre de un proyecto. `rag-eval` MUST resolverlo en cada fondo con `resolve_project` mismo (`registry.rs:848`), sin crear: primero como id y después como el nombre de un único proyecto vivo, sin distinguir mayúsculas. MUST usar el id que resulte en ese fondo, pasado por `canonical`. Un nombre sin proyecto, con dos (ambiguo), uno que lleva a un proyecto borrado o `project` sin `--background` MUST ser un golden inválido (salida 2). Si la save citada (RS6) tiene filas fuera del alcance del proyecto del caso (de otro proyecto, salvo `user_preference`), el golden MUST ser inválido. Sin `project` la búsqueda va sin proyecto, como el ancla. Un caso `unanswerable` puede llevar `project` igual que uno con respuesta.

*Decisión:* el nombre es lo único que sirve para los casos sin fila esperada y es el mismo en los dos snapshots mientras ambos ubiquen las filas bajo el mismo proyecto (decisión 1 de la propuesta); los ids de proyecto pueden diferir entre cortes y `origin_project` (V23) nombra el proyecto de la herramienta de origen, no el de clawg, y no existe en filas nativas.

#### Escenario: el mismo caso en dos cortes
- GIVEN dos snapshots donde el proyecto «app» tiene ids distintos
- WHEN corre el mismo caso con `"project": "app"` en cada uno
- THEN la búsqueda usa el id de cada snapshot

#### Escenario: nombre ambiguo o ausente
- GIVEN un fondo con dos proyectos vivos de nombre «app», o con ninguno
- WHEN valida
- THEN sale con 2 nombrando el caso

#### Escenario: sin respuesta, con proyecto
- GIVEN un `unanswerable` con `project`
- WHEN corre
- THEN busca bajo ese proyecto y cuenta en `abstention`

**Tests:** `a_project_name_resolves_to_each_backgrounds_own_id`, `an_ambiguous_or_unknown_project_name_fails_validation`, `project_without_background_fails_validation`, `an_expected_save_outside_the_case_project_fails_validation`, `an_unanswerable_case_may_name_a_project`.

### RS8. Informe por caso

Cada caso activo de `per_case` de una corrida con `--background` MUST llevar, sin cambiar el ranking: `project` (el nombre que dio el caso) y `expected_type` (el `memory_type` de la save esperada); `top5_types` (el `memory_type` de cada hit del top 5); `decay_age_days`, una lista alineada con `top5_types` con la edad que lee el decaimiento de cada hit, desde `last_accessed_at` (la decisión del brazo B3 lee la edad de las saves que inundan, no la de la esperada); `best_cosine` (el **máximo** coseno crudo 1 − d²/2 entre los hits no exactos devueltos, por `cosine_from_l2_unit`, sin decaimiento ni importancia; 1 si una fila es igual a la consulta; ausente si todos son exactos); `score_gap_1_10` (score del 1º menos el del 10º, ausente con menos de 10 hits); la pareja de idioma (`query_lang` del caso y `memory.lang` de la save esperada, o el detector si es NULL, como `by_pair`); `passes` (1 o 2); `pool_survival`, que es `{requested, saves, eligible, expected_in_pool}` (lo pedido al pool de `memory`, las saves distintas que devolvió, las filas elegibles cuando la ventana volvió corta y si una fila de la save esperada estaba en el pool; `expected_in_pool` ausente para un `unanswerable` y falso si sólo llegó por el canal léxico; no se informa qué pasada la trajo); y `short_pool` con su `short_pool_cause` (RS9). Un caso `unanswerable` MUST llevar `best_cosine`, `score_gap_1_10` y `top5_types` en un bloque aparte, sin entrar en las métricas ordenadas. Los campos MUST NOT llevar texto, ids de fila ni claves.

#### Escenario: coseno de una fila igual
- GIVEN una fila con el texto de la consulta
- WHEN se informa el caso
- THEN `best_cosine` es 1 (dentro de la tolerancia del embedder falso)

#### Escenario: tipos
- GIVEN un top 5 de 3 `session_summary` y 2 `semantic_fact`
- WHEN se informa
- THEN `top5_types` los lista en ese orden

**Tests:** `a_case_reports_the_type_of_each_top5_hit`, `best_cosine_of_an_identical_row_is_one`, `the_gap_is_first_minus_tenth`, `a_case_reports_passes_pool_survival_and_decay_age`, `an_unanswerable_case_reports_its_distribution_apart`, `the_per_case_report_holds_no_stored_text_or_row_id`; hoy `EvalHit` no trae tipo, proyecto ni edad (`eval_api.rs:203-213`).

### RS9. Pool corto

Un caso cuyo pool de KNN de `memory` vuelve, después de su reintento, con menos saves distintas que las 30 que pide (`SearchScope::all_default`) MUST llevar `short_pool: true` y MUST quedar fuera de las conclusiones de peso y de puerta (agregados RS10 y veredictos de pasos 2 y 3), contado aparte en `short_pool_cases` por causa. `short_pool_cause` MUST ser `scope` cuando las filas elegibles del alcance son menos de 30, y `crowded` cuando no lo son y la ventana de vec0 volvió tapada. El caso MUST seguir en `per_case` y en las métricas generales.

*Por qué el pool devuelto y no las saves alcanzables:* todo proyecto lee las filas sin proyecto y toda `user_preference`, y la importación dejó miles sin proyecto, así que ningún alcance de un fondo real baja de 30 saves alcanzables. Contarlas nunca dispararía, y el crowding pasaría sin verse. El pool devuelto sale del `KnnOutcome` sin ninguna consulta de más.

#### Escenario: alcance chico
- GIVEN un caso cuyo alcance tiene 12 saves
- WHEN se informa
- THEN `short_pool` es verdadero, `short_pool_cause` es `scope` y `short_pool_cases` lo cuenta

#### Escenario: ventana tapada
- GIVEN un caso cuyo alcance tiene más de 30 saves, pero la ventana de vec0 vuelve, después del reintento, con menos de 30 de ellas
- WHEN se informa
- THEN `short_pool` es verdadero y `short_pool_cause` es `crowded`

**Tests:** `the_memory_pool_counts_saves_and_reports_its_eligible_rows` (`search.rs`), `a_case_whose_pool_returns_fewer_than_thirty_saves_is_a_short_pool`, `a_crowded_window_is_told_apart_from_a_small_scope`, `short_pool_cases_stay_out_of_the_weight_aggregates`.

### RS10. Agregados del paso 1

El informe MUST agregar, sobre casos no `short_pool`: los lugares del top 3 por tipo contra la proporción de filas del tipo en los casos que no piden resúmenes; y las distribuciones de `best_cosine` y de `score_gap_1_10` con y sin respuesta, por pareja de idioma y por partición (`calibration`/`test`). Los agregados MUST ser lógica pura sobre `per_case`.

#### Escenario: lugares por tipo
- GIVEN 10 casos que no piden resúmenes, con 15 de 30 lugares del top 3 en `session_summary`
- WHEN se agrega
- THEN la parte de `session_summary` es 0,5 junto a su proporción de filas

**Tests:** `top3_places_by_type_against_row_share`, `distributions_split_by_answered_pair_and_partition`.

### RS11. Resultados sin texto personal

El archivo de resultados de una corrida con `--background` MUST llevar el bloque `background` (RS3) y MUST NOT llevar texto de filas del fondo, ids de fila ni `topic_key`. Los fondos, los casos y los resultados se escriben en `docs/eval/`, fuera de git. El nombre del archivo MUST llevar en su tag los 8 primeros hex del sha256 del fondo (`bg-<8 hex>`), para que el ancla con y sin fondo del mismo día no choquen. Un archivo existente MUST NOT pisarse (salida 4, como hoy).

**Test:** `the_results_file_with_a_background_holds_no_stored_text`.

### RS12. Línea base: el ancla con y sin fondo

`--diff` MUST comparar dos corridas del ancla (46 casos, sin `project`), una con `--background` y otra sin, y MUST informar los casos movidos por pareja como hoy; un golden de otra corrida con distinto `content_sha256` sigue siendo no comparable. Las dos corridas de una comparación sobre un mismo fondo deben ser del mismo día, porque el decaimiento lee el reloj al buscar y fijarlo está fuera de alcance. Nada puede forzarlo, así que se registra: `--diff` MUST imprimir el sha256 y `searched_at_ms` de cada lado y, cuando los dos leyeron el mismo fondo con más de 24 h entre medio, un aviso que nombra las dos horas, sin cambiar el código de salida (`--fail-on-regression` sigue mirando sólo las métricas). Una conclusión MUST pedir al menos 6 casos en una dirección; con menos, «no distinguible».

#### Escenario: ancla con fondo
- GIVEN los resultados del ancla con y sin fondo
- WHEN corre `rag-eval --diff sin.json con.json`
- THEN reporta `recall@5` y `search.found` con sus casos movidos y p exacto

#### Escenario: dos corridas de días distintos
- GIVEN dos resultados sobre el mismo fondo con 30 h entre sus búsquedas
- WHEN corre `rag-eval --diff`
- THEN imprime el aviso con las dos horas y sale con el código que dan las métricas

**Tests:** `diff_compares_an_anchor_run_with_and_without_background`, `a_diff_over_one_background_a_day_apart_warns`. Medición con el modelo real: la propuesta, «Medición con el modelo real», y el procedimiento de los dos snapshots en el [diseño](design.md#d10-procedimiento-los-dos-snapshots-verify-no-código).

## Pasos condicionales

Entran sólo si su puerta pasa; la forma exacta se decide en la puerta con los datos del paso 1.

### RS13. (Paso 2, si abre la puerta 1) Peso de lectura de `session_summary`

Sólo si a escala el top 3 sobre el corte nuevo está inundado por resúmenes (> 25 % en las consultas que no los piden) y el peso barrido se distingue del ruido, el score MUST pesar el tipo `session_summary` (forma por defecto B1, `type_weights` en `ScoringConfig`, con espejo en `EvalScoringConfig` y un `--sweep-*`; B2 si B1 no llega). Entra en `ScoringConfig::DEFAULT` sólo si cumple: **U1** (los lugares del top 3 que ocupan los resúmenes en las consultas que no los piden bajan de ≥ 50 % a ≤ 25 %), **U2** (`search.found` y `context.found` del set minado y del ancla no empeoran de forma distinguible, `--fail-on-regression` sale con 0, y el ancla no mueve ningún caso sin explicación) y **U4** (`es>en` no baja de forma distinguible; se informa). El peso MUST NOT cambiar lo guardado ni el pool de 5 del detector de contradicciones. Si no cumple, el paso se cierra sin cambiar `DEFAULT`.

#### Escenario: dos hits iguales salvo el tipo
- GIVEN un `session_summary` y un `semantic_fact` con el mismo coseno, edad e importancia
- WHEN se puntúan con el peso
- THEN el resumen puntúa menos y el hecho igual que antes

#### Escenario: 12 resúmenes y un hecho
- GIVEN 12 resúmenes a coseno `c` y un hecho a `c − ε`
- WHEN se busca con el peso
- THEN el hecho queda primero (hoy 13º)

#### Escenario: el detector no se mueve
- GIVEN el mismo store con y sin el peso
- WHEN el detector arma su pool de 5
- THEN es el mismo

**Tests:** `score.rs`: `a_session_summary_scores_below_an_equal_fact_with_the_weight`; `search.rs`: `twelve_summaries_no_longer_bury_a_slightly_less_close_fact`; `POST /v1/search` con `limit` 5: `at_most_the_agreed_places_of_the_top3_are_summaries`; `the_contradiction_pool_ignores_the_type_weight`. Con la excepción (i), los contratos `agent_contract.json` y `agent_contract.es.json` cambian y se bendicen de nuevo; con la (iii), ninguno cambia.

### RS14. (Paso 3, si abre la puerta 2) Puerta de abstención

Sólo si C1 (umbral sobre el `best_cosine`) o C2 (hueco 1º–10º) separan, en calibración, las consultas con respuesta de las sin respuesta con la cota de Wilson de U3, la búsqueda MUST abstenerse (mostrar 0 hits y decir la frase `no relevant memories for this query: …`) cuando el criterio se dispara, en el daemon y en el núcleo del contexto. Aceptación **U3**: abstiene bien al menos 16 de 20 sin respuesta (cota inferior de Wilson publicada) sin que `found` de las con respuesta baje más de 2 casos en prueba; **U2** y **U4** como en RS13. La puerta MAY bajar `MIN_SHOWN_HITS` a 0 sólo en la consulta en que se dispara; la constante sigue en 2. Su umbral MUST fijarse con un test que nombra el contrato de embeddings con que se calibró. Un daemon anterior se lee como hoy.

#### Escenario: todo bajo el umbral
- GIVEN una lista cuyo mejor coseno crudo está bajo el umbral
- WHEN se decide qué mostrar
- THEN se muestran 0 hits (hoy 2 por `MIN_SHOWN_HITS`)

#### Escenario: uno sobre y otro bajo
- GIVEN un hit sobre el umbral y otro bajo
- WHEN se decide
- THEN se conserva lo de hoy

#### Escenario: contexto
- GIVEN un `recall_context` con todos los hits bajo el umbral
- THEN dice la frase de abstención en vez de listarlos

**Tests:** `relevance.rs` (o la función pura que lo reemplace): `a_list_under_the_threshold_shows_nothing`, `a_list_with_one_hit_over_keeps_todays_behaviour`; `recall_context_abstains_when_every_hit_is_under_the_threshold`; `the_abstention_threshold_names_its_embedding_contract`.

## Invariantes

- El paso 1 no cambia el orden de producción ni el esquema; su código vive tras la feature `eval`.
- Lo que decide un caso (validación, resolución del proyecto, pool corto, agregados) es lógica pura, separada de la E/S.
- Ningún test del paso 1 necesita el modelo real; la medición con el modelo real necesita los dos snapshots, el ancla y el set minado en `docs/eval/`.
