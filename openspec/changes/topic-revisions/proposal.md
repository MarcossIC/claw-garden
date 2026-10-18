# topic-revisions — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | conflict-candidates (archivado) | ver [`state.yaml`](state.yaml) |

## Problema

Una decisión que evoluciona («la base es X», después «migramos a Y») termina en dos filas vivas que puntúan igual. Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código). Lo que sigue se volvió a comprobar contra `main` (`880aa07`), ya con conflict-candidates:

1. **Sólo un texto idéntico reemplaza algo.** La dedup refuerza una fila viva con el mismo agente, proyecto y `content_hash` (`REINFORCE_LOOKUP_SQL`, `crates/clawg-core/src/memory/store_insert.rs:284-291`); cualquier reescritura inserta otra fila (`crates/clawg-core/src/memory/worker.rs:753-776`).
2. **El reemplazo depende de la distancia.** Si el texto nuevo cae cerca, el detector anota un par y la respuesta del save lo nombra en `related` (`worker.rs:760-767`); el agente puede juzgarlo `supersedes`, salvo que un lado sea una preferencia o una decisión y el par esté pendiente: entonces tiene que pasar por el usuario (`crates/clawg-core/src/memory/dispute.rs:333-338`). Si cae lejos (una decisión reescrita más larga), no hay ninguna marca.
3. **No hay forma de decir «esto reemplaza aquello» sin el id.** Entre sesiones el agente no tiene el id de la versión vigente; tiene que buscarla, y la búsqueda puede traer otra fila.

## Decisiones

Tomadas sobre las [decisiones abiertas](exploration.md#decisiones-abiertas):

- **Enfoque E2:** encadenado por clave, sin mutar contenido. Un save con `topic_key` inserta filas nuevas por el camino de siempre y, en la transacción que termina el save, las filas vivas anteriores con esa clave pasan a `superseded` con `superseded_by` apuntando a la primera fila nueva. El id cambia en cada revisión.
- **No hay `replaces: [ids]` ni `content` en `update_memory`** en este cambio: la puerta por id queda como está (`resolve_conflict` con `supersedes`).
- **Preferencias y decisiones también se revisan por clave**, sin pasar por el usuario: la clave es una intención explícita del agente, la versión anterior queda en la historia y se recupera. La respuesta del save nombra lo que reemplazó.
- **Herencia:** la fila nueva toma el mejor tier y el mayor `access_count` de las que reemplaza. `memory_type` e `importance` son los del save, o los de la versión anterior si el save no los trae.
- **Formato de la clave:** segmentos `[a-z0-9._-]+` separados por `/`, a lo sumo 80 caracteres, sin segmento vacío. Una clave inválida es un `400` que nombra `topic_key`, antes de la cola.
- **Alcance:** agente + proyecto + clave, con `COALESCE(project_id, '')` como la dedup. Otro agente o proyecto con la misma clave no toca nada.

Decisiones menores, tomadas con el criterio de la exploración:

- **Pieza compartida con otro save:** no se reemplaza; se desengancha (su clave pasa a `NULL`) y queda viva para el otro save.
- **Texto idéntico a otra fila:** la dedup la refuerza como hoy. Si esa fila no tiene clave, la toma; si tiene otra, la conserva. Las versiones anteriores de la clave se reemplazan igual.
- **Unicidad entre vivas:** comprobación del worker (único consumidor secuencial) y de `restore`, no un índice único: las piezas de un save largo comparten la clave.
- **Sin aviso de «pasá `topic_key`»** en la respuesta de un save sin clave, y **sin búsqueda por clave** en `search_memory`: cambian tokens o ranking y se miden aparte.

## Alcance

1. Migración V16: columna `topic_key` (nullable) en `memory` e índice parcial `(agent_id, COALESCE(project_id, ''), topic_key)` sobre filas no borradas con clave.
2. `validate_topic_key`, pura, con su tabla de casos.
3. `topic_key` opcional en `MemoryEvent` (`#[serde(default)]`), `POST /v1/memories`, `SaveArgs` y `save_memory`. El daemon la devuelve en el cuerpo del save.
4. Revisión en el worker: las filas nuevas llevan la clave; al terminar el save se reemplazan las anteriores, se hereda tier y accesos, y el informe lista `replaced`.
5. El detector deja fuera las filas vivas de la misma clave.
6. Un save largo con clave reemplaza todas las piezas propias del save anterior; una pieza compartida se desengancha.
7. `restore` rechaza una fila cuya clave ya tiene otra viva, nombrándola.
8. `save_memory` dice qué reemplazó, y avisa si el daemon es anterior y guardó sin clave. `list_memories` muestra la clave y a quién reemplazó cada fila `superseded`.
9. `protocol.md`, `.claude/CLAUDE.md` y las specs `ingest`, `update`, `pieces`, `disputes`, `storage` y `mcp`.

## Fuera de alcance

- Reemplazo en el lugar con tabla de versiones (E1) y `replaces` por id (E3, E3b).
- Sugerir claves por heurística, o el aviso de posible revisión.
- Búsqueda por clave en `search_memory`, y cualquier cambio de ranking.
- Revisar filas de otro agente.
- Deshacer una revisión como operación propia: para un save de una pieza se deshace con `resolve_conflict` (`keep` = la vieja, `supersedes`), que ya acepta dos filas enlazadas por `superseded_by` (`dispute.rs:135-155`).
- Una herramienta MCP nueva para la historia.

## Datos guardados

- **Esquema:** una columna nullable y un índice en `memory`. Ninguna fila cambia al migrar: todas quedan con `topic_key = NULL` y se comportan como hoy.
- **Antes de migrar** se escribe la copia `pre-migration` (`crates/clawg-core/src/memory/db.rs`).
- **Rollback:** un binario anterior rechaza la base en V16 sin tocarla (`SchemaNewer`, `check_schema`). Se vuelve atrás con `clawg restore` del snapshot `pre-migration`, o con `DROP INDEX idx_memory_topic; ALTER TABLE memory DROP COLUMN topic_key; DELETE FROM refinery_schema_history WHERE version = 16;`. Las filas `superseded` por una revisión quedan así, igual que las que reemplazó un veredicto.
- **Outbox:** un evento con clave que repone un binario anterior pierde la clave y se guarda como fila nueva, sin reemplazar; no pierde el texto.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- guardar dos veces con la misma clave deja una sola fila viva con la clave, la nueva; la vieja queda `superseded` por ella, fuera de la búsqueda por KNN y por texto;
- la misma clave desde otro agente o proyecto no reemplaza nada;
- un save largo con clave reemplaza las piezas propias del anterior y desengancha la compartida;
- la base sembrada en V15 abre con la columna en `NULL` y sus filas siguen alcanzables;
- una clave inválida es un `400` que nombra `topic_key`;
- la respuesta de `save_memory` nombra lo reemplazado.

`rag-eval --diff`: ningún caso del golden set lleva clave, así que el ancla debe dar cero casos movidos. La [compuerta previa](exploration.md#compuerta-previa) de la exploración (cuántas decisiones que evolucionan quedan sin marca) necesita el modelo real; como en conflict-candidates, se construye ahora y se mide donde estén el modelo y `docs/eval/`. Queda pendiente en `verify.md`.
