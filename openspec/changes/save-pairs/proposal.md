# save-pairs — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — | ver [`state.yaml`](state.yaml) |

## Problema

Sale del análisis A1 de los rechazos `not in dispute` de `resolve_conflict` (`docs/completed-impl/hipotesis/A1/informe.md`). 216 de los 220 rechazos venían de que el agente pasaba otra pieza del save largo que había nombrado a la vieja. Esa causa ya está cerrada: `read_pair` acepta cualquier pieza de cada lado (`crates/clawg-core/src/memory/revision/dispute.rs:294-323`, con `SAVE_PAIRS_SQL` en `:276-286`). Pero ese arreglo se apoya en dos cosas: que dos saves tengan un solo par (`:318-322`) y que las piezas del save estén enlazadas en `memory_piece` (`save_rows!`, `:263-270`). Quedan dos casos en que una de las dos falla. Los dos se verificaron contra el código de hoy.

1. **Un save reanudado puede emparejarse dos veces con el mismo save.** `store_pieces` deja el par sólo en la primera pieza que encuentra cada save viejo, con un `HashSet<SaveKey> paired` que arranca vacío en cada toma (`crates/clawg-core/src/memory/ingest/worker.rs:1068-1070`, filtro en `:1089`). Las piezas anotadas por una toma anterior se toman tal como quedaron y no pasan por el detector (`gate_pieces`, `worker.rs:878-898`). Entonces una pieza que queda para después de la pausa vuelve a anotar un par con un save que ya tiene uno. Juzgar con una pieza que no tiene el par directo da `SeveralPairs` (`dispute.rs:321`), que también es un 409. Una toma se reanuda en tres casos:
   - una pausa del apagado (`worker.rs:393-399`);
   - un proceso muerto a mitad del save, cuya fila vuelve a correr en el siguiente arranque;
   - `clawg outbox retry` de una fila `failed`, que conserva sus piezas (`crates/clawg-core/src/runtime.rs:317-329`).

   Lo mismo pasa al revés: si mientras el save estaba pausado otro save X se emparejó con una de sus piezas, una pieza posterior puede emparejarse con X.
2. **Un save cuyo cierre falla nombra pares que nadie puede juzgar.** Todo save que llega por el daemon tiene su fila en el outbox (`runtime.rs:302`). Lo termina `outbox_finish` (`crates/clawg-core/src/memory/ingest/outbox.rs:475-500`), y si falla sólo queda un `log::warn!` (`worker.rs:743-746`). La respuesta sale igual como `saved`, con `related` (`worker.rs:419-421`; `crates/clawg-cli/src/daemon/api.rs:1276-1292`), pero las piezas quedan sin enlazar hasta el siguiente arranque. Si el agente juzga con el primer id, recibe `NotInDispute` cuando el par está en otra pieza. El `finish_save` que A1 cita (`worker.rs:1155-1169`) sólo lo usan los eventos sin fila, que son tests y evaluación (`crates/clawg-core/src/eval_api.rs:825`). No alcanza a ningún agente, así que en esa forma queda afuera.

A1 no vio ninguno de los dos casos en sus datos. Su peso es bajo, pero cada uno rompe una garantía de la que depende el arreglo de la 0.81.1.

## Decisiones

- **El save reanudado siembra `paired` con lo que ya anotó.** Antes de probar las piezas que quedan, el worker lee los pares de `memory_conflict` de cada pieza reanudada que este save **insertó** (`NotedPiece.inserted`), de cualquiera de los dos lados. Después pasa la otra fila de cada par a su `SaveKey`, con la misma regla que el detector (`crates/clawg-core/src/memory/revision/contradiction.rs:235-238`). La traducción es una función pura, aparte de la lectura. Se descartó anotar los saves emparejados en el progreso de la fila del outbox: cambia el formato de un dato guardado, y una fila escrita por una versión anterior no los tendría.
- **Un cierre que falla no nombra pares.** Si `outbox_finish` falla, la respuesta sigue diciendo `saved` con sus ids, como pide [`specs/pieces`](../../specs/pieces/spec.md) R3. Pero se va sin `related`, sin `revision_candidate` y sin `replaced`. La transacción que se deshizo no escribió ni la marca `topic_key_warned` ni la revisión, así que la línea de aviso tampoco corresponde. La fila queda `pending` con todas sus piezas anotadas, y el siguiente arranque la termina como hoy. Se descartaron dos alternativas:
  - dejar el pedido sin respuesta (`202 unconfirmed`): le diría al agente que no se guardó, cuando sus filas ya están y se encuentran;
  - reintentar el cierre en el mismo proceso: una transacción de SQLite que falló suele volver a fallar, y eso suma una espera en la cola por un caso raro.

## Alcance

1. `clawg-core`, `store_pieces` y `gate_pieces`: la siembra de `paired` en un save reanudado.
2. `clawg-core`, `process_tracked`: el informe de un save cuyo cierre falló.
3. Un test de punta a punta en el puente (`crates/clawg-cli/src/mcp/server.rs`, mod tests), como guarda de lo que ya hace el código.
4. Al archivar: `openspec/specs/disputes`, `openspec/specs/pieces` y `openspec/specs/storage`, y la línea del outbox en `docs/reference/daemon.md`.

## Fuera de alcance

- **Otro origen de `SeveralPairs`, encontrado al leer el código y sin test.** Una pieza que el dedup absorbe en una fila existente se enlaza al save nuevo (`specs/pieces` R3), pero no siembra `paired`. Si esa fila ya tenía un par con un save O, una pieza nueva puede anotar otro par con O. Es el caso de reescribir un save largo sin `topic_key`. Sembrar también desde las filas absorbidas haría que el save nuevo dejara de nombrar a O aunque su texto nuevo esté cerca. Decidir cuál de los dos pares vale merece un cambio propio.
- `related` y la marca `topic_key_warned` de un save reanudado cuentan sólo las candidatas de la última toma (`worker.rs:737-738`, `:1071`). Nadie lee esa respuesta: la pausa la deja sin contestar (`worker.rs:398`) y el replay no tiene a quién contestar. La marca sí se pierde, y eso sesga poco la medición de topic-key-warning.
- El bloque viejo del protocolo en `CLAUDE.md` (R1 de A1) y el texto del error para pares que ningún save anotó (R4 de A1).
- `crates/clawg-cli/src/daemon/api.rs`, el texto de `save_memory`, el esquema y los datos.

## Cómo se mide el éxito

Hay un test por requisito que hoy falla, con el embedder falso:

- el save reanudado da `SeveralPairs` y dos pares con la misma vieja;
- el cierre que falla nombra a la vieja en `related`.

El test de punta a punta pasa hoy y queda como guarda.

## Datos y vuelta atrás

No hay esquema nuevo, ni migración, ni formato nuevo del outbox. Los pares dobles que ya existen se quedan como están: `SeveralPairs` los lista, y cada uno se juzga por sus dos ids. Para volver atrás basta con el binario anterior.
