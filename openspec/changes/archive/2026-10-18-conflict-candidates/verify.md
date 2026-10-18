# conflict-candidates — Verificación

## Linux (2026-10-18)

Sobre `96fa419`, en `target`:

- `cargo test --workspace`: la suite pasa. Cada commit del cambio la corrió entera.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: termina con código 0.
- `cargo build -p clawg-cli`, sin la feature `eval`, da el mismo warning que ya traía `fdd71eb` (`ImportanceFactor::spanning`, `crates/clawg-core/src/memory/score.rs:88`). No viene de este cambio.
- El contrato del agente se bendijo una vez, a propósito. El diff se revisó a mano: la apertura suma una oración que pide juzgar las candidatas de `save_memory` (la apertura pasa a medir 1.948 de 2.048 caracteres); la descripción de `resolve_conflict` es nueva; `keep_both` sale del schema; entran `verdict` (enum de cuatro valores, obligatorio) y `reason`.
- `conflict-eval --fake` corre de punta a punta: 110 pares en cinco clases y un flujo con las oraciones con MUST de `openspec/specs`. Con el doble no se empareja nada, como se esperaba: esa corrida prueba el circuito, no los números.

| Requisito | Cómo quedó |
|---|---|
| C1 | `a_close_fact_is_a_pending_candidate_and_both_stay_active`, `a_close_preference_escalates_and_disputes`, `insert_writes_one_conflict_row_per_candidate` y `pending_candidates_alone_leave_the_row_active` |
| C2 | `needs_user_cases`, `escalate_below_pins_the_shipped_value` e `identifiers_are_shared_only_when_both_texts_hold_one`; después de medir, también `identifier_escalate_below_pins_the_shipped_value` (ver «Lo que cambió después de medir») |
| C3 | `a_long_save_pairs_once_with_an_earlier_save` y `a_chunk_still_pairs_with_an_older_row`. R8 sigue fijado por `sibling_chunks_of_one_save_do_not_dispute_each_other` |
| C4 | Los seis tests de `dispute.rs` que nombra la spec, más `the_old_resolve_records_its_verdict`, `open_pairs_count_and_list_the_live_pending_ones` y `every_verdict_parses_back_to_itself`. R1 a R3 de `specs/disputes` siguen con sus tests de siempre |
| C5 | `judge_records_a_verdict`, `judge_refuses_scoped_without_reason`, `resolve_with_keep_both_still_works`, `disputes_lists_pending_pairs_of_the_agent`, `resolve_conflict_reports_the_outcome`, `resolve_conflict_against_an_older_daemon_asks_for_a_restart` y `memory_status_counts_pending_candidates` |
| C6 | `a_confirmed_save_names_its_candidates`, `save_outcome_text_names_related_memories` y `save_body_leaves_related_out_when_empty` |
| C7 | `the_conflict_note_does_not_claim_a_contradiction` y `the_instructions_carry_the_saving_habits`; después de medir, también `the_conflict_note_is_no_longer_than_the_one_without_verdicts` |
| C8 | `a_restored_row_brings_its_escalated_pair_back`, `purging_drops_the_pairs_of_the_row` y `maintenance_drops_pairs_of_missing_rows`. `a_restored_row_settles_its_own_dispute` cambió a propósito: la disputa con la fila restaurada vuelve (R4) |
| C9 | `a_database_with_disputes_opens_with_escalated_pairs` y el checksum de V15 en `published_migration_checksums_are_pinned` |
| C10 | `wilson_interval_cases`, `labelled_pairs_parse_and_cover_every_class`, `a_fake_run_measures_every_class_and_the_flow`, `the_repository_specs_make_a_flow_of_at_least_two_hundred_facts` y `conflict_pairs_reads_the_table`; después de medir, también `a_flow_file_is_read_in_order` y `without_a_flow_file_the_flow_is_the_spec_sentences_as_facts` |

