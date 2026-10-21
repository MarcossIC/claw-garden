# protected-verdicts — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 4 | S–M (una guarda y un argumento; M si se agrega la columna) | — (antes de 44; se coordina con 52, 54, 51 y 56 por el número de migración) | exploración (borrador) |

**Tesis:** «una preferencia o una decisión se reemplaza sólo después de que el usuario contestó» es hoy una frase que se le dice al modelo, no una regla del daemon. La guarda de `judge_pair` mira un solo estado del par (`pending`) y deja pasar el mismo `supersedes` por tres puertas: un par `escalated` (que el propio agente puede crear con `conflicts`, sin que nadie responda), un par ya `judged` (basta un `compatible` antes) y un par sin fila en `memory_conflict`. Además mira el `memory_type` de la fila y no su origen, y `update_memory` puede cambiar ese tipo. El efecto es que un agente pisa, solo, una preferencia que el usuario dijo, por un error o por una instrucción inyectada en algo que leyó. No hay frontera real contra un agente malicioso (declara lo que quiera, igual que `user_stated`); lo que se busca es que pisar una fila protegida exija una declaración explícita y registrada, y no un baile de dos llamadas que ni el modelo nota. No cambia ranking, ni embeddings, ni el detector.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `supersedes` se rechaza sólo si el par es `pending` y una fila es protegida | **Cierta.** `if verdict == Verdict::Supersedes && state.as_deref() == Some("pending") && (keep_row.protected() || other_row.protected())` (`crates/clawg-core/src/memory/dispute.rs:349-354`); `protected()` es `user_preference` o `architectural_decision` (`:130-132`). |
| `conflicts` deja el par `escalated` sin verificar al usuario y `supersedes` sobre `escalated` no se guarda | **Cierta.** `Verdict::Conflicts => ("escalated", None, None)` (`dispute.rs:380`) y la guarda de arriba no mira ese estado. Lo fija el propio test: `conflicts` y después `supersedes` sobre la misma pareja devuelve `superseded` (`dispute.rs:996-998`, `a_protected_pending_pair_is_not_superseded_without_the_user`). La spec lo llama virtud: «Sobre un par escalado sí vale: el usuario ya fue consultado» (`openspec/specs/disputes/spec.md:115`). Nada registra que el usuario contestó: `escalated` sólo dice que se le *pregunta*, y el mismo agente lo puso. |
| Un par escalado al guardar nace supersedable | **Cierta.** `note_candidates` escribe `escalated` si `c.escalated` (`crates/clawg-core/src/memory/store_insert.rs:258`), con la regla de `needs_user` (`contradiction.rs:103-111`); la guarda no mira ese estado. La verificación de conflict-candidates lo registra: «Sobre el par escalado, `supersedes` sí se aceptó» (`openspec/changes/archive/2026-10-18-conflict-candidates/verify.md:104`). |
| Un par sin fila en `memory_conflict` tampoco se guarda | **Cierta, y hay un test que la ejerce.** `state` sale `None` si no hay fila (`dispute.rs:341-347`) y `check_pair` acepta el par igual si una fila nombra a la otra en `disputed_against` (`:140-156`). `the_old_resolve_records_its_verdict` siembra una `disputed` contra otra sin par (`dispute.rs:1019-1022`). En datos reales sólo aparece con restos anteriores a V15 o con un par purgado (`specs/disputes:287`). |
| **No estaba en la premisa:** un par `judged` tampoco se guarda | **Cierta, una cuarta puerta.** Un veredicto nuevo reescribe el par (`specs/disputes` R3, `:110`) y `compatible` sobre un par protegido `pending` no se rechaza (`dispute.rs:357-374`; test `:1013`); el par queda `judged`, `state != "pending"` y el `supersedes` siguiente pasa. |
| **No estaba en la premisa:** la guarda mira los dos lados | **Cierta y es relevante.** `keep_row.protected() \|\| other_row.protected()` (`:351`): hoy también se rechaza que una decisión reemplace a un hecho, y se rechazaría el *deshacer* (`keep` = la fila protegida que un veredicto había reemplazado, `specs/disputes:109`). Si se guarda en todos los estados, hay que mirar sólo la fila que pierde (`other`). |
| El alcance por dueño impide juzgar filas de otro agente | **Cierta, pero acotada.** `read_pair_row(.., owner)` trata la fila ajena como inexistente (`dispute.rs:218-240`) y el puente siempre manda su agente (`crates/clawg-cli/src/mcp/server.rs:904-918`). El daemon acepta `agent_id` ausente y entonces no filtra (`JudgeRequest.agent_id` es `Option`, `crates/clawg-cli/src/daemon/api.rs:771-773`); es la deuda ya anotada de `sharing.json` (README, backlog). La exposición de este cambio es otra: el agente pisa sus **propias** filas protegidas, incluida una que el usuario dijo. |
| `protected()` mira el tipo, no el origen | **Cierta.** `PairRow` ni siquiera lee `created_by_type` (`dispute.rs:116-122`, `SELECT_PAIR_ROW` `:218`). Un test fija que «el origen no juzga nada» (`dispute.rs:1062`) y la spec de procedencia dice que el origen no cambia ningún par, estado ni puntaje (`openspec/specs/provenance/spec.md:3`). |
| La descripción del tool dice que hay que esperar al usuario | **Cierta, y es sólo una señal para el modelo.** «A preference or a decision is superseded only after the user answered: until then judge it conflicts» (`server.rs:1160-1162`, `agent_contract.json:152`); el error repite la frase (`dispute.rs:75-79`) y el protocolo manda `conflicts` «if only the user can tell» (`protocol.md:27`). La descripción manda juzgar `conflicts` y el daemon deja pasar el `supersedes` que sigue: la frase y la guarda se contradicen. |
| Hay una vía de CLI para que el usuario juzgue | **Falsa.** `cli.rs` no tiene subcomando de veredictos; `clawg memories` es de sólo lectura (`rg judge\|resolve crates/clawg-cli/src/cli.rs` sólo da `Profile::resolve`). Habría que crearla (enfoque C). |
| Otros caminos son la misma puerta | **Parcial.** Ver «Otros caminos»: `update_memory` sí lo es, una revisión por `topic_key` es una puerta distinta y deliberada, `delete_memory` es reversible. |

