# docs-search-quality — Diseño

Decisiones con alternativas. Requisitos en la [spec](spec.md) (DQ1 a DQ9); decisiones de alcance en la [propuesta](proposal.md).

## D1. Dónde se deja fuera un doc que cita una pregunta (DQ1)

| Opción | A favor | En contra |
|---|---|---|
| **a. Un conjunto de rutas a dejar fuera que `refresh` recibe** (elegida) | Mide exactamente lo que indexa producción, salvo esas rutas: mismo recorrido, mismos topes, mismo `.gitignore`, mismos breadcrumbs | Toca `refresh`, aunque producción le pasa un conjunto vacío |
| b. Copiar los docs del recorrido a un árbol temporal sin los excluidos | Producción no cambia ni una línea | Duplica el repo en disco en cada corrida; un doc salteado por tamaño en el original no se copia y la cuenta de `skipped` deja de ser la de producción |
| c. Una lista de rutas en el golden | La más simple | Hay que mantenerla; un doc nuevo que cite una pregunta contamina en silencio |

`refresh` (`pub(crate)`) recibe `left_out: &HashSet<String>`, que el daemon pasa vacío, y `scan` deja fuera esas rutas antes de leerlas: el doc no entra en `disk`, así que `plan_refresh` no lo indexa y `RefreshReport::docs` no lo cuenta. `EvalDocIndex::index` recibe la lista. Decidir qué docs citan una pregunta es puro y vive en `rag-eval` (`docs.rs`): `quoting_docs(cases, docs)` pliega espacios y minúsculas en el texto y en cada pregunta, busca la subcadena y devuelve las rutas. Los textos se leen del disco en `rag-eval`, con `doc_paths` como hoy; un doc que no es UTF-8 no cita nada (el índice también lo saltea).

Un `expected` que cita una pregunta es un golden inválido (DQ2, salida 2) y no una exclusión silenciosa: dejarlo fuera haría fallar `check_corpus` con un mensaje que no dice por qué.

Un heading esperado sin pasaje también es un golden inválido (DQ2), y se comprueba así:

| Opción | A favor | En contra |
|---|---|---|
| **a. Partir el doc con el splitter del índice antes de indexar** (elegida) | Los breadcrumbs son los de producción: mismo enmascarado, mismo splitter, mismas secciones sin cuerpo que no dan pasaje; falla antes de los minutos del indexado real | Expone `EvalDocIndex::breadcrumbs` y saca el despacho por tipo de `refresh` a `project_docs::passages` |
| b. Buscar el heading con un regex en el markdown | No toca `clawg-core` | Da por bueno un heading cuya sección sólo tiene subsecciones, que no tiene pasaje, y no sirve para código |
| c. Comprobar contra los pasajes guardados después de indexar | Exacto | Con el modelo real falla después de embeber todo el repo |

`check_headings(cases, crumbs)` es pura y vive en `docs.rs`; `expected_breadcrumbs` pide al índice los breadcrumbs de cada doc que un caso espera bajo un heading, y `measure_with` corre las dos antes de `index`.

## D2. El archivo de resultados y su `--diff` (DQ3, DQ4)

| Opción | A favor | En contra |
|---|---|---|
| **a. Un archivo propio, `kind: "docs"`, y un diff propio en `docs.rs`** (elegida) | Las métricas de docs (Hit@k por pregunta, docs y código) no son las de memorias; nada de `RunResults` ni de `diff.rs` cambia | `run_diff` tiene que elegir el diff por el `kind` |
| b. Meter docs en `RunResults` | Un solo formato | `RunResults` arrastra decenas de campos de memorias que una corrida de docs no tiene; cada lector actual tendría que saltearlos |

El nombre sale de `report::results_filename` con el tag `docs` o `docs-{tag}`, así que una corrida de memorias y una de docs del mismo día no chocan. `--fake` sólo imprime, como `revision-eval` y `load-eval`: el embedder falso no mide calidad y un archivo suyo sólo confundiría un `--diff`. La comprobación de que el archivo no existe va antes de indexar, porque el indexado real tarda minutos.

`run_diff` lee los dos JSON una vez; si alguno tiene `kind: "docs"`, los dos tienen que tenerlo y el diff es `docs::diff_values`. Usa `stats::mcnemar_exact`, `paired_bootstrap_ci` y la misma semilla que `diff.rs`, con el `MetricDiff` de `diff.rs` (que ya es `pub`), así que el p y el intervalo de una corrida de docs se leen igual que los de memorias. El veredicto primario es Hit@3; `--fail-on-regression` mira sólo ese.

## D3. Expresión léxica de docs (DQ6, condicional)

Una función nueva, `docs_match_expression(query) -> DocsQuery { all: Option<String>, any: Option<String> }`, en `project_docs`, que reutiliza `fts_tokens` e `is_identifier` de `search.rs` (este último pasa a `pub(crate)`). `search` corre el `all` (AND de los 3 primeros) y, si no devuelve nada, el `any` (OR). Rechazado:

- cambiar `fts_match_expression`: mueve el canal léxico de las memorias y su ancla (decisión 1 de la propuesta);
- rareza por `fts5vocab`: una tabla virtual más o una consulta por token, por una ganancia sin medir sobre la heurística; se reabre si la heurística no pasa su puerta.

Las piezas 2a a 2e se encienden una por commit y se miden de a una, en ese orden.

## D4. Huella del índice (DQ7, DQ8, condicional)

| Opción | A favor | En contra |
|---|---|---|
| **a. `model_id + "+lex" + N` en la columna que ya existe, reutilizando por la parte del embedder** (elegida) | Sin migración; un cambio del texto léxico reescribe `project_doc_fts` sin re-embeber (para este repo, minutos de embedder en debug) | La reutilización compara una parte de la cadena, no la cadena |
| b. Subir la cadena entera | Sin código en `stored_vectors` | Re-embebe todos los docs de todos los proyectos por un cambio que no toca un vector |
| c. Una columna `lexical_version` | La más explícita | Migración V32 por un dato derivado y reconstruible |

Dos funciones puras, `docs_contract(model_id) -> String` y `vector_part(stored) -> &str` (todo lo anterior a `+lex`, o la cadena entera si no lo tiene). `stored_vectors` deja de filtrar por igualdad de `model_id` en SQL y compara `vector_part` en Rust. Mientras el refresco reescribe doc por doc, cada doc está entero en su versión vieja o en la nueva (`replace_doc` es una transacción), igual que hoy con un cambio de modelo.

## D5. Prefijo de docs (DQ9, condicional)

El experimento cambia `E5_PASSAGE_PREFIX` en un worktree descartable y mide sólo `--docs`. Si gana, se productiviza con una variante de `EmbedRole` propia de docs, que cada `Embedder` resuelve a su prefijo, y la parte del embedder de la huella lo nombra (`…/mean+docpassage`), lo que sí re-embebe. Si no gana, no hay código.

## Orden de commits

1. `feat(eval): leave out of the docs corpus the docs that quote a question` (D1, DQ1, DQ2)
2. `feat(eval): write a docs run's results file` (D2, DQ3)
3. `feat(eval): diff two docs runs` (D2, DQ4)
4. `docs(openspec): …` con la línea base nueva, cuando se corra con el modelo real.
5. En adelante, un commit por pieza de los pasos 2 a 4, cada uno con su medición.
