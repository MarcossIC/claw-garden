# memory-portability — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 2 | M | — (conviene después de los cambios de esquema pendientes: 24, 27, 36, 18; ver «Interacciones») | exploración cerrada; decisiones en la [propuesta](proposal.md#decisiones) |

**Tesis:** hoy la única copia de la memoria es el archivo SQLite. `backup` y `restore` mueven la base entera, atada a su versión de esquema: un binario rechaza una base más nueva. Con eso no se puede:

- mover una parte (un proyecto, un agente) de un perfil a otro o de una máquina a otra;
- leer la memoria sin `clawg`;
- traer memorias desde otra fuente.

Un export por fila, con una versión de formato propia e independiente del esquema, resuelve las tres cosas. Es también la puerta de entrada de history-import (44), y la base que el propio mapa nombra para cualquier sincronización futura.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| No hay export ni import | **Cierta.** `Command` tiene `Serve`, `Mcp`, `Status`, `Stop`, `Backup`, `Restore`, `Secrets`, `Purge`, `Sharing`, `Doctor`, `Memories` y `Path` (`crates/clawg-cli/src/cli.rs:32-122`). `memories` es un visor de sólo lectura. |
| `backup` y `restore` son de archivo entero | **Cierta.** `snapshot_into` usa `VACUUM INTO` (`crates/clawg-core/src/memory/backup.rs:291`). `restore` pasa por un archivo temporal, hace el snapshot `pre-restore`, valida con `check_restorable` y reemplaza el archivo (`backup.rs:336-399`). Exige el daemon parado. |
| Las relaciones usan ids locales | **Cierta.** `superseded_by` y `disputed_against` (un JSON de ids) viven en `memory` (`crates/clawg-core/migrations/V1__init.sql:24-28`; lectura en `crates/clawg-core/src/memory/dispute.rs:99-110`). `memory_derivation(memory_id, source_id)` está en `V10__synthesis.sql:8-13` y `memory_piece(save_id, piece, memory_id, context)` en `V11__memory_piece.sql:11`. Importar exige reasignar todos los ids. |
| `project_id` depende de la ruta | **Cierta.** `project(id, root_path)` (`V9__project.sql:6`). El id sale de un hash de la ruta raíz (`V9__project.sql:3-4`). En otra máquina u otra ruta, el mismo proyecto da otro id: el export tiene que llevar `root_path`. |
| Hay tablas derivadas que no se exportan | **Cierta.** Los vectores (`V2__vectors.sql:6-12`) están atados a `embedding_contract` (`V5__embedding_contract.sql:5`). El FTS se llena solo por triggers (`V8__memory_fts.sql`). El índice de docs se deriva del disco (`V12__project_docs.sql`). |
| El camino de ingest no conserva el estado de una fila | **Cierta.** `created_at` puede venir del evento (`timestamp_ms` → `created_at_ms`, `crates/clawg-core/src/memory/store_insert.rs:187`). Pero el insert fija `access_count = 1` y `last_accessed_at = created_at` (`PreparedRow::insert`, `store_insert.rs:536-586`), calcula el tier (`:522`) y sólo deja `active` o `disputed` (`:523`). No hay forma de reponer una fila `superseded`, borrada, ni con sus accesos. |
| Toda fila necesita un vector | **Cierta.** El insert recibe el embedding (`store_insert.rs:353-393`). Existe el embebido por lotes (`Embedder::embed_batch`, `crates/clawg-core/src/memory/embedder.rs:63`). |
| `serde_json` ya es dependencia | **Cierta.** `crates/clawg-core/Cargo.toml:28` y `crates/clawg-cli/Cargo.toml:28`. |
| El mapa ya la evaluó como útil | **Cierta.** «Export e import por fila (JSON versionado y atómico), y export a Markdown», calificado como útil y sin cambio todavía. Además, «Sincronizar entre máquinas» se descartó diciendo que su base sería este export (`openspec/README.md`, «Mejoras propuestas»). |

## Problema (verificado contra el código)

