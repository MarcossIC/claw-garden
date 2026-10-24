# piece-boundaries — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (guardar y recuperar bien) | S (el corte en frontera de palabra) – M (si se adopta algún brazo) | ninguno; va antes de la importación grande | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y premisas verificadas en la [exploración](exploration.md#problema-verificado-contra-el-código).

1. **El borde de una pieza cae dentro de una palabra.** `split_by_tokens` (`crates/clawg-core/src/memory/chunker/token_splitter.rs:74-103`) arma las ventanas con `plan_windows(offsets.len(), max, overlap)` (`:93`, definida en `:153-175`), que sólo cuenta índices de token, y corta el texto en `offsets[start].0 .. offsets[end - 1].1` (`:96-99`). Con un tokenizador de subpalabras, la ventana siguiente puede empezar en la continuación de una palabra: en el corpus importado hay piezas que empiezan en `ies` o `vant Files`. La fila guarda ese fragmento, `memory_fts` indexa un término que no existe y el vector promedia un pedazo sin sentido en sus extremos.
2. **La cabecera hereda el mismo defecto.** `split_save` corta la cabecera con el mismo divisor, `split(content, SAVE_HEAD_TOKENS, 0)` (`chunker/chat.rs:120`), así que termina donde caiga el token 48. Ese texto va a `memory_piece.context` y se antepone a cada pieza posterior al embeberla (`embedding_text`, `chunker/mod.rs:66-72`).
3. **No hay vía de reparación después.** Una fila conserva las piezas en que se partió, el re-embebido usa la cabecera guardada (`embedding_text`) y una reimportación encuentra la nota presente y no cambia nada (`.claude/CLAUDE.md`, `clawg import`). El contrato de embeddings no lleva ningún parámetro del divisor (`expected_contract`, `crates/clawg-core/src/memory/embedder.rs:145-152`), así que el arreglo tampoco dispara un re-embebido. Lo que se importe antes de este cambio queda con los bordes actuales.
4. **El divisor tiene cuatro clientes**, y todos cortan así: el worker (`split_save`, `worker.rs:761-765`), la traducción de la importación (`portability/translate.rs:328`), el índice de docs (`emit`, `project_docs/chunk.rs:127-144`) y `EvalDocIndex` (`eval_api.rs:924`), que parte como el índice de docs.

Los tests de hoy no lo ven: `word_tokenizer` (`chunker/mod.rs:13-19`) y `word_level_tokenizer` (`token_splitter.rs:716`) son `WordLevel` sobre `Whitespace`, y un token es siempre una palabra entera. No hay en `crates/` ningún fixture de subpalabras.

## Decisiones

