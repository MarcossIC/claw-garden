# revision-eval — Especificación

Requisitos nuevos sobre [`specs/eval`](../../specs/eval/spec.md), que gana una sección «`revision-eval`: revisiones por clave». Lo que no se nombra aquí sigue como está. Términos: la **clave** de un save es su `topic_key`; una **cadena** es una lista ordenada de versiones de un mismo tema; una **transición** es el paso de una versión a la siguiente (una cadena de n versiones tiene n-1); el **brazo sin clave**, el **brazo con clave** y el **control** son los tres brazos de `chains` (el control guarda sólo la última versión de cada cadena). Los tests nuevos viven en `crates/clawg-eval` (`src/bin/revision-eval/` y `tests/`) y en `crates/clawg-core/src/eval_api.rs`; los nombres son orientativos y los fija el diseño.

## E1. El set de cadenas

`crates/clawg-eval/data/revision-chains.json` MUST ser un objeto `{"version": 1, "chains": [...]}`. Cada cadena lleva `id`, `lang` (`es` o `en`), `memory_type`, `topic_key`, `query` y `versions` (de 2 o 3), cada una con `content`, `value` (el texto que distingue a esa versión de las otras de su cadena) y `change`: `value`, `rewrite` o `extend` (la primera versión no lleva `change`). `topic_key` MUST cumplir el formato de [`specs/ingest`](../../specs/ingest/spec.md); el parseo usa `deny_unknown_fields`, como el golden (`crates/clawg-eval/src/bin/rag-eval/golden.rs:39`).

El parseo MUST rechazar un set que no llegue a estos mínimos, con un error que nombra lo que falta y cuánto (como `Class::minimum`, `crates/clawg-eval/src/bin/conflict-eval/measure.rs:41`):

- 40 cadenas: 20 `es` y 20 `en`;
- 24 de dos versiones y 16 de tres (56 transiciones);
- 20 `architectural_decision`, 10 `user_preference` y 10 `semantic_fact`;
- al menos 8 cadenas con alguna versión de más de 480 tokens (se parten en pedazos);
- una `query` que no contiene el `value` de ninguna versión de su cadena: el set declara ese campo para que el parseo lo compruebe sin interpretar la prosa.

#### Escenario: el set entregado
- GIVEN `revision-chains.json` del repositorio
- WHEN se parsea
- THEN pasa todos los mínimos y la corrida registra su `version` y su sha256

#### Escenario: un set corto
- GIVEN un set con 39 cadenas, o con 7 largas, o sin ninguna `user_preference`
- WHEN se parsea
- THEN falla y el mensaje nombra la clase que falta (`chains`, `long chains`, `user_preference`)

#### Escenario: la consulta nombra el valor
- GIVEN una cadena cuya `query` contiene el valor de su versión 2
- THEN el parseo falla y nombra la cadena por `id`

#### Escenario: clave mal escrita
- GIVEN una cadena con `topic_key` inválida o con una clave de campo desconocida
- THEN el parseo falla

**Tests** (`revision-eval`, parseo): `the_shipped_chain_set_meets_every_minimum`, `a_set_below_a_minimum_names_what_is_missing`, `a_query_that_holds_a_versions_value_is_refused`, `a_chain_with_an_invalid_key_or_unknown_field_fails`. Fallan hoy: el binario y el set no existen.

## E2. `chains`: tres brazos y clasificación

```
revision-eval chains (--fake | --real) [--tag <slug>]
```

`chains` MUST ingerir por `EvalIngest` (E5) tres brazos sobre stores propios en el directorio temporal del sistema, con los mismos distractores en los tres: las oraciones con MUST de `openspec/specs`, guardadas antes como hechos. En el brazo sin clave cada versión se guarda sin clave; en el brazo con clave, con la clave de su cadena (mismo agente y proyecto en toda la cadena); en el control, sólo la última versión de cada cadena. MUST NOT abrir ningún store de un perfil para escribir.

En el brazo sin clave, cada transición MUST quedar en exactamente una clase, mirando el save nuevo contra la versión anterior:

| Clase | Condición |
|---|---|
| `escalated` | hay un par `memory_conflict` escalado entre ambas |
| `pending` | hay un par `pending` entre ambas |
| `no_mark` | no hay par; y se anota por qué: `below_floor` (distancia < `NEAR_DUPLICATE_FLOOR`, `crates/clawg-core/src/memory/contradiction.rs:45`), `above_threshold` (>= `CONTRADICTION_THRESHOLD`, `:37`), `out_of_pool` (dentro de la banda pero fuera de los `CANDIDATE_POOL` más cercanos, `:58`) o `not_claim_bearing` (el tipo no afirma, `store_insert.rs:59`) |

El par se lee del store; las distancias, entre todos los pedazos de las dos versiones. La herramienta MUST NOT recalcular el pool del detector: `out_of_pool` se deduce por descarte (dentro de la banda y sin par).

Las cuatro razones de `no_mark` MUST sumar el total de `no_mark`, y las tres clases, el total de transiciones (56 en el set entregado). El reporte da, por clase, conteo, tasa e intervalo de Wilson, y las tasas también por `change` y por `lang`.

#### Escenario: una transición lejos
- GIVEN dos versiones cuya distancia supera `CONTRADICTION_THRESHOLD`
- WHEN se clasifica la transición
- THEN es `no_mark` con razón `above_threshold`

#### Escenario: par escalado
- GIVEN dos `architectural_decision` más cerca que `ESCALATE_BELOW`
- THEN la transición es `escalated`

#### Escenario: fuera del pool
- GIVEN una versión vieja dentro de la banda con seis filas más cercanas al save nuevo
- THEN la transición es `no_mark` con razón `out_of_pool`

**Tests:** `every_transition_falls_in_exactly_one_class`, `a_far_pair_is_no_mark_above_the_threshold`, `a_close_pair_of_decisions_is_escalated`, `a_row_beyond_the_candidate_pool_is_out_of_pool`, `a_type_that_asserts_nothing_is_not_claim_bearing`, `the_control_arm_stores_only_the_last_version` (lógica pura, más una corrida `--fake`, E6).

## E3. `chains`: M1 y M2 sobre la salida del agente

Para cada cadena y cada brazo, `chains` MUST hacer la `query` por el router en proceso y registrar, tal como el puente corta el resultado (`relevant_hits`, `crates/clawg-cli/src/mcp/tools.rs:65`; `format_search`, `:79`; y el contexto de `recall_context`): la posición de cada versión en `search_memory` y en `recall_context`, si alguna versión vieja aparece, y si aparece arriba de la nueva. Una versión ausente tiene posición `none`.

**M2 pasa** cuando, en el brazo con clave y en las 40 cadenas: (a) hay una sola versión viva por clave; (b) ninguna versión vieja aparece en `search_memory` ni en `recall_context`; (c) la última queda en la misma posición que en el control en al menos 38 de las 40, sin bajar más de un puesto en ninguna. El reporte da el veredicto y, si falla, la lista de cadenas que lo rompen. Un M2 que no pasa MUST NOT arreglarse acá.

#### Escenario: M2 pasa
- GIVEN un brazo con clave donde cada cadena tiene una viva, ninguna vieja visible y la última en el puesto del control
- THEN el veredicto es `pass`

#### Escenario: una vieja visible
- GIVEN una cadena cuya versión 1 aparece en `recall_context` con clave
- THEN el veredicto es `fail` y nombra la cadena

#### Escenario: baja dos puestos
- GIVEN una cadena donde la última queda dos puestos más abajo que en el control
- THEN el veredicto es `fail` aunque las otras 39 coincidan

#### Escenario: 37 de 40
- GIVEN 37 cadenas con la última en el puesto del control y ninguna baja más de un puesto
- THEN el veredicto es `fail`

**Tests:** `m2_passes_when_all_three_conditions_hold`, `m2_fails_when_an_old_version_shows`, `m2_fails_when_the_last_drops_two_places`, `m2_needs_thirty_eight_matching_chains`, `an_old_version_above_the_new_one_is_recorded`.

