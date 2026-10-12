# retrieval-hygiene — Especificación

**Dominio destino:** `specs/retrieval/spec.md` (se fusiona al archivar).

**Alcance:** el orden entre empates de `search_all_memories`, el corte relativo de `POST /v1/search` y qué hits registran un acceso en `search` y en `recall_context`. Ningún score cambia, y el esquema tampoco.

## Definiciones

- **Corte relativo con ratio r:** sobre hits ordenados mejor primero, se conservan los primeros mientras la posición sea < 2 o `score >= r × mejor` (`crates/clawg-cli/src/mcp/tools.rs:65-76`). Es la regla actual del puente, sin cambios.
- **Acceso:** una entrada al access tracker, que en el flush suma `access_count`, mueve `last_accessed_at` y recalcula el tier (`crates/clawg-core/src/memory/access.rs:156-163`).
- **Hit contable:** hit de `kind = memory` con `score > 0`. Sólo los `memory` cuentan por la razón de `api.rs:492-495` (ids por tabla).

## R1. Desempate por distancia

Dos hits con el mismo score MUST ordenarse por `distance` ascendente. Si también empatan en distancia, MUST seguir valiendo `(kind, id)` ascendente, de modo que el orden sea total y determinista. El score de ningún hit MUST cambiar.

#### Escenario: ceros
- GIVEN dos hits `memory` con score 0: id 1 con distancia 0,70 e id 9 con distancia 0,50
- WHEN se ordenan
- THEN el id 9 va primero

#### Escenario: empate total
- GIVEN dos hits con el mismo score y la misma distancia, ids 3 y 5
- THEN el 3 va primero, como hoy

#### Escenario: el score manda
- GIVEN un hit con score 0,2 y distancia 0,9, y otro con score 0,1 y distancia 0,1
- THEN el de score 0,2 va primero

**Test:** `hit_cmp_breaks_score_ties_by_distance` en `search.rs` (mod tests). Falla hoy: `hit_cmp` pasa de score a `(kind, id)` (`search.rs:78-84`). El test existente `hit_cmp_is_a_strict_total_order_even_on_score_ties` (`search.rs:1164`) se mantiene: sus fixtures usan distancia 0 en todos los hits.

**Test de integración:** `ties_at_zero_follow_the_closest_row_not_the_oldest` en `search.rs`: siembra una fila vieja lejana y una nueva cercana, las dos bajo el piso, y la nueva sale primero. Falla hoy.

## R2. Corte relativo opcional en `/v1/search`

`POST /v1/search` MUST aceptar el campo opcional `min_relative_score` (número).

- Ausente: la respuesta MUST ser idéntica a la de hoy (mismos hits, mismo orden, mismo largo).
- Presente con un valor en `[0, 1]`: el daemon MUST devolver sólo los hits que sobreviven el corte relativo con ese ratio.
- Presente fuera de `[0, 1]`, o no finito: MUST responder 400 sin buscar ni registrar nada.

#### Escenario: sin campo
- GIVEN hits con scores 0,50, 0,40, 0,10
- WHEN se busca sin `min_relative_score`
- THEN vuelven los tres

#### Escenario: con corte
- GIVEN los mismos hits
- WHEN se busca con `min_relative_score = 0.6`
- THEN vuelven 0,50 y 0,40 (los dos mínimos; 0,10 < 0,30)

#### Escenario: fuera de rango
- WHEN se busca con `min_relative_score = 1.5`
- THEN la respuesta es 400

**Test:** `search_applies_the_requested_relative_cut` y `search_rejects_a_relative_score_outside_zero_to_one` en `crates/clawg-cli/tests/api.rs`. Fallan hoy: el campo no existe (`api.rs:229-236`) y serde lo ignora.

**Test:** la función del corte conserva los tests de `tools.rs:208-240` al moverse de módulo.

## R3. `search` registra sólo lo que devuelve

El handler `search` MUST registrar como acceso exactamente los hits contables de la respuesta que envía, después del corte de R2. Un hit cortado, o con score 0, MUST NOT cambiar `access_count` ni `last_accessed_at`.

#### Escenario: hit cortado
- GIVEN una fila A que la consulta encuentra con score alto y una fila B que queda bajo el corte de 0,6 en posición ≥ 2
- WHEN se busca con `min_relative_score = 0.6` y se hace flush del tracker
- THEN `access_count` de A sube en 1 y el de B no cambia, ni su `last_accessed_at`

#### Escenario: cero sin corte
- GIVEN una fila C que vuelve con score 0
- WHEN se busca sin `min_relative_score` y se hace flush
- THEN `access_count` de C no cambia

**Test:** `a_hit_cut_for_relevance_does_not_count_as_an_access` y `a_zero_score_hit_does_not_count_as_an_access` en `crates/clawg-cli/tests/api.rs`. Fallan hoy: se registra todo lo que devuelve la búsqueda (`api.rs:496-501`).

## R4. `recall_context` no cuenta los ceros

`build_memory_prompt` MUST registrar sólo los hits contables entre los que entran al prompt. Los hits con score 0 MUST seguir apareciendo en el prompt como hoy.

#### Escenario
- GIVEN un corpus donde la consulta deja 2 hits positivos y 3 en 0 dentro de `K_FINAL`
- WHEN se arma el prompt y se hace flush
- THEN los 5 aparecen en el prompt y sólo los 2 positivos suben `access_count`

**Test:** `recall_does_not_count_zero_score_hits_as_accesses` en `prompt_builder.rs`, con el helper de lectura de `prompt_builder.rs:1464-1475`. Falla hoy: registra todo el set (`prompt_builder.rs:168-175`).

## R5. El cliente pide el corte del puente

`DaemonClient::search` MUST mandar `min_relative_score` igual a `RELEVANCE_RATIO` (0,6). El puente MUST seguir aplicando el corte al renderizar, de modo que contra un daemon que ignora el campo el agente vea lo mismo que hoy.

#### Escenario
- GIVEN un servidor de prueba que registra el cuerpo recibido
- WHEN el cliente busca
- THEN el cuerpo contiene `"min_relative_score": 0.6`

**Test:** `search_sends_the_relevance_ratio` en `client.rs` (mod tests). Falla hoy: `SearchArgs` no tiene el campo (`client.rs:211-216`).

## Compatibilidad de datos

No hay requisito de migración: no cambia esquema, identificadores, estados ni embeddings (regla de `config.yaml`). Los contadores ya guardados no se tocan.
