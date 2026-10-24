# piece-boundaries — Diseño

## D1. La frontera sale de los bytes, no de `word_ids`

Un token abre una palabra cuando el corte justo antes de él no parte una palabra del texto. `word_starts(text, offsets) -> Vec<bool>` (pura, en `token_splitter.rs`) marca el token `i` cuando:

- `i == 0`, o
- el carácter anterior a `offsets[i].0` es espacio en blanco (`char::is_whitespace`), o
- el carácter en `offsets[i].0` es espacio en blanco.

La tercera condición cubre un tokenizador cuyo primer token de palabra incluye el espacio previo en su offset (`▁foo` sobre `" foo"`). La segunda cubre uno que no lo incluye. No hace falta saber cuál de los dos es el real.

Lo que cuenta es el texto, no los tokens. El embedder vuelve a tokenizar el texto de la pieza (`embedding_text`, `chunker/mod.rs:66-72`), y `memory_fts` indexa ese texto. La ventana de tokens sólo decide dónde cortar el texto. Un token vacío (un `▁` suelto con offset `(k, k)`) puede marcarse como inicio de palabra junto con el token que le sigue. Eso es correcto, porque cortar en cualquiera de los dos deja el corte en el mismo byte, al lado de un espacio.

| Alternativa | Por qué no |
|---|---|
| `Encoding::get_word_ids()` | Agrupa según el pre-tokenizador de `tokenizer.json`. El real no está en el repositorio y no se puede abrir sin el modelo. Con `Whitespace`, el pre-tokenizador del fixture actual, cada signo es una palabra y el corte podría caer entre `crates/clawg` y `-core`. Con un `Metaspace` sin `split`, el texto entero es una palabra y el arreglo no haría nada, en silencio |
| Recortar el texto de la pieza hasta el último espacio (B de la exploración) | Deja la pieza por debajo del tope sin contar cuánto, y el tope deja de ser del tokenizador |
| Marcar también los signos de puntuación como frontera | Un camino o un identificador (`crates/clawg-core/src/memory/…`, `Store::insert`) quedaría partido en el índice léxico, que es justamente lo que gana el bono por identificador |

Un texto sin espacios (CJK) sólo tiene la marca del token 0, y el plan cae en el corte al token de hoy (D2).

## D2. El plan de ventanas con marcas

`plan_word_windows(starts: &[bool], chars: &[(usize, usize)], max, overlap) -> Vec<(usize, usize)>`, pura, en `token_splitter.rs`. `chars[i]` son los caracteres del texto antes del inicio y antes del fin del token `i` (`char_spans(text, offsets)`, pura), lo que cuenta `join_pieces`. `n = starts.len()`. `overlap` se acota a `max - 1`, como lo acota `split`. Se parte de `s = 0` y `prev_e = 0`.