## E4. `chains`: M4, el costo de mostrar la clave

Con el tokenizador de las líneas de base (`EvalTokenCounter::real`, `crates/clawg-core/src/eval_api.rs:659`) y sobre las consultas del brazo con clave, `chains` MUST informar:

1. los tokens de la respuesta de `search_memory` con ` topic=K` y sin ella, y la diferencia media y total;
2. los tokens y los bytes del prompt de `recall_context` reescrito con ` topic=K` en la cabecera de cada hit con clave, contra el mismo prompt sin reescribir, y cuántos prompts pasan de su `budget` (8.000 bytes por defecto); y lo mismo como cota de arriba para «todas las filas con clave», con la clave más larga del set en cada hit sin clave;
3. el costo fijo de la clave: los tokens de su descripción en el schema de `save_memory` (`crates/clawg-cli/src/mcp/agent_contract.json:278-285`) y de su párrafo de `protocol.md` (`:69-74`);
4. los tokens de una línea de aviso de referencia, fijada antes de medir. Su costo medio por save del flujo lo da `store` (E7), que es el que conoce la tasa de disparo.

La reescritura del punto 2 MUST vivir en la herramienta de evaluación y MUST NOT cambiar lo que `recall_context` ni `format_hit_line` (`crates/clawg-core/src/memory/prompt_builder.rs:691`) devuelven en producción.

#### Escenario: filas con clave
- GIVEN un prompt con tres hits con clave
- WHEN se reescribe
- THEN cada cabecera lleva ` topic=K`, el resto del prompt queda igual y la diferencia en bytes es la suma de los ` topic=K`

#### Escenario: hit sin clave
- GIVEN un hit sin clave
- THEN su cabecera no cambia en la medición tal cual, y lleva la clave más larga del set en la cota de arriba

#### Escenario: el producto no cambia
- GIVEN `crates/clawg-cli/tests` del contexto
- WHEN corre la suite
- THEN el prompt de producción sigue sin mostrar la clave

**Tests:** `the_rewrite_adds_the_key_only_to_keyed_hits`, `the_byte_delta_is_the_sum_of_the_added_keys`, `a_prompt_over_budget_after_the_rewrite_is_counted`, `m4_reports_the_fixed_cost_of_the_key_from_the_contract_and_the_protocol`, `the_production_context_still_omits_the_key` (este último, sobre el prompt real, pasa hoy y MUST seguir pasando).

## E5. `eval_api`

