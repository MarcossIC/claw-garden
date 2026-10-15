# session-handoff — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 5 | M–L (M si sólo la columna y el bloque; L si se suma captura de prompts o tabla) | agent-lifecycle-hooks (25); para el bloque en el contexto, además context-budget (23) | exploración (borrador) |

**Tesis:** lo que una sesión nueva más necesita es saber dónde quedó la anterior, y hoy nada en el sistema lo sabe. Un `session_summary` es un `memory_type` más: no tiene estructura, no se distingue por sesión y no se muestra primero. Las columnas que identificarían la sesión existen desde V1 pero ningún código las escribe. Este cambio busca lo mínimo que haga cierto «retomá donde quedó la sesión anterior» sin construir maquinaria de sesiones.

## Problema (verificado contra el código)

### `session_summary` es sólo una etiqueta

- Es un valor más del tipo de memoria (`crates/clawg-core/src/memory/store_insert.rs:35`, `:60`). Sólo `PATCH` valida el tipo (`MemoryPatch::new` → `PatchError::UnknownType`, `crates/clawg-core/src/memory/admin.rs:141`, `:171`); un save con un tipo desconocido cae en `semantic_fact` sin avisar (`store_insert.rs:140-144`). La herramienta MCP lo lista como uno de siete (`crates/clawg-cli/src/mcp/server.rs:129-130`).
- Lo único que lo trata distinto: el decaimiento (`crates/clawg-core/src/memory/score.rs:54`, `:127`, `:231`) y el nivel inicial. Entra en el tier 2, y sube al tier 1 con `access_count >= 3` (`crates/clawg-core/src/memory/tier.rs:37-55`).
- **Ningún código de recuperación lo trata distinto.** `rg session_summary` sobre `crates` no da coincidencias en `search.rs` ni en `prompt_builder.rs`: sólo en tipos, decaimiento, tier, validación y herramientas de evaluación. El último resumen sólo aparece si la consulta se le parece. No hay «el último primero».
- No tiene estructura: es texto libre. Lo que debería contener (qué se hizo, qué quedó pendiente, qué decisión sigue) depende de lo que el modelo escriba. El texto del protocolo de 22 es el que debería fijarlo; aquí se decide dónde se lee.

### Las columnas de sesión no tienen escritor (premisa confirmada)

- `memory.source_session_id TEXT` y `memory.last_modified_by TEXT` existen desde V1 (`crates/clawg-core/migrations/V1__init.sql:16`, `:18`). `last_modified_by` está además en las otras dos tablas (`V1__init.sql:77`, `:128`).
- El `INSERT` de producción **no** las nombra (`crates/clawg-core/src/memory/store_insert.rs:355-359`: `scope … content_hash`). Ningún otro `INSERT`/`UPDATE` de código no-test escribe `source_session_id` ni `last_modified_by` (`rg` sobre `crates`). El único `INSERT` que escribe `source_session_id` es un test (`admin.rs:2094`, aserción en `:2111`).
- Hoy sólo se **lee** `source_session_id`: la lista y `clawg memories show` lo muestran como `session` (`admin.rs:571`, `:670`; `crates/clawg-cli/src/memories.rs:194`) y, en la práctica, siempre es `-`. `last_modified_by` no tiene lector fuera de los structs de `record.rs:80`, `:128`, `:176`.
- **No hay nada que migrar para escribir `source_session_id`:** la columna ya está. Las filas existentes tienen `NULL`.
- `episodic_trajectory.session_id` es otra cosa: `NOT NULL UNIQUE` (`V1__init.sql:67`), generado por un contador en tests (`maintenance.rs:779`) y sin uso en producción. No sirve para el id de sesión del host.

### El bridge no conoce la sesión del host

