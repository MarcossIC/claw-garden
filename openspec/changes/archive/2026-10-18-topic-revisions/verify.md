# topic-revisions — Verificación

## Linux (2026-10-18)

Sobre `c0382b8`, en `target`:

- `cargo test --workspace`: la suite pasa. Cada commit del cambio la corrió entera.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: termina con código 0.
- `cargo build -p clawg-cli`, sin la feature `eval`, da el mismo warning que ya traía `880aa07` (`ImportanceFactor::spanning`, `crates/clawg-core/src/memory/score.rs:88`). No viene de este cambio.
- El contrato del agente se bendijo una vez, a propósito. El diff se revisó a mano: sólo entra `topic_key` (string o null) en el schema de `save_memory`, con su descripción. Las instrucciones no cambian: la línea nueva de `protocol.md` va en «How to write a save», fuera de la apertura.
- Dos mutaciones a mano confirmaron que los tests muerden: sin sumar las filas de la clave a `siblings` falla `a_revision_does_not_pair_with_the_row_it_replaces`; sin la regla de la pieza compartida falla `a_piece_shared_with_another_save_is_detached`.

| Requisito | Cómo quedó |
|---|---|
| T1 | `a_database_from_before_topic_keys_opens_unchanged` (base V15 con filas, una `superseded`, piezas, un par y el índice de texto) y el checksum de V16 en `published_migration_checksums_are_pinned` |
| T2 | `topic_key_cases` y `the_error_names_the_field` |
| T3 | `a_save_refuses_an_invalid_topic_key` y `a_save_echoes_its_topic_key` en `tests/api.rs`; `an_event_without_a_topic_key_still_parses` |
| T4 | `a_keyed_save_replaces_the_live_row_of_its_key` (también por KNN y texto), `a_save_without_a_key_replaces_nothing`, `the_same_text_under_its_key_only_reinforces`, `saving_an_earlier_text_again_is_another_revision`, `a_revision_inherits_tier_and_accesses`, `a_revision_releases_the_rows_disputing_the_old_one`, `a_failed_revision_replaces_nothing`, `a_reinforced_row_without_a_key_takes_it` y `a_revision_is_finished_with_its_outbox_row` |
| T5 | `a_revision_keeps_the_type_and_importance_it_does_not_name` |
| T6 | `a_long_revision_replaces_every_piece_of_the_old_save` y `a_piece_shared_with_another_save_is_detached` |
| T7 | `a_revision_does_not_pair_with_the_row_it_replaces` |
| T8 | `a_key_is_scoped_to_its_agent_and_project` y `two_queued_revisions_apply_in_order` |
| T9 | `a_row_whose_key_is_live_again_is_not_restored` en `admin.rs` y `restore_refuses_a_row_whose_topic_is_live_again` en `tests/api.rs` |
| T10 | `save_outcome_text_names_the_replaced_memories`, `a_key_sent_to_an_older_daemon_is_reported`, `save_memory_revises_by_topic_key`, `rows_show_their_key_and_who_replaced_them` y `the_agent_contract_matches_its_file` |

## Diferencias con la propuesta y la spec

- **T6.** La spec decía «otro save con alguna fila fuera del alcance». Quedó «un save cuya primera pieza es una fila viva sin esa clave»: mirar todas las filas desenganchaba la versión vieja entera cuando la dedup le había sumado, en el medio, una fila de otra clave. La spec y el diseño se corrigieron antes del commit del núcleo.
- **T10.** El aviso de daemon anterior sale sólo en un save confirmado (`201`). Un `202` no promete ningún reemplazo, y su texto no cambia.
- **`link_pieces`** quedó sólo para tests: el worker termina todo save por `finish_save`, que enlaza y revisa en una transacción.

## Pendiente

- Windows, en `target-test`, después de `cargo clean --workspace --target-dir target-test`.
- Con el modelo real y `docs/eval/`: `rag-eval --real --diff` contra `2026-10-17-real-rank-fusion-tip.json`, que debe dar cero casos movidos porque ningún caso lleva clave, y la compuerta previa de la [exploración](exploration.md#compuerta-previa).
- Perfil dev con Claude Code: V16 sobre el store dev (snapshot `pre-migration`), una decisión revisada por clave, la respuesta con `replaces`, `list_memories` con `by=` y `topic=`, y `restore_memory` rechazado por la clave.
