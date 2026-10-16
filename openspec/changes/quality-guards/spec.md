# quality-guards — Especificación

Agrega redes de tests sobre lo que el modelo ve del servidor MCP, sobre los docs que lo enumeran y sobre la convención de código muerto. No cambia el comportamiento de ningún comando, ruta ni herramienta.

## Definiciones

- **Contrato del agente:** lo que el puente le da al cliente para que el modelo decida qué llamar: cada herramienta tal como sale en `tools/list` (nombre, descripción, `inputSchema` y cualquier otro campo que `rmcp` serialice) y el texto de `instructions` de `get_info` (`crates/clawg-cli/src/mcp/server.rs:1013`). No incluye `serverInfo`: su nombre depende del perfil y su versión cambia en cada bump.
- **Archivo del contrato:** `crates/clawg-cli/src/mcp/agent_contract.json`, versionado.

## R1. El contrato del agente está fijado

Un test MUST armar el contrato desde un puente en proceso y compararlo con el archivo del contrato. Si difieren, MUST fallar con un mensaje que nombre cada herramienta que cambió, que sobra o que falta, y si cambiaron las instrucciones.

La comparación MUST ser semántica: el orden de las claves de un objeto JSON y el orden en que el router registra las herramientas MUST NOT cambiar el resultado.

#### Escenario: cambia una descripción
- GIVEN el archivo del contrato al día
- WHEN cambia una palabra de la descripción de `delete_memory`
- THEN el test falla y el mensaje nombra `delete_memory`

#### Escenario: cambia un schema
- GIVEN el archivo del contrato al día
- WHEN se quita un campo opcional de `SearchMemoryArgs`
- THEN el test falla y el mensaje nombra `search_memory`

#### Escenario: sube la versión
- GIVEN el archivo del contrato al día
- WHEN cambia `[workspace.package] version`
- THEN el test pasa

**Test:** `the_agent_contract_matches_its_file` (`server.rs`), y los puros `contract_diff_names_the_changed_tool` y `contract_diff_ignores_key_order`.

## R2. Bendecir un cambio deja el diff a la vista

Con `CLAWG_BLESS=1` el test MUST reescribir el archivo del contrato y MUST fallar igual, con un mensaje que diga que se actualizó y que hay que revisar el diff y volver a correr. Sin la variable, el test MUST NOT escribir.

El archivo MUST escribirse con las claves ordenadas, las herramientas ordenadas por nombre, dos espacios de sangría y un salto de línea final, de modo que dos bendiciones seguidas del mismo contrato produzcan los mismos bytes.

#### Escenario
- GIVEN un contrato que cambió a propósito
- WHEN se corre el test con `CLAWG_BLESS=1`
- THEN el archivo queda con el contrato nuevo y el test falla
- AND la corrida siguiente, sin la variable, pasa

**Test:** `render_contract_is_stable_and_sorted` (puro, `server.rs`); el paso de bendecir se comprueba a mano en `verify.md`, porque un test que escribe el archivo versionado no puede correr en la suite.

## R3. El inventario de `.claude/CLAUDE.md` es el registrado

El conjunto de nombres entre comillas invertidas que siguen al `:` de la línea que empieza con `MCP tools (` en `.claude/CLAUDE.md` MUST ser igual al conjunto de herramientas registradas. Si difieren, el test MUST fallar nombrando las que sobran y las que faltan.

Extraer los nombres de esa línea MUST ser una función pura sobre el texto del documento.

#### Escenario
- GIVEN `.claude/CLAUDE.md` con las doce herramientas
- WHEN se registra una herramienta nueva sin nombrarla en el documento
- THEN el test falla y nombra la herramienta que falta

**Test:** `the_claude_md_tool_list_is_the_registered_one` (`server.rs`) y el puro `tool_names_in_doc_reads_the_mcp_tools_line`.

## R4. Ninguna fuente esconde código muerto

Ningún archivo `.rs` bajo `crates/*/src` ni `crates/*/tests` MUST tener un atributo `allow` o `expect`, interno o externo, también dentro de `cfg_attr`, que nombre `dead_code`, `unused` o un lint `unused_*`. El test MUST fallar con `ruta:línea` de cada hallazgo. Otros lints (por ejemplo `clippy::too_many_arguments`) MUST NOT contar.

Reconocer el atributo en una línea MUST ser una función pura.

#### Escenario: hoy
- GIVEN las fuentes de `main`
- WHEN corre el test
- THEN falla con `clock.rs:99`, `chunker/mod.rs:97` y `golden.rs:167`

#### Escenario: un comentario o un string
- GIVEN una línea de comentario o un literal que contiene `#[allow(dead_code)]`
- WHEN se la examina
- THEN no cuenta

**Test:** `no_source_allows_dead_code` (`crates/clawg-cli/tests/conventions.rs`) y el puro `hides_dead_code_recognises_the_attribute_forms`.

## R5. El código sin llamador se va

- `TimestampMs::age_days_from` (`crates/clawg-core/src/memory/clock.rs:103`) MUST borrarse con sus tests, y el comentario que lo nombra en `crates/clawg-core/src/memory/score.rs:247` MUST dejar de nombrarlo.
- `RefMap::id_of` (`crates/clawg-eval/src/bin/rag-eval/golden.rs:168`) MUST compilarse sólo en tests.
- El campo `Chunk::token_count` (`crates/clawg-core/src/memory/chunker/mod.rs:98`) MUST borrarse; los tests que lo leían MUST seguir fijando el texto exacto de cada pedazo.
- `insert_row_with_embedding` (`crates/clawg-core/src/memory/worker.rs:1280`) MUST borrarse.

`cargo test --workspace` MUST compilar sin warnings.

**Test:** R4 (deja de fallar) y la salida de `cargo test --workspace` sin líneas `warning:` del workspace, comprobada en `verify.md`.

## R6. Los docs dicen lo que hay

- `AGENTS.md` MUST mandar a `.claude/CLAUDE.md` y MUST NOT repetir su contenido; `README.md` MUST mandar a `.claude/CLAUDE.md` para las convenciones.
- `.claude/CLAUDE.md`, sección Testing, MUST dar el comando de `clippy` que pasa limpio y cómo bendecir el contrato.
- El comentario de `AUTO_KEEP` MUST nombrar los tres snapshots automáticos.
- `openspec/specs/storage/spec.md` MUST citar `AUTO_KEEP` y `to_prune` en sus líneas de hoy.
- `README.md` MUST NOT dar un número de versión como ejemplo de la salida de `clawg --version`.

**Test:** R3 cubre la lista de herramientas; el resto es revisión del diff.
