# project — Especificación

Fuente de verdad del comportamiento actual de la identidad de proyecto. Cubre cómo una ruta se resuelve a la raíz de su proyecto y cómo se deriva el `project_id`. También cómo las filas guardadas bajo un id viejo pasan al de la raíz sin perderse, qué marca el registro sobre si el daemon puede leer los archivos de la raíz (`index_docs`), y qué hace el puente cuando el id de una raíz ya pertenece a otra. Qué raíces se indexan y cómo se buscan sus docs está en [`specs/project-docs`](../project-docs/spec.md).

**Origen:**
- [project-identity](../../changes/archive/2026-10-12-project-identity/proposal.md), archivado el 2026-10-12.
- [project-root-guard](../../changes/archive/2026-10-15-project-root-guard/proposal.md), archivado el 2026-10-15 (la marca `index_docs` en R4 y R6, y la colisión como error en R6).
- [untrusted-recall](../../changes/archive/2026-10-20-untrusted-recall/proposal.md), archivado el 2026-10-20 (R6: la raíz del puente pide `index_docs` sólo si es un proyecto).
- [project-reassign](../../changes/archive/2026-10-22-project-reassign/proposal.md), archivado el 2026-10-22 (R8 a R13: mover filas de `memory` a otro proyecto o a ninguno, con su unidad, su registro y su vuelta atrás).

## Definiciones

- **Ruta cruda:** la que llega tal cual: el cwd del puente, `project_path` de una tool, o el cwd de `clawg memories --here`.
- **Raíz del proyecto:** el resultado de `resolve_project_root(ruta cruda)` (R1).
- **Id viejo:** `derive_project_id(ruta cruda)`, el que se usaba antes de resolver rutas (`crates/clawg-core/src/memory/identity.rs:11-15`).
- **Id nuevo:** `derive_project_id(raíz del proyecto)`. Mismo formato, `{basename}-{hash4}`.
- **Tablas con proyecto:** `memory`, `episodic_trajectory` y `procedural_skill` (`crates/clawg-core/migrations/V1__init.sql`).

## R1. Resolución de la raíz

`resolve_project_root` MUST devolver la misma raíz para todas las formas de nombrar un mismo directorio de un repo:

- canonicaliza la ruta (enlaces, `.`, `..`, relativa al cwd, caja real del disco en Windows) y le quita el prefijo `\\?\`;
- sube desde ahí hasta el primer directorio que contenga `.git`;
- si `.git` es un directorio, la raíz es ese directorio;
- si `.git` es un archivo `gitdir: <dir>` y `<dir>` tiene un archivo `commondir`, la raíz es el padre del directorio común (un worktree comparte la raíz del repo principal);
- si `.git` es un archivo sin `commondir` en su `gitdir` (submódulo), la raíz es el directorio que contiene ese `.git`;
- si ningún ancestro tiene `.git`, la raíz es la ruta canonicalizada;
- si la ruta no existe, la raíz es la ruta normalizada léxicamente (sin separador final, sin `.`, con `..` resueltos), sin buscar `.git`.

La decisión (qué marcador, qué raíz) MUST ser una función pura que no toca el filesystem. La lectura de `.git` y `commondir` va aparte.

#### Escenario: misma raíz por varios caminos
- GIVEN un repo en `<tmp>/repo` con `.git/` y un subdirectorio `src/lib`
- WHEN se resuelven `<tmp>/repo`, `<tmp>/repo/`, `<tmp>/repo/src/lib` y `<tmp>/repo/src/..`
- THEN las cuatro dan `<tmp>/repo`

#### Escenario: worktree
- GIVEN `<tmp>/wt/.git` con `gitdir: <tmp>/repo/.git/worktrees/wt`, y ese directorio con `commondir` = `../..`
- WHEN se resuelve `<tmp>/wt`
- THEN la raíz es `<tmp>/repo`

#### Escenario: submódulo
- GIVEN `<tmp>/repo/sub/.git` con `gitdir: ../.git/modules/sub`, sin `commondir`
- WHEN se resuelve `<tmp>/repo/sub`
- THEN la raíz es `<tmp>/repo/sub`

#### Escenario: sin git
- GIVEN `<tmp>/plain` sin `.git` en ningún ancestro
- WHEN se resuelve `<tmp>/plain/`
- THEN la raíz es `<tmp>/plain`

#### Escenario: Windows
- GIVEN `C:\Users\u\Repo` existente con `.git\`
- WHEN se resuelven `c:\users\u\repo` y `C:/Users/u/Repo/`
- THEN las dos dan `C:\Users\u\Repo`, sin `\\?\`

**Tests:** unit de la decisión pura en `identity.rs` (marcadores falsos, sin filesystem); integración con directorios temporales en `crates/clawg-core/tests/project_root.rs`. El de Windows corre sólo en Windows (`#[cfg(windows)]`).

