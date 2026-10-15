# long-save-context — Especificación

**Dominio destino:** `specs/pieces/spec.md` (creado al archivar). Al archivar, también:

- `specs/mcp` R2 y R3: las herramientas pasan de nueve a diez;
- `specs/disputes` R8: los pedazos hermanos, además de no disputarse, quedan vinculados (R3 de acá);
- `specs/eval`: el subconjunto `long`, la clave `answer` y las métricas de respuesta (R9 de acá);
- `specs/visibility` R3 y R4: `read_memory` y su ruta, con el mismo "no existe" para lo ajeno.

**Alcance:** cómo se parte y se embebe un texto que no entra en un pedazo, cómo se vinculan sus pedazos, cómo aparecen en la búsqueda, cómo se lee el save entero y cómo se mide. Los textos de un pedazo, el ranking de filas sueltas, el detector de contradicciones y el workspace no cambian.

## Definiciones

- **Save:** un evento de ingesta (`MemoryEvent`): un `save_memory` o un turno de chat. Es la misma unidad que los "pedazos hermanos" de [`specs/disputes`](../../../specs/disputes/spec.md).
- **Texto del save:** el contenido del evento después de enmascarar secretos (`redact`, `crates/clawg-core/src/memory/worker.rs:429-434`). Todo lo de abajo trabaja sobre ese texto.
- **Pedazo:** cada ventana que deja el chunker, después de colapsar los repetidos dentro del mismo evento (`collapse_duplicate_chunks`, `worker.rs:137`). Su posición es 1, 2, … en ese orden.
- **Save largo:** un save con dos pedazos o más.
- **Cabecera:** los primeros `SAVE_HEAD_TOKENS` = 48 tokens del texto del save, contados con el mismo splitter del chunker (en modo sin tokenizer, 4 bytes por token).
- **Texto embebido de un pedazo:** el pedazo, o `cabecera + "\n…\n" + pedazo` cuando tiene contexto.
- **Vínculo:** una fila de `memory_piece`: `(save_id, piece, memory_id, context)`.
- **Save de una fila:** si la fila tiene vínculos, el de menor `save_id`; si no, la fila sola, como save de un pedazo.

## R1. Un texto que entra en un pedazo no cambia

Un texto del save que el chunker deja en un pedazo con el tope de hoy (`CHUNK_MAX_TOKENS` = 480) MUST partirse, embeberse y guardarse exactamente como hoy: el mismo texto, el mismo vector y ningún vínculo.

#### Escenario
- GIVEN un save de 300 tokens
- WHEN se guarda
- THEN queda una fila con el texto entero, embebida con ese texto y sin filas en `memory_piece`

**Test:** `a_text_that_fits_one_piece_is_split_as_before` en `crates/clawg-core/src/memory/chunker/chat.rs`; `a_single_piece_save_embeds_its_text_and_leaves_no_link` en `worker.rs`. El primero protege lo que ya existe; el segundo falla hoy, porque la tabla no existe.

## R2. Un save largo se parte con lugar para la cabecera

Un texto del save que no entra en un pedazo de 480 tokens MUST partirse en pedazos de a lo sumo `PIECE_MAX_TOKENS` = 424 tokens, con el solapamiento de hoy (`CHUNK_OVERLAP_TOKENS` = 64). Cada pedazo desde el segundo MUST embeberse con contexto (cabecera, separador y pedazo); el primero, sin él, porque ya empieza con la cabecera. La fila MUST guardar sólo el pedazo: el contenido, el hash de dedup y el FTS no llevan la cabecera.

`SAVE_HEAD_TOKENS + reserva del separador + PIECE_MAX_TOKENS` MUST ser `<= CHUNK_MAX_TOKENS`, comprobado en compilación, así el texto embebido nunca pasa el límite que el embedder trunca en silencio (`chunker/mod.rs:28-38`).

El armado del texto embebido MUST ser una función pura, la misma que usa el re-embed (R7).

#### Escenario: el tema llega al vector del pedazo 2
- GIVEN un save de 1.200 tokens que nombra `clawg-ledger` sólo en su primera oración
- WHEN se guarda
- THEN queda en 3 pedazos o más
- AND el embedder recibe el pedazo 2 precedido de una cabecera que contiene `clawg-ledger`
- AND la fila del pedazo 2 guarda el pedazo sin la cabecera

