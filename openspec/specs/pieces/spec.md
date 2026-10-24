# pieces — Especificación

Fuente de verdad del comportamiento actual de un save que no entra en un pedazo: cómo se parte y se embebe (con la cabecera del save), cómo se vinculan sus pedazos en `memory_piece`, cómo aparece en la búsqueda (un hit por save, con `piece=K/N`), cómo se lee el save entero y qué pasa con las filas de la versión anterior. Los textos de un pedazo, el ranking de filas sueltas, el detector de contradicciones y el workspace no cambian: siguen en [`specs/retrieval`](../retrieval/spec.md), [`specs/disputes`](../disputes/spec.md) y [`specs/update`](../update/spec.md). La medición (subconjunto `long`, `answer`, métricas de agent-view) está en [`specs/eval`](../eval/spec.md). Qué agente puede leer qué save está en [`specs/visibility`](../visibility/spec.md) (R3 y R4).

**Origen:** [long-save-context](../../changes/archive/2026-10-14-long-save-context/proposal.md), archivado el 2026-10-14. Los ids de requisito son los de esa spec, salvo R9, que pasó a [`specs/eval`](../eval/spec.md). [long-save-ranking](../../changes/archive/2026-10-15-long-save-ranking/proposal.md), archivado el 2026-10-15, cambió el tope de R2 (de 424/64 a 96/24), sumó a R2 los saves partidos con el tope anterior y agregó R9 (el pozo de KNN cuenta saves); su R4 pasó a [`specs/eval`](../eval/spec.md). [topic-revisions](../../changes/archive/2026-10-18-topic-revisions/proposal.md), archivado el 2026-10-18, sumó R10 y la clave de la fila en R6. El save cortado de R3 lo cambió [outbox-lifecycle](../../changes/archive/2026-10-19-outbox-lifecycle/proposal.md), archivado el 2026-10-19: su fila del outbox ya no se olvida y un descarte borra sus pedazos. [agent-authorship](../../changes/archive/2026-10-22-agent-authorship/proposal.md), archivado el 2026-10-22, cambió el «otro agente» de R6 por lo que el lector no alcanza. [piece-boundaries](../../changes/archive/2026-10-24-piece-boundaries/proposal.md), archivado el 2026-10-24, hizo que con un tokenizer cargado todo corte del divisor caiga en frontera de palabra (PB1), que los pedazos y la cabecera de un save también terminen ahí, con la cabecera en 32 tokens (PB2 y PB6), que lo partido antes no cambie (PB4), y lo comprobó con el tokenizer real (PB5). Sus ids eran R1 a R6 y chocaban con los de esta spec; su R3, en lo que toca al índice de docs, está en [`specs/project-docs`](../project-docs/spec.md) (PB3).

## Definiciones

- **Save:** un evento de ingesta (`MemoryEvent`): un `save_memory` o un turno de chat. Es la misma unidad que los pedazos hermanos de [`specs/disputes`](../disputes/spec.md) R8.
- **Texto del save:** el contenido del evento después de enmascarar secretos (`redact`, en `ingest_pieces`, `crates/clawg-core/src/memory/worker.rs:751`). Todo lo de abajo trabaja sobre ese texto.
- **Pedazo:** cada ventana que deja el chunker, después de colapsar los repetidos dentro del mismo evento (`collapse_duplicate_chunks`, `worker.rs:166`). Su posición es 1, 2, … en ese orden.
- **Save largo:** un save con dos pedazos o más.
- **Frontera de palabra:** un byte `b` de un texto es frontera si es su principio o su fin, o si el carácter anterior a `b` o el que empieza en `b` es espacio en blanco (`char::is_whitespace`). Un texto empieza o termina dentro de una palabra si su primer o su último byte, como corte en el texto del que sale, no es frontera.
- **Cabecera:** el texto del save hasta la última frontera de palabra dentro de sus primeros `SAVE_HEAD_TOKENS` = 32 tokens (`chunker/mod.rs:40`), o esos 32 tokens si la primera palabra es más larga, contados con el mismo splitter del chunker (en modo sin tokenizer, 4 bytes por token y sin mirar fronteras).
- **Texto embebido de un pedazo:** el pedazo, o `cabecera + "\n…\n" + pedazo` cuando tiene contexto.
- **Vínculo:** una fila de `memory_piece`: `(save_id, piece, memory_id, context)`.
- **Save de una fila:** si la fila tiene vínculos, el de menor `save_id`; si no, la fila sola, como save de un pedazo.

## R1. Un texto que entra en un pedazo no cambia

Un texto del save que el chunker deja en un pedazo con el tope de `CHUNK_MAX_TOKENS` = 480 (`chunker/mod.rs:26`) MUST partirse, embeberse y guardarse como siempre: el mismo texto, el mismo vector y ningún vínculo. `ChatChunker::split_save` (`chunker/chat.rs:114`) devuelve ese pedazo sin cabecera.