## R2. Id estable en el caso común

El id nuevo de una ruta que ya es la raíz canónica MUST ser igual a `derive_project_id` de esa ruta hoy. Así, el puente que arranca en la raíz del repo no re-keyea nada.

#### Escenario
- GIVEN `<tmp>/repo` canónica, con `.git/`
- THEN `ProjectIdentity::resolve(<tmp>/repo).id == derive_project_id(<tmp>/repo)`, y su `legacy_id` es el mismo

**Tests:** `the_id_of_a_canonical_root_is_the_id_it_always_had` y `a_subdirectory_keeps_its_old_id_as_the_legacy_one` en `crates/clawg-core/tests/project_root.rs`.

## R3. Todos los caminos usan la misma resolución

El puente MCP (cwd y `project_path`) y `clawg memories --here` MUST derivar el id con `ProjectIdentity::resolve` (`crates/clawg-core/src/memory/identity.rs`). Si falla `current_dir()`, el puente MUST seguir como hoy: sin proyecto por defecto (`crates/clawg-cli/src/mcp/server.rs:423-425`).

#### Escenario
- GIVEN un puente con cwd en `<tmp>/repo/src`
- WHEN guarda una memoria sin `project_path`
- THEN la fila queda con el id de `<tmp>/repo`

**Tests:** `resolve_project` en `crates/clawg-cli/src/mcp/tools.rs` (la resolución entra como función) y `Scope::project_id` en `crates/clawg-cli/src/cli.rs` (entra el id ya resuelto), para seguir siendo puros. `a_bridge_in_a_subdirectory_brings_what_was_saved_there_to_the_root` en `crates/clawg-cli/src/mcp/server.rs`.

## R4. Registro de proyecto y re-key

`POST /v1/projects`, con token, recibe `{ "root_path": "...", "legacy_ids": ["..."], "index_docs": <bool> }` y responde `{ "project_id": "...", "rekeyed": <n> }` (`register`, `crates/clawg-cli/src/daemon/api.rs:1669`; `register_project`, `crates/clawg-core/src/memory/project.rs:62`).

- El daemon MUST derivar el id de `root_path` él mismo; no lo recibe.
- `index_docs` dice si el daemon puede leer los archivos de la raíz ([`specs/project-docs`](../project-docs/spec.md), «Qué raíces se leen»). Si falta MUST tomarse como `false` (`RegisterRequest::index_docs`, `api.rs:1656`): un puente de una versión anterior registra sin indexar, y lo arregla `clawg projects add` o un puente actual en esa raíz cuando es un proyecto.
- Con `index_docs: true` y una `root_path` que no es un directorio, MUST responder 400 antes de tocar `project` (`api.rs:1677-1679`). Sin `index_docs`, la misma ruta se sigue registrando como siempre.
- MUST guardar `(id, root_path, index_docs)` en `project` si no existe.
- Si la fila existe con la misma raíz y el registro pide indexar, MUST pasar `index_docs` de 0 a 1. MUST NOT bajarlo de 1 a 0: un save en otra sesión no le quita a una raíz aceptada su índice (`project.rs:98-100`). El resultado (`RegisterOutcome::index_docs`) lleva el valor que quedó guardado.
- Si el id ya está registrado con **otra** `root_path`, MUST responder 409 sin mover ninguna fila y sin tocar la marca de la otra raíz. El cuerpo es `{ "error": "project_root_collision", "detail": "<raíz> gets the project id <id>, which already belongs to <otra raíz>" }` (`api.rs:1690-1693`).
- Por cada id viejo distinto del nuevo, MUST pasar al id nuevo todas las filas de las tablas con proyecto que lo tengan, incluidas las borradas lógicamente, en **una** transacción.
- Por cada fila movida, MUST escribir en `project_rekey` la tabla, el id de la fila, el id viejo, el id nuevo y la fecha, en la misma transacción.
- Un id viejo con formato inválido (fuera de `{basename}-[0-9a-f]{4}`) MUST dar 400 sin mover nada.
- La operación MUST ser idempotente: repetirla no mueve filas ni agrega entradas a `project_rekey`.
- Un registro que tiene éxito (nuevo o ya conocido) y deja la raíz aceptada también lanza el refresco del índice de documentación del proyecto, en segundo plano y sin que la respuesta lo espere: [`specs/project-docs`](../project-docs/spec.md), «Registrar una raíz aceptada la indexa en segundo plano». No lo lanzan uno que deja la raíz sólo registrada ni uno que falla (400, 409).

