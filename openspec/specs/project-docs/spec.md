# project-docs — Especificación

Fuente de verdad del comportamiento actual del índice de la documentación y el código de un proyecto: de qué raíces lee el daemon los archivos, qué archivos entran, cómo se trocean, cómo se mantiene el índice al día, cómo se busca (`POST /v1/projects/docs/search`), cómo el usuario agrega una raíz (`clawg projects add`) y la herramienta `search_project` que lo expone al agente. La identidad del proyecto, su raíz registrada y la marca `index_docs` que guarda el registro están en [`specs/project`](../project/spec.md); la búsqueda de memorias, el ranking y `recall_context`, en [`specs/retrieval`](../retrieval/spec.md) (indexar no los cambia: último apartado); el enmascarado de secretos, en [`specs/secrets`](../secrets/spec.md); las herramientas del puente en general, en [`specs/mcp`](../mcp/spec.md); la medición con `rag-eval --docs`, en [`specs/eval`](../eval/spec.md).

**Origen:**
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15.
- [project-root-guard](../../changes/archive/2026-10-15-project-root-guard/proposal.md), archivado el 2026-10-15 (qué raíces se leen, el 403 de la búsqueda, `clawg projects add` y la columna `index_docs`).

## Definiciones

- **Raíz:** la `root_path` que `POST /v1/projects` guardó en `project` para un `project_id` ([`specs/project`](../project/spec.md), R4).
- **Raíz del puente:** la raíz que el puente resolvió de su cwd al arrancar (`default_project`, `crates/clawg-cli/src/mcp/server.rs:399-401`).
- **Raíz aceptada:** una fila de `project` con `index_docs = 1`: el daemon puede leer sus archivos.
- **Raíz sólo registrada:** una fila de `project` con `index_docs = 0`: sus memorias se re-keyean y se buscan como las de cualquier proyecto, pero el daemon no lee sus archivos.
- **Ruta relativa:** la ruta de un archivo respecto de la raíz, con `/` como separador en todas las plataformas.
- **Doc:** archivo de la raíz que el corpus acepta: un Markdown, un manifiesto o un archivo de código. El nombre alcanza a los tres.
- **Sección:** un encabezado ATX de Markdown y las líneas que le siguen hasta el próximo encabezado de cualquier nivel, o el texto anterior al primer encabezado (preámbulo).
- **Breadcrumb:** la ruta relativa seguida de los encabezados (o, en código, las cabeceras de definición) que contienen al pedazo, del más externo al propio, unidos con ` > `. El del preámbulo y el de un manifiesto es sólo la ruta relativa.
- **Pasaje:** una fila de `project_doc_chunk`: el breadcrumb, el texto de un pedazo de un doc, su vector y su entrada léxica.
- **Texto indexado:** `breadcrumb + "\n\n" + cuerpo` (`Passage::indexed_text`, `crates/clawg-core/src/memory/project_docs/chunk.rs:28`). Es lo que se embebe y lo que entra al índice léxico.
- **Huella de un doc:** el sha256 de sus bytes en el disco y el id del modelo que embebió sus pasajes (`Fingerprint`, `crates/clawg-core/src/memory/project_docs/mod.rs:91`).
- **Refresco:** la pasada que deja el índice de un proyecto igual a sus docs en el disco (`refresh`, `mod.rs:349`).

El índice vive en cuatro tablas propias (`project_doc_file`, `project_doc_chunk`, `project_doc_vectors`, `project_doc_fts`; `crates/clawg-core/migrations/V12__project_docs.sql`). `project_doc_vectors` es un `vec0` de 384 dimensiones con `project_id` como partition key, así que el filtro por proyecto se aplica dentro del KNN; `project_doc_fts` guarda su propio texto. Ninguna sentencia del índice nombra una tabla de memorias, y no hay access tracker, tiers ni decaimiento: un doc no "se usa", está en el disco.

## Qué raíces se leen

El daemon MUST leer los archivos sólo de una raíz aceptada. Una raíz queda aceptada cuando la registra el puente que arrancó en ella, o cuando el usuario la agrega con `clawg projects add` (abajo). Cualquier otra raíz que el modelo nombre en `project_path` queda sólo registrada. La marca se guarda en `project.index_docs` y sólo sube ([`specs/project`](../project/spec.md), R4 y R6).

