# long-save-ranking — Especificación

Cambia cómo se parte un save largo y cómo la búsqueda junta sus candidatos. Modifica R2 de [`specs/pieces`](../../specs/pieces/spec.md) (el tope de los pedazos) y agrega dos requisitos: el pozo de KNN cuenta saves, y el set `long` no depende del tope. Las definiciones (save, pedazo, cabecera, vínculo) son las de [`specs/pieces`](../../specs/pieces/spec.md#definiciones).

## R1. Un save largo se parte en pedazos chicos

*(Reemplaza el tope de R2 de `specs/pieces`; el resto de ese requisito sigue igual.)*

Un texto del save que no entra en un pedazo de `CHUNK_MAX_TOKENS` = 480 (`crates/clawg-core/src/memory/chunker/mod.rs:15`) MUST partirse en pedazos de a lo sumo `PIECE_MAX_TOKENS` = 96 tokens, con `PIECE_OVERLAP_TOKENS` = 24 de solapamiento entre vecinos (`ChatChunker::split_save`, `chunker/chat.rs:88`). Cada pedazo desde el segundo MUST seguir embebiéndose detrás de la cabecera de `SAVE_HEAD_TOKENS` = 48, y la fila MUST seguir guardando sólo el pedazo.

Un texto que entra en un pedazo de 480 MUST partirse, embeberse y guardarse como hoy (R1 de `specs/pieces`). `ChatChunker::chunk` (`chunker/chat.rs:76`), que usan los demás caminos, MUST seguir cortando con `CHUNK_OVERLAP_TOKENS` = 64.

Aserciones de compilación MUST garantizar `PIECE_OVERLAP_TOKENS < PIECE_MAX_TOKENS` y `SAVE_HEAD_TOKENS + HEAD_JOIN_RESERVE + PIECE_MAX_TOKENS <= CHUNK_MAX_TOKENS`.

#### Escenario: un save de 1.200 tokens
- GIVEN un texto de 1.200 tokens
- WHEN se parte para guardarlo
- THEN cada pedazo tiene a lo sumo 96 tokens
- AND dos pedazos vecinos comparten 24 tokens
- AND la cabecera son los primeros 48 tokens del texto

#### Escenario: un save de 300 tokens
- GIVEN un texto de 300 tokens
- WHEN se parte para guardarlo
- THEN queda en un pedazo, sin cabecera, igual que hoy

**Test:** `a_long_text_is_split_into_small_pieces` y `small_pieces_share_their_own_overlap` (`chunker/chat.rs`); los de R1 de `specs/pieces` siguen pasando sin cambios.

## R2. Los saves ya guardados no cambian

Las filas, vínculos y vectores de un save partido con el tope anterior (424 tokens, 64 de solapamiento) MUST seguir: apareciendo como un hit por save con `piece=K/N`, leyéndose enteros con `read_save_core` (`crates/clawg-core/src/memory/pieces.rs:95`) y re-embebiéndose con su contexto (`reembed_all`, `crates/clawg-core/src/memory/maintenance.rs:467`). Ningún camino MUST volver a partir una fila guardada.

`join_pieces` (`pieces.rs:58`) MUST unir sin repetir el solapamiento tanto pedazos de 424/64 como de 96/24.

#### Escenario
- GIVEN un save de 3 pedazos de 424 tokens, guardado y vinculado como lo dejaba la versión anterior
- AND un save de 12 pedazos de 96 tokens guardado con esta versión
- WHEN se busca por el tema de cada uno y se lee cada uno con `read_save_core`
- THEN cada save aparece en un solo hit y su texto se lee entero, sin solapamiento repetido

**Test:** `a_save_split_at_the_old_cap_is_still_found_and_read` (`pieces.rs`); `the_pieces_of_a_split_save_join_back_into_its_text` con el tope nuevo.

## R3. El pozo de KNN cuenta saves

La consulta KNN de `memory` (`knn_one`, `crates/clawg-core/src/memory/search.rs:391`) pide `k` vecinos (30 en `SearchScope::all_default`, `search.rs:39`). Cuando esa ventana vuelve llena y sus filas forman menos de `k` saves distintos (contando una fila sin vínculo como su propio save), la consulta MUST repetirse una vez con la ventana ampliada (`retry_window`, `search.rs:332`), y el resultado MUST recortarse a las filas que alcancen `k` saves distintos, en orden de puntaje.

Sigue habiendo a lo sumo una segunda pasada por búsqueda: la que pide un pozo vaciado por filas no elegibles y la que pide uno lleno de hermanos son la misma. Leer los vínculos MUST fallar suave: sin vínculos, el pozo cuenta filas, como hoy.

Contar saves y recortar MUST ser funciones puras sobre los hits y los vínculos.

#### Escenario
- GIVEN un save de 40 pedazos casi iguales a la consulta y 10 filas sueltas un poco más lejos
- WHEN se busca con un pozo de 30 y `k = 10`
- THEN el resultado tiene 10 hits: el mejor pedazo del save y las 9 mejores filas sueltas

**Test:** `a_save_with_more_pieces_than_the_pool_leaves_room_for_other_memories` (`search.rs`); `distinct_saves_counts_unlinked_rows_as_their_own` y `trim_to_saves_keeps_the_rows_up_to_k_saves` (`search.rs`).

## R4. El set `long` no depende del tope

`golden-mine synth` MUST generar las filas `long` con el largo que les daba el tope de 424 tokens: cada una llega a `LONG_MIN_PIECES` = 4 pedazos cortada con 424/64 sin tokenizer (`crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:430`), no con el tope de producción. Así base y punta miden el mismo set, y el hecho queda a dos tercios del texto como hoy.

#### Escenario
- GIVEN la misma base y `--seed 7`
- WHEN se genera el set con el tope nuevo
- THEN las 12 filas `long` son idénticas, byte a byte, a las que generaba el tope anterior

**Test:** `long_rows_do_not_depend_on_the_piece_cap` (`synth/mod.rs`); los de R9 de [`specs/eval`](../../specs/eval/spec.md) siguen pasando.

## R5. Medición

Con el set real de Marcos (`golden-mine synth --seed 7`) y los umbrales de la [propuesta](proposal.md#éxito):

- U0, la base de `main`, se registra antes de medir la punta.
- U1: el ancla de 46 casos no empeora en ninguna métrica.
- U2: `long` llega a `search_reachable` ≥ 0,917, `search_visible` ≥ 0,5 y `context_visible` ≥ 0,75.
- U3: los otros cinco subconjuntos no bajan más de 0,02 en ninguna métrica de agent-view, y `repeated_rows` es 0.

**Test:** no hay: se mide en `verify.md`.