#### Escenario: la marca sólo sube
- GIVEN una raíz registrada con `index_docs: false`
- WHEN se registra otra vez con `true`, y después con `false`
- THEN la marca queda en 1 después de la segunda llamada y sigue en 1 después de la tercera

#### Escenario: colisión
- GIVEN `project` con el id de `/work/repo` registrado con la raíz `/elsewhere/repo` y `index_docs = 0`
- WHEN se registra `/work/repo` pidiendo indexar
- THEN el registro falla con `RootCollision` (por HTTP, 409 `project_root_collision` con las dos raíces en `detail`), y la marca de `/elsewhere/repo` sigue en 0

#### Escenario: filas viejas visibles bajo el id nuevo
- GIVEN una base con 2 memorias, 1 trayectoria y 1 skill en el id viejo `src-1a2b`, y 1 memoria en otro proyecto
- WHEN se registra la raíz con `legacy_ids = ["src-1a2b"]`
- THEN las 4 filas tienen el id nuevo, la del otro proyecto no cambió, `project_rekey` tiene 4 entradas y la respuesta dice `rekeyed: 4`

#### Escenario: idempotencia
- WHEN se repite el mismo registro
- THEN `rekeyed: 0` y `project_rekey` sigue con 4 entradas

#### Escenario: rollback
- GIVEN el estado del primer escenario
- WHEN se aplica el SQL de rollback de `design.md`
- THEN cada fila vuelve a su id viejo

#### Escenario: búsqueda léxica después del re-key
- GIVEN una memoria re-keyeada con un término único
- WHEN se busca ese término en el proyecto nuevo
- THEN aparece por el canal léxico (FTS lee `project_id` de `memory`)

**Tests:** `rows_of_a_legacy_id_move_to_the_resolved_root`, `registering_again_moves_nothing`, `the_rekey_log_restores_every_row_it_moved`, `a_moved_row_is_found_lexically_under_its_new_project`, `a_root_whose_id_belongs_to_another_root_moves_nothing`, `a_malformed_legacy_id_is_refused_before_anything_moves`, `registering_to_index_raises_the_mark_and_never_lowers_it` y `a_collision_leaves_the_other_roots_mark_alone` en `crates/clawg-core/src/memory/project.rs`; HTTP en `crates/clawg-cli/tests/api.rs` (`registering_a_root_moves_the_rows_of_its_legacy_id`, `registering_a_root_whose_id_is_taken_by_another_is_a_conflict`, `registering_a_missing_root_to_index_it_is_a_bad_request`, y los de 400 y 401).

## R5. Datos de la versión anterior

Una base creada por la versión anterior, con filas en ids viejos, MUST abrir con la migración nueva sin cambiar ninguna fila, y las filas MUST seguir alcanzables por su id hasta que un registro las re-keyee.

#### Escenario
- GIVEN una base migrada hasta la última migración publicada, con filas en tres proyectos
- WHEN se abre con la versión nueva
- THEN las filas y sus `project_id` son idénticos, y existen `project` y `project_rekey` vacías

**Test:** `a_database_from_the_previous_version_opens_with_its_project_ids_untouched` en `crates/clawg-core/src/memory/db.rs` (siembra con el runner hasta V8).

La columna `index_docs` (V13) llega con 1 en cada fila de `project` que ya existía: [`specs/project-docs`](../project-docs/spec.md), «Datos de la versión anterior».

## R6. El puente registra cada raíz una vez

