# project-indexing — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 | M | eval-golden-expansion (archivado) | ver [`state.yaml`](state.yaml) |

## Problema

El objetivo 3, indexar proyectos para que la IA entienda cómo funciona cada uno, no tiene ningún camino vivo. Detalle verificado en la [exploración](exploration.md); lo que decide esta propuesta:

1. **Nada lee archivos de un proyecto.** No hay comando, endpoint ni watcher que recorra un repo. Lo único que el daemon sabe de un proyecto es su raíz, que guarda `POST /v1/projects` en la tabla `project` (`crates/clawg-core/migrations/V9__project.sql:6-10`, `crates/clawg-cli/src/daemon/api.rs:968-988`).
2. **El agente no tiene cómo preguntar "cómo funciona X" en lenguaje natural.** Tiene `grep` y `read`, que encuentran una palabra pero no una explicación escrita con otras palabras. Las herramientas MCP que existen sólo leen memorias (`crates/clawg-cli/src/mcp/server.rs:656-790`).
3. **El chunker AST está muerto y no es cAST.** `ChunkerKind::Code` lleva `#[allow(dead_code)]` y nunca se devuelve (`crates/clawg-core/src/memory/chunker/mod.rs:42-55`); el worker sólo lo alcanzaría con `language = None`, que cae al splitter plano (`crates/clawg-core/src/memory/worker.rs:447`). `CodeChunker` se construye en cada arranque sin uso (`crates/clawg-core/src/runtime.rs:358`) y arrastra cuatro dependencias de tree-sitter (`Cargo.toml:47-50`). Viola la convención de no dejar código muerto, y el README lo deja en el backlog para este cambio.

## Decisiones (cierran las abiertas de la exploración)

**Qué significa "entender cómo funciona".** Poder contestar una pregunta en lenguaje natural sobre el proyecto ("¿qué pasa con los saves si paro el daemon?", "¿cómo se elige el perfil?") con el pasaje de la documentación del propio proyecto que la responde, aunque la pregunta no use sus palabras. Las 10 preguntas del golden set de docs (ver **Éxito**) son la definición operativa.

**Opciones b y a: la documentación troceada por encabezado y el código troceado con cAST** (elegida por Marcos en la tarjeta de decisión, opción "Docs + código cAST"). Se indexan los Markdown del repo (README, `docs/`, `CLAUDE.md`, ADRs, specs), los manifiestos (`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`) y el código Rust, Python y TypeScript, en el mismo índice.

- **a** se construye sin medir antes a e5-small sobre código, como dice la opción elegida. El troceo se escribe de cero con el algoritmo cAST (dividir el árbol de arriba abajo y juntar los nodos chicos vecinos hasta el tope, medido en caracteres no blancos) y una cabecera de contexto con la ruta y las firmas de los contenedores. Del chunker muerto sólo se reusan los parsers de tree-sitter, que vuelven como dependencias.
- **c** (mapa generado por LLM) depende de un proveedor que no existe, por la misma razón que knowledge-synthesis descartó la opción b.
- **d** (mapa de símbolos) ya está descartada en la exploración.

**El índice va en tablas propias, derivadas y reconstruibles** (decisión ya cerrada en la exploración). No pasa por `save_memory`, no tiene tiers, decaimiento ni disputas, y borrarlo o reconstruirlo no toca una sola memoria.

**Se sirve con una herramienta propia, `search_project`, no dentro de `recall_context`.** `search_memory` y `recall_context` no cambian, así que el índice no puede desplazar memorias de sus resultados. Sumar pasajes de docs a `recall_context` necesita un piso de relevancia calibrado para docs, y el piso de coseno actual (0,8793) está calibrado sobre memorias. Queda como seguimiento, con el golden set de docs como base para calibrarlo.

**Pooling mean para todo el store.** Al medir la primera versión (Hit@3 0,20 en docs) apareció la causa: el embedder cargaba e5-small sin indicar pooling y fastembed tomaba el token CLS, mientras que e5 se entrena con mean. Todos los pasajes quedaban a 0,90–0,91 de coseno de cualquier pregunta. Antes de abrir el PR se midieron modelos más grandes y un reranker (opción "Investigar antes"); ninguno llegó al umbral y el pooling mean dio la mayor mejora por costo. Marcos eligió corregirlo para las memorias también ("Pooling en todo"), con el re-embebido de los vectores guardados al arrancar el daemon, después de un snapshot ("Solo al arrancar"). Los umbrales calibrados sobre CLS se mueven a valores provisionales y se recalibran con el golden set de memorias.

**Invalidación por hash del contenido de cada archivo** (decisión ya cerrada en la exploración), con refresco al registrar la raíz y antes de buscar si pasó un intervalo.

## Alcance

