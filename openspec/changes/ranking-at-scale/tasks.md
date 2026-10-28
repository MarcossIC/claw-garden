# ranking-at-scale — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla, por la aserción o porque no compila. Cada tarea de código es un commit convencional que compila y pasa la suite (`cargo test --workspace --target-dir target-test` en Windows) y los dos clippy de `.claude/CLAUDE.md`. Antes de la primera: `cargo clean --workspace --target-dir target-test`. Ningún test del paso 1 necesita el modelo real ni un perfil de datos.

Requisitos en la [spec](spec.md) (RS1 a RS14) y decisiones en el [diseño](design.md) (D1 a D10). El orden es el de su «Orden de commits», con los ajustes de abajo.

## Frente al diseño

| Punto | Diseño | Tareas | Por qué |
|---|---|---|---|
| `EvalSearch`, `EvalHit.memory_type` y `exact`, `last_accessed`, `cosine_from_distance` | commit 2 | tarea 8 | Ningún rojo del 2 los usa. Entran con el informe que los lee y con su test: nada de visibilidad sin un uso real (`openspec/config.yaml`, reglas de diseño) |
| `EvalStore::close` | commit 4 | tarea 6 | Su primer uso y su test (`the_background_copy_is_removed_even_when_the_run_fails`) son de `rag-eval` |
| Contrato en `EvalIngest::open` (D8) | commit 4, sin rojo | tarea 4, con `an_evaluation_ingest_records_and_checks_its_embedding_contract` | Es conducta nueva |
| `--background` con `--agent-view` | — | error de uso en la tarea 6, que la 9 levanta | Entre la 6 y la 9, agent-view ignoraría el fondo sin decirlo |
| `expected_is_last_summary` | D4 lo calcula; ni RS8 ni D7 lo informan | la tarea 5 lo resuelve y la 8 lo informa en los casos cuyo `expected_type` es `session_summary` | La puerta 1 lo lee para elegir la excepción (decisión 6 de la propuesta) |
| `golden_twins` (RS3) | sin test propio | `a_golden_row_also_in_the_background_is_named_a_twin`, tarea 6 | Cada requisito con un test que hoy falla |

Al implementar el paso 1 cambió además:

| Punto | Diseño | Implementado | Por qué |
|---|---|---|---|
| El pool de `memory` (D3) | `SearchTrace { telemetry, memory }` | `search_all_memories_inner` devuelve una tupla de tres, y `MemoryPool` es `pub` con campos `pub` | Sin la feature `eval`, nadie de producción lee el pool. Un struct `pub(crate)` dejaría campos sin leer, y el clippy del release lo trata como error |
| Fondos de los tests de la tarea 5 | `import_foreign` | filas de `EvalStore` marcadas `source = 'import'` por SQL, con piezas de `link_pieces` | La tarea prueba la resolución, no la traducción. Un export ajeno necesita el tokenizer y no suma nada a lo que se mide |
| Fondos de los tests de `rag-eval` | un export `clawg-memories` importado por el router en proceso | saves de un `EvalIngest` falso, marcadas como importadas sobre la conexión de su propio store y cerradas limpias | Conservan piezas, origen y proyecto sin escribir un export a mano ni sumar `rusqlite` a `clawg-eval` |
| `golden_twins` en agent-view | D5: agent-view no distingue un gemelo | `EvalIngest::holding` compara, antes de sembrar, el `content_hash` que tendría cada fila del corpus con las filas vivas del fondo | Así RS3 se cumple también en agent-view, donde la ingesta fusiona el gemelo con la fila del fondo |
| Parser de agent-view | — | `parse_search_line` lee el score antes de `by <author>`, y la salida salta la nota de filas compartidas (`shared_note`, en los dos idiomas) | Sobre un fondo real casi todo lo guardó otro agente (`migrated`), y el parser rechazaba esas líneas |
| La observable de `agent_view_sends_the_case_project_to_search_and_context` | que el contexto encuentre la save | la búsqueda se arma con la pura `search_request`, y el contexto abre con `LAST SESSION` sólo si recibió el proyecto | `read_context` no lee esa sección, y cambiarlo movería el ancla |

