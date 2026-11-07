# retire-legacy-kinds — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, en `target`, sobre `08560976`: el último commit de código. El commit de registro sólo toca esta carpeta.

- `cargo test --workspace --no-fail-fast` pasa, sin ningún test fallado. Los ignorados son los `#[ignore]` que ya tenía el repositorio: este cambio no agrega ninguno.
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también.
- Los contratos del agente pasan después de bendecirlos con `CLAWG_BLESS=1`, en los dos idiomas ([abajo](#el-contrato-del-agente)).
- Windows: lo revisa el CI del pull request. El cambio no toca código `cfg(windows)`.

## Los tests en rojo

Cada test se corrió antes del código que lo hace pasar, salvo los que la tabla dice.

| Requisito | Test | Cómo fallaba |
|---|---|---|
| R1, R2 (`project_rekey`) | `trajectories_and_skills_move_to_memory_and_their_tables_go` | Sin V32 la base quedaba en V31: ninguna fila vieja estaba en `memory` |
| R2 | `a_project_deleted_before_the_move_comes_back_with_its_trajectories_and_skills` | Deshacer el borrado traía de vuelta 0 filas de `memory` en lugar de 2 |
| R2 | `a_merge_from_before_the_move_takes_its_trajectories_and_skills_back` | Deshacer la unión devolvía 0 filas al proyecto de origen en lugar de 2 |
| R3 | `a_trajectory_and_a_skill_deleted_before_the_move_are_restored_and_purged` | La fila migrada que el test restaura no existía |
| R8 | `a_store_moved_from_trajectories_and_skills_breaks_no_invariant` | 0 filas migradas en lugar de 6 |
| R4 | `no_production_code_names_the_retired_tables` (`crates/clawg-cli/tests/conventions.rs`) | Listaba cada línea de producción que nombraba las tablas. Corrió en rojo con un primer nombre, `only_the_migrations_and_the_export_name_the_retired_tables`, que se cambió cuando quedó claro que mira sólo el código antes del módulo de tests |
| R4 | `a_search_records_how_long_each_arm_took` (`search.rs`) | `search.duration` registraba `knn_episodic` y `knn_skill` |
| R9 | `an_export_of_a_store_before_the_move_counts_the_rows_it_leaves` (`portability/export.rs`) | Una base recién creada todavía tenía las dos tablas |
| R5 | `a_retired_kind_is_refused_with_where_its_rows_went` (`crates/clawg-cli/tests/api.rs`) | `episodic` daba 404 `no Episodic row with id 1` en lugar del 400 con la explicación |
| R5 | `parse_kind_takes_memory_alone`, `a_retired_kind_says_where_its_rows_went`, `a_listing_of_a_retired_kind_is_refused`, `an_unknown_kind_names_the_value_and_the_only_kind` (`daemon/api.rs`) | No compilaban: `parse_kind` devolvía un `Option<MemoryKind>` |
| R5 | `the_answers_keep_what_a_bridge_of_the_previous_version_requires` (`tests/api.rs`) | Escrito después del código. La falta de `kind` en `POST /v1/synthesis` la mostró `save_then_list_then_search_then_patch_then_delete` (`client.rs`): `synthesis: Http("error decoding response body")`. La de `trajectories` y `skills` en la unión de una sugerencia salió de leer los structs del bridge anterior ([«Frente a lo previsto»](#frente-a-lo-previsto)) |
| R6 | `a_retired_kind_is_a_usage_error_that_says_where_its_rows_went` (`cli.rs`) | `memories list --kind episodic` se leía como `kind: Some("episodic")` |
| R7 | `a_retired_kind_is_answered_with_where_its_rows_went` (`mcp/tools.rs`) | `kind_refusal` sin cuerpo daba `None` para `episodic` |
| R7 | `a_retired_kind_is_answered_without_calling_the_daemon` (`mcp/server.rs`) | Las cinco herramientas llamaban al daemon, que no corría: `the clawg daemon is not running…` |
| R8 | `doctor_passes_over_a_store_whose_trajectories_and_skills_moved` (`crates/clawg-cli/tests/binary.rs`) | Escrito después de V32: fija que el binario nuevo abre una base que una copia congelada en V31 creó y se sembró por SQL, la lista entera como memorias y `doctor` no da ningún `warn` ni `error` del almacén. Lo que fallaba antes lo cubre el test de invariantes de arriba |
| R10 | `k_pool_mirrors_the_production_lexical_budget` y los de `report.rs` | Ajustados con la firma: `EvalScope` ya no tiene los dos pools |

**Tests que se fueron o cambiaron, y por qué:** los que sembraban trayectorias o habilidades con las funciones de inserción que este cambio borra, o fijaban un brazo, un conteo o una sección que ya no existe (las habilidades del workspace, los pools por tabla, `MemoryKind` en `PATCH`, `delete` y `restore`, los conteos por tabla de proyectos, secretos e `inspect`), se borraron o se reescribieron sobre `memory`. Las fixtures de la CLI y del MCP que simulaban filas `episodic` o `procedural` del daemon pasan a filas `memory`, y una fila sin tipo sigue probándose como la manda un daemon anterior a V32. `deleted_episodes_alone_are_not_offered_an_undelete` (`memories.rs`) se fue: todas las filas borradas son de `memory` y todas se ofrecen a `undelete`.

## La spec contra el código

| Requisito | Dónde |
|---|---|
| R1, R2, R3 | `crates/clawg-core/migrations/V32__retire_legacy_kinds.sql`, con su checksum fijado en `crates/clawg-core/src/memory/migrations.rs:76` |
| R4 | Un solo KNN, `knn_one` (`crates/clawg-core/src/memory/search.rs:460`), registrado como `knn_memory` (`:1226`); `search.knn_retry` sólo con `table=memory` (`:1242`). La convención, `no_production_code_names_the_retired_tables` (`crates/clawg-cli/tests/conventions.rs:92`), con `portability/export.rs` como única excepción (`:94`) |
| R5 | `parse_kind` y la explicación (`crates/clawg-cli/src/daemon/api.rs:220`, `:229`); `Kinded` (`:724`), `SynthesisBody` (`:868`), los conteos de `GET /v1/stats` (`:1810`, `:1823`) y `AnsweredMerge` (`:2881`) |
| R6 | `memory_kind` (`crates/clawg-cli/src/cli.rs:1016`), en `list` y `show` con `hide = true` (`:1029`, `:1077`) |
| R7 | `kind_refusal` (`crates/clawg-cli/src/mcp/tools.rs:514`); `#[schemars(skip)]` en los cinco args (`crates/clawg-cli/src/mcp/server.rs:303`, `:324`, `:334`, `:344`, `:354`) y la negativa antes de llamar al daemon (`:1318`, `:1352`, `:1372`, `:1393`, `:1445`); las descripciones en español en `crates/clawg-cli/src/mcp/tools_es.rs` |
| R8 | Los conteos de `inspect` sólo sobre `memory` (`IdRows`, `crates/clawg-core/src/memory/inspect.rs:99`), y `project_deletes` de `doctor` con ellos (`crates/clawg-cli/src/doctor/checks.rs:804`) |
| R9 | `legacy_rows`, contado sólo si la tabla existe (`crates/clawg-core/src/memory/portability/export.rs:127`) |
| R10 | `EvalScope` sin los dos pools (`crates/clawg-core/src/eval_api.rs:100`) y `KPool` (`crates/clawg-eval/src/bin/rag-eval/report.rs:32`) |

## El contrato del agente

Bendecido con `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`. En los dos idiomas cambia lo mismo: `kind` sale del `inputSchema` de `list_memories`, `read_memory`, `update_memory`, `delete_memory` y `restore_memory` (y de `required` en las tres que lo pedían); las descripciones de `delete_memory`, `list_memories`, `restore_memory`, `update_memory` y `search_memory` dejan de hablar de tablas, de ids por tabla y de kinds; los `id` de `delete_memory` y `update_memory` pierden la frase sobre el kind de un hit de `recall_context`, y `memory_type` de `list_memories` y `update_memory` deja de decir que vale sólo para `memory`. Las `instructions` no cambian. El diff en inglés:

```diff
     "delete_memory": {
-      "description": "Delete one stored memory by id and kind. Get the id from `search_memory`, `list_memories` or `recall_context` first — ids are per-kind, so the same number exists in all three tables. A memory marked `by <agent>` was saved by another agent, and deleting it takes it from every agent. Deleting the current memory of a `topic_key` makes the version it replaced current again. `restore_memory` undoes it.",
+      "description": "Delete one stored memory by id. Get the id from `search_memory`, `list_memories` or `recall_context` first. A memory marked `by <agent>` was saved by another agent, and deleting it takes it from every agent. Deleting the current memory of a `topic_key` makes the version it replaced current again. `restore_memory` undoes it.",
           "id": {
-            "description": "The id of the row to delete, as printed by `search_memory`,\n`list_memories` or `recall_context`. A `recall_context` hit with no kind\nin front of its type is a memory.",
+            "description": "The id of the memory to delete, as printed by `search_memory`,\n`list_memories` or `recall_context`.",
-          "kind": {
-            "description": "Which table the id belongs to: memory, episodic or procedural. Required,\nbecause each table numbers its rows independently — id 7 exists in all\nthree and means something different in each.",
-            "type": "string"
         "required": [
-          "id",
-          "kind"
+          "id"
     "list_memories": {
-      "description": "… Returns one line per row: id, kind, type, tier, importance, access count, status, `by <agent>` when another agent saved it, and a truncated summary.",
+      "description": "… Returns one line per memory: id, type, tier, importance, access count, status, `by <agent>` when another agent saved it, and a truncated summary.",
-          "kind": {
-            "default": null,
-            "description": "Which table to list: memory, episodic or procedural. Lists all three\nwhen omitted.",
-            "type": ["string", "null"]
-          },
-            "description": "Only rows of this memory type. Applies to `memory` rows only.",
+            "description": "Only memories of this type.",
     "read_memory": {
-          "kind": {
-            "default": null,
-            "description": "Which table the id belongs to: memory, episodic or procedural.\nDefaults to memory, the only kind saved in pieces.",
-            "type": ["string", "null"]
     "restore_memory": {
-      "description": "Undo a `delete_memory`: bring a deleted memory back, with the id and kind it was deleted with, …",
+      "description": "Undo a `delete_memory`: bring a deleted memory back, with the id it was deleted with, …",
-          "kind": {
-            "description": "The kind `delete_memory` was given. Only memory rows can be restored.",
-            "type": "string"
         "required": [
-          "id",
-          "kind"
+          "id"
     "search_memory": {
-      "description": "… with their ids, kinds and relevance scores; …",
+      "description": "… with their ids, types and relevance scores; …",
     "update_memory": {
-      "description": "Correct the importance, the type or the project of one stored memory, by id and kind, when the user says it matters more or less than it was saved with, that it is a different kind of memory, …",
+      "description": "Correct the importance, the type or the project of one stored memory, by id, when the user says it matters more or less than it was saved with, that it is a different type of memory, …",
           "id": {
-            "description": "The id of the row to change, as printed by `save_memory`,\n`search_memory`, `list_memories` or `recall_context`. A `recall_context`\nhit with no kind in front of its type is a memory.",
+            "description": "The id of the memory to change, as printed by `save_memory`,\n`search_memory`, `list_memories` or `recall_context`.",
-          "kind": {
-            "description": "Which table the id belongs to: memory, episodic or procedural.",
-            "type": "string"
-          },
-            "description": "The new type, only for kind memory, one of the listed values. Omit it\nto leave the type as it is.",
+            "description": "The new type, one of the listed values. Omit it to leave the type as\nit is.",
         "required": [
-          "id",
-          "kind"
+          "id"
```

El diff completo de los dos archivos está en el pull request.

## Frente a lo previsto

- **Dos respuestas más que un bridge anterior exige.** D6 nombraba un hit, una fila, un `PATCH`, un `/save` y `GET /v1/stats`. La suite mostró una tercera fila, la de `POST /v1/synthesis`: el `MemoryRow` del cliente exige `kind` y el núcleo ya no lo manda. Leyendo los structs del cliente anterior salió otra: el `MergeReport` exige `trajectories` y `skills`, y el bridge anterior lo lee en la respuesta de `answer_suggestion`. Las dos se mandan como D6 dice ahora, y las fija `the_answers_keep_what_a_bridge_of_the_previous_version_requires`. `Kept.table`, que la deshechura de un borrado ya no manda, no hace falta: sólo lo lee la CLI, que es la versión nueva.
- **El listado de borradas.** `clawg memories list --deleted` ofrecía `undelete` sólo si alguna fila era de `memory`; ahora todas lo son y la línea sale siempre.
- **Las líneas de un hit y de un listado no cambian.** Siguen llevando `Memory` y `memory`, como D6 pedía para no mover los tokens que mide `rag-eval --agent-view`.

## Pendiente en local

Lo que sigue necesita los datos reales o el modelo real, y no corre en la nube. Cada paso empieza de una copia nueva del directorio de datos, como en [preference-model](../archive/2026-11-06-preference-model/verify.md#medido-sobre-una-copia-de-prod): `memory.db` leída por una conexión de sólo lectura y copiada con la API de backup de SQLite en un solo paso, y `models/`, `workspaces/`, `sessions/` y `preferences.json`; nunca `daemon.json`, `token`, el socket, los locks, `metrics.db` ni `backups/`. Los binarios son builds debug con la feature `testing` de clawg-core, que muda el directorio de datos con `CLAWG_TEST_HOME` (`crates/clawg-core/src/paths.rs`) y, sin `CLAWG_TEST_EMBEDDER`, carga el modelo real de la copia. Un build debug no abre `prod`, así que cada copia, también la de `prod`, va en el `.clawg-dev` de su `CLAWG_TEST_HOME` y el daemon corre con `--profile dev --port 0`.

1. **La migración sobre una copia de `prod` y otra de `dev`.** Antes de abrirla, sobre la copia: `SELECT COUNT(*), SUM(deleted_at IS NULL) FROM` cada una de `memory`, `episodic_trajectory` y `procedural_skill`, y `SELECT max(version) FROM refinery_schema_history` (31). La puerta de la exploración contó 0 filas viejas en los dos perfiles, así que lo esperado es que V32 sólo borre las tablas. Después, con un daemon de este cambio sobre la copia: que abra y conteste `/health`; que `backups/` tenga el snapshot `pre-migration` en V31; que la base quede en V32, sin ninguna de las cuatro tablas y con `legacy_kind_move` con una fila por fila vieja; que `memory` tenga las filas de antes más las migradas, las vivas como vivas; y que `clawg doctor --profile dev` con el mismo `CLAWG_TEST_HOME` no dé ningún `error` ni ningún `warn` del almacén que la copia no diera ya con un daemon de `develop`.
2. **`search.duration` antes y después, sobre una copia.** Un daemon de `develop`, en un worktree aparte, y uno de este cambio, cada uno sobre su copia nueva del mismo store de `prod`: las mismas búsquedas (`POST /v1/search` con las 46 consultas del golden, repetidas en el mismo orden) y después `GET /v1/metrics`. Comparar `search.duration` por `arm`: `knn_episodic` y `knn_skill` ya no están, y `total` y `knn_memory` en p50 y p95, antes → después. Una búsqueda hace dos KNN y dos consultas de elegibles menos; lo que se espera es que `total` baje o quede igual, nunca que suba más que el ruido de dos corridas del mismo binario.
3. **`rag-eval --diff` con el modelo real y el golden de `docs/eval/`.** `rag-eval --real --tag retire-legacy-kinds-tip`, y `--agent-view` con la misma tag, con el `rag-eval` de este cambio sobre el golden de 46 casos con el sha256 de la línea base, y `cargo run -p clawg-eval --bin rag-eval -- --diff` contra los archivos de la línea base de `openspec/config.yaml`. Umbral fijado en la [propuesta](proposal.md#cómo-se-mide): **ningún caso se mueve**, ni en `similarity` ni en `retrieval` ni en `search.found`, porque el golden no tiene filas de estos tipos y el pool de `memory` no cambia.

## Veredicto

Sin CRITICAL en Linux. El CI del pull request revisa Windows. Los tres puntos de «Pendiente en local» quedan como WARNING hasta medirlos sobre copias.
