# update — Especificación

Fuente de verdad del comportamiento actual al corregir una memoria ya guardada: qué campos se pueden cambiar, cómo se valida el cambio, qué tier queda y qué exponen el daemon y el puente. El contenido no se cambia: para eso se guarda el texto nuevo y se borra el viejo, o se lo guarda bajo la misma `topic_key`, que reemplaza al anterior y lo deja en la historia ([`specs/ingest`](../ingest/spec.md), «Un save con clave reemplaza la fila viva de su clave»). Qué filas puede cambiar cada agente está en [`specs/visibility`](../visibility/spec.md) (R3 y R4).

**Origen:** [memory-update-tool](../../changes/archive/2026-10-14-memory-update-tool/proposal.md), archivado el 2026-10-14. Los ids de requisito son los de esa spec. [memory-protocol](../../changes/archive/2026-10-17-memory-protocol/proposal.md), archivado el 2026-10-17, sumó que el save y el cambio rechazan un tipo con el mismo mensaje y que el schema de `update_memory` declara los tipos.

## Definiciones

- **Cambio:** un pedido con `importance`, `memory_type` o los dos, sobre una fila `(kind, id)`.
- **Tipo efectivo de una fila:** su `memory_type` en `memory`; `'episodic_trajectory'` en `episodic_trajectory` y `'procedural_skill'` en `procedural_skill`, los mismos literales que usa el insert (`UPDATE_READ_*`, `crates/clawg-core/src/memory/admin.rs:190-195`).
- **Tier ganado** con una importancia `i` y un tipo efectivo `t`: `compute_tier(3, i, access_count, t)` (`crates/clawg-core/src/memory/tier.rs:22`), el tier que las reglas le dan a la fila sin contar nada más.
- **Escala de importancia:** de 0 a 10 inclusive, la misma del save.
- **Tipos conocidos:** `semantic_fact`, `user_preference`, `architectural_decision`, `session_summary`, `code_block`, `chat_message`, `tool_error` (`MemoryType::ALL`, `crates/clawg-core/src/memory/store_insert.rs:34`).

## R1. Qué cambio es válido

Un cambio MUST rechazarse, sin tocar la fila, cuando:

- no trae ni `importance` ni `memory_type`;
- `importance` no es un número finito dentro de la escala;
- `memory_type` no es un tipo conocido, con el mismo mensaje que da el save ([`specs/ingest`](../ingest/spec.md), «El tipo de memoria se valida al guardar»: `PatchError::UnknownType` envuelve `UnknownMemoryType`);
- trae `memory_type` y `kind` no es `memory`.

La validación es pura: `MemoryPatch::new` (`admin.rs:161`) devuelve `MemoryPatch` o un `PatchError` (`admin.rs:138`, `thiserror`) cuyo mensaje nombra el campo. A diferencia del save, una importancia fuera de escala MUST rechazarse, no recortarse: el save recorta lo que el agente estima al pasar, un cambio es una corrección explícita y recortarla en silencio diría "hecho" sobre un valor que nadie pidió.

#### Escenario: fuera de escala
- GIVEN una fila de `memory` con importancia 5
- WHEN se pide un cambio con `importance = 50`
- THEN se rechaza con un mensaje que nombra `importance` y la fila sigue con 5

#### Escenario: tipo en una tabla sin tipo
- GIVEN una fila de `procedural_skill`
- WHEN se pide un cambio con `memory_type = "user_preference"`
- THEN se rechaza con un mensaje que nombra `memory_type`

**Test:** `a_change_needs_at_least_one_field`, `importance_outside_the_scale_is_refused`, `an_unknown_memory_type_is_refused` y `only_memory_rows_have_a_type_to_change` en `crates/clawg-core/src/memory/admin.rs`.

## R2. Qué tier queda

Con el tier actual `c`, el tier ganado antes del cambio `a` y el tier ganado después `d` (tiers más bajos son mejores):

- si `d <= a` (el cambio no quitó tier), MUST quedar `compute_tier(c, importancia nueva, access_count, tipo nuevo)`: la promoción de siempre;
- si `d > a` y `c >= a`, MUST quedar `max(c, d)`: baja hasta lo que la fila gana sin el campo cambiado, y nunca le devuelve lo que ya había perdido por decaimiento;
- si `d > a` y `c < a`, MUST quedar `c`: ese tier no lo daban las reglas (vino de otra fuente, como la app de escritorio), así que el cambio no lo quitó.

La regla es la función pura `retier` (`tier.rs:115`), separada de `compute_tier`: `compute_tier` sigue siendo sólo de subida, y el decaimiento (`demote_inactive`) sigue siendo la única baja automática. Ésta es la única baja que pide alguien.

