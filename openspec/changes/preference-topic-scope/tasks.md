# preference-topic-scope — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El hit y el save leído dicen su proyecto (R1)

- [ ] Tests primero: `a_search_hit_and_the_save_read_carry_the_project` (`tests/api.rs`).
- [ ] `RankedHit.project_id` y `SaveText.project_id`, leídos en las consultas de búsqueda y en `read_save_core`, mandados siempre.
- Commit: `feat(core): say which project a search hit and a read save belong to`

## 2. La clave sólo donde revisa (R2, R3)

- [ ] Tests primero: `a_hit_tells_a_missing_project_from_none` (`client.rs`); `a_hit_of_another_project_does_not_show_its_topic_key`, `a_hit_without_a_project_shows_its_key_only_to_a_bridge_without_one`, `a_hit_from_an_older_daemon_shows_its_key` y `format_save_shows_the_key_only_where_a_save_revises_it` (`tools.rs`); `search_memory_shows_a_key_only_in_the_project_it_revises`, `read_memory_shows_a_key_only_in_the_project_it_revises` y `the_protocol_says_where_a_key_is_shown` (`server.rs`).
- [ ] `Hit.project_id` y `SavedText.project_id` (ausente o `null`), la regla pura, `format_search`/`format_hits` y `format_save` con el proyecto del save, `search_memory_impl` y `read_memory_impl`, y la línea de `protocol.md`.
- Commit: `fix(mcp): show a topic key only where a save under it revises the memory`

## 3. Docs

- [ ] `.claude/CLAUDE.md` y estas tareas.
- Commit: `docs: say where the bridge shows a topic key`

## 4. Verificación

- [ ] Suite y clippy en Linux sobre el último commit, y cada commit de código por separado ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.

## 5. Archive

- [ ] `specs/mcp` con R1-R3 y las citas realineadas (y la de `specs/pieces`); `git mv` a `changes/archive/2026-10-20-preference-topic-scope`; `state.yaml` archivado; fila 67 y el orden de ejecución del `README.md` de openspec; subir el minor.