1. **Limpieza** (`clawg-core`): borrar `chunker/code.rs`, `ChunkerKind::Code`, el parámetro `code_chunker` del worker, `Chunk.header` (sólo lo escribía `code.rs`) y las cuatro dependencias de tree-sitter. `MemorySource::CodeFile` y el tipo `code_block` se quedan: son valores que una fila guardada puede tener.
2. **Corpus y troceo** (`clawg-core`, puro): qué archivos entran, cómo se trocea un Markdown por encabezado con su breadcrumb y cómo se trocea un manifiesto.
3. **Índice** (`clawg-core`): migración `V11__project_docs.sql` con tres tablas nuevas; refresco incremental por hash, con los secretos enmascarados antes de trocear, y búsqueda híbrida (vectores + FTS5) dentro del proyecto.
4. **Daemon** (`clawg-cli`): el registro de una raíz lanza un refresco en segundo plano; `POST /v1/projects/docs/search` refresca si hace falta y busca.
5. **MCP** (`clawg-cli`): herramienta `search_project`, y las instrucciones del servidor dicen cuándo usarla.
6. **Medición** (`clawg-eval`): `rag-eval --docs`, que indexa una raíz y puntúa un golden set de preguntas sobre sus docs y su código, con números aparte para cada uno.
7. **Código** (`clawg-core`, puro): `chunk_code` con cAST para Rust, Python, TypeScript y TSX; el corpus suma esas extensiones y el resto del índice no cambia.
8. **Pooling** (`clawg-core`): e5-small con pooling mean y su id propio en el contrato; el bootstrap del daemon re-embebe un store de vectores CLS después de un snapshot `pre-reembed`; los umbrales de similitud pasan a valores provisionales sobre la distribución nueva.

## Fuera de alcance

