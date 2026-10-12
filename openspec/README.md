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
| 1 | [data-preservation](changes/data-preservation/exploration.md) | transversal | L (se parte en A–F) | — | exploración |
| 2 | [ingest-durability](changes/ingest-durability/exploration.md) | 1 | M | — | exploración |
| 3 | [retrieval-hygiene](changes/retrieval-hygiene/exploration.md) | 1 | S | — | exploración |
| 4 | [project-identity](changes/project-identity/exploration.md) | 2 | M | 1 (backup antes del re-key) | exploración |
| 5 | [eval-golden-expansion](changes/eval-golden-expansion/exploration.md) | medición | M | — | exploración |
| 6 | [visibility-policy](changes/visibility-policy/exploration.md) | 2, 4 | L | 4, 5 | exploración |
| 7 | [lexical-fusion](changes/lexical-fusion/exploration.md) | 1, 3 | M | 5 | exploración |
| 8 | [memory-update-tool](changes/memory-update-tool/exploration.md) | 1 | S | 6 (scope) | exploración |
| 9 | [dispute-resolution](changes/dispute-resolution/exploration.md) | 1, 5 | M | 5 | exploración |
| 10 | [mcp-protocol-upgrade](changes/mcp-protocol-upgrade/exploration.md) | 4 | M | 6 (identidad) | exploración |
| 11 | [knowledge-synthesis](changes/knowledge-synthesis/exploration.md) | 5 | decisión | — | exploración |
| 12 | [project-indexing](changes/project-indexing/exploration.md) | 3 | decisión | 5 | exploración |

Objetivos del producto (doc "Claw Garden — Cómo funciona hoy la memoria"):

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

## Revisión de `docs/analisis-proyecto.md` (verificada el 2026-10-11)

| Afirmación del análisis | Veredicto | Dónde queda |
|---|---|---|
| Las tablas episodic/procedural nunca se escriben, pero se consultan por KNN en cada búsqueda | **Cierto.** El costo está exagerado: son dos sentencias vacías por tabla | knowledge-synthesis |
| `save_memory` responde `queued` y los fallos se pierden en el log | **Cierto, y es peor:** tampoco hay drenaje al parar ni reintento del bootstrap del embedder | ingest-durability |
| `project_id` es un SipHash de 16 bits sobre el cwd, sin canonicalizar ni detectar la raíz git | **Cierto.** El problema real es la partición falsa, no la colisión | project-identity |
| Aislamiento estricto por `agent_id` (clientInfo.name) | **Cierto para search/recall.** list/delete/PATCH no tienen scope | visibility-policy |
| No hay WAL ni `busy_timeout`, y hay riesgo de `SQLITE_BUSY` | **Falso.** WAL está activo (`db.rs:191`), rusqlite pone 5 s de busy timeout y hay una sola conexión serializada | descartado |
| Las lápidas vectoriales degradan el KNN | **Mayormente falso.** La fila vec se borra y hay barrido diario de huérfanas. Quedan huecos en los chunks de sqlite-vec, sin medir | descartado (vigilar) |
| El canal léxico se re-puntúa por coseno y queda en 0 bajo 0,8793 | **Cierto.** BM25 sólo propone candidatos, no hay fusión | lexical-fusion |
| Migrar a RRF | **No se puede validar todavía:** el harness no ve el canal léxico | eval-golden-expansion, lexical-fusion |
| El rango de importancia 0,85–1,0 es demasiado estrecho | **Falso como razonamiento.** El sweep midió R@10 0,826 con 0,55–1,0 contra 0,891 con 0,85–1,0 | descartado |
| La importancia sólo se cambia por HTTP PATCH | **Cierto.** PATCH sólo acepta `importance` y nunca degrada el tier | memory-update-tool |
| Ventana fija de 480 tokens; no existe indexado de código | **Cierto.** Además, `code.rs` (tree-sitter) existe pero está muerto. Las cifras de cAST no salen del paper | project-indexing |
| MCP sin sesión (SEP-2567/2575) | **Cierto.** Spec 2026-07-28: también se va `initialize` en stdio y `clientInfo` pasa a opcional. `_meta.project_root_canonical` no es estándar | mcp-protocol-upgrade, visibility-policy |
| Visibilidad compartida, con `agent_id` sólo como autoría | **Incompleto.** El objetivo 4 pide control del usuario por agente | visibility-policy |
| extractor/miner/resolver compilados pero inactivos | **Cierto.** Necesitan proveedor LLM, clave, transcripts y un disparador; hoy no hay ninguno | knowledge-synthesis |
| El resolver pasa el registro viejo a `superseded` | **Falso.** Ningún código escribe `superseded`, ni siquiera el resolver muerto. La detección sí corre | dispute-resolution |
| Regenerar `workspace.md` ante eventos | **Innecesario.** Ya se regenera en cada `recall_context`, justo antes de leerse | descartado |

Hallazgos que el análisis no menciona:

| Hallazgo | Cambio |
|---|---|
| Inflación de accesos: se cuentan hits que el agente nunca ve | retrieval-hygiene |
| Los empates en score 0 se ordenan por id viejo | retrieval-hygiene |
| Los saves aceptados se pierden al hacer `clawg stop`, también en cada actualización | data-preservation |
| No hay backups, restore ni undelete. Un cambio de modelo esconde toda la memoria sin remedio | data-preservation |
| Cola infinita si falla el bootstrap del embedder | ingest-durability |
| Cualquier agente puede borrar cualquier memoria | visibility-policy |
| El filtro de scope se aplica después del corte k de vec0 (crowding) | visibility-policy |
| Disputas huérfanas al borrar la fila vieja | dispute-resolution |
| Detector de contradicciones basado sólo en proximidad vectorial | dispute-resolution |

## Backlog sin spec (arreglos sueltos)

- **Comentario desactualizado** en `crates/clawg-core/src/memory/worker.rs:344-355`: dice que ningún chunk carga afirmaciones, y los saves explícitos sí la cargan.
- **`docs/eval/README.md` desactualizado**: dice 13 casos y menciona el filtro `tier>1`.
- **`#[allow(dead_code)]` en el chunker** (`ChunkerKind::Code`): viola la convención del proyecto. Se resuelve con knowledge-synthesis o project-indexing.