- **Atado al esquema.** Un snapshot sólo lo abre una versión que entienda su esquema. Para volver a una versión anterior hace falta el snapshot `pre-migration` de ese momento. Un formato propio, versionado aparte, se lee en cualquier versión que lo conozca.
- **Todo o nada.** No se puede llevar un proyecto del perfil `dev` a `prod`, ni separar lo de un agente. Hoy, para probar algo con datos reales en `dev` hay que copiar la base entera.
- **No se lee sin `clawg`.** La memoria del usuario es suya: tiene que poder mirarla, buscar en ella o archivarla con herramientas comunes.
- **Sin entrada desde otra fuente.** history-import (44) necesita un formato y un camino de importación que conserven fechas, tipos y relaciones. El save común no sirve para eso (ver B).

## Lo que ya existe y se reutiliza

- `clawg_core::memory::backup` y la regla de no tocar sin snapshot: un import es una escritura en masa y lleva un snapshot previo, igual que `restore`.
- `embed_batch` para re-embeber lo importado en lotes.
- `ProjectIdentity::resolve` (`crates/clawg-core/src/memory/identity.rs:29`) para volver a calcular el `project_id` desde `root_path` en el destino.
- `content_hash` y el índice parcial de dedup (`V6__content_hash.sql:14`) para que un segundo import no duplique.
- La validación de `derived_from` y de los tipos (`MemoryType::from_db_str`, `store_insert.rs:55-66`) para rechazar filas inválidas sin abortar el lote.
- El permiso sólo para el dueño de los archivos sensibles (`restrict_to_owner`, `crates/clawg-cli/src/daemon/lifecycle.rs`) para el archivo exportado.

## Enfoques posibles

### A. JSON Lines con encabezado versionado y un camino de importación propio (recomendado)

- **Formato:**
  - Una primera línea con `format`, `schema_version` (el del formato, no el de la base), `exported_at` y el modelo de embedding de origen.
  - Una línea por fila, con un id de export estable.
  - Las relaciones (`superseded_by`, pares de disputa, derivaciones, piezas) se expresan con ids de export.
  - Los proyectos van como `{export_id, root_path}`.
- **Import:**
  - Un camino propio en el núcleo que inserta filas completas: fechas, accesos, tier, estado, cadenas, borradas si se pidieron, piezas y derivaciones.
  - Reasigna ids, vuelve a calcular el `project_id` desde `root_path` y re-embebe por lotes.
  - Todo en transacciones acotadas y con un snapshot previo.
- **A favor:** es fiel y legible, y una línea por fila permite procesar archivos grandes sin cargarlos enteros.
- **En contra:** es un segundo camino de escritura, que tiene que respetar todo lo que el worker garantiza: enmascarado, hash, FTS, piezas y la regla de una fila viva por hash.

### B. Importar con el save común (`POST /v1/memories`)

- **A favor:** no agrega código de escritura.
- **En contra:** pierde estado, accesos, tier, cadenas y piezas originales. Pasa cada fila por el detector, que abriría pares entre notas del mismo tema, y por el dedup, que refuerza en lugar de saltar. Sirve sólo como modo degradado.

### C. Exportar también los vectores

Sólo valen si el destino tiene el mismo contrato de embedding. Ahorran el re-embebido, pero suman 384 floats por fila.

- Recomendado como opción, nunca por defecto. Al importar, se usan sólo si el contrato coincide exactamente.

### D. Export a Markdown

Una vista para leer, no para importar. Queda como opción de salida del mismo comando o fuera de alcance.

## Descartado

| Opción | Razón técnica |
|---|---|
| Estrategia `replace` al importar | Pisa filas del destino. La regla del proyecto es que nada destruye memorias sin un camino de vuelta, y `merge` más un snapshot previo cubren el caso. |
| Versionar el formato con la versión del paquete | Obliga a mantener una lista de versiones de `clawg` aceptadas. Se versiona el formato, que cambia mucho menos. |
| Sincronización bidireccional | Ya descartada en el mapa por complejidad. Este cambio deja la base, no la sincronización. |
| Exportar el índice de docs | Es derivado del disco y se reconstruye solo. |

## Interacciones