#### Escenario
- GIVEN un save de 300 tokens
- WHEN se guarda
- THEN queda una fila con el texto entero, embebida con ese texto y sin filas en `memory_piece`

**Test:** `a_text_that_fits_one_piece_is_split_as_before` (`chunker/chat.rs`); `a_single_piece_save_embeds_its_text_and_leaves_no_link` (`worker.rs`).

## R2. Un save largo se parte con lugar para la cabecera

Un texto del save que no entra en un pedazo de `CHUNK_MAX_TOKENS` = 480 tokens (`chunker/mod.rs:26`) MUST partirse en pedazos de a lo sumo `PIECE_MAX_TOKENS` = 96 tokens (`chunker/mod.rs:50`), con `PIECE_OVERLAP_TOKENS` = 24 de solapamiento entre vecinos (`chunker/mod.rs:55`; `ChatChunker::split_save`, `chunker/chat.rs:114`), y con un tokenizer cargado los bordes de los pedazos y de la cabecera caen en frontera de palabra (PB1 y PB2). Un pedazo chico deja la respuesta de un save largo como una parte grande de su vector, en lugar de una frase promediada entre muchas. `ChatChunker::chunk` (`chunker/chat.rs:102`), que usan los demás caminos, MUST seguir cortando con `CHUNK_OVERLAP_TOKENS` = 64 (`chunker/mod.rs:32`).

Dos `const _: () = assert!(...)` (`chunker/mod.rs:59-60`) comprueban en compilación que `PIECE_OVERLAP_TOKENS < PIECE_MAX_TOKENS` y que `SAVE_HEAD_TOKENS + HEAD_JOIN_RESERVE + PIECE_MAX_TOKENS <= CHUNK_MAX_TOKENS` (32 y 8 tokens de reserva para el separador `"\n…\n"` y la suma de tokens en el borde), así el texto embebido nunca pasa el límite que el embedder trunca en silencio.

Cada pedazo desde el segundo MUST embeberse con contexto (la cabecera). El primero se embebe sin él, porque ya empieza con la cabecera. La fila MUST guardar sólo el pedazo: el contenido, el hash de dedup y el FTS no llevan la cabecera.

El armado del texto embebido es `embedding_text(context, piece)` (`chunker/mod.rs:66`), una función pura que usan el worker (`worker.rs:877`) y el re-embed (R7).

Un save de un pedazo de hasta 480 tokens no se parte con este tope: el tope de 96 sólo aplica si el de 480 no alcanza. El save más corto que se parte (más de 480 tokens) da al menos 7 pedazos.

**Saves partidos con el tope anterior.** Las filas, vínculos y vectores de un save partido antes con 424 tokens y 64 de solapamiento MUST seguir apareciendo como un hit por save con `piece=K/N` (R4 y R5), leyéndose enteros con `read_save_core` (R6) y re-embebiéndose con su contexto (R7). Ningún camino MUST volver a partir una fila guardada. `join_pieces` (`pieces.rs:67`) MUST unir sin repetir el solapamiento tanto pedazos de 424/64 como de 96/24. Lo mismo vale para los saves partidos con 96/24 antes de que los bordes cayeran en frontera de palabra (PB4).

#### Escenario: el tema llega al vector del pedazo 2
- GIVEN un save de 1.200 tokens que nombra `clawg-ledger` sólo en su primera oración
- WHEN se guarda
- THEN cada pedazo tiene a lo sumo 96 tokens y dos pedazos vecinos comparten unos 24 (los de la frontera de palabra más cercana, PB1)
- AND el embedder recibe el pedazo 2 precedido de una cabecera (las palabras enteras de los primeros 32 tokens del texto) que contiene `clawg-ledger`
- AND la fila del pedazo 2 guarda el pedazo sin la cabecera

#### Escenario: un save del tope anterior
- GIVEN un save de 3 pedazos de 424 tokens, guardado y vinculado como lo dejaba la versión anterior
- AND un save de 12 pedazos de 96 tokens guardado con esta versión
- WHEN se busca por el tema de cada uno y se lee cada uno con `read_save_core`
- THEN cada save aparece en un solo hit y su texto se lee entero, sin solapamiento repetido

**Test:** `a_long_text_is_split_into_small_pieces`, `small_pieces_share_their_own_overlap` y `the_head_is_the_first_tokens_of_the_text` (`chunker/chat.rs`); `a_piece_without_context_is_embedded_as_stored` y `a_piece_with_context_is_embedded_after_the_head` (`chunker/mod.rs`); `later_pieces_are_embedded_with_the_head_and_stored_without_it` (`worker.rs`); `a_save_split_at_the_old_cap_is_still_found_and_read` y `the_pieces_of_a_split_save_join_back_into_its_text` (`pieces.rs`).

## R3. El save deja su vínculo

