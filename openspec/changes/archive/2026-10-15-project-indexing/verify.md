# project-indexing — Verificación

**Código:** rama `claude/project-thread-851hmw`, un commit por tarea.
**Modo:** Strict TDD. Cada test nuevo se vio fallar antes de su código (por no compilar o por la aserción), salvo `a_code_block_save_is_chunked_like_any_other`, que pasa desde el principio porque protege un borrado.
**Estado:** verificado en Linux y en Windows; R1 a R15 pasan. Archivado el 2026-10-15 con dos criterios de salida que no se cumplen y que Marcos aceptó: la búsqueda en docs no llega al umbral de la propuesta (Hit@3 0,27 contra 0,70) y el ancla de memorias baja con el pooling mean (R@5 0,7826 → 0,6522, toda en las consultas en inglés). La verificación en Windows encontró que `RELEVANCE_RATIO` cortaba de más con mean y lo recalibró de 0,6 a 0,35 (ver «Windows»). Los otros umbrales de R15 quedaron medidos sobre el golden set y no cambian.

## Suite

`cargo test --workspace --no-fail-fast` en Linux, por commit:

| Commit | Pasan | Fallan |
|---|---|---|
| base (`35e448c`) | 1639 | 10 |
| propuesta | 1639 | 10 |
| quitar el chunker de código | 1629 | 10 |
| tablas (V11 entonces, V12 tras mergear main) | 1630 | 10 |
| corpus | 1637 | 10 |
| troceo | 1648 | 10 |
| HEAD antes de mergear main | 1716 | 10 |
| merge de main (long-save-context, update_memory) | 1798 | 10 |