**Test:** `a_long_text_is_split_below_the_cap_to_leave_room_for_the_head` y `the_head_is_the_first_tokens_of_the_text` en `chat.rs`; `a_piece_without_context_is_embedded_as_stored` y `a_piece_with_context_is_embedded_after_the_head` en `chunker/mod.rs`; `later_pieces_are_embedded_with_the_head_and_stored_without_it` en `worker.rs`, que también prueba que el pedazo 1 se embebe sin cabecera. Fallan hoy.

## R3. El save deja su vínculo

Cuando un save largo termina sin falla, el worker MUST escribir, en una sola transacción, un vínculo por pedazo:

- `save_id`: un número nuevo, uno más que el mayor de la tabla;
- `piece`: la posición del pedazo;
- `memory_id`: la fila que lo guarda, insertada por este evento o absorbida por el dedup (`absorb_duplicate`, `worker.rs:386`);
- `context`: la cabecera con la que se embebió la fila si este evento la insertó con contexto; si no, vacío. Una fila absorbida conserva el vector que ya tenía, así que su vínculo no dice que lo embebió con contexto.

Un save de un pedazo MUST NOT dejar vínculos. Un save que se corta a mitad de camino (`IngestReport.failure`) MUST NOT dejar vínculos: sus filas quedan sueltas, como hoy, y al guardarlo de nuevo los pedazos se absorben y el vínculo se escribe entonces.

Si escribir el vínculo falla, el save MUST seguir informándose como guardado: sus filas ya están y se encuentran como hoy. La falla va al log.

#### Escenario: pedazos absorbidos
- GIVEN una fila guardada con el texto exacto del pedazo 2 de un save
- WHEN se guarda el save
- THEN el pedazo 2 se absorbe en esa fila y su vínculo apunta a ella con `context` vacío
- AND los otros pedazos se insertan y sus vínculos llevan la cabecera desde el pedazo 3

#### Escenario: save cortado
- GIVEN un embedder que falla en un save largo
- WHEN se guarda
- THEN `memory_piece` no cambia

Guardar otra vez exactamente las mismas filas, en el mismo orden, MUST devolver el `save_id` que ya forman y no escribir nada: el mismo save no se vincula dos veces.

**Test:** `a_long_save_links_its_pieces_in_order`, `an_absorbed_piece_is_linked_without_context`, `a_save_cut_short_leaves_no_link` y `the_same_long_save_again_links_the_rows_it_left` en `worker.rs`; `link_pieces_numbers_saves_in_order` y `linking_the_same_rows_again_answers_the_save_they_already_form` en `pieces.rs`. Fallan hoy.

## R4. Un hit por save

La búsqueda (`search_all_memories_inner`, `crates/clawg-core/src/memory/search.rs`) MUST, después de ordenar y antes de cortar en `k`, quedarse con el primer hit de cada save y descartar los demás pedazos de ese save. Un hit de una fila sin vínculos, o de `episodic_trajectory` o `procedural_skill`, MUST quedar donde está.

Cada hit de una fila con vínculos MUST llevar su posición y el total de pedazos del save. El descarte MUST correr antes del corte, así los lugares que liberan los hermanos los ocupan otras memorias.

El descarte MUST ser una función pura sobre los hits y los vínculos de sus filas. Si leer los vínculos falla, la búsqueda MUST devolver los hits sin colapsar: es lo de hoy, no un error.

#### Escenario: los hermanos no ocupan el top
- GIVEN un save de 4 pedazos que puntúan 1.º, 2.º, 4.º y 5.º para una consulta, y 10 filas sueltas
- WHEN se busca con `k = 10`
- THEN el resultado tiene 10 hits: el pedazo que puntuó 1.º, con `piece = 1` y `pieces = 4`, y las 9 mejores filas sueltas

**Test:** `only_the_best_piece_of_a_save_survives`, `rows_without_links_keep_their_place`, `freed_slots_go_to_other_memories` y `a_link_read_failure_leaves_hits_uncollapsed` en `search.rs`. Fallan hoy.

## R5. El hit dice qué pedazo es

Un hit con vínculo MUST mostrar `piece=K/N` en su metadato:

- `search_memory`: `{rank}. #{id} [{kind}/{type}/t{tier} piece={K}/{N} score={score}] {contenido}`;
- `recall_context`: `[YYYY-MM-DD #{id} {type} piece={K}/{N}] {contenido}`.

Un hit sin vínculo MUST seguir con el formato de hoy. `Hit` en el cliente MUST leer los dos campos con `#[serde(default)]`, así un daemon viejo sigue respondiendo.

