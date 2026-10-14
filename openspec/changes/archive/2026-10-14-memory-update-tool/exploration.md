# memory-update-tool — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | visibility-policy (scope); ingest-durability (ids), deseable | cerrada: las decisiones están en la [propuesta](proposal.md#decisiones-cierran-las-abiertas-de-la-exploración) |

**Tesis:** el agente puede fijar la importancia al guardar, pero después no la puede corregir. Re-guardar el mismo contenido tampoco sirve.

## Problema (verificado)

**PATCH**

- `PATCH /v1/memories/{kind}/{id}` acepta sólo `{importance}` (`crates/clawg-cli/src/daemon/api.rs:344-349, 584-600`; `crates/clawg-core/src/memory/admin.rs:144-195`).
- Recalcula el tier sólo hacia arriba: bajar la importancia **nunca** degrada el tier.

**Herramientas MCP**

- No hay ninguna de update. Las 6 que existen están en `crates/clawg-cli/src/mcp/server.rs:391-479`: `save_memory`, `search_memory`, `recall_context`, `list_memories`, `delete_memory` y `memory_status`.

**Save**

- `save_memory` acepta `importance`: la recorta a 0–10 y el default es 5 (`crates/clawg-core/src/memory/store_insert.rs:126-153`).
- Re-guardar contenido idéntico sólo hace `access_count + 1` (`store_insert.rs:286-293`). La importancia no cambia.

**Ids**

- El agente no recibe ids al guardar, porque la respuesta es `queued`. Para actualizar, primero tiene que buscar o listar.

### El rango de importancia no es el problema

- Ensanchar el rango 0,85–1,0 empeora la recuperación. El sweep 2D midió:

  | Rango de importancia | R@10 | MRR | `importance_rank_dominance` |
  |---|---|---|---|
  | 0,55–1,0 | 0,826 | 0,563 | 0,313 |
  | 0,85–1,0 | 0,891 | 0,598 | 0,095 |

- La herramienta sí es útil, pero por curaduría, no para "hacer ganar" a una memoria importante.

## Alcance candidato

- **Herramienta MCP `update_memory(kind, id, importance?, memory_type?)`.**
- **Fuera de alcance: editar el contenido.** Implica re-embedding, dedup y sonda de contradicción. Eso es un "save + delete" y conviene tratarlo como tal.

## Decisiones abiertas

- [ ] **Campos editables.** ¿Sólo `importance`, o también `memory_type`? Cambiar el tipo cambia el lambda de decaimiento y el tier inicial.
- [ ] **Tier al bajar la importancia.** ¿Se recalcula en las dos direcciones?
- [ ] **Scope.** ¿Quién puede actualizar qué? Lo define visibility-policy. Si este cambio va antes, ¿con el scope actual (agente + proyecto) como mínimo?
- [ ] **Re-guardado.** ¿Re-guardar contenido idéntico con otra importancia debería actualizarla?
- [ ] **Promoción de scope.** ¿`update_memory` puede pasar una memoria de agente a proyecto o a global? Sólo a pedido explícito, nunca automático. Depende de la pregunta 8 de visibility-policy: si se comparte con la columna, promover deja `agent_id` en NULL (CHECK de `crates/clawg-core/migrations/V1__init.sql:30-34`) y la autoría queda en `created_by_ref`.

## Cómo se mide

Tests de la herramienta:

- actualiza la importancia;
- rechaza un id fuera de scope;
- el tier se recalcula según lo que se decida.

No toca el ranking, así que no hace falta `rag-eval`.
