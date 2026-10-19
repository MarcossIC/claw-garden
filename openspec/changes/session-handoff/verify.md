# session-handoff — Verificación

## Linux (2026-10-19)

Sobre `2e2d13e` (el último commit de código), en `target`, con rustc 1.97.0:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Cada commit de las [tareas](tasks.md) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` pasa sin bendecir: `save_memory` no ganó argumentos y la frase nueva de `protocol.md` va fuera de la apertura.

## Lectura del código

| Requisito | Cómo quedó |
|---|---|
| R1 | `SessionRegistry` (`crates/clawg-cli/src/daemon/sessions.rs`) con `a_session_registry_keeps_the_latest_per_agent_and_project` y `a_session_id_is_checked`; `POST /v1/sessions` responde `204` o `400`, en `a_save_carries_the_registered_session` (`tests/api.rs`) |
| R2 | `save_memory` lee el registro antes de armar el evento y lo pone en `MemoryEvent::session_id`; `NewMemoryRecord::source_session_id` llega al `INSERT`. `a_save_writes_its_session_on_every_row` (`worker.rs`) cubre todos los pedazos de un save largo y un save sin sesión; `a_queued_save_keeps_its_session_in_the_outbox` lee el id en el evento del outbox; `an_event_without_a_session_reads_as_none` lee un evento anterior al campo; `rows_without_a_session_stay_reachable` lista y busca una fila sin sesión |
| R3 | `with_last_session` y `fetch_last_session` (`prompt_builder.rs`), en los dos armados; el autor se filtra con `Reach::admits`, el mismo criterio de la búsqueda. Tests: `the_last_session_block_shows_the_newest_summary` (otro proyecto, sin proyecto, borrado y más viejo quedan fuera; va antes de RECENT), `the_last_session_block_respects_sharing`, `the_last_session_block_is_not_repeated_as_a_hit`, `a_context_without_a_query_opens_with_the_last_session` (también uno de hace un mes) |
| R4 | `fit_to_budget` corta LONG-TERM, RECENT, LAST SESSION y recién después los conflictos; `the_last_session_block_gives_way_after_the_hits` comprueba además que el resumen no entra en `Fitted::shown`, así que no registra accesos |
| R5 | Los dos eventos de `clawg hook` registran en paralelo con lo que ya pedían e ignoran el resultado; `a_hook_registers_the_session_of_the_saves` (`tests/binary.rs`) corre el binario real: el save después de `session-start` lleva `s1` y el que sigue a un `user-prompt-submit` de otra sesión lleva `s2` |
| R6 | `context_segments` lee LAST SESSION; `the_last_session_block_counts_as_shown` |

## Pendiente

- **CI** en Linux y Windows sobre la rama mergeada.
- **`rag-eval --real --agent-view`** contra la línea base (`recall_context` 39/46 a 1.086,4 tokens de media), con el umbral de la [propuesta](proposal.md#cómo-se-mide-el-éxito): ningún caso de `recall_context` perdido y el aumento medio de tokens no mayor que el tamaño medio de un `session_summary` del corpus. Pide el modelo real y el golden set de `docs/eval/`, que no estaban en el entorno de esta verificación. Como el eval siembra sin proyecto, todo caso con un `session_summary` sin proyecto en el corpus va a mostrar el más nuevo en LAST SESSION.
- **Perfil dev con Claude Code**, «Probar a mano» del [diseño](design.md#probar-a-mano).