## Problema (verificado contra el código)

1. **Un par protegido se reemplaza sin el usuario por cuatro puertas**, todas con las filas y el alcance propios del agente: (a) `conflicts` y `supersedes` seguidos (`dispute.rs:370-373`, `:380`); (b) un par que ya nació `escalated` (`store_insert.rs:258`), que ni siquiera necesita el primer paso; (c) `compatible` y `supersedes` seguidos (par `judged`); (d) un par sin fila (`dispute.rs:341-348`). El resultado se ve en `memory_status` como una fila `superseded` más; no queda quién ni por qué.
2. **Nada distingue la fila que dijo el usuario.** `created_by_type = 'user'` (`user_stated`, `provenance.rs:15-21`) no entra en la decisión. Una `user_preference` que el usuario declaró se pisa igual que una que el agente infirió. (Las filas anteriores al sello leen `explicit` / `user` aunque las haya dicho el agente, `.claude/CLAUDE.md`: el origen sirve para proteger de más, no para acusar.)
3. **El tipo se puede cambiar.** `update_memory` / `PATCH /v1/memories/{kind}/{id}` acepta `memory_type` (`crates/clawg-core/src/memory/admin.rs:164-178`) y `update_memory_core` lo escribe sin mirar de qué tipo viene (`:242-247`); la descripción del tool lo ofrece «when the user says … it is a different kind of memory» (`server.rs:1140-1145`). Un agente puede pasar una `user_preference` a `semantic_fact` y después juzgarla: `protected()` ya no la ve. Cierra la guarda por el costado.
4. **La frase del usuario es falsa en los dos sentidos.** La spec justifica el caso `escalated` con una consulta que nada registra (`specs/disputes:115`); el modelo, que lee «sólo después de que el usuario contestó», no tiene dónde declarar que sí contestó, así que el camino legítimo y el abusivo son la misma secuencia de llamadas.

## Otros caminos (¿misma puerta por otro lado?)

