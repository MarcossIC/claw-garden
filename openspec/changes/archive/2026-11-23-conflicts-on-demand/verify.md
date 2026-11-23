# conflicts-on-demand — Verificación

Implementado del 2026-11-23 en `develop`, de `efb94bea` a `fd111177`, desde `proposal.md`, `spec.md` (CO1 a CO13) y `tasks.md`, sin `design.md`.

## Qué corrió

- `cargo test --workspace --target-dir target-test`, sobre `ed573859`: los 25 binarios de test en verde (`--no-fail-fast`). La primera corrida paró en `no_source_allows_dead_code`: un `#[expect(dead_code)]` en un test de `client.rs`; lo quitó `23a36f1e`.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings`: limpio después de `ed573859`, que cambió un `format!` inútil de un test.
- Después de la revisión, los tests de cada archivo que cambiaron los arreglos (`prompt_builder`, `dispute`, `db`, `agent_view` de `rag-eval`, `a_context_records_its_state_its_size_and_the_blocks_it_cut`).
- Los contratos se regeneraron con `CLAWG_BLESS=1` dos veces (tareas 11 y 12). Su diff: el parámetro `conflicts` y la frase de `list_memories`; la frase de ACTIVE CONFLICTS de las instrucciones; las descripciones de `resolve_conflict` y de su `keep`; y la frase de `synthesize` (abajo).
- Largos, con la función `opening` de `protocol.rs`: la frase nueva en inglés mide 205 caracteres; las aperturas, 2.028 sin preferencia, 2.045 con `en` y 1.998 con `es`.

## Requisito → test

| Requisito | Tests |
|---|---|
| CO1 | `a_context_without_a_query_marks_its_disputed_rows_and_prints_no_pairs`, `a_context_without_a_query_or_pairs_is_the_same_as_before` (`prompt_builder.rs`) |
| CO2 | `the_conflict_mark_names_up_to_three_other_sides_in_order`, `a_hit_line_carries_its_conflict_mark_before_the_author`, `a_recent_save_and_a_workspace_entry_in_dispute_carry_each_others_id` (también que `workspace.md` no cambia), `a_row_whose_other_side_is_hidden_carries_no_mark`, `every_hit_of_a_capped_block_keeps_its_mark`, `a_malformed_dispute_list_hides_no_other_pair` |
| CO3 | `the_conflict_pairs_give_way_to_the_last_session`, `the_hits_in_their_room_outlast_the_last_session_and_the_conflicts`, `the_last_session_block_gives_way_after_the_hits`, `the_fit_names_each_block_it_cut` |
| CO4 | `a_mature_store_starts_its_session_with_the_whole_last_summary` |
| CO5 | `a_search_names_the_other_side_on_both_hits_of_an_escalated_pair`, `a_search_names_the_other_side_of_a_lone_hit`, `a_search_names_no_pending_deleted_or_hidden_side` (`tests/api.rs`), `ranked_hit_serializes_to_json` (`public_api.rs`) |
| CO6 | `format_hits_shows_the_conflict_of_a_hit` (`tools.rs`), `a_hit_from_a_daemon_without_conflicts_has_none` (`client.rs`), `search_memory_marks_both_sides_of_an_escalated_pair` (`server.rs`) |
| CO7 | `escalated_pairs_list_the_oldest_with_the_first_line_of_each_side`, `escalated_pairs_keep_the_reach_and_the_project` (`dispute.rs`), `disputes_list_ten_escalated_pairs_with_both_lines_on_request`, `disputes_without_the_flag_list_the_pending_pairs_as_before`, `escalated_disputes_keep_the_reach_and_the_project` (`tests/api.rs`) |
| CO8 | los cinco `list_memories_*conflicts*` de `server.rs`; `the_escalated_pairs_show_both_sides_and_leave_the_verdict_to_the_user`, `the_escalated_pairs_count_the_ones_left_out`, `no_escalated_pairs_say_none_open` (`tools.rs`); `a_daemon_that_predates_the_escalated_listing_refuses_its_query` (`client.rs`) |
| CO9 | `no_opening_waits_for_a_conflict_block_to_ask` (`protocol.rs`), `the_instructions_fit_older_clients`, los dos contratos |
| CO10 | `a_pair_left_to_the_user_says_where_it_is_listed`, `format_status_ends_with_the_open_pairs` (`tools.rs`), `memory_status_counts_only_the_pairs_that_wait_for_the_user` (`server.rs`) |
| CO11 | `a_context_records_its_state_its_size_and_the_blocks_it_cut` (`tests/api.rs`) |
| CO12 | `a_context_with_a_query_keeps_the_block_and_marks_its_hits` y los de antes del bloque, que siguen |
| CO13 | `a_waiting_pair_keeps_its_state_while_the_context_marks_it` (`tests/api.rs`) |

## Desvíos

- **La frase de `synthesize`.** «When it says memories are waiting…» se refería a `recall_context`, que la frase nueva de CO9 ya no nombra; pasó a «When told memories are waiting…» («Cuando se te diga que hay memorias…»), que cabe en el largo. CO9 sólo admitía su propia frase en el diff de los contratos: queda dicho en CO9 de `specs/mcp`.
- **CO1 y la consulta de pares.** El arranque sigue leyendo los pares de lo que muestra, sólo para la marca de CO2; no los imprime. La letra de CO1 en `specs/retrieval` lo dice así.
- **Tests que la tarea nombraba y no cambiaron.** `e2e_full_4_block_output` no tiene un par en el store, así que su línea de hit no lleva marca; la marca con consulta la prueba `a_context_with_a_query_keeps_the_block_and_marks_its_hits`. `the_instructions_carry_the_saving_habits` y su versión en español siguen pasando con el texto nuevo (las frases que fijan están en él); la frase nueva la fija `no_opening_waits_for_a_conflict_block_to_ask`. Que `workspace.md` no lleva la marca se prueba en `prompt_builder.rs`, donde se arma el prompt, y no en `workspace.rs`.
- **`README.md`.** Las líneas de `list_memories` y `resolve_conflict` de la lista de herramientas se actualizaron con la documentación (tarea 14), aunque la tarea no lo nombraba.
- **Dos conductas que el plan no pedía, de la revisión.** Una fila con un `disputed_against` que no es JSON válido ya no apaga las marcas ni el bloque de las demás (`fd111177`, el chequeo va dentro del `json_each`); y `new_line`/`old_line` son la primera línea con texto, sin espacios en los bordes. Las dos quedan en `specs/retrieval` (CO2) y `specs/disputes` (CO7).
- **El parser de `rag-eval --agent-view`** no entendía la marca y habría contado como no vista cada fila marcada; lo arregla `466a31ac` con su test.
- **La serie no es atómica en dos puntos:** `91b26ba3` dejó fallando `the_viewer_is_not_exposed_to_the_model` hasta `1e370d2f`, y `dbd82a88` dejó un `#[expect(dead_code)]` que rechaza `tests/conventions.rs` hasta `23a36f1e`. El historial no se reescribe.

