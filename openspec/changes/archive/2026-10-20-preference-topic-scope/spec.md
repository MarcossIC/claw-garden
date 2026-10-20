# preference-topic-scope — Especificación

## R1. El hit y el save leído dicen el proyecto de la fila

Cada hit de `POST /v1/search` MUST llevar `project_id`, el proyecto de la fila, y MUST mandarlo siempre: `null` para una fila sin proyecto (`RankedHit.project_id`). `GET /v1/memories/{kind}/{id}/save` MUST llevar el de la fila pedida igual (`SaveText.project_id`). El campo no cambia qué filas se leen, su orden ni su puntaje.

El cliente MUST distinguir un `project_id` ausente (un daemon anterior al campo) de uno `null` (una fila sin proyecto) en `Hit` y en `SavedText`.

#### Escenario: un hit con y sin proyecto
- GIVEN una fila guardada en el proyecto `P` y otra sin proyecto
- WHEN se las busca sin proyecto
- THEN el hit de la primera lleva `"project_id":"P"` y el de la segunda `"project_id":null`
- AND el save leído de cada una lleva el mismo `project_id`

**Tests:** `a_search_hit_and_the_save_read_carry_the_project` (`crates/clawg-cli/tests/api.rs`); `a_hit_tells_a_missing_project_from_none` (`client.rs`).

## R2. La clave se muestra sólo donde un save la revisa

Un hit propio de `search_memory` con clave MUST mostrar ` topic=K` sólo cuando el proyecto de la fila es el proyecto donde guardaría un `save_memory` con el mismo `project_path`: el de la búsqueda, o con `all_projects` el que tendría sin él. Una fila sin proyecto MUST contarse como de otro proyecto, salvo que el save tampoco tenga proyecto. Un hit sin `project_id` (de un daemon anterior) MUST mostrar la clave como antes. Un hit de otro agente MUST seguir sin mostrarla.

`read_memory` MUST aplicar la misma regla a su cabecera, comparando con el proyecto del puente, donde guarda un save sin `project_path`.

La decisión MUST ser una función pura del proyecto de la fila y el del save. La fila, su clave y el alcance de la revisión no cambian.

#### Escenario: una preferencia de otro proyecto
- GIVEN una `user_preference` del agente guardada con la clave `style/indent` en el proyecto `A`
- WHEN el agente la encuentra con `search_memory` desde el proyecto `B`
- THEN el hit no muestra `topic=`
- AND buscada con el `project_path` de `A` muestra `topic=style/indent`
- AND un save bajo `style/indent` desde `B` no la reemplaza: las dos filas quedan vivas

#### Escenario: una fila sin proyecto
- GIVEN un hit propio con clave y sin proyecto
- WHEN se lo muestra a un puente con proyecto
- THEN no muestra `topic=`; a un puente sin proyecto, sí

#### Escenario: un daemon anterior
- GIVEN un hit con clave sin el campo `project_id`
- THEN muestra `topic=K` como antes

#### Escenario: read_memory
- GIVEN una fila propia con clave de otro proyecto
- WHEN el agente la lee con `read_memory`
- THEN la cabecera no muestra `topic=`

**Tests:** `a_hit_of_another_project_does_not_show_its_topic_key`, `a_hit_without_a_project_shows_its_key_only_to_a_bridge_without_one`, `a_hit_from_an_older_daemon_shows_its_key` y `format_save_shows_the_key_only_where_a_save_revises_it` (`tools.rs`); `search_memory_shows_a_key_only_in_the_project_it_revises` y `read_memory_shows_a_key_only_in_the_project_it_revises` (`server.rs`).

## R3. El protocolo dice dónde se muestra la clave

`protocol.md` MUST decir, en «How to write a save», que `search_memory` y `read_memory` muestran la clave sólo donde guardar bajo ella reemplaza la memoria, y que una memoria de otro proyecto no la muestra. La apertura que se manda como instrucciones del servidor no cambia.

**Tests:** `the_protocol_says_where_a_key_is_shown` (`server.rs`).