Los parsers del agent-view (`crates/clawg-eval/src/bin/rag-eval/agent_view/parse.rs`) MUST leer los dos formatos y devolver el id de cada línea de búsqueda.

#### Escenario
- GIVEN un hit del pedazo 2 de 4, id 58
- THEN `search_memory` imprime `1. #58 [Memory/semantic_fact/t3 piece=2/4 score=0.81] …` (el `kind` sale como lo serializa el daemon, `Memory`)
- AND `recall_context` imprime `[2026-10-15 #58 semantic_fact piece=2/4] …`

**Test:** `format_hits_names_the_piece_of_a_long_save` en `crates/clawg-cli/src/mcp/tools.rs`; `format_hit_line_names_the_piece_of_a_long_save` en `crates/clawg-core/src/memory/prompt_builder.rs`; `a_hit_line_with_a_piece_yields_its_id_score_and_content` y `a_context_hit_with_a_piece_is_one_segment` en `parse.rs`. Fallan hoy.

## R6. El save entero

`GET /v1/memories/{kind}/{id}/save` con la query `agent` MUST devolver el save de la fila:

- `id`, `kind`, `piece` (la posición de la fila pedida), `pieces` (el total) e `ids`: el id de cada posición, o `null` si esa fila está borrada o es de otro agente;
- `missing`: las posiciones sin fila visible;
- `text`: los pedazos visibles en orden, sin el solapamiento entre vecinos.

Una fila sin vínculos MUST devolverse como save de un pedazo, con su contenido como `text`. Una fila que no existe, borrada o de otro agente cuando hay scope, MUST responder 404, como el resto de las rutas por id (`specs/visibility` R4).

Quitar el solapamiento MUST ser una función pura: entre dos pedazos vecinos busca el sufijo más largo del primero que es prefijo del segundo, de al menos 16 caracteres y dentro de los últimos 2.048 bytes, y lo deja una sola vez. Si no lo encuentra (porque falta un pedazo en el medio), MUST unirlos con un salto de línea y no recortar nada.

El puente MUST exponer `read_memory(id, kind?)`, con `kind = memory` por defecto y el agente que llama. Su respuesta MUST abrir con una línea que dice qué pedazo es y de cuántos, los ids del save y los que faltan, y seguir con `text`. Sobre una fila ajena MUST fallar como si no existiera.

`save_memory` MUST nombrar `read_memory` en su descripción (en lugar de "never reassembled") y en la respuesta de un save largo. `search_memory` MUST explicar `piece=K/N`. `tools/list` MUST listar diez herramientas.

#### Escenario: lo propio, con un pedazo borrado
- GIVEN un save de 4 pedazos del agente A, con el pedazo 3 borrado
- WHEN A llama `read_memory` con el id del pedazo 2
- THEN la respuesta dice `piece 2 of 4`, lista los ids con el 3 como faltante y trae los pedazos 1, 2 y 4: el 1 y el 2 sin el solapamiento repetido, el 4 después de un salto de línea

#### Escenario: lo ajeno
- GIVEN un save del agente A
- WHEN B llama `read_memory` con uno de sus ids
- THEN la respuesta es un error, igual que con un id que no existe

Contra un daemon que no tiene la ruta (su 404 no trae el error del daemon), `read_memory` MUST decir que hay que reiniciar el daemon, no que la fila no existe.

**Test:** `join_pieces_drops_the_overlap_once`, `join_pieces_keeps_both_sides_when_nothing_overlaps`, `the_pieces_of_a_split_save_join_back_into_its_text` y `read_save_lists_missing_pieces_and_skips_other_agents` en el core (`crates/clawg-core/src/memory/pieces.rs`); `get_save_returns_the_whole_text` y `get_save_of_another_agent_is_404` en `crates/clawg-cli/tests/api.rs`; `read_memory_returns_the_whole_save`, `an_agent_cannot_read_another_agents_save`, `a_daemon_without_the_save_route_is_named_as_such`, `save_memory_asks_for_one_fact_per_call`, `search_memory_explains_the_piece_of_a_long_save` y los conteos de herramientas en `crates/clawg-cli/src/mcp/server.rs` y `handshake.rs`; `format_save_names_the_piece_and_the_missing_ones` en `tools.rs`; la respuesta de un save largo en `save_outcome_text_covers_every_daemon_answer`. Fallan hoy.

## R7. El re-embed reconstruye el contexto

