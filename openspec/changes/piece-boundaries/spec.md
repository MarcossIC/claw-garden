# piece-boundaries — Especificación

Cambia dónde caen los bordes de los cortes del divisor de tokens (`TokenSplitter::split`, `crates/clawg-core/src/memory/chunker/token_splitter.rs:51`), y con eso alcanza las piezas de un save largo, su cabecera y los pasajes del índice de docs. Modifica R2 de [`specs/pieces`](../../specs/pieces/spec.md) (la cabecera y los pedazos) y el requisito del troceo de Markdown de [`specs/project-docs`](../../specs/project-docs/spec.md) (`chunk_markdown`). Los topes, el respaldo por bytes y los saves ya guardados no cambian. Las definiciones (save, pedazo, cabecera, vínculo) son las de [`specs/pieces`](../../specs/pieces/spec.md#definiciones), con el cambio de R2 en la cabecera.

**Fixture.** Los tests marcados *(subpalabras)* usan un tokenizador `WordPiece` sobre `WhitespaceSplit`, con raíces y continuaciones `##`, que parte palabras como `retrievies` en `retri ##evi ##es`. Los tokenizadores de prueba de hoy, `word_tokenizer` (`chunker/mod.rs:13-19`) y `word_level_tokenizer` (`token_splitter.rs:716`), no pueden partir una palabra, y por eso no ven el defecto.

**Frontera de palabra.** Un byte `b` de un texto es frontera si `b == 0`, si `b` es el largo del texto, o si el carácter anterior a `b` o el que empieza en `b` es espacio en blanco (`char::is_whitespace`). Un texto empieza o termina dentro de una palabra si su primer o su último byte, como corte en el texto del que sale, no es frontera.

## R1. Un corte con tokenizador cae en frontera de palabra

Con un tokenizador cargado, todo corte de `TokenSplitter::split`, salvo los dos extremos del texto, MUST caer en frontera de palabra cuando hay una dentro del rango que admite la ventana ([design D2](design.md#d2-el-plan-de-ventanas-con-marcas)). Si no hay ninguna, porque una palabra es más larga que el tope o el texto no tiene espacios, MUST cortar al token, como hoy. Ningún byte del texto MUST quedar fuera de todas las piezas.

Cada pieza MUST tener a lo sumo `max_tokens` tokens según `TokenSplitter::count` (`token_splitter.rs:41`). Dos piezas vecinas MUST compartir al menos `overlap` tokens, salvo cuando la ventana anterior se cortó al token sin frontera disponible. La ventana siguiente MUST empezar en la última frontera que deja al menos ese solape ([design D3](design.md#d3-el-solape-crece-hasta-la-palabra-no-se-achica)).

Las marcas (`word_starts`) y el plan (`plan_word_windows`) MUST ser funciones puras. Cuando todo token abre una palabra, el plan MUST ser exactamente el de `plan_windows` (`token_splitter.rs:153`).

#### Escenario: una pieza no empieza a mitad de palabra
- GIVEN un texto de 600 palabras, cada una de dos o tres subtokens *(subpalabras)*
- WHEN se parte con tope 96 y solape 24
- THEN ninguna pieza empieza ni termina dentro de una palabra
- AND cada pieza tiene a lo sumo 96 tokens
- AND dos piezas vecinas comparten al menos 24 tokens

#### Escenario: una palabra más larga que el tope
- GIVEN un texto con una palabra de 150 subtokens entre palabras cortas *(subpalabras)*
- WHEN se parte con tope 96 y solape 24
- THEN la palabra larga se corta al token, ninguna pieza pasa de 96 tokens y la unión de las piezas cubre el texto sin huecos
- AND los cortes fuera de la palabra larga caen en frontera

#### Escenario: un texto sin espacios
- GIVEN 300 caracteres CJK sin espacios
- WHEN se parte con tope 96 y solape 24
- THEN las piezas son las mismas que con `plan_windows`

#### Escenario: el tokenizador de palabras no cambia
- GIVEN cualquier texto y el `word_tokenizer`
- WHEN se parte con cualquier tope y solape
- THEN las piezas son las mismas que antes de este cambio

**Tests:** `word_starts_marks_tokens_after_or_on_whitespace`, `word_starts_takes_an_offset_that_holds_the_leading_space`, `plan_word_windows_with_every_start_marked_is_plan_windows`, `plan_word_windows_terminates_covers_and_never_nests` (barrido exhaustivo, como `plan_windows_terminates_and_every_window_is_bounded`, `token_splitter.rs:237`), `plan_word_windows_cuts_at_the_last_word_start_under_the_cap`, `plan_word_windows_cuts_a_word_longer_than_the_cap_by_token` y `plan_word_windows_keeps_at_least_the_overlap` (puros); `no_piece_starts_or_ends_inside_a_word` *(subpalabras)*, `a_word_longer_than_the_cap_is_cut_by_token_and_loses_no_byte` *(subpalabras)* y `a_text_without_spaces_is_cut_as_before` (`token_splitter.rs`). Los tests de hoy de `token_splitter.rs` y `chunker/chat.rs` siguen pasando sin cambios.

## R2. Las piezas y la cabecera de un save largo terminan en frontera

*(Modifica R2 de `specs/pieces`: la cabecera y los bordes. Los topes, el contexto, el armado del texto embebido y los saves partidos con el tope anterior siguen igual.)*

Un save largo MUST partirse como dice R2 de `specs/pieces`, con los bordes de R1. La **cabecera** pasa a ser el texto desde el principio hasta la última frontera de palabra dentro de los primeros `SAVE_HEAD_TOKENS` tokens (`split_save`, `chunker/chat.rs:114-122`), o los primeros `SAVE_HEAD_TOKENS` tokens si la primera palabra es más larga. Ni la cabecera guardada en `memory_piece.context` ni ninguna pieza MUST terminar ni empezar dentro de una palabra con un tokenizador cargado, salvo en los casos de R1.

`join_pieces` (`crates/clawg-core/src/memory/pieces.rs:65`) MUST rearmar exactamente el texto de un save partido con esos bordes.

#### Escenario: la cabecera
- GIVEN un save de 1.200 tokens cuyo token 48 es la continuación de una palabra *(subpalabras)*
- WHEN se parte para guardarlo
- THEN la cabecera termina al final de la palabra anterior a esa, y tiene a lo sumo 48 tokens

#### Escenario: el save se lee entero
- GIVEN un save de 1.200 tokens partido *(subpalabras)*
- WHEN se unen sus piezas con `join_pieces`
- THEN el resultado es el texto del save, sin saltos de línea agregados

**Tests:** `the_head_ends_at_a_word_boundary` y `no_piece_of_a_long_save_starts_inside_a_word` *(subpalabras)* (`chunker/chat.rs`); `the_pieces_of_a_subword_split_save_join_back_into_its_text` *(subpalabras)* (`pieces.rs`). Los de R2 de `specs/pieces` siguen pasando.

## R3. La importación y el índice de docs cortan igual

La traducción de una nota de otra herramienta (`portability/translate.rs:328`) y el troceo de un pasaje de docs largo (`emit`, `project_docs/chunk.rs:127-144`) MUST cortar con los bordes de R1, porque usan el mismo divisor. Un pasaje del índice ya guardado MUST NOT volver a partirse mientras su archivo no cambie de sha256 ni cambie el modelo, como hoy.

#### Escenario: una nota larga importada
- GIVEN una nota de engram de 1.200 tokens *(subpalabras)*
- WHEN se traduce
- THEN ninguna de sus filas empieza ni termina dentro de una palabra

#### Escenario: una sección de docs larga
- GIVEN una sección de Markdown de 1.600 palabras de varios subtokens *(subpalabras)*
- WHEN se trocea
- THEN ningún pasaje empieza ni termina dentro de una palabra, y cada uno, con su breadcrumb, entra en 480 tokens

**Tests:** `a_long_note_is_not_cut_inside_a_word` (`translate.rs`, con `ChatChunker::with_tokenizer`, `chunker/chat.rs:88`); `a_long_section_is_not_cut_inside_a_word` (`project_docs/chunk.rs`).

## R4. Lo guardado antes no cambia

Las filas, vínculos, cabeceras y vectores de un save partido antes de este cambio, con bordes a mitad de palabra, MUST seguir apareciendo como un hit por save con `piece=K/N`, leyéndose enteros con `read_save_core` y re-embebiéndose con la cabecera guardada en `memory_piece.context`. Ningún camino MUST volver a partirlas. El respaldo por bytes (`split_by_bytes`, `token_splitter.rs:105-142`) MUST cortar como hoy, y `golden-mine synth --seed 7` MUST dar el mismo archivo.

#### Escenario
- GIVEN un save de 12 piezas partido con el divisor anterior *(subpalabras)*, con piezas que empiezan a mitad de palabra, guardado y vinculado
- AND un save de 12 piezas guardado con esta versión
- WHEN se busca por el tema de cada uno, se lee cada uno con `read_save_core` y se re-embeben
- THEN cada save aparece en un solo hit, su texto se lee como se guardó, y el texto embebido de cada pieza es el mismo de antes del re-embebido

**Tests:** `a_save_cut_inside_words_before_is_still_found_read_and_reembedded` (`pieces.rs`); las de `split_by_bytes` en `token_splitter.rs` y `long_rows_do_not_depend_on_the_piece_cap` (`crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:1164`) siguen pasando.

## R5. El tokenizador real tampoco corta dentro de una palabra

Con el `tokenizer.json` del modelo, las piezas de un save largo en inglés y en español, con caminos, identificadores y signos, MUST cumplir R1 y R2. El test requiere el modelo descargado y no corre en la suite.

**Test:** `the_real_tokenizer_cuts_no_piece_inside_a_word` (`crates/clawg-core/tests/e5_golden.rs`, `#[ignore]`, con el modelo en `CLAWG_E5_MODEL_DIR` como `the_vectors_are_the_ones_fastembed_gave`, `:265`). Hoy falla.

## R6. Brazos medidos

Cada brazo entra sólo si pasa su umbral (U3 y U4 de la [propuesta](proposal.md#éxito)). Los requisitos de abajo valen sólo para el brazo que entre.

**R6a. Menos solape en fin de oración.** Cuando hay un inicio de oración ([design D4](design.md#d4-brazo-a-menos-solape-donde-el-corte-cae-en-fin-de-oración)) a lo sumo `SENTENCE_LOOKBACK_TOKENS` tokens antes del tope, el corte MUST caer en el último de ellos, y la pieza siguiente MUST empezar en la última frontera de palabra que deja al menos `MIN_OVERLAP_CHARS` caracteres de solape. `join_pieces` MUST seguir rearmando el texto exacto.

- GIVEN un save de oraciones de 20 tokens *(subpalabras)*
- WHEN se parte
- THEN cada borde con un fin de oración dentro del retroceso cae ahí, el solape tiene al menos 16 caracteres, y el save tiene menos piezas que sin el brazo

**Tests:** `sentence_starts_marks_a_word_after_a_full_stop_or_a_blank_line`, `a_cut_at_a_sentence_end_keeps_the_shortest_overlap_that_joins` y `the_pieces_cut_at_sentence_ends_join_back_into_their_text` *(subpalabras)*.

**R6b. Una cabecera más corta.** `SAVE_HEAD_TOKENS` MUST ser el valor adoptado. La cabecera MUST seguir terminando en frontera (R2).

**Tests:** `the_head_is_the_first_tokens_of_the_text` (`chunker/chat.rs:230`) y `the_head_ends_at_a_word_boundary` con el valor nuevo.
