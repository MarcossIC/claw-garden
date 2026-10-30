# docs-search-quality — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 | M (paso 1 S–M; pasos 2 a 4 S cada uno, cada uno detrás de su puerta) | 33 eval-rigor y 34 memory-rank-fusion, los dos archivados | ver [`state.yaml`](state.yaml) |

## Problema

`search_project` pone el pasaje correcto entre los tres primeros en poco más de una de cada cuatro preguntas: Hit@3 0,27 sobre las 15 del golden de docs (0,20 en las 10 de docs, 0,40 en las 5 de código; [verify de project-indexing](../archive/2026-10-15-project-indexing/verify.md#rag-eval---real---docs)). La [exploración](exploration.md) lo atribuye a tres cosas, verificadas otra vez contra el código de hoy:

- **La medición está sesgada.** `rag-eval --docs` indexa el repo entero (`default_docs_root`, `crates/clawg-eval/src/bin/rag-eval/cli.rs`) y el `verify.md` archivado de project-indexing cita las 15 preguntas textuales: sale primero en 7 de 11 fallas. Hoy sigue siendo el único archivo del corpus que cita una pregunta (comprobado buscando cada pregunta literal en el repo). Además `--docs` no escribe archivo de resultados, así que no hay `--diff` de docs ([`specs/eval`](../../specs/eval/spec.md), «`rag-eval --docs`»).
- **El canal léxico une con OR los 16 primeros tokens de la pregunta**, palabras vacías incluidas, por posición y con repetidos (`fts_match_expression`, `crates/clawg-core/src/memory/search.rs:748-756`; `MAX_LEXICAL_TOKENS`, `:728`). Docs la reusa tal cual (`crates/clawg-core/src/memory/project_docs/mod.rs:336`).
- **Los identificadores se tokenizan distinto según cómo se escriben.** `unicode61` parte en `_` pero no en una joroba camelCase (`V12__project_docs.sql:45-48`), así que `memoryType`, `memory_type` y «memory type» no se encuentran entre sí por el canal léxico.

### Premisas que cambiaron desde la exploración (2026-10-15)

| Premisa | Hoy |
|---|---|
| 33 (eval-rigor) y 34 (memory-rank-fusion) no existen | Los dos están archivados. `--diff` da el p exacto pareado (`stats::mcnemar_exact`) y un intervalo bootstrap; 34 no cambió `fts_match_expression`, sólo el corte relativo de las memorias. |
| Citas de `search.rs` y `project_docs/mod.rs` | `fts_match_expression` `search.rs:748` (era `:619`), `MAX_LEXICAL_TOKENS` `:728`, `fts_tokens` `:761`, `identifier_tokens` `:892`, `is_identifier` `:905`. En `project_docs/mod.rs`: `fuse` `:308`, `search` `:323`, el uso de la expresión `:336`, `plan_refresh` `:124`, `refresh` `:410`, `replace_doc` `:628`; `E5_PASSAGE_PREFIX` `embedder.rs:141`, `E5_MODEL_ID` `:146`. |
| Un cambio del texto léxico obliga a re-embeber | **Ya no hace falta.** Desde RM5 (`stored_vectors`, `mod.rs:516`) el refresco reutiliza el vector de un pasaje cuyo `indexed_text` no cambió, pero sólo si el doc se embebió con el mismo `model_id` (`mod.rs:531`). Si la huella sube de versión entera, no se reutiliza nada; ver la decisión 6. |
| El corpus | Ahora incluye el código Rust, Python y TypeScript partido por tree-sitter (`code.rs`) y respeta `.gitignore` (`corpus.rs:84-96`). |

## Objetivo

3: que la IA entienda cómo funciona cada proyecto. Primero medir bien; después, cada mejora barata del canal léxico de docs por separado, sin tocar el de las memorias ni su ancla.

## Decisiones

Cierran las [decisiones abiertas de la exploración](exploration.md#decisiones-abiertas) con la opción que la exploración recomendaba en cada una. Una puerta que no se abre deja su paso fuera y el cambio puede cerrar con una mejora parcial.

1. **Alcance de la expresión léxica: sólo docs.** Una función nueva en `project_docs`; `fts_match_expression` y el canal léxico de las memorias no cambian. Así no hace falta medir el ancla de memorias (R@5 0,6522) en cada paso.
2. **Paso 1 por contenido.** `rag-eval --docs` deja fuera del corpus medido todo doc cuyo texto contiene una pregunta del golden de docs, con los espacios plegados y sin distinguir mayúsculas. Cubre docs futuros sin una lista que mantener. Producción no cambia: el filtro vive en la ruta de evaluación.
3. **Un archivo de resultados y un `--diff` de docs.** `rag-eval --real --docs` escribe `docs/eval/results/{fecha}-real-docs[-{tag}].json` (fuera de git), con la posición de cada pregunta, y `--diff` compara dos de esos archivos con el mismo p exacto pareado que el resto. Sin eso, la puerta de cada paso es aritmética a mano.
4. **Umbral por paso, fijado ahora.** Con 15 preguntas un acierto son 6,7 puntos y ninguna prueba pareada llega a p < 0,05 con menos de 6 preguntas moviéndose en un sentido. Por eso la puerta de cada paso es: ninguna pregunta que estaba en el top 3 sale de él, y el total gana al menos 2 en Hit@3. El 0,70 de project-indexing queda como aspiración, no como salida. Si un paso pasa la puerta, `--diff` informa además su p (que no la decide).
5. **Un golden de docs más grande, antes del paso 2.** El set pasa de 15 a al menos 40 preguntas, mitad docs y mitad código, más al menos 10 sin respuesta en el proyecto (para el piso de relevancia). Las preguntas nuevas siguen la regla de hoy: evitan las palabras del pasaje que las responde. Si el set crece, la puerta pasa a ser `--diff` con `Hit@3` no peor de forma distinguible y la ganancia mínima de la decisión 4.
6. **La huella del índice separa vectores de texto léxico.** `project_doc_file.model_id` guarda `docs_contract(model_id)`: el id del embedder más la versión del formato léxico (por ejemplo `multilingual-e5-small/mean+lex2`). `plan_refresh` reindexa un doc cuya huella difiere, como hoy, y `stored_vectors` reutiliza un vector cuando coincide sólo la parte del embedder, así que cambiar el texto léxico reescribe `project_doc_fts` sin re-embeber. Una huella vieja (sin `+lex`) se lee como versión léxica 1. Sin migración de esquema: la columna ya existe.
7. **Rareza por heurística.** Identificadores primero (`is_identifier`), luego los tokens más largos, cada uno una vez; sin `fts5vocab`.
8. **Formas partidas sólo en el texto léxico.** `split_identifier` (`memoryType` → `memory type`) suma las formas partidas al texto que va a `project_doc_fts`, nunca al que se embebe ni al cuerpo que se muestra. Sin columna nueva ni migración.
9. **Palabras vacías: una constante en `project_docs`**, español e inglés, con la excepción de los identificadores (`id`, `db` sobreviven si se escriben como nombre).
10. **El prefijo `passage:` de docs se prueba en un worktree descartable** y se productiviza sólo si gana la puerta de la decisión 4; si no, se descarta sin código. Productivizarlo sube la parte del embedder de la huella, que sí re-embebe.
11. **Fuera:** reranker y modelos distintos (pasos 6 y 7 de la exploración), el nombre en el breadcrumb de código (paso 5, sólo si 2 a 4 dejan preguntas de código sin acertar, en otro cambio) y el piso de relevancia (paso 8), que necesita las preguntas sin respuesta de la decisión 5 y se propone aparte con su medición.

## Alcance

1. **Paso 1 (medición).** El filtro de docs que citan una pregunta, con su cuenta en el informe; un `expected` que cita su propia pregunta falla la validación (salida 2); el archivo de resultados de docs y su `--diff`. Línea base nueva con `--real`.
2. **Golden de docs ampliado** (decisión 5) y su línea base.
3. **Paso 2 (expresión léxica de docs).** `docs_match_expression`: sin palabras vacías ni tokens de menos de 3 caracteres que no sean identificadores, sin repetidos, identificadores y largos primero, una ruta de la pregunta como frase; intenta AND sobre los 3 más raros y cae al OR si no hay resultados; una pregunta toda de palabras vacías cae a la expresión de hoy. Cada pieza se mide sola (2a a 2e de la exploración).
4. **Paso 3 (identificadores partidos)** con `docs_contract` y su test de huella vieja.
5. **Paso 4 (experimento del prefijo)**, sin código si no gana.
6. `specs/project-docs` («Búsqueda», «Refresco incremental», «Datos de la versión anterior», «Límites conocidos») y `specs/eval` («`rag-eval --docs`»), con las mediciones antes y después.

### No-alcance

- La expresión léxica, el prefijo o el ranking de las memorias.
- El troceo de Markdown o de código, el tope de tokens de un pasaje, `RRF_K`, `CANDIDATES_PER_CHANNEL` y el intervalo de refresco.
- Reranker, modelos de embedding nuevos, un watcher o una herramienta `index_project`.
- El piso de relevancia de `search_project` (va en un cambio propio con las preguntas sin respuesta).

## Cómo se mide el éxito

- **Paso 1:** tests que hoy fallan (spec DQ1 a DQ4); `rag-eval --real --docs` antes y después, informando los docs excluidos. Su línea base es la que usan los pasos siguientes, y nadie debe leer su cambio como una regresión del producto.
- **Pasos 2 a 4:** cada uno contra el anterior con `rag-eval --real --docs` y `--diff`, bajo la puerta de la decisión 4, informando Hit@1/3/5 y MRR@10 de docs y de código, el tiempo de indexado y el de una búsqueda.
- La memoria no se mueve: `indexing_docs_leaves_memory_search_unchanged` y los tests de `fts_match_expression` siguen igual.

## Datos y vuelta atrás

- Los pasos 1 y 2 no tocan nada guardado.
- El paso 3 cambia la huella de los docs: el primer refresco de cada proyecto reescribe su `project_doc_fts` doc por doc y reutiliza los vectores (decisión 6). El índice se deriva del disco, no hay migración y ninguna memoria cambia. Volver a una versión anterior: su `plan_refresh` ve una huella distinta y reindexa (con el `model_id` viejo no reutiliza los vectores, así que re-embebe una vez), sin perder nada.
- Test con datos de la versión anterior (`openspec/config.yaml`): un índice con huella sin `+lex` abre, sigue contestando mientras se reescribe, no pierde pasajes y no llama al embedder para los pasajes que no cambiaron.
