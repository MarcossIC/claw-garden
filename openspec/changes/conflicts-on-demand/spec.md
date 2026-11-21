# conflicts-on-demand — Especificación

CO1 a CO13 son los requisitos de la [propuesta](proposal.md). Ninguno toca `memory.db`: el esquema, los estados, los ids y los veredictos no cambian (CO13). Al archivar, cada uno entra en la spec principal que dice su sección: [`specs/retrieval`](../../specs/retrieval/spec.md) (CO1 a CO6), [`specs/disputes`](../../specs/disputes/spec.md) (CO7), [`specs/mcp`](../../specs/mcp/spec.md) (CO8 a CO10), [`specs/metrics`](../../specs/metrics/spec.md) (CO11) y [`specs/hooks`](../../specs/hooks/spec.md) (una referencia en R4, CO1). Esta spec no edita ninguna.

## Definiciones

- **Par escalado:** el que `fetch_active_conflicts` trae hoy: una fila nueva `disputed` con la vieja en `disputed_against`, las dos sin `deleted_at` y las dos dentro del alcance del lector (`crates/clawg-core/src/memory/retrieval/prompt_builder.rs:1264-1325`). Es el mismo par que cuenta `memory_status`, salvo por el proyecto: `memory_status` cuenta por `memory_conflict.state = 'escalated'` (`open_pairs`, `crates/clawg-core/src/memory/revision/dispute.rs:524-568`).
- **Otro lado** de una fila: la otra fila de cada par escalado en que está. Una fila nueva que disputa a varias tiene varios; una de en medio de una cadena, uno por cada par.
- **Marca `conflict=`:** ` conflict=#N` con el id del otro lado. Con varios, los ids van ordenados de menor a mayor, separados por coma y sin espacios, hasta tres, y el resto se cuenta: `conflict=#9,#14,#20+2`. Tres, porque un par escalado exige dos preferencias o decisiones cercanas, o dos textos con un identificador común ([`specs/disputes`](../../specs/disputes/spec.md)), así que una fila rara vez está en más de dos, y la lista completa sigue en `list_memories` (CO8).
- **Fila mostrada:** un hit de RECENT o LONG-TERM, o una entrada de la sección AUTO del workspace que el prompt imprime (`shown`, `prompt_builder.rs:365`). La línea de LAST SESSION no es una fila mostrada: no entra en `shown` ni en los pares ([`specs/retrieval`](../../specs/retrieval/spec.md), «La última sesión»).
- **Contexto sin consulta:** `POST /v1/context` sin `query`, o con una en blanco, que arma `build_session_prompt` (`prompt_builder.rs:311-331`): lo que pide `clawg hook session-start`.

## CO1. El contexto sin consulta no trae ACTIVE CONFLICTS

`build_session_prompt` MUST NOT pedir pares escalados ni imprimir el bloque `=== ACTIVE CONFLICTS (review needed) ===` ni su NOTE, aunque una fila de RECENT o de AUTO sea un lado de un par (hoy los pide con los ids de RECENT y de AUTO, `prompt_builder.rs:365` y `:374`). Sigue llevando el workspace, LAST SESSION y RECENT, y las filas en disputa que imprime llevan la marca de CO2. El arranque MUST NOT decir cuántos pares esperan ni pedir que se pregunte por ellos: quien los quiere los pide (CO8). Vale para todo cliente que pida el contexto sin consulta, `clawg hook session-start` de Claude Code y de Cursor incluidos.

#### Escenario: una fila reciente y una preferencia en disputa
- GIVEN un par escalado cuyo lado nuevo es un save de ayer y cuyo lado viejo es una preferencia que la sección AUTO imprime
- WHEN se pide el contexto sin consulta
- THEN el prompt no contiene `ACTIVE CONFLICTS`, ni `NEW `, ni `OLD `, ni la NOTE
- AND las dos filas llevan la marca de CO2

#### Escenario: sin pares
- GIVEN un store sin pares escalados
- WHEN se pide el contexto sin consulta
- THEN el prompt es byte a byte el de antes

