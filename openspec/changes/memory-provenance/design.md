# memory-provenance — Diseño

## D1. Se marca lo inferido, no lo dicho

Las filas guardadas antes de este cambio dicen `explicit` / `user` aunque el usuario no las haya dicho, y no se reescriben. Una fila declarada con `user_stated` después del cambio queda igual: `explicit` / `user`. Las dos son indistinguibles.

| Alternativa | Por qué no |
|---|---|
| Marcar `user` en lo dicho por el usuario | Marcaría todas las filas anteriores como dichas por el usuario: el lector afirmaría lo mismo que hoy afirma la base. |
| Reescribir las filas anteriores a `unknown` | La exploración descarta reescribirlas, y la decisión fue dejarlas como están. |
| Una fecha de corte guardada en la base | Un dato más que mantener, y una versión anterior que se vuelva a abrir escribe `user` después del corte. |

**Elegido:** los lectores marcan sólo lo que nunca fue ambiguo. `created_by_type = 'agent'` en una fila `explicit` sólo lo escribe esta versión, y `synthesis` e `import` son valores nuevos. Una fila sin marca es «dicha por el usuario, o anterior a que se registrara el origen». El orden de un par pone primero el lado sin marca: una inferencia nunca va antes que algo que pudo decir el usuario.

La regla vive en `crates/clawg-core/src/memory/provenance.rs`, pura:

| `source` | `created_by_type` | Marca |
|---|---|---|
| `synthesis` | cualquiera | `conclusion` |
| `import` | cualquiera | `imported` |
| `explicit` | `agent` | `inferred` |
| cualquier otro | cualquier otro | ninguna |

## D2. `synthesis` es un valor de la fila, no del evento

El evento de un save sigue siendo `MemorySource::Explicit`: es el único origen que honra `memory_type`, `importance` y `derived_from` (`crates/clawg-core/src/memory/events.rs`, `sources()`). `MemorySource` suma la variante `Synthesis`, que sólo produce el sello, y nunca un evento.

Alternativa: un enum aparte para la columna. Duplica los cuatro valores que ya comparten evento y fila por un solo valor nuevo.

## D3. `user_stated` con `derived_from` se rechaza

Una conclusión generaliza varias memorias: la escribe el agente aunque las fuentes las haya dicho el usuario. Ignorar la declaración en silencio le diría al agente que se guardó lo que pidió. El 400 sale antes del outbox, como el de `memory_type`.

## D4. Un evento sin `user_stated` es del agente

Un evento del outbox escrito por la versión anterior, o de un puente viejo, no trae el campo. Sellarlo `user` seguiría afirmando lo que no se sabe; sellarlo `agent` es lo que vale para cualquier save sin declaración. `#[serde(default)]` hace que se lea sin fallar.

## D5. El orden va en `memory_status`, no en ACTIVE CONFLICTS

`memory_status` lista pares pendientes, que el agente juzga: ahí el lado sin marca va primero (`#4 close to #9 inferred`). ACTIVE CONFLICTS agrupa por la fila `disputed` (NEW y sus OLD), y reordenarla rompería el grupo. Ahí sólo se marca cada lado, en el rótulo o junto al id citado.
