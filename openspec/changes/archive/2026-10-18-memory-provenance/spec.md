# memory-provenance — Especificación

Requisitos nuevos. Al archivar pasan a `specs/provenance/spec.md`. Términos: el **origen** de una fila son sus columnas `source` y `created_by_type`; su **marca** es lo que un lector deriva de ellas ([diseño D1](design.md#d1-se-marca-lo-inferido-no-lo-dicho)).

## P1. Sello de un save

El insert de un evento `explicit` MUST sellar la fila así:

| `derived_from` | `user_stated` | `source` | `created_by_type` |
|---|---|---|---|
| no vacío | — | `synthesis` | `agent` |
| vacío | `true` | `explicit` | `user` |
| vacío | `false` o ausente | `explicit` | `agent` |

Un evento `chat` o `compaction` MUST sellarse como hoy. Cada pieza de un save largo MUST llevar el mismo origen.

#### Escenario: save sin declaración
- GIVEN un save por `POST /v1/memories` sin `user_stated`
- WHEN se guarda
- THEN la fila queda `explicit` / `agent`

#### Escenario: conclusión
- GIVEN dos memorias guardadas
- WHEN se guarda otra con `derived_from` = sus ids
- THEN la fila queda `synthesis` / `agent`

**Test:** `seal_cases` en `provenance.rs`; `an_undeclared_save_is_the_agents`, `a_declared_save_is_the_users` y `a_conclusion_is_sealed_as_synthesis` en `crates/clawg-cli/tests/api.rs`.

## P2. La declaración

`save_memory` MUST aceptar `user_stated` booleano opcional y su esquema MUST describirlo. `POST /v1/memories` MUST aceptarlo y MUST responder 400, sin escribir en el outbox, si llega `true` junto a un `derived_from` no vacío. Un evento serializado sin el campo MUST leerse como `false`.

**Test:** `user_stated_with_derived_from_is_refused` en `tests/api.rs`; `an_event_without_user_stated_reads_as_undeclared` en `events.rs`; `the_save_schema_names_user_stated` en `server.rs`; el contrato fijado en `agent_contract.json`.

## P3. Marca

`mark(source, created_by_type)` MUST ser pura y devolver `conclusion` para `synthesis`, `imported` para `import`, `inferred` para `explicit` con `agent`, y nada para cualquier otro par de valores, incluido `explicit` con `user`.

**Test:** `mark_cases` en `provenance.rs`.

## P4. Los pares

- `GET /v1/disputes` MUST traer la marca de cada lado de cada par listado.
- `memory_status` MUST nombrar primero, en cada par, un lado sin marca cuando el otro la tiene, y MUST escribir la marca junto al id del lado que la tiene.
- ACTIVE CONFLICTS MUST escribir la marca de cada lado que la tiene, en el rótulo cuando imprime su texto y junto al id cuando lo cita.
- El origen MUST NOT cambiar el estado de un par ni el de sus filas, ni el orden de los pares listados.

#### Escenario: inferencia contra lo dicho
- GIVEN un par pendiente cuya fila nueva es `explicit` / `agent` y la vieja `explicit` / `user`
- WHEN el agente llama a `memory_status`
- THEN la línea es `#<vieja> close to #<nueva> inferred` y el par sigue `pending`

**Test:** `open_pairs_carry_each_sides_mark` en `dispute.rs`; `the_unmarked_side_of_a_pair_goes_first` en `tools.rs`; `the_conflict_block_marks_an_inferred_side` en `prompt_builder.rs`.

## P5. Síntesis

`synthesize` MUST escribir la marca de cada memoria pendiente y de cada conclusión que la tiene.

**Test:** `the_synthesis_marks_each_rows_origin` en `tools.rs`.

## P6. Filas anteriores

Una base con filas `explicit` / `user` escritas por la versión anterior MUST abrir con todas sus filas, sin cambiar su origen, y ninguna MUST llevar marca.

**Test:** `rows_saved_before_provenance_keep_their_origin_and_no_mark` en `provenance.rs`.