El workspace muestra `user_preference` y `architectural_decision` sólo con importancia ≥ 8 **y** tier 1 (`crates/clawg-core/src/memory/workspace.rs:255-292`). Por eso una decisión guardada con 9,5 y bajada a 8,5 sale del workspace por el tier (queda en tier 2), aunque sigue pasando el filtro de importancia.

#### Escenario: la importancia era lo que daba el tier
- GIVEN un `semantic_fact` en tier 1 con importancia 9,5 y `access_count` 1
- WHEN se baja la importancia a 2
- THEN queda en tier 3

#### Escenario: el tier lo ganó por accesos
- GIVEN un `architectural_decision` en tier 1 con importancia 9,5 y `access_count` 3
- WHEN se baja la importancia a 2
- THEN sigue en tier 1

#### Escenario: baja sólo hasta lo que el resto todavía gana
- GIVEN un `architectural_decision` en tier 1 con importancia 9,5 y `access_count` 1
- WHEN se baja la importancia a 2
- THEN queda en tier 2

#### Escenario: el tipo era lo que daba el tier
- GIVEN un `user_preference` en tier 1 con importancia 5
- WHEN se cambia el tipo a `semantic_fact`
- THEN queda en tier 3

#### Escenario: un tier de otra fuente
- GIVEN un `semantic_fact` en tier 1 con importancia 5 (como los que dejaba la app de escritorio)
- WHEN se baja la importancia a 3, o se cambia el tipo a `chat_message`
- THEN sigue en tier 1

#### Escenario: subir sigue promoviendo
- GIVEN un `semantic_fact` en tier 3 con importancia 5
- WHEN se sube la importancia a 9, o se cambia el tipo a `user_preference`
- THEN queda en tier 1

**Test:** `lowering_the_importance_that_gave_tier_1_takes_it_away`, `lowering_to_a_value_that_still_earns_the_tier_keeps_it`, `a_tier_earned_by_access_survives_lowering_the_importance`, `a_lost_tier_falls_only_to_what_the_rest_still_earns`, `a_type_that_gave_tier_1_takes_it_with_it`, `a_tier_the_rules_never_gave_survives_any_change`, `a_tier_lost_to_inactivity_is_not_given_back_by_lowering` y `raising_still_promotes` en `crates/clawg-core/src/memory/tier.rs`.

## R3. Qué escribe un cambio

Un cambio válido sobre una fila viva (`deleted_at IS NULL`) MUST, en una sola transacción (`update_memory_core`, `admin.rs:210`):

- escribir los campos pedidos y dejar igual los que no;
- escribir el tier de R2;
- poner `last_modified_at` en ahora y sumar 1 a `update_count`;
- devolver la fila como quedó (`UpdatedRow`, `admin.rs:181`): `id`, `kind`, `importance`, `memory_type` (sólo en `memory`) y `tier`.

MUST NOT cambiar `status`, `access_count`, `last_accessed_at`, `superseded_by`, las disputas, la procedencia, el contenido ni el vector.

Sobre una fila que no existe, borrada o de otro agente cuando el cambio tiene scope, MUST devolver "no existe" (`None`) sin escribir nada, tampoco `last_modified_at` ni `update_count` (la regla de [`specs/visibility`](../visibility/spec.md) R4). Vale igual para las tres tablas.

#### Escenario: cambiar los dos campos
- GIVEN un `semantic_fact` con importancia 5, `update_count` 0 y `status = 'disputed'`
- WHEN se cambia a `user_preference` con importancia 8
- THEN la fila queda `user_preference`, 8, tier 1, `update_count` 1, `last_modified_at` nuevo y sigue `disputed`

**Test:** `an_update_writes_what_was_asked_and_counts_itself`, `an_update_leaves_status_accesses_and_disputes_alone`, `lowering_importance_takes_away_the_tier_it_gave`, `update_with_high_importance_promotes_to_tier_1`, `update_on_episodic_has_no_type_rule_below_importance_9`, `update_on_procedural_keeps_the_tier_its_table_gives`, `update_on_a_missing_id_changes_nothing`, `update_on_a_soft_deleted_row_changes_nothing` y `an_owner_mismatch_changes_no_importance` en `admin.rs`.

## R4. HTTP

`PATCH /v1/memories/{kind}/{id}` (`update_memory`, `crates/clawg-cli/src/daemon/api.rs:1244`) MUST aceptar el cuerpo `{importance?, memory_type?}` (`UpdateRequest`, `api.rs:454`) y la query `agent` de siempre.

