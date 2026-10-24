# visibility — Especificación

Fuente de verdad del comportamiento actual de la identidad del agente y de qué filas puede listar, leer entera, borrar, cambiar, restaurar o resolver cada agente. También define la ruta en disco del workspace de cada agente. El `agent_id` de una fila dice quién la escribió, nunca quién puede leerla o cambiarla: cada agente alcanza lo que `sharing.json` le deja leer ([`specs/sharing`](../sharing/spec.md)), y lo que alcanza lo lee, lo cambia, lo borra, lo restaura y lo juzga, sea de quien sea (R3 y R4).

**Origen:**
- [visibility-policy](../../changes/archive/2026-10-12-visibility-policy/proposal.md), partes A y B, archivado el 2026-10-12.
- [agent-sharing](../../changes/archive/2026-10-14-agent-sharing/proposal.md), archivado el 2026-10-14 (sólo lo que dice de lo ajeno compartido en R3 y R4).
- [agent-authorship](../../changes/archive/2026-10-22-agent-authorship/proposal.md), archivado el 2026-10-22: R3 y R4 dejan la frontera por agente y pasan al alcance del lector.
- [project-reassign](../../changes/archive/2026-10-22-project-reassign/proposal.md), archivado el 2026-10-22: el movimiento de un `PATCH` con lector (R4).

## Definiciones

- **Agente del puente:** el `agent_id` con el que el puente MCP lee y escribe en una llamada.
- **Fila del agente A:** fila de `memory`, `episodic_trajectory` o `procedural_skill` con `agent_id = A`; A es su autor.
- **Lector:** el agente que nombra una llamada al daemon (`agent` en la query de `DELETE`/`PATCH`, de `GET /v1/memories/{kind}/{id}/save` y de `POST /v1/memories/memory/{id}/restore`, `agent_id` en el cuerpo de `POST /v1/disputes/judge`, `/v1/disputes/resolve` y `POST /v1/synthesis` y en la query de `GET /v1/disputes`, `reader` en `GET /v1/memories`), con lo que `sharing.json` le deja alcanzar ([`specs/sharing`](../sharing/spec.md), definición «Alcance»). Una llamada sin lector es del usuario.
- **Segmento seguro:** un `agent_id` no vacío, sin `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`, `%` ni caracteres de control, que no termina en `.` ni en espacio (lo que también excluye `.` y `..`) y que no es un nombre de dispositivo de Windows.
- **Nombre de dispositivo de Windows:** un `agent_id` cuya parte antes del primer `.`, sin espacios finales y sin distinguir mayúsculas, es `CON`, `PRN`, `AUX`, `NUL`, o `COM`/`LPT` seguido de un dígito o de `¹`, `²`, `³`.

## R1. Identidad del agente

El agente del puente MUST ser, en este orden:

1. el valor de `clawg mcp --agent <nombre>`, recortado;
2. el nombre que declara el cliente, tal como llega, si no está vacío ni es sólo espacios. De dónde sale ese nombre en cada request (y no sólo en el handshake) lo define [`specs/mcp`](../mcp/spec.md) (su R4);
3. `"mcp"`.

`--agent` con un valor vacío o sólo espacios MUST rechazarse al parsear la línea de comandos.

El `--agent` de `clawg hook` MUST ser el del puente, para que el hook lea al mismo agente que guarda ([`specs/hooks`](../hooks/spec.md), R2).

#### Escenario: el cliente no se identifica
- GIVEN filas guardadas con `agent_id = "claude-code"` y un puente lanzado con `--agent claude-code`
- WHEN el cliente no manda `clientInfo`
- THEN `search_memory` y `list_memories` encuentran esas filas, y un `save_memory` nuevo queda con `agent_id = "claude-code"`

#### Escenario: el flag gana
- GIVEN un puente con `--agent claude-code`
- WHEN el cliente se identifica como `"Claude Code"`
- THEN el agente del puente es `"claude-code"`

#### Escenario: sin flag
- GIVEN un puente sin `--agent`
- WHEN el cliente se identifica como `"cursor"`, o no se identifica
- THEN el agente es `"cursor"`, o `"mcp"`

