# domain-types — Especificación

Son requisitos sobre [`specs/storage`](../../specs/storage/spec.md) y [`specs/retrieval`](../../specs/retrieval/spec.md), y se consolidan al archivar. Lo que no se nombra acá sigue como está.

Términos:

- **Un nombre conocido:** uno de los siete de `MemoryType::ALL` para `memory_type`, o uno de los tres de `MemoryStatus::ALL` (`active`, `superseded`, `disputed`) para `status`.
- **Un nombre desconocido:** cualquier otro texto de esas columnas, escrito por otra versión o por un import. `observation`, el `DEFAULT` de `memory_type` en `V1__init.sql`, es uno.
- **Leer:** sacar el valor de una columna de `memory`, de una fila del outbox o de un diario, o de un cuerpo JSON que mandó un daemon.

## R1. Leer un nombre desconocido no pierde la fila ni su texto

Leer `memory_type` o `status` MUST dar un `Stored<T>`: `Known(T)` para un nombre conocido y `Other(texto)` para cualquier otro, sin fallar. `as_str` MUST devolver el texto leído. Un valor que no es texto MUST fallar como falla hoy el `String` (`InvalidColumnType`).

#### Escenario: un nombre desconocido
- GIVEN una columna con `observation`
- WHEN se lee como `StoredType`
- THEN es `Other("observation")`, `as_str()` es `"observation"` y `known()` es `None`

**Tests** (`crates/clawg-core/src/memory/domain/record.rs`): `a_stored_name_the_code_knows_reads_as_its_variant`, `a_stored_name_the_code_does_not_know_keeps_its_text`, `a_stored_value_round_trips_through_sqlite_unchanged` y `a_stored_value_that_is_not_text_is_a_type_error`. Hoy no compilan: los tipos no existen.

## R2. Una fila de nombre desconocido sigue apareciendo donde hoy aparece

- Una búsqueda MUST devolver una fila de `memory_type` desconocido y `status` `active` o `disputed`, con su texto y un score mayor que 0.
- Una búsqueda MUST NOT devolver una fila de `status` desconocido, como hoy (`status IN ('active', 'disputed')`).
- Un listado MUST devolver las filas de `memory_type` o `status` desconocido, con su texto, y un filtro por ese texto MUST devolverlas.
- Un evento del outbox con un `memory_type` desconocido MUST leerse, y su fila MUST guardarse como `semantic_fact`, como hoy.

#### Escenario: búsqueda
- GIVEN tres filas con el vector de la consulta: `observation`/`active`, `observation`/`disputed` y `semantic_fact`/`archived`
- WHEN se busca
- THEN vuelven las dos primeras, con `memory_type` `observation`, y la tercera no

#### Escenario: listado
- GIVEN una fila `observation`/`active` y una `semantic_fact`/`archived`
- WHEN se lista sin filtro, por `memory_type` `observation` y por `status` `archived`
- THEN la primera lista ambas, la más nueva primero, y cada filtro la suya

**Tests:** `a_row_of_a_type_the_code_does_not_know_is_still_found` (`search.rs`), `rows_of_a_type_or_status_the_code_does_not_know_stay_listed` (`listing.rs`), `an_event_of_a_type_the_code_does_not_know_parses_and_keeps_it` (`events.rs`) y el ya existente `explicit_source_with_an_unknown_type_override_falls_back_to_semantic_fact` (`store_insert.rs`). Los tres primeros no compilan hoy.

## R3. Lo que se escribe y se serializa no cambia

`Stored<T>` MUST escribirse en SQLite y serializarse en JSON como su texto, sea `Known` u `Other`; su `Debug` MUST ser el del texto, como el del `String` de hoy. Ningún cuerpo de respuesta, mensaje de error ni contrato del agente MUST cambiar.

#### Escenario
- GIVEN `Known(UserPreference)`, `Other("observation")` y `Other("archived")`
- WHEN se serializan en un objeto JSON
- THEN el objeto es `{"known":"user_preference","other":"observation","status":"archived"}`

**Tests:** `a_stored_value_serializes_as_its_bare_text` y `a_stored_value_compares_by_its_text` (`record.rs`), la parte JSON de los dos tests de R2, y `crates/clawg-cli/tests/api.rs` y los dos contratos del agente sin editar ni `CLAWG_BLESS`.

## R4. Las decisiones se toman sobre el enum

Cada decisión sobre un tipo o un estado leído MUST hacerse con un `match` o una comparación con `MemoryType` o `MemoryStatus`, y un nombre desconocido MUST tomar el camino que tomaba el comodín de hoy:

| Decisión | Con un nombre desconocido |
|---|---|
| Puerta de estado del ranking (`score::score`) | no rankea (0) |
| Tasa de decaimiento (`score::lambda_for`) | la de reserva |
| Tier (`compute_tier`, `retier`, la función de SQL `compute_tier`) | el que tiene |
| Marca de una preferencia (`provenance::preference_mark`) | ninguna |
| Tipo protegido (PATCH, movimiento, disputa, borrado de proyecto, import) | no protegido |
| Fila viva (`StoredStatus::is_live`: revival, import, translate) | no viva |
| Fuente de una conclusión (`SourceState::of`) | viva |

`MemoryStatus::is_live` MUST ser verdadero sólo para `Active` y `Disputed`, lo mismo que admite `status IN ('active', 'disputed')`.

#### Escenario: tier
- GIVEN `compute_tier(3, 5.0, 9, 'observation')` en SQL
- WHEN se evalúa
- THEN da 3

**Tests:** `every_status_has_its_stored_name`, `only_an_active_or_disputed_row_is_live` (`record.rs`), `a_status_the_code_does_not_know_never_ranks` y `lambda_for_memory_dispatches_on_memory_type` (`score.rs`), `the_tier_udf_keeps_the_tier_of_a_type_it_does_not_know` (`tier.rs`) y `source_state_cases` (`synthesis.rs`), con un caso `archived`.

## R5. Lo que entra por la API sigue siendo estricto

- `POST /v1/memories` y `PATCH /v1/memories/{kind}/{id}` MUST rechazar un `memory_type` que no es un nombre conocido, con el texto de hoy.
- `GET /v1/memories?status=` MUST rechazar un estado que no es un nombre conocido con `unknown status "<valor>": accepted values are "active", "disputed" and "superseded"`, como hoy, ahora con `MemoryStatus::from_name`.
- El filtro `memory_type` de `GET /v1/memories` y de `POST /v1/memories/move` MUST aceptar cualquier texto, como hoy, para que las filas de un tipo desconocido se puedan listar y mover.

**Tests:** los de hoy en `crates/clawg-cli/src/daemon/api/memories.rs` y `crates/clawg-cli/tests/api.rs`, sin editar sus literales.

## R6. Cada nombre que compara el SQL es conocido

Los literales de `status` y `memory_type` MUST seguir escritos en el texto del SQL. Cada literal que el código de producción o una migración compara con esas columnas (`=`, `<>`, `!=`, `IN (...)`) MUST ser un nombre conocido.

**Tests** (`crates/clawg-cli/tests/conventions.rs`): `compared_names_reads_the_literals_a_line_compares` y `every_status_and_type_the_sql_compares_is_one_the_code_knows`.
