# save-pairs — Verificación

Veredicto: **cumple**. Ningún hallazgo CRITICAL. Dos WARNING de texto, para resolver al archivar, y tres SUGGESTION.

## La suite

Informado por la implementación, sobre el último commit de código y en Windows: `cargo test --workspace --target-dir target-test` pasa, y también `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings` y `cargo clippy -p clawg-cli --all-targets --target-dir target-test -- -D warnings`. Esta verificación es de lectura: no volvió a correr cargo. El cambio no toca el ranking, así que no corre `rag-eval`.

## Requisitos

| Requisito | Código | Test | Veredicto |
| --- | --- | --- | --- |
| R1: un save reanudado se empareja una vez con cada save | `gate_pieces` junta en `Gated.resumed_rows` sólo las piezas reanudadas con `inserted` (`crates/clawg-core/src/memory/ingest/worker.rs:910`). `store_pieces` siembra `paired` con `saves_already_paired` antes del bucle (`worker.rs:1090`). La lectura de `memory_conflict` en los dos lados y de `read_links` va en una sola llamada (`crates/clawg-core/src/memory/revision/contradiction.rs:267`). | `a_resumed_save_pairs_once_with_an_earlier_save` (`worker.rs:4141`) | Cumple. Sin la siembra, la segunda toma anota `(última pieza, O)` y el juicio con la primera pieza da `SeveralPairs`: el test falla por las dos aserciones. |
| R1: un save hecho mientras el otro esperaba | La otra fila de un par en que la pieza reanudada es `old_id` (`contradiction.rs:248`). | `a_resumed_save_pairs_once_with_a_save_made_while_it_waited` (`worker.rs:4166`) | Cumple. Sin la siembra, la última pieza anota `(última, X)` además del par de X: el test falla. |
| R1: una pieza absorbida no siembra | `if done.inserted` (`worker.rs:909-911`) | `a_resumed_absorbed_piece_does_not_seed_the_pairs` (`worker.rs:4190`) | Cumple. Es una guarda: pasa sin el arreglo, y falla si la siembra tomara también las filas absorbidas. |
| R1: la traducción es pura y usa la regla del detector | `saves_paired_with` (`contradiction.rs:248`) y `save_key` (`contradiction.rs:242`), la misma función que usa ahora `detect_contradiction` (`contradiction.rs:237`) | `the_saves_a_resumed_save_already_paired_with` (`contradiction.rs:380`): los dos lados, una fila de un save enlazado, un par ajeno y la entrada vacía | Cumple. |
| R2: un cierre que falla no nombra pares | `process_tracked` vacía `related` y deja `revision_candidate` en `None` cuando `outbox_finish` o `finish_save` fallan (`worker.rs:750-760`). `replaced` queda en el `Vec::new()` inicial. `finish_save` devuelve el `Result` (`worker.rs:1175`), y el log no cambia. Sin `related` ni `revision_candidate`, la respuesta `201` los omite (`skip_serializing_if`, `crates/clawg-cli/src/daemon/api.rs:613-623`) y `save_memory` no agrega ni la línea de cercanas ni el aviso (`crates/clawg-cli/src/mcp/server.rs:930-987`). | `a_save_whose_finish_fails_names_no_pairs` (`worker.rs:4220`) | Cumple. Antes del arreglo, el error de `outbox_finish` sólo vaciaba `replaced`: `related` nombraba a O y el test falla en `report.related.is_empty()`. El `TEMP TRIGGER` alcanza al cierre porque el store de test tiene una sola conexión en memoria. |
| R3: el primer id de la respuesta juzga el par | Sin cambios de código: `read_pair` desde la 0.81.1 | `a_long_save_is_judged_through_the_first_id_it_answers` (`crates/clawg-cli/src/mcp/server.rs:4198`) | Cumple como guarda. Comprueba la línea `saved`, la línea de cercanas, que el par es de una pieza que no es la primera, el juicio sin error y `superseded`. |

## Convenciones