### Tests que cambiaron de sentido

- `a_chunk_still_disputes_an_older_row` pasó a `a_chunk_still_pairs_with_an_older_row`. Antes cada pedazo quedaba `disputed` contra la fila vieja; ahora hay un solo par, `pending`, porque los dos son `semantic_fact` sin identificador en común.
- `a_conclusion_does_not_dispute_its_sources` lee el par pendiente en vez de la marca `disputed`.
- `resolve_rows_without_a_dispute_conflict` usa dos filas sin relación: un par ya juzgado sigue enlazado y se puede juzgar de nuevo (C4).

## Windows (2026-10-18)

En `target-test`, después de `cargo clean --workspace --target-dir target-test`:

- Sobre `e041ee6` (el merge del cambio): pasan 2.219, no falla ninguno y queda 1 ignorado. Clippy termina con código 0.
- Sobre `711b02e` (con los arreglos de abajo): pasan 2.224, no falla ninguno y queda 1 ignorado. Clippy termina con código 0.
- `disputed_status_does_not_change_score` (`crates/clawg-core/src/memory/search.rs`) falló una vez en una corrida de la librería y pasó en las cinco siguientes. Inserta dos filas con `last_accessed_at` en segundos de `strftime('now')`, y si caen a los dos lados de un cambio de segundo sus puntajes difieren. Está desde `57119d3` y quedó en el backlog del README.

## Modelo real (2026-10-18)

Con el modelo descargado en el perfil dev y `docs/eval/`. Los resultados están en `docs/eval/results/`, fuera de git.

### La compuerta falló sobre el cambio tal como se mezcló

`conflict-eval --real` sobre `e041ee6` (`2026-10-18-real-conflict-eval-tip.json`):

- Clase contradicción: candidata en 30 de 30 pares (Wilson 0,886–1). Esa parte de la compuerta pasa.
- Flujo de 440 oraciones de las specs, todas `semantic_fact`: 103 saves con un par escalado, el 23,4 % (19,7–27,6 %), contra un tope del 5 %. **Falla.** Todo lo que escalaba ahí lo hacía por identificador compartido: las oraciones de un proyecto nombran siempre los mismos (`agent_id`, `memory_type`, `V14`), y esos pares quedaban entre 0,34 y 0,40, con mediana 0,374.

Una corrida con los pares volcados mostró además que la regla del identificador atrapaba una sola contradicción `semantic_fact` de 21 en el set etiquetado, a 0,218.

`rag-eval --real --agent-view` sobre `e041ee6` subió los tokens medios de `recall_context` de 1.085,5 a 1.097,3, todo en el bloque de conflictos: la NOTE nueva medía 291 bytes. La línea de base de 1.086,4 que nombraba la propuesta se midió en `f3c44ef`, antes de la NOTE en inglés de memory-protocol; para separar los efectos se midió `fdd71eb` en un worktree aparte (`2026-10-18-real-memory-protocol-tip-agent-view.json`: 39/46 con 1.085,5).

### Un store real como segunda prueba

Para no medir sólo con hechos de specs, `conflict-eval` aprendió a reproducir un flujo de saves con su tipo (`--flow FILE`). Se reprodujo el store de un perfil prod en uso: 172 saves en orden de creación, con su tipo tal como se guardó (78 `semantic_fact`, 42 `architectural_decision`, 5 `user_preference` y 47 de tipos que el detector no mira), los saves en pedazos unidos con `read_memory`. El archivo de entrada tiene el texto de las memorias y no se guarda en ningún lado del repositorio; sólo los números quedan en `docs/eval/results/`.

Con la regla de `e041ee6`, el 28,5 % de esos saves escalaba (22,3–35,6 %). De los pares escalados, la mayoría tenía una `architectural_decision` de un lado, y 24 de las 42 "decisiones" eran notas de lo que pasó: un cambio archivado, una verificación, una medición.

### Lo que cambió después de medir

