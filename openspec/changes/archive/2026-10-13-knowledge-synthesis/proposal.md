# knowledge-synthesis — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 5 | M | — | ver [`state.yaml`](state.yaml) |

## Problema

El objetivo 5, extraer conclusiones con el tiempo, no tiene ningún camino vivo. Detalle verificado en la [exploración](exploration.md); lo que decide esta propuesta:

1. **El pipeline LLM está compilado y muerto.** `extractor.rs`, `miner.rs`, `resolver.rs` y `episodic_writer.rs` son módulos públicos sin llamador en producción (`crates/clawg-core/src/memory/mod.rs:64-67`), igual que `providers` y `services` (`crates/clawg-core/src/lib.rs:10, 14`). `serve` sólo lanza el bootstrap del embedder, la ingesta y el mantenimiento (`crates/clawg-cli/src/daemon/serve.rs:87-103`). Lo que los alimentaba (la app de escritorio, su configuración, credenciales y runtime de agente) se borró en `5678ded`. Son unas 6.200 líneas que sólo ejercitan sus propios tests.
2. **Dos lectores de esas tablas tampoco tienen llamador:** `get_failure_histogram_core` y `get_skill_steps_trend_core` (`crates/clawg-core/src/memory/admin.rs:889, 954`) eran comandos de la app.
3. **Las tablas `episodic_trajectory` y `procedural_skill` sí pueden tener datos.** Hasta `5678ded`, la app de escritorio corría el miner al cerrar cada sesión y escribía trayectorias y skills (`5678ded^:src-tauri/src/memory_runtime.rs:264-340`) en el mismo directorio de datos (`5678ded^:src-tauri/src/state.rs:136`). Una base existente puede tener filas ahí, y hoy se recuperan: KNN de 5 + 5 en cada búsqueda (`crates/clawg-core/src/memory/search.rs:35-36`), la sección de skills del workspace (`crates/clawg-core/src/memory/workspace.rs:319-360`), `list_memories`, `GET` y `DELETE`. La opción a de la exploración (borrar las tablas y sus ramas KNN) escondería esas filas, y eso lo prohíbe `config.yaml`.
4. **No hay forma de registrar una conclusión como tal.** El agente puede guardar "el usuario prefiere X" con `save_memory`, pero nada distingue esa fila de un hecho suelto, nada dice de qué memorias salió y nada le pide al agente que la escriba. El detector de contradicciones, además, puede disputar la conclusión contra sus propias fuentes: son filas cercanas, del mismo tema y con tipos que cargan afirmaciones (`crates/clawg-core/src/memory/worker.rs:522-525`, `crates/clawg-core/src/memory/contradiction.rs:122`).

## Decisiones (cierran las abiertas de la exploración)

**Qué es una conclusión.** Una afirmación que generaliza dos o más memorias guardadas: una preferencia del usuario, una convención o lineamiento del proyecto, un error recurrente con su arreglo, o una tendencia en las decisiones. Se guarda como una memoria más, con el tipo que le corresponde (`user_preference`, `architectural_decision`, `semantic_fact`), y con la lista de memorias de las que sale. Esa lista es lo que la hace conclusión. No reemplaza a sus fuentes: las fuentes siguen intactas y recuperables.

**Limpieza + opción c (síntesis dirigida por el agente).** Ni a ni b.

- **a** (borrar tablas y ramas KNN) escondería las filas que la app de escritorio dejó (punto 3). Sí se borra el código sin llamador, que no guarda ni lee datos de nadie.
- **b** (revivir extractor y miner con hooks de Claude Code y un proveedor) necesita clave o modelo local, un costo por sesión, y leer transcripts con datos sensibles. Nada de eso existe hoy y el código sigue en el historial de git si alguna vez se elige.
- **c**: el agente ya es un LLM, ya recupera memorias y ya sabe guardar. Le falta qué mirar, cómo marcar lo que escribe como conclusión y un recordatorio. Es el mismo razonamiento que eligió la opción a de dispute-resolution, y es lo que respalda la evidencia de la exploración (sistemas basados en archivos donde el propio agente escribe sus conclusiones).
- Costo aceptado: la calidad depende del agente y sin agente no hay síntesis. Es el comportamiento de hoy, no una regresión.

**El recordatorio no es un trabajo en segundo plano.** `recall_context` suma una línea cuando hay suficientes memorias nuevas sin revisar en el alcance. El agente decide si sintetiza.

**`ChunkerKind::Code` y su `#[allow(dead_code)]` no entran.** El chunker de código es de project-indexing (objetivo 3); borrarlo acá sería decidir por ese cambio.

## Alcance

1. **Limpieza** (`clawg-core`): borrar `extractor`, `miner`, `resolver`, `episodic_writer`, `providers`, `services`, los tipos de `models` que sólo usaban ellos y los dos lectores muertos de `admin.rs`; los escritores de trayectorias y skills de `store_insert.rs` quedan sólo para tests. Las tablas, sus vectores, su KNN, el workspace, el listado, `GET`, `DELETE`, `PATCH` y el re-embed de `procedural_skill` siguen igual (`skill_embedding_text` se muda a `maintenance.rs`, su único usuario vivo).
2. **Procedencia** (`clawg-core`): `save_memory` acepta `derived_from`, la lista de memorias de las que sale la fila. Se guarda en una tabla nueva, `memory_derivation`, en la misma transacción que el insert o el refuerzo. El detector de contradicciones no compara una fila contra sus fuentes. `GET` y `list_memories` muestran `derived_from`.
3. **Síntesis** (`POST /v1/synthesis` y herramienta MCP `synthesize`): devuelve las memorias del alcance que todavía no respalda ninguna conclusión viva, las conclusiones que ya existen y las instrucciones de qué guardar. Registra cuándo se revisó ese alcance.
4. **Recordatorio:** `POST /v1/context` devuelve cuántas memorias nuevas hay sin revisar desde la última síntesis del alcance, y el puente agrega una línea al final de `recall_context` cuando pasan de un umbral. Las instrucciones del servidor MCP nombran `synthesize`.

