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
| T10 | `save_outcome_text_names_the_replaced_memories`, `a_key_sent_to_an_older_daemon_is_reported`, `save_memory_revises_by_topic_key`, `rows_show_their_key_and_who_replaced_them` y `the_agent_contract_matches_its_file`; la clave en los hits y en `read_memory`, que agregó el perfil dev: `a_hit_shows_the_topic_key_it_is_revised_under`, `a_hit_without_a_key_is_shown_as_before`, `another_agents_hit_does_not_show_its_topic_key`, `format_save_names_the_topic_key_of_the_row`, `a_search_hit_carries_its_topic_key`, `read_save_carries_the_key_of_the_row_asked_for` y `a_search_hit_and_the_save_read_carry_the_topic_key` |

## Windows (2026-10-18)

Sobre `9a0ec3f` (0.19.0), después de `cargo clean --workspace --target-dir target-test`:

- `cargo test --workspace --target-dir target-test`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sale con 0.
- Después de la suite no queda vivo ningún `clawg.exe` hijo.

El arreglo de la clave en los hits (`b9e3a6b`) corrió la suite entera y el clippy en `target-test` antes de su commit; los dos pasan. Mientras tanto, otra sesión commiteaba en el mismo árbol (release-and-self-update), y dos corridas de `conventions.rs` fallaron en `the_signing_step_reads_the_key_password_from_stdin_without_echo`, un test de esa sesión que todavía estaba a medio escribir. Con su árbol en el estado siguiente pasa. La suite se corre otra vez sobre el bump a 0.20.0.

## Modelo real (2026-10-18)

Con el modelo cacheado en el perfil dev y `docs/eval/`. Los resultados están en `docs/eval/results/`, fuera de git. Ningún caso del golden set lleva clave, así que lo esperado era que nada se moviera:

| Corrida | Commit | Contra | Casos movidos |
|---|---|---|---|
| Ancla (`…-topic-revisions-tip.json`) | `4991ce7` | `2026-10-17-real-rank-fusion-tip.json` | 0 |
| `--agent-view` (`…-tip-agent-view.json`) | `4991ce7` | `2026-10-18-real-conflict-candidates-final-agent-view.json` | 0 |
| Ancla (`…-topic-revisions-final.json`) | `78b62ea` | `2026-10-17-real-rank-fusion-tip.json` | 0 |
| `--agent-view` (`…-final-agent-view.json`) | `78b62ea` | `2026-10-18-real-conflict-candidates-final-agent-view.json` | 0 |

`--agent-view`: `search_memory` 36/46 con 764,0 tokens de media y `recall_context` 39/46 con 1.083,7, los dos iguales a la línea de base. El diff de agent-view también cubre memory-provenance, que se archivó sin `rag-eval`. La segunda pareja de corridas mide el código con la columna `m.topic_key` agregada a `SEARCH_SQL_MEMORY` y a `LEXICAL_SCORE_SQL_PREFIX`: el ranking no cambia.

