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

`plan_word_windows(starts: &[bool], max, overlap) -> Vec<(usize, usize)>`, pura, al lado de `plan_windows` (`token_splitter.rs:153`). `n = starts.len()`. `overlap` ya viene acotado a `max - 1`, como en `plan_windows`. Se parte de `s = 0` y `prev_e = 0`.

1. **Fin.** Si `s + max >= n`, el fin es `n` y la ventana es la última. Si no, el fin es el mayor `e` en `(max(s + overlap, prev_e), s + max]` con `starts[e]`. Si no hay ninguno, se corta al token en `e = s + max`.
2. **Inicio siguiente.** Es el mayor `s'` en `(s, e - overlap]` con `starts[s']`. Si no hay ninguno, `s' = e - overlap`, al token.
3. `prev_e = e`, `s = s'`, y vuelta a 1.

Propiedades, que fijan los tests exhaustivos (como los de `plan_windows`, `token_splitter.rs:205-353`):

- **Termina.** `s` crece estrictamente: `s' > s` en la rama con marca, y `e - overlap > s` en la rama al token, porque `e > s + overlap`. `e` también crece estrictamente: `e > prev_e`, y en la rama al token `s + max > prev_e`, porque `prev_e <= s_anterior + max < s + max`.
- **Ninguna ventana pasa el tope** (`e - s <= max`) **ni está dentro de otra** (inicios y fines crecientes).
- **Cubre `[0, n)` sin huecos**, porque `s' <= e - overlap < e`.
- **Ninguna ventana queda chica** salvo la última: el fin está a más de `overlap` tokens de su inicio. Sin esa cota inferior, una palabra corta seguida de una larga (`key:` y un blob en base64) daría una pieza de un token, y una corrida CJK después de texto con espacios repetiría ventanas cada vez más cortas dentro de la anterior.
- **Con todas las marcas verdaderas da exactamente `plan_windows`**: el fin es `min(s + max, n)` y el inicio siguiente es `e - overlap = s + stride`. Ningún test de `word_tokenizer` ni de `word_level_tokenizer` cambia, porque en ellos todo token es una palabra.

`split_by_tokens` llama a `word_starts` y a `plan_word_windows` en lugar de `plan_windows`, y corta los bytes como hoy (`:96-99`). `plan_windows` queda para lo que ya la usa, y sus tests siguen como el modelo de referencia de D2. Si al implementar resulta que sólo la usan sus tests, se borra con ellos: el código de `.claude/CLAUDE.md` no deja código muerto.

## D3. El solape crece hasta la palabra, no se achica

La exploración proponía empezar la pieza siguiente en la primera frontera `>= e - overlap`, lo que achica el solape. D2 toma la última frontera `<= e - overlap`, lo que lo agranda. Las dos reglas son igual de simples. Se elige la segunda por dos razones:

- **`join_pieces`.** Necesita que la pieza siguiente empiece con al menos `MIN_OVERLAP_CHARS` = 16 caracteres del final de la anterior (`pieces.rs:57`, `overlap`, `:85-93`). Si una palabra larga (un camino de 20 tokens) cruza `e - overlap`, la primera frontera después de ese punto puede estar a un token de `e`, el solape queda por debajo de 16 caracteres y `read_memory` uniría con un salto de línea donde no lo hay. Con la segunda regla, el solape tiene al menos los 24 tokens de hoy.
- **La razón del solape.** Es que una oración cortada en el borde quede entera en una de las dos piezas (`PIECE_OVERLAP_TOKENS`, `chunker/mod.rs:52-55`). Achicarlo debilita justamente eso.

**Costo.** Cada ventana pierde al final, en promedio, menos de una palabra, y gana al principio menos de otra. Si cada palabra tiene unos 1,5 tokens (una suposición para inglés y español, que no se midió con el tokenizador real), el paso baja de 72 a unos 69-70 tokens, y las filas de un save largo suben de un 2 a un 4 %. Es una estimación. U2 lo mide sobre la importación en seco, con su techo del 5 %.

## D4. Brazo (a): menos solape donde el corte cae en fin de oración

