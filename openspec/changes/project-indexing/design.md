# project-indexing — Diseño

Las decisiones de producto (opción b, tablas propias, herramienta propia, invalidación por hash) están en la [propuesta](proposal.md). Acá van las decisiones de implementación.

## Módulos

```
crates/clawg-core/src/memory/project_docs/
  mod.rs      DocIndex (refresco y búsqueda, con I/O), plan_refresh y fuse (puros)
  corpus.rs   classify (pura) y walk (I/O: recorre la raíz)
  chunk.rs    chunk_markdown y chunk_manifest (puras, con un TokenSplitter)
crates/clawg-core/migrations/V11__project_docs.sql
crates/clawg-cli/src/daemon/api.rs           POST /v1/projects/docs/search; el registro lanza un refresco
crates/clawg-cli/src/client.rs               DaemonClient::search_docs
crates/clawg-cli/src/mcp/server.rs, tools.rs search_project y su formato
crates/clawg-eval/src/bin/rag-eval/docs.rs   modo --docs
crates/clawg-eval/golden/project-docs.json   golden set de docs
```

Toda la lógica de decisión (qué archivo entra, dónde corta un encabezado, qué doc se reindexa, cómo se fusionan las listas, si toca refrescar) son funciones puras sin filesystem ni base. Lo demás es plomería alrededor de ellas.

## D1. Recorrer la raíz

| Opción | A favor | En contra |
|---|---|---|
| **a. Crate `ignore` (el walker de ripgrep)** | Respeta `.gitignore`, `.ignore` y `.git/info/exclude` con las reglas de git, también anidados; no necesita `git` instalado; mismo resultado en Windows y Linux | Una dependencia nueva (con `globset`, `walkdir`, `same-file`; `walkdir` ya está en el lockfile) |
| b. `git ls-files -co --exclude-standard` | Exacto para un repo git | Lanza un proceso por refresco; no anda sin `git` en el PATH ni en una raíz sin `.git`; hay que parsear su salida con rutas con comillas |
| c. `walkdir` más una lista de directorios conocidos (`docs/`, `doc/`, `adr/`, la raíz) | Sin dependencia nueva | Deja afuera docs reales (en este repo, todo `openspec/`) o, si se abre a todo el árbol, entra a `node_modules/` y `target/` |

**Elegida a.** Es la única que da "los docs que el usuario versiona" sin depender de un binario externo, y es una dependencia con uso real (la regla de `config.yaml`). Configuración: `hidden(false)` (para `.claude/` y `.github/`), `git_ignore(true)`, `git_exclude(true)`, `ignore(true)`, `require_git(false)`, `follow_links(false)`, `parents(false)` (un `.gitignore` por encima de la raíz no es del proyecto), y un filtro que corta `.git`. El orden se fija ordenando las rutas relativas al final, no con el orden del walker, para que el tope de 2.000 sea determinista.

`classify(ruta relativa) -> Option<DocKind>` es pura y se prueba con rutas de texto; `walk(raíz) -> Walk { docs, skipped }` hace el I/O y aplica los topes de tamaño y UTF-8.

## D2. Tablas

```sql
CREATE TABLE project_doc_file (
    project_id   TEXT    NOT NULL,
    path         TEXT    NOT NULL,
    content_hash TEXT    NOT NULL,   -- sha256 hex de los bytes del disco
    model_id     TEXT    NOT NULL,   -- el embedder que hizo sus vectores
    chunks       INTEGER NOT NULL,
    indexed_at   INTEGER NOT NULL,
    PRIMARY KEY (project_id, path)
) WITHOUT ROWID;

CREATE TABLE project_doc_chunk (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id  TEXT    NOT NULL,
    path        TEXT    NOT NULL,
    ordinal     INTEGER NOT NULL,
    breadcrumb  TEXT    NOT NULL,
    body        TEXT    NOT NULL
);
CREATE INDEX idx_project_doc_chunk_file ON project_doc_chunk (project_id, path);

CREATE VIRTUAL TABLE project_doc_vectors USING vec0(
    project_id TEXT PARTITION KEY,
    embedding  float[384]
);

CREATE VIRTUAL TABLE project_doc_fts USING fts5(
    text,
    project_id UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);
```