Antes de la primera tool que use una raíz, el puente MUST llamar a `POST /v1/projects` con esa raíz, el id viejo de la ruta cruda (si difiere) e `index_docs` (`ClawgMcp::project_for`, `crates/clawg-cli/src/mcp/server.rs:695`). MUST NOT volver a llamarlo para la misma raíz y ruta en el mismo proceso, salvo en dos casos: una colisión (abajo) y una falla que se resuelve releyendo ([`specs/mcp`](../mcp/spec.md), «Registro del proyecto»).

`index_docs` MUST ser `true` sólo cuando la raíz de la llamada es la raíz del puente, la que resolvió de su cwd al arrancar (`server.rs:423-425`), y esa raíz es un proyecto que se indexa solo: tiene un `.git` o un manifiesto, no es el directorio del usuario ni una raíz del sistema de archivos ([`specs/project-docs`](../project-docs/spec.md), «El puente acepta su raíz sola sólo si es un proyecto»). En cualquier otro caso MUST ser `false`. Una raíz que el modelo nombra en `project_path`, o la del puente cuando no es un proyecto, queda así sólo registrada hasta que el usuario la agregue con `clawg projects add`. La decisión es una función pura de las dos raíces, sin filesystem (`indexes_docs`, `crates/clawg-cli/src/mcp/tools.rs:564`): el puente le pasa su raíz sólo cuando `auto_indexable` la aceptó al construirse (`own_root_to_index`, `server.rs:404`), y si no, ninguna. Compara `root` y no el id, porque dos raíces que chocan comparten id.

**Colisión.** Si el daemon responde 409 `project_root_collision`, la tool MUST fallar sin guardar ni buscar. El texto MUST nombrar las dos raíces y decir que clawg no mezcla las memorias de dos proyectos bajo un id. También MUST decir que el proyecto no se puede usar hasta renombrar una de las dos carpetas, y que `all_projects` sigue buscando en todos (`ClawgMcp::explain`, `server.rs:558-563`). El puente MUST NOT recordar la raíz como registrada (`server.rs:715-717`): la llamada siguiente vuelve a preguntar y vuelve a fallar mientras el choque siga. Así, un `clawg restore` que lo resuelve se nota sin reiniciar el puente. Vale para las cinco tools con proyecto: `save_memory`, `search_memory`, `recall_context`, `search_project` y `synthesize`. Con `all_projects: true` (en `search_memory` y `synthesize`) no hay proyecto ni registro, así que no fallan.

**Otras fallas.** Si el registro falla por otra causa (un daemon que no conoce la ruta, un 400, un error de transporte), la tool MUST seguir con el id nuevo y dejar un warn: un registro fallido no bloquea un save.

#### Escenario: un save en otra raíz
- GIVEN un puente en `<tmp>/repo/src`
- WHEN `save_memory` recibe `project_path: <tmp>/other`
- THEN la memoria se guarda bajo el id de `<tmp>/other`, que queda con `index_docs = 0`, y nada de `<tmp>/other` se indexa

#### Escenario: colisión
- GIVEN `project` con el id de la raíz del puente y otra `root_path`
- WHEN el puente guarda una memoria dos veces
- THEN las dos llamadas son error, nombran las dos raíces y `all_projects`, y no hay ninguna fila bajo ese id
- AND `search_memory` con `all_projects: true` responde sin error

#### Escenario: un daemon que no sabe registrar
- GIVEN un daemon que responde 404 a `/v1/projects`
- WHEN el puente guarda una memoria
- THEN el save tiene éxito bajo el id de la raíz

**Tests:** `a_root_is_registered_once_per_bridge`, `a_save_elsewhere_registers_the_root_without_reading_it`, `a_colliding_root_refuses_the_save_every_time`, `a_colliding_root_still_searches_all_projects`, `a_registration_the_daemon_cannot_do_does_not_block_the_save`, `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` y `a_bridge_in_a_folder_that_is_not_a_project_indexes_nothing` en `crates/clawg-cli/src/mcp/server.rs`, contra un daemon en proceso; `only_the_bridges_own_root_is_indexed` (puro) en `crates/clawg-cli/src/mcp/tools.rs`; `auto_indexable_wants_a_project_that_is_not_home` (puro) en `crates/clawg-core/src/memory/project_docs/corpus.rs`.

## R7. `workspace.md` sigue al proyecto