La guarda está junto a la lectura, no en quien la pide. `root_of` (`crates/clawg-core/src/memory/project_docs/mod.rs:433`) lee `root_path` e `index_docs`. Con una raíz sólo registrada MUST devolver `DocIndexError::NotAccepted` (`mod.rs:51`), un error distinto de `UnknownProject`. `try_refresh` pide la raíz a `root_of` antes de recorrer nada (`mod.rs:225`). Así ningún camino lee el disco de una raíz sólo registrada, aunque quien llame no mire la marca.

#### Escenario: registrada sin indexar
- GIVEN una raíz con un `README.md`
- WHEN se registra sin `index_docs` (o con `false`)
- THEN `project` tiene su fila con `index_docs = 0` y, una vez cargado el embedder, `project_doc_file` no tiene filas de ella

#### Escenario: el refresco se niega
- GIVEN una raíz sólo registrada
- WHEN se llama a `try_refresh` sobre ella
- THEN falla con `NotAccepted` y el índice no tiene ninguna fila

**Tests:** `a_root_registered_without_its_docs_is_not_indexed` en `crates/clawg-cli/tests/api.rs`; `a_root_that_was_not_accepted_is_not_read` en `crates/clawg-core/src/memory/project_docs/mod.rs`.

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

`refresh` (`crates/clawg-core/src/memory/project_docs/mod.rs:349`) MUST dejar en el índice del proyecto exactamente los docs del disco, y:

- un doc cuya huella coincide con la guardada MUST NOT re-leerse en pasajes ni re-embeberse;
- un doc nuevo o cambiado (otro sha256, u otro modelo) MUST reemplazar todos sus pasajes, vectores y entradas léxicas en **una** transacción por doc (`replace_doc`, `mod.rs:478`), así una búsqueda nunca ve medio doc. El embedding se hace antes de pedir la conexión, para no bloquear los saves mientras tanto;
- un doc que ya no está en el corpus MUST perder todos sus pasajes y su huella (`remove_docs`, `mod.rs:513`);
- un doc vacío o sin secciones con texto MUST guardar su huella sin pasajes;
- si la raíz no existe o no es un directorio, MUST NOT borrar nada y MUST devolver el error: el recorrido falla antes de leer las huellas guardadas, así que un disco "vacío" por error no arma un plan que borre todo;
- si el embedder falla en un doc, ese doc MUST quedar como estaba, el refresco MUST terminar con el error y el siguiente MUST reintentarlo: la huella vieja no coincide con el disco;
- MUST NOT leer ni escribir `memory`, `episodic_trajectory`, `procedural_skill`, sus vectores, su FTS ni el access tracker;
- MUST devolver cuántos docs vio, reindexó, dejó igual, quitó y salteó, y cuántos pasajes escribió (`RefreshReport`).

La huella se toma de los bytes que se leyeron para trocear, sin enmascarar, así un cambio en un secreto también reindexa. La decisión de qué hacer con cada doc MUST ser una función pura de las huellas del disco y las guardadas (`plan_refresh`, `mod.rs:108`). El `model_id` de cada doc es el del embedder: un cambio de modelo, o del pooling que su id nombra ([`specs/storage`](../storage/spec.md), «Pooling mean y re-embebido al arrancar»), reindexa todo sin código extra.

Sólo MUST correr un refresco por proyecto a la vez: un pedido de refresco mientras otro corre MUST volver enseguida diciendo que ya hay uno en curso, sin esperar (`DocIndex::begin` y `try_refresh`, `mod.rs:165` y `:215`). La marca se suelta también si el refresco falla o su tarea se cancela. La raíz y su `index_docs` se leen de `project` en cada refresco (`root_of`, arriba). Si un nuevo registro cambia la raíz, el próximo refresco usa la nueva. Una raíz sólo registrada no se refresca: el refresco falla con `NotAccepted` antes de recorrer el disco.

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

**Tests:** en `crates/clawg-core/src/memory/project_docs/mod.rs`: `plan_keeps_what_did_not_change_and_rewrites_what_did`, `plan_rewrites_every_doc_embedded_by_another_model` y `plan_of_an_empty_tree_drops_everything` (puros); `an_unchanged_project_embeds_nothing`, `a_changed_doc_replaces_its_passages_and_a_deleted_one_loses_them`, `another_model_reindexes_every_doc`, `a_missing_root_deletes_nothing`, `a_failed_embedding_keeps_the_doc_as_it_was`, `a_second_refresh_does_not_wait_for_the_first`, `an_unregistered_project_is_refused`, `a_root_that_was_not_accepted_is_not_read` y `refreshing_never_touches_memories`, contra un store en memoria y un directorio temporal.

