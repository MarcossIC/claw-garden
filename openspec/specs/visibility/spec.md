# visibility — Especificación

Fuente de verdad del comportamiento actual de la identidad del agente y de qué filas puede listar, borrar, cambiar o resolver cada agente. También define la ruta en disco del workspace de cada agente. Lo que la búsqueda y `recall_context` leen de otros agentes (por defecto todo, salvo lo que el usuario oculta en `sharing.json`) está en [`specs/sharing`](../sharing/spec.md): leer lo ajeno no da derecho a modificarlo, y para borrar, cambiar, resolver o leer un save, una fila ajena sigue siendo "no existe" (R3 y R4).

**Origen:**
- [visibility-policy](../../changes/archive/2026-10-12-visibility-policy/proposal.md), partes A y B, archivado el 2026-10-12.
- [agent-sharing](../../changes/archive/2026-10-14-agent-sharing/proposal.md), archivado el 2026-10-14 (sólo lo que dice de lo ajeno compartido en R3 y R4).

## Definiciones

- **Agente del puente:** el `agent_id` con el que el puente MCP lee y escribe en una llamada.
- **Fila del agente A:** fila de `memory`, `episodic_trajectory` o `procedural_skill` con `agent_id = A`.
- **Llamada con scope:** una llamada al daemon que nombra un agente (`agent` en la query de `DELETE`/`PATCH` y de `GET /v1/memories/{kind}/{id}/save`, `agent_id` en el cuerpo de `POST /v1/disputes/resolve`, `agent` en `GET /v1/memories`).
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

## R3. El puente sólo modifica y lista lo propio

- `list_memories` MUST listar sólo filas del agente del puente, de todos sus proyectos.
- `delete_memory` sobre una fila de otro agente MUST fallar como si el id no existiera, y MUST NOT tocar la fila, su vector ni las disputas que la nombran.
- `update_memory` sobre una fila de otro agente MUST fallar como si el id no existiera, y MUST NOT cambiar su importancia, su tipo ni su tier ([`specs/update`](../update/spec.md), R5).
- `resolve_conflict` con alguna de las dos filas de otro agente MUST fallar como si el id no existiera, y MUST NOT cambiar ninguna de las dos.
- `read_memory` sobre una fila de otro agente, o sobre el save de una fila de otro agente, MUST fallar como si el id no existiera ([`specs/pieces`](../pieces/spec.md), R6). Su ruta de daemon, `GET /v1/memories/{kind}/{id}/save`, responde `404` con el mismo "no existe" (R4).
- `restore_memory` sobre una fila borrada de otro agente MUST fallar como si el id no existiera, igual que sobre una fila viva o purgada, y MUST NOT cambiar la fila ([`specs/restore`](../restore/spec.md), R1).
- Sobre filas propias, las seis se comportan igual que sin scope.
- Que una fila ajena llegue al agente por `search_memory` o `recall_context` porque `specs/sharing` la comparte (marcada `by <autor>`) no cambia nada de lo anterior: para el agente que no es su autor, modificarla, resolverla o leer su save sigue siendo "no existe", y la fila queda intacta.

#### Escenario: borrar lo ajeno
- GIVEN una memoria de A
- WHEN B llama `delete_memory` con su id
- THEN la respuesta es un error, la fila sigue viva, y A la sigue encontrando con `search_memory`

#### Escenario: borrar lo compartido
- GIVEN una memoria de A que B acaba de ver en `search_memory` marcada `by A`
- WHEN B llama `delete_memory` con su id
- THEN la respuesta es un error, la fila sigue viva y A la sigue encontrando ([`specs/sharing`](../sharing/spec.md) R5)

#### Escenario: listar
- GIVEN una memoria de A y una de B
- WHEN B llama `list_memories`
- THEN sólo aparece la de B

#### Escenario: resolver lo ajeno
- GIVEN una disputa entre dos filas de A
- WHEN B llama `resolve_conflict` sobre el par
- THEN la respuesta es un error y las dos filas siguen igual

**Test:** `an_agent_cannot_delete_another_agents_memory`, `an_agent_cannot_update_another_agents_memory`, `an_agent_lists_only_its_own_memories`, `an_agent_cannot_resolve_another_agents_conflict` y `an_agent_cannot_read_another_agents_save` en `server.rs`; `a_shared_memory_is_read_only` en `server.rs` para lo compartido; `an_agent_cannot_restore_another_agents_memory` en `server.rs` para `restore_memory`.

## R4. Scope opcional en el daemon

- `DELETE` y `PATCH /v1/memories/{kind}/{id}` MUST aceptar `agent` en la query, y `POST /v1/memories/{kind}/{id}/restore` también (`restore_memory`, `crates/clawg-cli/src/daemon/api.rs:1469`); `GET /v1/memories/{kind}/{id}/save` MUST aceptar lo mismo; `POST /v1/disputes/resolve` MUST aceptar `agent_id` en el cuerpo.
- Con scope, si la fila (o alguna de las dos, al resolver) es de otro agente, la respuesta MUST ser `404`, igual que un id inexistente, y MUST NOT cambiar nada: ni la fila, ni su vector, ni sus disputas, ni su `last_modified_at`, ni su `update_count`. En el `GET` del save, el scope se aplica a la fila pedida (si es ajena, 404) y a cada posición del save: un pedazo de otro agente, que el dedup dejó dentro de un save propio, figura como faltante con `ids` en `null` ([`specs/pieces`](../pieces/spec.md), R6).
- La regla no depende de lo que el agente ve: una fila que `specs/sharing` le muestra en una búsqueda es igual de ajena para el scope. Lo mismo vale para `POST /v1/synthesis`, que con `agent_id` sólo mira filas de ese agente.
- Sin scope, la operación MUST valer sobre cualquier fila, como antes de existir el scope.
- La misma regla vale para las tres tablas (`memory`, `episodic`, `procedural`), salvo el restore, que sólo existe para `memory`.

#### Escenario: borrar con scope
- GIVEN una fila de `memory` de A con su vector
- WHEN se llama `DELETE /v1/memories/memory/{id}?agent=B`
- THEN responde `404`, y la fila y su vector siguen ahí

#### Escenario: sin scope
- GIVEN la misma fila
- WHEN se llama `DELETE /v1/memories/memory/{id}` sin `agent`
- THEN responde `200` con `deleted = 1`

**Test:** `delete_with_another_agent_is_404_and_keeps_the_row`, `patch_with_another_agent_is_404_and_keeps_the_importance`, `resolve_with_another_agent_is_404_and_changes_nothing` y `get_save_of_another_agent_is_404` en `crates/clawg-cli/tests/api.rs`; `an_owner_mismatch_deletes_nothing`, `an_owner_mismatch_changes_no_importance` en `crates/clawg-core/src/memory/admin.rs` y `a_pair_of_another_agent_is_not_found` en `crates/clawg-core/src/memory/dispute.rs`; para el restore, `restore_brings_a_deleted_memory_back_for_its_owner` (el `404` con otro agente) en `api.rs` y `restoring_needs_a_deleted_row_of_the_owner` en `admin.rs`.

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
