# preference-model — Especificación

Cambia R5 de [`specs/sharing`](../../../specs/sharing/spec.md) («Lo que sigue siendo de cada agente, y lo que no») y la línea de [`specs/retrieval`](../../../specs/retrieval/spec.md) que dice que el workspace ve sólo lo del agente. Suma a `specs/retrieval` qué preferencias entran al workspace y la marca en la línea de un hit, a [`specs/provenance`](../../../specs/provenance/spec.md) dónde se muestra la marca, y a [`specs/mcp`](../../../specs/mcp/spec.md) `all_projects` de `save_memory` y lo que dice el protocolo. Al archivar, estos requisitos pasan a esas specs.

## Definiciones

- **Preferencia elegible:** una fila de `memory` con `memory_type = 'user_preference'`, importancia 8 o más, tier 1, sin `deleted_at` y con estado distinto de `superseded`. Es lo que la consulta ya pide hoy (`fetch_auto_rows`, `crates/clawg-core/src/memory/workspace.rs:376-385`), menos el filtro por agente.
- **Declarada:** una fila con `created_by_type = 'user'`. La deja así un save con `user_stated` (`seal`, `crates/clawg-core/src/memory/provenance.rs:15`) y cualquier fila guardada antes del sello ([`specs/provenance`](../../../specs/provenance/spec.md), P6).
- **Alcance del lector:** el `Reach` que el daemon resuelve para el agente que pide el contexto desde `sharing.json` (`reach`, `crates/clawg-cli/src/daemon/api.rs:1557`), el mismo de la búsqueda ([`specs/sharing`](../../../specs/sharing/spec.md), R1 y R2).
- **Marca:** lo que devuelve `mark(source, created_by_type)` (`provenance.rs:37`): `inferred`, `imported`, `conclusion` o ninguna.

## PM1. El proyecto de una preferencia

Con un proyecto P, la sección AUTO del workspace MUST imprimir una preferencia elegible sólo si su `project_id` es P o es NULL. Sin proyecto, MUST imprimir las de todo proyecto, como hace con las decisiones y los skills. Las decisiones y los skills MUST NOT cambiar.

#### Escenario: tres proyectos
- GIVEN tres preferencias declaradas de importancia 9 y tier 1 del agente: una de P1, una de P2 y una sin proyecto
- WHEN se regenera el workspace de P1
- THEN imprime la de P1 y la sin proyecto, y no la de P2
- WHEN se regenera el workspace sin proyecto
- THEN imprime las tres

**Tests:** `a_preference_of_another_project_stays_out_of_the_workspace`, que reemplaza a `workspace_preferences_cross_project_boundaries`, y `an_unscoped_workspace_takes_the_preferences_of_every_project` (`crates/clawg-core/src/memory/workspace.rs`). Hoy el primero falla: la de P2 entra.

## PM2. Sólo entra sola una preferencia declarada

La sección AUTO MUST imprimir una preferencia elegible sólo si es declarada. Una con `created_by_type` `agent` o `system` MUST NOT entrar, cualquiera sea su importancia o su `source`. MUST seguir alcanzándose con la búsqueda (PM7).

#### Escenario: inferida y declarada
- GIVEN dos preferencias de importancia 9 y tier 1 del agente: una `explicit` / `agent` y una `explicit` / `user`, y una conclusión `synthesis` / `agent`
- WHEN se regenera el workspace
- THEN imprime sólo la `explicit` / `user`

**Test:** `only_a_preference_the_user_stated_enters_the_workspace` (`workspace.rs`). Hoy falla: entran las tres.

## PM3. Las preferencias siguen el alcance del lector

`regenerate_auto_section` MUST recibir el alcance del lector, y la consulta de las preferencias MUST usar el predicado de alcance de la búsqueda (`reach_predicate!`, `crates/clawg-core/src/memory/search.rs:78`): las del lector, las sin autor y, con `AllBut`, las de cada autor que no está oculto. Las decisiones y los skills MUST seguir siendo sólo del agente del pedido. La línea de una preferencia de otro agente MUST ser la misma que la de una propia. El daemon MUST pasar en `POST /v1/context`, con consulta o sin ella, el alcance que resuelve para su búsqueda, leído en cada llamada.

#### Escenario: lo que dijo la persona a otro agente
- GIVEN una preferencia declarada de `claude-code` y una decisión de importancia 9 de `claude-code`, las dos en tier 1
- WHEN se regenera el workspace de `cursor` con el alcance `AllBut([])`
- THEN imprime la preferencia, sin autor, y no la decisión
- WHEN se regenera con `AllBut(["claude-code"])` o con `Own`
- THEN no imprime ninguna de las dos

#### Escenario: por el daemon
- GIVEN un daemon sin `sharing.json` y una preferencia declarada de importancia 9 guardada por `claude-code` en el proyecto P
- WHEN `cursor` pide el contexto de P sin consulta
- THEN el bloque WORKSPACE trae la preferencia
- WHEN se escribe `{"version":1,"hides":{"cursor":["claude-code"]}}` y `cursor` lo pide de nuevo
- THEN el bloque WORKSPACE no la trae