## Secretos

El texto de un doc MUST pasar entero por `redact` (`crates/clawg-core/src/memory/redact.rs:207`; `project_docs/mod.rs:382`) antes de trocearse. Ningún pasaje, vector ni entrada léxica MUST contener un secreto que reconozca el detector de [`specs/secrets`](../secrets/spec.md) (R1); una marca privada abierta en un doc enmascara, en el índice, hasta el próximo cierre o el final del archivo. La huella usa los bytes del disco, sin enmascarar.

#### Escenario
- GIVEN `docs/setup.md` con una clave `sk-ant-…` de formato válido
- WHEN se refresca y se busca la clave literal
- THEN ningún pasaje la contiene y la búsqueda no la devuelve

**Test:** `a_secret_in_a_doc_never_reaches_the_index` en `project_docs/mod.rs`.

## Búsqueda

`POST /v1/projects/docs/search` (`search_docs`, `crates/clawg-cli/src/daemon/api.rs:1466`; ruta en `:725`), con token, recibe `{ "project_id": "...", "query": "...", "limit": n }` (`deny_unknown_fields`) y responde `{ "hits": [{ "path", "breadcrumb", "text", "score" }], "docs": <n>, "indexing": <bool>, "warning"?: "..." }`. `docs` es cuántos docs tiene el índice del proyecto.

- En este orden: una `query` vacía o sólo de espacios MUST dar 400; un `project_id` sin raíz registrada MUST dar 404; una raíz sólo registrada MUST dar 403, sin refrescar ni buscar.
- El `error` del 403 (`not_accepted`, `api.rs:1451`) MUST nombrar la raíz y el comando con que el usuario la acepta. Cuando el perfil del daemon no es `prod`, el comando lleva su `--profile`: «clawg reads the files of the project a bridge was started in, and of the ones the user added; `<raíz>` is neither. Ask the user to run ``clawg projects add "<raíz>" --profile dev`` if they want it searched, then call again». Lo arma el daemon, que sabe su perfil, así que también lo lee bien un puente de una versión anterior.
- Con el embedder sin cargar o fallado MUST responder como `POST /v1/search` (503 con el mismo cuerpo, [`specs/ingest`](../ingest/spec.md)). Se comprueba después del 404 y del 403.
- `limit` MUST tomar 5 si falta (`DEFAULT_DOCS_LIMIT`, `api.rs:1418`) y MUST quedar entre 1 y 20 (`clamp_docs_limit`, `api.rs:1420`; `MAX_DOCS_LIMIT`, `api.rs:1416`): 0 vale 1.
- Antes de buscar, si el último refresco del proyecto empezó hace 60 s o más (`REFRESH_INTERVAL`, `mod.rs:32`) o nunca empezó, y no hay uno en curso, MUST refrescar (`is_due`, `needs_refresh`). Un refresco fallido MUST NOT impedir la búsqueda sobre lo que ya está indexado: la respuesta MUST traer `warning` con el error.
- Si hay un refresco del proyecto en curso y el índice del proyecto está vacío (el primero, que suele lanzar el registro del puente un momento antes), MUST esperarlo hasta 20 s (`FIRST_INDEX_WAIT`, `api.rs:1409`) antes de buscar, para no contestar "los docs no dicen nada" mientras se arman. Con un índice ya armado no espera: contesta con lo que hay.
- `indexing` MUST ser `true` si un refresco del proyecto está en curso al responder.
- La búsqueda (`search`, `mod.rs:262`) MUST combinar dos listas, cada una de hasta 50 pasajes del proyecto (`CANDIDATES_PER_CHANNEL`, `mod.rs:37`): los vecinos más cercanos al vector de la consulta (rol `Query`) y los mejores por BM25 de FTS5 con la misma expresión de consulta que usa la búsqueda léxica de memorias (`fts_match_expression` con `MAX_LEXICAL_TOKENS`, `crates/clawg-core/src/memory/search.rs:625` y `:605`). MUST fusionarlas por rango recíproco (`fuse`, `mod.rs:247`: `1 / (60 + rango)`, rango desde 1, sumado si un pasaje está en las dos; `RRF_K`, `mod.rs:42`), ordenar por puntaje y desempatar por id de pasaje menor. Usa rangos y no el piso de coseno ni el bono léxico de las memorias, que se calibraron sobre otro corpus.
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

