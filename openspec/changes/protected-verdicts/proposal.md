# protected-verdicts — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 4 | S | — (antes de 44) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`e57a563`).

1. **Un par protegido se reemplaza sin el usuario.** La guarda de `judge_pair` frena `supersedes` sólo si el par está `pending` (`crates/clawg-core/src/memory/dispute.rs:351-356`). El mismo agente pasa juzgando antes `conflicts` (el par queda `escalated`, `:382`) o `compatible` (queda `judged`); un par que nació `escalated` al guardar (`crates/clawg-core/src/memory/store_insert.rs:258`) ni siquiera necesita ese paso, y un par sin fila (`state` `None`, `dispute.rs:343-349`) tampoco. El test `a_protected_pending_pair_is_not_superseded_without_the_user` fija la secuencia `conflicts` → `supersedes` como válida (`dispute.rs:1013-1015`).
2. **El tipo se puede bajar.** `update_memory_core` escribe el `memory_type` pedido sin mirar el que tenía (`crates/clawg-core/src/memory/admin.rs:236-247`): pasar una `user_preference` a `semantic_fact` la saca de la guarda.
3. **El texto contradice a la guarda.** La herramienta dice que una preferencia o una decisión se reemplaza «only after the user answered» (`crates/clawg-cli/src/mcp/server.rs:1187-1188`) y la spec justifica el par escalado con una consulta que nada registra (`openspec/specs/disputes/spec.md:115`); el modelo no tiene dónde declarar que el usuario contestó, así que el camino legítimo y el descuidado son la misma secuencia de llamadas.

## Objetivo que sirve

1 (lo que el usuario fijó como preferencia o decisión no se pierde por un veredicto) y 4 (varios agentes escriben el mismo cerebro y el usuario controla lo que vale). Va antes de 44 (history-import), que trae preferencias y decisiones en volumen.

## Decisiones

Las abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **Registro:** A-mín. La declaración viaja como `user_answered: true` y la respuesta del usuario en `reason`, que el par ya guarda; sin columna ni migración.
- **Qué lado:** sólo la fila que pierde (`other`). Deshacer un veredicto, o que una decisión reemplace a un hecho, no pide declaración.
- **Origen:** no se suma B. La guarda mira el tipo, no `created_by_type`; `specs/provenance` no cambia.
- **Cambio de tipo:** (i). Bajar una fila de `user_preference` o `architectural_decision` a otro tipo pide `user_answered: true`; el `PATCH` no guarda razón y no se registra. Promover no pide nada.
- **`replaced` de una revisión por clave:** sin cambio. Una revisión es un acto explícito y reversible (exploración, «Otros caminos»).
- **Compatibilidad:** un puente anterior no manda `user_answered` y se lee `false`, el lado seguro. No se agrega aviso: el `409` ya dice qué falta.

## Alcance

1. `dispute.rs`: `supersedes` sobre un `other` protegido falla con `NeedsUser` en cualquier estado del par, salvo con `user_answered`; `user_answered` sin `reason` falla con `AnswerRequired`; el `reason` de un veredicto declarado se guarda como `user answered: <reason>`.
2. `admin.rs`: `MemoryPatch` lleva `user_answered`; `update_memory_core` rechaza bajar una fila protegida sin él con `UpdateError::NeedsUser`.
3. Daemon: `user_answered` en `JudgeRequest` y en `UpdateRequest`; `AnswerRequired` es `400` y el descenso sin declaración `409`.
4. Cliente y puente: el argumento en `client.judge`, `client.update`, `ResolveConflictArgs` y `UpdateMemoryArgs`; descripciones de las dos herramientas y `protocol.md`; contrato bendecido.
5. Docs: `.claude/CLAUDE.md`; al archivar, `specs/disputes`, `specs/mcp` y `specs/update`.

## Fuera de alcance

- Una frontera de acceso o una identidad del usuario: el agente puede declarar `user_answered` sin preguntar, igual que `user_stated`. La guarda convierte un descuido en una declaración explícita y registrada; no detiene a un agente que miente.
- Una columna `judged_by` y su migración; proteger por origen; una CLI de veredictos.
- `topic_key`, `delete_memory` y el daemon llamado sin `agent_id`.
- El detector, el ranking, los vectores y el prompt de contexto.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sobre un `MemoryStore` en memoria, sin modelo ni red: `supersedes` sobre una protegida en un par `escalated` (por `conflicts` o desde el save), `judged` y sin fila es rechazado y no cambia nada; con `user_answered` y `reason` pasa y el par guarda la razón marcada; deshacer pasa sin declaración; bajar el tipo de una protegida sin declaración es rechazado; el daemon responde `409` y `400` con los textos nuevos.

`rag-eval` no aplica: no cambia ranking, vectores ni prompt. `conflict-eval` no pasa por `judge_pair` (`rg judge_pair crates/clawg-eval` no da nada), así que tampoco.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding. Los pares y filas que una versión anterior reemplazó siguen como están, y se deshacen como antes (R3 de `specs/disputes`), sin declaración porque la protegida es `keep`. Volver atrás es volver al binario anterior.