La compuerta previa de la [exploración](exploration.md#compuerta-previa) no se corrió. Iba a fijar la prioridad antes de escribir código, y el código ya estaba en `main`. Pasó, con otras mediciones, al cambio [revision-eval](../2026-10-21-revision-eval/exploration.md).

## Perfil dev con Claude Code (2026-10-18)

Con `target\debug\clawg.exe` (0.19.0) y sesiones de `claude -p` (Claude Code 2.1.302) que cargan sólo ese puente (`mcp --profile dev --agent claude-code`) con `--strict-mcp-config`, registrado como `clawg-verify`. El store dev ya estaba en V16: lo migró la verificación de memory-provenance, con su snapshot `pre-migration`.

### Antes del arreglo, sobre `9a0ec3f`

- **Sesión A.** Una decisión dicha por el usuario: el agente la guardó solo como `architectural_decision`, con `user_stated` y `topic_key: cli/export-format` (#68).
- **Sesión B, nueva, pidiendo cambiar la decisión.** `search_memory` y `read_memory` trajeron #68 sin la clave, porque ninguno de los dos la mostraba. El agente guardó la versión nueva bajo una clave inventada, `cli/export/format` (#69), y dijo que #68 «no tenía clave». No hubo revisión. Lo salvó el detector: las dos eran `architectural_decision` a menos de 0,40, el par escaló (`asks the user`), y el agente lo cerró con `supersedes` porque el usuario había dicho «en vez de».
- **Sesión C, con pasos explícitos.** Un save bajo `cli/export/format` respondió `saved #70; replaces #69 "clawg export output format: a SQLite file…"`. `list_memories` mostró `#70 … active topic=cli/export/format`, `#69 … superseded by=#70 topic=cli/export/format` y `#68 … superseded by=#69 topic=cli/export-format`. Se borró #70, se guardó #71 con la misma clave (no reemplazó nada, porque no quedaba ninguna viva) y `restore_memory` de #70 respondió `409: memory #70 was saved under topic cli/export/format, which #71 holds now; #70 stays deleted so the topic has one current memory`.

### Después del arreglo, sobre `cb9a744`

- **Sesión A2.** Otra decisión: `architectural_decision`, `user_stated`, `topic_key: cli/tui/library` (#72).
- **Sesión B2, nueva.** El hit decía `#72 [Memory/architectural_decision/t2 score=0.26 topic=cli/tui/library]` y `read_memory` abría con `#72 topic=cli/tui/library`. El agente guardó bajo esa clave, y la respuesta fue `saved #73; replaces #72 "Terminal viewer `clawg tui`: built with ratatui and crossterm…"`.

Las memorias de prueba (#68 a #73) se borraron. `GET /v1/disputes` quedó en `{"pending":0,"escalated":0,"pairs":[]}`, en el store sólo siguen vivas #1 y #2, y el daemon dev se paró con `clawg stop`.

## Lo que encontró la verificación

**Un agente en otra sesión no veía la clave.** `format_hits` y `format_save` (`crates/clawg-cli/src/mcp/tools.rs`) no la mostraban; sólo `list_memories` lo hacía. Una decisión cambia casi siempre en una sesión posterior, así que el uso principal de la clave dependía de que el agente adivinara el mismo texto. Se arregló antes de archivar (`b9e3a6b`, `cb9a744`):

- `RankedHit` y `SaveText` llevan `topic_key`, que se serializa sólo cuando existe, y `POST /v1/search` y `GET /v1/memories/{kind}/{id}/save` lo devuelven.
- El puente muestra ` topic=K` en cada hit propio y en la cabecera de `read_memory`. En un hit de otro agente no la muestra: un save con esa clave sólo revisa filas propias. `read_memory` lee siempre filas del lector (`read_memory_impl` pasa su `agent_id`), así que no necesita esa regla.
- `protocol.md`, fuera de la apertura, dice que una versión nueva se guarda bajo la clave que se ve.
- El contrato del agente no cambió.

`recall_context` sigue sin mostrar la clave, porque es un prompt con presupuesto y cada byte se mide. Queda en revision-eval (M3, M4), junto con cuántas veces un agente reusa la clave en muchas sesiones: acá hay una sola corrida por lado.

## Diferencias con la propuesta y la spec

- **T6.** La spec decía «otro save con alguna fila fuera del alcance». Quedó «un save cuya primera pieza es una fila viva sin esa clave»: mirar todas las filas desenganchaba la versión vieja entera cuando la dedup le había sumado, en el medio, una fila de otra clave. La spec y el diseño se corrigieron antes del commit del núcleo.
- **T10.** El aviso de daemon anterior sale sólo en un save confirmado (`201`). Un `202` no promete ningún reemplazo, y su texto no cambia. La clave en los hits y en `read_memory` no estaba en la propuesta; entró por el hallazgo del perfil dev, y la spec la suma a T10.
- **`link_pieces`** quedó sólo para tests: el worker termina todo save por `finish_save`, que enlaza y revisa en una transacción.

## Veredicto

Verificado en Linux, en Windows, con el modelo real y con Claude Code sobre el perfil dev, donde una sesión nueva revisa una decisión por su clave. Se archivó el 2026-10-18.