- **Filas propias, no `memory`** (decisión cerrada en la exploración). Ninguna sentencia del índice nombra una tabla de memorias, y R10 lo protege.
- **`project_id` como partition key de vec0.** sqlite-vec 0.1.9 (`Cargo.lock`) la soporta. El filtro por proyecto se aplica *dentro* del KNN, así que `k` vecinos son `k` vecinos del proyecto. Con el filtro después del corte, como en las memorias (`crates/clawg-core/src/memory/search.rs:98-104`), un repo grande podría llenar los `k` y dejar sin resultados a otro. Los límites que la exploración de visibility-policy encontró (sin `IN`, sin NULL) no aplican: el filtro es una igualdad y `project_id` nunca es NULL acá.
- **FTS5 con copia propia, no external content.** `memory_fts` usa external content porque `memory.content` no cambia nunca (`V8__memory_fts.sql`); un pasaje se reemplaza entero en cada reindexado, y una tabla con su propio texto se mantiene con `DELETE` + `INSERT` en la misma transacción, sin triggers ni la aritmética de restas que explica V8. El texto indexado es breadcrumb + cuerpo, así un término del encabezado encuentra la sección. Mismo tokenizer que `memory_fts`, por la misma razón (ES + EN, acentos).
- **`model_id` por doc.** El contrato de embedding de las memorias (`V5__embedding_contract.sql`) esconde las filas de otro modelo hasta re-embeberlas; acá el índice es derivado, así que un doc de otro modelo simplemente se reindexa (R5).
- **Sin claves foráneas ni triggers.** El refresco escribe las cuatro tablas en la misma transacción por doc (D4).
- **384 fijo**, como `memory_vectors` (`V2__vectors.sql`). Un embedder de otra dimensión ya es un cambio de contrato para todo el store.

## D3. Trocear

| Opción | A favor | En contra |
|---|---|---|
| **a. Una sección por encabezado, breadcrumb adelante, partir sólo las largas** | Simple y puro; cada pasaje sabe dónde está; el agente recibe secciones enteras | Secciones muy cortas dan pasajes cortos, con poco contexto |
| b. Además, fusionar secciones hermanas chicas hasta llenar el tope (como cAST) | Pasajes más parejos | El breadcrumb deja de nombrar una sección; un hit trae texto de varias y el agente no sabe cuál responde |
| c. Ventana fija de tokens sobre el archivo entero | Lo que hace hoy el chat chunker | Corta secciones por la mitad y pierde el encabezado, que es lo que la exploración propone arreglar |

**Elegida a.** El breadcrumb compensa el pasaje corto: "`docs/guia.md > Instalación > macOS`" ya dice de qué habla. Si el golden set muestra que las secciones cortas pierden, b se puede sumar sin cambiar el esquema.

Detalles:

- **Sólo encabezados ATX.** Los setext (`===` / `---` debajo de una línea) son raros en repos de código y `---` también es una regla horizontal o el cierre de un front matter; tratarlos como texto es el error más barato.
- **Bloques cercados.** Un `#` dentro de un bloque ```` ``` ```` es un comentario de shell o de Python, no un encabezado. Se sigue el cierre por carácter y largo, como CommonMark.
- **Tope.** El texto embebido es `breadcrumb + "\n\n" + cuerpo` y tiene que entrar en `CHUNK_MAX_TOKENS` (480, `crates/clawg-core/src/memory/chunker/mod.rs:15`), que ya respeta la ventana real de e5-small. El cuerpo se parte con el `TokenSplitter` existente a `480 - tokens(breadcrumb) - 2`, con el solapamiento de 64 de la ingesta. El breadcrumb se recorta a 256 bytes para que siempre quede presupuesto para el cuerpo.
- **Tokenizer.** El indexador usa el mismo tokenizer que la ingesta, el que `resolve_tokenizer_and_chunker` deja en `MemoryRuntime::tokenizer` (`crates/clawg-core/src/memory/worker.rs:572-581`). Si todavía no está, cae al splitter por bytes (`TokenSplitter::fallback`), como el chat chunker.
- **Secretos.** `redact` corre sobre el texto completo antes de trocear (R6), igual que en la ingesta (`crates/clawg-core/src/memory/worker.rs:428-433`). La huella se toma de los bytes sin enmascarar, así un cambio en el secreto también reindexa.

## D4. Refresco

```
walk(raíz)                         -> docs del disco: (ruta, bytes)            I/O
sha256(bytes)                      -> huella del disco                         puro
leer project_doc_file              -> huellas guardadas                        base
plan_refresh(disco, guardadas, m)  -> Plan { index, keep, remove }             puro
por cada doc de index:
    redact + chunk                 -> pasajes                                  puro
    embed_batch(pasajes, Passage)  -> vectores                                 embedder
    una transacción: borrar los pasajes del doc (las tres tablas), insertar los nuevos, upsert de la huella