1. **Fin.** Si `s + max >= n`, el fin es `n` y la ventana es la última. Si no, el fin es el mayor `e` en `(max(s + overlap, prev_e), s + max]` con `starts[e]`. Si no hay ninguno, se corta al token en `e = s + max`.
2. **Inicio siguiente** ([D3](#d3-el-solape-va-a-la-frontera-más-cercana)). Con `t = e - overlap`, es la marca más cercana a `t` en `(s, e)`, la anterior si empatan. Si el texto que esa marca comparte con la ventana, de `chars[s'].0` a `chars[e - 1].1`, tiene menos de `MIN_OVERLAP_CHARS` caracteres, es la última marca antes de ella que los tiene. Si no hay marca en `(s, e)`, o ninguna los tiene, es la última marca en `(s, t]`, o `t` al token. Con `overlap == 0` no hay piso, y el inicio siguiente es `e` cuando `e` es una marca.
3. `prev_e = e`, `s = s'`, y vuelta a 1.

Propiedades, que fijan los tests exhaustivos:

- **Termina.** `s` crece estrictamente, porque `s'` está en `(s, e)` o es `t`, y `t > s` porque `e > s + overlap`. `e` también crece estrictamente: `e > prev_e`, y en la rama al token `s + max > prev_e`, porque `prev_e <= s_anterior + max < s + max`.
- **Ninguna ventana pasa el tope** (`e - s <= max`) **ni está dentro de otra** (inicios y fines crecientes).
- **Cubre `[0, n)` sin huecos**, porque `s' <= e`. Con `overlap == 0` las ventanas cubren el texto sin repetirlo.
- **Ninguna ventana queda chica** salvo la última: el fin está a más de `overlap` tokens de su inicio. Sin esa cota inferior, una palabra corta seguida de una larga (`key:` y un blob en base64) daría una pieza de un token, y una corrida CJK después de texto con espacios repetiría ventanas cada vez más cortas dentro de la anterior.
- **Con todas las marcas verdaderas el paso es fijo**: el fin es `min(s + max, n)` y el inicio siguiente es `t = s + stride`, siempre que esos `overlap` tokens tengan 16 caracteres. Ningún test de `word_tokenizer` ni de `word_level_tokenizer` cambia, porque en ellos todo token es una palabra.

`split_by_tokens` llama a `word_starts`, a `char_spans` y a `plan_word_windows`, y corta los bytes como antes (`offsets[start].0 .. offsets[end - 1].1`). `plan_windows` quedó sin uso fuera de sus tests y se borró con ellos: el código de `.claude/CLAUDE.md` no deja código muerto. Sus propiedades se prueban sobre `plan_word_windows` con todas las marcas.

## D3. El solape va a la frontera más cercana

La exploración proponía empezar la pieza siguiente en la primera frontera `>= e - overlap`, lo que achica el solape. La primera versión de este diseño tomaba la última frontera `<= e - overlap`, lo que lo agranda. Se midió esa versión, y U2 no se cumplió: la importación en seco dejó **56.514 filas contra 51.996, un 8,7 % más**. La estimación de abajo suponía 1,5 tokens por palabra, pero un corte cae dentro de una palabra con una probabilidad proporcional a su largo, y el costo lo da la cola: las "palabras" sin espacios del corpus (caminos, identificadores, código). Con el tokenizador real, 804 ventanas perdían 40 tokens de paso o más.

Una simulación sobre el mismo export y el tokenizador real reproduce la medición (+8,85 %) y compara las reglas:

| Inicio siguiente | Filas | Uniones de menos de 16 caracteres | Cortes dentro de una palabra | Solapes de menos de 12 tokens |
|---|---|---|---|---|
| Paso fijo, antes de este cambio | 52.225 | 0 | 57.921 | 0 |
| Última frontera `<= t` (la primera versión) | +8,85 % | 0 | 332 | 0 |
| Primera frontera `>= t` | +0,30 % | 984 | 18 | 2.630 |
| Frontera más cercana a `t` | +4,28 % | 148 | 28 | 491 |
| Primera frontera `>= t`, con piso de 16 caracteres | +1,21 % | 0 | 78 | 1.668 |
| **Frontera más cercana a `t`, con piso de 16 caracteres** | **+4,50 %** | **0** | **82** | **349** |

Se elige la última fila:

- **La razón del solape.** Es que una oración cortada en el borde quede entera en una de las dos piezas (`PIECE_OVERLAP_TOKENS`, `chunker/mod.rs`). Con la frontera más cercana, el solape medio queda en 24,3 tokens, y sólo un 0,7 % de los bordes comparte menos de 12. Achicarlo hasta la frontera siguiente lo deja en 21,9 de media, con un 3,4 % por debajo de 12.
- **`join_pieces`.** Necesita que la pieza siguiente empiece con al menos `MIN_OVERLAP_CHARS` = 16 caracteres del final de la anterior (`pieces.rs`, `overlap`). Sin un piso, una palabra larga que cruza `t` deja la frontera siguiente a uno o dos tokens de `e`, y `read_memory` uniría con un salto de línea donde no lo hay. Eso pasa en 148 bordes con la frontera más cercana sin piso. El piso retrocede hasta la frontera que deja los 16 caracteres, y por eso `MIN_OVERLAP_CHARS` pasa a `pub(crate)`: las dos reglas tienen que concordar, y una constante duplicada se desfasaría.
- **El costo.** Un 4,5 % más de filas que antes de este cambio queda debajo del techo de U2, que se mantiene. Dejar la primera versión y subir el techo daba la garantía más fuerte sobre el solape a cambio de unas 4.500 filas más en la importación.

Con `overlap == 0` el piso no se aplica, porque no se pidió solape: la cabecera (`SAVE_HEAD_TOKENS`, sólo la primera ventana) y los tests que cubren el texto sin repetirlo siguen igual.

## D4. Brazo (a): menos solape donde el corte cae en fin de oración

Sólo entra si pasa U3 ([proposal](proposal.md#éxito)). Un token es **inicio de oración** si abre una palabra (D1) y el último carácter que no es espacio antes de su offset es `.`, `?` o `!`, o si entre ese carácter y el offset hay una línea en blanco. `sentence_starts(text, offsets)` es pura, como `word_starts`.

En el paso 1 de D2, si hay un inicio de oración en `[s + max - SENTENCE_LOOKBACK_TOKENS, s + max]` (dentro del rango que admite el paso 1), el fin es el mayor de ellos. En el paso 2, si el fin es un inicio de oración, el inicio siguiente es el mayor `s'` con marca de palabra tal que el texto entre `chars[s'].0` y el fin de la ventana tiene al menos `MIN_OVERLAP_CHARS` caracteres. Con eso, `join_pieces` sigue rearmando el texto exacto. El planificador ya recibe los caracteres por token para el piso de D3, y sigue siendo puro.

**Techo del retroceso.** Con un retroceso de `L` tokens y un solape de unos 4 (16 caracteres), el paso es de unos `96 - L - 4` tokens. Supera el de D3, de unos 69 tokens, sólo si `L` es menor que unos 23. Si `L` es grande, el brazo deja más filas, no menos. El barrido 4, 8 y 12 queda por debajo, y una aserción de compilación fija `SENTENCE_LOOKBACK_TOKENS + 4 < PIECE_OVERLAP_TOKENS`.

**Alternativa descartada:** el solape 0 en fin de oración. Es lo más barato en filas, pero `join_pieces` uniría cada borde con un salto de línea que el texto no tiene, y `read_memory` mostraría un save que no es el que se guardó.

## D5. Brazo (b): una cabecera más corta

Sólo entra si pasa U4. Es cambiar `SAVE_HEAD_TOKENS` (`chunker/mod.rs:40`). La aserción `SAVE_HEAD_TOKENS + HEAD_JOIN_RESERVE + PIECE_MAX_TOKENS <= CHUNK_MAX_TOKENS` (`:61`) sigue valiendo para todo valor del barrido. La cabecera ya pasa por D2, así que termina en frontera de palabra con cualquier valor.

El ahorro es `48 - H` tokens por cada pieza posterior a la primera, y no hace falta medirlo. Lo que se mide es lo que puede perderse: la clave del save, que la cabecera lleva al vector de cada pieza ([design D1 de long-save-ranking](../archive/2026-10-15-long-save-ranking/design.md#d1-bajar-el-tope-no-cambiar-la-fórmula)). Los casos `long` abren con la clave en la primera oración, y eso favorece a una cabecera corta: el resultado puede sobrestimarla, y `verify.md` lo dice.

## D6. La rama de medición

Los brazos se implementan con sus tests en una rama aparte que sale de la punta con D1-D3, y cada valor del barrido es un build de esa rama. Las corridas usan `target-test` (Windows) o `target`, con `cargo clean --workspace` entre una versión y la siguiente, como pide `.claude/CLAUDE.md`. Un brazo que pasa su umbral se lleva a la rama del cambio como un commit con sus tests. El que no pasa no deja código; su resultado queda en `verify.md`. La rama de medición y su worktree se borran al terminar.

Una constante que sólo sirva para apagar un brazo no adoptado sería una abstracción sin uso real, y por eso no se deja en el historial.

## D7. El respaldo por bytes no cambia

`split_by_bytes` (`token_splitter.rs:105-142`) también puede cortar dentro de una palabra, pero sólo corre mientras el tokenizador no cargó, o cuando `encode` falla (`:81-88`). Lo usa `fallback_pieces` (`eval_api.rs:69-75`) para dimensionar el set `long` de `golden-mine synth`, que tiene que salir idéntico (`long_rows_do_not_depend_on_the_piece_cap`, `synth/mod.rs:1164`) para que la base y la punta midan lo mismo. Arreglarlo movería el set sin arreglar nada que se guarde con el modelo cargado.
