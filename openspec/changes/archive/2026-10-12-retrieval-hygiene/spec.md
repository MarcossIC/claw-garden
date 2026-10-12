# retrieval-hygiene — Especificación

**Dominio destino:** `specs/retrieval/spec.md` (se fusiona al archivar).

**Alcance:** el corte relativo de `POST /v1/search` y qué hits registran un acceso en `search` y en `recall_context`. Ningún score ni orden del core cambia, y el esquema tampoco. El orden entre hits con score 0 pasó a lexical-fusion (ver `proposal.md`).

## Definiciones

- **Corte relativo con ratio r:** sobre hits ordenados mejor primero, se conservan los primeros mientras la posición sea < 2 o `score >= r × mejor` (`crates/clawg-cli/src/mcp/tools.rs:65-76` antes del cambio). La regla no cambia; pasa a `crates/clawg-cli/src/relevance.rs`.
- **Acceso:** una entrada al access tracker, que en el flush suma `access_count`, mueve `last_accessed_at` y recalcula el tier (`crates/clawg-core/src/memory/access.rs:156-163`).
- **Hit contable:** hit de `kind = memory` con `score > 0`. Sólo los `memory` cuentan porque los ids son por tabla y el flush actualiza `memory` (`api.rs:492-495` antes del cambio).

## R1. Corte relativo opcional en `/v1/search`

`POST /v1/search` MUST aceptar el campo opcional `min_relative_score` (número).

- Ausente: la respuesta MUST ser idéntica a la de hoy (mismos hits, mismo orden, mismo largo).
- Presente con un valor en `[0, 1]`: el daemon MUST devolver sólo los hits que sobreviven el corte relativo con ese ratio.
- Presente fuera de `[0, 1]`, o no finito: MUST responder 400 sin buscar ni registrar nada.

#### Escenario: sin campo
- GIVEN una fila igual a la consulta y tres sin relación
- WHEN se busca sin `min_relative_score`
- THEN vuelven las cuatro

#### Escenario: con corte
- GIVEN las mismas filas
- WHEN se busca con `min_relative_score = 0.6`
- THEN vuelven los primeros `relevant_len(scores, 0.6)` hits de la búsqueda sin corte, en el mismo orden, y son menos que cuatro

#### Escenario: fuera de rango
- WHEN se busca con `min_relative_score = 1.5` o `-0.1`
- THEN la respuesta es 400

**Test:** `search_without_a_relative_cut_is_unchanged`, `search_applies_the_requested_relative_cut` y `search_rejects_a_relative_score_outside_zero_to_one` en `crates/clawg-cli/tests/api.rs`; `a_relative_score_outside_zero_to_one_is_a_bad_request` (incluye `NaN` e infinito) en `api.rs`. Fallaban antes del cambio: el campo no existía (`api.rs:229-236`) y serde lo ignoraba.

**Test:** los tests del corte se mueven con la regla a `relevance.rs`.

## R2. `search` registra sólo lo que devuelve

El handler `search` MUST registrar como acceso exactamente los hits contables de la respuesta que envía, después del corte de R1. Un hit cortado, o con score 0, MUST NOT cambiar `access_count` ni `last_accessed_at`.

#### Escenario: hit cortado
- GIVEN una fila A con score alto y una fila B bajo el corte de 0,6 en posición ≥ 2
- WHEN se busca con `min_relative_score = 0.6` y se hace flush del tracker
- THEN `access_count` de A sube en 1, y el de B y su `last_accessed_at` no cambian

#### Escenario: cero sin corte
- GIVEN una fila C que vuelve con score 0
- WHEN se busca sin `min_relative_score` y se hace flush
- THEN `access_count` de C no cambia

**Test:** `record_hits_keeps_only_memory_hits_that_scored` y `record_hits_with_nothing_countable_sends_nothing` en `crates/clawg-core/src/memory/access.rs` fijan qué hits cuentan. El handler registra `record_hits` sobre la respuesta ya cortada, y `search_applies_the_requested_relative_cut` (R1) fija que esa respuesta no trae los cortados. El flush del tracker corre cada 30 s y no se puede adelantar desde un test HTTP, por eso la prueba se parte en esas dos piezas.

## R3. `recall_context` no cuenta los ceros

`build_memory_prompt` MUST registrar sólo los hits contables entre los que entran al prompt. Los hits con score 0 MUST seguir apareciendo en el prompt como hoy.

#### Escenario
- GIVEN una fila igual a la consulta y otra ortogonal (score 0)
- WHEN se arma el prompt y se hace flush
- THEN las dos aparecen en el prompt y sólo la primera sube `access_count`

**Test:** `recall_does_not_count_zero_score_hits_as_accesses` en `prompt_builder.rs`. Fallaba antes del cambio: se registraba todo el set (`prompt_builder.rs:168-175`).

## R4. El cliente pide el corte del puente

`DaemonClient::search` MUST mandar `min_relative_score` igual a `RELEVANCE_RATIO` (0,6). El puente MUST seguir aplicando el corte al renderizar, de modo que contra un daemon que ignora el campo el agente vea lo mismo que hoy.

#### Escenario
- WHEN el cliente arma el cuerpo de una búsqueda
- THEN el cuerpo contiene `"min_relative_score": 0.6` además de los campos de `SearchArgs`

**Test:** `search_request_carries_the_relevance_ratio` en `client.rs`. Fallaba antes del cambio: el cliente serializaba `SearchArgs` tal cual (`client.rs:211-216`, `:357-359`).

## Compatibilidad de datos

No hay requisito de migración: no cambia esquema, identificadores, estados ni embeddings (regla de `config.yaml`). Los contadores ya guardados no se tocan.