una transacción: borrar los docs de remove
```

- **Una transacción por doc.** Una búsqueda que corre en el medio ve el doc viejo o el nuevo, nunca uno a medias. La alternativa, todo el refresco en una transacción, bloquea la única conexión (y con ella todos los saves) durante el embedding de todo el repo.
- **Embedding fuera de la transacción.** El embedder tarda decenas de ms por pasaje; se embebe antes de pedir la conexión.
- **Error de embedding.** Se corta el refresco y se devuelve el error: el doc fallado conserva sus pasajes viejos y su huella vieja, así el próximo refresco lo ve cambiado y lo reintenta.
- **Raíz faltante.** `walk` falla antes de leer huellas, así que no hay un plan que borre todo porque "el disco está vacío".

### Coordinación

`DocIndex` vive en `MemoryRuntime` (campo nuevo `project_docs: Arc<DocIndex>`), como el resto de los handles que un host necesita. Guarda, por proyecto, si hay un refresco en curso y cuándo empezó el último, detrás de un `std::sync::Mutex` que nunca se toma a través de un `await`.

- `try_refresh(project_id)` marca "en curso" o devuelve `AlreadyRunning` sin esperar (R5). La marca se suelta con un guard, también si el refresco falla o la tarea se cancela.
- `needs_refresh(último, ahora, intervalo)` es pura: nunca refrescado, o empezado hace más de 60 s.
- La raíz se lee de `project` en cada refresco: si un re-registro la cambia, el próximo refresco usa la nueva.

| Disparador | Elegido | Por qué |
|---|---|---|
| **Al registrar la raíz, en segundo plano** | sí | El puente registra cada raíz antes de la primera herramienta (`crates/clawg-cli/src/mcp/server.rs:441-468`), así que el primer indexado empieza con la sesión y no con la primera búsqueda |
| **Antes de buscar, si pasaron 60 s** | sí | Un doc editado durante la sesión aparece en la próxima búsqueda; con huellas, un repo sin cambios cuesta recorrer y hashear sus docs, no embeber |
| Watcher del filesystem | no | Una dependencia y un hilo por raíz, eventos que se pierden en Windows y en volúmenes de red; el refresco por demanda cubre lo mismo |
| Herramienta `index_project` | no | El agente no sabe cuándo el índice está viejo; el daemon sí |

El refresco de fondo espera al embedder con `embedder_status.subscribe().wait_for(...)`: el puente registra la raíz en el arranque, normalmente antes de que el modelo termine de cargar.

## D5. Buscar

| Opción | A favor | En contra |
|---|---|---|
| **a. Rango recíproco (RRF, k = 60) sobre vecinos y BM25** | Sólo usa rangos: no necesita calibrar una escala de coseno ni de BM25 para docs; estándar y puro | Pierde la magnitud: un hit dense muy bueno y uno mediocre valen distinto sólo por su posición |
| b. Piso de coseno más bono léxico, como lexical-fusion | Coherente con la búsqueda de memorias | El piso (0,8793) y el bono se calibraron sobre memorias; usarlos en docs es aplicar un número medido en otro corpus |
| c. Sólo vectores | Lo más simple | Un identificador (`CHUNK_MAX_TOKENS`, `clawg purge`) es justo donde e5-small es débil y donde un doc técnico más se busca |

**Elegida a**, y se valida con el golden set de docs (R12). `fuse(dense, lexical, limit)` es pura: recibe las dos listas de ids en orden y devuelve ids con puntaje. Cada lista trae hasta 50 pasajes; con docs, 50 cubre de sobra un top 20.

La expresión FTS5 es la misma que arma la búsqueda léxica de memorias (`fts_match_expression`, `crates/clawg-core/src/memory/search.rs:507`), que ya neutraliza los operadores; pasa a `pub(crate)` para que la use el índice. El tope de términos es el de memorias.

No hay access tracker, tiers ni decaimiento: un doc no "se usa", está en el disco.

## D6. Servir

- **HTTP:** `POST /v1/projects/docs/search` (R7). Se anida bajo `/projects` porque el índice es del proyecto, no de un agente; no recibe `agent_id`. Un `project_id` sin raíz da 404 porque sin raíz no hay qué indexar.
- **MCP:** `search_project` resuelve el proyecto con `project_for`, igual que `search_memory`, así un puente que todavía no registró la raíz la registra (y lanza el primer indexado) antes de buscar.
- **Instrucciones:** una oración más en `INSTRUCTIONS` (`crates/clawg-cli/src/mcp/server.rs:107-115`): para preguntas sobre cómo funciona el proyecto, su arquitectura o sus convenciones, `search_project` antes de abrir archivos.
- **`recall_context` no cambia** (decisión de la propuesta).

## D7. Limpieza del chunker de código

Se borran `chunker/code.rs`, la variante `ChunkerKind::Code` y su brazo en el worker, el parámetro `code_chunker` de `process_event`, `memory_worker` y `ingest_worker_future`, `Chunk.header` (sólo lo escribía `code.rs`; el worker embebe `chunk.text`), y `tree-sitter`, `tree-sitter-rust`, `tree-sitter-typescript` y `tree-sitter-python` de los dos `Cargo.toml`. El comentario de `regex` en el `Cargo.toml` del workspace deja de nombrar a tree-sitter.

Se quedan `MemorySource::CodeFile` y `MemoryType::CodeBlock`: son valores que una fila o un volcado de la cola pueden tener (`crates/clawg-core/src/memory/events.rs:58`, `crates/clawg-core/src/memory/store_insert.rs:34`).

La alternativa, dejar el chunker para una futura opción a, conserva un `#[allow(dead_code)]` que viola la convención, cuatro dependencias que se compilan en cada build, y un algoritmo que la exploración ya dijo que se reescribe. Si la opción a se elige, los parsers vuelven en un commit.

## D8. Medición

`rag-eval --docs` usa la puerta `eval_api` (feature `eval`) con dos funciones nuevas: abrir un `DocIndex` sobre un store temporal y refrescar una raíz con un `EvalEmbedder`, y buscar. No arranca daemon ni HTTP: mide el índice, no la plomería.

El golden set de docs se versiona en `crates/clawg-eval/golden/project-docs.json` porque pregunta sobre los docs públicos de este repo, a diferencia de `docs/eval/golden.json`, que tiene memorias reales y está en `.gitignore`. La validación rechaza un `expected` que el corpus no tiene, así un doc renombrado rompe la medición en vez de bajarla en silencio.

Las preguntas se escriben sin las palabras del pasaje esperado (si la respuesta dice "volcado", la pregunta no dice "volcado"), para medir sentido y no coincidencia léxica. `verify.md` lista cada pregunta con el pasaje que la responde.