- `EvalIngest::save` MUST guardar con el agente, el proyecto y la clave que recibe, y devolver lo que el worker informa del save (`IngestReport`, `crates/clawg-core/src/memory/worker.rs:81-89`): las filas con su id, las candidatas del detector (`related`, pendientes y escaladas) y las filas reemplazadas. Una clave inválida MUST fallar antes de encolar, como en `POST /v1/memories`.
- `EvalIngest::send` (`crates/clawg-core/src/eval_api.rs:525`) MUST NOT cambiar: ya recibe el agente (`:527`) y guarda sin proyecto ni clave (`:540`, `:549`).
- `eval_api` MUST exponer la unión de los pedazos de un save, igual que la de `read_memory` (`join_pieces`, `crates/clawg-core/src/memory/pieces.rs:61`, hoy `pub(crate)`), y las dos constantes de la banda del detector, que MUST ser las del detector y no copias. El estado, `superseded_by` y la clave de una fila se leen por el router en proceso, que ya los da; `supersede_cause`, del export (E7).
- Los llamadores existentes MUST conservar su comportamiento: `rag-eval --agent-view` siembra por `send` (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:90-93`), `rag-eval --real` por `EvalStore::insert` (`crates/clawg-eval/src/bin/rag-eval/seed.rs:63-79`), y `conflict-eval` por `send`.

#### Escenario: guardar con clave
- GIVEN dos `save` con la misma clave, agente y proyecto
- THEN la segunda informa como reemplazada a la primera, y el router la lista `superseded` con `superseded_by` apuntando a la segunda

#### Escenario: otro agente
- GIVEN el mismo `topic_key` con otro agente
- THEN no reemplaza nada

#### Escenario: los llamadores de hoy
- GIVEN `rag-eval --real` y `--agent-view` con el golden de `docs/eval/` y el modelo del perfil dev
- WHEN se comparan con `--diff` contra `docs/eval/results/2026-10-18-real-search-abstention.json` y `2026-10-20-real-untrusted-recall-fix-agent-view.json`
- THEN ningún caso se mueve y los tokens no cambian

**Tests** (`eval_api.rs`): `save_stores_under_the_agent_project_and_key_it_is_given`, `a_second_save_under_the_same_key_reports_the_first_as_replaced`, `save_reports_the_candidates_the_detector_paired`, `save_with_an_invalid_key_fails`, `joined_pieces_equal_the_text_read_memory_returns`, `the_band_constants_are_the_detectors`; y en `revision-eval`, `the_router_lists_a_revised_row_superseded_by_the_new_one`. Fallan hoy: `save` no existe. La comparación con las líneas de base es una verificación con el modelo real, fuera de la suite, y se asienta en `verify.md`.

## E6. Corrida `--fake`

`revision-eval chains --fake` y `store --export … --fake` MUST correr sin el modelo real, por `EvalIngest` y por el router, como `a_run_finds_a_stored_row_in_both_outputs_and_cleans_up_after_itself` (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:353`). Comprueban la tubería: que cada brazo se guarda, que el router devuelve sus filas, que la reescritura de E4 encuentra cada cabecera con clave de un prompt real del router y que el reporte sale entero. No pueden ejercer las clases de E2 ni los veredictos de E3, porque el embedder falso no empareja nada (`crates/clawg-eval/src/bin/conflict-eval/main.rs:290`): esos los cubren los tests puros. Las corridas `--real` leen el modelo del perfil dev (`data_dir_from_env`) y MUST NOT ser parte de la suite ni de CI.

**Tests:** `a_fake_chains_run_stores_every_arm_and_reports_whole`, `the_rewrite_finds_every_keyed_header_of_a_real_prompt`, `a_fake_store_run_counts_and_labels_a_small_export`.

## E7. `store` (M5)

```
revision-eval store --export <FILE> [--seed N] (--fake | --real)   # reproduce; deja las etiquetas por llenar
revision-eval store --labels <FILE>                                 # puntúa; borra el directorio de trabajo
```

`store` va en dos pasos, porque etiquetar lleva horas y reproducir otra vez para puntuar costaría minutos de modelo. El primero MUST leer un `clawg export --include-deleted` con `format::parse` (`crates/clawg-core/src/memory/portability/format.rs:176`) y unir los pedazos de cada save. Una versión de formato desconocida MUST fallar sin escribir.

**Sin reproducir**, MUST contar: saves de hecho, decisión o preferencia con clave desde la primera clave del store; filas `superseded` por `revision` y por `verdict` (`supersede_cause`); y pares juzgados `supersedes`.

**Reproduciendo**, MUST pasar cada save por `EvalIngest::save`, en orden de `(created_at, id)`, con su agente, su proyecto y su clave (nunca `import`, que no corre el detector, `crates/clawg-core/src/memory/portability/import.rs:3-5`), y contar en cuántos saves sin clave dispararía el aviso. El aviso **dispara** en un save sin clave, de un tipo que afirma, cuyas candidatas (`related`) nombran una fila de su mismo agente y proyecto: las únicas que una clave podría revisar (`crates/clawg-core/src/memory/topic.rs:83-86`). Una candidata de otro proyecto se cuenta aparte, y también un disparo cuya candidata ya estaba borrada o reemplazada por veredicto en el store real, como cota del sesgo de no reproducir borrados ni veredictos. Con la tasa de disparo, el reporte da el costo de la línea de aviso de E4: máximo donde aparece y media por save del flujo.