**Tests (hoy fallan):** en `prompt_builder.rs` (mod tests, con el store), uno por escenario; el segundo guarda que lo no marcado no cambia. `e2e_full_4_block_output` sigue pasando: arma con consulta.

## CO2. Cada fila en disputa que el prompt imprime lleva `conflict=#N`

Con o sin consulta, una fila mostrada que es un lado de un par escalado MUST llevar la marca con el id del otro lado, y el otro lado MUST llevar la suya con el primero. La consulta y el alcance son los de `fetch_active_conflicts` (misma regla de lectura: un lado que el lector no alcanza no se nombra, ni como otro lado de un par que sí alcanza).

- **Un hit** (`format_hit_line`, `prompt_builder.rs:828`): la marca va dentro del encabezado, después del tipo, de la marca de origen y de `piece=`, y antes de ` by <autor>`: `[2026-11-20 #12 user_preference conflict=#9] texto`.
- **Una entrada de la sección AUTO**, que no imprime su id: `(#12 conflict=#9)` al final de su primera línea, con el id de la propia entrada. Sólo en el prompt: `workspace.md` MUST NOT cambiar.
- La marca no depende del tope de pares del bloque (`MAX_CONFLICT_PAIRS`, `prompt_builder.rs:57`): una fila de un par que el bloque no imprime, o que el presupuesto sacó (CO3), sigue marcada.
- Una fila sin pares escalados MUST imprimirse byte a byte como hoy. LAST SESSION no se marca.
- La marca no cambia el orden, el score ni el presupuesto de nadie: sólo suma sus bytes al texto que `fit_to_budget` mide.

El comentario de `crates/clawg-core/src/memory/retrieval/workspace.rs:390-393` MUST dar la razón nueva de que la sección AUTO conserve las filas `disputed`: la marca avisa de la disputa junto a cada una, y ya no el bloque. No tiene test: es un comentario, y `verify` lo mira en el diff.

#### Escenario: un hit y una entrada del workspace
- GIVEN el par (old 9, new 12), con 12 en RECENT y 9 en la sección AUTO
- WHEN se arma el prompt
- THEN la línea de 12 es `[<fecha> #12 <tipo> conflict=#9] <texto>`
- AND la entrada de 9 termina su primera línea con `(#9 conflict=#12)`
- AND `workspace.md` es el mismo archivo que sin la marca

#### Escenario: un lado fuera del alcance
- GIVEN un par cuyo otro lado es de un agente que `sharing.json` le oculta al lector
- WHEN se arma el prompt
- THEN la fila visible no lleva marca y ningún id del lado oculto aparece

#### Escenario: más de tres
- GIVEN una fila nueva que disputa a cinco filas (ids 4, 7, 9, 11 y 15)
- WHEN se imprime como hit
- THEN su encabezado lleva `conflict=#4,#7,#9+2`

#### Escenario: el bloque con tope
- GIVEN siete pares escalados con un lado en RECENT, y una consulta
- WHEN se arma el prompt
- THEN el bloque imprime cinco pares y `(+2 more conflicts not shown)`, y los siete hits llevan su marca

**Tests (hoy fallan):** en `prompt_builder.rs` (mod tests): la línea de un hit, la de una entrada de AUTO, el lado oculto (junto a `a_conflict_side_cannot_open_a_new_or_old_line` y los de alcance de `fetch_active_conflicts`), el tope de tres y el bloque con tope (junto a `conflict_block_keeps_five_pairs_and_counts_the_rest`); en `workspace.rs`, que `workspace.md` no lleva la marca. Los de línea sin marca que hoy pasan (`format_hit_line_drops_the_old_labels`, `hit_line_is_one_header_with_id_and_text`) guardan la regla byte a byte.

## CO3. Con consulta, el presupuesto corta los pares antes que LAST SESSION

