# knowledge-synthesis — Diseño

Las decisiones de producto (limpieza + opción c, qué es una conclusión, las tablas viejas se quedan) están en la [propuesta](proposal.md). Acá van las decisiones de implementación.

## D1. Dónde vive la procedencia

| Opción | A favor | En contra |
|---|---|---|
| **a. Tabla nueva `memory_derivation (memory_id, source_id, created_at)`** | Una fila por enlace; se consulta en las dos direcciones con un índice; no toca `memory` | Migración, y una versión anterior rechaza la base |
| b. Columna JSON nueva en `memory` | Una lectura por fila | También es migración, y "quién cita a X" es un `json_each` sobre toda la tabla |
| c. Reusar `created_by_ref` o `source_session_id` | Sin migración | Cambia el significado de columnas que ya tienen uno (`created_by_ref` es el agente, `store_insert.rs:184`); un lector viejo mostraría ids como si fueran un agente |
| d. Sin procedencia: el agente escribe "(de #12 y #34)" en el texto | Sin migración | Nada distingue una conclusión; el texto entra al vector y al FTS; "qué falta sintetizar" no se puede calcular |

**Elegida a.** La pregunta caliente de la síntesis es "¿esta memoria ya la respalda una conclusión viva?", o sea `source_id → memory_id`, y en b o c eso recorre toda la tabla. `PRIMARY KEY (memory_id, source_id)` evita duplicados y responde la otra dirección; un índice sobre `source_id` responde ésta. Sin claves foráneas: `memory` nunca se borra físicamente (borrar es `deleted_at`), y una FK obligaría a pensar el orden de inserción dentro de la transacción sin proteger nada.

**Migración aditiva** `V10__synthesis.sql`, con las dos tablas (D4). Se toca `migrations.rs` en el mismo commit (lo pide su comentario) y se agrega la fila a `published_migration_checksums_are_pinned`.

## D2. El enlace se escribe en la transacción de la fila

`MemoryEvent` y `NewMemoryRecord` llevan `derived_from: Vec<i64>` (`#[serde(default)]` en el evento, por el volcado). `insert_memory_with_vector` inserta los enlaces después de `memory_vectors`, antes del `commit`; `reinforce_if_duplicate` los inserta con `INSERT OR IGNORE` en su transacción. Así no existe una conclusión a medias: o está con sus fuentes o no está.

La alternativa, enlazar todo al final del evento en una transacción aparte, deja una ventana (un corte de luz, un `clawg stop`) donde la fila existe sin procedencia, y obliga a reunir los ids de insertados y reforzados en el worker.

El auto-enlace (refuerzo de la propia fuente) se filtra en SQL con `WHERE source_id <> memory_id`, para no repetir la regla en cada llamador.

## D3. Validar las fuentes en el daemon, antes de encolar

El save es asíncrono: el worker corre después de que el daemon respondió (o mientras espera la confirmación). Si la validación viviera en el worker, un id malo se descubriría tarde, y con el embedder cargando la respuesta ya habría sido `queued`.

- **Puro** (`api.rs`): `distinct_sources(pedidos)` deduplica, ordena y aplica el tope de 50; `missing_sources(pedidos, vivos)` devuelve los rechazados. Se prueban sin base.
- **Con base** (`synthesis.rs`): `existing_memory_ids(store, ids, agente)`, un `SELECT id FROM memory WHERE id = ? AND deleted_at IS NULL AND (? IS NULL OR agent_id = ?)` por id. El daemon pasa el agente del save: una fila de otro agente es una fuente que no existe, como en visibility-policy.
- **Carrera aceptada:** una fuente borrada entre la validación y el insert queda enlazada igual. Es lo mismo que borrarla un segundo después: la conclusión se queda y el enlace es historia (R6).

## D4. Revisión por alcance: tabla, no derivada

El recordatorio necesita saber "qué llegó desde la última vez que el agente miró". Opciones:

| Opción | A favor | En contra |
|---|---|---|
| a. Derivarlo de la conclusión más nueva del alcance | Sin estado | Si el agente revisa y no concluye nada, el recordatorio no se apaga nunca |
| **b. `synthesis_review (agent_id, scope_key, reviewed_at)`, clave `(agent_id, scope_key)`** | Se apaga al revisar, concluya o no | Una tabla más en la misma migración |
| c. Columna en `project` (V9) | Reusa una tabla | No cubre el alcance "todo el store", y es `ALTER TABLE` sobre una tabla con datos |

**Elegida b.** Va en la misma migración que D1, así que no suma una migración. `agent_id` es el agente o `''` para el dueño de la base; `scope_key` es el `project_id` o `''`. La revisión es por agente porque cada agente sólo ve sus memorias: que un agente revise no puede apagar el recordatorio de otro que nunca las miró.

## D5. La síntesis es una lectura más una marca