- **Save con `topic_key`.** Reemplaza toda fila viva de esa clave, del mismo agente y proyecto, sea del tipo y origen que sea (`SCOPE_ROWS_SQL`, `crates/clawg-core/src/memory/topic.rs:81-85`; `SUPERSEDE_SQL` con causa `revision` en `:131`). Una `user_preference` dicha por el usuario con clave `prefs/x` se reemplaza con un save del agente bajo esa clave, y la nueva hereda el tipo de la vieja (`topic.rs:106-110`) pero no su origen: queda `inferred`. **Fuera de alcance de la guarda**, con razón: una revisión es un acto explícito (el agente escribe el texto nuevo y nombra la clave), no un veredicto sobre algo que el detector encontró; es reversible (`revision-aware-delete (49)` revive la versión anterior al borrar la nueva, `specs/disputes` R4) y la respuesta lista lo reemplazado en `replaced`. Queda como decisión abierta si `replaced` debe marcar las filas protegidas o `user`.
- **`update_memory` con `memory_type`.** **Dentro del alcance**, es la puerta 3. Un tipo protegido que se cambia a uno no protegido es el paso que vuelve inútil la guarda; cambiar a uno protegido (promover) no lo es.
- **`delete_memory` de una fila protegida.** **Fuera de alcance.** Se recupera con `restore_memory` y con `clawg undelete` (el usuario), y avisa con `restore_memory with the same id…` (`specs/mcp:331`). No hay hueco silencioso: la fila sigue en la base.
- **Veredicto sobre un pedazo (backlog de README, «Un veredicto `supersedes` sobre un pedazo deja su save con un hueco»).** Toca el mismo `judge_pair` (`dispute.rs:316-401`, `:364`). Son cambios independientes en el mismo bloque: este agrega una guarda antes de `:356`, aquél decide qué se reemplaza en `:363-366`. Se coordina en el orden de los parches, no en el diseño; ver «Interacciones».

## Lo que ya existe y se reutiliza

- `judge_pair` ya corre en una transacción y guarda `reason` en el par (`RECORD_PAIR_SQL`, `dispute.rs:291-296`): una declaración del usuario puede viajar en `reason` sin migración.
- `ResolveError::NeedsUser` y su traducción a `409` (`dispute.rs:75-79`, `crates/clawg-cli/src/daemon/api.rs:1417-1419`); el puente ya muestra el cuerpo del error.
- `user_stated` como modelo de una declaración del agente que sella una fila (`provenance.rs:15-21`, `specs/provenance:38`); el mismo patrón sirve para «el usuario contestó».
- `created_by_type` y la marca de lectura (`provenance::mark`, `:37-45`), ya leída por `memory_status` y ACTIVE CONFLICTS.
- `the_agent_contract_matches_its_file` y `CLAWG_BLESS=1` para fijar el texto nuevo.

## Enfoques posibles

**A. Guarda en todos los estados, con una declaración (recomendado).** `supersedes` sobre un par donde la fila que **pierde** es protegida falla con `NeedsUser` en cualquier estado del par (`pending`, `escalated`, `judged`, sin fila), salvo que la llamada traiga `user_answered: true` y un `reason` con lo que el usuario contestó. El argumento viaja por `ResolveConflictArgs`, el cliente y `JudgeRequest`.
- **A-mín (sin migración):** la declaración queda en `reason` del par (ya existe), sin columna nueva. Alcanza para historia legible y para el cliente, que muestra el argumento al usuario en su pedido de permiso de la herramienta.
- **A-completo:** además una columna en `memory_conflict` (por ejemplo `judged_by`, `agent` o `user_declared`) para que `memory_status` y la exportación (43) la lean sin analizar texto. Es una **migración V18**, y V18 ya la quieren save-bounds (54), secret-coverage (51) y query-plans (56): hay que numerarlas juntas.
- **A favor:** cierra las cuatro puertas con una sola regla; mira la fila que pierde y por eso deja pasar el deshacer; no necesita saber el origen, así que sirve para las filas anteriores al sello.
- **En contra:** el agente puede mentir con `user_answered: true`, igual que con `user_stated`. Cambia el esquema del tool (y su contrato).

**B. Proteger por origen.** Una fila `created_by_type = 'user'` nunca se reemplaza por un veredicto del agente; sólo un save `user_stated` la revisa. Sin argumento nuevo.
- **A favor:** protege justo lo que el usuario dijo y no le agrega superficie al tool.
- **En contra:** contradice el principio de procedencia (el origen no decide, `specs/provenance:3`), deja desprotegida una preferencia que el agente guardó bien y el usuario aprobó, y las filas anteriores al sello (`explicit` / `user`) quedarían todas protegidas aunque las haya escrito el agente. No resuelve el cambio de tipo. Sirve como capa extra de A (decisión abierta), no sola.

**C. Un comando de CLI que sólo el usuario corre.** `clawg` ganaría un subcomando de veredicto; MCP sólo respondería `conflicts`. Es la más estricta, pero hoy no existe ninguno (premisa falsa de arriba), obliga a un flujo de CLI para cada disputa y un agente con shell puede ejecutarlo igual. Descartado como primera medida.

**D. Dejarlo y corregir el texto.** Decir en la spec, el tool y el protocolo que es una señal al modelo, y borrar «el usuario ya fue consultado». Es honesto y gratis, y deja el caso de uso real (un error o una inyección) abierto. Es el piso: si se descarta A, esto se hace igual.

