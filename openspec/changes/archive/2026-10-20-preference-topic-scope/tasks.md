# preference-topic-scope — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El hit y el save leído dicen su proyecto (R1)

- [x] Tests primero: `a_search_hit_and_the_save_read_carry_the_project` (`tests/api.rs`). Falló por la aserción: el hit no traía `project_id`.
- [x] `RankedHit.project_id` y `SaveText.project_id`, leídos en las consultas de búsqueda y en `read_save_core`, mandados siempre.
- Commit: `feat(core): say which project a search hit and a read save belong to`

## 2. La clave sólo donde revisa (R2, R3)

- [x] Tests primero: `a_hit_tells_a_missing_project_from_none` (`client.rs`); `a_hit_of_another_project_does_not_show_its_topic_key`, `a_hit_without_a_project_shows_its_key_only_to_a_bridge_without_one`, `a_hit_from_an_older_daemon_shows_its_key` y `format_save_shows_the_key_only_where_a_save_revises_it` (`tools.rs`); `search_memory_shows_a_key_only_in_the_project_it_revises`, `read_memory_shows_a_key_only_in_the_project_it_revises` y `the_protocol_says_where_a_key_is_shown` (`server.rs`). Fallaron por no compilar (las firmas nuevas de `format_search`/`format_hits`/`format_save` y el campo `project_id`); con la regla anterior (`revised_from` siempre verdadero) fallan por la aserción los de `tools.rs` (salvo `a_hit_from_an_older_daemon_shows_its_key`, que fija lo que no cambia) y los dos del puente, y `the_protocol_says_where_a_key_is_shown` falló antes de cambiar `protocol.md`.
- [x] `Hit.project_id` y `SavedText.project_id` (ausente o `null`), la regla pura, `format_search`/`format_hits` y `format_save` con el proyecto del save, `search_memory_impl` y `read_memory_impl`, y la línea de `protocol.md`.
- Commit: `fix(mcp): show a topic key only where a save under it revises the memory`

## 3. Docs

- [x] `.claude/CLAUDE.md` y estas tareas.
- Commit: `docs: say where the bridge shows a topic key`

## 4. Verificación

- [x] Suite y clippy en Linux sobre el último commit, y cada commit de código por separado ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.

## 5. Archive

- [x] `specs/mcp` con R1-R3 y las citas realineadas (y la de `specs/pieces`); `git mv` a `changes/archive/2026-10-20-preference-topic-scope`; `state.yaml` archivado; fila 67, el orden de ejecución y las citas del `README.md` de openspec; subir el minor.