- El puente aprende el agente por `--agent` o `clientInfo.name` (`mcp/server.rs:84-92`, `:369-374`) y el proyecto desde el directorio de trabajo al arrancar (`mcp/server.rs:315-318`). Nada más del host: no hay id de sesión.
- `SaveRequest` y `SaveArgs` no tienen campo de sesión (`crates/clawg-cli/src/daemon/api.rs:311-323`, `crates/clawg-cli/src/client.rs:288-298`). El `INSERT` tampoco.
- El modelo **no** debe pasar el id de sesión: no lo conoce, puede inventarlo y cada herramienta con un argumento más suma contexto fijo por sesión.

### Qué ofrece el host para obtener el id (de agent-lifecycle-hooks)

Verificado en https://code.claude.com/docs/en/hooks: todo hook recibe `session_id` por stdin. Ese es el id real. `SessionStart` tiene `source` (`startup`, `resume`, `clear`, `compact`, `fork`). **No verificado en la doc oficial:** que los subprocesos de servidores MCP stdio reciban `CLAUDE_CODE_SESSION_ID` en su entorno (sólo lo menciona un reporte de terceros: https://claudeissues.com/issue/63305-docs-mcp-stdio-docs-omit-session-environment-variables) y que la solicitud de herramienta MCP lleve el id en `_meta` (sin evidencia).

Un detalle que decide el diseño: un proceso MCP stdio se lanza una vez; `/clear` y `resume` cambian el `session_id` del host. Si el id llega por entorno al lanzar, puede quedar viejo después de `/clear`. Sin verificar.

### El modelo de concurrencia ya está resuelto

Un solo proceso es dueño de la base: bloqueo exclusivo sobre `memory.lock` y registro en `memory.owner` (`crates/clawg-core/src/paths.rs:196-208`, `crates/clawg-core/src/memory/store_lock.rs:23-24`). Los clientes pasan por el daemon. No hay dos escritores compitiendo por la base.

## Lo que ya existe y se reutiliza

- La columna `memory.source_session_id` y sus lectores (`admin.rs:571`, `:670`; `memories.rs:194`).
- El canal hook → daemon de agent-lifecycle-hooks (25): `DaemonClient` y su autenticación (`crates/clawg-cli/src/client.rs:429-444`).
- `ProjectIdentity::resolve(cwd)` para unir hook y puente por proyecto (`crates/clawg-core/src/memory/identity.rs:29`).
- El enmascarado de secretos `redact` (`crates/clawg-core/src/memory/redact.rs:131`), que ya corre en el ingest antes de trocear, hashear y embeber.
- `GET /v1/memories` con `memory_type`, `agent`, `project`, `limit`, ordenado por `created_at DESC` (`api.rs:352-372`, `admin.rs:636`): ya devuelve «el último `session_summary` de este proyecto» sin código nuevo en el daemon.
- El presupuesto por prioridades de context-budget (23), donde el bloque ocupa la prioridad 3.
- El snapshot previo a migrar (`crates/clawg-core/src/memory/db.rs:213`, `backup.rs:121-136`) si se llegara a necesitar un esquema nuevo.

## Enfoques posibles

### Cómo se entera el puente del id de sesión

| Opción | Cómo | A favor | En contra |
|---|---|---|---|
| **A. Registro en el daemon** | El hook de `SessionStart` manda `{agent, proyecto, session_id}` al daemon; el daemon lo recuerda en memoria. El guardado lo busca por `(agent, proyecto)` | Sin dependencias del host más allá del hook; el modelo no interviene | Dos sesiones simultáneas del mismo agente en el mismo proyecto son ambiguas: gana la última. Hay que decidir qué pasa si no hay registro (columna `NULL`). Estado en memoria que se pierde si el daemon se reinicia |
| **B. Entorno del puente** | El puente lee `CLAUDE_CODE_SESSION_ID` al arrancar y lo manda con cada guardado | Sin registro, sin ambigüedad entre sesiones simultáneas | **Sin verificar** en la doc oficial. Puede quedar viejo tras `/clear` o `resume`. No depende de 25 |
| **C. `_meta` de la solicitud MCP** | El cliente lo manda por herramienta | Siempre fresco | Sin evidencia de que el host lo mande |
| **D. El hook escribe el id en un archivo que el puente lee** | | Simple | Es estado por archivo con las mismas fallas que se descartaron en 25; carreras entre sesiones |
| **E. El modelo lo pasa como argumento** | | Trivial | Descartado: no se confía en el modelo y suma contexto fijo |

