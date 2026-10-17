# english-recall — Verificación

**Estado:** código, datos y medición hechos (tareas 1 a 12). **Veredicto: D.** El cruce de idiomas explica la brecha ((e) cumple U1 con 13 a 0 y 12 a 0), ningún prefijo la cierra ((b) falla U1 en los dos brazos) y el piso, el canal léxico y el código de las filas no son la causa ((a), (d), (c)). No sigue C. Lo que cambia el resultado es en qué idioma se guarda una memoria, y eso es de memory-protocol (22). Falta archivar.

## Código (tareas 1 a 7)

Un commit por tarea sobre `main`, en este orden:

| Tarea | Commit |
|---|---|
| 1 | `feat(eval): carry the language of a query from the golden set into each case result` |
| 2 | `feat(eval): report retrieval and similarity by the language pair of query and row` |
| 3 | `feat(eval): report agent-view hit rates by the language pair of query and row` |
| 4a | `feat(eval): cut retrieval and similarity diffs by the language pair of the cases` |
| 4b | `feat(eval): cut agent-view diffs by the language pair of the cases` |
| 5 | `docs(eval): describe the language-pair cut in the --diff help` |
| 6 | `docs: describe the language-pair cut when comparing two runs` |

En Windows, `cargo test --workspace --target-dir <otro-dir>` pasa y `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` termina con 0 antes de cada commit y en la punta. Las guardas de R1 y R3 (`a_case_without_query_lang_has_none`, `a_misspelled_query_lang_key_fails_to_parse`, `a_golden_without_query_lang_measures_exactly_as_before`) y las de `diff.rs` (`self_diff_is_a_no_op`, `a_significant_primary_regression_fails`, `five_lost_cases_do_not_fail`, `an_informative_regression_never_fails`, `diff_exits_six_on_a_significant_regression_only_with_the_flag`) quedaron en verde en cada commit. `a_golden_without_query_lang_measures_exactly_as_before` se corrió también antes del código de la tarea 2, y pasó.

### Diferencias con las tareas

- `a_case_without_query_lang_has_none` nombra el campo nuevo, así que no compila antes de la tarea 1; lo que la cubría antes es `valid_minimal_file_validates`.
- `audit.rs:664` no necesitó el campo: arma su caso con `..case(...)`.
- Se sumó `an_agent_view_run_labels_each_case_with_its_pair` (`main.rs`): corre `--fake --agent-view` con un golden etiquetado y es lo único que falla si `runner.rs` deja de copiar `query_lang` o `row_lang`.
- El mapa `ref → lang` sale de una función privada de `main.rs`, `row_langs`, que usan `run()` y el runner de agent-view, en vez de armarse dos veces.
- `--diff` arma su salida en una función pura (`render`) que `print_stdout` imprime: es lo que deja comparar las líneas `primary:` con y sin parejas.
- Los dos conteos (`pair differs between the files`, `pair in one file only`) se imprimen con cuatro espacios, bajo las filas globales de su bloque: con dos se leían como un bloque aparte.
- El resumen de `--agent-view` suma una línea por pareja, y la consola, una tabla por pareja para `similarity`, que antes no se imprimía.

## Datos (tarea 8)

Todo vive en `docs/eval/`, que git ignora. Se escribió antes de cualquier corrida de un brazo y sin mirar resultados por caso de este cambio.

### 8.1 Etiquetas de `golden.json`

- Copia previa: `docs/eval/golden.pre-english-recall.json`, idéntica byte a byte al archivo de antes.
- Regla: el idioma de una consulta o de una fila es el de su prosa; los identificadores y el código no cuentan.
- `query_lang` en los 46 casos: `en` en los casos de las filas c10, c15, c17, c33, c40, c42 y c45; `es` en los otros 39.
- `lang` de c01, c08, c09 y c35 pasa de `es` a `en`. Ninguna otra fila cambia.
- Contra la copia previa, el diff de texto son 46 líneas `query_lang` nuevas y las 4 líneas `lang`; consultas, filas, esperadas, pools y los 28 pares de contradicción son los mismos.
- Parejas resultantes: `es>es` 20, `en>en` 7, `es>en` 19, las de la exploración.
- `sha256` antes: `d2d9d7320bce841126eac163790f1052d268521912cfcc6aac4f9af466ed792a`; después: `363d3e08854bfa222ccf9944508938e0e58a8dab9808696eca7a10525da0a1fb`.