Cuando un save largo termina sin falla, el worker MUST escribir, en una sola transacción, un vínculo por pedazo (`link_in`, `crates/clawg-core/src/memory/pieces.rs:231`). Si el save tiene una fila del outbox, el vínculo y el borrado de la fila van en la misma transacción (`outbox_finish`, `crates/clawg-core/src/memory/outbox.rs:459`, llamado por `process_tracked`, `worker.rs:679`; [`specs/storage`](../storage/spec.md), «Un save aceptado está en el outbox hasta que termina»); un evento sin fila lo vincula `finish_save` (`worker.rs:987`, por `MemoryStore::finish_save`, `crates/clawg-core/src/memory/topic.rs:174`, que en la misma transacción aplica la revisión de su `topic_key`, ver R10 y [`specs/ingest`](../ingest/spec.md), «Un save con clave reemplaza la fila viva de su clave»):

- `save_id`: un número nuevo, uno más que el mayor de la tabla;
- `piece`: la posición del pedazo, desde 1;
- `memory_id`: la fila que lo guarda, insertada por este evento o absorbida por el dedup (`absorb_duplicate`, `worker.rs:587`);
- `context`: la cabecera con la que se embebió la fila si este evento la insertó con contexto; si no, vacío. Una fila absorbida conserva el vector que ya tenía, así que su vínculo no dice que lo embebió con contexto.

Un save de un pedazo MUST NOT dejar vínculos. Un save que se corta a mitad de camino (`IngestReport.failure`, `worker.rs:84`, o una pausa del apagado, `IngestReport.paused`) MUST NOT dejar vínculos todavía: sus filas quedan sueltas mientras tanto. Si tiene fila en el outbox, la fila MUST quedar aunque el que guardó esperaba la respuesta (que es `202 partially_saved`, [`specs/ingest`](../ingest/spec.md), «Un fallo del worker llega al que guardó»), y el siguiente arranque retoma después de las piezas ya guardadas y escribe el vínculo al terminar ([`specs/storage`](../storage/spec.md), «Un replay retoma donde quedó»); si llega a `failed`, `clawg outbox discard` borra las que insertó ([`specs/storage`](../storage/spec.md), «Una fila `failed` se reintenta o se descarta»). Sin fila (tests y evaluación), al guardarlo de nuevo los pedazos se absorben y el vínculo se escribe entonces.

Si escribir el vínculo falla, el save MUST seguir informándose como guardado: sus filas ya están y se encuentran igual. La falla va al log (`worker.rs:998`, y `worker.rs:725` con una fila en el outbox, que entonces queda para que el próximo arranque la termine).

Guardar otra vez exactamente las mismas filas, en el mismo orden, MUST devolver el `save_id` que ya forman (`existing_save`, `pieces.rs:270`) y no escribir nada: el mismo save no se vincula dos veces.

#### Escenario: pedazos absorbidos
- GIVEN una fila guardada con el texto exacto del pedazo 2 de un save
- WHEN se guarda el save
- THEN el pedazo 2 se absorbe en esa fila y su vínculo apunta a ella con `context` vacío
- AND los otros pedazos se insertan y sus vínculos llevan la cabecera desde el pedazo 3

#### Escenario: save cortado
- GIVEN un embedder que falla en un save largo
- WHEN se guarda
- THEN `memory_piece` no cambia

**Test:** `a_long_save_links_its_pieces_in_order`, `an_absorbed_piece_is_linked_without_context`, `a_save_cut_short_leaves_no_link` y `the_same_long_save_again_links_the_rows_it_left` (`worker.rs`); `link_pieces_numbers_saves_in_order` y `linking_the_same_rows_again_answers_the_save_they_already_form` (`pieces.rs`).

## R4. Un hit por save

La búsqueda (`search_all_memories_inner`, `crates/clawg-core/src/memory/search.rs:1187`) MUST, después de ordenar y antes de cortar en `k`, quedarse con el primer hit de cada save y descartar los demás pedazos de ese save (`collapse_saves`, `search.rs:145`, llamado en `search.rs:1330`; el corte es `search.rs:1336`). Un hit de una fila sin vínculos, o de `episodic_trajectory` o `procedural_skill`, MUST quedar donde está.

Cada hit de una fila con vínculos MUST llevar su posición y el total de pedazos del save (`RankedHit.piece: Option<PiecePosition>`, `search.rs:120-122`; `PiecePosition`, `search.rs:138`). El descarte corre antes del corte: los lugares que liberan los hermanos los ocupan otras memorias.

El descarte es una función pura sobre los hits y los vínculos de sus filas. Si leer los vínculos falla, la búsqueda MUST devolver los hits sin colapsar y dejar un aviso en el log (`search.rs:1329-1335`): es lo de antes, no un error.

#### Escenario: los hermanos no ocupan el top
- GIVEN un save de 4 pedazos que puntúan 1.º, 2.º, 4.º y 5.º para una consulta, y 10 filas sueltas
- WHEN se busca con `k = 10`
- THEN el resultado tiene 10 hits: el pedazo que puntuó 1.º, con `piece = 1` y `pieces = 4`, y las 9 mejores filas sueltas