Sólo entra si pasa U3 ([proposal](proposal.md#éxito)). Un token es **inicio de oración** si abre una palabra (D1) y el último carácter que no es espacio antes de su offset es `.`, `?` o `!`, o si entre ese carácter y el offset hay una línea en blanco. `sentence_starts(text, offsets)` es pura, como `word_starts`.

En el paso 1 de D2, si hay un inicio de oración en `[s + max - SENTENCE_LOOKBACK_TOKENS, s + max]` (dentro del rango que admite el paso 1), el fin es el mayor de ellos. En el paso 2, si el fin es un inicio de oración, el inicio siguiente es el mayor `s'` con marca de palabra tal que el texto entre `offsets[s'].0` y el fin de la ventana tiene al menos `MIN_OVERLAP_CHARS` caracteres. Con eso, `join_pieces` sigue rearmando el texto exacto. El planificador recibe el conteo acumulado de caracteres por token en lugar del texto, y sigue siendo puro.

`MIN_OVERLAP_CHARS` pasa a `pub(crate)` y se usa desde `chunker`. Es un uso real: las dos reglas tienen que concordar, y una constante duplicada se desfasaría como se desfasó el comentario de `pieces.rs:58-59`.

**Techo del retroceso.** Con un retroceso de `L` tokens y un solape de unos 4 (16 caracteres), el paso es de unos `96 - L - 4` tokens. Supera los 69-70 de D3 sólo si `L` es menor que unos 22. Si `L` es grande, el brazo deja más filas, no menos. El barrido 4, 8 y 12 queda por debajo, y una aserción de compilación fija `SENTENCE_LOOKBACK_TOKENS + 4 < PIECE_OVERLAP_TOKENS`.

**Alternativa descartada:** el solape 0 en fin de oración. Es lo más barato en filas, pero `join_pieces` uniría cada borde con un salto de línea que el texto no tiene, y `read_memory` mostraría un save que no es el que se guardó.

## D5. Brazo (b): una cabecera más corta

Sólo entra si pasa U4. Es cambiar `SAVE_HEAD_TOKENS` (`chunker/mod.rs:40`). La aserción `SAVE_HEAD_TOKENS + HEAD_JOIN_RESERVE + PIECE_MAX_TOKENS <= CHUNK_MAX_TOKENS` (`:61`) sigue valiendo para todo valor del barrido. La cabecera ya pasa por D2, así que termina en frontera de palabra con cualquier valor.

El ahorro es `48 - H` tokens por cada pieza posterior a la primera, y no hace falta medirlo. Lo que se mide es lo que puede perderse: la clave del save, que la cabecera lleva al vector de cada pieza ([design D1 de long-save-ranking](../archive/2026-10-15-long-save-ranking/design.md#d1-bajar-el-tope-no-cambiar-la-fórmula)). Los casos `long` abren con la clave en la primera oración, y eso favorece a una cabecera corta: el resultado puede sobrestimarla, y `verify.md` lo dice.

## D6. La rama de medición

Los brazos se implementan con sus tests en una rama aparte que sale de la punta con D1-D3, y cada valor del barrido es un build de esa rama. Las corridas usan `target-test` (Windows) o `target`, con `cargo clean --workspace` entre una versión y la siguiente, como pide `.claude/CLAUDE.md`. Un brazo que pasa su umbral se lleva a la rama del cambio como un commit con sus tests. El que no pasa no deja código; su resultado queda en `verify.md`. La rama de medición y su worktree se borran al terminar.

Una constante que sólo sirva para apagar un brazo no adoptado sería una abstracción sin uso real, y por eso no se deja en el historial.

## D7. El respaldo por bytes no cambia

`split_by_bytes` (`token_splitter.rs:105-142`) también puede cortar dentro de una palabra, pero sólo corre mientras el tokenizador no cargó, o cuando `encode` falla (`:81-88`). Lo usa `fallback_pieces` (`eval_api.rs:69-75`) para dimensionar el set `long` de `golden-mine synth`, que tiene que salir idéntico (`long_rows_do_not_depend_on_the_piece_cap`, `synth/mod.rs:1164`) para que la base y la punta midan lo mismo. Arreglarlo movería el set sin arreglar nada que se guarde con el modelo cargado.
