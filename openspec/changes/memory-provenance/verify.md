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

## Pendiente

- Windows: la suite y el clippy en `target-test`.
- Perfil dev con Claude Code: un save declarado, uno sin declarar y una conclusión, vistos en `clawg memories show` y en `memory_status`.