Recomendación: **A**, con **B** como complemento cuando se verifique, y la columna en `NULL` cuando no hay dato. A es la única que usa un contrato documentado (el `session_id` del hook). Tratar la ambigüedad de sesiones simultáneas como una limitación documentada, no como un error.

### Qué se guarda y dónde

| Opción | Costo | Notas |
|---|---|---|
| **1. Sólo la columna** (recomendado) | Cero migraciones: se escribe `source_session_id` en el `INSERT` | Alcanza para «las memorias de esta sesión» y para atribuir el resumen |
| **2. Columna + índice** `(project_id, memory_type, created_at)` | Migración V13 con snapshot previo | Sólo si medir muestra que la consulta del último resumen es lenta. Hoy hay un índice parcial por `(scope, project_id)` (`V1__init.sql:41-43`) |
| **3. Tabla `session`** (id, agente, proyecto, inicio, fin, resumen) | Migración, escritor, limpieza y nueva superficie | No se justifica: lo que se necesita (último resumen del proyecto, memorias de la sesión) sale de `memory` con la columna. Ver «Descartado» |

Recomendación: **1**. Sin tabla ni índice hasta tener una medición que lo pida.

### Bloque LAST SESSION en `recall_context`

- Una sección nueva en el prompt (junto a los separadores de `prompt_builder.rs:21-25`) con el `session_summary` más reciente del proyecto del agente, por `created_at`, fuera de la competencia por relevancia. Prioridad 3 en el presupuesto de 23: cede ante WORKSPACE y los conflictos de alto riesgo y se recorta antes que el resto.
- **Qué pasa en proyecto sin resumen:** el bloque no aparece. Qué pasa si el resumen más reciente es muy viejo (meses): decisión abierta (mostrarlo con su fecha o no mostrarlo).
- **Por agente o por proyecto:** el sharing actual decide quién lee qué (`sharing.json`, ver `CLAUDE.md`). El bloque debe respetarlo: el resumen de otro agente sólo si ese agente lo comparte con el lector, y marcado `by <author>`.
- Toca recuperación y contexto: **requiere `rag-eval --diff`**.

### Línea de tiempo de la sesión (opcional)

Con `source_session_id` escrito, «las memorias guardadas en la misma sesión alrededor de una dada» es una consulta por columna. Se puede exponer en `read_memory` o en el visor `clawg memories`. Es de bajo costo una vez que la columna se escribe; no hace falta para el valor central.

### Captura de prompts del usuario (opcional, opt-in)

`UserPromptSubmit` entrega el texto del prompt (verificado en la doc del host). Guardarlo permitiría reconstruir qué se pidió en la sesión.

- **A favor:** historia fiel de lo pedido, sin depender de que el modelo la resuma.
- **En contra:**
  - Privacidad: el usuario no pidió guardar cada mensaje. Los prompts traen rutas, secretos pegados y datos de terceros. El enmascarado de `redact.rs` cubre secretos de formato conocido, no todo lo sensible.
  - Ruido: notificaciones de tareas y mensajes generados por el agente llegan por la misma vía y habría que filtrarlos; el criterio es frágil.
  - Crecimiento de la base: un registro por prompt, sin límite natural, en una base cuyo dato más importante es lo guardado a propósito. Además cada fila se embebe.
  - Calidad de recuperación: cientos de mensajes cortos compiten con los hechos guardados deliberadamente.