## Paso 1: medir a escala

### 1. El pool de `memory` como dato (RS9; D3, D6)

- [x] RED: `the_memory_pool_counts_saves_and_reports_its_eligible_rows` (`search.rs`). `saves` cuenta saves distintas, no filas, y una ventana que vuelve corta por el alcance da `eligible`.
- [x] GREEN:
  - `KnnOutcome` suma `saves` y `eligible`. `search_all_memories_inner` devuelve `SearchTrace { telemetry, memory: MemoryPool }`;
  - `search_all_memories` lo descarta. `search_all_memories_with_scoring` lo devuelve, y `EvalStore::search` lo ignora hasta la tarea 8;
  - los tests de la línea de log (`format_pool_line`) pasan sin cambiar lo que comprueban.
- Commit: `refactor(core): hand back a search's memory pool as data`

### 2. La búsqueda de evaluación con el alcance del daemon y un proyecto (RS4; D2)

- [x] RED (`eval_api.rs`):
  - `production_reach_reads_other_agents_and_unowned_rows`;
  - `a_project_search_reads_its_own_unowned_and_every_preference`;
  - red de seguridad: `a_one_agent_store_ranks_the_same_under_the_daemons_reach`, contra `search_all_memories_with_scoring` con `Reach::Own`.
- [x] GREEN: `EvalStore::search(…, project: Option<&str>, …)` con `reader_for(agent).reach` y el proyecto pasado por `canonical`. `EvalScope` sigue siendo `Copy`. `main.rs` y `--follow-up` pasan `None`.
- [x] Los tests de `main.rs` pasan sin cambios: el ancla con `--fake` da el mismo `per_case`.
- Commit: `feat(core): search an evaluation store with the daemon's reach and a project`

### 3. La edad en la ingesta de evaluación (RS5; D8)

- [x] RED: `eval_ingest_backdates_created_at` (`eval_api.rs`).
- [x] GREEN: `EvalIngest::send(…, age_ms)` adelanta el `timestamp_ms` de `explicit_event`. `agent_view/runner.rs` pasa la edad de la fila, 0 sin `age_days`.
- Commit: `feat(core): backdate the saves of the evaluation ingest`

### 4. Copiar un snapshot o rehusarlo sin tocarlo (RS2, RS3; D1, D7, D8)

- [x] RED (`eval_api.rs`, con fondos de `seed_on_disk` y de un `EvalIngest` falso):
  - `a_snapshot_with_a_nonempty_wal_is_refused_untouched`;
  - `a_snapshot_of_another_embedding_contract_is_refused_untouched`, también uno CLS (el mensaje dice que hay que abrirlo con el daemon y respaldarlo de nuevo) y uno sin contrato;
  - `a_newer_schema_snapshot_is_refused`, también uno dañado y uno sin historia de clawg;
  - `an_older_schema_snapshot_is_migrated_in_the_copy` (`seed_on_disk(dir, Some(N))`);
  - `the_composition_counts_rows_without_text`;
  - `an_evaluation_ingest_records_and_checks_its_embedding_contract`;
  - los casos de la pura `background_contract`.
- [x] GREEN:
  - `wal_with_commits`, sacado de `check_restorable` (`backup.rs`), que `restore` sigue usando. `check_restorable` pasa a `pub(crate)` y devuelve su `SchemaPlan`. `inspect::contract` pasa a `pub(crate)`;
  - `copy_background`, que copia en bloques de 1 MiB con el sha256 en la misma pasada, decide con el plan y lee el contrato con `open_for_probe`. `EvalBackground`, y `BackgroundRefusal` con `thiserror` dentro de `EvalError::Background`;
  - la composición: un `GROUP BY` por eje, con `by_project` por nombre y `placement`;
  - `EvalIngest::open` llama a `ensure_embedding_contract` y falla ante `Mismatch`.