Las 10 fallas son las mismas en todos y vienen del entorno, no del código: los 7 `doctor::tests` (dan por hecho rutas de Windows) y los 3 `diff::tests` de `rag-eval` (leen `docs/eval/`, que no está en este clon). El conteo baja 10 con los tests de `chunker/code.rs` y sube 87 con los nuevos; el merge de main suma los suyos. Al mergear, `V11` ya era `memory_piece`, así que esta migración pasó a `V12__project_docs.sql`.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. El chunker de código se va | `a_code_block_save_is_chunked_like_any_other`; el build sin `#[allow(dead_code)]` ni `tree-sitter*`. `ChunkerKind::Code` se fue; `select_chunker` sigue (`chunker/mod.rs:95`) y elige entre `Chat` y `Skip` (ver «Lo que encontró el archive») | ✓ |
| R2. Corpus | `classify_takes_markdown_in_any_case`, `classify_takes_manifests_by_exact_name`, `classify_leaves_out_everything_else`, `the_walk_honours_gitignore_and_keeps_hidden_docs`, `oversized_and_binary_files_are_skipped`, `a_root_that_is_not_a_directory_is_an_error` | ✓ |
| R3. Troceo de Markdown | `sections_carry_their_breadcrumb`, `a_heading_inside_a_fence_does_not_split`, `a_fence_closes_only_with_its_own_character_and_length`, `an_empty_section_gives_no_passage_but_names_its_children`, `closing_hashes_are_not_part_of_the_heading`, `a_heading_indented_four_spaces_is_text`, `a_long_section_is_split_with_overlap_under_the_cap`, `a_long_breadcrumb_keeps_its_nearest_headings`, `an_empty_doc_has_no_passages` | ✓ |
| R4. Manifiestos | `a_manifest_is_one_passage_named_by_its_path` | ✓ |
| R5. Refresco | `plan_*`, `an_unchanged_project_embeds_nothing`, `a_changed_doc_replaces_its_passages_and_a_deleted_one_loses_them`, `another_model_reindexes_every_doc`, `a_missing_root_deletes_nothing`, `a_failed_embedding_keeps_the_doc_as_it_was`, `a_second_refresh_does_not_wait_for_the_first`, `refreshing_never_touches_memories` | ✓ |
| R6. Secretos | `a_secret_in_a_doc_never_reaches_the_index` | ✓ |
| R7. Búsqueda | `fuse_*`, `search_finds_a_passage_by_meaning`, `search_finds_an_identifier_lexically`, `search_never_crosses_projects`, `waiting_for_a_running_refresh_ends_when_it_does`; en `tests/api.rs` los `docs_search_*`, `a_root_that_vanished_still_answers_from_its_index` y `a_search_during_the_first_index_waits_for_it` | ✓ |
| R8. El registro indexa en segundo plano | `registering_a_root_indexes_its_docs_in_the_background` | ✓ |
| R9. `search_project` | `search_project_lists_each_hit_under_its_breadcrumb`, `instructions_say_when_to_search_the_project`, `docs_are_numbered_under_their_breadcrumbs`, `search_project_warns_while_indexing`, `an_empty_index_and_no_match_read_differently`, `a_failed_refresh_is_named_before_the_results`; los de handshake cuentan 9 herramientas | ✓ |
| R10. Indexar no cambia las memorias | `indexing_docs_leaves_memory_search_unchanged` | ✓ |
| R11. Datos de V11 | `a_database_from_the_previous_version_opens_with_an_empty_doc_index`, `published_migration_checksums_are_pinned` | ✓ |
| R12. `rag-eval --docs` | los de `docs.rs` (incluye `the_report_scores_docs_and_code_apart`), los `docs_*` de `cli.rs`, `a_doc_index_answers_from_the_root_it_indexed`, y `tests/docs_mode.rs` | ✓ |
| R13. Código con cAST | los 7 de `project_docs/code.rs`, `classify_takes_code_by_extension`, `a_refresh_indexes_code_by_definition` | ✓ |
| R14. Pooling mean y re-embebido | `the_e5_model_is_pooled_by_mean`, `the_model_id_names_its_pooling_apart_from_the_cls_vectors_of_before`, `only_the_cls_vectors_of_the_same_model_are_a_pooling_upgrade`, `cls_vectors_are_snapshotted_then_reembedded_by_mean`, `any_other_mismatch_is_left_as_it_is`, `a_store_that_matches_is_not_touched`, `snapshot_names_sort_in_time_order` | ✓ |
| R15. Umbrales provisionales | `scoring_config_default_pins_the_shipped_ranking_literals`, `contradiction_threshold_pins_the_shipped_value`, `near_duplicate_floor_pins_the_shipped_value`, `threshold_curve_*` | ✓ (valores provisionales) |

## `rag-eval --real --docs` (Linux, binario debug)

e5-small fp32 sobre este repo (199 archivos), 10 preguntas de docs y 5 de código:

| Variante | Pasajes | Indexado | Hit@1 | Hit@3 | Hit@5 | MRR@10 |
|---|---|---|---|---|---|---|
| Sólo docs, CLS | 1393 | 184 s | 0,10 | 0,20 | 0,20 | 0,19 |
| Docs + código, CLS: total | 4312 | 657 s | 0,20 | 0,20 | 0,27 | 0,23 |
| — docs | | | 0,10 | 0,10 | 0,20 | 0,15 |
| — código | | | 0,40 | 0,40 | 0,40 | 0,40 |
| **Docs + código, mean: total** | 4351 | 783 s | 0,20 | 0,27 | 0,27 | 0,26 |
| — docs | | | 0,20 | 0,20 | 0,20 | 0,24 |
| — código | | | 0,20 | 0,40 | 0,40 | 0,30 |

Ninguna variante llega a Hit@3 ≥ 0,70 ni MRR ≥ 0,50. El pooling mean mejora docs y empeora el primer puesto en código (c04 sigue primero; c02 baja a 2).

Antes de abrir el PR se probaron otros modelos sobre las 10 preguntas de docs, con un experimento fuera del repo sobre el mismo índice:

| Variante | Hit@3 | MRR@10 | Costo |
|---|---|---|---|
| e5-small CLS, fusión (lo de arriba) | 0,20 | 0,16 | |
| e5-small mean, sólo vectores | 0,30 | 0,29 | igual |
| e5-small mean + canal léxico sólo con tokens de 4 letras o más | 0,40 | 0,29 | igual |
| e5-base int8 mean + léxico ≥ 4 | 0,40 | 0,40 | ~2× |
| e5-large int8 mean, sólo vectores | 0,40 | 0,43 | 745 s para embeber |
| bge-reranker-v2-m3 sobre los 20 primeros | 0,50 | 0,28 | ~10 s por pregunta |

El canal léxico de hoy junta todos los tokens con OR, palabras vacías incluidas, y eso empuja pasajes que sólo comparten "de", "el" o "the". Filtrarlo es el siguiente paso barato; no está en este cambio.

## Golden set de docs

Cada pregunta evita las palabras del pasaje que la responde. Cuando el mismo contenido vive en dos archivos (`AGENTS.md`, `.claude/CLAUDE.md` y `.gemini/GEMINI.md`; una spec y su copia archivada), cualquiera cuenta.

| Id | Pregunta | Pasaje esperado | Qué dice el pasaje |
|---|---|---|---|
| d01 | el build falla porque en la máquina conviven dos toolchains del lenguaje, ¿cómo hago que use el compilador correcto? | `README.md` › Si tenés dos Rust instalados (o › Si algo falla) | fijar `RUSTC` con `rustup which rustc` y compilar con el toolchain MSVC |
| d02 | how do I remove the program from my computer, and does that also wipe what it remembered? | `README.md` › Desinstalar | `clawg stop`, `clawg path remove`, `claude mcp remove`; las memorias quedan en `~/.clawg` |
| d03 | el cliente pasó a presentarse con otro nombre y desaparecieron los recuerdos de antes | `README.md` › Fijar la identidad del agente | fijar `--agent` al registrar el servidor MCP |
| d04 | ¿con qué comandos reviso lo que almacenaron los agentes, sin alterar nada? | `README.md` › Ver lo que se guarda | el visor de sólo lectura `clawg memories stats/list/search/show` |
| d05 | which folder and port does each environment use, and can a debug binary reach the production data? | `AGENTS.md`, `.claude/CLAUDE.md` o `.gemini/GEMINI.md` › Data profiles, o `README.md` › Perfiles y datos | `~/.clawg-dev` en 27381, `~/.clawg` en 27380; un build debug rechaza `prod` |
| d06 | ¿en qué orden se redactan los documentos de una mejora antes de programarla? | `openspec/README.md` › Cómo se trabaja un cambio | exploration → proposal → spec → tasks → apply → verify → archive |
| d07 | two checkouts of one repository in separate folders made with git worktree: do they count as the same project? | `openspec/specs/project/spec.md` (o su copia archivada) › R1. Resolución de la raíz | un worktree comparte la raíz del repo principal por `commondir` |
| d08 | ¿qué contesta el servidor cuando le llegan más guardados de los que puede aceptar en ese momento? | `openspec/specs/ingest/spec.md` › Cola acotada | 503 `ingest_queue_full` con la cola de 1024 llena |
| d09 | si cierro el servidor mientras todavía procesa lo recibido, ¿se pierde algo? | `openspec/specs/storage/spec.md` › Ningún evento aceptado se pierde en un apagado ordenado | el apagado drena la cola o la vuelca a disco |
| d10 | after updating, my editor can no longer talk to the memory server; what should I do? | `README.md` › Si algo falla | el puente no se reconecta solo: `/mcp` en Claude Code |

Preguntas de código (cuenta la ruta, sin encabezado):