**Etiquetando**, MUST emitir, para cada save sin clave, el save anterior más cercano del mismo agente y proyecto, y tomar para etiquetar todos los saves en que dispara más una muestra con semilla fija de hasta 50 de los demás (la misma semilla da la misma muestra). La regla de la etiqueta es: el nuevo es una **revisión** si trata el mismo tema que el anterior con otro valor, alcance o motivo, de modo que el anterior ya no vale tal como está escrito; si no, no. El archivo de etiquetas MUST NOT decir cuál par disparó, y lleva por par la etiqueta que propone un agente (`proposed`) y la que decide el dueño del store (`label`). El segundo paso MUST rechazar un archivo con algún `label` vacío, MUST puntuar sólo con `label` y MUST informar el acuerdo entre `proposed` y `label`. Con las etiquetas leídas, el reporte da la tasa de revisión entre los sin clave de un tipo que afirma y entre los que disparan, con intervalo de Wilson, y las cotas de la fila «Aviso» de la tabla de umbrales.

#### Escenario: contar sin reproducir
- GIVEN un export con 3 saves bajo una clave, 2 filas `superseded` por `revision` y 1 por `verdict`
- THEN los conteos dan 3, 2 y 1

#### Escenario: dos agentes
- GIVEN dos saves sin clave de dos agentes en el mismo proyecto
- THEN el anterior más cercano de cada uno se busca sólo entre los de su agente

#### Escenario: la muestra
- GIVEN 200 saves sin clave, 12 en que dispara
- THEN se etiquetan 12 más 50, y con la misma semilla son los mismos

#### Escenario: export ilegible
- GIVEN un export con un formato desconocido
- THEN falla antes de escribir nada

#### Escenario: candidata de otro proyecto
- GIVEN un save sin clave cuya única candidata es una preferencia de otro proyecto
- THEN no cuenta como disparo y se cuenta aparte

**Tests:** `the_counts_that_need_no_replay_come_from_the_parsed_export`, `the_nearest_earlier_save_is_looked_up_within_agent_and_project`, `a_candidate_of_another_project_does_not_fire`, `every_firing_save_is_labelled_plus_a_seeded_sample`, `the_same_seed_picks_the_same_sample`, `the_labels_file_does_not_say_which_pair_fired`, `a_labels_file_with_an_empty_label_is_refused`, `the_score_reads_only_the_owners_label_and_reports_the_agreement`, `a_rate_carries_its_wilson_interval`, `an_unknown_export_format_writes_nothing`.

## E8. Privacidad de `store`

Nada que lleve texto guardado (el export, la lista de pares, las etiquetas, la muestra) MUST escribirse dentro del repositorio: va al directorio temporal del sistema y se borra al terminar, también si la corrida falla. El resultado de `store` en `docs/eval/results/` MUST llevar sólo conteos, tasas e intervalos de Wilson, sin `content`, ids de filas ni claves del usuario.

#### Escenario: lo que se escribe
- GIVEN una corrida `--fake` con un export que contiene un texto reconocible
- WHEN termina, con éxito o con error
- THEN el JSON de resultados no contiene ese texto y el directorio temporal fue borrado

**Tests:** `the_results_file_of_store_holds_no_stored_text`, `the_working_files_are_removed_even_when_the_run_fails`.

## E9. Resultados y veredictos

Cada corrida real MUST escribir un archivo fechado en `docs/eval/results/` con el contrato del modelo de embeddings, la `version` y el sha256 del set de cadenas (en `chains`) o el de formato y versión del export (en `store`, sin ruta ni nombre), y la semilla. `verify.md` MUST dar el veredicto contra la tabla de umbrales de la propuesta de **Aviso «pasá `topic_key`»** (M5, M4) y **mapeo de history-import** (M1, M2), y MUST dejar las otras tres decisiones (búsqueda por clave, clave en `recall_context`, quitar reglas) como pendientes de M3, con lo que M1, M2 y M4 ya dejan medido. Los umbrales MUST NOT ajustarse después de medir.

#### Escenario: el encabezado
- GIVEN un resultado de `chains`
- THEN lleva fecha, contrato del modelo, `version` y sha256 del set, y los tres brazos

**Tests:** `a_result_header_names_the_model_contract_and_the_set_hash`. Los veredictos de `verify.md` son verificación manual con el modelo real.
