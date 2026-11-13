# verdict-on-replace — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — ; se coordina con [topic-key-warning](../../topic-key-warning/proposal.md) (68) y [memory-delivery](../../memory-delivery/proposal.md) (127) | propuesta; las cuatro decisiones abiertas de la exploración se cerraron con su opción recomendada |

## Problema

La [exploración](exploration.md) muestra que, después de un save con memorias cercanas, el agente gasta un pedido al modelo en juzgar cada par: 382 pedidos que sólo juzgan en los transcripts medidos, el 2,7 % de todo el contexto leído. El 95 % de esos veredictos es `compatible`, que no cambia lo que se recupera ([`specs/disputes`](../../../specs/disputes/spec.md), R2), y se da sin leer la memoria vieja. El detector no puede elegir qué pares pedir: los reemplazos ocupan toda la banda de distancias. El trabajo lo piden dos textos:

- la línea de la respuesta del save, que ofrece `compatible` o `scoped` cuando valen las dos (`related_text`, `crates/clawg-cli/src/mcp/server.rs:391-411`);
- el protocolo, que ordena juzgar cada cercana (`crates/clawg-cli/src/mcp/protocol.md:24-27`, `protocol.es.md:24-27`).

Y un tercero lo vuelve a pedir más tarde: `memory_status` lista los pares pendientes y dice cómo juzgarlos (`format_open_pairs`, `crates/clawg-cli/src/mcp/tools.rs:292-335`).

## Alcance

En `crates/clawg-cli`, en el puente MCP:

- **La respuesta de `save_memory`.** La línea sigue nombrando cada cercana con su extracto y la marca de las que van a la persona. Pide `resolve_conflict` con `supersedes` si este save reemplaza una, y con `conflicts` si sólo la persona puede decir cuál vale. Dice que, si valen las dos, no hay que hacer nada. Ya no nombra `compatible` ni `scoped`.
- **La respuesta de `restore_memory`**, que nombra las memorias guardadas mientras la restaurada estuvo borrada (`tools.rs:497-505`): pide lo mismo que la del save.
- **El protocolo**, en inglés y en español: la frase «juzga cada una» pasa a decir lo mismo que la respuesta. La apertura sigue entrando en sus 2.048 caracteres.
- **`memory_status`.** Deja de listar los pares pendientes y de pedir que se juzguen. Sigue diciendo cuántos pares esperan a la persona en ACTIVE CONFLICTS.
- **La descripción de `resolve_conflict`**, en lo que diga que el agente juzga los pares que nombra `memory_status`.
- **El contrato fijado** (`agent_contract.json` y `agent_contract.es.json`), las specs [`mcp`](../../../specs/mcp/spec.md) y [`disputes`](../../../specs/disputes/spec.md), y `docs/reference/daemon.md` donde describa estos textos.

## Fuera de alcance

- El detector: su umbral, su banda y cuántas candidatas nombra no cambian.
- `resolve_conflict` sigue aceptando los cuatro veredictos, y el daemon y su API no cambian. Un puente anterior contra un daemon nuevo, y al revés, se comportan como hoy.
- Los pares se siguen escribiendo como `pending`, con su distancia. Los que ya existen quedan como están.
- Las cercanas que escalan a la persona y ACTIVE CONFLICTS. La nota de ese bloque sigue ofreciendo `compatible` y `scoped` (`crates/clawg-core/src/memory/prompt_builder.rs:846-859`): sobre un par escalado ese veredicto es la respuesta de la persona y lo saca del bloque.
- Avisar que se reuse la `topic_key` de una cercana: es otro cambio.
- El bloque del protocolo ya escrito en el archivo de instrucciones de un usuario se actualiza cuando vuelve a correr `clawg setup`, como con cualquier cambio del protocolo.

## Datos guardados

Ninguno cambia: ni el esquema, ni los estados, ni los pares existentes. No hay migración ni vuelta atrás que preparar; volver al comportamiento anterior es instalar la versión anterior.

## Cómo se mide el éxito

Tests que fallan hoy:

- la respuesta de un save con cercanas no nombra `compatible` ni `scoped`, pide `supersedes` y `conflicts`, y dice que no hay nada que hacer si valen las dos (`save_outcome_text_names_related_memories`, `server.rs`);
- la apertura y el protocolo, en los dos idiomas, ya no dicen «judge each» ni «juzga cada una» (los tests de la apertura en `server.rs`);
- `memory_status` con pares pendientes no los lista ni nombra `resolve_conflict` para ellos, y sigue contando los escalados (`memory_status_counts_pending_candidates`, `server.rs`; los tests de `format_open_pairs`, `tools.rs`);
- el contrato fijado coincide con sus dos archivos después de regenerarlos.

Con uso real, en local, y fijado antes de mirar: sobre los transcripts de algunas semanas con la versión nueva, los pedidos que sólo juzgan por cada save con cercanas bajan de 0,79 a menos de 0,15. Los `supersedes` por cada cien saves con cercanas (hoy 3,3) se informan; con tan pocos casos no deciden.

`rag-eval` y `conflict-eval` no aplican: no cambian el ranking, los vectores ni el detector.

## Riesgos

- **El agente detecta menos reemplazos sin el ritual.** No se puede medir antes. Hoy juzga con 80 caracteres y ya se le escapan; la medición con uso real lo vigila.
- **Los pares pendientes crecen sin que nadie los mire.** No pesan en ninguna lectura. Si algún día molestan, limpiarlos es un cambio aparte.
