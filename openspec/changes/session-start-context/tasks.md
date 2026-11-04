# session-start-context — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Para ver fallar por la aserción a los tests del recorte, primero se agrega lo que necesitan para compilar, sin conducta. Cada tarea es un commit convencional y se comprueba con los tests de los archivos que cambia. La suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencias a planes, fases, cambios ni números de tarea.

Los requisitos están en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

## 0. Antes del código

- [x] El problema de la exploración, verificado otra vez sobre `592a6e0`.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`.
- Commit: `docs(openspec): propose session-start-context`

## 1. El aviso no cuenta lo importado (SC6)

- [ ] Test en rojo: `the_reminder_leaves_out_what_an_import_wrote` (`synthesis.rs`).
- [ ] `unreviewed_count` cuenta las pendientes con `source <> 'import'`; `synthesis_core` no cambia.
- Commit: `fix(synthesis): leave what an import wrote out of the reminder`

## 2. La regeneración informa cada línea (SC1)

- [ ] Test en rojo: `the_regeneration_reports_each_line_it_wrote` (`workspace.rs`).
- [ ] `EmittedIds` guarda, de cada fila impresa, su kind, su id, su importancia, su `created_at` y su línea. `fetch_auto_rows` lee `importance` y `created_at` sin cambiar sus condiciones. `render_rows` y el informe arman la línea con la misma función.
- Commit: `refactor(workspace): report the line, importance and age of each row the AUTO section prints`

## 3. El recorte de la sección AUTO (SC2 a SC5)

- [ ] El workspace del prompt pasa a una estructura con la parte de la persona y las entradas de AUTO. Se arma con el archivo y el informe de la regeneración, y sin recorte se imprime igual que hoy. `PromptBlock::Workspace` existe, sin conducta todavía.
- [ ] Tests en rojo, que fallan por la aserción: `a_stocked_workspace_leaves_room_for_the_last_session`, `the_auto_rows_over_half_go_before_the_hits`, `with_nothing_waiting_the_auto_rows_take_the_whole_budget`, `the_auto_rows_go_by_importance_then_newest` y `what_the_user_wrote_is_never_cut`; también `the_fit_names_each_block_it_cut`, con el bloque `workspace`, y la frase nueva en `a_workspace_over_the_budget_is_kept_and_said` y `the_notes_of_the_prompt_follow_its_language`.
- [ ] `fit_to_budget` corta primero la sección AUTO que pasa de la mitad mientras hay algo esperando, y al final las entradas que quedan. Nombra lo que cortó y dice la frase nueva.
- Commit: `fix(context): cut the workspace's AUTO rows so the last session fits the session start`

## 4. Docs

- [ ] `docs/reference/daemon.md`: el recorte de AUTO y el aviso sin lo importado. `docs/reference/data-profiles.md`: el bloque `workspace` de `context.cut`.
- Commit: `docs(reference): say the context cuts the AUTO rows and the reminder leaves imports out`

## 5. Verificación

- [ ] La suite y `cargo clippy --workspace --all-targets -- -D warnings` en Linux, sobre el último commit ([verify](verify.md)).
- [ ] El CI del pull request en verde en Linux y Windows.
- [ ] `verify.md`, `state.yaml`, la fila 114 del `README.md` de openspec y la línea del cambio en `ROADMAP.md`.
- Commit: `docs(openspec): record session-start-context applied`

## 6. Archivo (fuera de este cambio)

- [ ] Llevar SC1 a SC5 a `specs/retrieval`, SC6 a `specs/synthesis` y el bloque `workspace` a `specs/metrics`; mover la carpeta a `changes/archive/`; subir el minor.