**Test:** `the_configured_agent_wins_over_the_client`, `without_a_flag_the_client_name_wins_over_the_fallback` y `a_configured_agent_keeps_its_memories_without_client_info` en `crates/clawg-cli/src/mcp/server.rs`; `mcp_takes_an_agent` y `mcp_rejects_a_blank_agent` en `crates/clawg-cli/src/cli.rs`.

## R2. `memory_status` dice quién es el agente

`memory_status` MUST incluir el agente del puente y su origen: `flag`, `client` o `fallback`.

#### Escenario
- GIVEN un puente con `--agent claude-code`
- WHEN se llama `memory_status`
- THEN el texto incluye `agent: claude-code (from --agent)`

**Test:** `memory_status_names_the_agent_and_where_it_came_from` en `server.rs`, `format_status_names_the_agent` en `crates/clawg-cli/src/mcp/tools.rs`.

## R3. El puente lee y modifica todo lo que alcanza

- `list_memories` MUST listar las filas que alcanza el agente del puente, de todos los proyectos, lo más nuevo primero, con ` by <autor>` en cada fila de otro agente ([`specs/mcp`](../mcp/spec.md)), y MUST aceptar un `agent` opcional que deja sólo las filas de ese autor.
- `read_memory`, `update_memory`, `delete_memory`, `restore_memory` y `resolve_conflict` MUST tratar una fila de otro agente, o sin autor, que el agente alcanza como una propia.
- Sobre una fila que `sharing.json` le oculta, cada una MUST fallar como si el id no existiera y MUST NOT tocar la fila, su vector, sus disputas ni su importancia, tipo o tier. `resolve_conflict` MUST fallar así si oculta cualquiera de las dos. `read_memory` MUST nombrar como faltante un pedazo del save que le oculta ([`specs/pieces`](../pieces/spec.md), R6).
- El workspace, RECENT, la sesión registrada y el aviso del hook siguen siendo del agente ([`specs/sharing`](../sharing/spec.md) R5).

#### Escenario: borrar lo compartido
- GIVEN una memoria de A que B acaba de ver en `search_memory` marcada `by A`
- WHEN B llama `delete_memory` con su id
- THEN la fila queda borrada, A deja de encontrarla, y `restore_memory` de cualquiera de los dos la trae de vuelta

#### Escenario: lo oculto
- GIVEN una memoria de A y `sharing.json` con `hide B A`
- WHEN B llama `delete_memory`, `update_memory`, `read_memory` o `restore_memory` con su id
- THEN la respuesta es un error de «no existe», la fila sigue igual y A la sigue encontrando

#### Escenario: listar
- GIVEN una memoria de A y una de B
- WHEN B llama `list_memories`
- THEN aparecen las dos, la de A con ` by A`; con `agent: A`, sólo la de A

**Test:** `another_agent_deletes_a_memory_unless_hidden`, `another_agent_restores_a_memory_unless_hidden`, `another_agent_reads_a_save_unless_hidden`, `another_agent_updates_a_memory_unless_hidden`, `another_agent_resolves_a_conflict_unless_hidden`, `a_shared_memory_is_changed_by_the_agent_that_found_it` y `list_memories_names_other_authors_and_filters_by_agent` en `server.rs`.

## R4. El lector en el daemon

- `DELETE`, `PATCH` y `GET …/save` de `/v1/memories/{kind}/{id}` y `POST /v1/memories/memory/{id}/restore` MUST aceptar `agent` en la query, y `POST /v1/disputes/judge`, `POST /v1/disputes/resolve`, `POST /v1/synthesis` y `GET /v1/disputes` MUST aceptar `agent_id`, como antes: un puente anterior que los manda no recibe un 400. Nombran al lector, cuyo alcance el daemon lee de `sharing.json` en cada pedido (`sharing::reader_in`); un `sharing.json` que no se puede leer MUST dejarlo con sus filas y las sin autor.
- Una fila fuera del alcance del lector (o alguna de las dos, al juzgar) MUST responderse `404`, igual que un id inexistente, sin cambiar nada: ni la fila, ni su vector, ni sus disputas, ni su `last_modified_at`, ni su `update_count`. En el `GET` del save, el alcance se aplica a la fila pedida y a cada posición: un pedazo fuera del alcance figura como faltante con `ids` en `null`.
- Sin lector, la operación MUST valer sobre cualquier fila.
- Un `PATCH` que mueve la fila de proyecto ([`specs/update`](../update/spec.md) R7) MUST rechazar, sin nombrarla, una unidad con una fila fuera del alcance del lector (`out_of_reach`), y MUST juzgar los choques en el destino sólo con las filas que el lector alcanza, como la dedup y la clave de un save. El lote de `POST /v1/memories/move` no tiene lector.
- La misma regla vale para las tres tablas (`memory`, `episodic`, `procedural`), salvo el restore, que sólo existe para `memory`.

