# preference-model — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Cada tarea es un commit convencional y se comprueba con los tests de los archivos que cambia. La suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencias a planes, fases, cambios ni números de tarea.

Los requisitos están en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

## Frente a lo previsto

| Punto | Previsto | Hecho | Por qué |
|---|---|---|---|
| El parser de `rag-eval --agent-view` | no estaba en el alcance | en el commit de la marca (tarea 4), con `a_marked_preference_is_one_segment` | Leía el encabezado de un hit sólo con el tipo de una palabra, así que una línea marcada no contaba como hit |
| La marca en RECENT | la spec hablaba del hit de la búsqueda | también en RECENT del contexto sin consulta, y la spec lo dice; su test, `a_recent_preference_shows_its_origin`, va en un commit propio después de los docs | Es la misma línea de hit (`format_hit_line`), y una preferencia inferida de los últimos siete días llega por ahí sin buscar. El test se vio fallar con RECENT sin el origen |
| Las marcas en el protocolo | entre backticks | sin backticks | `the_protocol_names_only_what_exists` sólo admite entre backticks herramientas, argumentos y tipos; el protocolo ya las escribía así en «How to write a save» |

## 0. Antes del código

- [x] El problema de la exploración, verificado otra vez sobre `4242171`.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`; las decisiones marcadas como cerradas en la exploración.
- Commit: `docs(openspec): propose preference-model`

## 1. Sólo entra sola una preferencia declarada (PM2)

- [x] Test en rojo: `only_a_preference_the_user_stated_enters_the_workspace` (`workspace.rs`).
- [x] La consulta de las preferencias pide `created_by_type = 'user'`. `seed_preference` (`prompt_builder.rs`) siembra una preferencia declarada, la que imprime el workspace.
- Commit: `fix(workspace): leave the preferences the user did not state out of the workspace`

## 2. El proyecto de una preferencia (PM1)

- [x] Tests en rojo: `a_preference_of_another_project_stays_out_of_the_workspace`, en lugar de `workspace_preferences_cross_project_boundaries`, y `an_unscoped_workspace_takes_the_preferences_of_every_project` (`workspace.rs`).
- [x] Test en rojo de PM7: `a_preference_out_of_the_workspace_is_still_found` (`prompt_builder.rs`), con filas sembradas como las escribe la versión anterior: la preferencia declarada de otro proyecto todavía entra al workspace.
- [x] La consulta de las preferencias pide el proyecto o ninguno cuando hay proyecto, como la de las decisiones; el comentario dice por qué.
- Commit: `fix(workspace): show a preference only in its project, or in every one when it has none`

## 3. Las preferencias siguen el alcance del lector (PM3)

- [x] Tests en rojo: `the_workspace_takes_the_preferences_the_reader_reaches` (`workspace.rs`) y `a_stated_preference_reaches_the_session_start_of_another_agent` (`crates/clawg-cli/tests/api.rs`).
- [x] `regenerate_auto_section` recibe el `Reach`; la consulta de las preferencias usa `reach_predicate!`; el daemon pasa el alcance del lector. `the_workspace_stays_own_and_conflicts_are_shared` fija lo propio con una decisión.
- Commit: `feat(workspace): show the preferences of every agent the reader reaches`

## 4. La marca de una preferencia en los hits (PM5)

- [x] Tests en rojo: `a_preference_hit_carries_its_origin` (`search.rs`), `a_preference_hit_line_shows_its_origin` (`prompt_builder.rs`), `format_hits_shows_the_origin_of_a_preference` (`tools.rs`) y `a_hit_from_an_old_daemon_has_no_origin` (`client.rs`).
- [x] `a_preference_out_of_the_workspace_is_still_found` pide además que el hit inferido llegue marcado.
- [x] `RankedHit.origin` desde `source` y `created_by_type` en las consultas de `memory`; `Hit.origin` en el cliente; la marca en `format_hit_line` y `format_hits`.
- Commit: `feat(search): mark a preference hit the user did not state`

## 5. `save_memory` con `all_projects` y el protocolo (PM4, PM6)

- [x] Tests en rojo: `a_save_for_every_project_has_no_project` y `all_projects_goes_without_a_project` (`server.rs`), y `the_protocol_says_which_preferences_open_every_session` (`protocol.rs`).
- [x] `SaveMemoryArgs.all_projects`, su descripción en inglés y en `tools_es.rs`, y la negativa con otro lugar. `protocol.md` y `protocol.es.md`, fuera de la apertura.
- [x] `CLAWG_BLESS=1` para los dos contratos, el diff revisado (sólo `all_projects` de `save_memory`), y los tests de nuevo.
- Commit: `feat(mcp): save a memory for every project with all_projects`

## 6. Docs

- [x] `docs/reference/daemon.md`: qué preferencias entran al workspace, el `origin` de un hit y `all_projects`.
- Commit: `docs(reference): say which preferences the workspace shows and how a hit marks one`

## 7. Verificación

- [x] La suite, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo clippy -p clawg-cli -- -D warnings` en Linux, sobre el último commit de código ([verify](verify.md)).
- [x] El CI del pull request en Linux y Windows; lo que marque se arregla. Windows marcó el helper de secciones del test del protocolo, que no leía saltos `\r\n`.
- Commit: `test(protocol): read a protocol section whatever its line breaks`
- [x] `verify.md`, con lo que se mide contra `prod` como pendiente en local; `state.yaml`; la línea del cambio en `ROADMAP.md` y la fila 117 del mapa de openspec.
- Commit: `docs(openspec): record preference-model applied`

## 8. Archivo (fuera de este cambio)

- [ ] Llevar PM1 a PM3 a `specs/retrieval` y R5 de `specs/sharing`, PM5 y PM9 a `specs/provenance` y PM5 también a `specs/retrieval`, PM4, PM6 y PM8 a `specs/mcp`, PM7 a `specs/retrieval`; mover la carpeta a `changes/archive/`; subir el minor.
