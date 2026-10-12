# lexical-fusion — Especificación

**Dominio destino:** `specs/retrieval/spec.md` (se fusiona al archivar).

**Alcance:** cómo un identificador exacto de la consulta entra al score de `search_all_memories`. No cambia el esquema, ni el piso de coseno, ni el orden entre hits que no matchean un identificador.

## Definiciones

- **Token:** cada tramo de `char::is_alphanumeric() || '_'` de la consulta, como los parte `fts_tokens` (`crates/clawg-core/src/memory/search.rs:515-519`).
- **Token identificador:** un token que cumple al menos una de:
  - tiene letras y dígitos (`getUser4821`, `v2`);
  - tiene `_` y al menos una letra (`E_AUTH_4821`, `snake_case`);
  - tiene una minúscula seguida inmediatamente de una mayúscula (`getEstadoLabel`).
  Un número solo (`4821`) no es identificador.
- **Identificadores de la consulta:** los tokens identificadores distintos de toda la consulta, en orden de aparición, hasta 8 (`MAX_IDENTIFIER_TOKENS`).
- **Match de identificador:** fila de `memory` visible para el llamador (mismos predicados de agente, proyecto, estado y `deleted_at` que el canal léxico) cuyo contenido tiene **todos** los identificadores de la consulta, según FTS5 (`"t1" AND "t2" …`).
- **Presupuesto léxico:** `SearchScope.lexical` (20 por defecto, `search.rs:38`).
- **Bono léxico:** `ScoringConfig.lexical_bonus`, 1,0 por defecto.

## R1. Tokens identificadores

`identifier_tokens(query)` MUST devolver los identificadores de la consulta según la definición, de **toda** la consulta, sin repetir y como mucho 8. Es una función pura.

#### Escenario: tipos de identificador
- GIVEN la consulta `qué hace getEstadoLabel con E_AUTH_4821 y getUser4821 en 2026`
- THEN devuelve `getEstadoLabel`, `E_AUTH_4821`, `getUser4821`, en ese orden
- AND no devuelve `qué`, `hace`, `con`, `y`, `en` ni `2026`

#### Escenario: identificador al final de una consulta larga
- GIVEN una consulta de 20 palabras comunes seguidas de `parseLedger7731`
- THEN devuelve `parseLedger7731`

#### Escenario: sin identificadores
- GIVEN `how do we deploy the importer`
- THEN devuelve una lista vacía

**Test:** `identifier_tokens_*` en `search.rs`, mod tests.

## R2. El score de un match suma el bono

Para un hit marcado como match de identificador, `score()` MUST valer `(cos' + lexical_bonus) × max(piso_tier, decaimiento) × importancia × estado`, donde `cos'` es el coseno ya re-escalado contra el piso. Para un hit sin match, MUST valer exactamente lo mismo que hoy.

#### Escenario: el bono se suma después del piso
- GIVEN un hit match con coseno bajo el piso (cos' = 0), tier 1, edad 0, importancia 1 y estado `active`
- THEN el score es `lexical_bonus × importancia`, mayor que 0

#### Escenario: sin match, sin cambio
- GIVEN el mismo hit sin match
- THEN el score es 0, igual que hoy

**Test:** `a_lexical_match_adds_the_bonus_after_the_floor` y `without_a_match_the_score_is_unchanged` en `score.rs`.

## R3. La búsqueda premia el identificador exacto

Con el canal léxico encendido y texto de consulta, `search_all_memories` MUST marcar como match de identificador cada fila que lo sea, venga del canal denso o del léxico, y MUST sumar los matches a los candidatos léxicos aunque estén fuera del top denso o pasado el token 16.

#### Escenario: el identificador le gana a los hermanos
- GIVEN 8 filas: 7 hermanos de coseno alto con `resolveLedger4812`, `resolveLedger4821`… y una fila de coseno más bajo que menciona `resolveLedger4182`
- WHEN se busca `what does resolveLedger4182 do`
- THEN la fila con `resolveLedger4182` sale primera

#### Escenario: identificador fuera del top denso
- GIVEN una fila con `parseLedger7731` lejos del vector de la consulta (fuera del pool denso) y la consulta con `parseLedger7731` después del token 16
- WHEN se busca
- THEN la fila vuelve con score > 0

#### Escenario: canal apagado
- GIVEN las mismas filas y `SearchScope.lexical = None`, o sin texto de consulta
- THEN los scores son los de hoy: no hay bono

**Test:** `an_exact_identifier_outranks_its_closer_siblings`, `an_identifier_past_the_sixteenth_token_still_reaches_its_row` y `no_bonus_without_the_lexical_channel_or_query_text` en `search.rs`.

## R4. Un identificador que no discrimina no da bono

Si las filas visibles que matchean los identificadores de la consulta son más que el presupuesto léxico, ninguna fila MUST recibir el bono. Si la consulta no tiene identificadores, MUST NOT correr la consulta de matches.

#### Escenario: identificador repetido
- GIVEN 21 filas que mencionan `getUser42` y un presupuesto de 20
- WHEN se busca `getUser42`
- THEN ningún hit lleva bono: los scores son los mismos que con `lexical_bonus = 0`

#### Escenario: consulta natural
- GIVEN una consulta sin tokens identificadores
- THEN los scores son bit a bit los de `lexical_bonus = 0`

**Test:** `an_identifier_shared_by_more_rows_than_the_budget_gives_no_bonus` y `a_query_without_identifiers_scores_exactly_as_without_the_bonus` en `search.rs`.

## R5. El bono respeta el scope

Un match de identificador MUST cumplir los mismos predicados que el candidato léxico: agente, proyecto (con la excepción de `user_preference`), estado `active`/`disputed` y `deleted_at IS NULL`. Una fila de otro agente, borrada o de otro proyecto MUST NOT recibir el bono ni contar para el presupuesto.

#### Escenario: fila de otro agente
- GIVEN 21 filas de otro agente y una del llamador con `getUser42`
- WHEN el llamador busca `getUser42` con presupuesto 20
- THEN su fila lleva el bono

**Test:** `rows_the_caller_cannot_see_neither_match_nor_count_against_the_budget` en `search.rs`.

## R6. La búsqueda no falla por el canal

Un error de la consulta de matches (FTS ausente, MATCH rechazado, conexión cerrada) MUST dejar la búsqueda como si no hubiera matches, igual que hoy con los candidatos léxicos (`search.rs:592-625`).

**Test:** `a_failing_identifier_query_leaves_the_search_unboosted` en `search.rs`.