**Cambio de tipo (parte de A).** Un `memory_type` de `update_memory` que saca a una fila de los tipos protegidos pide la misma declaración, pero el `PATCH` no guarda razón y no hay dónde registrarla. Opciones: (i) la declaración es sólo el booleano, sin registro; (ii) rechazar el descenso desde el tool y dejar el cambio al usuario por la CLI (hoy no hay `clawg memories update`); (iii) dejar la puerta y documentarla. Se recomienda (i), y decidir (ii) si hay evidencia de que un modelo la usa.

**Recomendación:** A-mín más el descenso de tipo con declaración, y D en el mismo parche (el texto del tool, la spec y el protocolo dicen que la declaración es del agente y que la guarda es una compuerta de intención, no de acceso). A-completo sólo si 43 o `memory_status` piden leerlo sin texto. B se deja como decisión abierta.

## Descartado

| Opción | Razón |
|---|---|
| Guardar sólo en `escalated` y `judged` y dejar `pending` | Es lo que ya hay con otro nombre; el test `:984-999` fija exactamente la secuencia a cerrar. |
| Impedir `conflicts` sobre una protegida | `conflicts` es la salida correcta («sólo el usuario puede decirlo») y alimenta ACTIVE CONFLICTS (`specs/disputes:103`). |
| Guardar también el save con `topic_key` | Es una revisión explícita y reversible; guardarla rompería el caso normal de una decisión que evoluciona (`specs/ingest`, topic-revisions (27)). |
| Un token distinto para el usuario | Es una frontera de acceso que el modelo de amenazas no tiene: los agentes son del mismo usuario y leen el token (`.claude/CLAUDE.md`, README, backlog de `sharing.json`). |
| Un clasificador de «parece que el usuario contestó» | Heurística sin medida; sin adivinanzas, como en `redact`. |

## Interacciones

- **untrusted-recall (52).** Cubre la inyección (cómo llega un texto ajeno al modelo); éste cubre la consecuencia (qué puede hacer el modelo con un texto inyectado sobre una preferencia). Son capas distintas y no se pisan; el texto del tool que cambia aquí no está en el prompt de contexto que toca 52.
- **history-import (44).** Importa preferencias y decisiones en volumen con origen `imported` (`provenance::mark`, `:41`); sin esta guarda, un par con una de ellas se reemplaza con la misma secuencia. Va antes.
- **memory-portability (43).** Si se agrega una columna a `memory_conflict`, el export tiene que llevarla (no pierde ni oculta datos); con A-mín no cambia nada.
- **doctor-checks (31).** Un chequeo posible: pares `judged` con `supersedes` sobre una fila protegida sin `user_answered`. Se anota, no se hace acá.
- **memory-provenance (41, archivado).** No cambia: A no lee el origen. B sí lo tocaría (`specs/provenance:3`).
- **conflict-candidates (24, archivado) y dispute-resolution (archivado, `openspec/changes/archive/2026-10-13-dispute-resolution/`).** Definen los pares y `resolve_conflict`; `specs/disputes` R3 pasa de «sobre un par escalado sí vale» a la regla nueva.
- **topic-revisions (27) y revision-aware-delete (49), archivados.** No cambian; ver «Otros caminos» sobre `replaced`.
- **Backlog «veredicto `supersedes` sobre un pedazo».** Mismo bloque de `judge_pair`; los dos parches se ordenan para no pisarse, y la prueba del que entra segundo cubre el caso protegido sobre un pedazo.
- **Migración V18.** Con A-completo comparte número con 54, 51 y 56: acordar el orden antes de aplicar (`openspec/README.md`, números de migración).
- **Specs y contrato.** `specs/disputes` (R3, R1 y R6, el escenario nuevo y el `409`), `specs/mcp` (`:288`, `:296`: argumento `user_answered`), `specs/update` (R5, descenso de tipo), `specs/provenance` (una línea: la declaración no es el origen). El contrato `agent_contract.json` (`:152` y el schema de `resolve_conflict` y `update_memory`) se bendice con `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file` y se revisa el diff.

## Alcance candidato

1. Guarda de `supersedes` sobre la fila que pierde protegida, en todos los estados del par, con `NeedsUser`.
2. Argumento `user_answered` (booleano) en `resolve_conflict`, en el cliente y en `JudgeRequest`; con `reason` obligatorio cuando se usa para una protegida; la razón queda en el par.
3. El mismo argumento en `update_memory` para sacar a una fila de los tipos protegidos.
4. Texto del tool, del error, de `protocol.md` y de `specs/disputes`: dice qué es la guarda y qué no.
5. Contrato bendecido; escenarios y pruebas en `specs/disputes`, `specs/mcp` y `specs/update`.