#### Escenario: raíz no aceptada
- GIVEN un daemon `dev` y una raíz sólo registrada con un `README.md`
- WHEN se buscan sus docs
- THEN la respuesta es 403, su `error` contiene ``clawg projects add "<raíz>" --profile dev``, y el índice no tiene filas de ella

**Tests:** `fuse_adds_the_reciprocal_ranks_of_both_lists`, `fuse_breaks_ties_by_id_and_stops_at_the_limit`, `search_finds_a_passage_by_meaning`, `search_finds_an_identifier_lexically`, `search_never_crosses_projects`, `waiting_for_a_running_refresh_ends_when_it_does` y `a_refresh_is_due_again_only_after_the_interval` en `crates/clawg-core/src/memory/project_docs/mod.rs`; `a_docs_search_returns_five_by_default_and_never_more_than_twenty` y `the_refusal_names_the_command_for_the_daemons_profile` (puro) en `crates/clawg-cli/src/daemon/api.rs` (mod tests); en `crates/clawg-cli/tests/api.rs`: `docs_search_indexes_on_first_use_and_answers`, `docs_search_takes_at_most_the_limit`, `docs_search_of_an_unregistered_project_is_404`, `searching_the_docs_of_a_root_that_was_not_accepted_is_forbidden`, `docs_search_with_an_empty_query_is_a_bad_request`, `docs_search_waits_for_the_embedder_like_any_search`, `docs_search_needs_a_token`, `a_root_that_vanished_still_answers_from_its_index` y `a_search_during_the_first_index_waits_for_it`.

## Registrar una raíz aceptada la indexa en segundo plano

Cuando `POST /v1/projects` registra o confirma una raíz y, después del registro, la raíz está aceptada (`RegisterOutcome::index_docs`), el daemon MUST lanzar un refresco de ese proyecto en segundo plano. Ese refresco espera a que el embedder deje de estar en `loading` (`spawn_doc_refresh`, `crates/clawg-cli/src/daemon/api.rs:1384`; llamado en `:1292-1294`). Un registro que deja la raíz sólo registrada MUST NOT lanzarlo. La respuesta del registro MUST NOT esperar el refresco y MUST NOT cambiar de forma. Un refresco fallido, o un embedder que no cargó, MUST quedar en el log como warn y MUST NOT afectar el registro.

El puente registra su propia raíz pidiendo indexarla antes de su primera herramienta ([`specs/project`](../project/spec.md), R6). Así, el primer indexado empieza con la sesión y no con la primera búsqueda.

#### Escenario
- GIVEN una raíz con un `README.md`
- WHEN se registra con `index_docs: true`
- THEN la respuesta es 200 y, poco después, el índice del proyecto tiene ese doc

**Tests:** `registering_a_root_indexes_its_docs_in_the_background` y `a_root_registered_without_its_docs_is_not_indexed` en `crates/clawg-cli/tests/api.rs`.

## `clawg projects add`

`clawg projects add <RUTA> [--profile P]` (`add`, `crates/clawg-cli/src/projects.rs:23`; rama en `crates/clawg-cli/src/main.rs:204-220`) es el camino por el que el usuario, no el modelo, acepta una raíz que no es la de un puente. MUST:

- resolver la raíz de RUTA con `ProjectIdentity::resolve` ([`specs/project`](../project/spec.md), R1);
- registrarla con `index_docs: true` y con el id viejo de RUTA si difiere, a través del daemon que está corriendo;
- fallar con `AddError::NotADirectory` sin llamar al daemon si la raíz no es un directorio;
- al terminar, imprimir `added <raíz> as project <id>; the daemon is indexing its files in the background`.

MUST NOT abrir la base ni arrancar un daemon. Sin daemon falla como `clawg undelete`, con el texto de `memories::explain_unavailable`. El daemon también rechaza con 400 un registro con `index_docs: true` cuya raíz no es un directorio, sin crear la fila ([`specs/project`](../project/spec.md), R4).

