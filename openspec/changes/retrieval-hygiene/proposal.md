# retrieval-hygiene — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — | apply |

## Problema

El ranking aprende de accesos que el agente nunca vio. La exploración también encontró un segundo defecto, los empates en score 0, que al implementarse resultó depender de lexical-fusion y pasó a ese cambio (abajo).

**1. Accesos que el agente nunca vio.**

- El handler `search` del daemon registra en el access tracker todos los hits `memory` que devuelve (`crates/clawg-cli/src/daemon/api.rs:492-501`), hasta `limit` (por defecto 10, máximo 50: `api.rs:46-48`).
- El puente MCP corta después: se queda con `score >= 0,6 × mejor` y siempre muestra 2 (`crates/clawg-cli/src/mcp/tools.rs:57-76`). Lo que corta ya se contó como acceso.
- `recall_context` rellena hasta `K_FINAL = 10` (`crates/clawg-core/src/memory/prompt_builder.rs:32`, `:156-157`) aunque sean filas con score 0, y las registra todas (`prompt_builder.rs:168-175`).
- Cada acceso suma `access_count + 1` y pone `last_accessed_at` en ahora (`crates/clawg-core/src/memory/access.rs:156-163`). Eso resetea el decaimiento, que se mide desde el último acceso (`crates/clawg-core/src/memory/score.rs:268-270`), y puede promover de tier: `architectural_decision` y `session_summary` suben a tier 1 con 3 accesos (`crates/clawg-core/src/memory/tier.rs:48-50`).
- Es un bucle "el rico se hace más rico": una fila que aparece de relleno gana puntaje para la próxima consulta.

**2. Empates en score 0 (pasa a lexical-fusion).**

- Bajo el piso de coseno el score es 0 exacto (`score.rs:201-202`), y `hit_cmp` desempata por `(kind, id ASC)` (`crates/clawg-core/src/memory/search.rs:78-84`): entre ceros sale primero la memoria más vieja.
- Se probó la opción a (desempatar por distancia) y rompió `the_two_sweep_lexical_points_search_differently_and_say_so_in_their_results` (`crates/clawg-eval/src/bin/rag-eval/main.rs:1534`). La causa: un candidato que sólo trae el canal léxico queda fuera de los k más cercanos del canal denso, así que por distancia siempre va detrás de todos los ceros densos. Hoy sólo llega al top 10 gracias al desempate por id viejo.
- O sea que la opción a tiene el mismo efecto que la exploración le veía a la b: borra los matches léxicos bajo el piso. El orden entre ceros se decide junto con la fusión de canales.

## Decisiones (cierran las abiertas de la exploración)

**Accesos: opción b, con el corte en el daemon y no en el core.**

- `POST /v1/search` acepta un campo opcional `min_relative_score`. Si viene, el daemon aplica el mismo corte que hoy aplica el puente (ratio y mínimo de 2) **antes** de registrar accesos y de responder. Si no viene, la respuesta es la de hoy.
- La función pura del corte (`relevant_hits`) se mueve de `mcp/tools.rs` a un módulo de `clawg-cli` que usan el daemon y el puente. `clawg-core` no cambia su ranking: `search_all_memories`, `rag-eval` (que llama a `store.search` directo, `crates/clawg-eval/src/bin/rag-eval/main.rs:398`) y R@k no se enteran. Así se respeta la decisión previa de dejar el corte fuera del ranking.
- El cliente de `clawg-cli` manda `min_relative_score = 0,6` (`client.rs:211-216`). El puente conserva su corte: es idempotente y lo protege si habla con un daemon de una versión anterior.
- Por qué no a: necesita una segunda llamada o estado entre llamadas para algo que el daemon puede decidir solo. Por qué no c: "los primeros N" no es lo que se mostró.

**Ningún hit con score 0 cuenta como acceso**, en `search` ni en `recall_context`. Una fila en 0 está ahí por relleno, no porque la consulta la haya encontrado. Se sigue mostrando como hoy.

**Ceros: ninguna opción en este cambio.** Pasan a lexical-fusion, por la evidencia de arriba.

## Alcance

1. `SearchRequest` gana `min_relative_score: Option<f64>` (`api.rs:229-236`). Valores fuera de `[0, 1]` o `NaN` responden 400.
2. El handler `search` corta (si se pide), filtra los ceros y registra sólo lo que devuelve.
3. `build_memory_prompt` registra sólo los hits `memory` con score > 0.
4. `DaemonClient::search` manda el ratio. `rag-eval --agent-view`, que usa ese cliente (`agent_view/runner.rs:124`), mide lo mismo que ve el agente.

## Fuera de alcance

- El orden entre hits con score 0 y ocultarlos: van con lexical-fusion.
- Corregir los `access_count` ya inflados. No hay forma de saber qué accesos fueron reales, y reescribir contadores guardados no cumple la regla de no tocar datos sin necesidad.
- El flush final del access tracker al parar (`access.rs:68`): es de data-preservation.
- Cambiar el ratio 0,6 o el mínimo de 2.
- Un corte relativo para `recall_context`.

## Éxito (umbrales comprometidos)

`cargo test --workspace` sin fallas nuevas contra la base. Tests nuevos que fallan hoy:

- un hit cortado por relevancia en `/v1/search` no incrementa `access_count` ni mueve `last_accessed_at`;
- un hit con score 0 no cuenta como acceso, ni en `search` ni en `recall_context`;
- `min_relative_score` ausente devuelve los mismos hits que hoy; fuera de rango da 400.

`rag-eval --real --diff` contra el baseline:

| Métrica | Hoy | Umbral |
|---|---|---|
| R@1 / R@5 / R@10 | 0,4348 / 0,7826 / 0,8913 | sin bajar ninguna |
| MRR / nDCG@10 | 0,5979 / 0,6693 | sin bajar ninguna |

`rag-eval --real --agent-view --diff`:

| Métrica | Umbral |
|---|---|
| hit rate natural (0,891) | sin bajar |
| casos perdidos respecto al baseline | 0 |
| tokens de `search_memory` | sin crecer |

El ranking del core no cambia, así que se espera un diff vacío. Las corridas de eval no registran accesos (`crates/clawg-core/src/eval_api.rs:461` usa un tracker inerte): el efecto sobre los accesos lo prueban los tests.

## Datos guardados

- No cambia el esquema, ni identificadores, ni estados, ni el contrato de embeddings.
- Sólo cambia qué filas reciben `access_count + 1` a partir de ahora. Ninguna fila se borra, se oculta ni se reescribe.
- No hace falta migración. El rollback es revertir los commits; los contadores vuelven a inflarse como hoy.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Menos accesos frena la promoción de tier de filas que sí se usan | Baja | Sólo se dejan de contar filas que el agente no vio o con score 0. Lo que el agente lee sigue contando igual |
| Un daemon viejo ignora `min_relative_score` y sigue inflando | Media en la transición | serde ignora campos desconocidos en `SearchRequest`; el puente mantiene su corte, así que el agente ve lo mismo. Se corrige al reiniciar el daemon con la versión nueva |
| Un caller externo de `/v1/search` esperaba que todo lo devuelto contara | Baja | El campo es opcional: sin él sólo cambia que los ceros no cuentan |

## Design

No hace falta `design.md`. Las alternativas están en la exploración, y la elección y su porqué quedan arriba. Lo que resta es comportamiento, y la spec lo fija con escenarios.