## Revisión

Dos revisores, en paralelo.

- Ronda 1: WARNINGS y BLOCKED. Los dos marcaron el parser de `rag-eval`, la frase de `synthesize`, `PAIR_LINE_LIMIT` público y la serie no atómica; uno, una aserción de `context.cut` que se había perdido, el `json_valid` que faltaba en `fetch_active_conflicts`, `first_line` sobre una línea en blanco y comentarios viejos. Todo se arregló o quedó escrito arriba.
- Ronda 2: WARNINGS en los dos, sin defectos de código. Se tomó la sugerencia de poner el chequeo del JSON dentro del `json_each` (`fd111177`).
- Quedan anotados, sin cambio: `is_refused_query` lee cualquier `400` de `/v1/disputes` como un daemon anterior (hoy es el único `400` posible de esa ruta); la marca sale de `disputed_against` sin filtro de proyecto, y la lista de `list_memories` sale de `memory_conflict` con el proyecto de la fila nueva, así que una fila puede llevar `conflict=#N` por un par que la lista del proyecto abierto no trae; `search_memory_marks_both_sides_of_an_escalated_pair` mira la línea del lado nuevo.

## Pendiente en local

Con la versión publicada e instalada, sobre el perfil real y con autorización:

- `rag-eval --real --agent-view` con el golden de `docs/eval/` contra la última línea base: `search.found` y `context.found` sin moverse («no distinguible» con menos de 6 casos en un sentido), y cuántos tokens suma la marca.
- Una semana de uso: `context.cut{block=conflicts,state=recent}` en 0, y `context.cut{block=last_session,state=recent}` en el 2 % o menos de los contextos con `context.retrieval_state{state=recent}` (antes, 134 de 250, los dos tipos juntos).
- `clawg setup claude-code`, para que un bloque `full` instalado tome la frase nueva.