#### Escenario
- GIVEN un daemon corriendo y una raíz sólo registrada `<tmp>/other`, en la que la búsqueda de docs da 403
- WHEN el usuario corre `clawg projects add <tmp>/other/docs`
- THEN imprime la raíz `<tmp>/other` con su id, `<tmp>/other` queda aceptada, y una búsqueda posterior de sus docs responde con su `README.md`

**Tests:** `projects_add_takes_a_path_and_a_profile` (parseo) en `crates/clawg-cli/src/cli.rs`; `adding_a_project_lets_its_docs_be_searched` y `adding_a_path_that_is_not_a_directory_is_refused` en `crates/clawg-cli/src/projects.rs`; `registering_a_missing_root_to_index_it_is_a_bad_request` en `crates/clawg-cli/tests/api.rs`.

## Herramienta MCP `search_project`

El puente MUST ofrecer `search_project(query, project_path?, limit?)` (`SearchProjectArgs`, `crates/clawg-cli/src/mcp/server.rs:224`; `search_project_impl`, `server.rs:930`). La herramienta resuelve y registra el proyecto como las demás ([`specs/project`](../project/spec.md), R3 y R6) y llama a la búsqueda de arriba. Pide indexar sólo la raíz del puente, así que con un `project_path` ajeno busca únicamente en una raíz que el usuario ya agregó. No lleva `agent_id`: la documentación es del proyecto, no de un agente, así que no hay sharing ni visibilidad que aplicar. Sin proyecto (ni `project_path` ni uno por defecto) MUST fallar diciendo que se pase `project_path`. Una raíz cuyo id ya pertenece a otra raíz MUST fallar como en las demás herramientas ([`specs/project`](../project/spec.md), R6).

La descripción de `project_path` (`server.rs:231-233`) MUST decir que, por defecto, se busca en el directorio en que arrancó el puente, y que los archivos de otro proyecto se buscan sólo después de que el usuario lo agregue con `clawg projects add`.

La salida (`format_docs`, `crates/clawg-cli/src/mcp/tools.rs:325`) MUST ser, por hit, una línea `[n] <breadcrumb>` seguida del texto del pasaje entero, separados por una línea en blanco. Además:

- si `indexing` es `true`, MUST empezar avisando que el índice todavía se está armando y que los resultados pueden estar incompletos;
- si hay `warning`, MUST nombrarlo antes de los resultados (`The index could not be refreshed (…); these results are from before.`);
- si no hay hits, MUST decirlo, y MUST distinguir un índice vacío y sin refresco en curso ("No docs are indexed…") de uno que no tiene ningún pasaje que coincida;
- un 403 del daemon (raíz sólo registrada) MUST devolverse como error de la herramienta con el texto del daemon tal cual, sin el prefijo `daemon returned 403:` (`server.rs:945`);
- un 404 sin cuerpo de error de un daemon que no conoce la ruta MUST explicarse como un daemon más viejo que el puente, que hay que reiniciar con `clawg stop`.

#### Escenario: ruta ajena
- GIVEN un puente `dev` en `<tmp>/repo` y otra carpeta `<tmp>/other` con un `README.md`
- WHEN `search_project` recibe `project_path: <tmp>/other`
- THEN es un error que empieza con el texto del daemon y contiene ``clawg projects add "<tmp>/other" --profile dev``, y el índice no tiene filas de `<tmp>/other`

#### Escenario: la raíz del puente
- GIVEN un puente en `<tmp>/repo` con un `README.md`
- WHEN `search_project` recibe la consulta sin `project_path`
- THEN responde con el pasaje del `README.md` bajo su breadcrumb

Las instrucciones del servidor (la apertura de `crates/clawg-cli/src/mcp/protocol.md`, que `opening` arma en `server.rs:122` y entrega en `server.rs:1231`) MUST nombrar `search_project` para preguntas sobre cómo funciona el proyecto (su diseño, convenciones, comandos o dónde está implementado algo), antes de abrir archivos. La descripción de la herramienta (`server.rs:1189-1196`) y las instrucciones MUST decir que busca en los docs y en el código del proyecto. `recall_context` no consulta este índice.