- [x] Los tests de `restore` pasan sin cambios.
- Commit: `feat(core): copy a backup snapshot into an evaluation store or refuse it untouched`

### 5. Resolver el proyecto y la save de un caso (RS6, RS7; D4)

- [x] RED (`eval_api.rs`, con un fondo armado con `import_foreign`, el embedder falso y el chunker por bytes):
  - `a_project_name_resolves_to_each_backgrounds_own_id`;
  - `an_ambiguous_or_unknown_project_name_is_refused`, también uno que lleva a un proyecto borrado;
  - `an_origin_resolves_to_every_live_piece_of_its_save`. Sólo cuentan las filas con `source = 'import'`: una save nativa con el mismo `created_by_ref` no entra;
  - `an_origin_without_a_live_row_is_refused`, también la save que quedó `superseded`;
  - `an_expected_save_outside_the_case_project_is_refused`;
  - `a_resolved_save_says_whether_it_is_the_last_summary_of_its_project`.
- [x] GREEN: `resolve_cases` en `EvalStore` y en `EvalIngest`, sobre una sola función de `&MemoryStore`. Más `EvalCaseRef`, `EvalResolveError` y `EvalResolved`, que lleva el tipo, el idioma (con `detect` si `lang` es NULL), el texto unido con `join_pieces` y `expected_is_last_summary`.
- Commit: `feat(core): resolve a case's project by name and its expected save by origin`

### 6. `rag-eval --background` (RS1, RS2, RS3, RS11; D1, D5, D7)

- [x] RED (`cli.rs`, `main.rs`):
  - `background_parses`;
  - `background_with_docs_is_a_usage_error`. Lo mismo con `--lang-detect`, `--fts-tokenizer`, `--follow-up`, `--audit`, `--lexical-only` y, hasta la tarea 9, `--agent-view`. Los `--sweep-*` se aceptan;
  - `a_background_run_leaves_the_snapshot_byte_identical`;
  - `the_background_copy_is_removed_even_when_the_run_fails`;
  - `a_run_without_background_measures_as_before`;
  - `a_refused_background_exits_with_one_naming_its_cause`;
  - `the_report_describes_the_background_without_text`;
  - `a_golden_row_also_in_the_background_is_named_a_twin`;
  - `the_results_file_with_a_background_holds_no_stored_text`, que comprueba también el tag `bg-<8 hex>`;
  - `a_background_hit_ranked_first_keeps_its_place`, que hoy desaparece por el `filter_map` (`main.rs:446`).
- [x] GREEN:
  - `--background` en `cli.rs` y en la ayuda;
  - la corrida pasa a una función interna que devuelve el código, y la externa siempre cierra (`EvalStore::close`) y borra. `remove_scratch` va a un módulo compartido de `rag-eval`;
  - el rango con `bg` (D5), sólo con fondo;
  - `RunResults.background`, con `searched_at_ms` y `golden_twins`, y el tag armado con `fold_tag`.
- Commit: `feat(eval): measure over a copy of a snapshot with --background`

### 7. Casos por origen y con proyecto (RS6, RS7; D4, D5)

- [x] RED (`golden.rs`, `main.rs`). El fondo con orígenes es un export `clawg-memories` v1 escrito en el test, importado por el router en proceso (`serve_ephemeral`, `DaemonClient::import`) sobre un `EvalIngest` falso, que después se cierra y se copia como snapshot.
  - RS6: `a_case_may_cite_its_expected_save_by_origin`, `an_origin_without_a_row_fails_validation`, `an_origin_case_without_background_fails_validation`, `an_origin_case_cannot_also_carry_corpus_refs`, `a_hit_on_any_piece_of_the_cited_save_counts`, `a_background_hit_not_cited_never_counts` y `a_golden_without_origins_measures_exactly_as_before`;
  - RS7: `an_ambiguous_or_unknown_project_name_fails_validation`, `project_without_background_fails_validation`, `an_expected_save_outside_the_case_project_fails_validation` y `an_unanswerable_case_may_name_a_project`.