En un re-key, por cada agente con `<workspaces>/<agente>/<id viejo>/workspace.md`:

- si no existe `<workspaces>/<agente>/<id nuevo>/workspace.md`, el daemon MUST copiarlo ahí (escritura atómica, como `write_atomically`);
- si existe, MUST dejar los dos como están y registrar un warn con las dos rutas;
- MUST NOT borrar ni modificar el archivo viejo.

#### Escenario
- GIVEN `agent-a/src-1a2b/workspace.md` con una sección USER escrita a mano
- WHEN se registra la raíz con ese id viejo
- THEN `agent-a/<id nuevo>/workspace.md` tiene la misma sección USER, y el viejo sigue igual

**Tests:** `a_rekey_copies_the_legacy_workspace_and_keeps_it`, `adopting_never_overwrites_the_projects_own_workspace` y `adopting_without_any_workspace_is_a_no_op` en `crates/clawg-core/src/memory/workspace.rs`; `registering_carries_the_legacy_workspace_over` en `project.rs`.

## R8. Mover cambia el proyecto de una unidad entera y nada más

Un **movimiento** cambia el `project_id` de filas de `memory` a un **destino**: un id de proyecto o «sin proyecto». La **unidad** de una fila es su cierre: las filas de todo save de `memory_piece` que la contiene, las que reemplazó con causa `revision` y la que la reemplazó con esa causa, las que su borrado revivió (`revived_from`) y la que revivió por el suyo, repetido hasta que no entra ninguna más, borradas y `superseded` incluidas (`read_units_in`, `crates/clawg-core/src/memory/relocate.rs`).

Un movimiento MUST cambiar, en una transacción, `project_id` y, en una fila sin agente, `scope` (`project` con destino, `global` sin él; `scope_after`) de cada fila de cada unidad aceptada, y MUST NOT tocar otra columna de `memory` (ni `last_modified_at`, ni `update_count`, ni `status`), `memory_vectors`, `memory_fts`, `memory_conflict`, `memory_derivation` ni `memory_piece`. No MUST necesitar el embedder ni correr el detector. Después, la búsqueda, RECENT, LAST SESSION, la dedup, la clave de tema, el revivir y la revisión de conclusiones MUST tratar las filas como guardadas en el destino. Una fila sin proyecto se sigue viendo desde todos los proyectos.

#### Escenario: sin proyecto a uno
- GIVEN una fila sin proyecto que se encuentra desde A y desde B
- WHEN se mueve a B
- THEN se encuentra desde B, por vector y por texto, y no desde A

#### Escenario: la cadena se mueve con su vigente
- GIVEN una clave con su vigente y la versión que reemplazó, en A
- WHEN se mueve la versión vieja a B y después se borra la vigente
- THEN las dos se movieron y la versión anterior vuelve viva en B

**Test:** `a_row_without_a_project_moves_into_one`, `moving_to_no_project_shows_the_row_from_every_project`, `a_row_without_an_agent_keeps_the_scope_check`, `the_pieces_of_a_save_move_together`, `two_saves_that_share_a_row_move_together`, `a_revision_chain_moves_with_its_head_and_deleting_the_head_still_revives`, `after_a_move_a_save_of_the_same_text_reinforces_the_moved_row`, `after_a_move_a_save_under_its_key_revises_it`, `a_session_summary_moved_into_a_project_is_its_last_session`, `moving_a_superseded_source_does_not_make_its_conclusions_reappear_for_review`, `a_move_leaves_vectors_and_the_text_index_alone` y `scope_after_a_move_follows_the_check` (`relocate.rs`).

## R9. Lo que rechaza una unidad

Una unidad MUST quedarse donde está, sin escribir ninguna de sus filas, cuando una fila viva del destino al alcance del lector tiene el `content_hash` (`live_twin`) o la `topic_key` (`topic_taken`) de una fila viva de la unidad, cuando una unidad aceptada antes en el mismo movimiento lleva ese hash o esa clave a ese destino (se mueve la primera, por id), cuando sus filas están en más de un proyecto (`spans_projects`) o cuando tiene una fila fuera del alcance del lector (`out_of_reach`, sin nombrarla). El motivo MUST nombrar la fila de la unidad y la que sostiene el texto o la clave. Una unidad que ya está en el destino queda `unchanged` y no se registra. Una fila borrada MUST moverse con su unidad sin chocar; un `restore` posterior valida en el proyecto nuevo. Los pares abiertos que quedan entre dos proyectos MUST nombrarse (`split_pairs`) y quedar como están. La decisión es pura (`decide`).

