# quality — Especificación

Fuente de verdad de las redes de tests que vigilan lo que el modelo ve del servidor MCP, los docs que enumeran sus herramientas y la convención de no esconder código muerto, y de dónde están documentados esos chequeos. Ninguno cambia el comportamiento de un comando, una ruta o una herramienta. Lo que hace cada herramienta está en [`specs/mcp`](../mcp/spec.md).

**Origen:** [quality-guards](../../changes/archive/2026-10-15-quality-guards/proposal.md), archivado el 2026-10-15. Los ids de requisito son los de esa spec. R5 (borrar el código sin llamador que escondían los `allow`) fue un arreglo de una vez y no queda como requisito: lo que impide que vuelva es R4 y el comando de `clippy` de R6.

## Definiciones

- **Contrato del agente:** lo que el puente le da al cliente para que el modelo decida qué llamar: cada herramienta tal como sale en `tools/list` (nombre, descripción, `inputSchema` y cualquier otro campo que `rmcp` serialice) y el texto de `instructions` (`INSTRUCTIONS`, `crates/clawg-cli/src/mcp/server.rs:109`, que `get_info` entrega en `server.rs:1096`). No incluye `serverInfo`: su nombre depende del perfil y su versión cambia en cada bump.
- **Archivo del contrato:** `crates/clawg-cli/src/mcp/agent_contract.json`, versionado (`CONTRACT_PATH`, `server.rs:1783`).

## R1. El contrato del agente está fijado

Un test MUST armar el contrato desde un puente en proceso (`agent_contract`, `server.rs:1786`) y compararlo con el archivo del contrato. Si difieren, MUST fallar con un mensaje que nombre cada herramienta que cambió (con los campos que cambiaron), que sobra o que falta, y si cambiaron las instrucciones (`contract_diff`, `server.rs:1802`). Si el archivo no existe, MUST fallar diciendo cómo crearlo.

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

**Test:** `the_agent_contract_matches_its_file` (`server.rs:1939`), y los puros `contract_diff_names_the_changed_tool` (`server.rs:1853`) y `contract_diff_ignores_key_order` (`server.rs:1877`).

## R2. Bendecir un cambio deja el diff a la vista

Con `CLAWG_BLESS=1` el test MUST reescribir el archivo del contrato y MUST fallar igual, con un mensaje que diga que se actualizó y que hay que revisar el diff y volver a correr (`server.rs:1942`). Sin la variable, el test MUST NOT escribir.

El archivo MUST escribirse con las claves de cada objeto ordenadas (y por lo tanto las herramientas por nombre), dos espacios de sangría y un salto de línea final (`render_contract`, `server.rs:1846`), de modo que dos bendiciones seguidas del mismo contrato produzcan los mismos bytes.

#### Escenario
- GIVEN un contrato que cambió a propósito
- WHEN se corre `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file`
- THEN el archivo queda con el contrato nuevo y el test falla
- AND la corrida siguiente, sin la variable, pasa

**Test:** `render_contract_is_stable_and_sorted` (puro, `server.rs:1888`). Bendecir se comprueba a mano: un test que escribe el archivo versionado no puede correr en la suite.

## R3. El inventario de `.claude/CLAUDE.md` es el registrado

El conjunto de nombres entre comillas invertidas que siguen al `): ` de la primera línea que empieza con `MCP tools (` en `.claude/CLAUDE.md` (hoy `.claude/CLAUDE.md:58`) MUST ser igual al conjunto de herramientas registradas. Si difieren, el test MUST fallar nombrando las que faltan en el documento y las que el documento nombra sin estar registradas.

Extraer los nombres de esa línea MUST ser una función pura sobre el texto del documento (`tool_names_in_doc`, `server.rs:1903`).

#### Escenario: herramienta nueva sin documentar
- GIVEN `.claude/CLAUDE.md` con las doce herramientas
- WHEN se registra una herramienta nueva sin nombrarla en el documento
- THEN el test falla y nombra la herramienta que falta

#### Escenario: el documento nombra una que no existe
- GIVEN `.claude/CLAUDE.md` sin `restore_memory` y con `forget_all`
- WHEN corre el test
- THEN falla con `missing ["restore_memory"], not registered ["forget_all"]`

**Test:** `the_claude_md_tool_list_is_the_registered_one` (`server.rs:1920`) y el puro `tool_names_in_doc_reads_the_mcp_tools_line` (`server.rs:1910`).

## R4. Ninguna fuente esconde código muerto

Ningún archivo `.rs` bajo `crates/*/src` ni `crates/*/tests`, en cualquier subdirectorio, MUST tener un atributo `allow` o `expect`, interno o externo, también dentro de `cfg_attr`, que nombre `dead_code`, `unused` o un lint `unused_*`. El test MUST fallar con `ruta:línea` de cada hallazgo, la ruta relativa a `crates/`. Otros lints (por ejemplo `clippy::too_many_arguments` o `clippy::unused_io_amount`) MUST NOT contar.

Reconocer el atributo en una línea MUST ser una función pura (`hides_dead_code`, `crates/clawg-cli/tests/conventions.rs:10`). Se examina línea por línea: un atributo partido en varias líneas no se reconoce.

#### Escenario: un `allow` en una fuente
- GIVEN una fuente con `#[allow(dead_code)]` delante de una función
- WHEN corre el test
- THEN falla con la ruta y la línea del atributo

#### Escenario: un comentario o un string
- GIVEN una línea de comentario o un literal que contiene `#[allow(dead_code)]`
- WHEN se la examina
- THEN no cuenta

**Test:** `no_source_allows_dead_code` (`conventions.rs:62`) y el puro `hides_dead_code_recognises_the_attribute_forms` (`conventions.rs:37`).

## R6. Los docs dicen dónde están las convenciones y cómo se chequean

- `AGENTS.md` MUST mandar a `.claude/CLAUDE.md` y MUST NOT repetir su contenido; `README.md` MUST mandar a `.claude/CLAUDE.md` para las convenciones (`README.md:412`).
- `.claude/CLAUDE.md`, sección Testing, MUST dar el comando de `clippy` que hace fallar los warnings del compilador, `dead_code` y `unused` incluidos (`.claude/CLAUDE.md:99`), y cómo bendecir el contrato (`.claude/CLAUDE.md:107`). Ese comando MUST terminar con código 0:

  ```
  cargo clippy --workspace --all-targets -- -A clippy::all -D warnings
  ```

  Los lints propios de `clippy` quedan afuera con `-A clippy::all` hasta que se arreglen los que hay; están en el backlog de [`openspec/README.md`](../../README.md#backlog-sin-spec-arreglos-sueltos).
- `README.md` MUST NOT dar un número de versión como ejemplo de la salida de `clawg --version`.

**Test:** R3 cubre la lista de herramientas. El comando de `clippy` y el resto se comprueban a mano; terminó con código 0 al archivar, en Linux y en Windows.