- [x] GREEN:
  - `expected_origins` y `project` en `RetrievalCase`, con `deny_unknown_fields` como está. La validación de clase y de mezcla;
  - `resolve_cases` corre antes de medir y sale con 2;
  - el origen entra en el rango (D5, paso 2) y el proyecto del caso llega a `EvalStore::search`;
  - el bloque de similitud excluye los casos por origen.
- Commit: `feat(eval): cite a case's save by origin and name its project`

### 8. El informe por caso y sus agregados (RS8, RS9, RS10; D3, D6, D7)

- [x] RED (`main.rs`, `report.rs`, `eval_api.rs`):
  - RS8: `a_case_reports_the_type_of_each_top5_hit`, `best_cosine_of_an_identical_row_is_one`, `the_gap_is_first_minus_tenth`, `a_case_reports_passes_pool_survival_and_decay_age`, `an_unanswerable_case_reports_its_distribution_apart`, `the_per_case_report_holds_no_stored_text_or_row_id` y `a_case_expecting_a_summary_says_whether_it_is_the_last_one`;
  - RS9: `a_case_whose_pool_returns_fewer_than_thirty_saves_is_a_short_pool`, `a_crowded_window_is_told_apart_from_a_small_scope` y `short_pool_cases_stay_out_of_the_weight_aggregates`;
  - RS10, puras sobre `per_case`: `top3_places_by_type_against_row_share` y `distributions_split_by_answered_pair_and_partition`.
- [x] GREEN:
  - `EvalStore::search` devuelve `EvalSearch { hits, pool }`, y `EvalHit` suma `memory_type` y `exact`. Más `last_accessed` y `cosine_from_distance`, con `cosine_from_l2_unit` en `pub(crate)`;
  - los campos de `CaseResult` llevan `serde(default, skip_serializing_if)` y salen sólo con fondo. Los casos por origen llenan `lang`, y los `unanswerable` van en `retrieval.unanswerable`;
  - `RetrievalResults.scale`: `top3_by_type`, las distribuciones (n, p5, p25, p50, p75, p95) y `short_pool_cases`.
- Commit: `feat(eval): report each case's types, cosine, gap, pool and ages`

### 9. Agent-view sobre un fondo (RS4, RS6; D8)

- [x] RED (`agent_view/runner.rs`, `agent_view/matcher.rs`): `agent_view_sends_the_case_project_to_search_and_context` y `an_origin_case_is_found_in_any_piece_of_its_save`. Sale el error de uso de `--background` con `--agent-view` de la tarea 6.
- [x] GREEN:
  - `copy_background` corre antes de `EvalIngest::open(scratch)`. Después van `resolve_cases`, la composición y recién ahí el corpus;
  - el id resuelto va en `SearchArgs.project_id` y en `context`;
  - el `Corpus` del matcher suma `(origen, texto unido)`.
- Commit: `feat(eval): run agent-view over a background with each case's project`

### 10. `--diff` nombra el fondo de cada lado (RS12; D9)

- [x] RED (`diff.rs`): `diff_compares_an_anchor_run_with_and_without_background` y `a_diff_over_one_background_a_day_apart_warns`. El aviso no cambia la salida de `--fail-on-regression`.
- [x] GREEN: `--diff` imprime el sha256 y `searched_at_ms` de cada lado y, cuando los dos leyeron el mismo fondo con más de 24 h entre medio, el aviso.
- Commit: `feat(eval): name each run's background in --diff`

### 11. Docs

- [x] `.claude/CLAUDE.md`, sección Testing: `rag-eval --background`, la forma de caso por origen y con proyecto, y el informe a escala. La fila 86 de `openspec/README.md`.
- Commit: `docs: rag-eval --background and cases by origin`

## Medición del paso 1 (verify)