| Id | Pregunta | Archivo esperado |
|---|---|---|
| c01 | ¿dónde se elige el entorno de datos a partir de la línea de comandos, una variable y si el binario es de depuración? | `crates/clawg-core/src/paths.rs` |
| c02 | where are API keys and passwords hidden before text is stored? | `memory/redact.rs` o `memory/secrets.rs` |
| c03 | ¿qué guarda en disco lo que no alcanzó a procesarse al apagar, para retomarlo en el próximo arranque? | `memory/spill.rs` |
| c04 | how does it tell that a linked checkout shares the folder of its parent repository? | `memory/identity.rs` |
| c05 | ¿qué código sube o baja la temperatura de un recuerdo según cuánto se usa? | `memory/tier.rs` o `memory/access.rs` |

## Lo que cambió durante la implementación

- **El código entra al índice (R13).** Marcos eligió "Docs + código cAST": el chunker muerto se borró igual (R1) y el troceo de código se escribió de cero con cAST.
- **Pooling mean (R14).** Midiendo, apareció que e5-small se cargaba con pooling CLS. Se corrigió para todo el store, con re-embebido al arrancar después de un snapshot `pre-reembed`. El seam de verificación del bootstrap pasa a recibir el embedder.
- **Umbrales provisionales (R15).** Con el piso de coseno viejo, el 91 % de los top-10 con mean quedaba debajo. Se movieron con un proxy (los docs de este repo y 20 pares escritos a mano) porque el golden set de memorias no está en este entorno. Los fixtures de contradicción se movieron a L2 ≈ 0,26 para quedar dentro de la banda nueva.

- **La búsqueda espera el primer índice (R7).** La primera prueba de `search_project` contestaba "indexando, sin resultados": el registro del puente lanza el refresco de fondo un instante antes de la búsqueda, y la búsqueda encontraba el refresco en curso y el índice vacío. Ahora, si hay un refresco en curso y el proyecto no tiene ningún doc indexado, la búsqueda lo espera hasta 20 s. Con un índice ya armado no espera: contesta con lo que hay y `indexing: true`.
- **`is_none_or` no se usa:** la MSRV del workspace es 1.77.2 y ese método es de 1.82; se usa `map_or(true, …)`.
- **`rag-eval --docs` imprime y no escribe archivo de resultados.** Los umbrales se comparan contra el número impreso; no hay baseline de docs con el que hacer `--diff` todavía.

## Windows (2026-10-15)

Sobre `main` `fc0606d`, que tiene project-indexing y agent-sharing mergeados (respecto de `9bbc78f` sólo cambia la versión), con rustc 1.99.0 `x86_64-pc-windows-msvc`. Binarios debug en `--target-dir target-test`.

### Suite

`cargo test --workspace --no-fail-fast --target-dir target-test`: **1.863 pasan, 0 fallan**, en 13 binarios. Las 10 fallas de Linux pasan acá. El único warning es el de `insert_row_with_embedding`.

Con `RELEVANCE_RATIO` en 0,35 (abajo), la corrida completa dio 1.863 y 1 falla: `relevant_hits_keeps_the_ones_the_cut_lets_through_in_order` (`mcp/tools.rs`), cuyo fixture estaba en la escala de CLS. Pasado a la de mean, `cargo test -p clawg-cli` da 393 y 0. Total: **1.864**, con el test nuevo `relevance_ratio_pins_the_shipped_value`.

### `rag-eval --real` sobre el golden set de memorias

`docs/eval/golden.json` (sha256 `d2d9d732…`, 46 casos, 28 pares de contradicción, corpus de 48 filas: 26 en español y 22 en inglés). La base es la última corrida con CLS: `2026-10-15-real-long-save-context-tip-anchor.json` y `…-agent-view.json` (piso 0,8793). Resultados en `docs/eval/results/2026-10-15-real-project-indexing-*`.