**Test:** `only_the_best_piece_of_a_save_survives`, `rows_without_links_keep_their_place`, `freed_slots_go_to_other_memories` y `a_link_read_failure_leaves_hits_uncollapsed` (`search.rs`).

## R5. El hit dice qué pedazo es

Un hit con vínculo MUST mostrar `piece=K/N` en su metadato:

- `search_memory` (`crates/clawg-cli/src/mcp/tools.rs:191`): `{rank}. #{id} [{kind}/{type}/t{tier} piece={K}/{N} score={score}] {contenido}`;
- `recall_context` (`format_hit_line`, `crates/clawg-core/src/memory/prompt_builder.rs:763-775`): `[YYYY-MM-DD #{id} {type} piece={K}/{N}] {contenido}` para `memory`; las otras tablas llevan su kind antes del tipo, como ya hacían.
- un hit de otro agente suma ` by <autor>` antes del `]` en los dos ([`specs/sharing`](../sharing/spec.md) R4).

Un hit sin vínculo MUST seguir con el formato de antes. `Hit` en el cliente (`crates/clawg-cli/src/client.rs:181`) MUST leer `piece` con `#[serde(default)]` (`client.rs:197`), así un daemon viejo sigue respondiendo.

Los parsers del eval (`crates/clawg-eval/src/bin/rag-eval/agent_view/parse.rs`) MUST leer los dos formatos y devolver el id de cada línea de búsqueda.

#### Escenario
- GIVEN un hit del pedazo 2 de 4, id 58
- THEN `search_memory` imprime `1. #58 [Memory/semantic_fact/t3 piece=2/4 score=0.81] …` (el `kind` sale como lo serializa el daemon, `Memory`)
- AND `recall_context` imprime `[2026-10-15 #58 semantic_fact piece=2/4] …`

**Test:** `format_hits_names_the_piece_of_a_long_save` (`tools.rs`); `format_hit_line_names_the_piece_of_a_long_save` (`prompt_builder.rs`); `a_hit_line_with_a_piece_yields_its_id_score_and_content` y `a_context_hit_with_a_piece_is_one_segment` (`parse.rs`).

## R6. El save entero

`GET /v1/memories/{kind}/{id}/save` (`crates/clawg-cli/src/daemon/api.rs:1036`, handler `get_save`, `api.rs:1662`) con la query `agent` MUST devolver el save de la fila (`read_save_core`, `crates/clawg-core/src/memory/pieces.rs:104`):

- `id`, `kind`, `piece` (la posición de la fila pedida), `pieces` (el total) e `ids`: el id de cada posición, o `null` si esa fila está borrada o fuera del alcance del lector (`agent`);
- `missing`: las posiciones sin fila visible;
- `text`: los pedazos visibles en orden, sin el solapamiento entre vecinos.
- `topic_key`: la clave bajo la que se guardó la fila pedida (`SaveText.topic_key`, `crates/clawg-core/src/memory/pieces.rs:49`), la de esa fila y no la de sus vecinas; sin clave, el campo no va.

Una fila sin vínculos MUST devolverse como save de un pedazo, con su contenido como `text`. Una fila que no existe, borrada o fuera del alcance del lector, MUST responder 404, como el resto de las rutas por id ([`specs/visibility`](../visibility/spec.md) R4).

Quitar el solapamiento es `join_pieces` (`pieces.rs:67`), una función pura: entre dos pedazos vecinos busca el sufijo más largo del primero que es prefijo del segundo, de al menos 16 caracteres (`MIN_OVERLAP_CHARS`, `pieces.rs:58`, el mismo piso que respeta el divisor, PB1) y dentro de los últimos 2.048 bytes (`MAX_OVERLAP_BYTES`, `pieces.rs:62`), y lo deja una sola vez (`overlap`, `pieces.rs:87`). Si no lo encuentra, une con un salto de línea y no recorta nada. `read_save_core` aplica `join_pieces` a cada tramo de pedazos visibles consecutivos y une los tramos con un salto de línea: nunca compara dos pedazos que no eran vecinos.

El puente MUST exponer `read_memory(id, kind?)` (`crates/clawg-cli/src/mcp/server.rs:1365`, `read_memory_impl`, `server.rs:1065`), con `kind = memory` por defecto y el agente que llama. La respuesta (`format_save`, `crates/clawg-cli/src/mcp/tools.rs:304`) MUST abrir con una línea que dice qué pedazo es y de cuántos, los ids del save y los que faltan, y seguir con `text`. Si la fila pedida tiene clave, esa línea MUST terminar con ` topic=K` (un save de un pedazo abre con `#id topic=K`), salvo que la fila sea de otro proyecto que el del puente, donde un save bajo esa clave no la revisaría ([`specs/mcp`](../mcp/spec.md), «La clave de tema en las herramientas»). Sobre una fila que `sharing.json` le oculta MUST fallar como si no existiera; la de otro agente que alcanza la lee entera.