## Fuera de alcance

- Revivir el pipeline LLM (opción b) o ejecutar algo en segundo plano.
- Borrar `episodic_trajectory`, `procedural_skill`, sus vectores o las ramas que las leen.
- El chunker de código (`ChunkerKind::Code`): es de project-indexing.
- Compartir memorias entre agentes (parte C de visibility-policy). `synthesize` y `derived_from` siguen la regla que visibility-policy ya dejó en `main`: un agente sólo revisa y cita sus propias filas.
- Un tipo de memoria nuevo para las conclusiones: cambiaría tiers y pesos de ranking, y eso se mide aparte.
- Prompts MCP (slash commands): la herramienta y el recordatorio alcanzan; un prompt se puede sumar después sin cambiar el contrato.

## Éxito

`cargo test --workspace` sin fallas nuevas contra la base, con menos tests en total (se van los del código borrado) y el conteo justificado en `verify.md`. Tests nuevos que fallan hoy:

- una conclusión guardada con `derived_from` queda enlazada a sus fuentes, también cuando el save se parte en pedazos o refuerza una fila existente;
- `derived_from` con un id inexistente o borrado se rechaza con 400 y no guarda nada;
- una conclusión no se disputa contra sus fuentes, y sí contra otra fila cercana;
- `synthesize` devuelve las memorias sin conclusión del alcance, no las que ya respalda una conclusión viva, y las vuelve a devolver si esa conclusión se borra;
- el recordatorio aparece al pasar el umbral y se apaga después de una síntesis;
- filas de `episodic_trajectory` y `procedural_skill` sembradas como las escribía la app siguen en la búsqueda, el workspace y el listado, y se re-embeben igual que antes (los tests que ya existen siguen pasando), y una base en V9 con esas filas abre con todo intacto (dato de la versión anterior).

`rag-eval --real --diff` contra el baseline: la búsqueda no cambia (el golden set no tiene filas episodic, procedural ni derivadas), así que se espera diff vacío.

| Métrica | Hoy | Umbral |
|---|---|---|
| R@1 / R@5 / R@10 | 0,4348 / 0,7826 / 0,8913 | sin bajar ninguna |
| MRR / nDCG@10 | 0,5979 / 0,6693 | sin bajar ninguna |

La calidad de las conclusiones no tiene métrica automática: depende del agente. Se revisa a mano en la verificación, con el perfil dev (ver `verify.md`).

## Datos guardados

- **Esquema:** una migración aditiva, `V10__synthesis.sql`, con dos tablas nuevas: `memory_derivation (memory_id, source_id, created_at)` y `synthesis_review (agent_id, scope_key, reviewed_at)`. No toca ninguna tabla existente. `MemoryStore::open` ya guarda un snapshot `pre-migration` en `<data>/backups` antes de aplicarla (`specs/storage`).
- **Estados:** no cambian. Una conclusión es una fila de `memory` como cualquier otra (`source = 'explicit'`), con su tier, su importancia y su lugar en el ranking.
- **Borrado:** borrar una conclusión no toca sus fuentes ni sus enlaces; sus fuentes vuelven a aparecer en `synthesize` porque ya no las respalda una conclusión viva. Borrar una fuente no toca la conclusión.
- **Datos de la versión anterior:** un test siembra filas de las tres tablas como las deja la versión actual (y las trayectorias y skills como las escribía la app) en una base en V9, la abre con la migración nueva y prueba que siguen alcanzables por búsqueda, workspace, listado y re-embed.
- **Volcado de la cola (`spill`):** `derived_from` viaja en `MemoryEvent` con `#[serde(default)]`; un volcado escrito por la versión anterior se lee igual.
- **Rollback:** una versión anterior rechaza la base sin tocarla (`SchemaNewer`). Hay dos vueltas atrás documentadas:
  - `clawg restore` del snapshot `pre-migration`: pierde lo guardado después de actualizar;
  - sin pérdida: `DROP TABLE memory_derivation; DROP TABLE synthesis_review; DELETE FROM refinery_schema_history WHERE version = 10;` con el daemon parado. Las conclusiones quedan como memorias comunes, sin su procedencia.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| El agente escribe conclusiones pobres o repite una memoria | Media | Las instrucciones piden dos o más fuentes y no repetir; las fuentes no se tocan, y una conclusión se borra o se disputa como cualquier fila |
| El recordatorio molesta | Media | Sólo aparece sobre un umbral de memorias nuevas desde la última revisión del alcance, y se apaga al llamar `synthesize` aunque no se guarde nada |
| Otro cambio agrega una migración `V10` antes que éste | Baja | visibility-policy se mergeó sin migración. Si otro llega primero, se renumera el archivo y la fila de `published_migration_checksums_are_pinned`; V10 no se publicó todavía |
| Un daemon viejo recibe `derived_from` o `synthesize` | Baja | `derived_from` se ignora (serde no lo rechaza) y la fila se guarda sin procedencia; `POST /v1/synthesis` da 404 y el puente lo informa. Se corrige al reiniciar el daemon |
| Alguien quiere volver a la opción b | Baja | El código borrado sigue en el historial de git (`git show <commit>^:crates/clawg-core/src/memory/miner.rs`) |
