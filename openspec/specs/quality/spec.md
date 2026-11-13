# quality — Especificación

Fuente de verdad de las redes de tests que vigilan lo que el modelo ve del servidor MCP, la convención de no esconder código muerto y la de no dejar un store abierto en un test de integración, de cómo se organiza por dentro el despacho de los comandos de `clawg`, de en qué carpetas vive el código de `clawg_core::memory` y qué carpeta puede nombrar a cuál, y de dónde están documentados esos chequeos. Ninguno cambia el comportamiento de un comando, una ruta o una herramienta. Lo que hace cada herramienta está en [`specs/mcp`](../mcp/spec.md), y lo que dice cada comando, en la spec de su dominio.

**Origen:**
- [quality-guards](../../changes/archive/2026-10-15-quality-guards/proposal.md), archivado el 2026-10-15. Los ids de requisito son los de esa spec. R5 (borrar el código sin llamador que escondían los `allow`) fue un arreglo de una vez y no queda como requisito: lo que impide que vuelva es R4 y el comando de `clippy` de R6. R3 (que la línea `MCP tools` de `.claude/CLAUDE.md` nombrara las herramientas registradas) se quitó: ningún test lee un archivo de instrucciones para agentes (R6).
- [agent-language](../../changes/archive/2026-10-23-agent-language/proposal.md), archivado el 2026-10-23 (sólo el contrato de una sesión en español en R1 y R2; qué lee esa sesión está en [`specs/mcp`](../mcp/spec.md), AL2 a AL4).
- [clippy-enforced](../../changes/archive/2026-11-03-clippy-enforced/proposal.md), archivado el 2026-11-03, cuyo código salió en 0.73.0 (CE1, CE3 y CE4 en R6: qué lints de clippy rigen, cómo se anota una excepción y qué dicen los docs; CE2 está en [`specs/install`](../install/spec.md), «CI»).
- [cli-dispatch](../../changes/archive/2026-11-04-cli-dispatch/proposal.md), archivado el 2026-11-04, sale en 0.75.0 (CD1 a CD4). No cambió ningún comando: el binario de antes y el de después dieron la misma salida, el mismo stderr y el mismo código en 244 casos, y `crates/clawg-cli/tests/binary.rs` pasó sin editarse ([verify](../../changes/archive/2026-11-04-cli-dispatch/verify.md#el-binario-de-antes-y-el-de-después)).
- [memory-module-layout](../../changes/archive/2026-11-13-memory-module-layout/proposal.md), archivado el 2026-11-13, sale en 0.87.0 (ML1 a ML5, en «Las carpetas de `memory`»). Sus R1 a R5 son ML1 a ML5 acá, para no chocar con los ids de esta spec. R6, que ninguna firma, tipo ni respuesta cambia, es de conservación y no se consolida. R7, que las citas de las specs, de `docs/reference` y del golden de docs nombren archivos que existen, fue un barrido de una vez ([verify](../../changes/archive/2026-11-13-memory-module-layout/verify.md#las-citas)). El motivo del reparto y de cada grupo en círculo está en su [diseño](../../changes/archive/2026-11-13-memory-module-layout/design.md#d1-el-reparto).

## Definiciones

- **Contrato del agente:** lo que el puente le da al cliente para que el modelo decida qué llamar: cada herramienta tal como sale en `tools/list` (nombre, descripción, `inputSchema` y cualquier otro campo que `rmcp` serialice) y el texto de `instructions` (la apertura del protocolo del idioma de la sesión, `opening`, `crates/clawg-cli/src/mcp/protocol.rs:33`, que `get_info` entrega en `server.rs:1534`; el protocolo está en [`specs/mcp`](../mcp/spec.md)). No incluye `serverInfo`: su nombre depende del perfil y su versión cambia en cada bump.
- **Archivo del contrato:** `crates/clawg-cli/src/mcp/agent_contract.json`, versionado (`CONTRACT_PATH`, `server.rs:2563`), con el contrato del puente sin preferencia; `agent_contract.es.json` (`CONTRACT_ES_PATH`, `server.rs:2737`), con el del puente en español (`with_language(Some(Lang::Es))`). Con la preferencia `en` sólo cambian dos frases del protocolo, que fijan los tests de [`specs/mcp`](../mcp/spec.md) (LP9), y no hay un tercer archivo.
- **Lo común:** lo que un comando necesita antes de hacer lo suyo y que comparte con otros: su perfil, su directorio de datos, su idioma y, si lo necesita, el daemon corriendo.
- **Función del comando:** la función de su módulo en `clawg_cli` a la que `dispatch` le pasa lo común y los argumentos del comando, y que devuelve lo que se imprime o el error.
- **Frase compuesta:** un texto que clawg arma para la persona, en el idioma del comando ([`specs/preferences`](../preferences/spec.md)): la que enmarca un fallo, una negativa o un informe.
- **`memory`:** `crates/clawg-core/src/memory/`.
- **Carpeta:** un directorio de `memory` que es una fila de la tabla `MEMORY_FOLDERS` (`crates/clawg-cli/tests/conventions.rs:309`).
- **Módulo:** una entrada de una carpeta: un `.rs`, o un directorio con sus fuentes. `project_docs` y `portability` son un módulo cada una (`ONE_MODULE_FOLDERS`, `conventions.rs:323`).
- **Código de producción** de un fuente: sus líneas antes del primer `#[cfg(test)]` seguido de `mod <nombre> {`, sin las líneas de comentario (`production_code`, `conventions.rs:449`). Un fuente que su `mod.rs` declara bajo `#[cfg(test)]` no tiene (`declared_for_tests_only`).
- **Dependencia:** un módulo depende de otro cuando su código de producción lo nombra con una ruta que empieza en `crate::memory::` o en un `super::` (`memory_dependencies`, `conventions.rs:594`). Un grupo `{…}` da una ruta por elemento, un nombre que la raíz reexporta se sigue hasta su módulo, y una ruta al propio módulo no cuenta.
- **Otro crate:** `clawg-cli`, `clawg-eval` o un test de integración de `crates/*/tests/`.

## R1. El contrato del agente está fijado

Un test MUST armar el contrato desde un puente en proceso (`agent_contract`, `server.rs:2566`) y compararlo con el archivo del contrato, y otro MUST hacer lo mismo con el puente en español y `agent_contract.es.json`. Si difieren, MUST fallar con un mensaje que nombre cada herramienta que cambió (con los campos que cambiaron), que sobra o que falta, y si cambiaron las instrucciones (`contract_diff`, `server.rs:2582`). Si el archivo no existe, MUST fallar diciendo cómo crearlo.

La comparación MUST ser semántica: el orden de las claves de un objeto JSON y el orden en que el router registra las herramientas MUST NOT cambiar el resultado.

#### Escenario: cambia una descripción
- GIVEN el archivo del contrato al día
- WHEN cambia una palabra de la descripción de `delete_memory`
- THEN el test falla con `delete_memory changed: description`

#### Escenario: cambia un schema
- GIVEN el archivo del contrato al día
- WHEN se quita un campo opcional de `SearchMemoryArgs`
- THEN el test falla y el mensaje nombra `search_memory`

#### Escenario: sube la versión
- GIVEN el archivo del contrato al día
- WHEN cambia `[workspace.package] version`
- THEN el test pasa

**Test:** `the_agent_contract_matches_its_file` (`server.rs:2719`) y `the_spanish_agent_contract_matches_its_file` (`server.rs:2742`), y los puros `contract_diff_names_the_changed_tool` (`server.rs:2633`) y `contract_diff_ignores_key_order` (`server.rs:2657`).

## R2. Bendecir un cambio deja el diff a la vista

Con `CLAWG_BLESS=1` el test MUST reescribir el archivo del contrato y MUST fallar igual, con un mensaje que diga que se actualizó y que hay que revisar el diff y volver a correr (`server.rs:2722-2724`, y `:2745-2747` para el español, que se bendice con `CLAWG_BLESS=1 cargo test -p clawg-cli the_spanish_agent_contract_matches_its_file`). Sin la variable, el test MUST NOT escribir.

El archivo MUST escribirse con las claves de cada objeto ordenadas (y por lo tanto las herramientas por nombre), dos espacios de sangría y un salto de línea final (`render_contract`, `server.rs:2626`), de modo que dos bendiciones seguidas del mismo contrato produzcan los mismos bytes.

#### Escenario
- GIVEN un contrato que cambió a propósito
- WHEN se corre `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file`
- THEN el archivo queda con el contrato nuevo y el test falla
- AND la corrida siguiente, sin la variable, pasa

**Test:** `render_contract_is_stable_and_sorted` (puro, `server.rs:2668`). Bendecir se comprueba a mano: un test que escribe el archivo versionado no puede correr en la suite.

## R4. Ninguna fuente esconde código muerto

Ningún archivo `.rs` bajo `crates/*/src` ni `crates/*/tests`, en cualquier subdirectorio, MUST tener un atributo `allow` o `expect`, interno o externo, también dentro de `cfg_attr`, que nombre `dead_code`, `unused` o un lint `unused_*`. El test MUST fallar con `ruta:línea` de cada hallazgo, la ruta relativa a `crates/`. Otros lints (por ejemplo `clippy::too_many_arguments` o `clippy::unused_io_amount`) MUST NOT contar.

Reconocer el atributo en una línea MUST ser una función pura (`hides_dead_code`, `crates/clawg-cli/tests/conventions.rs:12`). Se examina línea por línea: un atributo partido en varias líneas no se reconoce.

#### Escenario: un `allow` en una fuente
- GIVEN una fuente con `#[allow(dead_code)]` delante de una función
- WHEN corre el test
- THEN falla con la ruta y la línea del atributo

#### Escenario: un comentario o un string
- GIVEN una línea de comentario o un literal que contiene `#[allow(dead_code)]`
- WHEN se la examina
- THEN no cuenta

**Test:** `no_source_allows_dead_code` (`conventions.rs:64`) y el puro `hides_dead_code_recognises_the_attribute_forms` (`conventions.rs:39`).

## R6. Los docs dicen dónde están las convenciones y cómo se chequean

- `AGENTS.md` MUST mandar a `.claude/CLAUDE.md` y MUST NOT repetir su contenido; `README.md` MUST mandar a `.claude/CLAUDE.md` para las convenciones (`README.md:412`).
- `.claude/CLAUDE.md` MUST guardar sólo las reglas y dónde está el resto; el comportamiento (perfiles, comandos, API, tests, CI, release) MUST estar en `docs/reference/` o en las specs, no en él.
- Ningún test MUST leer un archivo de instrucciones para agentes (`.claude/`, `.cursor/`, `.gemini/`, `AGENTS.md`): son texto para el modelo y cambian sin que cambie el comportamiento. Los tests de `clawg setup` y `doctor` que escriben o leen el `CLAUDE.md` de un usuario en un directorio temporal prueban el producto y no cuentan.
- `.claude/CLAUDE.md`, sección Testing, MUST dar el comando de `clippy` que hace fallar los warnings del compilador (`dead_code` y `unused` incluidos) y los lints de clippy, y cómo bendecir el contrato. Ese comando MUST terminar con código 0:

  ```
  cargo clippy --workspace --all-targets -- -D warnings
  ```

  Qué lints de clippy corren lo dice `[workspace.lints.clippy]` del `Cargo.toml` raíz, que cada paquete toma con `[lints] workspace = true`: el grupo por defecto (`all` en `warn` con `priority = -1`, para que un lint suelto pese más que el grupo), `large_futures`, `allow_attributes` y `allow_attributes_without_reason`. El comando MUST NOT apagar ninguno. Un hallazgo que no se arregla MUST anotarse en el ítem con `#[expect(clippy::<lint>, reason = "<motivo>")]`, nunca con `#[allow]` ni en la tabla; cuando el lint deja de emitirse, la expectativa no cumplida hace fallar el comando hasta que se quita. Sólo se anota lo que pide un refactor con su propio cambio (`too_many_arguments`) o lo que el lint cuida y en ese lugar no pasa, y el motivo MUST decir cuál de las dos y por qué. `every_package_takes_the_workspace_lints` y `no_workflow_turns_clippy_lints_off` (`crates/clawg-cli/tests/conventions.rs`) fijan la tabla y los comandos del CI ([`specs/install`](../install/spec.md), «CI»).
- `docs/reference/testing.md` («Local checks» y «CI and release workflows») MUST dar ese mismo comando, y MUST decir que `[workspace.lints.clippy]` es la que dice qué lints corren y que una excepción es un `#[expect]` con motivo.
- `README.md` MUST NOT dar un número de versión como ejemplo de la salida de `clawg --version`.

#### Escenario: un paquete sin la tabla
- GIVEN un miembro del workspace cuyo `Cargo.toml` no tiene `[lints]` con `workspace = true`
- WHEN corre la suite
- THEN el test falla y nombra el paquete

#### Escenario: una excepción del workspace
- GIVEN `[workspace.lints.clippy]` con un lint en `allow`
- WHEN corre la suite
- THEN el test falla y nombra el lint

#### Escenario: un `allow` de un lint de clippy
- GIVEN `#[allow(clippy::too_many_arguments)]` sobre una función
- WHEN corre clippy
- THEN falla con `clippy::allow_attributes`

#### Escenario: una excepción que dejó de hacer falta
- GIVEN una función con `#[expect(clippy::too_many_arguments, reason = "…")]`
- WHEN un cambio la deja con siete parámetros o menos
- THEN clippy falla con `unfulfilled_lint_expectations` hasta que se quita el atributo

**Test:** ninguno que lea `.claude/CLAUDE.md`, por la regla de arriba: el comando de `clippy` y el resto se comprueban a mano, y terminó con código 0 al archivar, en Linux y en Windows. La tabla y los comandos del CI los fijan los dos tests de `conventions.rs` nombrados arriba.

## R7. Un test de integración cierra el store que abre en disco

Un test bajo `crates/*/tests` que abre un runtime con `MemoryRuntime::open` MUST llamar a `MemoryRuntime::close` antes de terminar. Soltar un runtime sólo le avisa al hilo de su conexión, y en Windows un archivo abierto no se puede borrar: `tempfile` se rinde sin error, el store queda en el temporal del sistema y ningún test falla. Un test de integración no alcanza `crate::test_dir`, que en `src/` detecta este caso. El test de esta convención MUST fallar con la ruta, relativa a `crates/`, y el nombre de cada test que no cierra.

Reconocerlo MUST ser una función pura (`tests_leaving_a_store_open`, `crates/clawg-cli/tests/conventions.rs:1668`). Lee el texto de cada test: un `.close().await` alcanza, así que un test con dos runtimes puede dejar uno abierto.

#### Escenario: un test que abre en disco y no cierra
- GIVEN un test con `MemoryRuntime::open` y sin `.close().await`
- WHEN corre la convención
- THEN falla y nombra el archivo y el test

#### Escenario: un test en memoria
- GIVEN un test con `MemoryRuntime::new_for_test`, que no toca el disco
- WHEN corre la convención
- THEN no cuenta

**Test:** `an_integration_test_closes_the_store_it_opens_on_disk` (`conventions.rs:1707`) y el puro `tests_leaving_a_store_open_names_each_test_without_a_close` (`conventions.rs:1679`).

## CD1. Un error llega a `main` con su tipo y su texto

`dispatch` (`crates/clawg-cli/src/main.rs`) MUST devolver `Result<ExitCode, CliError>`, y `CliError` (`crates/clawg-cli/src/error.rs`) MUST derivar `thiserror::Error`. Para un error tipado, el `Display` de `CliError` MUST ser exactamente el de la fuente. Una frase compuesta MUST mostrarse tal como se compuso, y MUST entrar en `CliError` de forma explícita, con `CliError::Failed`: `CliError` MUST NOT implementar `From<String>`. `main` MUST imprimir `clawg: ` y el error en stderr y salir con 1.

#### Escenario: un perfil inválido
- GIVEN `CLAWG_PROFILE=bogus`
- WHEN se corre cualquier comando con perfil
- THEN stderr dice `clawg: ` seguido del texto que da `ProfileArg::resolve`, y el código es 1

#### Escenario: una frase en español
- GIVEN una negativa compuesta en español
- WHEN llega a `main`
- THEN se imprime tal cual, sin pasar por ninguna otra traducción

**Test:** `a_typed_error_reads_as_its_source` y `a_composed_sentence_reads_as_it_was_written` (`crates/clawg-cli/src/error.rs`).

## CD2. La frase que enmarca un fallo la compone el módulo del comando

Cuando un comando dice un fallo con palabras propias, la frase MUST componerla una función pura de su módulo que recibe el idioma del comando, y MUST NOT componerla `main` ni `dispatch`. La causa técnica MUST ir tal cual dentro de la frase, como pide [`specs/preferences`](../preferences/spec.md). Los comandos que necesitan el daemon y no lo arrancan (`import`, `undelete`, `outbox`, `projects` y `memories`) MUST decir que no lo encontraron con `memories::explain_unavailable`, a través de un solo lugar, `Running::find` (`crates/clawg-cli/src/running.rs`).

Tres fallos se dicen sólo en inglés y no reciben idioma, aunque también los compone su módulo: `name the file to import, or --undo N` (`import.rs`), `name a memory to purge, or a delete with --project-delete` (`secrets.rs`) y los de `clawg path` (`user_path.rs`). Traducirlos es un cambio de conducta, con su propia spec.

#### Escenario: `stop` sin daemon
- GIVEN un directorio de datos sin daemon
- WHEN se corre `clawg stop` con el idioma `en`, y después con `es`
- THEN dice `no clawg daemon is running for profile dev`, y después `no hay ningún daemon de clawg corriendo para el perfil dev`, con código 1

#### Escenario: un comando que necesita el daemon
- GIVEN un directorio de datos sin daemon
- WHEN un comando busca el daemon con el idioma `es`
- THEN el error dice `no hay ningún daemon de clawg corriendo para el perfil dev; arráncalo con 'clawg serve --profile dev'`

**Test:** `a_missing_daemon_is_named_in_both_languages` y `the_stop_line_speaks_both_languages` (`crates/clawg-cli/src/stop.rs`); `a_missing_daemon_says_how_to_start_it_in_both_languages` y `a_published_daemon_is_found` (`crates/clawg-cli/src/running.rs`).

## CD3. `status` se prueba sin `main`

La función de `clawg status` (`status::command`) MUST recibir el perfil y el directorio de datos y devolver el JSON que se imprime y el código de salida: 0 si `/health` contesta, 1 si no. Con un daemon, el informe MUST llevar su `health` y MUST NOT sondear el store; sin daemon, MUST llevar lo que dice el store.

#### Escenario: sin daemon
- GIVEN un directorio de datos vacío
- WHEN se llama la función de `status`
- THEN el JSON dice `"running": false` y `"store": {"state": "missing", …}`, y el código es 1

#### Escenario: con un daemon
- GIVEN un daemon en proceso que publicó su estado en el directorio
- WHEN se llama la función de `status`
- THEN el JSON dice `"running": true`, lleva el `health` del daemon y `"store": null`, y el código es 0

**Test:** `status_without_a_daemon_reports_the_store_and_exits_1` y `status_with_a_daemon_reports_its_health_and_exits_0` (`crates/clawg-cli/src/status.rs`).

## CD4. Cada comando con lógica propia se prueba sin `main`

Un comando que hace algo además de resolver lo común, llamar una función y mostrar lo que devuelve (elige entre caminos, traduce un error, saca el código de salida del resultado o da un segundo paso) MUST tener una función del comando en su módulo, y lo que decide MUST estar probado ahí:

| Comando | Lo que decide | Test (en `crates/clawg-cli/src/`) |
|---|---|---|
| `backup` | el snapshot lo saca el daemon si corre; sin daemon, la copia sin lock, y su error tal cual | `a_running_daemon_takes_the_snapshot`, `without_a_daemon_the_missing_store_is_said_as_it_came` (`backup.rs`) |
| `restore` | un store que tiene el daemon se niega diciendo cómo liberarlo, en el idioma del comando | `a_store_the_daemon_holds_is_refused_in_the_commands_language` (`restore.rs`) |
| `export` | `--project` se resuelve contra el store antes de escribir nada | `a_project_without_a_store_is_refused_before_anything_is_written` (`export.rs`) |
| `import` | importar un archivo o deshacer una importación, después de encontrar el daemon | `neither_a_file_nor_an_undo_is_refused_once_the_daemon_answers` (`import.rs`) |
| `secrets`, `purge` | sin `--yes` se niega antes de abrir el store; `purge` pide una memoria o un borrado | `a_purge_without_yes_is_refused_before_the_store_is_opened`, `a_purge_names_a_memory_or_a_delete`, `a_scan_without_a_store_fails_as_the_store_says` (`secrets.rs`) |
| `metrics report` | el informe de una métrica o los eventos | `the_flags_choose_the_report_or_the_events` (`metrics.rs`) |
| `projects` | la acción, su negativa y su código de salida | `each_projects_action_names_its_profile` (`cli.rs`), `a_listing_and_a_refused_rename_go_through_the_running_daemon` (`projects.rs`) |
| `path` | fuera de Windows, la negativa | `outside_windows_the_path_command_says_what_to_do_instead` (`user_path.rs`) |

`export` MUST leer el idioma después de resolver el proyecto, y `secrets` sin `--mask-reasons`, después de leer el store: `language_in` escribe en stderr una línea por un `preferences.json` inválido, y leerlo antes de un paso que puede fallar sumaría esa línea donde no sale.

**Test:** los de la tabla.

## Las carpetas de `memory`

El árbol de `memory` dice qué hace el sistema, y dos reglas de `crates/clawg-cli/tests/conventions.rs` fijan hacia dónde puede depender cada parte. Leen los fuentes como texto: no compilan nada ni agregan una dependencia. Las dos cuentan una dependencia de la misma forma, la de «Definiciones».

### ML1. Cada módulo vive en una carpeta

`memory` MUST contener sólo su `mod.rs` y las diez carpetas de la tabla de ML2. Un fuente o un directorio suelto junto a ellas MUST hacer fallar la regla, que lo nombra, y también una carpeta de la tabla que no existe.

| Carpeta | Módulos |
|---|---|
| `domain` | `clock`, `events`, `provenance`, `record`, `redact`, `sharing`, `vector` |
| `store` | `backup`, `conn_probe`, `db`, `dedup`, `error`, `latency`, `migrations`, `store_lock`, `tier` |
| `embedding` | `chunker/`, `embedder`, `idle_embedder`, `model_download` |
| `retrieval` | `access`, `language`, `listing`, `pieces`, `prompt_builder`, `query_context`, `score`, `search`, `search_log`, `workspace`, y `crowding`, que sólo se compila para los tests |
| `revision` | `contradiction`, `dispute`, `revival`, `topic` |
| `project_docs` | la carpeta es el módulo |
| `projects` | `identity`, `inspect`, `merge`, `project`, `project_delete`, `registry`, `relocate`, `subfolder` |
| `ingest` | `outbox`, `save`, `spill`, `store_insert`, `worker` |
| `upkeep` | `admin`, `bootstrap`, `maintenance`, `secrets`, `synthesis` |
| `portability` | la carpeta es el módulo |

La regla fija las carpetas, no la lista de módulos de cada una: un módulo nuevo entra en la carpeta de lo que hace, y lo que comprueba la regla es que su código nombre sólo lo que la fila de esa carpeta permite.

#### Escenario: un fuente suelto
- GIVEN un fuente `extra.rs` suelto en `memory`, junto a las carpetas
- WHEN corre la regla
- THEN falla y nombra `extra`

**Test:** `a_memory_folder_names_only_the_folders_its_row_allows` (`conventions.rs:627`).

### ML2. Una carpeta nombra sólo las que su fila permite

El código de producción de una carpeta MUST depender sólo de módulos de su carpeta y de las carpetas de su fila (`MEMORY_FOLDERS`, `conventions.rs:309`):

| Carpeta | Puede nombrar |
|---|---|
| `domain` | ninguna |
| `store` | `domain` |
| `embedding` | `domain`, `store` |
| `retrieval` | `domain`, `store`, `embedding` |
| `revision` | `domain`, `store`, `retrieval` |
| `project_docs` | `domain`, `store`, `embedding`, `retrieval` |
| `projects` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `project_docs` |
| `ingest` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `projects` |
| `upkeep` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `project_docs`, `projects` |
| `portability` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `projects`, `ingest`, `upkeep` |

- Una fila MUST nombrar sólo filas escritas más arriba, así la tabla no tiene ciclos y ningún par de carpetas depende una de la otra.
- Cada par de la tabla MUST usarse: una fila que permite una carpeta que su código no nombra hace fallar la regla. Una fila que permite más de lo que el código hace dejaría pasar una dependencia nueva sin que nadie la decida.
- Una ruta `crate::memory::…` del código de producción MUST llevar a un módulo.
- La regla MUST fallar con `archivo:línea` y el módulo nombrado de cada hallazgo.

Una dependencia nueva entre carpetas se decide editando la fila, en el mismo cambio que la agrega.

#### Escenario: una dependencia hacia arriba
- GIVEN un fuente de `store` cuyo código de producción escribe `crate::memory::retrieval::search::RankedHit`
- WHEN corre la regla
- THEN falla y nombra ese archivo, esa línea y `retrieval/search`

#### Escenario: el código de tests no cuenta
- GIVEN un fuente de `retrieval` que nombra `crate::memory::upkeep::admin` sólo dentro de su `#[cfg(test)] mod tests`
- WHEN corre la regla
- THEN pasa

**Tests:** `a_memory_folder_names_only_the_folders_its_row_allows`, y los puros `leading_paths_opens_groups_under_what_leads_to_them` (`conventions.rs:436`), `memory_paths_reads_absolute_and_relative_paths` (`:494`) y `module_named_follows_a_folder_a_module_and_a_reexport` (`:547`).

### ML3. Dentro del núcleo, la ruta nombra la carpeta

El código de producción de `memory` MUST nombrar un módulo de `memory` por su carpeta: `crate::memory::<carpeta>::<módulo>`. Una ruta que llegue a un módulo por un nombre que la raíz reexporta (`crate::memory::db`, `crate::memory::MemoryStore`) MUST hacer fallar la regla: con dos formas de escribir cada ruta, la que no nombra la carpeta esconde hacia dónde va la dependencia.

La excepción es `project_docs`, listada en `NAMES_THROUGH_THE_ROOT` (`conventions.rs:327`): sus fuentes siguen nombrando por la raíz, y sus dependencias se cuentan igual. Para que esas rutas resuelvan, `memory/mod.rs` conserva dos reexportaciones `pub(crate)` que nada más usa: `retrieval::search` y, bajo `cfg(test)`, `embedding::model_download`. Cuando los fuentes de `project_docs` pasen a la forma con carpeta, salen la excepción y las dos reexportaciones.

El código del núcleo que está fuera de `memory` (`crates/clawg-core/src/runtime.rs`, `eval_api.rs`) alcanza las carpetas, que son `pub(crate) mod`, y la regla no lo lee.

**Test:** `a_memory_folder_names_only_the_folders_its_row_allows`.

### ML4. Los módulos dependen en círculo sólo donde está escrito

Los grupos de más de un módulo en los que cada uno depende de los demás, directa o indirectamente, MUST ser exactamente los de `MEMORY_CYCLES` (`conventions.rs:331`), cada uno con su motivo. Por ML2, cada grupo queda dentro de una sola carpeta:

| Grupo | Motivo |
|---|---|
| `store`: `backup`, `db`, `dedup`, `error`, `latency`, `store_lock`, `tier` | `MemoryError` lleva `BackupError` y `StoreOwner`, y los módulos que los definen, y los que registran funciones de SQL en la conexión, fallan con `MemoryError`. El store guarda la latencia de su búsqueda vectorial, que se mide sobre el store |
| `retrieval`: `listing`, `pieces`, `search`, `search_log` | un hit lleva el lugar de su pieza en el save, un save se lee entero desde la fila de su cabeza, y las líneas de log de una búsqueda cuentan sus hits |
| `retrieval`: `prompt_builder`, `workspace` | `workspace.md` se escribe con la sangría del prompt, y el prompt lo lee |
| `projects`: `inspect`, `merge`, `project_delete`, `relocate`, `subfolder` | un merge, un movimiento y un borrado de proyecto corren los pasos de los otros dentro de una transacción |
| `projects`: `project`, `registry` | registrar una carpeta lee los nombres del registro, y el registro reconoce un id de proyecto por su forma |

La regla MUST fallar tanto por un grupo que no está listado como por uno listado que ya no existe, y MUST imprimir cada grupo que encontró con su tamaño y la cantidad de módulos de `memory`, para que la cuenta se pueda leer de ahí:

```bash
cargo test -p clawg-cli --test conventions the_memory_modules_cycle_only_where_listed
```

#### Escenario: un ciclo nuevo
- GIVEN un fuente de `domain/record` que pasa a nombrar `crate::memory::domain::provenance`, que ya lo nombra a él
- WHEN corre la regla
- THEN falla e imprime los cinco grupos listados y uno más, de 2 módulos, con `domain/provenance` y `domain/record`

**Tests:** `the_memory_modules_cycle_only_where_listed` (`conventions.rs:707`) y el puro `cycles_groups_the_nodes_that_reach_each_other` (`:698`).

### ML5. Los otros crates nombran un módulo sin su carpeta

- Las carpetas MUST ser `pub(crate) mod` en `memory/mod.rs`, salvo `portability` y `project_docs`, que son un módulo cada una y siguen `pub mod`. Otro crate MUST NOT alcanzar una carpeta.
- `memory/mod.rs` MUST reexportar, con una línea `pub use <carpeta>::<módulo>;` cada uno, los módulos de una carpeta que otro crate usa. Así `clawg_core::memory::<módulo>` resuelve sin nombrar la carpeta, y mover un módulo de carpeta no cambia ningún fuente de `clawg-cli` ni de `clawg-eval`.
- La lista de los módulos que la raíz muestra MUST leerse de esas líneas y de los `pub mod` (`public_memory_modules`, `conventions.rs:286`). Qué módulos puede mostrar lo dice SB9 de [`specs/storage`](../storage/spec.md): los que otro crate nombra.

#### Escenario
- GIVEN una raíz con `pub mod portability;`, `pub(crate) mod store;`, `pub use store::db;` y `pub use store::db::MemoryStore;`
- WHEN se leen sus módulos públicos
- THEN son `portability` y `db`

**Tests:** `every_public_memory_module_is_named_outside_core` (`conventions.rs:264`) y el puro `public_memory_modules_reads_declarations_and_reexports` (`:301`); además, que los otros crates compilan.