`save_memory` MUST nombrar `read_memory` en su descripción (`server.rs:1308-1316`) y en la respuesta de un save largo (`saved as N pieces: …; read_memory with any of them returns the whole text`). `search_memory` MUST explicar `piece=K/N` en su descripción (`server.rs:1336-1338`). `tools/list` MUST listar `read_memory` entre sus herramientas; el conteo vigente, doce, y la lista completa están en [`specs/mcp`](../mcp/spec.md).

Contra un daemon que no tiene la ruta, su 404 no trae el error JSON del daemon: `route_failure_text` (`crates/clawg-cli/src/mcp/server.rs`) MUST decir que hay que reiniciar el daemon, no que la fila no existe.

#### Escenario: lo propio, con un pedazo borrado
- GIVEN un save de 4 pedazos del agente A, con el pedazo 3 borrado
- WHEN A llama `read_memory` con el id del pedazo 2
- THEN la respuesta dice `piece 2 of 4`, lista los ids con el 3 como faltante y trae los pedazos 1, 2 y 4: el 1 y el 2 sin el solapamiento repetido, el 4 después de un salto de línea

#### Escenario: lo ajeno
- GIVEN un save del agente A
- WHEN B llama `read_memory` con uno de sus ids
- THEN recibe el save entero; con `hide B A`, un error, igual que con un id que no existe

**Test:** `join_pieces_drops_the_overlap_once`, `join_pieces_keeps_both_sides_when_nothing_overlaps`, `the_pieces_of_a_split_save_join_back_into_its_text` y `read_save_lists_missing_pieces_and_another_agent_reads_it_whole` y `a_hidden_authors_piece_reads_as_missing` (`pieces.rs`); `get_save_returns_the_whole_text` y `get_save_of_another_agent_reads_whole_unless_hidden` (`crates/clawg-cli/tests/api.rs`); `read_memory_returns_the_whole_save`, `another_agent_reads_a_save_unless_hidden`, `a_daemon_without_the_save_route_is_named_as_such`, `save_memory_asks_for_one_fact_per_call`, `search_memory_explains_the_piece_of_a_long_save` y `save_outcome_text_covers_every_daemon_answer` (`server.rs`); `format_save_names_the_piece_and_the_missing_ones` (`tools.rs`) y `format_save_names_the_topic_key_of_the_row` (`tools.rs`); `read_save_carries_the_key_of_the_row_asked_for` (`pieces.rs`) y `a_search_hit_and_the_save_read_carry_the_topic_key` (`tests/api.rs`); el conteo de herramientas en `server.rs` y `handshake.rs`.

## R7. El re-embed reconstruye el contexto

`reembed_all` (`crates/clawg-core/src/memory/maintenance.rs:507`) MUST re-embeber una fila de `memory` que tiene un vínculo con `context` no vacío con el mismo texto embebido de R2, armado con `embedding_text`. La consulta del lote (`select_reembed_batch`, `maintenance.rs:644`) trae ese `context` con una subconsulta sobre `memory_piece` (`maintenance.rs:680-682`). Una fila sin vínculos, o con `context` vacío, MUST re-embeberse con su contenido, como antes.

#### Escenario
- GIVEN el pedazo 2 de un save largo, guardado con contexto
- WHEN corre `reembed_all` en modo `ReInfer`
- THEN el embedder recibe la cabecera, el separador y el pedazo, igual que al guardarlo

**Test:** `reembed_rebuilds_a_pieces_context` (`maintenance.rs`).

## R8. Datos de la versión anterior

La migración `V11__memory_piece.sql` (`crates/clawg-core/migrations/`) crea `memory_piece` con `CREATE TABLE IF NOT EXISTS` y su índice por `memory_id`, y no escribe filas: MUST NOT escribir vínculos para filas existentes. Una base en V10 se abre con V11 después del snapshot `pre-migration` (la regla de [`specs/storage`](../storage/spec.md)). Sus filas, también las de un save largo guardado antes (pedazos sueltos, sin vínculo), MUST seguir alcanzables por búsqueda, listado, `read_memory` (como saves de un pedazo) y re-embed (sin contexto).

**Test:** `rows_from_v10_survive_the_piece_table` (`pieces.rs`): siembra V10 con SQL directo, abre en V11 y comprueba que no hay vínculos, que las filas se buscan, se listan, se leen como saves de un pedazo y se re-embeben con su propio texto.

## R9. El pozo de KNN cuenta saves

La consulta KNN de `memory` (`knn_one`, `crates/clawg-core/src/memory/search.rs:490`) pide `k` vecinos (30 en `SearchScope::all_default`, `search.rs:37`). Cuando esa ventana vuelve llena y sus filas forman menos de `k` saves distintos (`distinct_saves`, `search.rs:171`, que cuenta una fila sin vínculo como su propio save), la consulta MUST repetirse una vez con la ventana ampliada (`retry_window`, `search.rs:425`), y el resultado MUST recortarse, en orden de puntaje, a las filas que alcanzan `k` saves distintos (`trim_to_saves`, `search.rs:178`, llamado en `search.rs:618`). Sin esto, un save con más pedazos cerca de la consulta que el tamaño del pozo lo llena de hermanos, y R4 los colapsa en un solo hit.

