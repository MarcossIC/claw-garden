# save-pairs — Especificación

Se suma a [`specs/disputes`](../../specs/disputes/spec.md) («El detector anota candidatas», R3), [`specs/pieces`](../../specs/pieces/spec.md) (R3) y [`specs/storage`](../../specs/storage/spec.md) («Un replay retoma donde quedó»).

## R1. Un save reanudado se empareja una vez con cada save

Cuando una toma retoma piezas que una toma anterior del mismo save insertó (`NotedPiece.inserted`), el worker MUST tratar como ya emparejado cada save que tiene un par en `memory_conflict` con alguna de esas piezas, de cualquiera de los dos lados. Ese save es el `SaveKey` de la otra fila del par. Una pieza que se prueba en esta toma MUST NOT anotar un par con esos saves. Entre dos saves MUST quedar a lo sumo un par, igual que si el save se hubiera guardado en una sola toma.

La siembra MUST NOT salir de una pieza reanudada que el dedup absorbió (`inserted` falso): sus pares los anotó otro save. Pasar de los pares y los vínculos a los `SaveKey` MUST ser una función pura, separada de la lectura.

#### Escenario: una pausa entre dos piezas cercanas a la misma vieja
- GIVEN una fila vieja O, y un save largo explícito cuya segunda pieza y cuya última pieza caen en la banda del detector de O
- AND la primera toma guarda las dos primeras piezas y se corta antes de la tercera
- WHEN la toma siguiente termina el save
- THEN `memory_conflict` tiene un solo par entre el save y O, el de la segunda pieza
- AND juzgar `(keep = la primera pieza, other = O)` con `supersedes` juzga ese par, sin `SeveralPairs`

#### Escenario: un save que se emparejó con una pieza mientras el save estaba pausado
- GIVEN un save largo cortado después de su segunda pieza
- AND otro save X, guardado en el medio, que anotó un par con esa segunda pieza
- WHEN la toma siguiente termina el save, y una pieza posterior cae en la banda de X
- THEN entre el save y X queda un solo par, el que anotó X

#### Escenario: una pieza reanudada que se había absorbido
- GIVEN una fila R con un par anotado contra una fila vieja O, y un save largo cuya segunda pieza tiene el texto exacto de R
- AND la primera toma absorbió esa pieza en R y se cortó antes de la tercera
- WHEN la toma siguiente termina el save, y una pieza posterior cae en la banda de O
- THEN esa pieza anota su par con O, igual que hoy

**Tests:** `a_resumed_save_pairs_once_with_an_earlier_save` y `a_resumed_save_pairs_once_with_a_save_made_while_it_waited`, que fallan hoy, y `a_resumed_absorbed_piece_does_not_seed_the_pairs`, como guarda (`worker.rs`); `the_saves_a_resumed_save_already_paired_with` (la función pura, en `contradiction.rs`).

## R2. Un save cuyo cierre falla no nombra pares

Cuando falla la transacción que termina un save (`outbox_finish`, o `finish_save` para un evento sin fila), el informe MUST seguir sin `failure`, con sus `memories`. Pero MUST llevar `related` y `replaced` vacíos y `revision_candidate` en `None`. La respuesta de `POST /v1/memories` es entonces `201 saved` sin `related`, `replaced` ni `revision_candidate`, y `save_memory` no agrega la línea `close to earlier memories` ni la del aviso. Con una fila del outbox, la fila MUST quedar `pending` con todas sus piezas anotadas. El siguiente arranque la termina, y entonces enlaza las piezas y anota la marca que corresponda.

#### Escenario: el enlace falla
- GIVEN una fila vieja O, y un save largo con fila en el outbox cuya segunda pieza cae en la banda de O
- AND el store rechaza escribir en `memory_piece`
- WHEN el worker termina el save
- THEN el informe nombra las piezas, sin `failure` ni `related`
- AND la fila del outbox sigue con todas las piezas anotadas
- WHEN el store vuelve a aceptar la escritura y se corre el replay
- THEN las piezas quedan enlazadas, la fila desaparece, y juzgar `(keep = la primera pieza, other = O)` con `supersedes` funciona

**Tests:** `a_save_whose_finish_fails_names_no_pairs` (`worker.rs`), que falla hoy porque el informe nombra a O. El fallo se provoca con un `TEMP TRIGGER` que aborta los inserts en `memory_piece`, y el mismo test lo quita antes del replay.

## R3. De punta a punta: el primer id de la respuesta juzga el par

Esto lo cumple el código desde la 0.81.1, y el requisito lo fija contra lo que el agente realmente lee. Un save largo de `save_memory` que nombra una memoria cercana en `close to earlier memories` MUST poder juzgarse con `resolve_conflict(keep = el primer id de la línea saved, other = la cercana, verdict = supersedes)` sin error, aunque el par esté en otra pieza. Después la cercana MUST quedar `superseded`.

#### Escenario
- GIVEN el puente en el proceso del test sobre un daemon con el embedder falso (el mismo `FakeEmbedder` que elige `CLAWG_TEST_EMBEDDER=fake`)
- AND una memoria vieja cuyo vector se ubicó en la banda del detector de la última pieza del texto largo. Ese vector se lee de una copia del mismo texto guardada donde el save del test no la alcanza.
- WHEN se guarda el texto con `save_memory`
- THEN la respuesta empieza con `saved #<primer id>`, nombra a la vieja en `close to earlier memories`, y el par en `memory_conflict` es de una pieza que no es la primera
- WHEN se llama a `resolve_conflict` con ese primer id, la vieja y `supersedes`
- THEN la llamada no es un error y la vieja queda `superseded`

**Tests:** `a_long_save_is_judged_through_the_first_id_it_answers` (`crates/clawg-cli/src/mcp/server.rs`, mod tests, con `bridge_with_store`). Su equivalente en `clawg-core` ya existe: `the_pair_a_long_save_names_is_judged_through_its_first_piece` (`worker.rs:2572`). Ese no pasa por el texto del puente.