- Un cambio inválido (R1) MUST responder 400 con el mensaje de la validación.
- Un campo desconocido en el cuerpo MUST responder 400 o 422, nunca 200: un campo mal escrito no puede pasar por un cambio hecho (`deny_unknown_fields`).
- "No existe" (R3) MUST responder 404.
- Un cambio hecho MUST responder 200 con `updated = 1` y `importance`, `memory_type` y `tier` de la fila como quedó.

`DaemonClient::update` (`crates/clawg-cli/src/client.rs:880`) es el lado del cliente.

#### Escenario
- GIVEN una memoria guardada por HTTP
- WHEN se manda `PATCH` con `{"importance": 8, "memory_type": "user_preference"}`
- THEN responde 200 con `updated = 1`, `importance = 8`, `memory_type = "user_preference"` y `tier = 1`, y `GET` muestra lo mismo con `update_count = 1`
- WHEN se manda `{"importance": 11}`, `{}` o `{"importanse": 3}`
- THEN no responde 200 y la fila no cambia

**Test:** `patch_changes_importance_and_type_and_reports_the_row`, `an_invalid_patch_changes_nothing`, `patch_importance_updates_then_404s_on_a_missing_row`, `patch_with_another_agent_is_404_and_keeps_the_importance` y `an_unknown_kind_is_a_400_everywhere_it_appears` en `crates/clawg-cli/tests/api.rs`.

## R5. Herramienta MCP `update_memory`

El puente MUST exponer `update_memory` con `kind`, `id`, `importance` (opcional) y `memory_type` (opcional; su schema declara los tipos conocidos como `enum`, ver [`specs/mcp`](../mcp/spec.md), «El schema de `memory_type` declara los tipos») (`UpdateMemoryArgs`, `crates/clawg-cli/src/mcp/server.rs:305`), y MUST mandar siempre el agente que llama (`update_memory_impl`, `server.rs:883`).

- Sobre una fila propia, MUST devolver un texto que nombra el id y cómo quedaron importancia, tipo y tier (`format_update`, `crates/clawg-cli/src/mcp/tools.rs:196`): `updated #12 memory: user_preference, importance 8.0, tier 1`.
- Sobre una fila de otro agente, MUST fallar como si el id no existiera ([`specs/visibility`](../visibility/spec.md) R3), y la fila MUST NOT cambiar.
- Un cambio inválido MUST fallar con el mensaje de la validación.
- La descripción (`server.rs:1137-1142`) MUST decir que es para cuando el usuario corrige la importancia o el tipo, y que para cambiar el contenido se guarda el nuevo y se borra el viejo.

`save_memory` MUST nombrar `update_memory`: en su descripción, junto a `delete_memory` (`server.rs:1017-1025`), y en la respuesta de un refuerzo, que ya dice el id (`save_outcome_text`, `server.rs:570-574`).

`tools/list` MUST listar `update_memory` entre sus herramientas; el conteo vigente, doce, y la lista completa están en [`specs/mcp`](../mcp/spec.md).

#### Escenario: lo propio
- GIVEN una memoria del agente A con importancia 5
- WHEN A llama `update_memory` con su id e `importance = 2`
- THEN la respuesta nombra el id, importancia 2 y el tier, y `list_memories` la muestra con `imp=2.0`

#### Escenario: lo ajeno
- GIVEN una memoria del agente A
- WHEN B llama `update_memory` con su id
- THEN la respuesta es un error y la fila no cambia

**Test:** `update_memory_changes_an_own_memory_and_says_how_it_ended` y `an_agent_cannot_update_another_agents_memory` en `crates/clawg-cli/src/mcp/server.rs`; `format_update_names_the_fields_and_the_tier` en `crates/clawg-cli/src/mcp/tools.rs`; el refuerzo en `save_outcome_text_covers_every_daemon_answer` (`server.rs`); los conteos de `every_tool_is_registered_under_its_wire_name` (`server.rs`), `a_bare_probe_and_its_cancellation_leave_the_legacy_handshake_working` y `a_session_without_initialize_lists_the_tools_in_name_order` (`crates/clawg-cli/src/mcp/handshake.rs`).

## R6. Datos de la versión anterior

Filas sembradas con SQL directo como las dejaba la versión anterior (una de `memory` por cada combinación de R2) y como las escribía la app de escritorio (`episodic_trajectory` y `procedural_skill`, con tiers que ninguna regla da) MUST poder cambiarse, y su tier MUST seguir R2.

**Test:** `rows_from_the_previous_version_can_be_updated` en `admin.rs`.
