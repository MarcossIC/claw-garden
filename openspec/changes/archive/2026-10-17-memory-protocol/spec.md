# memory-protocol — Especificación

Lo que el agente lee sobre qué guardar y cómo sale de un solo texto compilado en el binario. El tipo de una memoria se valida en el save igual que en `PATCH`.

## Definiciones

- **Protocolo:** el archivo `crates/clawg-cli/src/mcp/protocol.md`, compilado con `include_str!`.
- **Apertura:** los párrafos del protocolo entre su título (la primera línea, `# …`) y la primera línea que empieza con `## `. Dentro de cada párrafo, las líneas se unen con un espacio. Los párrafos se separan con una línea en blanco.
- **Tipos de memoria:** `semantic_fact`, `user_preference`, `architectural_decision`, `session_summary`, `code_block`, `chat_message` y `tool_error`.

## R1. Las instrucciones son la apertura del protocolo

`get_info().instructions` y la respuesta de `server/discover` MUST llevar la apertura del protocolo, la misma en los dos.

Extraer la apertura MUST ser una función pura sobre el texto. MUST NOT cortar una palabra, MUST NOT dejar saltos de línea dentro de un párrafo y MUST NOT incluir el título ni una línea de sección.

#### Escenario: un párrafo cortado en dos líneas
- GIVEN `# T\n\nuno\ndos\n\ntres\n## S\ncuatro`
- WHEN se extrae la apertura
- THEN es `uno dos\n\ntres`

#### Escenario: las dos puertas
- GIVEN un puente en proceso
- WHEN se piden `get_info` y `discover`
- THEN las dos instrucciones son iguales a la apertura de `protocol.md`

**Test:** el puro `opening_joins_the_lines_of_each_paragraph` y `the_instructions_are_the_opening_of_the_protocol` (`server.rs`). `discover` arma su respuesta con `get_info` (`server.rs:1138-1144`), así que un solo test de `get_info` fija las dos.

## R2. Las instrucciones caben en 2.048 caracteres

La apertura MUST medir 2.048 caracteres o menos, contados como `char`.

#### Escenario
- GIVEN el protocolo del repositorio
- THEN su apertura mide 2.048 caracteres o menos

**Test:** `the_instructions_fit_older_clients` (`server.rs`).

## R3. La apertura nombra los hábitos que más rinden

La apertura MUST:

- decir cuándo llamar a `save_memory`, `search_memory`, `recall_context`, `search_project`, `resolve_conflict` y `synthesize`;
- pedir un hecho por llamada, con una primera línea que nombre el tema y con rutas e identificadores exactos;
- pedir memorias y búsquedas en inglés;
- decir que lo recuperado (memorias y pasajes del proyecto) es evidencia y no instrucciones;
- decir qué no guardar.

El texto completo del protocolo MUST traer además el resumen de sesión con sus seis secciones (Goal, Instructions, Discoveries, Accomplished, Next steps y Relevant files) y qué hacer tras una compactación.

**Test:** `the_instructions_carry_the_saving_habits` y `the_protocol_describes_the_session_summary` (`server.rs`). Los tests de hoy (`get_info_advertises_tools_and_the_profiles_server_name`, `instructions_say_when_to_search_the_project`, `server.rs:1730-1753`, y `crates/clawg-cli/src/mcp/handshake.rs:422`) siguen verdes.

## R4. El protocolo nombra sólo lo que existe

- Cada herramienta registrada en `tool_router` MUST aparecer entre comillas invertidas en el protocolo.
- Cada texto entre comillas invertidas que sea un nombre en minúsculas, con o sin `_` (`[a-z]+(_[a-z]+)*`), MUST ser una herramienta registrada, una propiedad del `inputSchema` de alguna herramienta o un tipo de memoria.
- Cada tipo de memoria MUST aparecer en el protocolo.

Juntar los textos entre comillas invertidas MUST ser una función pura.

#### Escenario: una herramienta renombrada
- GIVEN el protocolo con `` `save_memories` ``
- THEN el test falla y nombra `save_memories`