- Recomendación: **descartar** en este cambio. Si se retoma, sólo opt-in explícito, enmascarado con `redact`, con filtro y con retención acotada, en un cambio propio.

## Descartado

- **Maquinaria de sesiones en tiempo de ejecución** (continuaciones de sesión, modos de propiedad, arrendamientos o leases). Esos problemas existen cuando varios procesos disputan el mismo estado. Aquí un solo proceso (el daemon) es dueño de la base bajo bloqueo exclusivo (`paths.rs:196-208`, `store_lock.rs:23-24`): no hay nada que arrendar ni heredar.
- **Tabla `session` propia.** Duplica lo que dan la columna y el último `session_summary`, agrega una migración con su test de preservación y un ciclo de vida (cuándo termina una sesión) que el host no garantiza: `SessionEnd` tiene un presupuesto de 1,5 s y no siempre corre.
- **Que el modelo pase el id de sesión.** No lo conoce, puede inventarlo y suma un argumento a cada herramienta.
- **Reutilizar `episodic_trajectory.session_id`.** Es `UNIQUE`, `NOT NULL` y de otra tabla sin uso en producción (`V1__init.sql:67`).
- **Captura de prompts por defecto.** Ver arriba.

## Interacciones

- **history-import (44).** La fuente que mide 44 trae sesiones con su id y su directorio, y `session_summary` es uno de sus tipos más frecuentes (552 de 3.554 notas). Si la columna ya se escribe, lo importado puede llevar su sesión, y el bloque LAST SESSION tiene contenido desde el primer día en proyectos con historia.
- **memory-provenance (41).** `source_session_id` es una columna de origen, igual que `source` y `created_by_type`. La regla es la misma en los dos cambios: el daemon sella con lo que sabe y el modelo no inventa ids.
- **bridge-reconnect (39).** El registro de sesión de la opción A vive en memoria del daemon y se pierde si se reinicia. Con 39, el puente sigue funcionando después del reinicio, pero los saves quedan con `NULL` hasta el siguiente `SessionStart`. Es coherente con «mejor `NULL` que un id falso».

## Alcance candidato

1. Escribir `memory.source_session_id` en el `INSERT` de producción (`store_insert.rs:355`) cuando la solicitud lo trae; `NULL` si no. Sólo la tabla `memory`; las otras dos no se crean en producción.
2. Campo opcional en `SaveRequest` y `SaveArgs`, fuera del cable cuando está vacío (como `derived_from`, `client.rs:294-297`) para que un daemon más viejo lea igual.
3. Canal hook → daemon para el id de sesión (opción A) y su uso al guardar desde el puente.
4. Bloque LAST SESSION en el prompt de `recall_context`, con prioridad 3 del presupuesto de 23 y respetando el sharing.
5. Mostrar la sesión en el visor (`clawg memories show` ya imprime `session`, `memories.rs:194`).
6. (Opcional) consulta de memorias de la misma sesión.
7. `CLAUDE.md` del repo: el comportamiento nuevo del daemon y el bloque.

### No-alcance

- Tabla `session`, continuaciones, propiedad, arrendamientos.
- Captura de prompts (decisión abierta: descartar).
- Cambiar el esquema salvo que una medición lo pida (índice).
- Escribir `last_modified_by` (sin valor identificado para este objetivo; ver decisión abierta).
- Estructura obligatoria del resumen (texto de 22).
- Hooks de `SessionEnd`.

## Decisiones abiertas

Son del usuario.