`fit_to_budget` (`prompt_builder.rs:468-540`) MUST quitar, de a uno y midiendo el texto entero después de cada paso (como hoy), en este orden: las entradas de AUTO que pasan de la mitad del presupuesto (SC2), los hits que pasan de la reserva, **los pares desde el último**, **LAST SESSION**, los hits que quedan y las entradas de AUTO que queden. Hoy LAST SESSION sale antes que los pares (`:495-504`). Un lado cuyo hit se cortó sigue imprimiéndose con su texto en el bloque (`render`, `:657`; `format_conflict_block`, `:893-901`), y la marca CO2 queda en los hits que no se cortaron. La reserva (un cuarto del presupuesto con consulta, 0 sin ella) y SC5 no cambian. Sin consulta no hay pares, así que su orden es el de antes: AUTO sobre la mitad, todos los saves, LAST SESSION y el resto de AUTO.

Un texto que entra entero MUST seguir siendo byte a byte el que se arma sin presupuesto, y `context.cut` MUST seguir contando `conflicts` por cada paso que quitó un par (`PromptBlock::Conflicts`).

#### Escenario: la reserva
- GIVEN cuatro hits de 1.000 bytes, un resumen de LAST SESSION y un par, y una reserva en la que caben dos hits
- WHEN el presupuesto alcanza quitando los dos hits de menor score
- THEN se van esos dos, y LAST SESSION y el par quedan enteros
- AND con un presupuesto que no alcanza para eso, el par se va antes que LAST SESSION, y LAST SESSION antes que los dos hits de la reserva

#### Escenario: los pares ceden al resumen
- GIVEN una consulta, un resumen de 3.000 bytes, un hit y tres pares, y un presupuesto en el que no entra todo
- WHEN se cortan los pares y con eso el texto entra
- THEN LAST SESSION queda entero, el hit sigue marcado y el bloque cuenta los pares que se fueron en `(+N more conflicts not shown)`