- Generar mapas con LLM (opción c) o un mapa de símbolos (opción d).
- Otros lenguajes de código (Go, Java, C#…). Se suman con su gramática de tree-sitter sin cambiar el esquema.
- Un embedder de código, o cambiar de modelo. Se midieron e5-base, e5-large y un reranker (bge-reranker-v2-m3): ninguno llega al umbral y cuestan de 2 a 30 veces más (ver `verify.md`).
- Pasajes de docs dentro de `recall_context` o de `search_memory`.
- Un watcher del filesystem. El refresco es por demanda y por intervalo.
- Búsqueda de docs entre proyectos (`all_projects`): un pasaje sólo tiene sentido dentro de su repo.
- Visibilidad por agente del índice. Los docs están en el disco y cualquier agente que trabaja en ese repo ya los puede leer; el índice se comparte entre los agentes que registran la misma raíz.
- Un comando `clawg` para reindexar o borrar el índice. Se puede sumar sin cambiar el contrato.

## Éxito

`cargo test --workspace` sin fallas nuevas contra la base. El conteo baja por los tests de `code.rs` y sube por los nuevos; el número queda justificado en `verify.md`. Tests nuevos que fallan hoy:

- el corpus incluye los Markdown y manifiestos del repo, respeta `.gitignore` y deja fuera `.git`, los binarios y los archivos más grandes que el tope;
- un Markdown se parte en cada encabezado, cada pedazo lleva su breadcrumb, un encabezado dentro de un bloque de código no corta, y una sección más larga que el tope se parte con solapamiento sin perder el breadcrumb;
- un refresco sin cambios no re-embebe nada; un archivo cambiado reemplaza sus pedazos; uno borrado los pierde; un cambio de modelo reindexa todo;
- un secreto de formato conocido en un doc no llega al índice;
- `search_project` encuentra un pasaje por sentido (vector) y por identificador exacto (FTS5), y nunca devuelve pasajes de otro proyecto;
- `search_memory` y `recall_context` devuelven lo mismo con y sin índice;
- una base en V10 con memorias abre con V11 con todo intacto (dato de la versión anterior).

**Golden set de docs:** `crates/clawg-eval/golden/project-docs.json`, 10 preguntas sobre la documentación de este mismo repo, escritas sin copiar las palabras del pasaje que las responde, cada una con el archivo y el encabezado esperados. Se mide con `rag-eval --real --docs --root <repo>`:

| Métrica | Umbral |
|---|---|
| Hit@3 (el pasaje esperado entre los 3 primeros) | ≥ 0,70 |
| MRR | ≥ 0,50 |
| Tiempo del primer indexado de este repo | se reporta, sin umbral |

Además, 5 preguntas sobre el código de este repo, con la ruta del archivo que las responde. Para ellas Hit@3 y MRR se reportan aparte y sin umbral: e5-small no está medido sobre código y este cambio es la primera medición.

`rag-eval --real --diff` y `--agent-view --diff` contra el baseline de memorias: con el pooling mean los vectores cambian, así que el diff ya no es vacío. Después de recalibrar los umbrales (R15) ninguna métrica puede bajar:

| Métrica | Hoy | Umbral |
|---|---|---|
| R@1 / R@5 / R@10 | 0,4348 / 0,7826 / 0,8913 | sin bajar ninguna |
| MRR / nDCG@10 | 0,5979 / 0,6693 | sin bajar ninguna |

## Datos guardados

- **Esquema:** una migración aditiva, `V11__project_docs.sql`, con `project_doc_file`, `project_doc_chunk`, `project_doc_vectors` (vec0 con `project_id` como partition key) y `project_doc_fts` (FTS5). No toca ninguna tabla existente. `MemoryStore::open` ya guarda un snapshot `pre-migration` antes de aplicarla (`specs/storage`).
- **Identificadores y estados:** no cambian. El índice usa el `project_id` que ya deriva `ProjectIdentity::resolve` y la raíz que ya guarda `project`.
- **Memorias:** ninguna fila de `memory`, `episodic_trajectory` ni `procedural_skill` se lee, escribe ni borra al indexar o buscar docs.
- **Vectores de las memorias:** el primer arranque con esta versión re-embebe los tres tipos de memoria con pooling mean y cambia el `model_id` del contrato a `multilingual-e5-small/mean`. El texto y el resto de cada fila no cambian. Antes escribe un snapshot `pre-reembed`.
- **Datos de la versión anterior:** un test siembra memorias en una base en V10, la abre con la migración nueva y prueba que siguen idénticas y alcanzables, y que las tablas del índice existen vacías.
- **Backups:** los snapshots copian la base entera, así que incluyen el índice. Crece con el tamaño de los docs (del orden de 2 KB por pedazo más su vector de 1,5 KB).
- **Rollback:** una versión anterior rechaza la base sin tocarla (`SchemaNewer`). Hay dos vueltas atrás documentadas:
  - `clawg restore` del snapshot `pre-migration` (o del `pre-reembed`, que ya tiene V11 pero los vectores CLS): pierde lo guardado después de actualizar;
  - sin pérdida: con el daemon parado, `DROP TABLE project_doc_fts; DROP TABLE project_doc_vectors; DROP TABLE project_doc_chunk; DROP TABLE project_doc_file; DELETE FROM refinery_schema_history WHERE version = 11;`. Ninguna memoria depende de esas tablas.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Un repo grande hace lento el primer indexado o llena la base | Media | Topes de archivos por proyecto y de bytes por archivo; el primer indexado corre en segundo plano y la búsqueda responde con lo que haya, avisando que sigue indexando |
| El agente no llama a `search_project` | Media | Las instrucciones del servidor y la descripción de la herramienta dicen cuándo usarla. Si no alcanza, el seguimiento es llevar pasajes a `recall_context` con un piso calibrado |
| El índice queda viejo mientras se edita un doc | Media | Refresco por hash antes de cada búsqueda si pasó el intervalo; un archivo sin cambios no se re-embebe |
| Un secreto escrito en un doc entra al índice | Baja | Se enmascara con el mismo `redact` de la ingesta antes de trocear, embeber e indexar |
| Indexar compite con los saves por CPU | Baja | Un solo refresco por proyecto a la vez, por archivos; la ingesta y la búsqueda de memorias no esperan al índice |
| e5-small no separa bien pasajes de código, ni de docs cuando la pregunta no usa sus palabras | Alta (medido en docs: Hit@3 0,20) | El canal léxico encuentra identificadores exactos; `rag-eval --docs` reporta docs y código por separado para decidir con números si hace falta otro modelo o un reranker |
| El código multiplica los pasajes y el primer indexado tarda | Media | Mismos topes de archivos y bytes; el refresco es incremental por hash, así que sólo el primero cuesta |
| Otro cambio agrega una migración `V11` antes que éste | Baja | memory-update-tool no necesita esquema. Si otro llega primero, se renumera el archivo y la fila de `published_migration_checksums_are_pinned` |
| Re-embeber un store grande deja la memoria en `loading` mucho rato en el primer arranque | Media | Pasa una sola vez; el estado lo dice y el README lo explica. En este entorno e5-small embebe unos 6,5 pasajes de hasta 480 tokens por segundo (4.312 en 657 s); las memorias son más cortas |
| Los umbrales provisionales ordenan peor que los de CLS | Media | Salen de la misma posición en la distribución nueva; quedan marcados como provisionales y se recalibran con el golden set antes de salir del draft |
| Un binario viejo abre un store ya re-embebido | Baja | Ve otro `model_id` y deshabilita la memoria (mismatch) sin tocar nada; volver atrás es restaurar el `pre-reembed` |
| Un daemon viejo recibe `search_project` | Baja | `POST /v1/projects/docs/search` da 404 y el puente pide reiniciar el daemon, como con `synthesize` |