- **conflict-candidates (24), topic-revisions (27), ingest-outbox (36) y recoverable-delete (18)** agregan tablas o columnas: candidatas, `topic_key`, outbox y estado de borrado. Hacer este cambio después evita versionar el formato varias veces. Las filas del outbox no se exportan: se drenan antes, o el export avisa que hay pendientes.
- **memory-provenance (41).** Lo importado queda con origen `import`, y el origen exportado viaja y se conserva.
- **session-handoff (28).** `source_session_id` viaja.
- **project-clone-identity (30).** Exportar por `root_path` y no por `project_id` hace que un re-key no rompa un export viejo.
- **Sharing.** `sharing.json` es configuración, no memoria: no se exporta. Una fila conserva su `agent_id`, y el import puede reasignarlo (decisión).
- **Secretos.** Lo exportado ya está enmascarado si se guardó después del enmascarado. Las filas anteriores pueden tener secretos en claro: `clawg secrets` las lista, y el export puede pasar `redact` de nuevo (decisión). El archivo exportado contiene toda la memoria en texto plano.
- **history-import (44).** Es el consumidor: convierte otra fuente a este formato o usa el mismo camino de importación.

## Alcance candidato

1. La especificación del formato con su `schema_version` y un test que fija un ejemplo.
2. `clawg export [--project P] [--agent A] [--include-deleted] [--with-vectors] --out FILE`, a través del daemon si está corriendo, igual que `backup`.
3. `clawg import FILE [--dry-run] [--agent A]` con el camino propio del núcleo, el snapshot previo, la reasignación de ids y el re-embebido por lotes.
4. Un reporte de `--dry-run`: cuántas filas por tipo y proyecto, cuántas se saltan por hash y cuántas son inválidas.
5. Docs: `.claude/CLAUDE.md` (comandos y API) y una spec nueva.

### No-alcance

- Sincronización.
- Importar desde formatos de otras herramientas (44).
- Exportar el índice de docs o la configuración de sharing.

## Decisiones abiertas

Son del usuario.

- [x] **Por dónde escribe el import:** a través del daemon (un solo escritor; recomendado) o fuera de línea con el daemon parado, como `restore`.
- [x] **Borradas:** ¿se exportan con una bandera, y se importan como borradas?
- [x] **Agente en el destino:** ¿se conserva el `agent_id` de origen o se reasigna con `--agent`?
- [x] **Accesos y tier:** ¿se conservan, o lo importado arranca de cero?
- [x] **Detector al importar:** ¿se saltea (las filas ya convivían en el origen) o corre?
- [x] **`redact` al exportar** filas anteriores al enmascarado.
- [x] **Vectores:** ¿se ofrecen como opción?
- [x] **Markdown:** ¿entra o no?

## Cómo se mide

Tests RED primero:

1. Ida y vuelta: export de una base sembrada (filas de los siete tipos, piezas, derivaciones, una cadena `superseded`, un par `disputed`, un proyecto con `root_path`) e import en una base vacía. Cada fila se alcanza por búsqueda, y se conservan los conteos por estado, tipo y proyecto, las cadenas y los vínculos.
2. Un segundo import del mismo archivo no agrega filas ni suma accesos.
3. Un archivo con una fila inválida importa el resto y reporta la inválida.
4. Un archivo con una `schema_version` desconocida se rechaza sin escribir.
5. Un `root_path` distinto en el destino da el `project_id` del destino.
6. El import deja un snapshot previo.
7. **Base sembrada en la versión anterior** (regla de `openspec/config.yaml`): una base V12 se exporta, y lo importado a una base nueva conserva lo mismo.

`rag-eval`: el corpus del golden set exportado e importado tiene que dar el mismo ancla (`--diff` vacío). Es una prueba de fidelidad, no un cambio de ranking. Hay que medir el costo del re-embebido por fila con el modelo real.

## Riesgos

- **Un segundo camino de escritura que se desvía del worker.** Cada garantía del worker (enmascarado, hash, FTS, piezas, una viva por hash) necesita su test en el import.
- **Un archivo con toda la memoria en texto plano.** Se escribe sólo para el dueño, y el comando lo advierte.
- **Volumen.** El re-embebido de miles de filas tarda. Hay que medirlo y mostrar el progreso.
- **Un formato que cambia con cada cambio de esquema** si se hace antes de que se asienten 24, 27, 36 y 18.