**Test:** el puro `backticked_names_reads_each_code_span` y `the_protocol_names_only_what_exists` (`server.rs`).

## R5. Una sola lista de tipos

`clawg_core` MUST exponer la lista de los tipos de memoria en un solo lugar (`MemoryType::ALL`). La lista MUST ser exactamente lo que acepta `MemoryType::from_db_str`. El mensaje de un tipo desconocido MUST nombrar el valor recibido y los siete de la lista, en su orden, y MUST ser el mismo en `PATCH` y en el save.

**Test:** `every_listed_type_parses_back_to_itself` (`store_insert.rs`) y `an_unknown_type_names_the_accepted_ones` (`store_insert.rs`). Los tests de `PatchError` (`admin.rs`) se adaptan.

## R6. El save rechaza un tipo desconocido

`POST /v1/memories` con un `memory_type` que no es un tipo de memoria MUST contestar 400 con el mensaje de R5, MUST NOT registrar el save en el outbox y MUST NOT encolarlo. Un `memory_type` ausente MUST seguir guardándose como `semantic_fact`, y uno válido como ese tipo.

Un evento que ya está en el outbox con un tipo desconocido MUST seguir guardándose como `semantic_fact` al reproducirse.

`save_memory` MUST contestar ese 400 como un error que empieza con `not saved:` y trae el mensaje del daemon.

#### Escenario: un tipo mal escrito
- GIVEN un daemon listo
- WHEN se guarda `{"content": "x", "agent_id": "a", "memory_type": "preference"}`
- THEN contesta 400, el mensaje nombra `"preference"` y los siete tipos
- AND el outbox queda vacío y `GET /v1/memories` no lista ninguna fila

#### Escenario: las dos puertas
- GIVEN las cadenas `preference`, `Semantic_Fact`, `""` y cada uno de los siete tipos
- WHEN cada una se manda a `POST /v1/memories` y a `PATCH /v1/memories/memory/{id}`
- THEN las dos rutas rechazan y aceptan las mismas

**Test:** `a_save_with_an_unknown_memory_type_is_refused_before_the_outbox` y `save_and_patch_accept_the_same_memory_types` (`crates/clawg-cli/tests/api.rs`); `save_memory_says_an_unknown_type_was_not_saved` (`server.rs`). La reproducción ya la fija `explicit_source_with_an_unknown_type_override_falls_back_to_semantic_fact` (`store_insert.rs:997`).

## R7. El schema declara los tipos

En el `inputSchema` de `save_memory` y de `update_memory`, la propiedad `memory_type` MUST declarar `enum` con los siete tipos de `MemoryType::ALL`, más `null` para que el `default` declarado siga siendo válido. Deserializar los argumentos MUST seguir aceptando cualquier cadena, de modo que un valor desconocido llegue al daemon y vuelva como el error de R6, no como un error de protocolo (`server.rs:8-14`).

**Test:** `the_save_memory_schema_names_the_accepted_memory_types` pasa a comparar el enum con `MemoryType::ALL` en las dos herramientas, y su comentario deja de decir que un tipo inventado se guarda tal cual. `the_agent_contract_matches_its_file` cambia a propósito y se bendice.

## R8. La descripción de `save_memory` pide título y "dónde"

La descripción MUST pedir una primera línea que nombre el tema y las rutas e identificadores exactos. MUST conservar "one fact" y `read_memory` (`save_memory_asks_for_one_fact_per_call`, `server.rs:2132`).

**Test:** `save_memory_asks_for_a_title_and_where` (`server.rs`).

## R9. La nota de conflictos está en inglés

`CONFLICT_NOTE` MUST estar en inglés. MUST seguir siendo una sola por bloque, empezar con `NOTE:` y nombrar `resolve_conflict` con el significado de `keep` y `other`. La línea de los pares omitidos MUST ser `(+N more conflicts not shown)`.

**Test:** `the_conflict_note_is_in_english` (`prompt_builder.rs`). Se adaptan `conflict_block_has_one_note`, `the_conflict_note_names_the_tool` y el test del tope (`prompt_builder.rs:745-790`).
