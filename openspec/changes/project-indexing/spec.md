# project-indexing — Especificación

**Dominio destino:** `specs/project-docs/spec.md` (nuevo; se crea al archivar). R10 se suma a `specs/retrieval/spec.md` y R12 a `specs/eval/spec.md`.

**Alcance:** qué archivos de un proyecto se indexan, cómo se trocean, cómo se mantiene el índice al día, cómo se busca en él y cómo se mide. La búsqueda de memorias, el ranking, los tiers y `recall_context` no cambian.

## Definiciones

- **Raíz:** la `root_path` que `POST /v1/projects` guardó en `project` para un `project_id` (`specs/project`, R4).
- **Ruta relativa:** la ruta de un archivo respecto de la raíz, con `/` como separador en todas las plataformas.
- **Doc:** archivo de la raíz que el corpus acepta (R2): un Markdown o un manifiesto.
- **Sección:** un encabezado ATX de Markdown y las líneas que le siguen hasta el próximo encabezado de cualquier nivel, o el texto anterior al primer encabezado (preámbulo).
- **Breadcrumb:** la ruta relativa seguida de los encabezados que contienen a la sección, del más externo al propio, unidos con ` > `. El del preámbulo y el de un manifiesto es sólo la ruta relativa.
- **Pasaje:** una fila de `project_doc_chunk`: el breadcrumb, el texto de un pedazo de una sección, su vector y su entrada léxica.
- **Huella de un doc:** el sha256 de sus bytes en el disco, y el id del modelo que embebió sus pasajes.
- **Refresco:** la pasada que deja el índice de un proyecto igual a sus docs en el disco (R5).

## R1. El chunker de código se va sin cambiar la ingesta

`ChunkerKind::Code`, `CodeChunker`, `Chunk.header` y las dependencias `tree-sitter*` MUST desaparecer. La ingesta MUST seguir troceando cada save con el chunker de chat, también los de tipo `code_block`, y una fila guardada con `source = 'code_file'` o `memory_type = 'code_block'` MUST seguir leyéndose.

#### Escenario: un save de código
- GIVEN un `save_memory` con `memory_type = "code_block"` y un bloque de Rust de 3 líneas
- WHEN lo procesa el worker
- THEN queda una fila con ese contenido y ese tipo, igual que hoy

**Test:** `a_code_block_save_is_chunked_like_any_other` en `crates/clawg-core/src/memory/worker.rs`, más los tests de `select_chunker` que ya existen. No fallan hoy: protegen el borrado. El borrado lo verifica la compilación (`cargo build` sin `#[allow(dead_code)]` en `chunker/mod.rs`).

## R2. Corpus

La decisión de qué archivo es un doc MUST ser una función pura de la ruta relativa:

- es **Markdown** si su extensión es `md`, `mdx` o `markdown`, sin distinguir mayúsculas;
- es **manifiesto** si su nombre es exactamente `Cargo.toml`, `package.json`, `pyproject.toml` o `go.mod`;
- cualquier otro archivo MUST NOT entrar.

El recorrido del disco, aparte de esa función:

- MUST respetar `.gitignore`, `.ignore` y `.git/info/exclude`, también en una raíz sin `.git`;
- MUST NOT entrar a `.git` ni seguir enlaces simbólicos;
- MUST incluir los directorios ocultos que no estén ignorados (`.claude/`, `.github/`);
- MUST saltear un archivo de más de 256 KiB o que no sea UTF-8 válido, y contarlo como salteado;
- MUST tomar a lo sumo 2.000 docs por proyecto, en orden de ruta relativa, y contar el resto como salteados.

#### Escenario: qué entra
- GIVEN una raíz con `README.md`, `docs/guia.MD`, `.claude/CLAUDE.md`, `crates/a/Cargo.toml`, `src/main.rs`, `target/x.md` con `target/` en `.gitignore`, y `.git/HEAD`
- WHEN se recorre
- THEN los docs son `.claude/CLAUDE.md`, `README.md`, `crates/a/Cargo.toml` y `docs/guia.MD`, en ese orden

#### Escenario: topes
- GIVEN un Markdown de 300 KiB y otro con bytes que no son UTF-8
- WHEN se recorre
- THEN ninguno es doc y los dos cuentan como salteados

**Tests:** `classify_*` (puros) en `crates/clawg-core/src/memory/project_docs/corpus.rs`; `the_walk_honours_gitignore_and_keeps_hidden_docs` y `oversized_and_binary_files_are_skipped` en el mismo módulo, con directorios temporales. Fallan hoy: el módulo no existe.

