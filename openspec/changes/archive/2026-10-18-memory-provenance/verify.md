# memory-provenance — Verificación

## Linux (2026-10-18)

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- `cargo build -p clawg-cli` sigue dando el warning de `ImportanceFactor::spanning`, que ya estaba en el backlog y este cambio no toca.
- El contrato del agente (`agent_contract.json`) cambia sólo en el argumento `user_stated` de `save_memory`; las instrucciones del servidor no cambian, porque `protocol.md` cambió fuera de su apertura.

| Requisito | Tests |
|---|---|
| P1 | `seal_cases` (`provenance.rs`); `an_undeclared_save_is_the_agents`, `a_declared_save_is_the_users`, `a_conclusion_is_sealed_as_synthesis` (`tests/api.rs`); `explicit_overrides_reach_the_stored_row` (`tests/public_api.rs`) |
| P2 | `user_stated_with_derived_from_is_refused` (`tests/api.rs`); `an_event_without_user_stated_reads_as_undeclared` (`events.rs`); `the_save_schema_names_user_stated` (`server.rs`); `a_save_sends_user_stated_only_when_declared` (`client.rs`) |
| P3 | `mark_cases` (`provenance.rs`) |
| P4 | `open_pairs_carry_each_sides_mark` (`dispute.rs`); `the_unmarked_side_of_a_pair_goes_first` (`tools.rs`); `the_conflict_block_marks_an_inferred_side` (`prompt_builder.rs`); `memory_status_counts_pending_candidates` (`server.rs`), que por HTTP ve dos saves sin declarar marcados `inferred` |
| P5 | `the_synthesis_marks_each_rows_origin` (`tools.rs`) |
| P6 | `rows_saved_before_provenance_keep_their_origin_and_no_mark` (`provenance.rs`) |

`rag-eval`: no se corrió. El ranking y las líneas de `search_memory` no cambian, y `EvalIngest` siembra las filas con `user_stated`, así que el texto de agent-view, ACTIVE CONFLICTS incluido, es el mismo que el de las líneas de base.

## Windows (2026-10-18)

Sobre `e13dca7` (0.18.1, con topic-revisions mergeado), después de `cargo clean --workspace --target-dir target-test`:

- `cargo test --workspace --target-dir target-test`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sale con 0.
- Después de la suite no queda vivo ningún `clawg.exe` hijo.

## Perfil dev con Claude Code (2026-10-18)

Con `target\debug\clawg.exe` compilado desde `e13dca7` y tres sesiones de `claude -p` (Claude Code 2.1.302) que cargan sólo ese puente (`mcp --profile dev --agent claude-code`) con `--strict-mcp-config`. El server se registró como `clawg-verify`: la config local deshabilita `clawg-dev` en este proyecto, y eso también esconde un server del mismo nombre que llega por `--mcp-config` (el puente ni arranca).

- Al conectar, el daemon escribió el snapshot `pre-migration` y aplicó V16 sobre el store dev, que estaba en V15.
- Una preferencia dicha por el usuario: el agente la guardó como `user_preference` con `user_stated: true` (#65). Un hecho que encontró leyendo `crates/clawg-cli/tests/binary.rs`: `semantic_fact`, sin `user_stated` (#66). La respuesta del segundo save nombró a #65 como cercana.
- `memory_status`: `1 for you to judge` y `#65 close to #66 inferred`, el lado sin marca primero; el par siguió `pending`.
- `synthesize` listó `#66 semantic_fact origin=inferred …`, `#65 user_preference …` sin marca, y una fila anterior del store (#2), también sin marca. Una conclusión con `derived_from = [65, 66]` (#67, `architectural_decision`) no abrió ningún par con sus fuentes; el segundo `synthesize` la listó como `#67 architectural_decision origin=conclusion from=#65,#66 …`, y #65 y #66 dejaron de estar pendientes.
- `clawg memories show`: #65 `source: explicit`, `created by: user (claude-code)`; #66 `explicit`, `agent (claude-code)`; #67 `synthesis`, `agent (claude-code)`; #2, guardada el 2026-09-13, sigue `explicit` / `user`.
- El par se juzgó `compatible`, #65 a #67 se borraron y `memory_status` quedó en `conflict candidates: none open`. El daemon dev se paró con `clawg stop`.

## Lo que encontró la verificación

El agente sumó a la preferencia declarada (#65) una oración propia, sacada de `.claude/CLAUDE.md` (`binary.rs` y `CLAWG_TEST_HOME`), y el save entero quedó sellado `user`. Lo avisó en su respuesta. El sello es por save, y `protocol.md` dice cuándo declarar (`crates/clawg-cli/src/mcp/protocol.md:75-78`), pero no que un save declarado lleve sólo lo que dijo el usuario; la regla de escribir cada save con sus rutas e identificadores empuja a agregar contexto. Queda en el backlog del README: el arreglo es una oración en el protocolo (guardar aparte, sin declarar, lo que el agente agrega).

## Al archivar

- `specs/provenance/spec.md` es nueva, con P1 a P6 y las citas en HEAD. `specs/disputes` (lo que trae `GET /v1/disputes` y la línea de `memory_status`) y `specs/synthesis` (R7) la citan.
- Las citas `path:line` de las specs y del README que apuntan a los archivos que cambiaron desde `d32e471`, el último archivo (este cambio y topic-revisions), se llevaron a HEAD por identidad de línea: 320 en 15 documentos.
- Al alinearlas aparecieron citas que ya estaban mal en `d32e471`, y se corrigieron: `read_links` en `specs/pieces`; `import_spill` en `specs/storage` y `specs/secrets`; `probe`, `open_for_probe`, `immutable_uri` y `to_prune` (`backup.rs`) en `specs/storage`; las rutas `/backup` y `/projects/docs/search` en `specs/storage` y `specs/project-docs`; la llamada a `spawn_doc_refresh` en `specs/project`; el corte `min_relative_score`, `search_request` y `reach` en `specs/retrieval`; `AccessTracker::inert` en `specs/retrieval`; y en el README `to_prune`, `AUTO_KEEP`, `record_hits` y la llamada a `project_for` de `search_project`.
- `specs/pieces` R3 nombraba `link_save`, que topic-revisions convirtió en `finish_save` (`crates/clawg-core/src/memory/worker.rs:837`, por `MemoryStore::finish_save` en `topic.rs`, que también aplica la revisión); se reescribió con el nombre y el aviso nuevos.
- El backlog del README suma el hallazgo de `user_stated`.

## Veredicto

Verificado en Linux, en Windows y con Claude Code sobre el perfil dev. `rag-eval` no se corrió: el ranking no cambia y el texto de agent-view es el mismo que el de las líneas de base. Se archivó el 2026-10-18 en [`specs/provenance`](../../../specs/provenance/spec.md).