`synthesis.rs` (nuevo, en `clawg-core/src/memory/`):

- `synthesis_core(store, SynthesisParams { agent_id, project_id, limit }, now) -> Synthesis { pending, pending_total, conclusions }`: tres consultas y un upsert en `synthesis_review`, todo en un `conn.call` con una transacción, para que el total y la lista vengan del mismo estado.
- `unreviewed_count(store, agent_id, project_id) -> usize`: un `COUNT` con la misma condición de pendiente más `created_at > reviewed_at`.
- La condición de "pendiente" es una sola constante SQL que usan las dos, igual que la expresión del estado de reposo en dispute-resolution:

```sql
m.deleted_at IS NULL AND m.status <> 'superseded'
AND NOT EXISTS (SELECT 1 FROM memory_derivation d WHERE d.memory_id = m.id)
AND NOT EXISTS (
  SELECT 1 FROM memory_derivation d JOIN memory c ON c.id = d.memory_id
  WHERE d.source_id = m.id AND c.deleted_at IS NULL AND c.status <> 'superseded'
)
```

- Las filas se devuelven como `MemoryAdminRow`, el mismo tipo del listado, para que `derived_from` y el renderizado sean uno solo. `SELECT_MEMORY` suma `derived_from` con un subselect `json_group_array` ordenado; `map_memory_row` lo convierte en `None` cuando está vacío.
- Sin filtro de texto (spec R7): la síntesis devuelve lo que falta revisar, y la búsqueda por texto ya es `search_memory`.
- No registra accesos: mirar memorias para sintetizar no es usarlas, y contarlo inflaría los tiers (retrieval-hygiene).

## D6. El recordatorio va en el puente, no en el prompt

| Opción | A favor | En contra |
|---|---|---|
| a. Una línea dentro de `build_memory_prompt` | Un lugar | Cambia los tokens que mide `rag-eval --agent-view` y entra al terreno de context-density |
| **b. `ContextBody.unreviewed` y la línea la agrega `recall_context`** | El prompt no cambia; el texto es puro y testeable en `tools.rs` | Un `COUNT` más por pedido de contexto |

**Elegida b.** El `COUNT` usa los índices de `memory` (`idx_memory_scope_project`) y el de `memory_derivation.source_id`; es del mismo orden que las consultas del workspace que ya corren en cada `recall_context`. `client.context` pasa a devolver el cuerpo entero; un daemon viejo no manda el campo y `#[serde(default)]` lo deja en 0 (sin recordatorio).

El umbral (20) es una constante en `tools.rs`, junto al texto. No se configura: no hay configuración en el puente, y un número equivocado se ve rápido y se cambia en una línea.

## D7. Qué se borra y qué se muda

Se borra (sin llamador en producción, `rg` lo confirma en cada commit):

- `memory/{extractor,miner,resolver,episodic_writer}.rs`;
- `providers/` y `services/`;
- de `models`: `ToolCall`, `ToolFunction`, `ToolDefinition`, `ToolFunctionDef`, `ChatRequest`, `ChatStreamResult`, `LlmError` y las variantes de `StreamEvent` y `AppError` que sólo producían los proveedores. Se quedan `Role`, `ChatMessage`, `current_time_millis`, `StreamEvent::EmbedderDownload` y lo que use el resto del crate (lo decide el compilador);
- de `store_insert.rs`: `NewEpisodicTrajectoryRecord`, `insert_episodic_trajectory`, `update_trajectory_root_cause`, `fetch_prior_trajectories`, `promote_to_procedural_skill` y sus tests;
- de `admin.rs`: `get_failure_histogram_core`, `get_skill_steps_trend_core` y sus tipos;
- ninguna dependencia: `reqwest`, `async-trait`, `futures`, `uuid` y `mockito` siguen en uso (descarga del modelo, embedder, worker y sus tests).

Se muda: `skill_embedding_text` a `maintenance.rs`, su único usuario vivo.

Se queda todo lo que lee o administra las tablas (punto 1 del alcance de la propuesta, R1 de la spec). Los tests que ya cubren esa lectura usan `insert_episodic_trajectory` y `promote_to_procedural_skill` para sembrar; esos dos escritores se quedan detrás de `#[cfg(test)]` (y `testing` para el primero, que usan los tests HTTP de `clawg-cli`), así las filas de prueba son exactamente las que escribía la app. Reescribir unas 30 siembras a SQL crudo era más código y menos fiel.

## D8. Orden de los commits

Primero la limpieza (con R1 cubierta por los tests que ya existen) en commits que compilan solos, después la migración, la procedencia, la síntesis, el HTTP y el MCP. Así la limpieza se puede revisar (o revertir) sin mirar la funcionalidad nueva, y si se elige "sólo limpieza" la rama se corta después del commit de limpieza.