`reembed_all` MUST re-embeber una fila de `memory` que tiene un vínculo con `context` no vacío con el mismo texto embebido de R2, armado con la misma función. Una fila sin vínculos, o con `context` vacío, MUST re-embeberse con su contenido, como hoy.

#### Escenario
- GIVEN el pedazo 2 de un save largo, guardado con contexto
- WHEN corre `reembed_all` en modo `ReInfer`
- THEN el embedder recibe la cabecera, el separador y el pedazo, igual que al guardarlo

**Test:** `reembed_rebuilds_a_pieces_context` en `crates/clawg-core/src/memory/maintenance.rs`. Falla hoy.

## R8. Datos de la versión anterior

Una base en V10 con filas de `memory` y sus vectores, sembradas con SQL directo como las deja `main` (también un save largo: pedazos sueltos, sin vínculo), MUST abrirse con la migración V11 después del snapshot `pre-migration`, y sus filas MUST seguir alcanzables por búsqueda, listado, `read_memory` (como saves de un pedazo) y re-embed (sin contexto). La migración MUST NOT escribir vínculos para filas existentes.

**Test:** `rows_from_v10_survive_the_piece_table` en `crates/clawg-core/src/memory/pieces.rs`. Falla hoy: la migración no existe.

## R9. Medición

`golden-mine synth` MUST agregar, después de todo lo que agrega hoy y sin cambiar nada de eso, 12 casos del subconjunto `long` (6 en inglés y 6 en español):

- cada caso tiene una fila propia de al menos 3 pedazos con el chunker de producción, aun en modo sin tokenizer;
- la fila nombra la clave del caso sólo en su primera oración, y el dato que responde la consulta aparece una sola vez, después del primer tercio del texto;
- el relleno entre los dos no nombra la clave y no repite una secuencia de 6 palabras de una fila de la base; entre dos filas `long` lo que se exige es que no compartan ningún pedazo (el dedup los absorbería y el caso mediría otra cosa). Las filas `long` salen de las mismas plantillas de relleno, así que entre ellas sí pueden repetirse frases cortas;
- el caso lleva la clave nueva `answer`: el dato tal como aparece en la fila.

Con la misma base y la misma semilla, las filas y los casos que el generador agrega hoy MUST salir idénticos.

`RetrievalCase` (y el `Case` de `golden-mine`) MUST aceptar `answer` opcional, y `CaseSubset` el valor `long`. Un golden sin esas claves MUST parsear igual que hoy.

`rag-eval --agent-view` MUST reportar, para los casos con `answer`:

- **respuesta visible:** el texto que devolvió la herramienta contiene `answer`, comparado con los espacios normalizados (`normalize`, `agent_view/matcher.rs:22`);
- **respuesta alcanzable** (sólo `search_memory`): visible, o visible en el save que devuelve `GET /v1/memories/memory/{id}/save` para la primera línea que carga la fila esperada. Contra un daemon sin esa ruta, alcanzable es igual a visible;
- **pedazos repetidos:** cuántas respuestas de `search_memory` muestran dos líneas del mismo save (dos líneas con el mismo id, o con filas del mismo caso);
- **por subconjunto:** las métricas de agent-view separadas por `subset` (`by_subset`), así `long` y los otros cinco se comparan por separado (U2 y U4).

#### Escenario: el dato después del corte
- GIVEN un caso `long` con clave `amber-falcon` y `answer` `"retry budget of 7 attempts"`
- THEN su fila nombra `amber-falcon` sólo en la primera oración, `retry budget of 7 attempts` aparece una vez después del primer tercio, y la fila se parte en 3 pedazos o más

**Test:** `there_are_twelve_long_cases_at_any_size`, `a_long_row_names_its_key_only_in_its_first_sentence`, `the_answer_appears_once_after_the_first_third`, `a_long_row_splits_into_at_least_three_pieces` y `adding_long_cases_leaves_the_rest_of_the_set_unchanged` en `crates/clawg-eval/src/bin/golden-mine/synth/mod.rs`; `an_answer_and_the_long_subset_parse` en `rag-eval/golden.rs`; `a_long_case_writes_its_subset_and_answer` en `golden-mine/synth/golden.rs`; `the_answer_counts_only_when_its_text_is_visible`, `a_read_save_makes_a_hidden_answer_reachable`, `a_line_repeating_an_earlier_lines_row_is_counted` y `build_results_splits_active_cases_by_subset_with_their_answers` en `agent_view/mod.rs`; `a_case_that_names_its_answer_reports_whether_the_answer_reached_the_agent` en `agent_view/runner.rs`. Fallan hoy.
