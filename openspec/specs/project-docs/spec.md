# project-docs — Especificación

Fuente de verdad del comportamiento actual del índice de la documentación y el código de un proyecto: qué archivos entran, cómo se trocean, cómo se mantiene el índice al día, cómo se busca (`POST /v1/projects/docs/search`) y la herramienta `search_project` que lo expone al agente. La identidad del proyecto y su raíz registrada están en [`specs/project`](../project/spec.md); la búsqueda de memorias, el ranking y `recall_context`, en [`specs/retrieval`](../retrieval/spec.md) (indexar no los cambia: último apartado); el enmascarado de secretos, en [`specs/secrets`](../secrets/spec.md); las herramientas del puente en general, en [`specs/mcp`](../mcp/spec.md); la medición con `rag-eval --docs`, en [`specs/eval`](../eval/spec.md).

**Origen:** [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15.

## Definiciones

- **Raíz:** la `root_path` que `POST /v1/projects` guardó en `project` para un `project_id` ([`specs/project`](../project/spec.md), R4).
- **Ruta relativa:** la ruta de un archivo respecto de la raíz, con `/` como separador en todas las plataformas.
- **Doc:** archivo de la raíz que el corpus acepta: un Markdown, un manifiesto o un archivo de código. El nombre alcanza a los tres.
- **Sección:** un encabezado ATX de Markdown y las líneas que le siguen hasta el próximo encabezado de cualquier nivel, o el texto anterior al primer encabezado (preámbulo).
- **Breadcrumb:** la ruta relativa seguida de los encabezados (o, en código, las cabeceras de definición) que contienen al pedazo, del más externo al propio, unidos con ` > `. El del preámbulo y el de un manifiesto es sólo la ruta relativa.
- **Pasaje:** una fila de `project_doc_chunk`: el breadcrumb, el texto de un pedazo de un doc, su vector y su entrada léxica.
- **Texto indexado:** `breadcrumb + "\n\n" + cuerpo` (`Passage::indexed_text`, `crates/clawg-core/src/memory/project_docs/chunk.rs:28`). Es lo que se embebe y lo que entra al índice léxico.
- **Huella de un doc:** el sha256 de sus bytes en el disco y el id del modelo que embebió sus pasajes (`Fingerprint`, `crates/clawg-core/src/memory/project_docs/mod.rs:87`).
- **Refresco:** la pasada que deja el índice de un proyecto igual a sus docs en el disco (`refresh`, `mod.rs:345`).

El índice vive en cuatro tablas propias (`project_doc_file`, `project_doc_chunk`, `project_doc_vectors`, `project_doc_fts`; `crates/clawg-core/migrations/V12__project_docs.sql`). `project_doc_vectors` es un `vec0` de 384 dimensiones con `project_id` como partition key, así que el filtro por proyecto se aplica dentro del KNN; `project_doc_fts` guarda su propio texto. Ninguna sentencia del índice nombra una tabla de memorias, y no hay access tracker, tiers ni decaimiento: un doc no "se usa", está en el disco.

## Corpus

La decisión de qué archivo es un doc MUST ser una función pura de la ruta relativa (`classify`, `crates/clawg-core/src/memory/project_docs/corpus.rs:30`):

- es **manifiesto** si su nombre es exactamente `Cargo.toml`, `package.json`, `pyproject.toml` o `go.mod`;
- es **Markdown** si su extensión es `md`, `mdx` o `markdown`, sin distinguir mayúsculas;
- es **código** si su extensión es `rs`, `py`, `ts` o `tsx`, sin distinguir mayúsculas (`CodeLang::from_extension`, `crates/clawg-core/src/memory/project_docs/code.rs:42`);
- cualquier otro archivo MUST NOT entrar.

El recorrido del disco (`walk`, `corpus.rs:69`), aparte de esa función:

- MUST respetar `.gitignore`, `.ignore` y `.git/info/exclude`, también en una raíz sin `.git`;
- MUST NOT entrar a `.git` ni seguir enlaces simbólicos;
- MUST incluir los directorios ocultos que no estén ignorados (`.claude/`, `.github/`);
- MUST NOT considerar un `.gitignore` por encima de la raíz ni el ignore global de git;
- MUST saltear un archivo de más de 256 KiB (`MAX_DOC_BYTES`, `corpus.rs:14`) o que no sea UTF-8 válido, y contarlo como salteado. El tamaño se descarta al recorrer; el UTF-8, al leer el doc en el refresco, que suma ese archivo a los salteados del reporte;
- MUST tomar a lo sumo 2.000 docs por proyecto (`MAX_DOCS`, `corpus.rs:17`), en orden de ruta relativa (orden de bytes de la cadena), y contar el resto como salteados. Se ordena al final y no por el orden del recorrido, para que el tope elija los mismos docs en cada corrida y en cada plataforma;
- MUST devolver un error si la raíz no existe o no es un directorio (`CorpusError::NotADirectory`).

#### Escenario: qué entra
- GIVEN una raíz con `README.md`, `docs/guia.MD`, `.claude/CLAUDE.md`, `crates/a/Cargo.toml`, `src/main.rs`, `target/x.md` con `target/` en `.gitignore`, y `.git/HEAD`
- WHEN se recorre
- THEN los docs son `.claude/CLAUDE.md`, `README.md`, `crates/a/Cargo.toml`, `docs/guia.MD` y `src/main.rs`, en ese orden

#### Escenario: topes
- GIVEN un Markdown de 300 KiB y otro con bytes que no son UTF-8
- WHEN se recorre
- THEN el grande no es doc y cuenta como salteado; el otro se descarta al leerlo y también cuenta como salteado

**Tests:** `classify_takes_markdown_in_any_case`, `classify_takes_manifests_by_exact_name`, `classify_takes_code_by_extension` y `classify_leaves_out_everything_else` (puros), `the_walk_honours_gitignore_and_keeps_hidden_docs`, `oversized_and_binary_files_are_skipped` y `a_root_that_is_not_a_directory_is_an_error` (con directorios temporales) en `crates/clawg-core/src/memory/project_docs/corpus.rs`.

## Troceo de Markdown

`chunk_markdown(ruta, texto, splitter)` (`crates/clawg-core/src/memory/project_docs/chunk.rs:41`) MUST ser pura y:

- reconocer como encabezado una línea con hasta 3 espacios, de 1 a 6 `#` y un espacio o el fin de línea; el texto del encabezado MUST quedar sin los `#` de apertura ni de cierre;
- MUST NOT reconocer un encabezado dentro de un bloque de código cercado (```` ``` ```` o `~~~`, que cierra con el mismo carácter y al menos el mismo largo);
- dar un pasaje por sección con texto no vacío; una sección sin cuerpo MUST NOT dar pasaje, pero su encabezado MUST seguir en el breadcrumb de sus subsecciones;
- formar el breadcrumb con los encabezados de nivel estrictamente menor que la contienen, más el propio;
- recortar un breadcrumb de más de 256 bytes (`MAX_BREADCRUMB_BYTES`, `chunk.rs:12`) por el principio, en un límite de carácter, con `…` adelante, para conservar los encabezados más cercanos;
- si el texto indexado pasa de `CHUNK_MAX_TOKENS` (480, `crates/clawg-core/src/memory/chunker/mod.rs:15`), partir el cuerpo con el `TokenSplitter` en pedazos que, con el breadcrumb, entren en el tope, con `CHUNK_OVERLAP_TOKENS` (64, `chunker/mod.rs:21`) de solapamiento, y repetir el breadcrumb en cada pedazo (`emit`, `chunk.rs:127`).

Sólo se reconocen los encabezados ATX: los de subrayado (`===` o `---` bajo una línea) son texto, porque `---` también es una regla horizontal o el cierre de un front matter. Un encabezado con cuatro espacios de sangría es texto.

El `TokenSplitter` usa el tokenizer de la ingesta si ya cargó; si no, cuenta por bytes (`TokenSplitter::fallback`), como el chunker de los saves.

#### Escenario: secciones y breadcrumb
- GIVEN `docs/guia.md` con `intro`, `# Guía`, `texto A`, `## Instalación`, `### macOS`, `texto B`, `## Uso`, `texto C`
- WHEN se trocea
- THEN los pasajes son `docs/guia.md` / `intro`, `docs/guia.md > Guía` / `texto A`, `docs/guia.md > Guía > Instalación > macOS` / `texto B` y `docs/guia.md > Guía > Uso` / `texto C`

#### Escenario: código cercado
- GIVEN una sección con un bloque ```` ```sh ```` que contiene `# no es encabezado`
- WHEN se trocea
- THEN la sección da un solo pasaje que incluye esa línea

#### Escenario: sección larga
- GIVEN una sección de 1.600 palabras
- WHEN se trocea
- THEN da varios pasajes, cada uno con el breadcrumb de la sección, ninguno de más de 480 tokens, y el final de cada uno se repite al principio del siguiente

**Tests:** en `crates/clawg-core/src/memory/project_docs/chunk.rs`: `sections_carry_their_breadcrumb`, `a_heading_inside_a_fence_does_not_split`, `a_fence_closes_only_with_its_own_character_and_length`, `an_empty_section_gives_no_passage_but_names_its_children`, `closing_hashes_are_not_part_of_the_heading`, `a_heading_indented_four_spaces_is_text`, `a_long_section_is_split_with_overlap_under_the_cap`, `a_long_breadcrumb_keeps_its_nearest_headings` y `an_empty_doc_has_no_passages`.

## Troceo de manifiestos

Un manifiesto MUST dar un único pasaje con breadcrumb igual a su ruta relativa y su texto completo como cuerpo, partido como el de Markdown si no entra en el tope (`chunk_manifest`, `chunk.rs:80`). Uno sin texto que no sea blanco MUST NOT dar pasaje.

**Tests:** `a_manifest_is_one_passage_named_by_its_path` y `an_empty_doc_has_no_passages` en `chunk.rs`.

## Troceo de código

Un archivo de código MUST trocearse con `chunk_code(ruta, texto, lenguaje, splitter)` (`crates/clawg-core/src/memory/project_docs/code.rs:63`), pura, con un parser de tree-sitter por lenguaje (Rust, Python, TypeScript y TSX; uno por llamada, así que no hay estado compartido). El método es de arriba abajo y junta los hermanos chicos hasta el tope, de modo que una definición se corta sólo si ella sola no entra:

- el tamaño de un nodo MUST medirse en caracteres que no son espacio en blanco (`non_whitespace_len`), para que el tope no dependa del formateo; el tope es `CODE_CHUNK_MAX_CHARS` (1.200, `code.rs:13`);
- los hijos de la raíz MUST recorrerse en orden: un nodo que entra en el tope se suma al pedazo en curso si el total sigue dentro, o abre un pedazo nuevo; un nodo que no entra cierra el pedazo en curso y se trocea por sus propios hijos con la misma regla (`split`, `code.rs:142`); una hoja que no entra va sola en su pedazo;
- el texto de un pedazo MUST ser el rango del archivo desde el principio de su primer nodo hasta el final del último, con los comentarios y las líneas en blanco entre ellos, así un doc comment (`///`, `#`, `/** */`) queda con la definición que documenta;
- el breadcrumb MUST ser la ruta seguida, con ` > `, de la cabecera de cada contenedor en el que se bajó: su primera línea sin el cuerpo (lo que viene desde `{`, o sin el `:` final de Python), recortada a 120 bytes (`MAX_LABEL_BYTES`), por ejemplo `crates/a/src/lib.rs > impl DocIndex`. Un nodo que es el cuerpo de su contenedor (`block`, `declaration_list`, `class_body`…) no suma etiqueta, ni uno que empieza en la misma línea que la última etiqueta (`export` y `class` comparten una). Si un pedazo junta nodos de distintas etiquetas, lleva las de su nodo más grande. El breadcrumb pasa por el mismo recorte de 256 bytes que el de Markdown;
- cada pedazo MUST pasar además por el tope de tokens de Markdown (480 con el breadcrumb), partido con solapamiento si no entra, así una hoja larga no se trunca en silencio al embeber;
- si el parser no devuelve árbol, el archivo entero MUST partirse con el `TokenSplitter` bajo su ruta; un árbol con nodos de error se trocea igual;
- un archivo sin texto que no sea blanco MUST NOT dar pasajes, y un pedazo sin ningún carácter alfanumérico (por ejemplo, sólo `}`) tampoco.

No se suman los imports al breadcrumb: alargan cada pasaje y el agente los tiene a un `read` de distancia. Todo lo demás (huella, refresco, secretos, búsqueda, `search_project`) es igual para el código que para los docs.

#### Escenario: funciones chicas juntas, un impl grande partido
- GIVEN un `lib.rs` con un `use`, una función de 3 líneas con su `///`, y un `impl Store` cuyo cuerpo pasa el tope y tiene dos métodos
- WHEN se trocea
- THEN el `use`, el doc comment y la función salen en un mismo pasaje con breadcrumb `lib.rs`, y cada método sale bajo `lib.rs > impl Store`, sin cortar ninguno por la mitad

#### Escenario: Python y TypeScript
- GIVEN una clase de Python y una de TypeScript que pasan el tope, con dos métodos cada una
- WHEN se trocean
- THEN sus métodos salen bajo `<ruta> > class <Nombre>` (`<ruta> > export class <Nombre>` en TypeScript con `export`)

**Tests:** en `crates/clawg-core/src/memory/project_docs/code.rs`: `non_whitespace_size_ignores_formatting`, `small_siblings_merge_and_keep_their_doc_comments`, `a_node_over_the_cap_is_split_by_its_children_under_its_header`, `python_and_typescript_classes_split_by_method`, `a_leaf_over_the_cap_falls_back_to_the_token_splitter`, `every_passage_fits_the_token_cap` y `an_empty_file_has_no_passages`; `classify_takes_code_by_extension` en `corpus.rs`; `a_refresh_indexes_code_by_definition` en `project_docs/mod.rs`.

## Refresco incremental

`refresh` (`crates/clawg-core/src/memory/project_docs/mod.rs:345`) MUST dejar en el índice del proyecto exactamente los docs del disco, y:

- un doc cuya huella coincide con la guardada MUST NOT re-leerse en pasajes ni re-embeberse;
- un doc nuevo o cambiado (otro sha256, u otro modelo) MUST reemplazar todos sus pasajes, vectores y entradas léxicas en **una** transacción por doc (`replace_doc`, `mod.rs:464`), así una búsqueda nunca ve medio doc. El embedding se hace antes de pedir la conexión, para no bloquear los saves mientras tanto;
- un doc que ya no está en el corpus MUST perder todos sus pasajes y su huella (`remove_docs`, `mod.rs:499`);
- un doc vacío o sin secciones con texto MUST guardar su huella sin pasajes;
- si la raíz no existe o no es un directorio, MUST NOT borrar nada y MUST devolver el error: el recorrido falla antes de leer las huellas guardadas, así que un disco "vacío" por error no arma un plan que borre todo;
- si el embedder falla en un doc, ese doc MUST quedar como estaba, el refresco MUST terminar con el error y el siguiente MUST reintentarlo: la huella vieja no coincide con el disco;
- MUST NOT leer ni escribir `memory`, `episodic_trajectory`, `procedural_skill`, sus vectores, su FTS ni el access tracker;
- MUST devolver cuántos docs vio, reindexó, dejó igual, quitó y salteó, y cuántos pasajes escribió (`RefreshReport`).

La huella se toma de los bytes que se leyeron para trocear, sin enmascarar, así un cambio en un secreto también reindexa. La decisión de qué hacer con cada doc MUST ser una función pura de las huellas del disco y las guardadas (`plan_refresh`, `mod.rs:104`). El `model_id` de cada doc es el del embedder: un cambio de modelo, o del pooling que su id nombra ([`specs/storage`](../storage/spec.md), «Pooling mean y re-embebido al arrancar»), reindexa todo sin código extra.

Sólo MUST correr un refresco por proyecto a la vez: un pedido de refresco mientras otro corre MUST volver enseguida diciendo que ya hay uno en curso, sin esperar (`DocIndex::begin` y `try_refresh`, `mod.rs:161` y `:211`). La marca se suelta también si el refresco falla o su tarea se cancela. La raíz se lee de `project` en cada refresco: si un nuevo registro la cambia, el próximo usa la nueva.

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

**Tests:** en `crates/clawg-core/src/memory/project_docs/mod.rs`: `plan_keeps_what_did_not_change_and_rewrites_what_did`, `plan_rewrites_every_doc_embedded_by_another_model` y `plan_of_an_empty_tree_drops_everything` (puros); `an_unchanged_project_embeds_nothing`, `a_changed_doc_replaces_its_passages_and_a_deleted_one_loses_them`, `another_model_reindexes_every_doc`, `a_missing_root_deletes_nothing`, `a_failed_embedding_keeps_the_doc_as_it_was`, `a_second_refresh_does_not_wait_for_the_first`, `an_unregistered_project_is_refused` y `refreshing_never_touches_memories`, contra un store en memoria y un directorio temporal.

## Secretos

El texto de un doc MUST pasar por `redact` (`crates/clawg-core/src/memory/redact.rs:131`) antes de trocearse. Ningún pasaje, vector ni entrada léxica MUST contener un secreto de formato conocido. La huella usa los bytes del disco, sin enmascarar.

#### Escenario
- GIVEN `docs/setup.md` con una clave `sk-ant-…` de formato válido
- WHEN se refresca y se busca la clave literal
- THEN ningún pasaje la contiene y la búsqueda no la devuelve

**Test:** `a_secret_in_a_doc_never_reaches_the_index` en `project_docs/mod.rs`.

## Búsqueda

`POST /v1/projects/docs/search` (`search_docs`, `crates/clawg-cli/src/daemon/api.rs:1109`; ruta en `:553`), con token, recibe `{ "project_id": "...", "query": "...", "limit": n }` (`deny_unknown_fields`) y responde `{ "hits": [{ "path", "breadcrumb", "text", "score" }], "docs": <n>, "indexing": <bool>, "warning"?: "..." }`. `docs` es cuántos docs tiene el índice del proyecto.

- `query` vacía o sólo espacios MUST dar 400, y un `project_id` sin raíz registrada MUST dar 404, en ese orden.
- Con el embedder sin cargar o fallado MUST responder como `POST /v1/search` (503 con el mismo cuerpo, [`specs/ingest`](../ingest/spec.md)). Se comprueba después del 404.
- `limit` MUST tomar 5 si falta (`DEFAULT_DOCS_LIMIT`, `api.rs:1078`) y MUST quedar entre 1 y 20 (`clamp_docs_limit`, `api.rs:1080`; `MAX_DOCS_LIMIT`, `api.rs:1076`): 0 vale 1.
- Antes de buscar, si el último refresco del proyecto empezó hace 60 s o más (`REFRESH_INTERVAL`, `mod.rs:32`) o nunca empezó, y no hay uno en curso, MUST refrescar (`is_due`, `needs_refresh`). Un refresco fallido MUST NOT impedir la búsqueda sobre lo que ya está indexado: la respuesta MUST traer `warning` con el error.
- Si hay un refresco del proyecto en curso y el índice del proyecto está vacío (el primero, que suele lanzar el registro del puente un momento antes), MUST esperarlo hasta 20 s (`FIRST_INDEX_WAIT`, `api.rs:1069`) antes de buscar, para no contestar "los docs no dicen nada" mientras se arman. Con un índice ya armado no espera: contesta con lo que hay.
- `indexing` MUST ser `true` si un refresco del proyecto está en curso al responder.
- La búsqueda (`search`, `mod.rs:258`) MUST combinar dos listas, cada una de hasta 50 pasajes del proyecto (`CANDIDATES_PER_CHANNEL`, `mod.rs:37`): los vecinos más cercanos al vector de la consulta (rol `Query`) y los mejores por BM25 de FTS5 con la misma expresión de consulta que usa la búsqueda léxica de memorias (`fts_match_expression` con `MAX_LEXICAL_TOKENS`, `crates/clawg-core/src/memory/search.rs:563` y `:543`). MUST fusionarlas por rango recíproco (`fuse`, `mod.rs:243`: `1 / (60 + rango)`, rango desde 1, sumado si un pasaje está en las dos; `RRF_K`, `mod.rs:42`), ordenar por puntaje y desempatar por id de pasaje menor. Usa rangos y no el piso de coseno ni el bono léxico de las memorias, que se calibraron sobre otro corpus.
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

**Tests:** `fuse_adds_the_reciprocal_ranks_of_both_lists`, `fuse_breaks_ties_by_id_and_stops_at_the_limit`, `search_finds_a_passage_by_meaning`, `search_finds_an_identifier_lexically`, `search_never_crosses_projects`, `waiting_for_a_running_refresh_ends_when_it_does` y `a_refresh_is_due_again_only_after_the_interval` en `crates/clawg-core/src/memory/project_docs/mod.rs`; `a_docs_search_returns_five_by_default_and_never_more_than_twenty` en `crates/clawg-cli/src/daemon/api.rs` (mod tests); en `crates/clawg-cli/tests/api.rs`: `docs_search_indexes_on_first_use_and_answers`, `docs_search_takes_at_most_the_limit`, `docs_search_of_an_unregistered_project_is_404`, `docs_search_with_an_empty_query_is_a_bad_request`, `docs_search_waits_for_the_embedder_like_any_search`, `docs_search_needs_a_token`, `a_root_that_vanished_still_answers_from_its_index` y `a_search_during_the_first_index_waits_for_it`.

## El registro indexa en segundo plano

Cuando `POST /v1/projects` registra o confirma una raíz, el daemon MUST lanzar un refresco de ese proyecto en segundo plano, que espera a que el embedder deje de estar en `loading` (`spawn_doc_refresh`, `crates/clawg-cli/src/daemon/api.rs:1044`; llamado en `:1036`). La respuesta del registro MUST NOT esperarlo y MUST NOT cambiar de forma. Un refresco fallido, o un embedder que no cargó, MUST quedar en el log como warn y MUST NOT afectar el registro. El puente registra cada raíz antes de su primera herramienta ([`specs/project`](../project/spec.md), R6), así que el primer indexado empieza con la sesión y no con la primera búsqueda.

**Test:** `registering_a_root_indexes_its_docs_in_the_background` en `crates/clawg-cli/tests/api.rs`.

## Herramienta MCP `search_project`

El puente MUST ofrecer `search_project(query, project_path?, limit?)` (`SearchProjectArgs`, `crates/clawg-cli/src/mcp/server.rs:178`; `search_project_impl`, `server.rs:717`), que resuelve y registra el proyecto como las demás herramientas ([`specs/project`](../project/spec.md), R3 y R6) y llama a la búsqueda de arriba. No lleva `agent_id`: la documentación es del proyecto, no de un agente, así que no hay sharing ni visibilidad que aplicar. Sin proyecto (ni `project_path` ni uno por defecto) MUST fallar diciendo que se pase `project_path`.

La salida (`format_docs`, `crates/clawg-cli/src/mcp/tools.rs:237`) MUST ser, por hit, una línea `[n] <breadcrumb>` seguida del texto del pasaje entero, separados por una línea en blanco. Además:

- si `indexing` es `true`, MUST empezar avisando que el índice todavía se está armando y que los resultados pueden estar incompletos;
- si hay `warning`, MUST nombrarlo antes de los resultados (`The index could not be refreshed (…); these results are from before.`);
- si no hay hits, MUST decirlo, y MUST distinguir un índice vacío y sin refresco en curso ("No docs are indexed…") de uno que no tiene ningún pasaje que coincida;
- un 404 sin cuerpo de error de un daemon que no conoce la ruta MUST explicarse como un daemon más viejo que el puente, que hay que reiniciar con `clawg stop`.

Las instrucciones del servidor (`INSTRUCTIONS`, `server.rs:108`) MUST nombrar `search_project` para preguntas sobre cómo funciona el proyecto (su diseño, convenciones, comandos o dónde está implementado algo), antes de abrir archivos. La descripción de la herramienta (`server.rs:939`) y las instrucciones MUST decir que busca en los docs y en el código del proyecto. `recall_context` no consulta este índice.

**Tests:** `search_project_lists_each_hit_under_its_breadcrumb` e `instructions_say_when_to_search_the_project` en `crates/clawg-cli/src/mcp/server.rs`; `docs_are_numbered_under_their_breadcrumbs`, `search_project_warns_while_indexing`, `an_empty_index_and_no_match_read_differently` y `a_failed_refresh_is_named_before_the_results` en `crates/clawg-cli/src/mcp/tools.rs`. El conteo de herramientas (once) está en `every_tool_is_registered_under_its_wire_name` (`server.rs`) y en los tests de `handshake.rs` ([`specs/mcp`](../mcp/spec.md)).

## Indexar no cambia las memorias

Con un proyecto indexado, `search_all_memories`, `POST /v1/search`, `POST /v1/context`, `search_memory` y `recall_context` MUST devolver exactamente lo mismo que sin índice: ninguna de esas rutas lee las tablas del índice. El detalle de lo que no cambia está en [`specs/retrieval`](../retrieval/spec.md) («Indexar un proyecto no cambia la búsqueda de memorias»).

**Tests:** `indexing_docs_leaves_memory_search_unchanged` y `refreshing_never_touches_memories` en `project_docs/mod.rs`.

## Datos de la versión anterior

Una base creada por la versión anterior (migrada hasta V11), con memorias, conclusiones y proyectos, MUST abrir con V12 sin cambiar ninguna fila, y las cuatro tablas del índice MUST existir vacías. La apertura sigue la regla de [`specs/storage`](../storage/spec.md) (snapshot `pre-migration` antes de aplicar la migración pendiente).

**Test:** `a_database_from_the_previous_version_opens_with_an_empty_doc_index` en `crates/clawg-core/src/memory/db.rs`, y la fila de V12 en `published_migration_checksums_are_pinned` (`crates/clawg-core/src/memory/migrations.rs`).

## Invariantes

- El índice se deriva del disco y se puede borrar y rehacer sin perder nada del usuario.
- Un doc se reemplaza entero o no se toca: ninguna búsqueda ve pasajes de dos versiones del mismo doc.
- El refresco no corre dos veces a la vez para un proyecto, y ni el registro ni la búsqueda lo esperan, salvo la espera del primer índice.

## Límites conocidos

- La calidad de la búsqueda está medida, no exigida. `rag-eval --real --docs` sobre este repo (199 archivos, 10 preguntas de docs y 5 de código, e5-small con pooling mean, Linux, binario debug) dio Hit@3 0,27 y MRR@10 0,26 con docs y código juntos; los números de Windows no están tomados. Las preguntas y el criterio de acierto están en [`specs/eval`](../eval/spec.md).
- El canal léxico une todos los tokens de la consulta con OR, palabras vacías incluidas, y eso empuja pasajes que sólo comparten "de", "el" o "the". Filtrarlo es una mejora posible; no cambia este contrato.
- No hay watcher del filesystem ni herramienta `index_project`: el índice se pone al día al registrar la raíz y en la próxima búsqueda pasado el intervalo. Un doc editado aparece en la búsqueda siguiente a los 60 s.
