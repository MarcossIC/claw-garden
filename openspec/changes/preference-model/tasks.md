# preference-model — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Cada tarea es un commit convencional y se comprueba con los tests de los archivos que cambia. La suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencias a planes, fases, cambios ni números de tarea.

Los requisitos están en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

## 0. Antes del código

- [x] El problema de la exploración, verificado otra vez sobre `4242171`.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`; las decisiones marcadas como cerradas en la exploración.
- Commit: `docs(openspec): propose preference-model`

## 1. Sólo entra sola una preferencia declarada (PM2)

- [ ] Test en rojo: `only_a_preference_the_user_stated_enters_the_workspace` (`workspace.rs`).
- [ ] La consulta de las preferencias pide `created_by_type = 'user'`. `seed_preference` (`prompt_builder.rs`) siembra una preferencia declarada, la que imprime el workspace.
- Commit: `fix(workspace): leave the preferences the user did not state out of the workspace`

## 2. El proyecto de una preferencia (PM1)

- [ ] Tests en rojo: `a_preference_of_another_project_stays_out_of_the_workspace`, en lugar de `workspace_preferences_cross_project_boundaries`, y `an_unscoped_workspace_takes_the_preferences_of_every_project` (`workspace.rs`).
- [ ] Test en rojo de PM7: `a_preference_out_of_the_workspace_is_still_found` (`prompt_builder.rs`), con filas sembradas como las escribe la versión anterior: la preferencia declarada de otro proyecto todavía entra al workspace.
- [ ] La consulta de las preferencias pide el proyecto o ninguno cuando hay proyecto, como la de las decisiones; el comentario dice por qué.
- Commit: `fix(workspace): show a preference only in its project, or in every one when it has none`

## 3. Las preferencias siguen el alcance del lector (PM3)

- [ ] Tests en rojo: `the_workspace_takes_the_preferences_the_reader_reaches` (`workspace.rs`) y `a_stated_preference_reaches_the_session_start_of_another_agent` (`crates/clawg-cli/tests/api.rs`).
- [ ] `regenerate_auto_section` recibe el `Reach`; la consulta de las preferencias usa `reach_predicate!`; el daemon pasa el alcance del lector. `the_workspace_stays_own_and_conflicts_are_shared` fija lo propio con una decisión.
- Commit: `feat(workspace): show the preferences of every agent the reader reaches`

## 4. La marca de una preferencia en los hits (PM5)

- [ ] Tests en rojo: `a_preference_hit_carries_its_origin` (`search.rs`), `a_preference_hit_line_shows_its_origin` (`prompt_builder.rs`), `format_hits_shows_the_origin_of_a_preference` (`tools.rs`) y `a_hit_from_an_old_daemon_has_no_origin` (`client.rs`).
- [ ] `a_preference_out_of_the_workspace_is_still_found` pide además que el hit inferido llegue marcado.
- [ ] `RankedHit.origin` desde `source` y `created_by_type` en las consultas de `memory`; `Hit.origin` en el cliente; la marca en `format_hit_line` y `format_hits`.
- Commit: `feat(search): mark a preference hit the user did not state`

## 5. `save_memory` con `all_projects` y el protocolo (PM4, PM6)

- [ ] Tests en rojo: `a_save_for_every_project_has_no_project` y `all_projects_goes_without_a_project` (`server.rs`), y `the_protocol_says_which_preferences_open_every_session` (`protocol.rs`).
- [ ] `SaveMemoryArgs.all_projects`, su descripción en inglés y en `tools_es.rs`, y la negativa con otro lugar. `protocol.md` y `protocol.es.md`, fuera de la apertura.
- [ ] `CLAWG_BLESS=1` para los dos contratos, el diff revisado (sólo `all_projects` de `save_memory`), y los tests de nuevo.
- Commit: `feat(mcp): save a memory for every project with all_projects`

## 6. Docs

- [ ] `docs/reference/daemon.md`: qué preferencias entran al workspace, el `origin` de un hit y `all_projects`.
- Commit: `docs(reference): say which preferences the workspace shows and how a hit marks one`

## 7. Verificación

- [ ] La suite, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo clippy -p clawg-cli -- -D warnings` en Linux, sobre el último commit ([verify](verify.md)).
- [ ] El CI del pull request en Linux y Windows; lo que marque se arregla.
- [ ] `verify.md`, con lo que se mide contra `prod` como pendiente en local; `state.yaml`; la línea del cambio en `ROADMAP.md`.
- Commit: `docs(openspec): record preference-model applied`

## 8. Archivo (fuera de este cambio)

- [ ] Llevar PM1 a PM3 a `specs/retrieval` y R5 de `specs/sharing`, PM5 a `specs/provenance` y `specs/retrieval`, PM4 y PM6 a `specs/mcp`, PM7 a `specs/retrieval`; mover la carpeta a `changes/archive/`; subir el minor.