### 8.2 Goldens de control

Los cuatro parten de `golden.json` ya etiquetado y conservan su formato (JSON con sangría de dos espacios y CRLF). Ninguno entra en `golden.json` ni en git.

**`golden-cross.json`** (U6). Regla, fijada antes de escribir la primera consulta:

1. Una consulta por fila del corpus, 48 en total: en inglés para cada una de las 22 filas cuya prosa está en español (c04, c05, c06, c13, c14, c19, c20, c21, c22, c23, c24, c28, c29, c30, c31, c32, c34, c37, c38, c39, c47, c48), y en español para cada una de las 26 filas cuya prosa está en inglés.
2. La consulta pregunta por un hecho de la fila como lo haría alguien que busca esa memoria sin recordar sus palabras: una sola pregunta, sin copiar frases de la fila y sin traducir la consulta que la fila ya tiene en `golden.json`.
3. Sin identificadores de código (nada de lo que en la fila va entre comillas invertidas, ni nombres de archivos, clases, funciones, campos o componentes) y sin nombres propios de tecnologías, productos o proyectos: lo que comparte una consulta con su fila es significado, no tokens.
4. Cada caso espera sólo su fila. Su pool (`corpus_refs`) es el del caso de `golden.json` que espera la misma fila; c47 y c48, que no tienen caso, usan el par `[c47, c48]` con la propia primero.
5. Mismas 48 filas y mismos 28 pares de contradicción que `golden.json`.

**`golden-c-prosa.json`** (control de (c)). En las filas esperadas por los casos `es>en` que tienen al menos un tramo entre comillas invertidas (c02, c03, c12, c18, c25 y c26) se borra cada tramo con sus comillas; después se colapsan los espacios repetidos y se quita el espacio que quede antes de `,`, `.`, `)`, `;` o `:`. Nada más cambia.

**`golden-e-consulta-en.json`** (control de (e)). Las consultas de los 19 casos `es>en` se traducen al inglés, frase por frase y sin sumar palabras de la fila; su `query_lang` pasa a `en`. Nada más cambia. Como la consulta cambia, el diff contra la base no las une: se aparean a mano por la fila esperada.

**`golden-e-fila-es.json`** (control de (e)). Las 19 filas esperadas por los casos `es>en` se traducen al español, con la prosa completa y sin tocar identificadores, rutas, código, tramos entre comillas invertidas, números ni nombres propios; su `lang` pasa a `es`. Las consultas no cambian, así que el diff contra la base las une por `query`.

| Archivo | Casos | Parejas | `sha256` |
|---|---|---|---|
| `golden-cross.json` | 48 | `en>es` 22, `es>en` 26 | `6a04ea4b0d3e88e87db9387cc933bedb9b499337c86749d8fa3e1a7dd79e5f65` |
| `golden-c-prosa.json` | 46 | las de `golden.json` | `21ba283ad3b1c4d8e20e986dfe20c955bf6b776bc7adb6b88340ed68b5936dcf` |
| `golden-e-consulta-en.json` | 46 | `en>en` 26, `es>es` 20 | `09f266e22df38dc82900b9040e766760c2fd3c7bf88f36528ea5a2387e41f7cc` |
| `golden-e-fila-es.json` | 46 | `en>en` 7, `es>es` 39 | `8606335bfb16123dc3a65ae32012ad54edad73d1c3f92c433cdc6ab54120caa1` |

Cada uno se comparó con `golden.json` campo por campo: sólo cambia lo que su regla nombra, y los 28 pares de contradicción son los mismos. Los cinco archivos, `golden.json` incluido, validan con `rag-eval --fake --golden <archivo> --out <dir temporal>` (salida 0), y cada resultado lleva en `golden_set.content_sha256` el sha de arriba. Una consulta de `golden-cross.json` que repetía un identificador de su fila (`subestados` contra `subEstado`) se reescribió antes de fijar el archivo.