Necesita el modelo real y autorización para leer `~/.clawg-dev` en el momento. Los fondos, los casos y los resultados quedan en `docs/eval/` o en el temporal, fuera de git. Las dos corridas de una comparación sobre un mismo fondo se hacen el mismo día.

### 12. Suite y CI

- [x] La suite y los dos clippy de `.claude/CLAUDE.md` en Windows, en `target-test`.
- [ ] El CI del pull request en verde en Linux y en Windows.

### 13. Los dos snapshots (D10) **[autorización, modelo real]**

- [ ] Corte viejo: `clawg backup --profile dev`, copiado como `old-cut.db` en el temporal. Una corrida con el ancla da su `placement` y su `by_source`. Si `by_source` no tiene filas `import`, el corte viejo sólo atribuye con el ancla (riesgo del diseño).
- [ ] Corte nuevo: en un home aislado, los mismos proyectos por nombre. Después `engram export --all` y `clawg import --from engram … --project-map "<origin>=project:<nombre>"`, y un backup copiado como `new-cut.db`. Al final se borran el home y el export.
- [ ] Comprobar que el `placement` de `new-cut.db` es el del viejo, salvo las notas que engram sumó o borró.

### 14. El set minado **[usuario]**

- [ ] Minar del historial al menos 80 casos con respuesta y 20 sin respuesta, con las 12 búsquedas de validación como semilla, en las parejas `es>es`, `en>en` y `es>en`. Cada uno lleva `expected_origins` (salvo los sin respuesta) y `project` por nombre.
- [ ] Validarlos uno por uno con el usuario. Los que no validan en los dos cortes (`NoLiveRow`, `OutsideProject`) se sacan y se cuentan.

### 15. Línea base

- [ ] El ancla de 46 sin fondo y con cada corte, comparada con `rag-eval --diff` (RS12): el costo de la escala.
- [ ] El set minado en los dos cortes, en la ruta ordenada y en agent-view: por tipo, pareja y partición, con `scale.top3_by_type`, las distribuciones y `short_pool_cases`.
- [ ] Los resultados en `docs/eval/results/`, y en `verify.md` las cifras y las cuentas, sin texto.

### 16. Puerta 1

- [ ] Leer `scale.top3_by_type` del corte nuevo, en calibración, contra U1. Si `session_summary` ocupa ≤ 25 % de los lugares en los casos que no lo piden, el paso 2 se cierra sin código.
- [ ] Si abre, presentar al usuario los datos de las tres elecciones de la decisión 6 de la propuesta y escribir lo decidido en `verify.md`. La excepción la deciden los casos con `expected_type` `session_summary` y `expected_is_last_summary` falso. La forma es B1 por defecto. B3 entra según los `decay_age_days` de los resúmenes del top 3 frente a los 209 días. Si la elección no es la de por defecto, las tareas 17 a 19 se reescriben antes de empezar.

## Paso 2 (sólo si abre la puerta 1)

Escrito para la forma por defecto: B1 con la excepción (iii).

### 17. El peso, con 1,0 por defecto (RS13)

- [ ] RED:
  - `a_session_summary_scores_below_an_equal_fact_with_the_weight` (`score.rs`);
  - `twelve_summaries_no_longer_bury_a_slightly_less_close_fact` (`search.rs`);
  - el parseo de `--sweep-summary-weight` (`cli.rs`);
  - redes de seguridad: `the_contradiction_pool_ignores_the_type_weight` y `eval_scoring_config_default_matches_scoring_config_default_field_for_field`.
- [ ] GREEN:
  - `TypeWeights { session_summary }` en `ScoringConfig`, con 1,0 en `DEFAULT`, así que nada cambia bit a bit. El factor va al final de `score()`;
  - `EvalScoringConfig.session_summary_weight` y `with_session_summary_weight`;
  - `--sweep-summary-weight`, repetible, con un eje más en `build_grid` y el slug `sumw0.50`.
- Commit: `feat(core): weigh session summaries in the score`

### 18. El barrido **[modelo real]**