- Ningún test, comentario ni mensaje de commit nombra el cambio, un plan o una fase. Las palabras «save pairs» de dos nombres de test son la frase que describe la conducta, no el nombre del cambio.
- Sin `#[allow]`, sin código muerto. La única visibilidad nueva es `saves_already_paired`, `pub(crate)`, y la usa `worker.rs`. `read_links` y `SaveLink` ya eran `pub(crate)`.
- Ningún test deja archivos en el temp del sistema. Los tests de `clawg-core` usan `MemoryStore::new_for_test`, en memoria. El de `clawg-cli` usa `bridge_with_store` y `elsewhere`, cuyos `TempDir` se borran al terminar.
- `as_chunks` (`ff597dcd`) es estable desde Rust 1.88, y el `rust-version` del workspace es 1.90.
- Las tareas pedían `NanAtEmbedder` y un embedder por texto. La implementación los juntó en `PiecesAt` (`worker.rs:4094`), que ubica cada pieza por su texto y puede devolver NaN en una posición del lote. La cobertura es la misma.

## Hallazgos

### WARNING

1. **R2 promete una marca que el replay no escribe.** La spec dice que el siguiente arranque «enlaza las piezas y anota la marca que corresponda». En el replay todas las piezas vuelven reanudadas, el bucle de `store_pieces` no prueba ninguna, `related` queda vacío y `unkeyed_revision` da `None`. Entonces la marca `topic_key_warned` no se escribe. Es el límite que la propuesta ya deja fuera de alcance («La marca sí se pierde»), pero al llevar R2 a `specs/pieces` R3 la frase tiene que decir que el replay enlaza las piezas y no escribe la marca. El test no comprueba la marca, así que nada lo contradice.
2. **La línea del outbox de `docs/reference/daemon.md`** todavía no dice qué responde un save cuyo cierre falló. Ya está en la tarea 5. Se anota acá porque esa respuesta cambió: antes nombraba los pares, ahora sale sin ellos.

### SUGGESTION

1. **Un save de una sola pieza cuyo cierre falla también pierde `related`.** Su par está en el único id, que el agente sí puede juzgar. R2 pide vaciarlo siempre y el costo es bajo, porque un save de una pieza sólo llega al cierre con una revisión o un aviso. Si alguna vez importa, la condición puede ser «dos piezas o más».
2. **Una toma reanudada con otro corte del texto no siembra.** Si `resume_positions` no encaja (`worker.rs`, `gate_pieces`, el aviso «noted for a different split»), el gate vuelve a absorber las piezas ya insertadas como filas existentes (`inserted` falso). Así no siembran, y es el mismo caso de las filas absorbidas que la propuesta deja fuera de alcance. Sólo pasa si cambia el chunker entre la toma cortada y la siguiente, por ejemplo con una actualización entre un proceso muerto y el próximo arranque.
3. **Fuera de alcance, anterior a este cambio.** En `store_pieces`, `paired.insert` corre antes de `insert_unless_twin`. Si la pieza termina absorbida por un gemelo que apareció mientras se calculaba el vector (`Insert::Absorbed`, `crates/clawg-core/src/memory/ingest/store_insert.rs:370`), sus candidatas no se escriben, pero el save queda marcado como emparejado. Una pieza posterior ya no anota el par, y entre los dos saves no queda ninguno. Es raro, porque hace falta un `restore` en medio de un save, pero merece su propio cambio.

## Pendiente en local

- El CI en verde en Linux y Windows.
- Los tests usan vectores ubicados a mano o el embedder falso. Con el modelo real no se comprobó que un save largo real se corte y retome, y no deje dos pares con la misma vieja. Hay tres formas de cortarlo: una pausa del apagado, un proceso muerto a mitad del save, o `clawg outbox retry` de una fila `failed`. La comprobación: después del arranque siguiente, `memory_conflict` tiene a lo sumo un par entre el save y cada vieja, y `resolve_conflict` con el primer id no da `SeveralPairs`.
- El fallo del cierre sólo se provoca con un trigger. No hay forma de verlo con un save real sin romper el store a propósito.
