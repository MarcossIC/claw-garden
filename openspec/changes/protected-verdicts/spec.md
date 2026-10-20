# protected-verdicts — Especificación

Una **fila protegida** es una fila de `memory` cuyo `memory_type` es `user_preference` o `architectural_decision`. Una **declaración** es `user_answered: true` en el pedido: el agente dice que el usuario contestó. La declaración no autentica a nadie; la guarda pide que el reemplazo sea explícito y quede registrado.

## R1. `supersedes` sobre una protegida pide la declaración en cualquier estado del par

`supersedes` cuyo `other` (la fila que pierde) es protegida MUST fallar sin cambiar nada con `ResolveError::NeedsUser`, en un par `pending`, `escalated` (por un `conflicts` del agente o desde el save), `judged` o sin fila, salvo que el pedido traiga la declaración. Que `keep` sea protegida MUST NOT pedirla: así se deshace un veredicto que reemplazó a una protegida, y una decisión reemplaza a un hecho.

El error MUST nombrar la fila protegida y decir que se juzgue `conflicts`, se pregunte al usuario y se llame otra vez con `user_answered` y su respuesta en `reason`. El daemon lo responde `409`.

#### Escenario: `conflicts` y después `supersedes`
- GIVEN un par `pending` con `other` una `user_preference` del agente
- WHEN el agente lo juzga `conflicts` y después `supersedes` sin declaración
- THEN el segundo falla con `NeedsUser`, `other` sigue viva y el par sigue `escalated`

#### Escenario: un par que nació escalado
- GIVEN un par `escalated` desde el save con `other` una `architectural_decision`
- WHEN se juzga `supersedes` sin declaración
- THEN falla y nada cambia

#### Escenario: `compatible` y después `supersedes`
- GIVEN un par protegido juzgado `compatible`
- WHEN se juzga `supersedes` sin declaración
- THEN falla y el par sigue `judged` con `compatible`

#### Escenario: un par sin fila
- GIVEN una fila `disputed` contra una `user_preference` sin par en `memory_conflict`
- WHEN se juzga `supersedes` sobre la preferencia sin declaración
- THEN falla y nada cambia

#### Escenario: deshacer
- GIVEN una `user_preference` reemplazada por un hecho con un veredicto declarado
- WHEN se juzga `(keep = la preferencia, other = el hecho)` con `supersedes` sin declaración
- THEN la preferencia vuelve `active` y el hecho queda `superseded`

**Tests:** `a_protected_row_is_not_superseded_after_conflicts`, `a_pair_escalated_at_save_does_not_open_the_guard`, `a_judged_pair_does_not_open_the_guard`, `a_pair_without_a_row_does_not_open_the_guard`, `a_protected_keep_needs_no_answer` (`dispute.rs`); `a_protected_verdict_without_the_answer_is_a_409` (`crates/clawg-cli/tests/api.rs`).

## R2. La declaración lleva la respuesta y queda en el par

Un veredicto con la declaración MUST traer un `reason` no vacío, o fallar sin cambiar nada con `ResolveError::AnswerRequired` (`400`). Con él, el veredicto se aplica como sin la guarda, y el par MUST guardar el `reason` enmascarado precedido de `user answered: `, para que quien lea el par distinga un veredicto declarado de uno del agente. Sin la declaración el `reason` se guarda como hoy.

#### Escenario: con la respuesta
- GIVEN un par `escalated` con `other` una `user_preference`
- WHEN se juzga `supersedes` con `user_answered: true` y `reason: "the user now prefers tabs"`
- THEN `other` queda `superseded` y el par `judged` con `supersedes` y `reason = "user answered: the user now prefers tabs"`

#### Escenario: sin la respuesta
- WHEN se juzga con `user_answered: true` y sin `reason` (o en blanco)
- THEN falla con `AnswerRequired` y nada cambia

**Tests:** `a_declared_answer_supersedes_and_is_recorded`, `a_declared_answer_needs_a_reason` (`dispute.rs`).

## R3. Bajar el tipo de una protegida pide la declaración

Un cambio de `memory_type` que lleva una fila protegida a un tipo que no lo es MUST fallar sin cambiar nada con `UpdateError::NeedsUser` (`409`), salvo con la declaración, que no se registra (el `PATCH` no guarda razón). Promover a un tipo protegido, cambiar entre los dos protegidos o cambiar sólo `importance` MUST NOT pedirla. El cuerpo del `PATCH` acepta `user_answered`; un puente que no lo manda se lee `false`.

#### Escenario: bajar sin declaración
- GIVEN una `user_preference`
- WHEN se pide `memory_type: semantic_fact` sin `user_answered`
- THEN falla y la fila sigue `user_preference` con su `update_count`

#### Escenario: bajar con declaración, promover sin ella
- WHEN se pide lo mismo con `user_answered: true`
- THEN la fila queda `semantic_fact`
- AND pedir `user_preference` sobre un `semantic_fact` pasa sin declaración

**Tests:** `demoting_a_protected_row_needs_the_answer`, `promoting_or_changing_importance_needs_no_answer` (`admin.rs`); `a_demotion_without_the_answer_is_a_409` (`crates/clawg-cli/tests/api.rs`).

## R4. Las herramientas lo dicen

- `resolve_conflict` y `update_memory` MUST declarar `user_answered` (booleano, opcional) en su schema, y el puente MUST mandarlo al daemon (`client.judge`, `client.update`); `update` lo omite cuando es `false`, así un daemon anterior, cuyo `PATCH` rechaza campos desconocidos, sigue aceptando el pedido de siempre.
- La descripción de `resolve_conflict` MUST decir que reemplazar una preferencia o una decisión pide `user_answered` con la respuesta del usuario en `reason`, y sólo después de preguntarle; la de `update_memory`, que sacarle el tipo protegido también lo pide.
- `protocol.md` MUST pedir registrar la respuesta del usuario a ACTIVE CONFLICTS con `user_answered` y su respuesta en `reason`.
- `agent_contract.json` cambia a propósito.

**Tests:** `the_agent_contract_matches_its_file`, `resolve_conflict_sends_the_users_answer` y `update_memory_sends_the_users_answer` (`server.rs`).