**Tests (hoy fallan):** cambian `the_hits_in_their_room_outlast_the_last_session_and_the_conflicts` y `conflicts_go_after_every_hit` (`prompt_builder.rs`), y el escenario «la reserva» de [`specs/retrieval`](../../specs/retrieval/spec.md#el-prompt-cumple-el-presupuesto); es nuevo uno del segundo escenario. `the_last_session_block_gives_way_after_the_hits` sigue (sin pares).

## CO4. El arranque trae entero el resumen de la sesión anterior

Un contexto sin consulta con un workspace, un resumen largo y saves recientes que no entran juntos MUST traer LAST SESSION entero mientras lo que sobra pueda cortarse de la sección AUTO (sobre la mitad del presupuesto, SC2) y de RECENT. Sólo se acorta cuando lo que escribió la persona o lo que no se puede cortar pasa solo el presupuesto (SC5). Es el criterio del piloto «al abrir una sesión, el contexto trae entero el resumen de la sesión anterior» ([ROADMAP](../../ROADMAP.md#piloto-una-semana-de-trabajo-real-no-una-noche)): hoy falla porque el texto de los lados de los pares pasa al bloque cuando se corta su hit y, con él, `fit_to_budget` llega a LAST SESSION antes que a los pares (`prompt_builder.rs:495-504`).

#### Escenario: un store maduro al empezar la sesión
- GIVEN un workspace con la sección AUTO llena, un resumen de sesión de 3.500 bytes, diez saves recientes y cinco pares escalados con un lado entre ellos, que no entran juntos en 8.000 bytes
- WHEN se pide el contexto sin consulta
- THEN el prompt trae el resumen entero bajo LAST SESSION, cabe en el presupuesto y no contiene `ACTIVE CONFLICTS`
- AND `context.cut` no registra `last_session`

**Tests (hoy fallan):** en `prompt_builder.rs` (mod tests, con el store), junto a `a_context_without_a_query_keeps_the_last_session_over_its_saves` y `a_stocked_workspace_leaves_room_for_the_last_session`; con pares, que ésos no tienen.

## CO5. `POST /v1/search` manda el otro lado de cada hit

Cada hit de `memory` que `POST /v1/search` devuelve MUST llevar `conflicts_with`, la lista de los ids del otro lado de cada par escalado en que está, con la regla de lectura de `fetch_active_conflicts` (los dos lados vivos y alcanzables por el lector). MUST omitirse cuando es vacía, como `origin` (PM5 de [`specs/retrieval`](../../specs/retrieval/spec.md)), así que un puente anterior lee la respuesta como siempre. Se calcula con una consulta por búsqueda, sobre los ids de los hits que devuelve. MUST NOT nombrar un par `pending`, un par con un lado `deleted_at`, ni un lado que el lector no alcanza. La búsqueda no cambia: ni el ranking, ni el score, ni lo que `search.found` mide.

#### Escenario: los dos lados
- GIVEN el par escalado (old 1, new 2), y una búsqueda que devuelve las dos filas
- WHEN responde `POST /v1/search`
- THEN el hit de 2 trae `conflicts_with: [1]` y el de 1 trae `conflicts_with: [2]`

#### Escenario: lo que no cuenta
- GIVEN un par pendiente (no escalado), un par escalado con un lado borrado y otro con un lado de un agente oculto
- WHEN la búsqueda devuelve esas filas
- THEN ningún hit lleva `conflicts_with`, y el cuerpo de cada hit es el de antes

#### Escenario: un solo lado devuelto
- GIVEN el par (old 1, new 2) y una búsqueda cuyos hits no traen a 1
- WHEN responde
- THEN el hit de 2 trae `conflicts_with: [1]`

**Tests (hoy fallan):** en `crates/clawg-cli/tests/api.rs`, uno por escenario; `ranked_hit_serializes_to_json` (`crates/clawg-core/tests/public_api.rs`) guarda que el campo se omite vacío.

## CO6. `search_memory` muestra ` conflict=#N`

El `Hit` del puente MUST leer `conflicts_with` con `#[serde(default)]` (`crates/clawg-cli/src/client.rs:212`): un daemon anterior no lo manda y el hit lo toma como vacío. `format_hits` (`crates/clawg-cli/src/mcp/tools.rs:232-267`) MUST mostrar la marca de las Definiciones justo después de `score=` y antes de `topic=`, `project=` y ` by`: `N. #id [Memory/user_preference/t1 score=0.83 conflict=#9] texto`. Un hit sin `conflicts_with` MUST imprimirse byte a byte como hoy, y la marca no cambia qué hits se muestran ni su orden.

#### Escenario: un hit en disputa
- GIVEN un hit con `conflicts_with: [9]` y score 0,83
- THEN su línea es `1. #12 [Memory/user_preference/t1 score=0.83 conflict=#9] <texto>`

#### Escenario: un daemon anterior
- GIVEN una respuesta de `/v1/search` sin `conflicts_with`
- THEN `search_memory` muestra la línea de hoy

**Tests (hoy fallan):** en `tools.rs` (mod tests), junto a `format_hits_shows_the_origin_of_a_preference`; en `client.rs`, junto a `a_hit_from_an_old_daemon_has_no_origin`; en `server.rs`, `search_memory` contra un daemon de prueba que marca.

## CO7. `GET /v1/disputes` lista los escalados a pedido

`GET /v1/disputes` MUST aceptar `escalated=true`, un booleano. Con él, `pairs` MUST traer los pares **escalados** en lugar de los pendientes (`open_pairs` filtra hoy `p.state = 'pending'`, `dispute.rs:545`): los más viejos primero, con el orden de ahora (`p.created_at, p.new_id, p.old_id`), hasta **10** pares (`ESCALATED_LISTED`, nombre propuesto; los pendientes siguen en 5, `OPEN_PAIRS_SHOWN`, `crates/clawg-cli/src/daemon/api/disputes.rs:59`). Cada par MUST traer, además de lo que trae un par pendiente (ids, distancia, fecha y marca de origen de cada lado), la primera línea del texto de cada lado (`new_line` y `old_line`), cortada a los 500 caracteres de la vista previa de `list_memories` y `search_memory` (`CONTENT_PREVIEW_LIMIT`, `crates/clawg-cli/src/mcp/tools.rs:41`). Los conteos `pending` y `escalated`, el alcance (el lector, y el proyecto de la fila nueva como la búsqueda) y los predicados de filas vivas no cambian; sin el parámetro, la respuesta MUST ser la de hoy, byte a byte.

El tope es 10, no 5 ni todos. El bloque de ACTIVE CONFLICTS llega sin que nadie lo pida y por eso cuesta 5; esta lista se pide y puede ser más larga, pero cada par exige que el agente le pregunte a la persona, y diez pares con sus textos caben en unos 10 KB como máximo. En este repositorio hay más de 16: el pedido siguiente trae los demás, porque un par que la persona contesta sale de la lista, y el conteo `escalated` dice cuántos quedan.

El parámetro MUST declararse en `DisputesQuery`, que rechaza lo que no conoce (`deny_unknown_fields`, `disputes.rs:46-48`): un daemon anterior responde un error de consulta, y es lo que lee CO8.

#### Escenario: diez de doce
- GIVEN doce pares escalados y tres pendientes del lector
- WHEN `GET /v1/disputes?escalated=true`
- THEN `escalated` es 12, `pending` es 3, `pairs` trae diez pares escalados, los más viejos, y ninguno pendiente
- AND cada par trae `new_line` y `old_line`

#### Escenario: sin el parámetro
- GIVEN los mismos pares
- WHEN `GET /v1/disputes`
- THEN `pairs` trae los pendientes de siempre, sin `new_line` ni `old_line`

#### Escenario: alcance
- GIVEN un par escalado cuyo lado viejo es de un agente que `sharing.json` le oculta al lector, y uno de otro proyecto
- WHEN `GET /v1/disputes?escalated=true&project_id=P`
- THEN no trae ninguno de los dos

**Tests (hoy fallan):** en `crates/clawg-core/src/memory/revision/dispute.rs` (mod tests), el listado de escalados con su tope y su orden, junto a los de `open_pairs`; en `crates/clawg-cli/tests/api.rs`, los tres escenarios.

## CO8. `list_memories` con `conflicts` lista los pares que esperan a la persona

`ListMemoriesArgs` (`crates/clawg-cli/src/mcp/server.rs:313-333`) MUST aceptar `conflicts`, un booleano opcional. Con `conflicts: true`, `list_memories` MUST listar, en lugar de filas, los pares escalados del proyecto abierto cuando las carpetas abiertas llevan a uno, y los de todos cuando llevan a varios o a ninguno (la regla de `memory_status`, [BI8](../../specs/mcp/spec.md#bi8-memory_status-y-read_memory); `one_project`, `server.rs:664`), con `GET /v1/disputes?escalated=true` (CO7). La salida MUST:

- traer, por cada par, `NEW #N` con la primera línea de la fila nueva y `OLD #M` con la de la vieja, cada lado con su marca de origen cuando la tiene (P4 de [`specs/provenance`](../../specs/provenance/spec.md)), los ids que recibe `resolve_conflict`;
- decir cuántos de los pares escalados quedan sin listar cuando `escalated` pasa de los que trae el daemon (`(+N more waiting, ask again once these are answered)`; en español, su equivalente);
- terminar con una línea que pida preguntarle a la persona por cada par y registrar sólo su respuesta con `resolve_conflict`, `user_answered` y sus palabras en `reason`, sin juzgar ninguno por cuenta propia;
- decir `conflict candidates: none open` (`candidatos a conflicto: ninguno abierto`) y nada más cuando no hay ninguno.

Con `conflicts: true` junto a `agent`, `tier`, `memory_type`, `limit` u `offset`, que son filtros de filas, MUST negarse sin llamar al daemon y decir que `conflicts` no se combina con ellos. Contra un daemon que rechaza `escalated` (CO7), MUST decir que se reinicie el daemon con `clawg stop` y MUST NOT mostrar los pendientes como si fueran escalados, como `resolve_conflict` contra un daemon sin `/v1/disputes/judge` (`server.rs:1599`). Sin `conflicts`, o con `conflicts: false`, `list_memories` MUST ser el de hoy.

La descripción de `conflicts` MUST ser de una oración que diga que lista los pares de memorias que esperan a la persona, con los dos textos, en lugar de filas, y la de `list_memories` MUST nombrarlo en una frase. Pesa unos 150 caracteres por idioma y los paga sólo quien carga `list_memories`.

#### Escenario: los pares del proyecto
- GIVEN dos pares escalados en el proyecto abierto, uno en otro proyecto y uno pendiente
- WHEN el agente llama a `list_memories` con `conflicts: true`
- THEN lista los dos del proyecto, cada uno con `NEW #N <línea>` y `OLD #M <línea>`, y termina con la línea que pide preguntar
- AND no lista el de otro proyecto ni el pendiente

#### Escenario: sin pares
- GIVEN un proyecto sin pares escalados
- WHEN se llama con `conflicts: true`
- THEN la respuesta es `conflict candidates: none open`

#### Escenario: más de diez
- GIVEN doce pares escalados
- WHEN se llama con `conflicts: true`
- THEN lista diez y dice que quedan dos

#### Escenario: con un filtro de filas
- GIVEN `conflicts: true` y `tier: 1`
- WHEN se llama
- THEN se niega sin llamar al daemon

#### Escenario: un daemon anterior
- GIVEN un daemon que rechaza `escalated=true`
- WHEN se llama con `conflicts: true`
- THEN la respuesta pide `clawg stop` y no lista nada

**Tests (hoy fallan):** en `server.rs` (mod tests), uno por escenario, junto a `memory_status_counts_only_the_pairs_that_wait_for_the_user`; en `tools.rs`, el formato (junto a `format_status_ends_with_the_open_pairs`) y que la línea final no juzga ningún par (como `pending_pairs_are_not_put_to_the_agent` para VR4); `the_agent_contract_matches_its_file` y su versión en español (CO9). `list_memories` sin `conflicts`: los existentes, sin cambio.

## CO9. La apertura pide preguntar por una marca, no por el bloque

La frase de ACTIVE CONFLICTS de las instrucciones del servidor (`crates/clawg-cli/src/mcp/protocol.md:28-30` y `protocol.es.md:27-29`) MUST dejar de depender de que `recall_context` liste el bloque, y MUST pedir, en las dos lenguas, que se le pregunte a la persona cuál memoria vale cuando una respuesta depende de una memoria marcada `conflict=`, o cuando la persona pide revisar los conflictos, y que se registre con `resolve_conflict`, `user_answered` y sus palabras en `reason`. MUST conservar «ask the user which memory holds» y «pregúntale a la persona qué memoria vale» (`the_instructions_carry_the_saving_habits` y su versión en español), y MUST NOT nombrar `list_memories`: en inglés no entra. En inglés MUST medir 205 caracteres o menos (la que reemplaza mide 204), y las tres aperturas (sin preferencia, `en` y `es`) MUST seguir en 2.048 caracteres o menos (`the_instructions_fit_older_clients`, `server.rs`; hoy con `en` miden 2.047). Se pierde de la frase vieja «set it only when the user said so»: la guarda sigue en la descripción de `user_answered` y en el `409` del daemon para una fila protegida ([`specs/disputes`](../../specs/disputes/spec.md), R3), y la frase nueva dice «their words». La otra mitad de VR3, la de los saves con cercanas, no cambia.

El español y el inglés MUST nombrar lo mismo entre comillas invertidas (`protocol.rs:270-280` y `:444-454`); `conflict=` no cuenta como nombre (`is_name`, `server.rs:2935-2938`). `protocol.lean*.md` no lleva la apertura y no cambia. El bloque `full` instalado queda `stale` (PB4 de [`specs/hooks`](../../specs/hooks/spec.md)) hasta correr `clawg setup claude-code`; `lean`, `short` y `off` siguen `current`.

#### Escenario: la frase
- GIVEN la apertura en inglés y en español
- THEN dice ``marked `conflict=` `` (o su equivalente) y la condición de pedir la revisión, y conserva la frase de «qué memoria vale»
- AND no dice `ACTIVE CONFLICTS` ni `lists`/`liste` aplicado a `recall_context`

#### Escenario: el largo
- GIVEN las tres variantes
- THEN la frase en inglés mide 205 caracteres o menos y ninguna apertura pasa de 2.048

**Tests (hoy fallan):** `the_instructions_fit_older_clients` y `the_instructions_carry_the_saving_habits` (y su versión en español) en `server.rs`, con el texto nuevo; en `protocol.rs`, que ninguna apertura nombra `ACTIVE CONFLICTS` ni depende de `recall_context` para preguntar, y que `lean` sigue igual. Los contratos (`crates/clawg-cli/src/mcp/agent_contract.json` y `agent_contract.es.json`) se regeneran con `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file` y `... the_spanish_agent_contract_matches_its_file`; el diff se revisa: MUST ser sólo la frase de las instrucciones, las descripciones de `resolve_conflict` y de su `keep`, y la de `list_memories` con su parámetro nuevo.

## CO10. `resolve_conflict` y las líneas de `memory_status` dicen dónde se ven los pares

- La descripción de `resolve_conflict` (`crates/clawg-cli/src/mcp/server.rs:1973-1981`; en español, `tools_es.rs`) MUST decir que juzga un par que nombra `save_memory`, o uno marcado `conflict=` o listado por `list_memories` con `conflicts`, una vez que la persona dijo cuál vale, en lugar de «one under ACTIVE CONFLICTS in `recall_context`», y MUST NOT nombrar `memory_status` como fuente de pares (VR5). Lo demás de la descripción no cambia. La de `keep` (`server.rs:458`) MUST decir que su id es el que muestran la marca `conflict=`, `list_memories` o `save_memory`, en lugar de «ACTIVE CONFLICTS or `save_memory`».
- La línea de `memory_status` (`format_open_pairs`, `tools.rs:286-293`) y la de un veredicto `conflicts` (`format_resolution`, `tools.rs:270-282`) MUST dejar de decir «in ACTIVE CONFLICTS» (`en ACTIVE CONFLICTS`) y decir dónde se listan: que `list_memories` con `conflicts` los lista. VR4 sigue: la línea de `memory_status` dice sólo cuántos pares esperan, sin listar ninguno ni decir cómo juzgarlos; con ninguno, `conflict candidates: none open`. VR5 sigue: ninguna lista de pares en `memory_status`.

#### Escenario: memory_status
- GIVEN un par escalado
- WHEN el agente llama a `memory_status`
- THEN la línea dice que hay 1 esperando a la persona y nombra `list_memories` con `conflicts`
- AND no lista el par ni dice cómo juzgarlo

#### Escenario: un veredicto `conflicts`
- GIVEN un `resolve_conflict` con `conflicts`
- THEN la respuesta dice que el par espera a la persona y dónde se lista, sin nombrar ACTIVE CONFLICTS

**Tests (hoy fallan):** cambian `format_status_ends_with_the_open_pairs`, el de `format_resolution` (`tools.rs:2228`) y `memory_status_counts_only_the_pairs_that_wait_for_the_user` (`server.rs`), que hoy fijan «in ACTIVE CONFLICTS»; `pending_pairs_are_not_put_to_the_agent` sigue. Los contratos, como en CO9.

## CO11. `context.cut` lleva `state`

`record_context` (`crates/clawg-cli/src/daemon/api/memories.rs:1120-1126`) MUST registrar `context.cut` con las etiquetas `block` y `state`, donde `state` es el nombre de `RetrievalState` que ya lleva `context.retrieval_state` (`searched`, `recent`, `embedder_not_ready`, `embedder_failed`, `search_failed`). Las filas viejas de `metrics.db` no la tienen: un reporte por `state` cuenta sólo las nuevas, y no hay nada que migrar. `context.retrieval_state` y `context.prompt_bytes` no cambian.

#### Escenario: un arranque y una consulta
- GIVEN un contexto sin consulta cortado por el presupuesto y otro con consulta
- WHEN se registran
- THEN `context.cut` tiene `block=recent,state=recent` una vez y ninguna serie `state=searched` para ese bloque

**Tests (hoy fallan):** `a_context_records_its_state_its_size_and_the_blocks_it_cut` (`crates/clawg-cli/tests/api.rs`), que hoy lee `context.cut` sólo por `block`; su `gauge_of` pasa a pedir las dos etiquetas.

## CO12. Con consulta, ACTIVE CONFLICTS sigue

Un contexto con consulta (`build_memory_prompt`) MUST seguir pidiendo los pares con los ids de sus hits y de las entradas de AUTO, y MUST seguir imprimiendo el bloque, su NOTE (`conflict_note`, `prompt_builder.rs:845-859`, que no cambia: ya pide preguntar antes de una decisión que dependa de esas memorias), los grupos `NEW`/`OLD`, el orden y el tope de 5 pares con su `(+N more conflicts not shown)`, salvo por lo que CO2 y CO3 cambian (la marca en los hits y el lugar del bloque en el recorte). El detector, `needs_user`, los veredictos y las respuestas de `resolve_conflict` MUST NOT cambiar.

#### Escenario: una consulta con un par
- GIVEN un par escalado cuyo lado nuevo es un hit de la consulta, y un presupuesto que lo deja entrar
- WHEN se pide el contexto
- THEN el prompt trae el bloque con la NOTE y `NEW #N` / `OLD […]` como hoy, y el hit lleva `conflict=#M`

**Tests:** los que hoy pasan y siguen: `conflict_block_cites_shown_hits_by_id_and_prints_text_only_for_the_rest`, `conflict_block_has_one_note`, `the_conflict_note_names_the_tool`, `conflict_pairs_rank_by_best_hit_position_workspace_only_last` y `e2e_full_4_block_output`, éste con la marca de CO2 en la línea del hit (`prompt_builder.rs`). Pasa un test nuevo del escenario.

## CO13. Lo que no cambia

Ninguno de estos requisitos escribe en `memory.db`: los pares que esperan siguen `escalated`, sus filas nuevas siguen `disputed` y las dos filas se siguen encontrando (`status IN ('active', 'disputed')`, `crates/clawg-core/src/memory/retrieval/search.rs:287`). Volver atrás es instalar la versión anterior, que vuelve a poner los pares en el arranque. `memory_status` sigue contando los escalados. `rag-eval` no ve un cambio de ranking.

#### Escenario: un par que ya esperaba
- GIVEN un par escalado de antes de este cambio
- WHEN se pide el contexto sin consulta, se busca con una consulta que lo toca y se llama a `list_memories` con `conflicts`
- THEN el arranque no lo imprime pero marca sus filas, la búsqueda marca sus dos lados, la lista lo trae, y su estado, el de sus filas y su `disputed_against` son los de antes

**Tests (hoy fallan):** en `crates/clawg-cli/tests/api.rs`, el escenario completo contra un store con un par, que lee la tabla `memory_conflict` y `memory` antes y después.

## Cómo se mide

**Sin pagar:** los tests de arriba, más `cargo test --workspace` y los dos clippy de `.claude/CLAUDE.md` al cerrar la serie. Los dos contratos se regeneran con `CLAWG_BLESS=1` y su diff es el de CO9.

**`rag-eval`:** una corrida `--real --agent-view` con el golden de `docs/eval/` contra la última línea base MUST NOT mover `search.found` ni `context.found` (con menos de 6 casos en un sentido el resultado es «no distinguible», [`docs/reference/testing.md`](../../../docs/reference/testing.md)) y MUST informar cuántos tokens suma la marca en `search_memory` y `recall_context`. Con el presupuesto por defecto ningún caso del golden se corta, así que el orden de CO3 no se ve ahí.

**Con uso real, fijado antes de instalar** (en local, sobre el perfil real y con autorización; no es un test): con la versión instalada una semana, `context.cut` con `block=conflicts` y `state=recent` MUST ser 0, y con `block=last_session` y `state=recent` MUST ser el 2 % o menos de los contextos con `context.retrieval_state{state=recent}`. Hoy son 134 de 250, los dos tipos de contexto juntos. Por encima de ese umbral, lo que corta el resumen es la mitad que se lleva la sección AUTO (SC2) o lo que escribió la persona (SC5), y es otro cambio. Si `context.cut{block=conflicts,state=searched}` sigue alto, queda B2 de la [exploración](exploration.md#b-el-contexto-con-consulta-recall_context) como paso siguiente.
