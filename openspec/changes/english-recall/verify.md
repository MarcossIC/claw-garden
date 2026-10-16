# english-recall — Verificación

**Estado:** el código está hecho (tareas 1 a 7) y los datos de la tarea 8 están escritos y fijados por sha. Falta medir: el paso 0 y las hipótesis (a) a (f) necesitan el modelo real en el caché del perfil.

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

Pendientes: necesitan el modelo real en el caché del perfil.