Sigue habiendo a lo sumo una segunda pasada por búsqueda: la que pide un pozo vaciado por filas no elegibles y la que pide uno lleno de hermanos son la misma (`search.rs:574-592`). Los vínculos se leen en la misma conexión (`pieces::read_links`, `pieces.rs:246`). Si esa lectura falla, el pozo MUST contar filas, como antes, y dejar un aviso en el log (`search.rs:560-569`). Una tabla que no es `memory` no tiene vínculos y cuenta filas.

`distinct_saves` y `trim_to_saves` son funciones puras sobre los hits y los vínculos.

#### Escenario
- GIVEN un save de 40 pedazos casi iguales a la consulta y 10 filas sueltas un poco más lejos
- WHEN se busca con un pozo de 30 y `k = 10`
- THEN el resultado tiene 10 hits: el mejor pedazo del save y las 9 mejores filas sueltas

**Test:** `distinct_saves_counts_unlinked_rows_as_their_own`, `trim_to_saves_keeps_the_rows_up_to_k_saves` y `a_save_with_more_pieces_than_the_pool_leaves_room_for_other_memories` (`search.rs`).

## R10. Un save largo con clave reemplaza lo anterior entero

Un save largo con `topic_key` MUST reemplazar todas las filas vivas del alcance de la clave que no son pedazos suyos, de modo que una versión de varios pedazos reemplaza a una de varios pedazos sin dejar ninguno vivo ([`specs/ingest`](../ingest/spec.md), «Un save con clave reemplaza la fila viva de su clave»). El reemplazo corre en la transacción que escribe los vínculos de R3 (`finish_save`, `topic.rs:174`), así que la versión nueva ya está vinculada cuando la vieja deja de estar viva.

Una fila del alcance que `memory_piece` enlaza a un save cuya primera pieza es una fila viva sin esa clave MUST quedar viva y sin clave, en vez de reemplazada (`SHARED_PIECE_SQL`, `crates/clawg-core/src/memory/topic.rs:91`): es un pedazo de otro save que el dedup le sumó a este, y reemplazarlo abriría un hueco en un save que no se revisó. Se mira la primera pieza y no todas las filas del save porque un save con clave se la da a todas sus filas, así que la primera la lleva aunque el dedup le haya sumado, en el medio, una fila de otra clave; mirar todas desenganchaba la versión vieja entera en ese caso.

#### Escenario: una versión larga
- GIVEN un save largo de 3 pedazos guardado con la clave K
- WHEN se guarda otro save largo con K
- THEN los 3 pedazos viejos quedan `superseded` por la primera fila del save nuevo, y los pedazos nuevos están vinculados y vivos

#### Escenario: un pedazo compartido
- GIVEN una fila con la clave K que es el pedazo 2 de un save cuya primera pieza es una fila viva sin clave
- WHEN se guarda otro save con K
- THEN esa fila queda viva y sin clave, y el save al que pertenece sigue entero

**Tests:** `a_long_revision_replaces_every_piece_of_the_old_save` y `a_piece_shared_with_another_save_is_detached` (`worker.rs`, mod tests).

## PB1. Un corte con tokenizer cae en frontera de palabra

