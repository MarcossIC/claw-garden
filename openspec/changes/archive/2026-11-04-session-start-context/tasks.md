# session-start-context — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Para ver fallar por la aserción a los tests del recorte, primero se agrega lo que necesitan para compilar, sin conducta. Cada tarea es un commit convencional y se comprueba con los tests de los archivos que cambia. La suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencias a planes, fases, cambios ni números de tarea.

Los requisitos están en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

## Frente a lo previsto

| Punto | Previsto | Hecho | Por qué |
|---|---|---|---|
| El informe de la regeneración (SC1) | un commit propio, antes del recorte | en el commit del recorte (tarea 2) | Hasta que el prompt lo lee, la importancia, la fecha y la línea de cada fila sólo las leería un test. El build las marca como código muerto, y los warnings son errores |
| El mensaje del commit del recorte | `… so the last session fits the session start` | `… so the last session fits` | Más corto; el cuerpo dice que vale para el arranque y para el contexto con consulta |

## 0. Antes del código

- [x] El problema de la exploración, verificado otra vez sobre `592a6e0`.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`.
- Commit: `docs(openspec): propose session-start-context`

## 1. El aviso no cuenta lo importado (SC6)

- [x] Test en rojo: `the_reminder_leaves_out_what_an_import_wrote` (`synthesis.rs`): el aviso daba 28.
- [x] `unreviewed_count` cuenta las pendientes con `source <> 'import'` (`unreviewed_condition`); `synthesis_core` no cambia.
- Commit: `fix(synthesis): leave what an import wrote out of the reminder`

## 2. La regeneración informa cada línea, y el recorte de la sección AUTO (SC1 a SC5)

- [x] Test en rojo: `the_regeneration_reports_each_line_it_wrote` (`workspace.rs`), escrito antes del informe.
- [x] `EmittedIds` guarda, de cada fila impresa, su kind, su id, su importancia, su `created_at` y su línea (`EmittedRow`). `fetch_auto_rows` lee `importance` y `created_at` sin cambiar sus condiciones. `render_rows` y el informe arman la línea con las mismas funciones (`memory_line`, `format_skill_line`).
- [x] El workspace del prompt pasa a `PromptWorkspace`, con la parte de la persona y las entradas de AUTO (`AutoEntry`). `workspace_for_prompt` lo arma con el archivo y el informe, y sin recorte se imprime igual que antes: los tests del prompt pasaron sin cambios. `PromptBlock::Workspace` existe, sin conducta todavía.
- [x] Tests en rojo, que fallaron por la aserción: `a_stocked_workspace_leaves_room_for_the_last_session`, `the_auto_rows_over_half_go_before_the_hits`, `with_nothing_waiting_the_auto_rows_take_the_whole_budget`, `the_auto_rows_go_by_importance_then_newest` y `what_the_user_wrote_is_never_cut`; también `the_fit_names_each_block_it_cut`, con el bloque `workspace`, y la frase nueva en `a_workspace_over_the_budget_is_kept_and_said` y `the_notes_of_the_prompt_follow_its_language`.
- [x] `fit_to_budget` corta primero la sección AUTO que pasa de la mitad mientras hay algo esperando (`AUTO_ROOM_DIVISOR`), y al final las entradas que quedan (`PromptWorkspace::next_to_cut`, `cut_auto`). Nombra lo que cortó y dice la frase nueva.
- Commit: `fix(context): cut the workspace's AUTO rows so the last session fits`

## 3. Docs

- [x] `docs/reference/daemon.md`: el recorte de AUTO y el aviso sin lo importado. `docs/reference/data-profiles.md`: el bloque `workspace` de `context.cut`.
- Commit: `docs(reference): say the context cuts the AUTO rows and the reminder leaves imports out`

## 4. Verificación

- [x] La suite, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo clippy -p clawg-cli -- -D warnings` en Linux, sobre el último commit ([verify](verify.md)).
- [x] El CI de #129 en verde en Linux y Windows, sobre `26c52fb`.
- [x] `verify.md`, `state.yaml`, la fila 114 del `README.md` de openspec y la línea del cambio en `ROADMAP.md`.
- Commit: `docs(openspec): record session-start-context applied`

## 5. Archivo (fuera de este cambio)

- [x] Llevar SC1 a SC5 a `specs/retrieval`, SC6 a `specs/synthesis` y el bloque `workspace` a `specs/metrics`; mover la carpeta a `changes/archive/`; subir el minor, a 0.77.0.
- Commits: `docs(openspec): archive session-start-context and fold it into the specs` y `chore: bump version to 0.77.0`