## Paso 0 y las hipótesis (tareas 9 a 12)

### Cómo se midió

- **Commit:** `fed1e3f` de `main`. Trae las tareas 1 a 6 y, además, el embedder sobre ONNX Runtime sin fastembed (`71fd1fa`), cuyos vectores son bit a bit los de antes. El paso 0 lo vuelve a comprobar: no se mueve ningún caso.
- **Qué necesita:** el golden set y sus controles en `docs/eval/` (ignorado por git), el modelo en el caché del perfil `dev` (`rag-eval --real --check-only` sale con 0) y un build debug de `rag-eval`.
- **Comandos:** `rag-eval --real [--agent-view] [--golden docs/eval/<archivo>] --tag <tag>`, y `rag-eval --diff <base> <otro>` para cada comparación. Cada corrida escribe `docs/eval/results/2026-10-17-real-<tag>[-agent-view].json` (los barridos, `-cos<piso>.json` y `-sweep.json`; el léxico, `-lexical-only.json`).
- **Prueba:** McNemar exacto bilateral sobre los casos que se mueven, la misma de `--diff`. Cuando las consultas de un control no son las de la base (`golden-e-consulta-en.json`, `golden-cross.json`) o la pareja de sus casos cambia (`golden-e-fila-es.json`), `--diff` no los une en un bloque de pareja. En esos casos la prueba se calcula apareando por fila esperada (cada caso espera una fila distinta) sobre el `per_case` de los dos archivos.
- **Intervalos:** Wilson al 95 %, los del `hits` de cada `by_pair`.

| `--tag` | Golden (`sha256`) | Qué es |
|---|---|---|
| `english-recall-base` | `golden.json` (`363d3e08…`) | A0, con y sin `--agent-view` |
| `english-recall-base-cross` | `golden-cross.json` (`6a04ea4b…`) | A0, con y sin `--agent-view` |
| `english-recall-a` | `golden.json` | (a): cinco pisos |
| `english-recall-d` | `golden.json` | (d): `--lexical-only` |
| `english-recall-c-prosa` | `golden-c-prosa.json` (`21ba283a…`) | control de (c), en A0 |
| `english-recall-e-consulta-en` | `golden-e-consulta-en.json` (`09f266e2…`) | control de (e), en A0 |
| `english-recall-e-fila-es` | `golden-e-fila-es.json` (`8606335b…`) | control de (e), en A0 |
| `english-recall-b-a1`, `-b-a1-cross` | `golden.json`, `golden-cross.json` | brazo A1, con y sin `--agent-view` |
| `english-recall-b-a2`, `-b-a2-cross` | `golden.json`, `golden-cross.json` | brazo A2, con y sin `--agent-view` |

### Paso 0: la base

**9.1** `--diff` de `2026-10-16-real-eval-rigor-tip-anchor.json` contra `2026-10-17-real-english-recall-base.json`: 0 casos movidos en las cinco métricas de `retrieval` y de `similarity` (p = 1,0 en todas, cambio medio de MRR y nDCG@10 +0,0000 [+0,0000, +0,0000]). Además, las 46 listas rankeadas son idénticas caso por caso en los dos bloques, la curva de contradicciones y la distribución de cosenos son las mismas, y `content_sha256` pasa de `d2d9d732…` a `363d3e08…` sólo por las etiquetas. `--diff` informa `pair in one file only: 46` en cada bloque, como corresponde: el ancla no tiene `query_lang`. Global de `retrieval`: R@1 0,4348, R@5 0,6522, R@10 0,8478, MRR 0,5524, nDCG@10 0,6215.