Las cinco decisiones abiertas de la [exploración](exploration.md#decisiones-abiertas) se toman con la opción que la exploración recomendaba, salvo donde se dice otra cosa. Se pueden cambiar antes de `apply`.

- **El arreglo entra en `TokenSplitter::split`**, y alcanza los cuatro clientes y la cabecera sin código aparte. Un pasaje de docs sólo se vuelve a partir cuando su archivo cambia de sha256 o cambia el modelo, así que el índice existente no se mueve hasta entonces. Un parámetro que sólo active `split_save` dejaría al índice de docs con el defecto a cambio de nada.
- **La frontera se lee en los bytes, no en `word_ids`** ([design D1](design.md#d1-la-frontera-sale-de-los-bytes-no-de-word_ids)). Esto cambia lo que proponía la exploración: `word_ids` depende del pre-tokenizador de `tokenizer.json`, que no está en el repositorio y no se puede verificar sin el modelo. Además, con `Whitespace` trata cada signo como frontera y partiría `crates/clawg-core` en `crates/clawg` y `-core`. La regla de bytes (un corte es frontera si le sigue o le precede espacio en blanco) vale igual si el offset del primer token incluye o no el espacio previo, que era el primer riesgo de la exploración.
- **El solape sigue siendo de 24 tokens como mínimo**, y se alarga hacia atrás hasta el inicio de la palabra ([design D3](design.md#d3-el-solape-crece-hasta-la-palabra-no-se-achica)). Achicarlo hasta la frontera, como decía la exploración, podía dejar un solape de un token cuando una palabra larga cruza el punto `e - 24`. Entonces `join_pieces` no encuentra los 16 caracteres que necesita (`MIN_OVERLAP_CHARS`, `pieces.rs:57`) y `read_memory` uniría con un salto de línea donde no lo hay.
- **Brazo (a), menos solape en fin de oración:** con solape mínimo de `MIN_OVERLAP_CHARS` caracteres alineado a palabra, para que `join_pieces` siga rearmando el texto exacto. Se busca el fin de oración a lo sumo `SENTENCE_LOOKBACK_TOKENS` tokens hacia atrás desde el tope, con el barrido 4, 8 y 12. El techo sale de la aritmética: con un retroceso de `L` tokens y un solape de unos 4, el paso es `96 - L - 4`, y sólo supera los 72 de hoy si `L < 20` ([design D4](design.md#d4-brazo-a-menos-solape-donde-el-corte-cae-en-fin-de-oración)).
- **Brazo (b), una cabecera más corta:** barrido 32, 24 y 16 contra los 48 de hoy.
- **El criterio de «no pierde»** está fijado abajo, en [Éxito](#éxito), antes de medir.
- **El recuento de filas sale de la importación en seco** (`clawg import --from engram --dry-run`), que ya parte cada nota con el tokenizador del modelo y cuenta las filas que insertaría. No se escribe una herramienta de conteo. Los tokens embebidos que ahorra el brazo (b) no se miden: salen de la cuenta, `48 - H` por cada pieza posterior a la primera.
- **Si ningún brazo gana, el cambio cierra con el corte en frontera de palabra solo.** Los brazos se miden en una rama de medición. Sólo el que se adopta entra en el historial, con sus tests, y el que no se adopta deja su resultado en `verify.md` y nada de código.

## Alcance

1. **Ventanas en frontera de palabra** (`chunker/token_splitter.rs`): una función pura que marca qué tokens abren una palabra a partir del texto y los offsets, y otra, también pura, que planifica las ventanas con esas marcas. `split_by_tokens` las usa. Con todas las marcas verdaderas, el plan es exactamente el de `plan_windows`.
2. **Un fixture de subpalabras** compartido por los tests del divisor, de `split_save`, de `join_pieces` y de la traducción: un `WordPiece` sobre `WhitespaceSplit` con raíces y continuaciones `##`, en `cfg(test)`.
3. **Un test contra el `tokenizer.json` real**, `#[ignore]` y con el modelo en `CLAWG_E5_MODEL_DIR` como `crates/clawg-core/tests/e5_golden.rs:265`, que fija que en el tokenizador real tampoco hay piezas a mitad de palabra.
4. **La medición**: el recuento de filas con la importación en seco y `rag-eval` sobre el ancla y el set `long`, con el corte solo y con cada brazo.
5. **Los brazos (a) y (b)**, sólo si pasan su umbral.
6. **El comentario desfasado de `pieces.rs:58-59`**, que todavía dice que el chunker repite 64 tokens.

## Fuera de alcance

- **El respaldo por bytes** (`split_by_bytes`, `token_splitter.rs:105-142`). También puede cortar dentro de una palabra, pero `eval_api::fallback_pieces` (`eval_api.rs:69-75`) lo usa para dimensionar el set `long` de `golden-mine synth`, y `long_rows_do_not_depend_on_the_piece_cap` (`crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:1164`) exige que ese set no cambie. Sin tokenizador, el corte es un respaldo transitorio.
- **Los topes**: 96/24 de las piezas, 480/64 de un chunk y el 480 que decide si un save se parte.
- **Volver a partir filas guardadas o notas importadas.**
- **El esquema, el contrato de embeddings y el prefijo E5.**
- **Textos sin espacios (CJK).** No tienen frontera dentro de la ventana y se siguen cortando al token, como hoy.
- **El ranking de los resúmenes de sesión** (ranking-at-scale).

## Datos guardados

No cambia el esquema, ningún identificador ni ningún estado, y no hay migración. Las filas, vínculos, cabeceras y vectores de los saves ya partidos quedan como están: `join_pieces` busca el solape en el texto y no depende de dónde se cortó, y el re-embebido usa la cabecera guardada en `memory_piece.context`. Lo que este cambio deja distinto es sólo dónde caen los bordes de un save nuevo. El test de R4 de la [spec](spec.md) guarda saves con el divisor anterior y comprueba que siguen apareciendo, leyéndose enteros y re-embebiéndose igual.

**Rollback:** el binario anterior abre la base sin cambios. Los saves partidos con bordes de palabra son pedazos vinculados como cualquier otro, y la versión anterior los colapsa, los lee y los re-embebe igual.

## Éxito

**Tests que fallan hoy** (detalle en la [spec](spec.md)), todos con el fixture de subpalabras:

- ninguna pieza de un save largo empieza ni termina dentro de una palabra;
- la cabecera termina en frontera de palabra;
- ningún pasaje del índice de docs empieza ni termina dentro de una palabra;
- la traducción de una nota larga no deja una fila que empiece a mitad de palabra;
- `join_pieces` rearma el texto exacto de un save partido con subpalabras.

**Guardas, que pasan hoy y tienen que seguir pasando:** el tope de 96 tokens por pieza, una palabra más larga que el tope cortada al token sin perder bytes, el plan de `plan_windows` cuando todo token abre palabra (así ningún test de `word_tokenizer` cambia), el respaldo por bytes y `long_rows_do_not_depend_on_the_piece_cap`.

**Medición, con umbrales fijados antes de medir.** Requiere el modelo real del perfil dev, el golden set local (`docs/eval/golden.json`, fuera de git), el set `long` de `golden-mine synth --seed 7` y, para el recuento, un export de `engram export --all` fuera del repositorio con un daemon de cada build sobre el perfil dev. «Sin regresión» es: `rag-eval --diff --fail-on-regression` no sale con 6 sobre el ancla de 46 casos, ni sin `--agent-view` ni con él, y en `long` ningún caso pasa de visible a no visible en `search_visible` ni en `context_visible`, ni de hallado a no hallado.

| # | Condición | Corrida |
|---|---|---|
| U0 | registrar la base de `develop`: ancla, `long` en agent-view (`hit_rate`, `search_reachable`, `search_visible`, `context_visible`) y filas de la importación en seco | `rag-eval --real`, `rag-eval --real --agent-view --golden <synth seed 7>` y `clawg import --from engram --dry-run` |
| U1 | el corte en frontera de palabra, sin regresión contra U0; en los otros subconjuntos de agent-view ninguna métrica baja más de 0,02 | las mismas, con la punta |
| U2 | el corte en frontera de palabra deja a lo sumo un 5 % más de filas que U0 en la importación en seco (se estima de +2 a +4 %, [design D3](design.md#d3-el-solape-crece-hasta-la-palabra-no-se-achica)) | importación en seco |
| U3 | el brazo (a) se adopta con el menor `SENTENCE_LOOKBACK_TOKENS` del barrido que deja al menos un 5 % menos de filas que U1 y no tiene regresión contra U1 | las tres, con cada valor |
| U4 | el brazo (b) se adopta con la menor cabecera del barrido sin regresión contra U1 | `rag-eval`, con cada valor |

Con 12 casos `long`, una conclusión necesita al menos 6 moviéndose en el mismo sentido. Un empate es «no distinguible», no «sin diferencia», y por eso el umbral es que ningún caso empeore y no una diferencia de medias. Un valor se elige leyendo `by_split.calibration` y se informa `by_split.test`. Los dos brazos se miden por separado contra U1. Si los dos se adoptan, la combinación se mide una vez más con el mismo criterio antes de que entre. Si U1 o U2 no se cumplen, el cambio no se archiva y vuelve a diseño.