**Tests:** `the_workspace_takes_the_preferences_the_reader_reaches` (`workspace.rs`) y `a_stated_preference_reaches_the_session_start_of_another_agent` (`crates/clawg-cli/tests/api.rs`). Hoy el primero no compila, porque `regenerate_auto_section` no recibe el alcance, y el segundo falla: la preferencia de `claude-code` no entra al workspace de `cursor`. `the_workspace_stays_own_and_conflicts_are_shared` (`crates/clawg-core/src/memory/prompt_builder.rs`) pasa a fijar con una decisión que el workspace no mezcla lo que sigue siendo del agente.

## PM4. `save_memory` con `all_projects`

`save_memory` MUST tomar `all_projects`, un booleano opcional (`SaveMemoryArgs`, `crates/clawg-cli/src/mcp/server.rs`). Con `true`, el save MUST llegar al daemon sin `project_id`. Con `project` o `project_path`, MUST negarse sin llamar al daemon, con una frase que nombra los argumentos, en el idioma de la sesión; `new_project` ya se niega sin `project`. Su descripción MUST decir que guarda la memoria sin proyecto, así la lee cada proyecto, y que es para lo que vale en todo proyecto, como una preferencia que la persona tiene en todo lo que hace. Ausente o `false`, el save MUST hacer lo de hoy.

#### Escenario: una preferencia general
- GIVEN un puente en el proyecto P
- WHEN el agente llama a `save_memory` con `memory_type: "user_preference"` y `all_projects: true`
- THEN la fila guardada tiene `project_id` NULL

#### Escenario: dos lugares
- WHEN el agente llama a `save_memory` con `all_projects: true` y `project`, o con `all_projects: true` y `project_path`
- THEN la respuesta es un error que nombra los dos argumentos, y no se guarda nada

