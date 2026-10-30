# docs-search-quality — Especificación

Suma DQ1 a DQ4 (paso 1, entero) a [`specs/eval`](../../specs/eval/spec.md), DQ5 (el golden ampliado) también a `specs/eval`, y DQ6 a DQ9 (condicionales, cada uno detrás de su puerta) a [`specs/project-docs`](../../specs/project-docs/spec.md). Origen: [proposal](proposal.md).

Las salidas de `rag-eval` son las de `crates/clawg-eval/src/bin/rag-eval/main.rs:52-62`: 1 fallo de ejecución o de uso, 2 golden inválido, 3 sin modelo, 4 archivo de resultados existente, 6 regresión.

## Paso 1: medir bien

### DQ1. El corpus medido no cita las preguntas

`rag-eval --docs` MUST dejar fuera del índice que mide todo doc del corpus cuyo texto contiene, con los espacios plegados a uno y sin distinguir mayúsculas, la pregunta de algún caso del golden de docs. El filtro MUST vivir en la ruta de evaluación: el refresco del daemon y `corpus::walk` MUST NOT cambiar. El informe MUST decir cuántos docs dejó fuera y cuáles, por ruta relativa.

#### Escenario: un doc que cita una pregunta
- GIVEN una raíz con `README.md`, que responde la pregunta q1, y `notes.md`, que la copia textual
- WHEN corre `rag-eval --fake --docs`
- THEN `notes.md` no está entre los docs indexados ni puede salir como hit, y el informe dice `left out 1 doc quoting a question: notes.md`

#### Escenario: una cita con otros saltos de línea
- GIVEN un doc que tiene la pregunta partida en dos líneas y con otra capitalización
- WHEN se decide qué docs citan una pregunta
- THEN ese doc la cita

#### Escenario: nada que excluir
- GIVEN una raíz sin docs que citen preguntas
- WHEN corre `rag-eval --fake --docs`
- THEN el informe dice `left out 0 docs quoting a question` y los números son los de antes

### DQ2. Un `expected` que cita su pregunta invalida el golden

Si un doc que un caso espera cita la pregunta de ese mismo caso, o de cualquier caso, `rag-eval --docs` MUST salir con 2 nombrando el caso y el doc, antes de indexar: medirlo sería medir coincidencia textual. Un `expected` que no está en el corpus MUST seguir saliendo con 2 como hoy (`check_corpus`).

#### Escenario
- GIVEN un caso q1 que espera `notes.md`, y `notes.md` copia la pregunta de q1
- WHEN corre `rag-eval --fake --docs`
- THEN sale con 2 y stderr nombra `q1` y `notes.md`

### DQ3. El archivo de resultados de docs

`rag-eval --real --docs` MUST escribir `{out-dir}/{fecha}-real-docs.json`, o `{fecha}-real-docs-{tag}.json` con `--tag`, en el directorio de resultados de siempre (`--out-dir`, por defecto `docs/eval/results/`, fuera de git). Un archivo existente MUST NOT pisarse sin `--force` (salida 4), y la comprobación MUST hacerse antes de indexar. `--fake --docs` MUST sólo imprimir. El archivo MUST llevar:

- `kind: "docs"`, la fecha, el modo, el tag, el `model_id` del embedder y el sha corto de git;
- el golden: su sha256 y la cantidad de casos (sin ruta absoluta);
- el corpus: docs, pasajes, salteados y las rutas relativas que DQ1 dejó fuera;
- el tiempo de indexado en segundos;
- `aggregate`: Hit@1, Hit@3, Hit@5 y MRR@10 de todas las preguntas, y aparte de docs y de código;
- `per_case`: por pregunta, `id`, `code`, `rank` (desde 1, `null` fuera de los 10 primeros) y `top`, el breadcrumb del primer hit.

MUST NOT llevar el texto de ningún pasaje ni rutas absolutas.

#### Escenario
- GIVEN dos preguntas, una en el puesto 2 y otra sin acierto
- WHEN se arman los resultados y se escriben en un directorio temporal
- THEN el archivo se llama `{fecha}-real-docs.json`, `per_case` trae `rank` 2 y `null`, y un segundo intento sin `--force` falla con `AlreadyExists`

### DQ4. `--diff` de dos corridas de docs

`rag-eval --diff A B` con dos archivos `kind: "docs"` MUST unir las preguntas por `id` y MUST informar, para todas y aparte para docs y para código, cuántas mejoraron y cuántas empeoraron en Hit@1, Hit@3 y Hit@5, con el p exacto pareado (`stats::mcnemar_exact`), y el cambio medio de MRR@10 con su intervalo bootstrap. MUST nombrar las preguntas que entraron o salieron del top 3, las que están en un solo archivo (fuera de la cuenta), y avisar cuando los dos archivos difieren en el sha256 del golden o en los docs excluidos. Con `--fail-on-regression` MUST salir con 6 si Hit@3 empeora con p < 0,05. Un archivo de docs contra uno que no es de docs MUST ser un error (salida 1).