**Test:** `decide_*` (puros), `a_live_row_of_the_same_text_in_the_target_refuses_the_move`, `a_live_holder_of_the_topic_key_in_the_target_refuses_the_move`, `two_selected_saves_that_would_collide_in_the_target_move_only_the_first`, `a_unit_across_projects_is_refused`, `a_hidden_row_in_the_unit_refuses_it_without_naming_it`, `a_clash_is_judged_within_the_readers_reach`, `a_deleted_row_moves_with_its_save_and_is_restored_into_the_new_project` y `a_pair_left_across_projects_is_reported` (`relocate.rs`).

## R10. El destino está registrado

El daemon MUST rechazar con 404, sin escribir, un destino que no está en `project`; «sin proyecto» siempre es un destino. El puente (`to_project_path`) y `clawg projects move --to PATH` MUST registrar antes la raíz de la ruta, sin `index_docs`, y MUST rechazar una raíz que no es un directorio.

**Test:** `an_unregistered_target_is_refused` (`relocate.rs`), `a_move_to_an_unregistered_project_is_404` (`tests/api.rs`), `update_memory_refuses_both_destinations_and_a_path_that_is_not_a_directory` (`server.rs`).

## R11. El registro y la vuelta atrás

Cada fila que un movimiento cambia, por cualquier puerta, MUST dejar en `memory_move` (V21) el número del movimiento (`MAX + 1`), la fila, el proyecto y el `scope` de origen y de destino y el momento, en la misma transacción. Deshacer el movimiento N (`undo_move_core`) MUST mover de vuelta a su origen, con R8 y R9 y sin exigir que el origen esté registrado, las filas de N que siguen en el destino que N anotó, como un movimiento nuevo; una fila que se movió otra vez después queda donde está. Un N que no existe es un 404.

**Test:** `every_moved_row_is_logged_and_the_log_undoes_it` y `undoing_leaves_a_row_that_moved_again` (`relocate.rs`).

## R12. El lote

`POST /v1/memories/move` MUST tomar `dry_run` y una selección (al menos uno de `ids`, `from_project`/`from_none`, `agent`, `memory_type`, `source`, `session_id` o `q`; sin `include_deleted`, filas no borradas; `q` con `include_deleted` es un 400) con un destino (`to_project` o `to_none`), o `undo`. Es del usuario: sin lector y sin `user_answered`; cuenta en `protected` las preferencias y decisiones vivas que movió. Con algo que mover y sin `dry_run`, MUST tomar antes el snapshot `pre-move`. Un ensayo MUST responder lo mismo que el movimiento, salvo `move_id` y `snapshot`, sin escribir nada. `clawg projects move` manda ese pedido al daemon que corre, sin arrancar uno, e imprime el reporte; su análisis de argumentos es puro (`MoveOpts::request`).

**Test:** `a_dry_run_reports_what_a_move_does_and_writes_nothing`, `a_selection_needs_a_filter_and_text_only_over_live_rows` y `the_filters_narrow_the_selection` (`relocate.rs`); `a_move_asks_for_one_target_or_an_undo` (`api.rs`); `the_batch_moves_reports_and_snapshots` y `a_batch_without_a_selection_or_a_target_is_400` (`tests/api.rs`); `move_takes_a_selection_and_one_target_or_an_undo` (`cli.rs`); `a_move_report_names_what_moved_what_stayed_and_the_way_back` y `projects_move_moves_a_memory_and_undoes_it` (`projects.rs`); `projects_move_moves_and_undoes_a_seeded_store` (`tests/binary.rs`).

## R13. Listar lo que no tiene proyecto

`GET /v1/memories?no_project=true` MUST listar sólo las filas sin proyecto; con `project` es un 400. `clawg memories list` y `search` toman `--no-project`.

**Test:** `a_listing_of_rows_without_a_project` (`admin.rs`), `no_project_and_project_together_are_refused` (`api.rs`), `the_listing_reaches_the_rows_without_a_project` (`tests/api.rs`), `list_and_search_take_no_project` (`cli.rs`).
