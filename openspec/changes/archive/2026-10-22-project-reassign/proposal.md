# project-reassign — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 | M | agent-authorship (76, archivado el 2026-10-22) | ver [`state.yaml`](state.yaml) |

## Problema

Una fila de `memory` queda para siempre en el proyecto con que se guardó. El detalle, con cada lugar citado, está en la [exploración](exploration.md#problema-verificado-contra-el-código). En resumen:

1. **No hay forma de acomodar una fila.** `PATCH /v1/memories/{kind}/{id}` y `update_memory` sólo cambian `importance`, `memory_type` y `user_answered` (`UpdateRequest`, `crates/clawg-cli/src/daemon/api.rs`, con `deny_unknown_fields`; `update_memory_core`, `crates/clawg-core/src/memory/admin.rs`). Lo único que mueve filas es el re-key de `register_project` (`crates/clawg-core/src/memory/project.rs`), que traslada todas las de un id viejo sin mirar choques. Una fila guardada sin proyecto, o en el equivocado, sólo se acomoda con SQL a mano y el daemon parado.
2. **El visor no puede listar lo que hay que acomodar.** El filtro de proyecto es `project_id = ?` (`push_filters`, `admin.rs`); no existe «sin proyecto».
3. **history-import (44) lo necesita:** el 43 % de lo que trae (1.594 de 3.713 notas del export medido) entra sin proyecto, y su propuesta lo deja «para otro cambio». Es este.

Sin proyecto no es neutro: la fila entra al KNN y al detector de todos los proyectos, nunca es LAST SESSION y nunca entra en la síntesis de un proyecto. La regla de que una fila sin proyecto se ve desde todos los proyectos se queda; el cambio sólo permite reubicar.

## Decisiones

Tomadas en la exploración ([«Decisiones tomadas»](exploration.md#decisiones-tomadas)) y ajustadas al modelo que dejó 76 (cualquier agente cambia lo que alcanza; dedup, clave y detector por proyecto dentro del alcance del que guarda):

- La operación cambia `project_id` y, en una fila sin autor, `scope` (el CHECK de `V1__init.sql` lo exige). Nada más: ni vectores, ni índice de texto, ni `last_modified_at`, ni `update_count`, ni el detector. No necesita el embedder.
- La unidad es el save: piezas, saves que comparten una fila, cadena de revisiones (causa `revision`) y lo que un borrado revivió se mueven juntos, borrados incluidos.
- Sólo `memory`. Un movimiento en otra tabla es un 400.
- El destino tiene que estar en `project`.
- Pares y derivaciones no se mueven: son ids. El reporte nombra los pares que quedan entre dos proyectos.

Las [decisiones abiertas](exploration.md#decisiones-abiertas) se cierran con la opción recomendada:

1. **Choque en el destino: rechazar (1.a).** Una fila viva del destino al alcance del lector con la misma `topic_key` o el mismo `content_hash` rechaza la unidad, nombrando la fila que la sostiene; en el lote se omite y se informa, y el resto sigue. Dos unidades del mismo lote que chocarían entre sí en el destino: se mueve la primera (la de id menor).
2. **Unidad: el cierre (2.a).** Lo arrastrado sin nombrarse se informa como `pulled_in`. Un cierre que abarca filas de más de un proyecto se rechaza (`spans_projects`), y uno con una fila fuera del alcance del lector también (`out_of_reach`), sin nombrarla.
3. **Vuelta atrás: registro, snapshot y `--undo` (3.c).** Cada fila movida, por cualquier puerta, deja una fila en `memory_move` (V21) con el movimiento, su proyecto y `scope` de origen y de destino y el momento. El lote toma antes un snapshot `pre-move`. `clawg projects move --undo ID` devuelve, con las mismas reglas, las filas de ese movimiento que siguen donde las dejó; la vuelta es otro movimiento registrado.
4. **Puerta para los agentes: `update_memory` (4.a)** con `to_project_path` (el proyecto de esa ruta, que tiene que ser un directorio) o `clear_project`. Ausentes, el proyecto no cambia.
5. **Comando masivo: `clawg projects move` (5.a)** con `--to PATH`, `--to-project ID` o `--to-none`.
6. **Filas protegidas (6.a):** un `PATCH` que mueve una unidad con una `user_preference` o una `architectural_decision` viva pide `user_answered`, como sacarle el tipo. El lote es del usuario y no la pide.
7. **Destino registrado (7.a):** el daemon exige el id en `project` (404 si no); el puente y `--to PATH` registran la raíz antes, como `projects add` pero sin `index_docs`. El chequeo de `doctor` para filas bajo un id no registrado queda fuera.
8. **Sin detector al mover (8.a).** El reporte cuenta las filas protegidas vivas que llegan al destino.
9. No aplica a este cambio (lo guarda 44).
10. **Filtros del lote:** `--id` (repetible), `--from-project ID`, `--from-none`, `--here`, `--agent`, `--type`, `--source`, `--session`, `--text` e `--include-deleted`; al menos uno de selección. `GET /v1/memories` y `clawg memories list|search` suman «sin proyecto» (`no_project`, `--no-project`).

## Alcance

1. `crates/clawg-core/src/memory/relocate.rs`: el plan puro (cierre ya leído, choques, `scope`) y su lectura y aplicación dentro de una transacción de la conexión única, con el registro.
2. V21 `memory_move` y `Reason::PreMove`.
3. `PATCH /v1/memories/memory/{id}` con `project_id` o `no_project` (con o sin `importance`/`memory_type`, todo en una transacción); 400, 404 y 409 sin escribir; `project_id` y lo que se arrastró en la respuesta.
4. `no_project` en `ListParams`, `GET /v1/memories` y `clawg memories list|search`.
5. `POST /v1/memories/move` (`dry_run`, selección o `undo`, destino) con su reporte y el snapshot; `clawg projects move`.
6. `update_memory` con `to_project_path` y `clear_project`, `agent_contract.json` y una línea en `protocol.md`.
7. Las specs (`update`, `project`, `mcp`, `visibility`, `storage`) y `.claude/CLAUDE.md`.

## Fuera de alcance

- Reconciliar un choque (`history`, reemplazar, fundir) y correr el detector al mover.
- `episodic_trajectory` y `procedural_skill`.
- La identidad de un proyecto, los clones y `clawg projects merge` (30), que conviene construir sobre este motor.
- Mover la sección USER de un workspace o el índice de docs.
- Un chequeo de `doctor` para filas bajo un id que no está en `project`.
- Guardar el nombre de proyecto de la fuente y filtrar por él: es de 44.
- Una puerta masiva para los agentes.
- El ranking: `rag-eval` no aplica.

## Datos guardados

- **Esquema:** V21 crea `memory_move` y su índice. Ninguna fila existente cambia.
- **Antes de migrar** se escribe el snapshot `pre-migration` (`crates/clawg-core/src/memory/db.rs`).
- **Rollback del esquema:** un binario anterior rechaza la base en V21 sin tocarla (`SchemaNewer`). Se vuelve con `clawg restore` del snapshot `pre-migration` o, para conservar lo guardado después, primero deshaciendo los movimientos (`clawg projects move --undo`) y luego:

  ```sql
  DROP TABLE memory_move;
  DELETE FROM refinery_schema_history WHERE version = 21;
  ```

  Una fila movida y no deshecha queda en su proyecto nuevo, que la versión anterior lee igual.
- **Rollback de un movimiento:** `clawg projects move --undo ID`, o el snapshot `pre-move` del lote con `clawg restore`, que descarta lo guardado después. A mano: `UPDATE memory SET project_id = m.from_project, scope = m.from_scope FROM memory_move m WHERE m.move_id = ID AND m.row_id = memory.id AND memory.project_id IS m.to_project`.
- **Dos escritores de `project_id`:** el SQL de vuelta del re-key (`V9__project.sql`) mueve las filas que hoy están en el proyecto que anotó, así que una fila re-keyada de A a B y movida de C a B volvería a A; la vuelta de un movimiento sólo toca filas que siguen en su destino.
- **Volver a importar un export de clawg después de mover duplica:** una fila está presente si coinciden agente, proyecto, hash y fecha, y la movida ya no tiene el proyecto del archivo.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- una fila sin proyecto se mueve a uno y se encuentra desde él y no desde otro; pasar a sin proyecto la muestra desde todos;
- una fila sin autor pasa de `project` a `global` y al revés;
- las piezas de un save, dos saves que comparten una fila y la cadena de una clave se mueven juntas, y borrar la vigente movida sigue reviviendo la anterior;
- un gemelo vivo o un titular de la clave en el destino rechaza el movimiento, nombra la fila y no cambia nada; dos saves del lote que chocarían se resuelven moviendo el primero;
- una fila borrada se mueve con su save y se restaura en el proyecto nuevo;
- mover una fuente reemplazada no hace reaparecer conclusiones por revisar;
- una `session_summary` movida es la LAST SESSION del destino;
- un `--dry-run` informa lo mismo que el movimiento y no escribe nada;
- cada fila movida queda registrada y `--undo` la devuelve, sin tocar una que se movió otra vez después;
- un destino no registrado es un 404; mover una fila protegida por `PATCH` pide `user_answered`;
- la base sembrada en V20 abre con `memory_move` vacía y cada fila igual;
- `GET /v1/memories?no_project=true` lista las filas sin proyecto; `update_memory` con `to_project_path` mueve el save; `clawg projects move` mueve, informa y deshace con el binario real.