#### Escenario: una pregunta sale del top 3
- GIVEN A con q1 en el puesto 1 y q2 en el 4, y B con q1 en el 5 y q2 en el 2
- WHEN corre `rag-eval --diff A B`
- THEN Hit@3 da 1 mejor y 1 peor, p = 1, y el informe nombra `q1` como salida del top 3 y `q2` como entrada

#### Escenario: archivos de dos clases
- GIVEN A de docs y B de memorias
- WHEN corre `rag-eval --diff A B`
- THEN sale con 1 y dice que no compara una corrida de docs con una de memorias

### DQ5. El golden de docs ampliado

`crates/clawg-eval/golden/project-docs.json` MUST tener al menos 40 preguntas con respuesta, al menos 15 de docs y 15 de código, que sigan evitando las palabras del pasaje que las responde. `the_shipped_golden_set_names_docs_this_workspace_has` MUST comprobar que ninguna cita su propia respuesta (DQ2) y la cantidad mínima, no un número exacto. Las preguntas sin respuesta quedan para el cambio del piso de relevancia: el formato de hoy exige un `expected` por caso.

## Pasos 2 a 4: condicionales

Cada uno entra sólo si pasa su puerta ([propuesta](proposal.md), decisión 4); si no, su requisito no se consolida.

### DQ6. Expresión léxica propia de docs

`project_docs::search` MUST armar su consulta léxica con `docs_match_expression` y MUST NOT usar `fts_match_expression`, que las memorias siguen usando sin cambios. `docs_match_expression` MUST ser pura y:

- quitar las palabras vacías de español e inglés (`DOCS_STOPWORDS`) y los tokens de menos de 3 caracteres, salvo los que son identificadores (`is_identifier`);
- tomar cada token una vez, identificadores primero y después de más largo a más corto, desempatando por posición, hasta `MAX_LEXICAL_TOKENS`;
- buscar como frase una ruta de la pregunta (un token con `/` o un nombre de archivo con extensión);
- si quitar las vacías deja la consulta sin tokens, caer a la expresión de hoy y no a ninguna.

La búsqueda MUST intentar primero el AND de los 3 primeros tokens de esa lista y, si no devuelve ningún pasaje, el OR de todos.

#### Escenario: una pregunta larga
- GIVEN «¿qué hace el daemon con los saves pendientes cuando se apaga de golpe?»
- WHEN se arma la expresión
- THEN no contiene `"qué"`, `"el"`, `"con"`, `"los"`, `"de"` ni `"se"`, y contiene `"pendientes"`, `"daemon"` y `"saves"`

#### Escenario: un identificador corto
- GIVEN «where is CHUNK_MAX_TOKENS used and what is db_id»
- WHEN se arma la expresión
- THEN `"CHUNK_MAX_TOKENS"` y `"db_id"` están primero y `"is"`, `"and"`, `"what"` no están

#### Escenario: sólo palabras vacías
- GIVEN «what is the»
- WHEN se arma la expresión
- THEN es la de `fts_match_expression` para esa pregunta

#### Escenario: las raras ganan
- GIVEN un pasaje que comparte con la pregunta sólo «de», «el» y «the», y otro que comparte sus dos palabras raras
- WHEN se busca
- THEN el segundo queda primero por el canal léxico

### DQ7. Identificadores partidos en el texto léxico

El texto que va a `project_doc_fts` MUST ser el `indexed_text` del pasaje seguido, tras una línea en blanco, de las formas partidas (`split_identifier`) de sus identificadores camelCase y PascalCase, cada una una vez. El texto embebido, el `body` guardado y el que devuelve la búsqueda MUST NOT cambiar. La consulta MUST sumar las formas partidas de sus identificadores.

#### Escenario
- GIVEN un pasaje que dice `memoryType` y otro que dice `memory_type`
- WHEN se busca «memory type», `memoryType` y `memory_type`
- THEN las tres consultas encuentran los dos pasajes por el canal léxico

### DQ8. La huella separa vectores y texto léxico

`project_doc_file.model_id` MUST guardar `docs_contract(model_id)`, el id del embedder seguido de `+lex{N}` con la versión del formato léxico. `plan_refresh` MUST reindexar un doc cuya huella difiere. `stored_vectors` MUST reutilizar un vector cuando coincide la parte del embedder (todo lo anterior a `+lex`), aunque la versión léxica difiera. Una huella sin `+lex` MUST leerse como versión léxica 1.

#### Escenario: un índice de la versión anterior
- GIVEN un índice armado con la huella `m1` (sin `+lex`)
- WHEN refresca un embedder `m1` con la versión léxica 2
- THEN cada doc se reescribe, `project_doc_fts` trae las formas partidas, el embedder no recibe ningún texto, y una búsqueda durante y después del refresco devuelve los mismos pasajes

#### Escenario: otro modelo
- GIVEN un índice con huella `m1+lex2`
- WHEN refresca un embedder `m2`
- THEN cada pasaje se embebe otra vez, como hoy

### DQ9. Prefijo de docs

Sólo si el experimento gana: los pasajes de docs MUST embeberse con un prefijo propio del índice, y la parte del embedder de la huella MUST nombrarlo, de modo que un índice viejo se re-embeba. Las memorias MUST NOT cambiar de prefijo.