### No-alcance

- Una frontera de acceso o una identidad del usuario (A declara, no autentica).
- `topic_key`, `delete_memory` y el alcance del daemon sin `agent_id`.
- La columna `judged_by` (A-completo) salvo decisión abierta, y su migración.
- Proteger por origen (B), una CLI de veredictos (C) y cualquier cambio al detector, al ranking o a los vectores.

## Decisiones abiertas

Son del usuario.

- [ ] **Registro.** ¿A-mín (la razón del par) o A-completo (columna V18 coordinada con 54, 51 y 56)?
- [ ] **Qué pierde.** ¿La guarda mira sólo la fila que pierde (recomendado, deja pasar el deshacer) o los dos lados como hoy?
- [ ] **Origen.** ¿Se suma B: una fila `user` sólo la reemplaza un `supersedes` cuyo `keep` también sea `user`, aun con `user_answered`?
- [ ] **Cambio de tipo.** ¿Declaración sin registro (i), rechazo desde el tool (ii) o se deja la puerta (iii)?
- [ ] **`replaced`.** ¿Una revisión por clave marca en la respuesta que reemplazó una fila protegida o `user`?
- [ ] **Compatibilidad.** Un puente anterior no manda `user_answered`: tomarlo como `false` (el caso seguro) es lo esperado; ¿se avisa cuando el daemon es más nuevo que el puente?

## Cómo se mide

Tests RED primero (TDD estricto), sobre `MemoryStore` en memoria con `note_pair` y `seed_typed`, sin modelo ni red:

1. `conflicts` y luego `supersedes` sobre una pareja con una `user_preference` propia: hoy se reemplaza (`dispute.rs:996-998`); con el cambio falla con `NeedsUser` y la fila sigue `active`.
2. Un par que nació `escalated` (`note_pair(.., "escalated")`) con una `architectural_decision` como `other`: hoy se reemplaza; con el cambio falla.
3. `compatible` y luego `supersedes` (par `judged`): hoy pasa; con el cambio falla.
4. Un par sin fila, enlazado sólo por `disputed_against` (como `:1019-1022`): hoy pasa; con el cambio falla.
5. Con `user_answered: true` y `reason`, el mismo `supersedes` pasa y el par guarda la razón; sin `reason`, falla con un error que la pide.
6. Deshacer: la protegida como `keep` sobre una `superseded` por un veredicto vuelve `active` sin declaración.
7. `update_memory` con `memory_type: semantic_fact` sobre una `user_preference` falla sin `user_answered`; promover a `user_preference` pasa.
8. El contrato y el texto: `the_agent_contract_matches_its_file` cambia a propósito; el `409` del daemon lleva el texto nuevo (`crates/clawg-cli/tests/api.rs`).
9. Con A-completo: una base en V17 con pares `judged` abre en V18 con los pares iguales y alcanzables (regla de `openspec/config.yaml` sobre cambios de esquema).

`rag-eval` no se ve afectado: no cambia ranking, vectores ni el prompt de contexto. `conflict-eval` tampoco debería moverse: mide el detector (`needs_user`, `contradiction.rs:103-111`) sobre pares etiquetados, y no pasa por `judge_pair` (`rg judge_pair crates/clawg-eval` no da nada en `conflict-eval`); se corre una vez para confirmar que no se movió.

## Riesgos

- **Falsa seguridad.** El modelo declara `user_answered: true` sin preguntar. La guarda convierte un descuido en una mentira explícita y deja huella; no detiene a un agente que miente. Hay que decirlo en la spec.
- **Fricción en el caso bueno.** Un usuario que dice «reemplazá esa preferencia» exige que el modelo pase el argumento y una razón; si el tool no lo enseña bien, el modelo caerá en `conflicts` de más. Se mide mirando los textos del contrato con un agente real en el perfil dev.
- **Guardar mal el deshacer.** Mirar los dos lados bloquearía restaurar la protegida; el test 6 lo cubre.
- **Esquema compartido.** A-completo agrega una migración que otros tres cambios también quieren; un número mal repartido rompe el arranque. Se numera antes de aplicar.
- **Un puente viejo.** Sin `user_answered`, un puente anterior nunca puede reemplazar una protegida: es el lado seguro, pero rompe el flujo hasta actualizar.
- **Lo que no se cierra.** Un titular del token omite `agent_id` y juzga cualquier fila (`api.rs:771-773`); la deuda de `sharing.json` sigue abierta y éste no la toca.
