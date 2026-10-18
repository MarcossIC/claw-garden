# revision-eval — Exploración

## De dónde sale

[topic-revisions](../archive/2026-10-18-topic-revisions/exploration.md#compuerta-previa) (27) proponía, antes de escribir código, una compuerta: medir con el modelo real cuántas veces una decisión que evoluciona termina como dos filas vivas. Esa medición iba a fijar la prioridad del cambio. Cuando se verificó, el código ya estaba en `main` y medirla ahí no cambiaba ninguna decisión, así que se cerró sin correrla y pasó a este cambio.

La verificación con Claude Code sobre el perfil dev sumó otra pregunta. Un agente en una sesión nueva encontró la decisión con `search_memory` y `read_memory`, no vio su clave, inventó otra, y la revisión no ocurrió ([verify](../archive/2026-10-18-topic-revisions/verify.md#perfil-dev-con-claude-code-2026-10-18)). topic-revisions mostró entonces la clave en los hits y en `read_memory`. Si eso alcanza para que el agente la reuse, se mide acá.

Este cambio **sólo mide**. Con lo que salga se decide si conviene cambiar, agregar o quitar algo, y cada arreglo es un cambio aparte.

## Lo que hay hoy (verificado contra el código)

- **Sin clave**, una versión nueva de una decisión es otra fila. El detector la empareja con la vieja sólo si la distancia cae en la banda `[NEAR_DUPLICATE_FLOOR, CONTRADICTION_THRESHOLD)` = `[0,10, 0,42)` (`crates/clawg-core/src/memory/contradiction.rs:37`, `:45`). El par escala al usuario si las dos son `user_preference` o `architectural_decision` y quedan por debajo de `ESCALATE_BELOW` (0,40, `:63`), o comparten un identificador por debajo de `IDENTIFIER_ESCALATE_BELOW` (0,30, `:70`); si no, queda `pending` para que lo juzgue el agente. Fuera de la banda no hay ninguna marca, y las dos filas puntúan igual.
- **Con clave**, la transacción que termina el save deja `superseded` a la fila viva del alcance (`revise_in`, `crates/clawg-core/src/memory/topic.rs:111`, desde `MemoryStore::finish_save`, `:174`).
- **Dónde ve el agente la clave:** en `list_memories` (` topic=K`, y ` by=#N` en una fila reemplazada), en cada hit propio de `search_memory` y en `read_memory` (`crates/clawg-cli/src/mcp/tools.rs`). `recall_context` no la muestra (`crates/clawg-core/src/memory/prompt_builder.rs`).
- **Lo que topic-revisions dejó afuera a propósito:** el aviso «pasá `topic_key`» en la respuesta de un save sin clave, y la búsqueda por clave en `search_memory`, porque cambian tokens o ranking y se miden aparte ([propuesta](../archive/2026-10-18-topic-revisions/proposal.md)).
- **Herramientas que ya existen:** `EvalIngest` guarda por el mismo worker que el daemon (`crates/clawg-core/src/eval_api.rs:481`); `conflict-eval --flow` reproduce un flujo de saves con su tipo (`crates/clawg-eval/src/bin/conflict-eval/main.rs:65`, `FlowSave` en `measure.rs:60`, que no lleva clave); `rag-eval --agent-view` cuenta lo que las herramientas le dan al agente.
- **El golden set no tiene ningún caso con clave.** Por eso el `rag-eval` de topic-revisions no movió nada y no puede ver una revisión.

## Qué se mide

| # | Pregunta | Cómo |
|---|---|---|
| M1 | Sin clave, ¿cuántas veces la versión vieja queda viva sin marca? Es la compuerta de topic-revisions | Un set de ≥ 30 decisiones que evolucionan en 2 o 3 versiones, cortas y largas, en ES y en EN, guardado por `EvalIngest` con el modelo real. Por versión: cuántas quedan escaladas, cuántas `pending` y cuántas fuera de la banda; y la posición de la vieja y de la nueva en `search_memory` con la consulta de la última versión |
| M2 | Con clave, ¿la nueva queda arriba y la vieja desaparece del ranking? | El mismo set con su clave: una sola fila viva por clave, la vieja fuera de KNN y del índice de texto, y la posición de la nueva frente al resto del corpus |
| M3 | ¿El agente pone una clave al guardar una decisión, y la reusa en otra sesión? | Sesiones de `claude -p` con `--strict-mcp-config` sobre el perfil dev, como en las verificaciones: la sesión 1 guarda una decisión y una sesión nueva la cambia. Se anota si la sesión 1 pasó clave, si la nueva la reusó tal cual, por qué herramienta la vio y si hubo revisión. Es caro y no corre en CI |
| M4 | ¿Cuánto cuesta mostrar la clave? | Tokens de `search_memory` con un corpus que tenga claves (`--agent-view`), y lo que costaría mostrarla en `recall_context` |
| M5 | ¿Cuántos saves sin clave son en realidad revisiones? | El flujo de un store real, como el que usó conflict-candidates (los textos fuera del repositorio, sólo los números en `docs/eval/results/`): cuántos saves caen cerca de uno anterior del mismo tema. Decide si vale el aviso «pasá `topic_key`» |

## Decisiones que alimenta

- [ ] ¿El aviso «pasá `topic_key`» en la respuesta de un save sin clave que cae en la banda? (M5, M4)
- [ ] ¿Búsqueda por clave en `search_memory`? Cambia el ranking y exige un `rag-eval --diff` sin movimientos para las consultas sin clave. (M2, M3)
- [ ] ¿Mostrar la clave en `recall_context`? (M3, M4)
- [ ] ¿Quitar algo? Si M3 muestra que el agente casi nunca pone ni reusa la clave, puede no valer lo que cuesta mantener reglas como la de la pieza compartida (T6 de topic-revisions).
- [ ] history-import (44) trae notas de engram, el 75 % con clave de tema. Lo que digan M1 y M3 también dice cuánto rinde mapearlas a `topic_key`.

## Lo que hace falta

- **El set de M1 y M2** es texto escrito para la medición, no datos de un usuario, así que se versiona en `crates/clawg-eval/data/`, como `conflict-pairs.json`.
- **Un modo que guarde versiones con clave y consulte la última:** `FlowSave` podría sumar `topic_key` y una consulta, o puede ser un binario aparte. Se decide en la propuesta.
- **M3** necesita un guion repetible de sesiones y un número de corridas que dé un intervalo útil. Cada corrida cuesta llamadas al modelo.

## Dependencia y tamaño

Depende de topic-revisions (27, archivado). Conviene antes de history-import (44), que trae claves en volumen. Tamaño M.