| Pareja | n | Bloque | R@1 | R@5 | R@10 | MRR |
|---|---|---|---|---|---|---|
| `es>es` | 20 | `similarity` | 14/20 [0,481, 0,855] | 18/20 [0,699, 0,972] | 19/20 [0,764, 0,991] | 0,8050 |
| `en>en` | 7 | `similarity` | 5/7 [0,359, 0,918] | 6/7 [0,487, 0,974] | 6/7 [0,487, 0,974] | 0,7619 |
| `es>en` | 19 | `similarity` | 3/19 [0,055, 0,376] | **6/19 [0,154, 0,540]** | 11/19 [0,363, 0,769] | 0,2511 |
| `es>es` | 20 | `retrieval` | 13/20 [0,433, 0,819] | 18/20 [0,699, 0,972] | 19/20 [0,764, 0,991] | 0,7800 |
| `en>en` | 7 | `retrieval` | 4/7 [0,250, 0,842] | 6/7 [0,487, 0,974] | 7/7 [0,646, 1,000] | 0,7083 |
| `es>en` | 19 | `retrieval` | 3/19 [0,055, 0,376] | 6/19 [0,154, 0,540] | 13/19 [0,460, 0,846] | 0,2555 |

**9.2** `--diff` de `2026-10-15-real-long-save-ranking-tip-agent-view.json` contra `2026-10-17-real-english-recall-base-agent-view.json`: 0 casos movidos en `search.found`, `context.found` y los tokens de las dos herramientas (cambio medio +0,0 [+0,0, +0,0]). La base de agent-view no se movió con lo que entró después de `04ab3cd`, así que el archivo nuevo la reemplaza sin cambio de cifras: `search_memory` 36/46 a 764,0 tokens, `recall_context` 39/46 a 1.086,4.

| Pareja | n | `search.found` | `context.found` | Tokens de `search` | Tokens de `context` |
|---|---|---|---|---|---|
| `es>es` | 20 | 19/20 | 19/20 | 842,8 | 1.124,2 |
| `en>en` | 7 | 6/7 | 7/7 | 354,3 | 978,9 |
| `es>en` | 19 | 11/19 | 13/19 | 832,2 | 1.086,3 |

**9.3** Base de U6, `golden-cross.json` (48 casos):

| Pareja | n | `similarity` R@5 | `similarity` R@10 | `retrieval` R@5 | `search.found` | `context.found` |
|---|---|---|---|---|---|---|
| `en>es` | 22 | 21/22 [0,782, 0,992] | 22/22 | 21/22 | 21/22 | 22/22 |
| `es>en` | 26 | 19/26 [0,539, 0,863] | 23/26 | 19/26 | 22/26 | 23/26 |

Agent-view: `search_memory` 43/48 a 644,9 tokens, `recall_context` 45/48 a 1.054,7.

**9.4, hipótesis (f).** El `by_pair` del paso 0 reproduce la tabla de la exploración: n 20/7/19 y R@5 0,90/0,86/0,32 en `retrieval` y en `similarity`, y también sus R@10 (`retrieval` 0,95/1,00/0,68; `similarity` 0,95/0,86/0,58). **Veredicto: cierta.** Las etiquetas leídas a mano dan las mismas parejas que la exploración, y con n = 19 cada caso de `es>en` vale 5 puntos: el intervalo de R@5 va de 0,15 a 0,54.

### (a) El piso

`--sweep-cos-floor` 0,70, 0,75, 0,79, 0,81 y 0,83 sobre A0; cada punto, `--diff` contra la base. U1 se lee en `retrieval`, el único bloque con piso:

| Piso | `es>en` R@5 | Ganados / perdidos | p | `en>en` R@5 |
|---|---|---|---|---|
| 0,70 | 7/19 | 1 / 0 | 1,0000 | 6/7 |
| 0,75 | 7/19 | 1 / 0 | 1,0000 | 6/7 |
| 0,79 (la de hoy) | 6/19 | 0 / 0 | 1,0000 | 6/7 |
| 0,81 | 9/19 | 3 / 0 | 0,2500 | 6/7 |
| 0,83 | 9/19 | 5 / 2 | 0,4531 | 2/7 (1 / 5) |

`similarity` no se mueve en ningún punto, como corresponde: no tiene piso. **Veredicto: descartada.** Ningún piso cumple U1 contra 0,79, y el único que se acerca (0,83) se lo cobra con `en>en`.

### (b) El prefijo