| Ancla | R@1 | R@5 | R@10 | MRR | nDCG@10 |
|---|---|---|---|---|---|
| CLS, piso 0,8793 | 0,4348 | 0,7826 | 0,8913 | 0,5979 | 0,6693 |
| **Mean, piso 0,79** | 0,4348 | 0,6522 | 0,8478 | 0,5524 | 0,6215 |
| — en español (24), CLS → mean | 0,5833 → 0,5833 | 0,8750 → 0,8333 | 0,8750 → 0,9583 | 0,7153 → 0,7167 | 0,7564 → 0,7745 |
| — en inglés (22), CLS → mean | 0,2727 → 0,2727 | 0,6818 → 0,4545 | 0,9091 → 0,7273 | 0,4698 → 0,3733 | 0,5743 → 0,4545 |

`rag-eval --diff` contra la base: R@5 1 mejora y 7 empeoran (neto −6), MRR 10 y 13 (neto −3). Empeoran c02, c03, c07, c09, c11, c12, c16, c18, c25, c26, c29, c35 y c41, nueve de ellos en inglés. Mejoran c08, c10, c17, c28, c36, c38, c42, c43, c44 y c45. Español queda igual o mejor en todo salvo R@5; la baja es del inglés.

**Piso de coseno** (`--sweep-cos-floor`):

| Piso | R@1 | R@5 | R@10 | MRR | nDCG@10 |
|---|---|---|---|---|---|
| 0,70 | 0,4130 | 0,6739 | 0,8043 | 0,5294 | 0,5942 |
| 0,73 | 0,4130 | 0,6739 | 0,8043 | 0,5326 | 0,5968 |
| 0,75 | 0,4130 | 0,6739 | 0,8043 | 0,5308 | 0,5954 |
| 0,77 | 0,4130 | 0,6739 | 0,8043 | 0,5362 | 0,5998 |
| **0,79** | 0,4348 | 0,6522 | 0,8478 | 0,5524 | 0,6215 |
| 0,81 | 0,4348 | 0,7174 | 0,8043 | 0,5489 | 0,6099 |
| 0,83 | 0,3913 | 0,6087 | 0,6957 | 0,4738 | 0,5263 |

0,79 tiene el mejor MRR, nDCG@10 y R@10, y queda. Ningún piso acerca el ancla a la de CLS: la baja es del pooling, no del umbral.

**Contradicciones** (curva del reporte, 28 pares):

| L2 | recall | fpr no contradicción | fpr idénticos |
|---|---|---|---|
| 0,30 | 0,20 | 0 | 0,50 |
| 0,35 | 0,40 | 0 | 0,75 |
| 0,40 | 0,80 | 0 | 1,00 |
| 0,45 | 0,90 | 0 | 1,00 |
| 0,50 | 1,00 | 0 | 1,00 |

Ningún par compatible del golden set queda bajo 0,50, y 0,42 cae entre recall 0,8 y 0,9. Los idénticos no llegan al detector: los ataja antes el hash de contenido. Subir el umbral gana recall, pero se come el margen que pide el par compatible escrito a mano a 0,469. `CONTRADICTION_THRESHOLD` queda en 0,42. `NEAR_DUPLICATE_FLOOR` queda en 0,10 con la medición del proxy; el golden set no tiene pares a esa distancia.

**Audit** (`--audit`): `USABLE`. Saturación R@1 0,4348 y R@10 0,8478 (límite 0,90), acuerdo del rango modal 0,1283 (límite 0,80), solapamiento léxico medio 0,0402 (límite 0,35). El intruso más frecuente en el top 5 es c05, en 24 de 46 casos.

**Canal léxico** (`--sweep-lexical` 0, 5, 10 y 20): las cuatro corridas dan lo mismo que el ancla. Este set sigue sin poder medirlo.

### `RELEVANCE_RATIO`: de 0,6 a 0,35

