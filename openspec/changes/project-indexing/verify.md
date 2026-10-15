# project-indexing — Verificación

**Código:** rama `claude/project-thread-851hmw`, un commit por tarea.
**Modo:** Strict TDD. Cada test nuevo se vio fallar antes de su código (por no compilar o por la aserción), salvo `a_code_block_save_is_chunked_like_any_other`, que pasa desde el principio porque protege un borrado.
**Estado:** verificado en Linux; R1 a R15 pasan. La búsqueda en docs no llega al umbral de la propuesta (Hit@3 0,20 contra 0,70). Los umbrales de R15 son provisionales. Quedan pendientes en la máquina del usuario la calibración sobre el golden set de memorias, la suite de Windows, `rag-eval --real --diff` y la prueba en el perfil dev (ver «Pendiente»). No se archiva hasta entonces.

## Suite

`cargo test --workspace --no-fail-fast` en Linux, por commit:

| Commit | Pasan | Fallan |
|---|---|---|
| base (`35e448c`) | 1639 | 10 |
| propuesta | 1639 | 10 |
| quitar el chunker de código | 1629 | 10 |
| tablas V11 | 1630 | 10 |
| corpus | 1637 | 10 |
| troceo | 1648 | 10 |
| HEAD | 1716 | 10 |

Las 10 fallas son las mismas en todos y vienen del entorno, no del código: los 7 `doctor::tests` (dan por hecho rutas de Windows) y los 3 `diff::tests` de `rag-eval` (leen `docs/eval/`, que no está en este clon). El conteo baja 10 con los tests de `chunker/code.rs` y sube 87 con los nuevos.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. El chunker de código se va | `a_code_block_save_is_chunked_like_any_other`; el build sin `#[allow(dead_code)]` ni `tree-sitter*`. `select_chunker` y sus tests se fueron con `ChunkerKind::Code`: con un solo chunker no queda nada que elegir | ✓ |
| R2. Corpus | `classify_takes_markdown_in_any_case`, `classify_takes_manifests_by_exact_name`, `classify_leaves_out_everything_else`, `the_walk_honours_gitignore_and_keeps_hidden_docs`, `oversized_and_binary_files_are_skipped`, `a_root_that_is_not_a_directory_is_an_error` | ✓ |
| R3. Troceo de Markdown | `sections_carry_their_breadcrumb`, `a_heading_inside_a_fence_does_not_split`, `a_fence_closes_only_with_its_own_character_and_length`, `an_empty_section_gives_no_passage_but_names_its_children`, `closing_hashes_are_not_part_of_the_heading`, `a_heading_indented_four_spaces_is_text`, `a_long_section_is_split_with_overlap_under_the_cap`, `a_long_breadcrumb_keeps_its_nearest_headings`, `an_empty_doc_has_no_passages` | ✓ |
| R4. Manifiestos | `a_manifest_is_one_passage_named_by_its_path` | ✓ |
| R5. Refresco | `plan_*`, `an_unchanged_project_embeds_nothing`, `a_changed_doc_replaces_its_passages_and_a_deleted_one_loses_them`, `another_model_reindexes_every_doc`, `a_missing_root_deletes_nothing`, `a_failed_embedding_keeps_the_doc_as_it_was`, `a_second_refresh_does_not_wait_for_the_first`, `refreshing_never_touches_memories` | ✓ |
| R6. Secretos | `a_secret_in_a_doc_never_reaches_the_index` | ✓ |
| R7. Búsqueda | `fuse_*`, `search_finds_a_passage_by_meaning`, `search_finds_an_identifier_lexically`, `search_never_crosses_projects`, `waiting_for_a_running_refresh_ends_when_it_does`; en `tests/api.rs` los `docs_search_*`, `a_root_that_vanished_still_answers_from_its_index` y `a_search_during_the_first_index_waits_for_it` | ✓ |
| R8. El registro indexa en segundo plano | `registering_a_root_indexes_its_docs_in_the_background` | ✓ |
| R9. `search_project` | `search_project_lists_each_hit_under_its_breadcrumb`, `instructions_say_when_to_search_the_project`, `docs_are_numbered_under_their_breadcrumbs`, `search_project_warns_while_indexing`, `an_empty_index_and_no_match_read_differently`, `a_failed_refresh_is_named_before_the_results`; los de handshake cuentan 9 herramientas | ✓ |
| R10. Indexar no cambia las memorias | `indexing_docs_leaves_memory_search_unchanged` | ✓ |
| R11. Datos de V10 | `a_database_from_the_previous_version_opens_with_an_empty_doc_index`, `published_migration_checksums_are_pinned` | ✓ |
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

## Pendiente (máquina del usuario)

- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows.
- [ ] `rag-eval --real --docs` en Windows, para confirmar los números de arriba con el binario de esa plataforma.
- [ ] `rag-eval --real --audit`, `--sweep-cos-floor` (por ejemplo 0,75 / 0,77 / 0,79 / 0,81 / 0,83) y el sweep de contradicciones sobre el golden set de memorias; fijar los umbrales de R15 con lo que den.
- [ ] `rag-eval --real --diff` y `--agent-view --diff` contra el baseline de memorias con esos umbrales: ninguna métrica baja.
- [ ] Arrancar el daemon dev sobre una copia de un store CLS: snapshot `pre-reembed`, `loading` mientras re-embebe, `ready` con el contrato `multilingual-e5-small/mean`.
- [ ] Perfil dev en Claude Code: abrir una sesión en este repo, preguntar algo de los docs, ver que el agente usa `search_project`, editar un doc y ver el resultado nuevo pasado un minuto.