- [ ] Barrer sobre el corte nuevo en calibración e informar en prueba: U1, U2 (`--fail-on-regression` sale con 0, y el ancla no mueve ningún caso sin explicación) y U4 (`es>en`). Si no se cumplen, el paso se cierra sin cambiar `DEFAULT`.

### 19. El valor por defecto (si cumple)

- [ ] RED: `at_most_the_agreed_places_of_the_top3_are_summaries` (`POST /v1/search` con `limit` 5, `crates/clawg-cli/tests/api.rs`).
- [ ] GREEN: el peso elegido en `ScoringConfig::DEFAULT` y en su espejo. También la línea base del ancla en el `context` de `openspec/config.yaml`, con la corrida nueva, y en `.claude/CLAUDE.md` cómo puntúa una búsqueda.
- Commit: `feat(core): rank session summaries below equally close facts`

### 20. Puerta 2

Corre aunque la puerta 1 haya cerrado el paso 2.

- [ ] Leer las distribuciones de `best_cosine` (C1) y de `score_gap_1_10` (C2), con y sin respuesta, por pareja y en calibración. La pregunta es si separan con la cota de Wilson de U3 (0,584 para 16 de 20). Si no separan, el paso 3 se cierra dejando escritos los casos y las distribuciones.
- [ ] Si el 1 y el 2 tardaron, el paso 3 pasa a un cambio propio (decisión 5 de la propuesta).

## Paso 3 (sólo si abre la puerta 2)

Escrito para la forma por defecto: C1, en el daemon y en el núcleo. Si la puerta elige C2, estas tareas se reescriben.

### 21. El umbral, en calibración

- [ ] Elegir `ABSTAIN_COSINE` con los `best_cosine` del set minado, en calibración y por pareja. Con los datos del paso 1 se simula qué casos con respuesta perderían su fila, sin tocar código.

### 22. La puerta de abstención (RS14)

- [ ] RED:
  - `a_list_under_the_threshold_shows_nothing` y `a_list_with_one_hit_over_keeps_todays_behaviour`, sobre la función pura que se aplica junto a `shown_hits`;
  - `recall_context_abstains_when_every_hit_is_under_the_threshold` (`prompt_builder.rs`);
  - `the_abstention_threshold_names_its_embedding_contract` (`score.rs`), como el del piso.
- [ ] GREEN:
  - `abstains(best_cosine, umbral)` y `ABSTAIN_COSINE`;
  - cuando se dispara, el daemon retiene todo hit no exacto, sin `MIN_SHOWN_HITS`, y `withheld` los cuenta;
  - el contexto la aplica antes de `retain(score > 0)`;
  - el puente no cambia. En `.claude/CLAUDE.md`, la abstención.
- Commit: `feat(core): abstain when no memory is close enough`

### 23. U3 en prueba **[modelo real]**

- [ ] En prueba, agent-view sobre el corte nuevo: la puerta abstiene bien en al menos 16 de 20, con la cota de Wilson publicada, y `found` de los casos con respuesta no baja más de 2. Además U2 y U4. Si no se cumple, se revierte el commit 22.

## Verificación y archivo

### 24. `verify.md`

- [ ] Informar que la suite pasa (nunca cuántos tests), cada métrica de `rag-eval` que el cambio toca, las puertas con sus datos y lo decidido en cada una. Si el paso 3 pasó a otro cambio, decirlo.

### 25. Archive

- [ ] RS1 a RS12 en `specs/eval`, y RS13 y RS14 en `specs/retrieval` sólo si entraron (`specs/mcp` sólo si cambió el contrato que ve el modelo). Después, `git mv` a `changes/archive/<fecha>-ranking-at-scale`, `state.yaml` archivado, y la fila 86 y el orden de ejecución del `README.md` de openspec.
- [ ] Subir el minor de `[workspace.package] version` y correr `cargo update --workspace`. El commit es `chore: bump version to X.Y.Z`.

La ubicación de las filas sin proyecto del import en prod (la precondición de la propuesta) no es una tarea de este cambio.