Agent-view con 0,6 y mean: `search_memory` encuentra 30/46 (media 454,7 tokens), contra 41/46 (851,5) con CLS, y `recall_context` 39/46 contra 41/46. El ranking no explica esa baja de `search_memory`, porque R@10 sólo pasa de 0,89 a 0,85. Lo que la explica es el corte relativo: con mean, la mediana del mejor score pasa de 0,335 a 0,19, y `search_memory` muestra 4,5 hits por consulta en vez de 8,5. Con 0,6, 16 de las 46 consultas mostraban sólo 2. c44, por ejemplo, sube en el ranking (de 9.º a 8.º) y aun así desaparece de la salida.

`rag-eval --diff` sobre agent-view no lo ve, porque compara sólo `retrieval.per_case` y `similarity.per_case`, que son iguales en las dos corridas. Se comparó a mano.

Barrido recompilando `rag-eval` con cada valor en un worktree descartable (`--tag relsweep-<r>`):

| Ratio | `search_memory` | Tokens media / máx. |
|---|---|---|
| 0,60 | 30/46 | 454,7 / 1.081 |
| 0,50 | 33/46 | 573,4 / 1.123 |
| 0,45 | 33/46 | 634,5 / 1.206 |
| 0,40 | 34/46 | 700,9 / 1.206 |
| **0,35** | **36/46** | **764,0 / 1.206** |
| 0,30 | 36/46 | 815,7 / 1.285 |
| 0,25 | 36/46 | 857,6 / 1.285 |
| 0,20 | 37/46 | 894,4 / 1.285 |

`recall_context` da 39/46 y 1.086,4 tokens con cualquier ratio. 0,35 es el ratio más alto del plateau de 36 y cuesta menos tokens que CLS. Bajar a 0,2 gana un solo caso y pasa ese costo. Con mean, el score del esperado (0,03–0,41) y el del resto (hasta 0,38) se superponen, así que el corte sigue siendo relativo.

Con el binario de la constante nueva (`--tag project-indexing-relcut`): `search_memory` 36/46 con 764,0 / 1.206 tokens, `recall_context` 39/46 con 1.086,4 / 1.408, y el ancla idéntica a la de arriba. Vuelven c07, c12, c16, c40 y c44. c38, que con CLS no aparecía, ahora aparece. c02, c09, c11, c18, c26 y c41 no vuelven con ningún ratio, ni con 0,2.

### `rag-eval --real --docs`

226 docs y 4.984 pasajes, 0 salteados, indexados en 645 s (binario debug).

| | Hit@1 | Hit@3 | Hit@5 | MRR@10 |
|---|---|---|---|---|
| Total (15) | 0,27 | 0,27 | 0,33 | 0,29 |
| Docs (10) | 0,20 | 0,20 | 0,30 | 0,23 |
| Código (5) | 0,40 | 0,40 | 0,40 | 0,40 |

Aciertan primeras d01, d03, c02 y c04. d02 sale 8.ª y d05 5.ª, y las otras 9 no aparecen. Lejos de Hit@3 ≥ 0,70 y MRR ≥ 0,50.

**El corpus se contamina con este archivo.** La tabla «Golden set de docs» de arriba cita las preguntas textuales, y el pasaje `openspec/changes/project-indexing/verify.md > … > Golden set de docs` sale primero en 7 de las 11 fallas (d02, d05, d07, d10, c01, c03 y c05). Archivado, el archivo sigue en el corpus. En Linux no pasaba porque la tabla se escribió después de medir. Aun sin la contaminación, ninguna variante medida llegó al umbral.

### Perfil dev y Claude Code

Con la evidencia que ya había, decidido con Marcos. No se tocó `~/.clawg-dev`.

