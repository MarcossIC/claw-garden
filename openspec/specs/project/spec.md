# project — Especificación

Fuente de verdad del comportamiento actual de la identidad de proyecto. Cubre cómo una ruta se resuelve a la raíz de su proyecto y cómo se deriva el `project_id`. También cómo las filas guardadas bajo un id viejo pasan al de la raíz sin perderse, qué marca el registro sobre si el daemon puede leer los archivos de la raíz (`index_docs`), y qué hace el puente cuando el id de una raíz ya pertenece a otra. Qué raíces se indexan y cómo se buscan sus docs está en [`specs/project-docs`](../project-docs/spec.md).

**Origen:**
- [project-identity](../../changes/archive/2026-10-12-project-identity/proposal.md), archivado el 2026-10-12.
- [project-root-guard](../../changes/archive/2026-10-15-project-root-guard/proposal.md), archivado el 2026-10-15 (la marca `index_docs` en R4 y R6, y la colisión como error en R6).

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

El puente MCP (cwd y `project_path`) y `clawg memories --here` MUST derivar el id con `ProjectIdentity::resolve` (`crates/clawg-core/src/memory/identity.rs`). Si falla `current_dir()`, el puente MUST seguir como hoy: sin proyecto por defecto (`crates/clawg-cli/src/mcp/server.rs:165-167`).

#### Escenario
- GIVEN un puente con cwd en `<tmp>/repo/src`
- WHEN guarda una memoria sin `project_path`
- THEN la fila queda con el id de `<tmp>/repo`

**Tests:** `resolve_project` en `crates/clawg-cli/src/mcp/tools.rs` (la resolución entra como función) y `Scope::project_id` en `crates/clawg-cli/src/cli.rs` (entra el id ya resuelto), para seguir siendo puros. `a_bridge_in_a_subdirectory_brings_what_was_saved_there_to_the_root` en `crates/clawg-cli/src/mcp/server.rs`.

## R4. Registro de proyecto y re-key

`POST /v1/projects`, con token, recibe `{ "root_path": "...", "legacy_ids": ["..."], "index_docs": <bool> }` y responde `{ "project_id": "...", "rekeyed": <n> }` (`register`, `crates/clawg-cli/src/daemon/api.rs:1064`; `register_project`, `crates/clawg-core/src/memory/project.rs:62`).

- El daemon MUST derivar el id de `root_path` él mismo; no lo recibe.
- `index_docs` dice si el daemon puede leer los archivos de la raíz ([`specs/project-docs`](../project-docs/spec.md), «Qué raíces se leen»). Si falta MUST tomarse como `false` (`RegisterRequest::index_docs`, `api.rs:1051`): un puente de una versión anterior registra sin indexar, y lo arregla `clawg projects add` o un puente actual en esa raíz.
- Con `index_docs: true` y una `root_path` que no es un directorio, MUST responder 400 antes de tocar `project` (`api.rs:1072-1074`). Sin `index_docs`, la misma ruta se sigue registrando como siempre.
- MUST guardar `(id, root_path, index_docs)` en `project` si no existe.
- Si la fila existe con la misma raíz y el registro pide indexar, MUST pasar `index_docs` de 0 a 1. MUST NOT bajarlo de 1 a 0: un save en otra sesión no le quita a una raíz aceptada su índice (`project.rs:98-100`). El resultado (`RegisterOutcome::index_docs`) lleva el valor que quedó guardado.
- Si el id ya está registrado con **otra** `root_path`, MUST responder 409 sin mover ninguna fila y sin tocar la marca de la otra raíz. El cuerpo es `{ "error": "project_root_collision", "detail": "<raíz> gets the project id <id>, which already belongs to <otra raíz>" }` (`api.rs:1085-1088`).
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

Antes de la primera tool que use una raíz, el puente MUST llamar a `POST /v1/projects` con esa raíz, el id viejo de la ruta cruda (si difiere) e `index_docs` (`ClawgMcp::project_for`, `crates/clawg-cli/src/mcp/server.rs:563`). MUST NOT volver a llamarlo para la misma raíz y ruta en el mismo proceso, salvo en dos casos: una colisión (abajo) y una falla que se resuelve releyendo ([`specs/mcp`](../mcp/spec.md), «Registro del proyecto»).

`index_docs` MUST ser `true` sólo cuando la raíz de la llamada es la raíz del puente, la que resolvió de su cwd al arrancar (`server.rs:331-333`), y `false` en cualquier otro caso. Una raíz que el modelo nombra en `project_path` queda así sólo registrada hasta que el usuario la agregue con `clawg projects add` ([`specs/project-docs`](../project-docs/spec.md)). La decisión es una función pura de las dos raíces, sin filesystem (`indexes_docs`, `crates/clawg-cli/src/mcp/tools.rs:333`). Compara `root` y no el id, porque dos raíces que chocan comparten id.

**Colisión.** Si el daemon responde 409 `project_root_collision`, la tool MUST fallar sin guardar ni buscar. El texto MUST nombrar las dos raíces y decir que clawg no mezcla las memorias de dos proyectos bajo un id. También MUST decir que el proyecto no se puede usar hasta renombrar una de las dos carpetas, y que `all_projects` sigue buscando en todos (`ClawgMcp::explain`, `server.rs:465-470`). El puente MUST NOT recordar la raíz como registrada (`server.rs:583-586`): la llamada siguiente vuelve a preguntar y vuelve a fallar mientras el choque siga. Así, un `clawg restore` que lo resuelve se nota sin reiniciar el puente. Vale para las cinco tools con proyecto: `save_memory`, `search_memory`, `recall_context`, `search_project` y `synthesize`. Con `all_projects: true` (en `search_memory` y `synthesize`) no hay proyecto ni registro, así que no fallan.

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

**Tests:** `a_root_is_registered_once_per_bridge`, `a_save_elsewhere_registers_the_root_without_reading_it`, `a_colliding_root_refuses_the_save_every_time`, `a_colliding_root_still_searches_all_projects`, `a_registration_the_daemon_cannot_do_does_not_block_the_save` y `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` en `crates/clawg-cli/src/mcp/server.rs`, contra un daemon en proceso; `only_the_bridges_own_root_is_indexed` (puro) en `crates/clawg-cli/src/mcp/tools.rs`.

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