Dos worktrees descartables desde `fed1e3f`, cada uno con un solo cambio en `crates/clawg-core/src/memory/embedder.rs`. A1 pone `E5_PASSAGE_PREFIX` en `"passage: "` (línea 128). A2 pone `E5_QUERY_PREFIX` y `E5_PASSAGE_PREFIX` en `""` (líneas 107 y 128). Cada brazo se compiló con su propio directorio de build dentro del worktree. `similarity.query_prefix`/`passage_prefix` de cada archivo prueban qué brazo corrió: A0 `"query: "`/`"query: "`, A1 `"query: "`/`"passage: "`, A2 `""`/`""`. Los archivos de agent-view no guardan los prefijos: su brazo lo prueban el `--tag` y que los escribió el mismo binario que la corrida sin `--agent-view` de ese brazo.

| # | Métrica | Base (A0) | A1 | A2 |
|---|---|---|---|---|
| U1 | `es>en` R@5, `similarity` | 6/19 | 5/19: 1 / 2, p = 1,0000. **No** | 4/19: 1 / 3, p = 0,6250. **No** |
| U2 | `es>en` R@10, `similarity` | 11/19 | 7/19: 1 / 5, p = 0,2188. **Baja** | 10/19: 2 / 3, p = 1,0000. **Baja** |
| U3 | `es>es` R@5 / R@10, `similarity` | 18/20 / 19/20 | 19/20 / 19/20. Cumple | 19/20 / 19/20. Cumple |
| U4 | `en>en` R@5 / R@10, `similarity` | 6/7 / 6/7 | 6/7 / 6/7. Cumple | 6/7 / 7/7. Cumple |
| U5 | Global `similarity`: R@1, R@5, R@10, MRR, nDCG@10 | 0,4783, 0,6522, 0,7826, 0,5697, 0,6199 | 0,5217, 0,6522, **0,6957**, 0,5759, **0,6047**. **Baja** | **0,4348**, **0,6304**, 0,7826, **0,5327**, **0,5915**. **Baja** |
| U6 | `golden-cross.json`, R@5 `es>en` / `en>es`, `similarity` | 19/26 / 21/22 | **14/26** (0 / 5, p = 0,0625) / 21/22. **Baja** | **12/26** (0 / 7, p = 0,0156) / 21/22. **Baja** |
| U7 | Global `retrieval`: R@1, R@5, R@10, MRR, nDCG@10 | 0,4348, 0,6522, 0,8478, 0,5524, 0,6215 | 0,5217 (5 / 1, p = 0,2188), 0,6522 (2 / 2, p = 1,0000), 0,7391 (0 / 5, p = 0,0625), 0,5814 (+0,0289 [−0,0290, +0,0899]), 0,6187 (−0,0028 [−0,0581, +0,0528]) | 0,4130 (4 / 5, p = 1,0000), 0,6522 (3 / 3, p = 1,0000), 0,7826 (0 / 3, p = 0,2500), 0,5245 (−0,0279 [−0,1157, +0,0601]), 0,5860 (−0,0355 [−0,1095, +0,0375]) |
| U8 | `search.found` / `context.found` | 36/46 / 39/46 | 31/46 (0 / 5, p = 0,0625) / 34/46 (0 / 5, p = 0,0625) | 34/46 (1 / 3, p = 0,6250) / 36/46 (0 / 3, p = 0,2500) |
| U9 | Tokens de `search` / `context`, cambio medio | 764,0 / 1.086,4 | 761,8 (−2,3 [−44,0, +38,7]) / 1.115,8 (+29,4 [−2,1, +60,8]) | 886,3 (+122,2 [+66,3, +183,9]) / 1.106,5 (+20,2 [−12,8, +53,3]) |
| U10 | Recall de contradicciones en 0,40 / 0,45 / 0,50 (`fpr_no_contradiction` 0 en todos) | 0,8 / 0,9 / 1,0 | 0,8 / 0,8 / 0,9 (`fpr_identical` 0,88 en 0,40) | 0,8 / 0,9 / 0,9 |

En agent-view, `es>en` pierde en los dos brazos: `search.found` 11/19 → 6/19 en A1 y 9/19 en A2; `context.found` 13/19 → 8/19 y 10/19. `es>es` y `en>en` no se mueven. En el golden cruzado, `search.found` baja de 43/48 a 36/48 (A1) y a 40/48 (A2).

