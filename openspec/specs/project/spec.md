# project — Especificación

Fuente de verdad del comportamiento actual de la identidad de proyecto: cómo una ruta se resuelve a la raíz de su proyecto, cómo se deriva el `project_id`, y cómo las filas guardadas bajo un id viejo pasan al de la raíz sin perderse.

**Origen:** [project-identity](../../changes/archive/2026-10-12-project-identity/proposal.md), archivado el 2026-10-12.

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

`POST /v1/projects`, con token, recibe `{ "root_path": "...", "legacy_ids": ["..."] }` y responde `{ "project_id": "...", "rekeyed": <n> }`.

- El daemon MUST derivar el id de `root_path` él mismo; no lo recibe.
- MUST guardar `(id, root_path)` en `project` si no existe.
- Si el id ya está registrado con **otra** `root_path`, MUST responder 409 sin mover ninguna fila.
- Por cada id viejo distinto del nuevo, MUST pasar al id nuevo todas las filas de las tablas con proyecto que lo tengan, incluidas las borradas lógicamente, en **una** transacción.
- Por cada fila movida, MUST escribir en `project_rekey` la tabla, el id de la fila, el id viejo, el id nuevo y la fecha, en la misma transacción.
- Un id viejo con formato inválido (fuera de `{basename}-[0-9a-f]{4}`) MUST dar 400 sin mover nada.
- La operación MUST ser idempotente: repetirla no mueve filas ni agrega entradas a `project_rekey`.

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

**Tests:** `rows_of_a_legacy_id_move_to_the_resolved_root`, `registering_again_moves_nothing`, `the_rekey_log_restores_every_row_it_moved`, `a_moved_row_is_found_lexically_under_its_new_project`, `a_root_whose_id_belongs_to_another_root_moves_nothing` y `a_malformed_legacy_id_is_refused_before_anything_moves` en `crates/clawg-core/src/memory/project.rs`; HTTP en `crates/clawg-cli/tests/api.rs` (`registering_a_root_moves_the_rows_of_its_legacy_id`, 400, 409, 401).

## R5. Datos de la versión anterior

Una base creada por la versión anterior, con filas en ids viejos, MUST abrir con la migración nueva sin cambiar ninguna fila, y las filas MUST seguir alcanzables por su id hasta que un registro las re-keyee.

#### Escenario
- GIVEN una base migrada hasta la última migración publicada, con filas en tres proyectos
- WHEN se abre con la versión nueva
- THEN las filas y sus `project_id` son idénticos, y existen `project` y `project_rekey` vacías

**Test:** `a_database_from_the_previous_version_opens_with_its_project_ids_untouched` en `crates/clawg-core/src/memory/db.rs` (siembra con el runner hasta V8).

## R6. El puente registra cada raíz una vez

Antes de la primera tool que use una raíz, el puente MUST llamar a `POST /v1/projects` con esa raíz y el id viejo de la ruta cruda (si difiere), y MUST NOT volver a llamarlo para la misma raíz y ruta en el mismo proceso. Si la llamada falla, la tool MUST seguir con el id nuevo y registrar un warn: un registro fallido no bloquea un save.

**Tests:** `a_root_is_registered_once_per_bridge` y `a_refused_registration_does_not_block_the_save` en `crates/clawg-cli/src/mcp/server.rs`, contra un daemon en proceso.

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
