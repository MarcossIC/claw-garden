# retire-legacy-kinds — Especificación

Requisitos sobre [`specs/storage`](../../../specs/storage/spec.md), [`specs/retrieval`](../../../specs/retrieval/spec.md), [`specs/project`](../../../specs/project/spec.md), [`specs/restore`](../../../specs/restore/spec.md), [`specs/secrets`](../../../specs/secrets/spec.md), [`specs/portability`](../../../specs/portability/spec.md), [`specs/doctor`](../../../specs/doctor/spec.md), [`specs/mcp`](../../../specs/mcp/spec.md) y [`specs/eval`](../../../specs/eval/spec.md); se consolidan al archivar. Lo que no se nombra acá sigue como está.

Términos: una **fila vieja** es una fila de `episodic_trajectory` o `procedural_skill`; su **fila migrada** es la que V32 escribe en `memory`; un **kind retirado** es `episodic` o `procedural`; la **explicación** es el texto que dice que ese kind ya no existe, que sus filas son ahora filas de `memory` de tipo `semantic_fact` con otro id, y dónde se buscan.

## R1. Cada fila vieja pasa a `memory`

V32 MUST escribir, por cada fila vieja, viva o borrada, una fila migrada con un id nuevo mayor que todo id que `memory` haya dado, `memory_type = 'semantic_fact'` y `source` igual al nombre de su tabla. Cada columna de la fila vieja MUST quedar en la columna de `memory` del mismo nombre, en `source_session_id` (`session_id` de una trayectoria) o en una línea del texto, como dice [D2](design.md#d2-qué-va-a-cada-columna). `content_hash` MUST ser el hash del texto, como lo calcula un save.

Una fila viva MUST quedar con el vector de su tabla, si tenía, y en el índice de texto. Una borrada MUST quedar con su `deleted_at`, sin vector y fuera del índice de texto.

`superseded_by`, cada id de `disputed_against` y cada id de `source_trajectory_ids` MUST nombrar el id nuevo de la fila a la que apuntaban; uno que no nombra ninguna fila de su tabla MUST quitarse. Una fila migrada `superseded` con reemplazante MUST tener `supersede_cause = 'verdict'`, y cada id que una disputada nombra MUST ser un par `escalated`, como V17 y V15 hicieron con las filas de `memory`.

`legacy_kind_move` MUST tener una fila `(tbl, old_id, new_id)` por cada fila migrada. Después MUST NOT quedar `episodic_trajectory`, `procedural_skill`, `episodic_vectors`, `procedural_vectors` ni sus tablas sombra. Ninguna fila de `memory` que ya estaba, y ningún proyecto, MUST cambiar.

#### Escenario: base de la versión anterior
- GIVEN una base en V31 con una fila de `memory`, una trayectoria viva con vector, una reemplazada por otra, una borrada, una habilidad viva disputada contra otra que cita dos trayectorias, una de ellas inexistente, y un proyecto borrado
- WHEN se abre con la versión nueva
- THEN cada fila vieja está en `memory` como dice R1, una búsqueda encuentra cada fila viva por su texto y por su vector, la borrada no aparece, el índice de texto tiene exactamente las filas vivas, la fila de `memory` y el proyecto borrado siguen igual, y no queda ninguna de las cuatro tablas

**Test:** `trajectories_and_skills_move_to_memory_and_their_tables_go` (`crates/clawg-core/src/memory/migrations.rs`); el checksum de V32 en `published_migration_checksums_are_pinned`.

## R2. Los diarios nombran las filas migradas

V32 MUST reescribir cada entrada de `project_delete_row` y de `project_rekey` que nombra una fila vieja a `tbl = 'memory'` y el id nuevo. Cada cambio `trajectory` o `skill` de `project_merge_change` MUST pasar a ser una fila de `memory_move` de la unión, de `source` a `target`, con el alcance de la fila y el `merged_at` de la unión; una unión sin `move_id` MUST recibir uno nuevo. Una entrada que nombra una fila que ya no existe MUST quedar como está.

#### Escenario: un proyecto borrado antes
- GIVEN en V31 un proyecto borrado con sus memorias, con una trayectoria y una habilidad en su diario
- WHEN se abre con la versión nueva y se deshace el borrado
- THEN las dos vuelven vivas al proyecto, como `semantic_fact`, con su vector y en el índice de texto

#### Escenario: una unión hecha antes
- GIVEN en V31 la unión de A en B que movió una trayectoria y una habilidad, sin filas de `memory`
- WHEN se abre con la versión nueva y se deshace la unión
- THEN A vuelve a ser un proyecto y las dos filas vuelven a A

**Test:** `a_project_deleted_before_the_move_comes_back_with_its_trajectories_and_skills`, `a_merge_from_before_the_move_takes_its_trajectories_and_skills_back` y, para `project_rekey`, `trajectories_and_skills_move_to_memory_and_their_tables_go` (`migrations.rs`).

## R3. Una fila borrada antes se restaura y se purga

Una fila migrada borrada MUST poder restaurarse con `restore_memory_core` (vector calculado al restaurar, de vuelta en el índice de texto) y purgarse con `purge`.

#### Escenario: borradas a mano
- GIVEN en V31 una trayectoria y una habilidad borradas que ningún diario nombra
- WHEN se abre con la versión nueva
- THEN se restaura la migrada de la trayectoria y una búsqueda la encuentra, y se purga la de la habilidad y no queda en `memory` ni en `memory_vectors`

**Test:** `a_trajectory_and_a_skill_deleted_before_the_move_are_restored_and_purged` (`migrations.rs`).

## R4. Nada más las lee

Ningún archivo de `crates/*/src` MUST nombrar las cuatro tablas salvo `migrations.rs`, que siembra la versión anterior, y `portability/export.rs`, que cuenta las filas viejas de una base que todavía no migró. La búsqueda MUST correr un solo KNN, sobre `memory`, y `search.duration` MUST NOT registrar `knn_episodic` ni `knn_skill`. Mantenimiento, el registro, la unión, el borrado, la purga y la deshechura de proyectos e importaciones, `inspect` y el espacio de trabajo MUST leer y escribir sólo `memory`. `TableKind` y `MemoryKind` MUST NOT existir.

**Test:** `only_the_migrations_and_the_export_name_the_retired_tables` (`crates/clawg-cli/tests/conventions.rs`); `a_search_runs_one_knn` (`crates/clawg-core/src/memory/search.rs`).

## R5. La API

El segmento `{kind}` de `/v1/memories/{kind}/{id}`, `/save` y `/restore`, y el filtro `kind` de `GET /v1/memories`, MUST aceptar `memory`. Un kind retirado MUST dar un 400 con la explicación, que nombra `search_memory`; cualquier otro valor, un 400 que dice que `memory` es el único. Las respuestas MUST seguir llevando `kind` (`memory` en una fila, `Memory` en un hit) y `GET /v1/stats` MUST seguir llevando `totals.episodic` y `totals.procedural` en 0, porque un bridge de la versión anterior los exige; las filas MUST NOT llevar `success_count`, `trigger` ni `source_trajectory_ids`.

**Test:** `a_retired_kind_is_refused_with_where_its_rows_went` (`crates/clawg-cli/tests/api.rs`); `parse_kind_takes_memory_alone` y `a_retired_kind_says_where_its_rows_went` (`crates/clawg-cli/src/daemon/api.rs`).

## R6. La CLI

`--kind` de `clawg memories list` y `show` MUST quedar fuera de la ayuda, aceptar `memory` y rechazar un kind retirado con un error de uso que lleva la explicación y nombra `clawg memories search`. `memories stats`, los informes de `projects` y `purge --project-delete` MUST NOT nombrar trayectorias ni habilidades.

**Test:** `a_retired_kind_is_a_usage_error_that_says_where_its_rows_went` (`crates/clawg-cli/src/cli.rs`).

## R7. El MCP

`kind` MUST NOT estar en el esquema de `list_memories`, `delete_memory`, `restore_memory`, `read_memory` ni `update_memory`. Un llamado que igual lo manda MUST aceptar `memory`, y con un kind retirado MUST responder la explicación, en el idioma del bridge y nombrando `search_memory`, sin llamar al daemon. Las descripciones MUST NOT hablar de tablas ni de ids por tabla.

**Test:** `a_retired_kind_is_answered_with_where_its_rows_went` (`crates/clawg-cli/src/mcp/tools.rs`); `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, bendecidos.

## R8. `doctor` y las invariantes

Sobre una base recién migrada desde V31 con filas viejas, las invariantes de `inspect` MUST estar vacías, y `clawg doctor` MUST NOT dar ningún hallazgo `error` ni `warn` del almacén.

**Test:** `a_store_moved_from_trajectories_and_skills_breaks_no_invariant` (`migrations.rs`); `doctor_passes_over_a_store_whose_trajectories_and_skills_moved` (`crates/clawg-cli/tests/binary.rs`).

## R9. Exportación

Exportar una base en V32 MUST NOT leer las tablas viejas. Exportar una base anterior a V32 MUST seguir diciendo cuántas filas vivas viejas no se exportan.

**Test:** `an_export_of_a_store_before_the_move_counts_the_rows_it_leaves` (`crates/clawg-core/src/memory/portability/export.rs`) y los tests de exportación que ya corren en la versión nueva.

## R10. Eval

`rag-eval` MUST pedir sólo el pool de `memory` y el léxico, y `k_pool` MUST NOT registrar `episodic` ni `procedural`. `rag-eval --diff` contra la línea base de `openspec/config.yaml` MUST NOT mover ningún caso (pendiente en local, con el modelo real).

**Test:** `k_pool_mirrors_the_production_lexical_budget` y los de `report.rs`, ajustados.
