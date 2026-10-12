# openspec — cambios de Claw Garden

Cada cambio vive en su carpeta, `changes/<nombre>/`. No hay un documento único con todo.

Este archivo es solamente el **mapa**: qué cambios hay, en qué orden y de dónde salieron.

## Cómo se trabaja un cambio

```
exploration -> proposal -> spec (+ design si hace falta) -> tasks -> apply -> verify -> archive
```

- **`exploration.md`**: el punto de partida. Tiene el problema verificado contra el código (con `ruta:línea`), los enfoques posibles y las decisiones abiertas.
- **`proposal.md` / `spec.md` / `design.md` / `tasks.md`**: se escriben cuando le toca al cambio. El detalle es proporcional al cambio: un fix chico es una propuesta de una página más las tareas.
- **`state.yaml`**: en qué fase está el cambio.
- **Archivo**: cuando un cambio se cierra, pasa a `changes/archive/AAAA-MM-DD-<nombre>/`. Sus requisitos se consolidan en `specs/<dominio>/spec.md`, la fuente de verdad del comportamiento actual.
- **Reglas del proyecto para cada fase**: están en `config.yaml`.

## Mapa de cambios (orden recomendado)

| # | Cambio | Objetivo | Tamaño | Depende de | Estado |
|---|--------|----------|--------|------------|--------|
| 1 | [data-preservation](changes/archive/2026-10-12-data-preservation/proposal.md) | transversal | M (partes A + F; B–E siguen en la [exploración](changes/archive/2026-10-12-data-preservation/exploration.md#alcance-candidato-partes)) | — | A + F archivadas el 2026-10-12 ([`specs/storage`](specs/storage/spec.md)); B + C, D y E sin cambio propio todavía |
| 2 | [ingest-durability](changes/archive/2026-10-12-ingest-durability/proposal.md) | 1 | M | — | archivado el 2026-10-12 ([`specs/ingest`](specs/ingest/spec.md)) |
| 3 | [retrieval-hygiene](changes/archive/2026-10-12-retrieval-hygiene/proposal.md) | 1 | S | — | archivado el 2026-10-12 ([`specs/retrieval`](specs/retrieval/spec.md)) |
| 4 | [project-identity](changes/project-identity/exploration.md) | 2 | M | 1 (backup antes del re-key) | exploración |
| 5 | [eval-golden-expansion](changes/archive/2026-10-12-eval-golden-expansion/proposal.md) | medición | M | — | archivado el 2026-10-12 ([`specs/eval`](specs/eval/spec.md)) |
| 6 | [visibility-policy](changes/visibility-policy/exploration.md) | 2, 4 | L | 4, 5 | exploración |
| 7 | [lexical-fusion](changes/lexical-fusion/exploration.md) | 1, 3 | M | 5 | exploración |
| 8 | [memory-update-tool](changes/memory-update-tool/exploration.md) | 1 | S | 6 (scope) | exploración |
| 9 | [dispute-resolution](changes/dispute-resolution/exploration.md) | 1, 5 | M | 5 | exploración |
| 10 | [mcp-protocol-upgrade](changes/mcp-protocol-upgrade/exploration.md) | 4 | M | 6 (identidad) | exploración |
| 11 | [knowledge-synthesis](changes/knowledge-synthesis/exploration.md) | 5 | decisión | — | exploración |
| 12 | [project-indexing](changes/project-indexing/exploration.md) | 3 | decisión | 5 | exploración |
| 13 | [secret-hygiene](changes/secret-hygiene/exploration.md) | 1, 4 | S | — (tiene que cerrar antes de 6) | exploración |
| 14 | [context-density](changes/context-density/exploration.md) | 1 | S–M | — (se suma a 9) | exploración |
| 15 | [long-save-context](changes/long-save-context/exploration.md) | 1 | S–M | 5, después de 9 (1 si toca el esquema) | exploración |

Objetivos del producto:

1. Guardar y clasificar memorias de forma eficiente.
2. Separarlas por proyecto y también usarlas de forma global.
3. Indexar proyectos para que la IA entienda cómo funciona cada uno.
4. Ser compartido entre agentes, con control del usuario sobre qué ve cada uno.
5. Extraer conclusiones con el tiempo.

Por qué este orden:

- **1 va primero, por regla de producto: la base del usuario no se pierde.** Hoy cada actualización puede perder saves aceptados, y no hay backups. Además, todo cambio que toque datos (el re-key de **4**, los vec0 de **6**) necesita esa red antes.
- **2 y 3** son defectos con pérdida de datos o de calidad. Son independientes y chicos.
- **4** va antes que **6**: la política de visibilidad necesita identidades estables.
- **5** es requisito de **7** y **12**. Con el golden set actual, el canal léxico no se puede medir: todas las corridas `lex*` dan métricas idénticas.
- **11** y **12** no tienen spec todavía. Primero hay una decisión de producto.
- **13** se numera al final, pero tiene que cerrar antes de **6**: compartir entre agentes hace viajar un secreto guardado por error a otros clientes y proveedores. Su purga se decide junto con la parte E de **1**.
- **15 es de los últimos en probarse, por su naturaleza:** todo lo que necesita viene de antes.
  - Sin memorias largas en el golden set de **5**, y sin casos cuya respuesta quede después del corte, no se puede medir.
  - Conviene que **9** confirme o descarte antes la hipótesis de los pedazos hermanos que se disputan, que tiene la misma causa.
  - Si se elige el puntero entre pedazos, toca el esquema y necesita la red de **1**.
  - Se vuelve urgente con la migración desde engram: una importación trae textos largos que no siguen el "un hecho por save".

## Ideas evaluadas

Ideas y afirmaciones contrastadas con el código. Las descartadas quedan con su razón, para no retomarlas por error.

### Sobre el estado actual

| Afirmación | Veredicto | Dónde queda |
|---|---|---|
| Las tablas episodic/procedural nunca se escriben, pero se consultan por KNN en cada búsqueda | **Cierto.** El costo es chico: dos sentencias vacías por tabla | knowledge-synthesis |
| `save_memory` responde `queued` y los fallos se pierden en el log | **Cierto, y es peor:** tampoco hay drenaje al parar ni reintento del bootstrap del embedder | ingest-durability |
| `project_id` es un SipHash de 16 bits sobre el cwd, sin canonicalizar ni detectar la raíz git | **Cierto.** El problema real es la partición falsa, no la colisión | project-identity |
| Aislamiento estricto por `agent_id` (clientInfo.name) | **Cierto para search/recall.** list/delete/PATCH no tienen scope | visibility-policy |
| No hay WAL ni `busy_timeout`, y hay riesgo de `SQLITE_BUSY` | **Falso.** WAL está activo (`db.rs:191`), rusqlite pone 5 s de busy timeout y hay una sola conexión serializada | descartado |
| Las lápidas vectoriales degradan el KNN | **Mayormente falso.** La fila vec se borra y hay barrido diario de huérfanas. Quedan huecos en los chunks de sqlite-vec, sin medir | descartado (vigilar) |
| El canal léxico se re-puntúa por coseno y queda en 0 bajo 0,8793 | **Cierto.** BM25 sólo propone candidatos, no hay fusión | lexical-fusion |
| Migrar a RRF | **No se puede validar todavía:** el harness no ve el canal léxico | eval-golden-expansion, lexical-fusion |
| El rango de importancia 0,85–1,0 es demasiado estrecho | **Falso.** El sweep midió R@10 0,826 con 0,55–1,0 contra 0,891 con 0,85–1,0 | descartado |
| La importancia sólo se cambia por HTTP PATCH | **Cierto.** PATCH sólo acepta `importance` y nunca degrada el tier | memory-update-tool |
| Ventana fija de 480 tokens; no existe indexado de código | **Cierto.** Además, `code.rs` (tree-sitter) existe pero está muerto, y no implementa cAST | project-indexing |
| MCP sin sesión (SEP-2567/2575) | **Cierto.** Spec 2026-07-28: también se va `initialize` en stdio y `clientInfo` pasa a opcional. `_meta.project_root_canonical` no es estándar | mcp-protocol-upgrade, visibility-policy |
| Visibilidad compartida, con `agent_id` sólo como autoría | **Incompleto.** El objetivo 4 pide control del usuario por agente | visibility-policy |
| extractor/miner/resolver compilados pero inactivos | **Cierto.** Necesitan proveedor LLM, clave, transcripts y un disparador; hoy no hay ninguno | knowledge-synthesis |
| El resolver pasa el registro viejo a `superseded` | **Falso.** Ningún código escribe `superseded`, ni siquiera el resolver muerto. La detección sí corre | dispute-resolution |
| Regenerar `workspace.md` ante eventos | **Innecesario.** Ya se regenera en cada `recall_context`, justo antes de leerse | descartado |

### Hallazgos del código

| Hallazgo | Cambio |
|---|---|
| Inflación de accesos: se cuentan hits que el agente nunca ve | retrieval-hygiene |
| Los empates en score 0 se ordenan por id viejo (es lo único que hoy deja ver un match léxico bajo el piso) | lexical-fusion |
| Los saves aceptados se pierden al hacer `clawg stop`, también en cada actualización | data-preservation |
| No hay backups, restore ni undelete. Un cambio de modelo esconde toda la memoria sin remedio | data-preservation |
| Cola infinita si falla el bootstrap del embedder | ingest-durability |
| Cualquier agente puede borrar cualquier memoria | visibility-policy |
| El filtro de scope se aplica después del corte k de vec0 (crowding) | visibility-policy |
| Disputas huérfanas al borrar la fila vieja | dispute-resolution |
| Detector de contradicciones basado sólo en proximidad vectorial | dispute-resolution |
| ACTIVE CONFLICTS es el 65 % de los tokens de `recall_context` y repite filas ya mostradas | context-density |
| Un save largo se parte en pedazos que pierden el tema y que nada vuelve a armar | long-save-context |

### Mejoras propuestas

| Idea | Veredicto | Dónde queda |
|---|---|---|
| Indexación híbrida FTS5 + vectores | **Ya existe:** sqlite-vec y FTS5 (`crates/clawg-core/migrations/V8__memory_fts.sql`). Lo que falta es la fusión de scores | lexical-fusion |
| Búsqueda híbrida 70:30 vector + BM25 (89 % de recall en OpenClaw) | **Coherente con el plan, pero no transferible:** es otro corpus. Se valida con el golden set propio | lexical-fusion |
| Índice HNSW (vecstore, tpt-raglite, Qdrant) | **Descartado.** vec0 es fuerza bruta y alcanza para un store personal. Cambiar de motor rompe la transacción única memoria + vector + FTS (`store_insert.rs:329-372`). `rag-eval --agent-view` ya mide la latencia p50/p99, y el corpus distractor de eval-golden-expansion la va a mostrar con más filas | descartado (vigilar con el corpus distractor) |
| Embeddings locales con fastembed-rs | **Ya existe:** fastembed con multilingual-e5-small. Sólo se descarga el modelo (`model_download.rs:21`), no se envía contenido | descartado |
| GPU opcional para ONNX | **Descartado.** Modelo chico (384 dims); el costo de build y distribución no se justifica | descartado |
| Resumen automático para no sobrecargar el contexto | **Premisa falsa.** El contexto no crece con la memoria: `recall_context` corta en `K_FINAL = 10` (`prompt_builder.rs:32`). Un resumen que reemplace originales violaría la regla de `config.yaml` | knowledge-synthesis (la decisión del objetivo 5 ya está abierta) |
| Pasar el contexto de JSON a Markdown | **No aplica:** el agente ya recibe texto plano (`server.rs:205-207`). Sólo `memory_status` usa JSON, y no está en el camino caliente | descartado |
| Referencias por id en lugar de copias (PROV-AGENT), y Markdown denso | **Aplica:** es lo que resuelve la repetición de ACTIVE CONFLICTS | context-density |
| Workspace con envejecimiento hot/warm/cold | **Ya existe:** tiers 1/2/3 con decaimiento. El workspace sólo carga tier 1, con un tope | descartado |
| Grafos episódico-semánticos (Synapse, E-mem, MLMF) | **Descartado por ahora:** el paso de abstracción necesita un LLM, y lo determinista (decaimiento, activación) ya está cubierto por el puntaje. Además, las comparaciones publicadas miden F1 sobre configuraciones distintas | knowledge-synthesis (evidencia para la decisión) |
| Síntesis dirigida por el agente; "memory flush" antes de compactar (OpenClaw) | **Evidencia para la opción c:** los sistemas basados en archivos coinciden en que el propio agente escribe sus conclusiones | knowledge-synthesis |
| Duplicados y contradicciones con verificación por LLM | **Ya cubierto.** El dedup por hash existe (`dedup.rs`) y la detección de contradicciones corre en cada save. El resolver LLM ya es la opción b de dispute-resolution | dispute-resolution |
| Versionado tipo git (Memoria) | **Excesivo.** Lo necesario son backups, restore y undelete | data-preservation (B, E) |
| Tokio, sqlx/diesel, patrón repositorio | **Ya hay** tokio y `tokio_rusqlite` con una sola conexión serializada. Migrar a sqlx es churn sin un problema que resolver | descartado |
| Cifrado en reposo | **Descartado por ahora.** El robo del disco lo cubre el cifrado del sistema operativo. SQLCipher complica cargar sqlite-vec y los backups, y una clave perdida es una base perdida, que es justo el riesgo que más cuidamos. Hubo un diseño previo, nunca implementado (`Cargo.toml:32`) | descartado |
| Inyección en consultas FTS | **Ya resuelto por construcción:** los operadores no pueden sobrevivir a la tokenización (`search.rs:489-519`) y hay tests (`search.rs:2809-2897`) | descartado |
| Enmascarar secretos y evitar fugas de contexto entre agentes | **Necesario, y no estaba cubierto.** Un secreto guardado por error queda en claro para siempre (no hay purga) y, con el cerebro compartido, viaja a otros agentes | secret-hygiene; procedencia en visibility-policy |
| Tests, docs y benchmarks | **Ya existen:** 1304 tests, golden set y `rag-eval` con latencia. Nada nuevo que agregar | descartado |

## Backlog sin spec (arreglos sueltos)

- **Comentario desactualizado** en `crates/clawg-core/src/memory/worker.rs:344-355`: dice que ningún chunk carga afirmaciones, y los saves explícitos sí la cargan.
- **`#[allow(dead_code)]` en el chunker** (`ChunkerKind::Code`): viola la convención del proyecto. Se resuelve con knowledge-synthesis o project-indexing.
- **`clawg stop` mientras carga el modelo:** el apagado ordenado termina en 1–3 s, pero el proceso sigue vivo hasta que vuelve la carga del embedder. La carga corre en `spawn_blocking` (`crates/clawg-core/src/memory/bootstrap.rs:318`), y el runtime de `#[tokio::main]` (`crates/clawg-cli/src/main.rs:30`) espera a esas tareas. Con el binario debug, una vez se pasó de `STOP_TIMEOUT` y `stop` salió con 1. Los datos no corren riesgo: el volcado y el checkpoint terminan antes. Después de "stopped", el log dice "model load task panicked: task N was cancelled" y "retrying in 30s", y eso confunde. Un arreglo posible es `shutdown_timeout` del runtime cuando `serve::run` vuelve. Detalle en el `verify.md` archivado de data-preservation.
- **`enum Scope` sin uso** (`crates/clawg-core/src/memory/record.rs:19`): se exporta, pero el insert escribe el literal `"agent"` (`store_insert.rs:177`). Se resuelve con visibility-policy: se usa o se borra.