Cinco commits en `main`, cada uno con su test primero:

| Commit | Qué |
|---|---|
| `659a628` | Un identificador compartido escala sólo bajo `IDENTIFIER_ESCALATE_BELOW` = 0,30 |
| `951c6c2` | La NOTE vuelve a 228 bytes o menos, con los mismos veredictos |
| `1c4efc3` | `conflict-eval --flow FILE` reproduce un arreglo JSON de `{content, memory_type}` |
| `2d9a11b` | Un par escala por tipo sólo si **los dos** lados son preferencias o decisiones. Un hecho cerca de una decisión queda pendiente para el agente, que no puede reemplazar la decisión sin pasar por el usuario (`NeedsUser`) |
| `711b02e` | La descripción de `memory_type` en `save_memory` dice que una decisión es una elección de diseño y su motivo, y que lo hecho, hallado o medido es un `semantic_fact`. `protocol.md` dice lo mismo y ubica un bug arreglado como `semantic_fact` |

### Resultado sobre `711b02e`

`conflict-eval --real` (`2026-10-18-real-conflict-eval-final.json`):

| Clase | Candidata | Escalada |
|---|---|---|
| contradicción | 30/30 | 10/30 |
| actualización | 15/15 | 4/15 |
| reformulación | 14/15 | 4/15 |
| complemento | 6/20 | 2/20 |
| distinto | 0/30 | 0/30 |

- Flujo de specs: 3 de 440 saves con un par escalado, el 0,7 % (0,2–2,0 %). **La compuerta pasa**, y la clase contradicción sigue con 30 de 30 candidatas. Las contradicciones protegidas del set etiquetado siguen escalando 9 de 9, porque sus dos lados son del mismo tipo.
- Store real, con los tipos como se guardaron (`2026-10-18-real-conflict-eval-store-flow-final.json`): 19 de 172, el 11,0 % (7,2–16,6 %).
- Store real con las 24 notas de estado reetiquetadas como `semantic_fact` (`…-store-flow-relabelled.json`): 9 de 172, el 5,2 % (2,8–9,6 %).

Hay que leerlo con cuidado. El corte de 0,30 y la regla de los dos lados se eligieron mirando estos mismos flujos, así que el 0,7 % y el 5,2 % no son una confirmación sobre datos nuevos. La compuerta fijada en la propuesta, la del flujo de hechos distintos, pasa; el store real, que la propuesta no fijaba, queda en el 11 % con las etiquetas que tiene. Lo que falta para bajarlo es la etiqueta, no el detector: `711b02e` cambia lo que lee el modelo al guardar, y las notas que ya están se reetiquetan al instalar.

`rag-eval --real` sobre `711b02e`:

- Ancla contra `2026-10-17-real-rank-fusion-tip.json`: ningún caso se movió, ni sobre `e041ee6` ni sobre `711b02e`.
- `--agent-view` contra `fdd71eb`: `recall_context` 39/46 con 1.083,7 tokens (era 1.085,5), y `search_memory` 36/46 con 764,0, igual. Los 42 casos con bloque de conflictos bajan 1,8 tokens de media: lo que muestra el bloque no cambió, sólo la NOTE.

## Perfil dev con Claude Code (2026-10-18)

Con `target\debug\clawg.exe` compilado desde `711b02e` (0.17.0) y registrado en Claude Code como `clawg-dev`:

- Al reconectar, el daemon escribió el snapshot `pre-migration` y aplicó V15 sobre el store dev. Las instrucciones llegaron con la oración nueva sobre juzgar las candidatas.
- Dos `semantic_fact` cercanos: las dos `active`; la respuesta del segundo save dijo `close to earlier memories: #59 "…"`, sin `asks the user`; `memory_status` contó `1 for you to judge` y citó `#60 close to #59`.
- `resolve_conflict` con `scoped` sin motivo: `400` que pide el motivo. Con `verdict: "duplicate"`: `400` que lista los cuatro. Con `supersedes`: #59 reemplazada por #60.
- Un `semantic_fact` cerca de una `architectural_decision`: quedó pendiente. `supersedes` del hecho sobre la decisión: `409` que pide `conflicts` y preguntar. Con `conflicts`, el par pasó a ACTIVE CONFLICTS con la NOTE nueva; se cerró `compatible`.
- Dos `user_preference` cercanas: el save dijo `asks the user` y `memory_status` contó `1 waiting for the user`. Sobre el par escalado, `supersedes` sí se aceptó.
- Las seis memorias de prueba (#59 a #64) se borraron.

Claude Code, después de reconectar, siguió mostrando los schemas viejos de las herramientas (`resolve_conflict` con `keep_both` y sin `verdict`), pero pasó `verdict` al servidor igual. El modelo ve el `enum` de veredictos recién en una sesión nueva.

## Diferencias con la propuesta y la spec

- **Rollback.** La propuesta dice que un binario anterior abre una base en V15 «porque refinery acepta migraciones aplicadas que no conoce». No es así: `check_schema` (`crates/clawg-core/src/memory/db.rs`) la rechaza sin tocarla con `SchemaNewer`, que nombra `clawg restore` (`a_newer_database_is_refused_untouched`). Después de migrar, se vuelve atrás con el snapshot `pre-migration`, o borrando la tabla y su fila en `refinery_schema_history`. `specs/disputes` lo dice así.
- **C2.** El escalamiento cambió después de medir: por tipo, sólo si los dos lados son preferencias o decisiones; por identificador, sólo bajo 0,30.
- **C7.** La NOTE tiene además un tope de 228 bytes.
- **C10.** `conflict-eval` lee también un flujo de un archivo (`--flow`).
- **La compuerta.** La propuesta la escribe como «menos del 5 % de pares escalados por save»; `conflict-eval` mide la fracción de saves con algún par escalado (`saves_escalated`), que es lo que dice `verify.md` desde el principio. Con los pares por save (`escalated_per_save`) el flujo de specs da 0,007.

## Al archivar

- `specs/disputes` se reescribió con los pares, el escalamiento, los veredictos, las candidatas en la respuesta del save, R4 con restaurar y purgar, V15 y el rollback. `specs/mcp`, `specs/retrieval`, `specs/storage`, `specs/eval` y `specs/ingest` suman lo suyo.
- Las citas `path:line` de los archivos que tocaron el cambio y los arreglos se llevaron a HEAD en todas las specs y en el README, por identidad de línea desde `fdd71eb` (la última alineación), y después desde cada commit nuevo.
- Al revisarlas aparecieron citas que ya estaban mal antes de este cambio y se corrigieron:
  - `client.rs` estaba cinco líneas arriba en `specs/mcp` (seis citas), `specs/retrieval`, `specs/secrets` y `specs/ingest` (dos);
  - `EvalIngest` en `specs/ingest`;
  - `K_FINAL` y el rango de `purge` en `secrets.rs`, los dos en el README.
- El backlog del README pierde la tabla CLS del comentario del umbral, que este cambio borró, y el tipo de un bug arreglado, que resuelve `711b02e`. Suma el test intermitente.

## Veredicto

Verificado en Linux, en Windows, con el modelo real y con Claude Code sobre el perfil dev. La compuerta de la propuesta pasa después de los arreglos; el store real queda en el 11 % y baja al 5,2 % con las etiquetas corregidas. Se archivó el 2026-10-18 en [`specs/disputes`](../../../specs/disputes/spec.md), con ajustes en [`specs/mcp`](../../../specs/mcp/spec.md), [`specs/retrieval`](../../../specs/retrieval/spec.md), [`specs/storage`](../../../specs/storage/spec.md), [`specs/eval`](../../../specs/eval/spec.md) y [`specs/ingest`](../../../specs/ingest/spec.md).
