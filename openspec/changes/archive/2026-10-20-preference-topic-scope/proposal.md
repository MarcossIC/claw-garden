# preference-topic-scope — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | topic-revisions (27, archivado); antes de 44 | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`71b4276`).

1. **La búsqueda cruza proyectos.** Desde un proyecto, `memory_project_predicate` lee las filas del proyecto, las que no tienen proyecto y toda `user_preference` (`crates/clawg-core/src/memory/search.rs:92-99`). El hit (`RankedHit`, `search.rs:107-130`; `Hit`, `crates/clawg-cli/src/client.rs:166-192`) lleva `topic_key` pero no el proyecto de la fila.
2. **El puente muestra la clave igual.** `format_hits` imprime ` topic=K` en todo hit propio (`crates/clawg-cli/src/mcp/tools.rs:111-113`), y `read_memory` en su cabecera (`format_save`, `tools.rs:182-183`).
3. **La clave revisa sólo en su proyecto.** `SCOPE_ROWS_SQL` compara agente, `COALESCE(project_id, '')` y clave (`crates/clawg-core/src/memory/topic.rs:81-86`), y el save toma el proyecto del puente o de su `project_path` (`crates/clawg-cli/src/mcp/server.rs:746`).
4. **El protocolo manda reusarla.** `protocol.md:72-74`: «`search_memory` and `read_memory` show a memory's key as `topic=K`: save its new version under that key, the one shown, never a new one». Un agente que encuentra una preferencia propia de otro proyecto y guarda la versión nueva bajo esa clave deja dos filas vivas con la misma clave, y la respuesta (`saved #N`) no dice que no reemplazó nada.

## Objetivo que sirve

1: una clave de tema revisa de verdad lo que promete revisar. Va antes de 44 (history-import), que trae claves de tema en volumen.

## Decisiones

Las decisiones abiertas de la exploración se cierran así:

- **Enfoque:** A. El puente deja de mostrar ` topic=K` donde un save no revisaría la fila. B (que la clave alcance otros proyectos) se descarta porque esconde una fila de otro proyecto; C (avisar después del save) y D (mostrar el proyecto) quedan afuera.
- **Qué proyecto se compara:** el que tomaría un save con el mismo `project_path`: en `search_memory`, el de la búsqueda (el del puente o el de `project_path`); con `all_projects`, el mismo que sin él, que es donde guardaría un `save_memory` con ese `project_path`. `read_memory` no toma `project_path`, así que compara con el proyecto del puente, que es donde guarda un save sin él.
- **Superficies:** `search_memory` y `read_memory`, las dos que nombra el protocolo como el lugar de donde se toma la clave. `list_memories` y `restore_memory` siguen igual: el primero muestra la historia de las versiones, y ninguno de los dos es de donde el protocolo manda copiar la clave.
- **Filas sin proyecto:** como las de otro proyecto, salvo que el save tampoco tenga proyecto (un puente sin raíz): es lo que hace el alcance (`COALESCE(project_id, '')`).
- **Protocolo:** la línea de `protocol.md` dice que la clave se muestra sólo donde guardar bajo ella reemplaza la memoria, y que una memoria de otro proyecto no la muestra.
- **C:** afuera. Se suma si se mide que los agentes inventan la misma clave sin verla.

## Alcance

1. `clawg-core`: `RankedHit.project_id` y `SaveText.project_id`, el proyecto de la fila, que el daemon manda siempre (`null` para una fila sin proyecto). Las consultas de búsqueda ya leen de `memory`; suman la columna.
2. `clawg-cli`, cliente: `Hit.project_id` y `SavedText.project_id` distinguen un campo ausente (un daemon anterior) de uno `null` (una fila sin proyecto).
3. `clawg-cli`, puente: una regla pura decide si la clave se muestra; `format_search`/`format_hits` y `format_save` reciben el proyecto del save; `search_memory_impl` y `read_memory_impl` se lo pasan.
4. `protocol.md`: la línea de la clave.
5. Docs: `.claude/CLAUDE.md`, y al archivar `openspec/specs/mcp` (y la cita de `specs/pieces`).

## Fuera de alcance

- El alcance de una clave (`SCOPE_ROWS_SQL`, revival, `restore`, import), el esquema y los datos.
- Avisar en la respuesta del save (C).
- `list_memories`, `restore_memory`, `recall_context` y `session-start`.
- Ranking: el orden y el puntaje de los hits no cambian; no se corre `rag-eval`.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), con el embedder falso, sin red ni perfil:

- `format_hits` con un hit propio de otro proyecto, o sin proyecto, y clave no muestra ` topic=`; hoy la muestra;
- un hit de un daemon que no manda `project_id` la muestra como hoy;
- `format_save` de una fila de otro proyecto no la muestra;
- de punta a punta con el puente: una preferencia guardada bajo `K` en otro proyecto aparece en `search_memory` desde el del puente sin ` topic=`, y con `project_path` de su proyecto con ` topic=K`; guardar bajo `K` desde el del puente deja las dos filas vivas, que es por qué la clave no se muestra;
- `POST /v1/search` y `GET /v1/memories/{kind}/{id}/save` mandan `project_id`, `null` incluido.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding, y no cambia qué filas se leen ni cuáles se revisan: sólo qué muestra el puente. Un puente nuevo contra un daemon anterior muestra la clave como hoy; un puente anterior ignora el campo nuevo. Volver atrás es volver al binario anterior.