- **Re-embebido al arrancar.** El 2026-10-15 a las 01:38 UTC, la prueba de agent-sharing levantó un daemon de `9bbc78f` sobre el store dev, que estaba en V11 con vectores CLS. Dejó `memory-2026-10-15T01-38-17.341Z-pre-migration.db` y `memory-2026-10-15T01-38-30.827Z-pre-reembed.db`, migró a V12, re-embebió por media y quedó `ready` a los ~15 s ([verify de agent-sharing](../2026-10-14-agent-sharing/verify.md#perfil-dev), paso 2). La instalación de 0.3.0 en prod repitió lo mismo: `memory-2026-10-15T02-28-17.864Z-pre-migration.db` y `…02-28-19.943Z-pre-reembed.db`, con `memory_status` en `ready`. `loading` no se observó en vivo. Lo publica `run_embedder_bootstrap` antes de verificar el contrato (`bootstrap.rs:339`).
- **`search_project` desde Claude Code.** En esta sesión (perfil prod, 0.3.0, agente `claude-code`), la pregunta «¿qué pasa al arrancar si el store tiene vectores CLS del mismo modelo?» trajo `bootstrap.rs > mod tests` (`cls_vectors_are_snapshotted_then_reembedded_by_mean`) y `README.md > Claw Garden > Si algo falla`, el pasaje sobre `loading` después de actualizar. Todavía con el aviso de índice en curso, porque el daemon tenía 6 minutos. Editar un doc cambia el resultado: minutos después de escribir la sección «Windows» de este archivo, «¿a qué valor se recalibró RELEVANCE_RATIO con pooling mean y por qué ese valor?» la trajo 3.ª, y la línea nueva de `specs/retrieval` salió 1.ª.

## Lo que encontró el archive

Al consolidar en `specs/project-docs` (y en `ingest`, `retrieval`, `eval`, `storage`, `disputes`, `mcp` y `project`), la spec del cambio y el código no coincidían en esto. Las specs principales dicen lo que hace el código:

- **R1:** `select_chunker` no se fue. Sigue en `chunker/mod.rs:95` y elige entre `Chat` y `Skip` (contenido vacío o rol System). Lo que se fue es `ChunkerKind::Code`.
- **R2:** un archivo con UTF-8 inválido no se descarta en el recorrido. Se descarta al leerlo en el refresco, donde suma a `skipped` (`project_docs/mod.rs:330-334` y `:374`).
- **R7:** `limit` se acota a 1–20, así que 0 vale 1 (`api.rs:1080`).
- **R9:** el mensaje de índice vacío sólo nombra Markdown y manifiestos, no el código.
- **R10:** los tests cubren `search_all_memories` y las tablas de memoria. `/v1/search`, `/v1/context`, `search_memory` y `recall_context` no tienen test propio de esto.
- **R12:** con `--real`, el modelo se carga del directorio de datos que resuelve `production_base_dir` (`rag-eval/main.rs:68`), el mismo caché del resto de `rag-eval --real`. El store sí es temporal. El detalle por pregunta se imprime cada vez que no acierta primera, no sólo en las fallas.
- **R13:** un pedazo que junta varios nodos lleva las etiquetas de su nodo más grande. Una hoja que no entra va sola, y la parte el tope de tokens sólo si lo pasa. Un pedazo sin ningún carácter alfanumérico, como `}`, no da pasaje (`code.rs`).
- **R14:** si `reembed_all` no termina limpio, el estado es `Mismatch` y ese arranque no reintenta (el próximo sí). Si falla el snapshot, es un fallo de verificación y se reintenta.

Huecos que quedan para otro cambio:

- **La búsqueda en docs no llega al umbral.** Lo siguiente barato está medido arriba: filtrar las palabras vacías del canal léxico (Hit@3 0,40 en el experimento) y sacar de `rag-eval --docs` el pasaje que cita el golden set. Después vienen e5-base o un reranker.
- **El pooling mean bajó la recuperación en inglés.** En inglés, R@5 pasa de 0,68 a 0,45; en español queda igual. El corpus no es cross-lingual (22 filas en inglés), así que hay que explicarlo antes de tocar nada.
- **`rag-eval --diff` no compara agent-view.** Sólo mira `per_case` de retrieval y similarity.
