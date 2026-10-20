# untrusted-recall — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. Margen en el prompt de contexto (R1, D1)

- [x] Tests primero: `indent_continuations_keeps_every_line_behind_a_margin`, `a_saved_text_cannot_open_a_block_of_the_prompt`, `a_saved_text_cannot_imitate_a_hit_header`, `a_session_summary_keeps_its_lines_behind_the_margin`, `a_conflict_side_cannot_open_a_new_or_old_line`.
- [x] `indent_continuations` en `format_hit_line` y en los lados de ACTIVE CONFLICTS.
- Commit: `feat(core): keep a saved text's lines behind a margin in the context prompt`

## 2. El autor saneado (R2, D2)

- [x] Tests primero: `display_author_drops_what_could_break_a_line`, `a_hit_by_an_odd_name_stays_on_its_line`, `search_memory_shows_an_odd_author_on_its_line`.
- [x] `display_author` en la cabecera del hit y en `search_memory`.
- Commit: `feat(core): show an agent's name without what could break its line`

## 3. El workspace (R3, D3)

- [x] Tests primero: `a_copied_text_cannot_move_the_user_section`, `a_copied_text_keeps_its_lines_behind_the_margin`, `sections_holding_finds_the_copied_form`.
- [x] `render_rows` y `format_skill_line` con margen y marcadores desarmados; `sections_holding` con las dos formas.
- Commit: `feat(core): keep a copied text from moving the sections of workspace.md`

## 4. La raíz del puente (R4, D4)

- [x] Tests primero: `auto_indexable_wants_a_project_that_is_not_home`, `a_git_or_a_manifest_marks_a_project`, `a_bridge_in_a_folder_that_is_not_a_project_indexes_nothing`.
- [x] `auto_indexable` y `has_project_marker`; el puente las usa; la frase de `not_accepted`.
- Commit: `feat(mcp): index the bridge's root on its own only when it is a project`

## 5. Docs

- [x] `.claude/CLAUDE.md` y estas tareas.
- Commit: `docs: say how saved text is shown and which root a bridge indexes`

## 6. Verificación

- [ ] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] `rag-eval --real --agent-view` contra la línea base (necesita el modelo real y el golden set de `docs/eval/`).
- [ ] El CI en verde en Linux y Windows.

## 7. Archive

- [ ] `specs/retrieval`, `specs/sharing`, `specs/project-docs`; `git mv` a `changes/archive/`; `state.yaml`; fila 52, `Ideas evaluadas` y el orden de ejecución del `README.md` de openspec; subir el minor.