**Veredicto: (b) no explica la brecha.** Es la última fila de la tabla de lectura: U1 a U6 no se cumplen en ninguno de los dos brazos, y U7 y U8 no mejoran con p < 0,05. Ninguno de los dos brazos gana en `es>en`, y A2 pierde 7 casos de `es>en` en la réplica (p = 0,0156). Con mean, ningún brazo supera al `"query: "` simétrico de hoy en el global ni en el cruce, lo mismo que el comentario de `embedder.rs` cuenta de CLS. Lo único que mejora es el MRR de `es>es` en A1 (4 / 0, p = 0,1250, +0,0867 [+0,0117, +0,1733]), que no decide nada.

### (c) Composición

Casos `es>en` que fallan R@5 en `similarity`, según la fila esperada tenga o no al menos un tramo entre comillas invertidas:

| Brazo | Con código (n = 6) | Sólo prosa (n = 13) |
|---|---|---|
| A0 | 6 fallan (c02, c03, c12, c18, c25, c26) | 7 fallan (c07, c08, c09, c11, c27, c41, c44) |
| A1 | 6 fallan (los mismos) | 8 fallan (c08, c09, c11, c27, c35, c36, c41, c44) |
| A2 | 6 fallan (los mismos) | 9 fallan (c01, c07, c09, c11, c27, c36, c41, c44, c46) |

Control `golden-c-prosa.json` en A0, apareado por fila con la base: en `es>en`, R@1 3/19 → 3/19 y R@5 6/19 → 6/19 (0 / 0), y R@10 11/19 → 9/19 (0 / 2: c12 y c25, p = 0,5000). **Veredicto (informativa): el código no es la causa.** Las 6 filas con código fallan en todos los brazos, pero borrar sus tramos no recupera ninguna. El código acompaña al fallo, no lo produce.

### (d) Lo léxico

`--lexical-only` sobre A0: `es>en` R@5 2/19 [0,029, 0,314] y R@10 2/19. Contra la base, en `es>en`, R@5 da 2 / 6 (p = 0,2891) y R@10 0 / 11 (p = 0,0010). Para comparar: `en>en` R@5 5/7 y `es>es` 16/20. Los dos aciertos son casos que el ranking completo deja fuera de los 5 primeros:

- c03: el canal léxico lo pone 2.º y el ranking completo 6.º. Comparten `plural` y `singular`, que se escriben igual en los dos idiomas.
- c08: el canal léxico lo pone 4.º y el ranking completo 10.º. Comparten `research`, una palabra en inglés dentro de la consulta en español.

**Veredicto: confirmada.** Una consulta en español casi no comparte términos con una fila en inglés, así que el canal léxico no rescata el cruce y no se toca.

### (e) Traducción

Los dos controles en A0, apareados por fila esperada con la base (`similarity`, los 19 casos de `es>en` del ancla):

| Control | R@1 | R@5 | R@10 |
|---|---|---|---|
| `golden-e-consulta-en.json` (la consulta en inglés) | 3/19 → 17/19, 14 / 0, p = 0,000122 | 6/19 → **19/19 [0,832, 1,000]**, **13 / 0, p = 0,000244** | 11/19 → 19/19, 8 / 0, p = 0,007812 |
| `golden-e-fila-es.json` (la fila en español) | 3/19 → 14/19, 11 / 0, p = 0,000977 | 6/19 → **18/19 [0,754, 0,991]**, **12 / 0, p = 0,000488** | 11/19 → 19/19, 8 / 0, p = 0,007812 |

`retrieval` da los mismos R@5. En `golden-e-fila-es.json`, que cambia 19 filas del corpus, las otras parejas no se mueven: `es>es` y `en>en` quedan en 0 / 0 en R@5 y `es>es` pierde un caso en R@10 (0 / 1). **Veredicto: el cruce explica la brecha.** Las dos variantes cumplen U1 contra la base con mucho margen: una fila se encuentra cuando consulta y fila están en el mismo idioma, sin importar cuál de los dos lados se traduzca. No hay brazo ganador en (b), así que (e) se midió sólo en A0.