**Tests:** `search_project_lists_each_hit_under_its_breadcrumb`, `search_project_does_not_read_a_root_the_user_did_not_add` e `instructions_say_when_to_search_the_project` en `crates/clawg-cli/src/mcp/server.rs`; `docs_are_numbered_under_their_breadcrumbs`, `search_project_warns_while_indexing`, `an_empty_index_and_no_match_read_differently` y `a_failed_refresh_is_named_before_the_results` en `crates/clawg-cli/src/mcp/tools.rs`. El conteo de herramientas (once) está en `every_tool_is_registered_under_its_wire_name` (`server.rs`) y en los tests de `handshake.rs` ([`specs/mcp`](../mcp/spec.md)).

## Indexar no cambia las memorias

Con un proyecto indexado, `search_all_memories`, `POST /v1/search`, `POST /v1/context`, `search_memory` y `recall_context` MUST devolver exactamente lo mismo que sin índice: ninguna de esas rutas lee las tablas del índice. El detalle de lo que no cambia está en [`specs/retrieval`](../retrieval/spec.md) («Indexar un proyecto no cambia la búsqueda de memorias»).

**Tests:** `indexing_docs_leaves_memory_search_unchanged` y `refreshing_never_touches_memories` en `project_docs/mod.rs`.

## Datos de la versión anterior

Una base migrada hasta V11, con memorias, conclusiones y proyectos, MUST abrir con V12 sin cambiar ninguna fila, y las cuatro tablas del índice MUST existir vacías.

Una base migrada hasta V12, con memorias, proyectos y un índice de docs, MUST abrir con V13 (`crates/clawg-core/migrations/V13__project_index_docs.sql`) sin cambiar ninguna fila de memorias, de `project` ni del índice. Cada fila de `project` MUST quedar con `index_docs = 1`: una raíz registrada antes se registró trabajando en ella, así que sigue indexada. Una raíz que se registra después sin pedir indexar entra con 0, porque el registro siempre nombra la columna ([`specs/project`](../project/spec.md), R4).

La apertura sigue la regla de [`specs/storage`](../storage/spec.md) (snapshot `pre-migration` antes de aplicar la migración pendiente).

#### Escenario
- GIVEN una base en V12 con dos memorias, las raíces `/work/repo` y `/work/web` en `project` y un pasaje indexado de `/work/repo`
- WHEN se abre con la versión actual
- THEN las filas son idénticas y las dos raíces están aceptadas

**Tests:** `a_database_from_the_previous_version_opens_with_an_empty_doc_index` y `a_database_from_the_previous_version_keeps_its_projects_indexed` en `crates/clawg-core/src/memory/db.rs`, y las filas de V12 y V13 en `published_migration_checksums_are_pinned` (`crates/clawg-core/src/memory/migrations.rs`).

## Invariantes

- El daemon no lee el disco de una raíz que el usuario no aceptó, ni para indexar ni para buscar.
- El índice se deriva del disco y se puede borrar y rehacer sin perder nada del usuario.
- Un doc se reemplaza entero o no se toca: ninguna búsqueda ve pasajes de dos versiones del mismo doc.
- El refresco no corre dos veces a la vez para un proyecto, y ni el registro ni la búsqueda lo esperan, salvo la espera del primer índice.

## Límites conocidos

- La calidad de la búsqueda está medida, no exigida. `rag-eval --real --docs` sobre este repo (199 archivos, 10 preguntas de docs y 5 de código, e5-small con pooling mean, Linux, binario debug) dio Hit@3 0,27 y MRR@10 0,26 con docs y código juntos; los números de Windows no están tomados. Las preguntas y el criterio de acierto están en [`specs/eval`](../eval/spec.md).
- El canal léxico une todos los tokens de la consulta con OR, palabras vacías incluidas, y eso empuja pasajes que sólo comparten "de", "el" o "the". Filtrarlo es una mejora posible; no cambia este contrato.
- No hay watcher del filesystem ni herramienta `index_project`: el índice se pone al día al registrar una raíz aceptada y en la próxima búsqueda pasado el intervalo. Un doc editado aparece en la búsqueda siguiente a los 60 s.
- La marca no es un límite contra quien tiene el `token`: cualquier cliente con él puede mandar `index_docs: true` por cualquier raíz, igual que puede registrarla. Lo que cierra es que el modelo, por medio de `project_path`, haga leer al daemon una carpeta que el usuario no eligió.
- `clawg projects add` sólo agrega. No hay `list` ni `remove`, y una raíz aceptada no vuelve a quedar sólo registrada sin editar la base a mano.