#### Escenario: otro agente borra
- GIVEN una fila de `memory` de A con su vector
- WHEN se llama `DELETE /v1/memories/memory/{id}?agent=B`
- THEN responde `200` con `deleted = 1`; con `hide B A`, `404`, y la fila y su vector siguen ahí

**Test:** `get_save_of_another_agent_reads_whole_unless_hidden`, `delete_with_a_hidden_author_is_404_and_another_agent_deletes_otherwise`, `patch_with_a_hidden_author_is_404_and_another_agent_patches_otherwise`, `resolve_with_a_hidden_author_is_404_and_another_agent_resolves_otherwise`, `restore_brings_a_deleted_memory_back_for_any_agent_that_reaches_it` y `an_unreadable_policy_leaves_an_agent_its_own_rows_and_the_authorless` en `crates/clawg-cli/tests/api.rs`; `a_hidden_authors_row_is_not_found_and_stays`, `another_agent_updates_a_row_it_reaches_but_not_a_hidden_one` y `restoring_needs_a_deleted_row_in_reach` en `crates/clawg-core/src/memory/admin.rs`; `another_agent_judges_a_pair_it_reaches` y `a_pair_with_a_hidden_side_is_not_found` en `crates/clawg-core/src/memory/dispute.rs`.

## R5. Ruta del workspace

- La ruta del workspace de un agente MUST quedar dentro de la carpeta de workspaces para todo `agent_id`.
- Un segmento seguro MUST usarse tal cual: su ruta es la misma que antes de codificar.
- Un `agent_id` que no es segmento seguro MUST codificarse con `%XX` (hexadecimal en mayúsculas) por cada byte de un carácter no admitido, de cada `.` o espacio del final, y de la primera letra de un nombre de dispositivo de Windows. Dos `agent_id` distintos MUST dar segmentos distintos, también para Windows, que descarta los `.` y espacios finales.
- La codificación MUST ser la misma en todas las plataformas.
- Adoptar el workspace de un id de proyecto heredado MUST usar el nombre de la carpeta del agente tal como está en disco, sin volver a codificarlo.

#### Escenario: nombre peligroso
- GIVEN un agente `../escape`
- WHEN se regenera su workspace
- THEN el archivo queda bajo `<workspaces>/..%2Fescape/...`, dentro de la carpeta de workspaces

#### Escenario: nombres que Windows reescribe
- GIVEN los agentes `agent`, `agent.`, `con` y `nul.txt`
- WHEN se escribe el workspace de cada uno
- THEN quedan en `agent`, `agent%2E`, `%63on` y `%6Eul.txt`, cada uno con su propio archivo

#### Escenario: nombre normal
- GIVEN un `workspace.md` con sección USER escrita a mano para `claude-code`
- WHEN se regenera
- THEN la ruta es `<workspaces>/claude-code/<proyecto>/workspace.md` y la sección USER se conserva

**Test:** `a_safe_agent_id_is_its_own_segment`, `an_unsafe_agent_id_is_encoded_inside_the_root`, `encoding_never_makes_two_agents_share_a_folder`, `adopting_a_legacy_workspace_keeps_an_encoded_folder`, `a_name_windows_would_rewrite_is_encoded` y `windows_naming_rules_keep_agents_apart_on_disk` en `crates/clawg-core/src/memory/workspace.rs`.

## R6. Datos de la versión anterior

**Test:** `rows_and_workspace_of_a_client_name_survive_pinning_it` en `server.rs`: siembra con SQL directo filas con el `agent_id` que reportaba un cliente (como las escribía el puente) y un `workspace.md` con sección USER en la ruta de siempre. Un puente con `--agent` igual a ese nombre, sin `clientInfo`, las encuentra con `search_memory` y `list_memories`, y la sección USER sigue en el contexto de `recall_context`.