**Tests:** `a_save_for_every_project_has_no_project` y `all_projects_goes_without_a_project` (`server.rs`); `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, bendecidos. Hoy no compilan: `save_memory` no tiene `all_projects`.

## PM5. Una preferencia que no declaró la persona se marca al buscarla

Un hit de `memory` cuyo `memory_type` es `user_preference` MUST llevar su marca en `RankedHit.origin`, y cualquier otro hit MUST llevar `None`. `POST /v1/search` MUST mandarla como `origin`, y omitirla cuando no hay. El cliente del puente MUST leer un hit sin `origin`, el de un daemon anterior, como sin marca (`Hit.origin`, `crates/clawg-cli/src/client.rs`).

La línea del hit MUST llevar la marca después del tipo, separada por un espacio: `[AAAA-MM-DD #id user_preference inferred] contenido` en `recall_context` (`format_hit_line`, `crates/clawg-core/src/memory/prompt_builder.rs:851`) y `N. #id [Memory/user_preference/t1 inferred score=S] contenido` en `search_memory` (`format_hits`, `crates/clawg-cli/src/mcp/tools.rs:192`), antes de `piece=`, `score=`, `topic=` y `by`. Vale también para RECENT del contexto sin consulta, que lee `source` y `created_by_type` en su propia consulta e imprime la misma línea. Una línea sin marca MUST quedar byte a byte como hoy.

#### Escenario: tres hits
- GIVEN una preferencia `explicit` / `agent`, una preferencia `explicit` / `user` y un `semantic_fact` `explicit` / `agent`, cerca de la consulta
- WHEN se busca
- THEN el hit de la primera trae `origin` `inferred`, y los otros dos no traen nada
- AND en `recall_context` y en `search_memory` sólo la línea de la primera lleva ` inferred` después del tipo

**Tests:** `a_preference_hit_carries_its_origin` (`crates/clawg-core/src/memory/search.rs`), `a_preference_hit_line_shows_its_origin` y `a_recent_preference_shows_its_origin` (`prompt_builder.rs`), `format_hits_shows_the_origin_of_a_preference` (`tools.rs`) y `a_hit_from_an_old_daemon_has_no_origin` (`client.rs`). Hoy no compilan: el hit no tiene origen.

## PM6. El protocolo dice qué preferencias entran solas

`protocol.md`, en «Type and importance», y `protocol.es.md`, en «Tipo e importancia», MUST decir que una `user_preference` con 8 o más, guardada con `user_stated`, se imprime arriba de cada `recall_context` de su proyecto, o de todos si se guardó con `all_projects`, para cualquier agente que la lea, y que una búsqueda marca la que no declaró la persona. La línea de `user_preference` de la lista de tipos MUST decir cuándo usar `all_projects`. La apertura de los dos MUST NOT cambiar.

**Tests:** `the_protocol_says_which_preferences_open_every_session` (`crates/clawg-cli/src/mcp/protocol.rs`), en los dos idiomas; `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one`; `the_instructions_fit_older_clients` y los dos contratos, que no cambian en `instructions`. Hoy el primero falla: el protocolo dice «in every project».

## PM7. Lo que sale del workspace se sigue alcanzando

Una base con filas escritas como las escribe la versión anterior MUST abrirse sin migrar ni reescribir nada. Una preferencia que por PM1 o PM2 deja de entrar al workspace MUST seguir encontrándose con la búsqueda desde el proyecto, con su marca, y sus columnas MUST quedar como estaban. Una fila guardada antes del sello (`explicit` / `user`) del proyecto MUST seguir entrando.

#### Escenario: filas de antes
- GIVEN, sembradas con SQL, tres preferencias de importancia 9 y tier 1: una `explicit` / `agent` de P1, una `explicit` / `user` de P2 y una `explicit` / `user` de P1
- WHEN se regenera el workspace de P1 y se pide el contexto de P1 con una consulta cercana a las tres
- THEN el workspace imprime sólo la tercera, y las otras dos son hits, la primera marcada `inferred`
- AND las tres conservan su `agent_id`, `project_id`, `source`, `created_by_type`, importancia, tier, estado y contenido

**Test:** `a_preference_out_of_the_workspace_is_still_found` (`prompt_builder.rs`). Hoy falla: las tres entran al workspace y ninguna es un hit.

## PM8. Una preferencia sin proyecto se reemplaza con `all_projects`

Cambia «La clave de tema en las herramientas» de [`specs/mcp`](../../../specs/mcp/spec.md), que cuenta una fila sin proyecto como de otro proyecto. Desde un puente con proyecto, `search_memory` y `read_memory` MUST mostrar la clave de una `user_preference` sin proyecto como ` topic=K all_projects`, en el lugar de ` topic=K`, porque un save con `all_projects` y esa clave la revisa ([`specs/ingest`](../../../specs/ingest/spec.md), «Alcance de una clave»). Cualquier otra fila sin proyecto MUST seguir sin mostrarla, y un puente sin proyecto MUST seguir viendo ` topic=K`. `GET /v1/memories/{kind}/{id}/save` MUST mandar `memory_type` en una fila que lo tiene; una respuesta sin él, de un daemon anterior, MUST leerse como hasta ahora. `protocol.md` y `protocol.es.md` MUST decir, en «How to write a save» y en «Cómo escribir un save», cómo se reemplaza esa preferencia.

#### Escenario: reemplazar una preferencia general
- GIVEN una `user_preference` con la clave `style/indent` guardada con `all_projects`, y un puente en el proyecto P
- WHEN el agente la encuentra con `search_memory` o la lee con `read_memory`
- THEN ve `topic=style/indent all_projects`
- AND un save con `all_projects` y esa clave responde que la reemplaza, y la fila nueva tampoco tiene proyecto

**Tests:** `a_hit_without_a_project_shows_how_a_save_revises_it`, que reemplaza a `a_hit_without_a_project_shows_its_key_only_to_a_bridge_without_one`, `a_marked_key_stays_before_the_author_of_the_hit` y `format_save_shows_the_key_only_where_a_save_revises_it` (`crates/clawg-cli/src/mcp/tools.rs`); `a_preference_of_every_project_is_replaced_with_all_projects_and_the_key_shown` y `the_protocol_says_where_a_key_is_shown` (`crates/clawg-cli/src/mcp/server.rs`).

## PM9. Un save declarado sella la fila que repite

Un save con `user_stated` cuyo texto ya está vivo en su alcance MUST dejar esa fila con `source = 'explicit'` y `created_by_type = 'user'`, además de reforzarla (`reinforce_in`, `crates/clawg-core/src/memory/store_insert.rs`), sea que la encuentre antes de calcular el embedding o después. También MUST subirla a la importancia del save, sin bajarla nunca, y recalcular su tier con ella: si no, lo que dijo la persona quedaría fuera del workspace por haberse inferido antes con menos importancia. Un save sin `user_stated` MUST dejar el sello y la importancia como estaban. Una fila `import` MUST conservar los dos, porque un segundo import y `clawg doctor` la encuentran por su `source`, y una `synthesis` también, porque una conclusión es del agente aunque se apoye en lo que dijo la persona.

#### Escenario: lo inferido, y después declarado
- GIVEN una preferencia guardada sin `user_stated`, que queda `explicit` / `agent` con importancia 5
- WHEN la persona dice lo mismo y el agente guarda el mismo texto con `user_stated` e importancia 9
- THEN sigue habiendo una fila, es `explicit` / `user` con importancia 9, y entra al workspace
- AND otro save declarado del mismo texto con importancia 2 la deja en 9

**Tests:** `a_stated_save_seals_the_row_it_repeats_as_the_users`, `an_inferred_save_leaves_the_seal_of_the_row_it_repeats`, `a_stated_save_leaves_an_import_and_a_conclusion_as_they_are`, `a_stated_save_raises_the_importance_of_the_row_it_repeats_and_never_lowers_it`, `an_inferred_save_leaves_the_importance_of_the_row_it_repeats`, `a_preference_stated_after_it_was_inferred_enters_the_workspace` y `a_twin_restored_while_the_save_is_embedded_is_reinforced_not_copied` (`crates/clawg-core/src/memory/worker.rs`).