- [ ] **Origen del id:** registro en el daemon (A), entorno del puente (B), o ambos. B exige verificar `CLAUDE_CODE_SESSION_ID` contra la doc oficial y su comportamiento tras `/clear`.
- [ ] **Sesiones simultáneas:** ¿se acepta «gana la última» para el mismo agente y proyecto, o se deja `NULL` cuando hay más de un registro?
- [ ] **Resumen demasiado viejo:** ¿el bloque LAST SESSION se muestra con su fecha o se omite pasado un umbral?
- [ ] **Sharing del bloque:** ¿sólo el propio resumen del agente, o también los de otros agentes que le comparten?
- [ ] **Índice:** ¿se mide antes de tocar el esquema? (recomendado)
- [ ] **`last_modified_by`:** ¿se escribe, se deja, o se elimina del esquema en otro cambio? Hoy no tiene escritor ni lector.
- [ ] **Línea de tiempo de sesión:** ¿entra, o se deja para después?
- [ ] **Captura de prompts:** ¿se descarta (recomendado), o va en un cambio opt-in aparte?
- [ ] **Dependencia:** si se elige sólo la opción B, ¿sigue dependiendo de 25 o basta 23?

## Cómo se mide

Tests RED primero:

1. Un guardado con id de sesión escribe `source_session_id` en la fila de `memory`; sin id queda `NULL`.
2. Un daemon recibe un guardado con el campo ausente (cliente viejo) y se comporta como hoy.
3. El registro hook → daemon: tras `SessionStart`, un guardado del mismo `(agente, proyecto)` lleva ese id; tras un segundo `SessionStart` con otro id, lleva el nuevo; otro agente u otro proyecto no se mezclan.
4. `recall_context` con un `session_summary` en el proyecto incluye el bloque LAST SESSION con el **más reciente**, aunque no se parezca a la consulta. Sin resumen, no hay bloque.
5. El bloque cede ante la prioridad de WORKSPACE y respeta el tope de 23.
6. Sharing: el resumen de un agente oculto para el lector no aparece; el compartido aparece marcado `by <author>`.
7. Un pedido que pasa un id de sesión por argumento de herramienta no existe en el esquema de `save_memory` (test que fija los argumentos).
8. Base sembrada en la versión anterior: filas con `source_session_id` `NULL` siguen siendo recuperables, buscables y mostradas con `-`; es la regla de `openspec/config.yaml` para datos guardados y vale aunque el esquema no cambie.
9. Si se agrega índice o tabla: migración V13 con un test que siembre una base en V12, abra con el binario nuevo, compruebe el snapshot `pre-migration` y que todo siga alcanzable.

`rag-eval --diff`: **sí**, porque el bloque LAST SESSION cambia el contenido de `recall_context`. Comparar contra la línea base de `openspec/config.yaml` (agent-view de `recall_context`: 39/46 a 1086,4 tokens medios). Umbral a fijar en la propuesta: sin caída de aciertos y el aumento de tokens dentro de lo que 23 asigne a la prioridad 3. El golden set casi no tiene `session_summary` como objetivo (`golden-mine` los genera, `crates/clawg-eval/src/bin/golden-mine/main.rs:42`), así que puede hacer falta casos nuevos.

## Riesgos

- **Id equivocado:** con sesiones simultáneas o tras `/clear` el registro puede atribuir un guardado a la sesión errónea. Mejor `NULL` que un id falso; la consecuencia es sólo de atribución.
- **Dependencia de variable de entorno sin verificar:** apoyarse en `CLAUDE_CODE_SESSION_ID` sin confirmarla puede dejar la opción B muerta.
- **Costo de contexto:** el bloque se paga en cada `recall_context`; depende del presupuesto de 23.
- **Resumen de baja calidad:** si el modelo escribe un resumen pobre, el bloque lo repite con autoridad. Depende del texto de 22.
- **Retroceso de recuperación:** tocar `recall_context` exige `rag-eval --diff`.
- **Fuga entre agentes:** un bloque que ignore el sharing mostraría resúmenes que el usuario ocultó.
- **Privacidad (si se retoma la captura de prompts):** ver arriba; no entra en este cambio.
- **Dependencia posiblemente sobrada:** 25 sólo es necesaria para la opción A. Con la opción B alcanzaría 23.