### Fuera del preregistro

Esta lectura es informativa y no entra en ningún veredicto. `golden-cross.json` tiene una consulta en español para cada una de las 19 filas que esperan los casos `es>en` del ancla. Apareadas por fila, esas consultas aciertan R@5 en `similarity` 16/19, contra 6/19 de las del ancla (11 / 1, p = 0,0063). Las del cruzado describen el hecho de la fila casi como una traducción. Las del ancla preguntan por él con otras palabras (por ejemplo, «¿Sobre qué framework está construido el portal…?» para una fila que nombra el framework). Una consulta que cruza de idioma encuentra su fila cuando la describe de cerca, y la pierde cuando además la parafrasea.

### Veredicto

| Hipótesis | Veredicto |
|---|---|
| (a) El piso | Descartada |
| (b) El prefijo | No explica la brecha: U1 falla en A1 y en A2 |
| (c) Composición | Informativa: el código acompaña al fallo pero no lo causa |
| (d) Lo léxico | Confirmada: no rescata el cruce y no se toca |
| (e) Traducción | El cruce explica la brecha (13 / 0 y 12 / 0) |
| (f) Etiquetas y n | Cierta: el paso 0 reproduce las parejas y las cifras de la exploración |

**Sigue D, no C.** La brecha de `es>en` es una propiedad del modelo con este contenido cuando consulta y fila están en idiomas distintos, y ningún prefijo la cierra. Así que el ranking y el embedder no cambian, y model-reembed (17) no gana urgencia. Lo que la cierra es que consulta y fila estén en el mismo idioma, y eso depende de cómo se escriben las memorias: va a memory-protocol (22).

### Para memory-protocol (22)

Cifras de A0 por pareja consulta → fila:

| Pareja | n | `similarity` R@5 | `search_memory` encuentra | `recall_context` encuentra |
|---|---|---|---|---|
| `es>es` | 20 | 18/20 [0,699, 0,972] | 19/20 | 19/20 |
| `en>en` | 7 | 6/7 [0,487, 0,974] | 6/7 | 7/7 |
| `es>en` | 19 | 6/19 [0,154, 0,540] | 11/19 | 13/19 |
| `es>en`, con la fila escrita en español (control de (e)) | 19 | 18/19 [0,754, 0,991] | — | — |
| `en>es` (`golden-cross.json`) | 22 | 21/22 [0,782, 0,992] | 21/22 | 22/22 |
| `es>en` (`golden-cross.json`) | 26 | 19/26 [0,539, 0,863] | 22/26 | 23/26 |

Una memoria guardada en el idioma en que se la va a buscar se encuentra; una guardada en inglés y buscada en español se pierde en dos de cada tres casos en R@5. 22 decide qué hacer con eso.

### Diferencias con las tareas (9 a 12)

- El paso 0 corrió sobre `fed1e3f`, que además de las tareas 1 a 6 trae el embedder sin fastembed. El paso 0 mismo prueba que eso no movió nada.
- Los prefijos están ahora en `embedder.rs:107` y `:128`, no en `:102` y `:123`.
- La base del golden cruzado lleva `--tag english-recall-base-cross` (y los brazos, `-b-a1-cross` y `-b-a2-cross`). Con `english-recall-base` habría pisado el archivo del paso 0.
- Los worktrees de (b) y sus directorios de build se armaron en un directorio temporal fuera del repositorio. `git worktree remove --force` les quitó el registro, pero en Windows no pudo borrar los directorios por rutas demasiado largas dentro del build, y se borraron aparte. Al terminar, `git worktree list` sólo muestra `main` y `git status` no tiene cambios de prefijo.
- Los controles de (c) y (e) corrieron con un `rag-eval` compilado mientras el árbol tenía un cambio sin commitear en `agent_view/runner.rs`, que sólo cambia cómo se cierra el store al terminar agent-view. Esas corridas no son de agent-view y no ejecutan ese código.
- No se anota cuántos tests tiene la suite: la cuenta queda vieja con cada cambio.