Con un tokenizer cargado, todo corte de `TokenSplitter::split` (`crates/clawg-core/src/memory/chunker/token_splitter.rs:53`), salvo los dos extremos del texto, MUST caer en frontera de palabra cuando hay una dentro del rango que admite la ventana ([design D2](../../changes/archive/2026-10-24-piece-boundaries/design.md#d2-el-plan-de-ventanas-con-marcas)). Si no hay ninguna, porque una palabra es más larga que el tope o el texto no tiene espacios, MUST cortar al token. Ningún byte del texto MUST quedar fuera de todas las piezas. Esto alcanza a todo lo que parte con el divisor: los pedazos y la cabecera de un save y la traducción de una nota importada (PB2), y los pasajes del índice de docs ([`specs/project-docs`](../project-docs/spec.md) PB3), también los de `EvalDocIndex`, que parte como él.

Cada pieza MUST tener a lo sumo `max_tokens` tokens según `TokenSplitter::count`. La ventana siguiente MUST empezar en la frontera más cercana al token que deja `overlap` tokens compartidos, y, con `overlap > 0`, dos piezas vecinas MUST compartir al menos `MIN_OVERLAP_CHARS` caracteres, que es lo que busca `join_pieces` (R6; [design D3](../../changes/archive/2026-10-24-piece-boundaries/design.md#d3-el-solape-va-a-la-frontera-más-cercana)). Con `overlap == 0` las piezas MUST cubrir el texto sin repetirlo.

Las marcas (`word_starts`, `token_splitter.rs:156`), los caracteres por token (`char_spans`, `token_splitter.rs:172`) y el plan (`plan_word_windows`, `token_splitter.rs:192`) MUST ser funciones puras. Un token abre una palabra si es el primero o si hay espacio en blanco justo antes de su offset o en él; un token hecho sólo de espacio en blanco, como un `▁` suelto, abre la palabra que le sigue, y el token siguiente ya no, porque el tokenizer vuelve a anteponer ese `▁` al contar sola una pieza que empieza después de él y la pieza pasaría el tope ([design D1](../../changes/archive/2026-10-24-piece-boundaries/design.md#d1-la-frontera-sale-de-los-bytes-no-de-word_ids)). Los signos no son frontera, así que un camino o un identificador no se parten en `/` ni en `::`. Cuando todo token abre una palabra, el plan MUST avanzar exactamente `max - overlap` tokens por ventana.

Los tests marcados *(subpalabras)* usan `subword_tokenizer` (`token_splitter.rs:946`, `cfg(test)`), un `WordPiece` sobre `WhitespaceSplit` con raíces y continuaciones `##` que parte palabras como `retrievies` en `retri ##evi ##es`; `word_tokenizer` no puede partir una palabra y no ve el defecto.

#### Escenario: una pieza no empieza a mitad de palabra
- GIVEN un texto de 600 palabras, cada una de dos o tres subtokens *(subpalabras)*
- WHEN se parte con tope 96 y solape 24
- THEN ninguna pieza empieza ni termina dentro de una palabra
- AND cada pieza tiene a lo sumo 96 tokens
- AND dos piezas vecinas comparten 24 tokens, uno más o uno menos, y al menos 16 caracteres

#### Escenario: una palabra más larga que el tope
- GIVEN un texto con una palabra de 150 subtokens entre palabras cortas *(subpalabras)*
- WHEN se parte con tope 96 y solape 24
- THEN la palabra larga se corta al token, ninguna pieza pasa de 96 tokens y la unión de las piezas cubre el texto sin huecos
- AND los cortes fuera de la palabra larga caen en frontera

#### Escenario: un texto sin espacios
- GIVEN 300 caracteres CJK sin espacios
- WHEN se parte con tope 96 y solape 24
- THEN las piezas son las de antes de este requisito, una ventana cada 72 tokens

#### Escenario: el tokenizer de palabras no cambia
- GIVEN cualquier texto y el `word_tokenizer`
- WHEN se parte con cualquier tope y solape
- THEN las piezas son las mismas que antes de este requisito

**Tests:** `word_starts_marks_tokens_after_or_on_whitespace`, `word_starts_takes_an_offset_that_holds_the_leading_space`, `char_spans_count_the_characters_before_each_token`, `plan_word_windows_with_every_start_marked_keeps_a_fixed_stride`, `plan_word_windows_clamps_the_overlap_and_never_hangs`, `plan_word_windows_terminates_covers_and_never_nests` (barrido exhaustivo), `plan_word_windows_cuts_at_the_last_word_start_under_the_cap`, `plan_word_windows_cuts_a_word_longer_than_the_cap_by_token`, `plan_word_windows_starts_at_the_word_start_nearest_the_overlap` y `plan_word_windows_keeps_the_characters_join_pieces_needs` (puros); `no_piece_starts_or_ends_inside_a_word` *(subpalabras)*, `a_word_longer_than_the_cap_is_cut_by_token_and_loses_no_byte` *(subpalabras)* y `a_text_without_spaces_is_cut_as_before` (`token_splitter.rs`).

## PB2. Los pedazos y la cabecera de un save largo terminan en frontera

Un save largo MUST partirse como dice R2, con los bordes de PB1. La cabecera es la de las definiciones: el texto hasta la última frontera de palabra dentro de los primeros `SAVE_HEAD_TOKENS` tokens (`split_save`, `chunker/chat.rs:114-122`), o esos tokens si la primera palabra es más larga. Con un tokenizer cargado, ni la cabecera guardada en `memory_piece.context` ni ningún pedazo MUST empezar ni terminar dentro de una palabra, salvo en los casos de PB1. `join_pieces` (R6) MUST rearmar exactamente el texto de un save partido con esos bordes.

La traducción de una nota de otra herramienta la parte con `split_save` (`crates/clawg-core/src/memory/portability/translate.rs:328`; [`specs/portability`](../portability/spec.md) HI6), así que sus filas MUST tener los mismos bordes.

#### Escenario: la cabecera
- GIVEN un save de 1.200 tokens de palabras de dos y tres subtokens *(subpalabras)*
- WHEN se parte para guardarlo
- THEN la cabecera son las palabras enteras que caben en los primeros `SAVE_HEAD_TOKENS` tokens, y termina al final de la última de ellas

#### Escenario: el save se lee entero
- GIVEN un save de 1.200 tokens partido *(subpalabras)*
- WHEN se unen sus pedazos con `join_pieces`
- THEN el resultado es el texto del save, sin saltos de línea agregados

#### Escenario: una nota larga importada
- GIVEN una nota de engram de 1.200 tokens *(subpalabras)*
- WHEN se traduce
- THEN ninguna de sus filas empieza ni termina dentro de una palabra

**Tests:** `the_head_ends_at_a_word_boundary` y `no_piece_of_a_long_save_starts_inside_a_word` *(subpalabras)* (`chunker/chat.rs`); `the_pieces_of_a_subword_split_save_join_back_into_its_text` *(subpalabras)* (`pieces.rs`); `a_long_note_is_not_cut_inside_a_word` (`translate.rs`, con `ChatChunker::with_tokenizer`, `chunker/chat.rs:88`).

## PB4. Lo partido antes no cambia

Las filas, vínculos, cabeceras y vectores de un save partido antes de PB1, con bordes a mitad de palabra, MUST seguir apareciendo como un hit por save con `piece=K/N` (R4 y R5), leyéndose enteros con `read_save_core` (R6) y re-embebiéndose con la cabecera guardada en `memory_piece.context` (R7). Ningún camino MUST volver a partirlas: una reimportación encuentra la nota presente y no la vuelve a partir. El respaldo por bytes (`split_by_bytes`, `token_splitter.rs:107`), que corre mientras el tokenizer no cargó o cuando `encode` falla, MUST cortar como antes, sin mirar fronteras, y `golden-mine synth --seed 7` MUST dar el mismo archivo.

#### Escenario
- GIVEN un save de 12 pedazos partido con el divisor anterior *(subpalabras)*, con pedazos que empiezan a mitad de palabra, guardado y vinculado
- AND un save de 12 pedazos guardado con los bordes de PB1
- WHEN se busca por el tema de cada uno, se lee cada uno con `read_save_core` y se re-embeben
- THEN cada save aparece en un solo hit, su texto se lee como se guardó, y el texto embebido de cada pedazo es el mismo de antes del re-embebido

**Tests:** `a_save_cut_inside_words_before_is_still_found_read_and_reembedded` (`pieces.rs`); los de `split_by_bytes` en `token_splitter.rs` y `long_rows_do_not_depend_on_the_piece_cap` (`crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:1164`).

## PB5. El tokenizer real tampoco corta dentro de una palabra

Con el `tokenizer.json` del modelo, los pedazos de un save largo en inglés y en español, con caminos, identificadores y signos, MUST cumplir PB1 y PB2. El test requiere el modelo descargado y no corre en la suite.

**Test:** `the_real_tokenizer_cuts_no_piece_inside_a_word` (`crates/clawg-core/tests/e5_golden.rs:297`, `#[ignore]`, con el modelo en `CLAWG_E5_MODEL_DIR` como `the_vectors_are_the_ones_fastembed_gave`). Fallaba con el divisor anterior.

## PB6. La cabecera de 32 tokens

`SAVE_HEAD_TOKENS` MUST ser 32 (`chunker/mod.rs:40`), y la cabecera MUST seguir terminando en frontera (PB2). Cada pedazo desde el segundo se embebe con unos 16 tokens menos que con los 48 de antes. Se midieron 32, 24 y 16 contra los 48: con 32 ningún caso `long` perdió la respuesta ni bajó otro subconjunto, con 24 bajó un caso de `multi_ref` y con 16 un caso `long` dejó de verse en las dos herramientas. Los casos `long` nombran su tema en la primera oración, lo que favorece a una cabecera corta, y por eso no se tomó 24 ([verify](../../changes/archive/2026-10-24-piece-boundaries/verify.md#u3-y-u4-los-brazos)). Un corte en fin de oración con menos solape se midió y no entró: no ahorraba un 5 % de filas y perdía casos `long`.

**Tests:** `the_head_is_the_first_tokens_of_the_text` (`chunker/chat.rs:230`) y `the_head_ends_at_a_word_boundary` (`chunker/chat.rs:239`), que leen `SAVE_HEAD_TOKENS`.

## Tabla

```sql
CREATE TABLE IF NOT EXISTS memory_piece (
    save_id    INTEGER NOT NULL,
    piece      INTEGER NOT NULL,
    memory_id  INTEGER NOT NULL,
    context    TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (save_id, piece)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_memory_piece_memory ON memory_piece (memory_id);
```

Sin claves foráneas, como `memory_derivation`. Un vínculo a una fila borrada (`deleted_at`) o que ya no está en `memory` dice qué pedazo falta: `read_save_core` lee las posiciones con un `LEFT JOIN` (`pieces.rs:143-145`). `clawg purge` (`crates/clawg-core/src/memory/secrets.rs:181`) deja los vínculos y, si purga la posición 1, vacía el `context` de todos los del save, que guardaba su texto ([`specs/secrets`](../secrets/spec.md) R5); un re-embed de los pedazos siguientes los embebe entonces sin cabecera. El vínculo de la fila purgada queda, pero el id no se reusa (`memory.id` es `AUTOINCREMENT`, `V1__init.sql:5`). Volver atrás es un `DROP TABLE`.