## R3. Troceo de Markdown

`chunk_markdown(ruta, texto)` MUST ser pura y:

- reconocer como encabezado una línea con hasta 3 espacios, de 1 a 6 `#` y un espacio o el fin de línea; el texto del encabezado MUST quedar sin los `#` de apertura ni de cierre;
- MUST NOT reconocer un encabezado dentro de un bloque de código cercado (``` o `~~~`, que cierra con el mismo carácter y al menos el mismo largo);
- dar un pasaje por sección con texto no vacío; una sección sin cuerpo MUST NOT dar pasaje, pero su encabezado MUST seguir en el breadcrumb de sus subsecciones;
- formar el breadcrumb con los encabezados de nivel estrictamente menor que la contienen, más el propio;
- recortar un breadcrumb de más de 256 bytes por el principio, en un límite de carácter, con `…` adelante, para conservar los encabezados más cercanos;
- el texto embebido e indexado de un pasaje MUST ser `breadcrumb + "\n\n" + cuerpo`;
- si ese texto pasa de `CHUNK_MAX_TOKENS` (480), partir el cuerpo con el `TokenSplitter` en pedazos que, con el breadcrumb, entren en el tope, con `CHUNK_OVERLAP_TOKENS` (64) de solapamiento, y repetir el breadcrumb en cada pedazo.

#### Escenario: secciones y breadcrumb
- GIVEN `docs/guia.md` con `intro`, `# Guía`, `texto A`, `## Instalación`, `### macOS`, `texto B`, `## Uso`, `texto C`
- WHEN se trocea
- THEN los pasajes son `docs/guia.md` / `intro`, `docs/guia.md > Guía` / `texto A`, `docs/guia.md > Guía > Instalación > macOS` / `texto B` y `docs/guia.md > Guía > Uso` / `texto C`

#### Escenario: código cercado
- GIVEN una sección con un bloque ```` ```sh ```` que contiene `# no es encabezado`
- WHEN se trocea
- THEN la sección da un solo pasaje que incluye esa línea

#### Escenario: sección larga
- GIVEN una sección de 2.000 tokens
- WHEN se trocea
- THEN da varios pasajes, cada uno con el breadcrumb de la sección, ninguno de más de 480 tokens, y el final de cada uno se repite al principio del siguiente

**Tests:** en `crates/clawg-core/src/memory/project_docs/chunk.rs`: `sections_carry_their_breadcrumb`, `a_heading_inside_a_fence_does_not_split`, `an_empty_section_gives_no_passage_but_names_its_children`, `closing_hashes_are_not_part_of_the_heading`, `a_long_section_is_split_with_overlap_under_the_cap`, `a_long_breadcrumb_keeps_its_nearest_headings`. Fallan hoy.

## R4. Troceo de manifiestos

Un manifiesto MUST dar un único pasaje con breadcrumb igual a su ruta relativa y su texto completo como cuerpo, partido como en R3 si no entra en el tope.

**Test:** `a_manifest_is_one_passage_named_by_its_path` en `chunk.rs`. Falla hoy.

## R5. Refresco incremental

`refresh(project_id)` MUST dejar en el índice del proyecto exactamente los docs del disco (R2), y:

- un doc cuya huella coincide con la guardada MUST NOT re-leerse en pasajes ni re-embeberse;
- un doc nuevo o cambiado (otro sha256, u otro modelo) MUST reemplazar todos sus pasajes, vectores y entradas léxicas en **una** transacción por doc, así una búsqueda nunca ve medio doc;
- un doc que ya no está en el corpus MUST perder todos sus pasajes y su huella;
- un doc vacío o sin secciones con texto MUST guardar su huella sin pasajes;
- si la raíz no existe o no es un directorio, MUST NOT borrar nada y MUST devolver el error;
- si el embedder falla en un doc, ese doc MUST quedar como estaba, el refresco MUST terminar con el error y el siguiente MUST reintentarlo;
- MUST NOT leer ni escribir `memory`, `episodic_trajectory`, `procedural_skill`, sus vectores, su FTS ni el access tracker;
- MUST devolver cuántos docs vio, reindexó, dejó igual, quitó y salteó, y cuántos pasajes escribió.

Sólo MUST correr un refresco por proyecto a la vez: un pedido de refresco mientras otro corre MUST volver enseguida diciendo que ya hay uno en curso, sin esperar.

La decisión de qué hacer con cada doc MUST ser una función pura de las huellas del disco y las guardadas.

#### Escenario: nada cambió
- GIVEN un proyecto ya indexado
- WHEN se refresca sin tocar el disco
- THEN el embedder no recibe ningún texto y el reporte dice 0 reindexados

#### Escenario: un doc cambió y otro se borró
- GIVEN `a.md` y `b.md` indexados
- WHEN se edita `a.md`, se borra `b.md` y se refresca
- THEN `a.md` tiene sólo sus pasajes nuevos, `b.md` no tiene ninguno, y el reporte dice 1 reindexado y 1 quitado

#### Escenario: otro modelo
- GIVEN un proyecto indexado con el modelo `m1`
- WHEN se refresca con un embedder `m2`
- THEN todos los docs se reindexan

#### Escenario: raíz que ya no existe
- GIVEN un proyecto indexado cuya raíz se borró
- WHEN se refresca
- THEN el refresco falla y los pasajes siguen ahí

**Tests:** `plan_*` (puros) en `crates/clawg-core/src/memory/project_docs/mod.rs`; `an_unchanged_project_embeds_nothing`, `a_changed_doc_replaces_its_passages_and_a_deleted_one_loses_them`, `another_model_reindexes_every_doc`, `a_missing_root_deletes_nothing`, `a_failed_embedding_keeps_the_doc_as_it_was`, `a_second_refresh_does_not_wait_for_the_first` y `refreshing_never_touches_memories` en el mismo módulo, contra un store en memoria y un directorio temporal. Fallan hoy.

## R6. Secretos

El texto de un doc MUST pasar por `redact` (`crates/clawg-core/src/memory/redact.rs`) antes de trocearse. Ningún pasaje, vector ni entrada léxica MUST contener un secreto de formato conocido. La huella usa los bytes del disco, sin enmascarar.

#### Escenario
- GIVEN `docs/setup.md` con una clave `sk-ant-…` de formato válido
- WHEN se refresca y se busca la clave literal
- THEN ningún pasaje la contiene y la búsqueda no la devuelve

**Test:** `a_secret_in_a_doc_never_reaches_the_index` en `project_docs/mod.rs`. Falla hoy.

## R7. Búsqueda

`POST /v1/projects/docs/search`, con token, recibe `{ "project_id": "...", "query": "...", "limit": n }` y responde `{ "hits": [{ "path", "breadcrumb", "text", "score" }], "docs": <n>, "indexing": <bool> }`.

- `project_id` sin raíz registrada MUST dar 404; `query` vacía MUST dar 400.
- Con el embedder sin cargar o fallado MUST responder como `POST /v1/search` (503 con el mismo cuerpo).
- `limit` MUST tomar 5 si falta y MUST recortarse a 20.
- Antes de buscar, si el último refresco del proyecto empezó hace más de 60 s (o nunca) y no hay uno en curso, MUST refrescar. Un refresco fallido MUST NOT impedir la búsqueda sobre lo que ya está indexado: la respuesta MUST traer `warning` con el error.
- `indexing` MUST ser `true` si un refresco del proyecto está en curso al responder.
- La búsqueda MUST combinar dos listas, cada una de hasta 50 pasajes del proyecto: los vecinos más cercanos al vector de la consulta (rol `Query`) y los mejores por BM25 de FTS5 con la misma expresión de consulta que usa la búsqueda léxica de memorias. MUST fusionarlas por rango recíproco (`1 / (60 + rango)`, rango desde 1, sumado si un pasaje está en las dos), ordenar por puntaje y desempatar por id de pasaje.
- MUST NOT devolver pasajes de otro proyecto.
- `text` MUST ser el cuerpo del pasaje, sin el breadcrumb repetido.

#### Escenario: por sentido
- GIVEN un proyecto con un pasaje sobre "volcar la cola de ingesta a disco al apagar" y otro sobre "elegir el puerto"
- WHEN se busca "¿qué pasa con los saves pendientes si paro el daemon?"
- THEN el primer hit es el pasaje de la cola

#### Escenario: por identificador
- GIVEN un pasaje que menciona `CHUNK_MAX_TOKENS` y nada más parecido
- WHEN se busca `CHUNK_MAX_TOKENS`
- THEN ese pasaje está en los hits aunque su vector no sea el más cercano

#### Escenario: aislamiento
- GIVEN dos proyectos con el mismo `README.md`
- WHEN se busca en el primero
- THEN todos los hits son del primero

**Tests:** `fuse_*` (puros) y `search_finds_a_passage_by_meaning`, `search_finds_an_identifier_lexically`, `search_never_crosses_projects` en `project_docs/mod.rs`; en `crates/clawg-cli/tests/api.rs`: `docs_search_indexes_on_first_use_and_answers`, `docs_search_of_an_unregistered_project_is_404`, `docs_search_needs_a_token`. Fallan hoy.

## R8. El registro indexa en segundo plano

Cuando `POST /v1/projects` registra o confirma una raíz, el daemon MUST lanzar un refresco de ese proyecto en segundo plano, que espera a que el embedder esté listo. La respuesta del registro MUST NOT esperarlo y MUST NOT cambiar de forma. Un refresco fallido MUST quedar en el log como warn y MUST NOT afectar el registro.

**Test:** `registering_a_root_indexes_its_docs_in_the_background` en `crates/clawg-cli/tests/api.rs`. Falla hoy.

## R9. Herramienta MCP `search_project`

El puente MUST ofrecer `search_project(query, project_path?, limit?)`, que resuelve y registra el proyecto como las demás herramientas (`specs/project`, R3 y R6) y llama a R7. La salida MUST ser, por hit, una línea `[n] <breadcrumb>` seguida del texto. Si `indexing` es `true`, MUST empezar avisando que el índice todavía se está armando; si no hay hits, MUST decirlo. Un 404 de un daemon que no conoce la ruta MUST explicarse como un daemon más viejo que el puente, que hay que reiniciar. Las instrucciones del servidor MUST nombrar `search_project` para preguntas sobre cómo funciona el proyecto.

**Tests:** `search_project_lists_each_hit_under_its_breadcrumb`, `search_project_warns_while_indexing` y `instructions_say_when_to_search_the_project` en `crates/clawg-cli/src/mcp/server.rs` o `tools.rs`; el conteo de herramientas de los tests de handshake pasa de 8 a 9. Fallan hoy.

## R10. Las memorias no cambian

Con un proyecto indexado, `search_all_memories`, `POST /v1/search`, `POST /v1/context`, `search_memory` y `recall_context` MUST devolver exactamente lo mismo que sin índice.

#### Escenario
- GIVEN un agente con memorias en un proyecto
- WHEN se busca y se pide contexto antes y después de indexar docs del mismo proyecto con textos casi iguales a esas memorias
- THEN los resultados son idénticos

**Test:** `indexing_docs_leaves_memory_search_unchanged` en `project_docs/mod.rs`. Pasa hoy de forma trivial; protege la separación.

## R11. Datos de la versión anterior

Una base creada por la versión anterior (migrada hasta V10), con memorias, conclusiones y proyectos, MUST abrir con V11 sin cambiar ninguna fila, y las tablas del índice MUST existir vacías.

**Test:** `a_database_from_the_previous_version_opens_with_an_empty_doc_index` en `crates/clawg-core/src/memory/db.rs`, y la fila de V11 en `published_migration_checksums_are_pinned`. Fallan hoy.

## R12. Medición con `rag-eval --docs`

`rag-eval (--fake | --real) --docs [--root <dir>] [--golden <path>]` MUST:

- indexar la raíz (por defecto, la del workspace de `clawg-eval`) en un store temporal con el embedder elegido, sin leer `~/.clawg*`;
- leer el golden set de docs (por defecto `crates/clawg-eval/golden/project-docs.json`, con `deny_unknown_fields`): una lista de `{ "id", "question", "expected": [{ "path", "heading"? }] }`;
- contar un acierto cuando un hit tiene la ruta esperada y, si se da `heading`, el último tramo de su breadcrumb igual a ese encabezado;
- reportar Hit@1, Hit@3, Hit@5, MRR, el tiempo de indexado, la cantidad de docs y de pasajes, y el detalle de cada pregunta;
- salir con 2 si el golden set no valida, o si un `expected` nombra un doc que el corpus no tiene.

**Tests:** `the_docs_golden_set_parses_and_rejects_unknown_fields`, `hits_count_when_path_and_heading_match` y `an_expected_doc_missing_from_the_corpus_fails_validation` en `crates/clawg-eval/src/bin/rag-eval/docs.rs`; `docs_mode_runs_end_to_end_with_the_fake_embedder` en `crates/clawg-eval/tests/`. Fallan hoy.
