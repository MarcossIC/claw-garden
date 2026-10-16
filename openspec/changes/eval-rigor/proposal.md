# eval-rigor — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición | M | — (condiciona a english-recall 21, memory-rank-fusion 34, search-abstention 35, knn-prefilter 37, docs-search-quality 20 y a la medición previa de conflict-candidates 24) | ver [`state.yaml`](state.yaml) |

## Problema

`rag-eval` reporta cada número como un punto sin intervalo, y `--diff` cuenta casos que mejoran y empeoran sin decir si la diferencia se distingue del ruido. Las constantes de ranking se eligieron y se reportan sobre las mismas 46 consultas. Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código); lo que sigue está verificado otra vez contra `main` (`f8a361e`):

1. **No hay ninguna prueba ni intervalo.** `accumulate` suma `better`/`worse` y `net` (`crates/clawg-eval/src/bin/rag-eval/diff.rs:146-154`), y `print_metric` imprime esos tres números (`diff.rs:177-179`). Con 46 casos, un caso son 2,2 puntos, y la menor mejora neta que alcanza p < 0,05 en una prueba pareada exacta son **6 casos en una sola dirección** (p = 0,031); 5 a 0 da 0,0625. Las diferencias con las que se eligieron `RELEVANCE_RATIO` 0,35 y el piso 0,79 son de 1 a 4 casos.
2. **`--diff` no ve lo que recibe el agente.** `diff_values` sólo lee `retrieval.per_case` y `similarity.per_case` (`diff.rs:108-118`). Dos corridas de `--agent-view` con `found` distinto dan 0/0/0. Al verificar project-indexing, `search_memory` bajó de 41/46 a 30/46 y se comparó a mano ([verify de project-indexing](../archive/2026-10-15-project-indexing/verify.md), backlog de [`openspec/README.md`](../../README.md#backlog-sin-spec-arreglos-sueltos)).
3. **Lo que se elige y lo que se reporta salen de lo mismo.** `--sweep-cos-floor`, `--sweep-imp` y `--sweep-lexical` miden sobre el mismo golden que produce el ancla, y no hay forma de separar una parte para reportar.
4. **El esquema no puede expresar dos clases que piden los cambios que siguen.** `RetrievalCase` (`crates/clawg-eval/src/bin/rag-eval/golden.rs:55-82`) no tiene casos sin respuesta ni de versión nueva contra vieja. `validate` no rechaza un `expected_refs` vacío, pero `recall_at_k` lo cuenta como 0 (`crates/clawg-eval/src/bin/rag-eval/metrics.rs:31-36`): un caso sin respuesta escrito hoy bajaría los agregados. Las filas se siembran con la hora actual (`crates/clawg-core/src/eval_api.rs:277-280`), así que tampoco se puede decir cuál es la vieja.
5. **No hay referencia para un número absoluto.** `--sweep-lexical 0` apaga el canal léxico (`cli.rs:49-54`), pero falta el brazo opuesto: ordenar sólo por BM25, sin vectores. Sin él no se puede decir si el embedder aporta algo más que buscar las palabras.
6. **Un acierto con varias esperadas no está definido.** `recall_at_k` es fraccional (`metrics.rs:21-37`), y 12 de los 112 casos del set expandido tienen varias esperadas. Nada dice cuántos casos encontraron todas y cuántos al menos una.
7. **Tres tests de `--diff` leen un archivo ignorado.** `self_diff_is_a_no_op`, `diff_isolates_similarity_from_ranked_movement` y `diff_files_reads_both_paths_and_produces_the_same_result_as_diff_values` leen `docs/eval/results/2026-08-16-real-symmetric.json` (`diff.rs:190-205`), que `.gitignore:31-32` deja fuera de git: fallan en toda máquina que no lo tenga, también en Linux en `main`.

Una corrección a la exploración: su tabla de p exactos tiene dos filas corridas. `(1, 8)` da 0,0391 y `(1, 7)` da 0,0703; `(1, 9)` da 0,0215. La conclusión no cambia.

## Decisiones

Las decisiones abiertas de la exploración se cierran con la opción recomendada en cada una:

- **Prueba pareada:** la binomial exacta de dos colas sobre los casos que se movieron (McNemar exacto, que en datos pareados es la prueba del signo). Vale para toda métrica por caso, binaria o continua, porque sólo mira la dirección del cambio.
- **Intervalo de una diferencia continua** (MRR, nDCG@10, tokens): bootstrap pareado percentil, 10.000 remuestreos, semilla fija en el código.
- **Nivel y métricas primarias:** 95 %. La primaria de recuperación es R@5; la de agent-view, `search.found`. El resto se imprime como informativo, y el texto dice «no distinguible con N casos», nunca «sin diferencia».
- **Acierto con varias esperadas:** se informan las dos, siempre juntas: **estricto** (todas en el top-k) y **parcial** (al menos una). R@k fraccional sigue igual.
- **Partición:** 70 % calibración y 30 % prueba por el sha256 de la consulta, reportada en cada corrida. No hay un flag de barrido nuevo: la regla para una constante nueva es elegirla mirando calibración y citar prueba ([diseño, D4](design.md#d4-partición-por-hash)).
- **Clases nuevas:** `unanswerable` (sin esperadas; se mide la abstención) y `update` (una fila nueva y otras viejas del mismo hecho; se mide que la nueva quede antes). El caso multiagente y multiproyecto queda fuera.
- **Dónde se mide la abstención:** en la recuperación (la lista rankeada vacía) y en agent-view (`search_memory` no imprime ninguna línea). Calibrar una puerta es de search-abstention (35).
- **`--fail-on-regression`:** opcional, sólo con `--diff`; falla si una métrica primaria empeora con p < 0,05.
- **`--docs` no entra:** queda para docs-search-quality (20).
- **Generador del bootstrap:** un SplitMix64 propio en `rag-eval`, copiado del de `golden-mine` y fijado con los mismos valores; no hay crate de biblioteca compartido ([diseño, D3](design.md#d3-generador-del-bootstrap)).
- **Set versionado y escrito a mano (enfoque D):** queda fuera mientras Marcos no elija otra cosa en la tarjeta del hilo, que recomienda dejarlo fuera. Los tests usan fixtures dentro del código.

## Alcance

1. Módulo `stats.rs` de `rag-eval` con `wilson`, `mcnemar_exact`, `paired_bootstrap_ci` y `min_one_way_significant`, puras y probadas contra valores conocidos.
2. `--diff` con p exacto en cada métrica, intervalo bootstrap en las continuas, y bloques nuevos: `agent_view` (`search.found`, `context.found` y tokens de cada herramienta, unidos por `query`), la abstención y la frescura. Un archivo sin esos bloques sigue leyéndose.
3. `--fail-on-regression` opcional.
4. En el reporte: aciertos estrictos y parciales con su intervalo de Wilson en el agregado, por idioma, por subconjunto y por partición; y en la nota de procedencia, cuántos casos en una dirección hacen falta para una conclusión.
5. Clases `unanswerable` y `update` en el golden (`stale_refs` en el caso y `age_days` en la fila), con su validación, su métrica y su bloque; fuera de R@k, MRR y nDCG la primera, dentro la segunda.
6. Brazo `--lexical-only`: ordena por BM25 con la misma consulta FTS del canal léxico de producción, sin vectores.
7. Los tres tests de `--diff` leen un fixture dentro del código.
8. `.claude/CLAUDE.md` (nombra `rag-eval` en la tabla del workspace pero no cómo comparar dos corridas: se agrega una línea), `openspec/README.md` (mapa y backlog) y el texto de ayuda de `rag-eval`.

## Fuera de alcance

- Cambiar `RELEVANCE_RATIO`, el piso de coseno, el umbral de contradicción o cualquier ranking. Este cambio mide; si la medición dice que una constante no se sostiene, se abre otro cambio.
- Correr nada sobre `~/.clawg*` ni ampliar el set privado.
- Un set versionado (enfoque D), salvo que Marcos lo pida en la tarjeta.
- La clase multiagente y multiproyecto, y pruebas de significancia para `--docs`.
- Intervalos de latencia y un reporte HTML.
- La tabla de CLS del comentario de `contradiction.rs:35-45` (es del core; va al backlog).

## Cómo se mide el éxito

Cada requisito tiene un test que falla hoy ([spec](spec.md)). Además:

- **Los números no cambian.** `rag-eval --real` sobre el golden privado da un `--diff` contra el ancla (`2026-10-15-real-project-indexing-tip-anchor.json`) con 0 mejores y 0 peores en todas las métricas, y el mismo `content_sha256`. Corre en la máquina de Marcos, donde está el set.
- **El diff ve lo que hoy no ve.** `--diff` entre `2026-10-15-real-relsweep-*-agent-view.json` 0,60 y 0,35 informa en `search.found` 6 mejores, 0 peores y p = 0,031; entre 0,35 y 0,20, 1 y 0, p = 1,0.
- **La suite:** 1.902 tests en Windows antes del cambio; en Linux, las 3 fallas de `diff::tests` desaparecen.

## Migración y vuelta atrás

No toca `memory.db` ni ningún dato guardado: `rag-eval` siembra un store temporal en cada corrida. Los archivos de resultados viejos se siguen leyendo (cada campo nuevo es opcional al leer). Un golden con las claves nuevas no lo lee un `rag-eval` anterior (`deny_unknown_fields`), lo cual es aceptable dentro del repo; un golden sin ellas se lee y mide igual que hoy. Volver atrás es revertir los commits.
