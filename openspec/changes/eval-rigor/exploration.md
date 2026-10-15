# eval-rigor — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición | M | — (condiciona a english-recall 21, memory-rank-fusion 34, search-abstention 35, knn-prefilter 37, docs-search-quality 20 y a la medición previa de conflict-candidates 24) | exploración (borrador) |

**Tesis:** las constantes que deciden la calidad de la recuperación (el piso de coseno, `RELEVANCE_RATIO`, el umbral de contradicción) se eligieron barriendo sobre el mismo golden set chico en el que después se reportan, y el arnés no puede decir si una diferencia es real o ruido. Con 46 casos, un caso son 2,2 puntos y las diferencias que se usaron para elegir son de 1 a 3 casos. Antes de que más cambios de ranking (21, 34, 35, 37, 20) se apoyen en esta medición, hay que hacerla confiable: una prueba pareada con intervalo, un `--diff` que vea también lo que el agente recibe, una parte del set que no se use para elegir y casos que hoy el esquema no puede expresar (sin respuesta, versión nueva contra vieja).

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| No hay pruebas de significancia en `clawg-eval` | **Cierta.** Una búsqueda de bootstrap, McNemar, Wilson, intervalo, permutación, p-valor y varianza en `crates/clawg-eval/src` y en `eval_api.rs` sólo encuentra el generador de permutaciones de identificadores de `golden-mine synth` (`synth/mod.rs:256,281`). Todo número del reporte es un punto sin intervalo. |
| La única comparación es `--diff` y cuenta mejores, peores y neto | **Cierta.** `accumulate` suma `better`/`worse` y `net = better - worse` (`diff.rs:146-154`); `diff_block` une por `query` (`diff.rs:125-138`). No hay umbral ni prueba: un 1 a 0 y un 6 a 0 se imprimen igual. |
| `--diff` sólo mira `retrieval` y `similarity` | **Cierta.** `diff_values` lee `retrieval.per_case` y `similarity.per_case` (`diff.rs:108-118`) y nada más. El texto de ayuda lo dice (`cli.rs:161-165`). El archivo de `--agent-view` trae `agent_view.per_case` con `search.found` y `context.found` por caso, que el diff nunca abre. |
| Eso ya costó un error de medición | **Cierta.** Al verificar project-indexing, `search_memory` bajó de 41/46 a 30/46 al pasar a pooling mean con `RELEVANCE_RATIO` 0,6 y `--diff` no lo mostró; se comparó a mano (`openspec/changes/archive/2026-10-15-project-indexing/verify.md:165-167`). El mismo hueco figura en el backlog del mapa (`openspec/README.md:187`); este cambio toma ese punto. |
| Los barridos y los números reportados usan el mismo set | **Cierta, con un matiz.** `--sweep-cos-floor`, `--sweep-imp` y `--sweep-lexical` corren sobre el mismo `golden.json` que produce el ancla (`cli.rs:143-151`). El piso 0,79 sale de un percentil 5 de cosenos consulta-pasaje de la documentación del repo, y el barrido sobre el golden set lo confirma, no lo origina (`score.rs:161-168`, `095f270`). El umbral de contradicción 0,42 sale de los 20 pares del golden set (`contradiction.rs:15-33`). `RELEVANCE_RATIO` se barrió sobre las 46 consultas y se reporta sobre ellas (`relevance.rs:8-19`). |
| El golden set es de 48 filas y 46 casos, con 28 pares de contradicción | **Cierta.** `golden.json`, `golden-keywords.json` y `golden-oracle.json`: 48 filas de corpus (26 en español, 22 en inglés), 46 casos, todos con una sola fila esperada, 28 pares (10 contradicción, 10 compatibles, 8 idénticos). `golden-expanded.json`: 1.028 filas, 112 casos (46 naturales, 30 léxicos, 12 con varias respuestas, 12 y 12 de importancia), los mismos 28 pares. |
| Los golden sets y los resultados no están versionados | **Cierta.** `.gitignore:31-32` ignora `docs/eval/**` y `docs/**/results/*`. El commit `ed2ddff` lo explica: «the golden sets are mined from private memories and results are machine-specific». |
| El esquema no tiene casos sin respuesta ni de actualización | **Cierta, y hay un matiz.** `RetrievalCase` tiene `query`, `corpus_refs`, `expected_refs`, `status`, `blocked_by`, `history` y `answer` (`golden.rs:55-81`). La validación sólo exige que cada esperada esté en `corpus_refs` (`golden.rs:288-296`), y **no rechaza** una lista de esperadas vacía; pero `recall_at_k` devuelve 0,0 para una lista vacía (`metrics.rs:31-36`), así que un caso sin respuesta escrito hoy contaría como un fallo total y bajaría los agregados. No hay nada que lo mida bien. |
| `golden-mine synth` genera casos de más clases | **Cierta, pero no de éstas.** Genera los subconjuntos `lexical`, `multi_ref`, `high_importance`, `low_importance` y `long` (`synth/golden.rs:56-64`, `synth/mod.rs:236,349,372-373,454`). Ninguno es sin respuesta ni de versión nueva contra vieja. Además el vocabulario de las filas sintéticas sale de la base en el momento de generar (`openspec/changes/archive/2026-10-12-eval-golden-expansion/design.md:25`), y `synth` rechaza una base vacía (`SynthError::EmptyBase`, `synth/mod.rs:46,68`): el set expandido sigue dependiendo del set privado. |

## Problema (verificado contra el código)

### Una diferencia de 1 a 3 casos no se distingue del ruido

Con 46 casos, cada caso pesa 1/46 = 0,0217. La prueba pareada natural es la de McNemar exacta (dos colas) sobre los casos discordantes: los que cambian de acierto a fallo o al revés entre dos corridas. Cálculo sobre `n` discordantes:

| Discordantes (a favor / en contra) | p exacto (dos colas) |
|---|---|
| 5 / 0 | 0,0625 |
| 6 / 0 | **0,0312** |
| 8 / 1 | 0,0703 |
| 9 / 1 | **0,0391** |
| 1 / 0 | 1,0 |

Consecuencia: sobre 46 casos la menor mejora neta que alcanza p < 0,05 es **6 casos, 13,0 puntos**, y sólo si ningún caso se mueve en contra. Sobre los 112 del set expandido sería 5,4 puntos. Cualquier diferencia neta de 1 a 5 casos es indistinguible del azar con este diseño.

Los intervalos de Wilson al 95 % de los aciertos que se usaron para elegir constantes se solapan casi por completo:

| Medida | Casos | Wilson 95 % |
|---|---|---|
| `search_memory` con ratio 0,35 | 36/46 | 0,644 a 0,877 |
| `search_memory` con ratio 0,20 | 37/46 | 0,668 a 0,893 |
| R@10 con piso 0,79 | 39/46 | 0,718 a 0,924 |
| R@5 con piso 0,79 | 30/46 | 0,508 a 0,773 |
| R@5 con piso 0,81 | 33/46 | 0,575 a 0,827 |

### Lo que dicen los propios archivos de resultados

Se calcularon los discordantes por caso con los archivos que ya están en `docs/eval/results/` (no hace falta correr nada):

- **Ratio del corte relativo** (`2026-10-15-real-relsweep-*-agent-view.json`, campo `search.found` por caso): 0,60 a 0,35 mejora 6 casos y empeora 0 (p = 0,031); 0,35 a 0,20 mejora 1 y empeora 0 (p = 1,0); 0,50 a 0,35 mejora 3 y 0 (p = 0,25); 0,40 a 0,35 mejora 2 y 0 (p = 0,5). El valor 0,35 se eligió por ser «el más alto del plateau de 36» (`verify.md:182`); esa meseta es indistinguible de 0,30, 0,25 y de 0,20 (37). La única diferencia que la prueba sostiene es la regresión de 0,60, que justamente es lo que el diff no vio.
- **Piso de coseno** (`2026-10-15-real-project-indexing-tip-sweep-cos*.json`, `retrieval.per_case`): 0,79 contra 0,77 en R@5 da 1 mejor y 0 peores (p = 1,0), en R@10 da 0 y 2 (p = 0,5); contra 0,81, en R@5 da 4 y 1 (p = 0,375) y en R@10 da 1 y 3 (p = 0,625). El MRR de 0,79 (0,5524) supera al de 0,81 (0,5489) por 0,0035. Ningún piso entre 0,70 y 0,81 se separa de 0,79 en ninguna métrica de recall. El comentario de `score.rs:165-168` dice que 0,79 «keeps the best MRR, nDCG@10 and recall@10»; es cierto y no significa que sea mejor: pierde en R@5 contra 0,81 por 3 casos. Se eligió entre 7 candidatos y 3 métricas sobre los mismos datos en que se reporta, sin corrección.
- **Umbral de contradicción** (`contradiction.rs:15-33`): 10 pares contradictorios y 10 compatibles. Con 0 falsos positivos de 10 compatibles, la cota superior de Wilson al 95 % es 0,278; el recall 0,8 en 0,40 son 8/10 (0,49 a 0,94) y el 0,9 en 0,45 son 9/10 (0,60 a 0,98). El margen que sostiene al umbral es un par escrito a mano a 0,469. Además la tabla de ese comentario (`contradiction.rs:35-45`) sigue siendo la de CLS y el texto que la rodea habla de mean: se lee mal.

### Lo que se reporta y lo que se eligió salen de lo mismo

- Las constantes de arriba se eligieron y se reportan sobre las mismas 46 consultas. Elegir el máximo de una grilla en los datos que luego se informan sobreestima el resultado: el número publicado (`specs/eval`, ancla R@5 0,6522, MRR 0,5524) incluye lo que el barrido aprendió del propio set.
- `DOMAIN_SET_MIN_CASES = 30` y `DOMAIN_SET_MIN_PAIRS = 20` (`report.rs:177-178`) deciden si un set es «provisional». Con 46 casos el set se declara apto para sostener una comparación, y la aritmética de arriba dice que no distingue menos de 6 casos de diferencia. La bandera dice que se puede confiar, y es la parte débil.
- El canal léxico no se pudo medir con este set: las corridas `lex*` dan lo mismo que el ancla (`verify.md:161`). Es un caso extremo del mismo problema: el set no tenía poder para ver la diferencia y nada lo marcó.

### El set no se puede reproducir desde el repo

`docs/eval/**` está ignorado. Lo muestra el propio proyecto: un test leía `docs/eval/results/2026-08-15-fake.json`, un archivo ignorado que no existía en el disco, y fallaba en todas las máquinas (`openspec/changes/archive/2026-10-12-eval-golden-expansion/proposal.md:14`). `rag-eval` registra el sha256 del golden (`report.rs:166-170`, `golden.rs:241`), lo que permite saber si dos corridas usaron el mismo archivo, pero no volver a obtener ese archivo. Los números de esta exploración sólo se pueden reproducir en la máquina donde se corrieron.

La causa es de privacidad y es legítima: los sets se minan de memorias privadas (`ed2ddff`; `golden-mine mine` lee un archivo de candidatos y arma el corpus con ellos, `golden-mine/main.rs:1-18`). Un set versionado tendría que ser **escrito a mano o sintético desde cero**, nunca minado del store real, y `synth` hoy no sirve para eso porque parte de una base privada.

## Lo que ya existe y se reutiliza

- `diff_values`/`diff_block`/`accumulate` (`diff.rs:108-154`): la unión por `query` y la lectura tolerante de bloques ausentes (`diff.rs:89-102`). Se extiende, no se reemplaza.
- `CaseResult` con las cinco métricas por caso (`report.rs:63-85`) y `CaseView` con `search.found` y `context.found` (`agent_view/mod.rs:70-84`): todo lo necesario para un diff pareado ya está en el JSON.
- `by_lang` y `by_subset` del reporte (`report.rs:116,119`): un intervalo por grupo se calcula con lo ya agrupado.
- `clawg_core::eval_api::sha256_hex` (`golden.rs:241`): una partición determinista por hash de la consulta no necesita estado ni semilla.
- El patrón de funciones puras con tests (`metrics.rs`, `diff.rs`) y `SplitMix64` escrito a mano porque `rand` no garantiza reproducibilidad entre versiones (`golden-mine/synth/rng.rs:1-4`; `design.md:37` de eval-golden-expansion). `clawg-eval` no depende de `rand` (`crates/clawg-eval/Cargo.toml:28-38`).
- `CaseSubset` con `#[serde(default)]` para que los resultados viejos sigan leyéndose (`golden.rs:85-93`).
- `CaseStatus::KnownRed` con `blocked_by` (`golden.rs:112-116,297`): el precedente de una clase de caso con reglas propias en el reporte.
- La regla de `config.yaml` de que un cambio de ranking se valida con `rag-eval --diff` contra la base: este cambio la vuelve verificable.

## Enfoques posibles

### A. Prueba pareada exacta más bootstrap, en el reporte y en `--diff`, incluido agent-view (recomendado)

Funciones puras en un módulo nuevo de `rag-eval` (`stats.rs`), sin dependencias:

- `wilson(k, n, z) -> (lo, hi)` para tasas de acierto.
- `mcnemar_exact(b, c) -> p` (binomial exacto a dos colas) para métricas binarias por caso: `found` de agent-view y el acierto en R@k.
- `paired_bootstrap_ci(deltas, resamples, seed, alpha)` para métricas continuas (MRR, nDCG@10, tokens) sobre las diferencias por caso, con un generador determinista local (el mismo SplitMix64 de `synth/rng.rs`, que habría que duplicar o mover porque `golden-mine` y `rag-eval` son binarios distintos sin crate de biblioteca compartido).
- `--diff` imprime, además de better/worse/net, el p exacto y el intervalo, y gana un bloque `agent_view` (unido por `query`) con `search.found`, `context.found` y la diferencia de tokens por caso.
- El reporte de una corrida suma el intervalo de Wilson a cada agregado, por idioma y por subconjunto.

- A favor: convierte «neto +6» en un enunciado con p; cierra el hueco que ya costó un error; puro y testeable contra valores conocidos (`wilson(36, 46)` = 0,644 a 0,877; `mcnemar_exact(0, 6)` = 0,03125; `(0, 5)` = 0,0625; `(1, 9)` = 0,0391; `(1, 8)` = 0,0703).
- En contra: más texto en el diff. Con 46 casos casi todo cambio dará «no significativo»; eso es información, no un defecto, pero hay que saber leerla. Sobre muchas métricas y muchas comparaciones hay que declarar una métrica primaria antes de correr (ver decisiones) o se repite el sesgo de elegir entre siete candidatos.

### B. Partición calibración y prueba

Un caso pertenece a calibración o a prueba según `sha256(query) mod k`: sin semilla, y agregar casos no reubica los anteriores. Los barridos se miden y eligen sobre calibración; el valor elegido se reporta una vez sobre prueba.

- A favor: quita el sesgo de elegir y reportar en lo mismo.
- En contra, medido: con 46 casos, partir a la mitad deja 23 y 23, y el intervalo de Wilson de un 0,78 con 23 casos es de unos ±0,17, más ancho que hoy. **No es útil sobre el set actual.** Sirve sobre el set expandido (112 casos, pero sus subconjuntos sintéticos salen de plantillas del mismo generador, por lo que no son independientes entre sí) o sobre el set ampliado por C y D. Recomendado en esa forma: como regla para las constantes **nuevas** de 34, 35 y 37, y como validación informativa única de las ya fijadas.

### C. Clases nuevas de caso en el esquema (recomendado, en dos clases)

- **Sin respuesta (`unanswerable`)**: sin `expected_refs`; el éxito es que ninguna memoria del corpus pase una puerta, o que la salida diga que no hay nada. Hace falta que `recall_at_k` no cuente estos casos como cero: se agregan al reporte aparte, con su propia métrica (tasa de abstención correcta), y se excluyen de R@k, MRR y nDCG. Es lo que search-abstention (35) necesita.
- **Actualización (`update`)**: dos filas con el mismo hecho, una vieja y una nueva con `created_at` posterior; la esperada es la nueva y debe quedar antes que la vieja. Es lo que mide memory-rank-fusion (34) y, más adelante, topic-revisions (27). La métrica es «la nueva antes que la vieja», pareada por caso.
- Una tercera clase, **multiagente y multiproyecto**, es el hueco que ya señaló eval-golden-expansion (`exploration.md:32` de ese cambio) y lo necesitan agent-sharing y knn-prefilter (37). Se deja como decisión abierta de tamaño.
- Mínimos propuestos: 20 casos sin respuesta (con 0 errores de 20 la cota de Wilson es 0,161; con 30, 0,114; con 46, 0,077) y 12 de actualización (el mismo número por dirección que ya usa el generador, `CONTESTED_CASES`, `synth/mod.rs:27`).
- Costo: un campo nuevo en `RetrievalCase` (que usa `deny_unknown_fields`, `golden.rs:31`, así que los archivos nuevos no los lee un `rag-eval` viejo, lo cual es aceptable dentro del mismo repo) y una clase más en el reporte. La compatibilidad hacia atrás —un golden sin la clase sigue leyéndose igual— hay que dejarla con un test.

### D. Un set pequeño y versionado, escrito a mano y sin datos privados (recomendado, con la regla de privacidad)

Un `docs/eval/` no, porque es la carpeta ignorada; una carpeta nueva del repo, con el set mínimo que ejercita las clases de C y los casos de contradicción, escrito a mano: sobre tecnologías públicas y nombres inventados, nunca minado del store ni derivado de él. `golden-mine synth` podría expandirlo sin cambios si se le permitiera partir de esa base.

- A favor: los tests y la medición corren en cualquier máquina sin autorización sobre `~/.clawg*`; el test que hoy lee un archivo ignorado deja de existir como categoría; un cambio se puede verificar sin depender del set privado.
- En contra: es un set chico (no sustituye al privado, que es el que se parece al uso real); mantener otro set cuesta; revierte una decisión anterior del usuario (`exploration.md:51` de eval-golden-expansion: «el golden set no se versiona») y por eso es decisión suya. Con 20 a 30 casos tampoco da poder estadístico: sirve para que el arnés y las clases nuevas se prueben, no para elegir constantes.

### E. `--diff` con código de salida ante una regresión significativa

Hoy `run_diff` devuelve siempre `EXIT_OK` si pudo leer los archivos (`main.rs:690-704`). Un flag opcional (`--fail-on-regression`) devolvería un código distinto de cero si alguna métrica primaria empeora con p < 0,05.

- A favor: vuelve ejecutable la regla de `config.yaml` («validado con `--diff` contra la base»).
- En contra: con 46 casos sólo se dispara ante regresiones de 6 casos o más; no ve las chicas. Por eso va como opción, nunca por defecto, y no reemplaza leer el diff.

### F. Un piso sólo léxico, y acierto estricto junto a acierto parcial

- **Piso léxico.** `--sweep-lexical 0` apaga el canal léxico y deja sólo el denso (`cli.rs:49-53`). Falta el brazo opuesto: ordenar sólo por el BM25 de FTS5, sin vectores.
  - Ese brazo dice si el embedder aporta algo más que buscar las palabras. Si el ranking completo no le gana a «sólo léxico» por una diferencia que sostenga la prueba pareada de A, el problema está en el modelo o en el puntaje, no en el set. Hoy no se puede afirmar ni una cosa ni la otra.
  - english-recall (21) es un ejemplo: una consulta en un idioma sobre una fila en otro es justo donde el léxico no llega y el denso debería llegar.
- **Estricto y parcial, lado a lado.** La decisión abierta sobre qué cuenta como acierto con varias esperadas (`recall_k == 1,0` o `> 0`) se resuelve informando las dos, siempre juntas. Una métrica que se conforma con una de N sobreestima los casos `multi_ref` del set expandido.
- **Un caso que falla con error cuenta como fallo.** Queda para el diseño verificar cómo trata hoy el arnés un caso cuya búsqueda devuelve error. Un caso que se cae del denominador sube la métrica sin que nada lo diga.
- A favor: es barato, porque el canal FTS ya existe (`search.rs:563-571`), y le da una referencia a cualquier número absoluto.
- En contra: es un brazo más que mantener.

**Recomendación:** A (incluyendo agent-view), C y F como núcleo; E como flag opcional encima de A; B en la forma acotada de arriba; D si el usuario acepta revertir la decisión de no versionar, con la regla de privacidad como condición.

## Descartado

| Opción | Razón técnica |
|---|---|
| Sólo ampliar el set (más casos, sin estadística) | Más casos acercan el intervalo pero no dicen cuándo una diferencia alcanza. Se necesitan las dos cosas, y A funciona con cualquier tamaño. |
| Partir el set de 46 a la mitad ya | Ver B: 23 casos dan un intervalo más ancho que el actual. |
| Una dependencia de estadística para los intervalos | Los cálculos son una binomial exacta, una fórmula cerrada y un remuestreo; entran en funciones puras. Sin una dependencia nueva, como pide `config.yaml`. |
| Una prueba t sobre las métricas continuas | MRR y nDCG por caso están acotados y apelmazados en 0 y 1; un bootstrap pareado no asume normalidad. (Un Wilcoxon de rangos con signo sería equivalente en uso; el bootstrap da además el intervalo.) |
| Corregir por comparaciones múltiples (Bonferroni) de todo | Con 46 casos nada pasaría el umbral. Se resuelve declarando una métrica primaria antes de correr, no corrigiendo después. |
| Minar el store real para el set versionado | Rompe la razón por la que hoy no se versiona (`ed2ddff`). |
| Medir abstención como una nueva métrica dentro de R@k | Un caso sin respuesta no tiene esperada; en R@k cuenta cero y castiga el agregado (`metrics.rs:31-36`). |

## Interacciones

- **english-recall (21).** El problema que explora (R@5 en inglés de 0,68 a 0,45) son 5 casos de 22 en ese subconjunto: con p exacto de 5/0 = 0,0625 no cruza el umbral, y es el tipo de afirmación que este cambio vuelve explícita. Conviene que 21 corra después de A o que reporte el intervalo de Wilson por idioma.
- **memory-rank-fusion (34).** Cambia la escala del score; su validación necesita el `--diff` con p y los casos `update` de C.
- **search-abstention (35).** Depende de este cambio: sin casos `unanswerable` no hay con qué calibrar una puerta, y sin partición no hay con qué reportarla.
- **knn-prefilter (37).** Mide recall y latencia con más filas; necesita el set expandido o uno multiagente y multiproyecto (C) y la comparación pareada para decir que el prefiltro no pierde casos.
- **docs-search-quality (20).** `rag-eval --docs` imprime y no escribe archivo (`specs/eval`, sección `--docs`), así que no hay `--diff` de docs. Con 11 fallas sobre un Hit@3 de 0,27 el problema de ruido es el mismo; extender A al modo docs queda como decisión abierta.
- **conflict-candidates (24).** Su medición previa ya pide intervalos de Wilson por clase (`exploration.md` de 24, «Cómo se mide»); comparte la función `wilson`. Mejor que la escriba este cambio una sola vez.
- **context-budget (23).** Su resultado esperado se expresa sobre `found` y tokens de agent-view; con el bloque nuevo de `--diff` se compara por caso en lugar de a mano.
- **quality-guards (32).** El módulo nuevo no puede traer código muerto: cada función pura se usa desde el reporte o el diff, y su regla de `#[allow(dead_code)]` aplica.
- **install-smoke (40).** Una compuerta que corra sola, en CI, necesita un set que se pueda leer en esa máquina. El set privado está ignorado (`.gitignore:31-32`), así que sólo sirve el set versionado y escrito a mano de D. Si 40 agrega CI, lo que correría ahí es `--fail-on-regression` (E) sobre ese set. Con el set privado, la compuerta sigue siendo local, antes de archivar cada cambio de ranking.

## Alcance candidato

1. Módulo `stats.rs` de `rag-eval` con `wilson`, `mcnemar_exact` y `paired_bootstrap_ci`, puras y con tests contra valores conocidos.
2. `--diff` con p exacto e intervalo por métrica, y un bloque `agent_view` unido por `query` (`search.found`, `context.found`, tokens). Un archivo sin el bloque sigue leyéndose, como hoy con `similarity`.
3. Intervalo de Wilson en los agregados del reporte (global, por idioma, por subconjunto) y en el contexto de `provisional`: la nota debe decir cuántos casos de diferencia hacen falta para una conclusión.
4. Clases `unanswerable` y `update` en `RetrievalCase`, con su validación (una `unanswerable` no tiene esperadas; una `update` tiene dos filas con fechas distintas y una esperada), su métrica y su bloque en el reporte, sin tocar R@k de los casos naturales.
5. Regla de partición por hash de la consulta (B) y un flag de los barridos que mide sobre calibración y reporta sobre prueba, cuando hay casos suficientes.
6. `--fail-on-regression` opcional (E).
7. Si se acepta D, un set mínimo versionado y escrito a mano, y su regla de privacidad en `docs/`.
8. Actualizar `CLAUDE.md` del repo (comando `--diff` y flags nuevos), `specs/eval` y retirar del backlog de `openspec/README.md` el punto de `--diff` y agent-view.
9. El brazo sólo léxico como piso, y R@k estricto y parcial en el reporte y en `--diff` (F).

### No-alcance

- Cambiar `RELEVANCE_RATIO`, el piso de coseno, el umbral de contradicción o cualquier ranking. Este cambio **mide**; si la medición dice que una constante no se sostiene, se abre un cambio aparte.
- Ampliar o rehacer el set privado ni correr nada sobre `~/.clawg*` sin autorización.
- La medición de latencia (ya existe en agent-view) y sus intervalos.
- Un reporte HTML o gráficos.
- Pruebas de significancia para `rag-eval --docs` (decisión abierta).

## Decisiones abiertas

Son del usuario.

- [ ] **Prueba para métricas binarias.** McNemar exacto (recomendado) o prueba del signo. Son equivalentes cuando sólo cuentan los discordantes; ¿alcanza con nombrarla una vez?
- [ ] **Intervalo para métricas continuas y remuestreos.** Bootstrap pareado percentil (recomendado) o por sesgo corregido; ¿cuántos remuestreos y qué semilla fija (propuesta: 10.000 y una constante del código)?
- [ ] **Nivel y métrica primaria.** ¿95 % y una métrica primaria declarada antes de cada comparación (propuesta: R@5 y `found`), con el resto informativo?
- [ ] **Qué cuenta como acierto binario en R@k** cuando un caso tiene varias esperadas (12 de 112 casos del expandido): `recall_k == 1,0` o `> 0`.
- [ ] **Proporción de la partición.** ¿Mitad y mitad, o 70/30 con k pliegues? Sobre 46 casos no se recomienda ninguna; sobre el expandido, 70/30 por hash.
- [ ] **Dónde vive un set versionado y cuál es la regla.** Carpeta, formato, y la regla escrita («escrito a mano o sintético desde cero; nunca derivado del store ni de memorias reales»). ¿Se revierte la decisión de no versionar?
- [ ] **Mínimos por clase** (propuesta: 20 sin respuesta, 12 de actualización, y para multiagente y multiproyecto un número a definir).
- [ ] **¿`--diff` falla la corrida?** Opcional con flag (recomendado), por defecto, o nunca. Y con qué métrica primaria.
- [ ] **Dónde se mide la abstención:** en el ranking del core (puntajes por consulta) o en agent-view (lo que recibe el agente). Se inclina a agent-view para la decisión y al core para calibrar; lo cierra 35.
- [ ] **¿`--docs` entra?** Extender A al modo de documentación o dejarlo para docs-search-quality.
- [ ] **El generador de remuestreo** duplicado en `rag-eval` o movido a un lugar compartido (hoy no hay crate de biblioteca en `clawg-eval`).

## Cómo se mide

Tests RED primero (cada uno falla hoy porque el código no existe):

1. `wilson(36, 46)` da 0,644 a 0,877 (tolerancia de 0,001); `wilson(0, 10)` da un límite superior de 0,278; `n = 0` no divide por cero.
2. `mcnemar_exact(0, 6)` = 0,03125; `(0, 5)` = 0,0625; `(1, 9)` = 0,0391; `(1, 8)` = 0,0703; `(b, c)` simétrico; `(0, 0)` = 1,0.
3. `paired_bootstrap_ci` es determinista con la misma semilla, da `[0, 0]` para diferencias nulas y un intervalo que no incluye 0 para diferencias todas positivas.
4. `--diff` de un archivo contra sí mismo: p = 1,0 en todas las métricas, 0/0/0 como hoy (`self_diff_is_a_no_op`, `diff.rs:219`), y el bloque `agent_view` también.
5. `--diff` entre dos archivos de agent-view con `found` distinto en un caso: hoy da 0/0/0; con el cambio informa mejor/peor y el p.
6. Un archivo sin `agent_view` (resultados anteriores) sigue leyéndose y se compara con uno que sí lo tiene sin error.
7. Con los archivos reales de `relsweep` (`0,60` y `0,35`): 6 mejores, 0 peores, p = 0,031; `0,35` contra `0,20`: 1 y 0, p = 1,0. Si los archivos no están (son ignorados), el test usa fixtures sintéticos con los mismos conteos.
8. Un caso `unanswerable` no aparece en R@k, MRR ni nDCG; con ninguna memoria sobre la puerta cuenta como abstención correcta.
9. Un caso `update` mide «la nueva antes que la vieja» y falla si las fechas están al revés.
10. Un golden sin clases nuevas sigue produciendo el ancla exacta (R@5 0,6522, MRR 0,5524) y el mismo sha256.
11. La partición por hash es estable: agregar casos no mueve a los existentes de lado.
12. `--fail-on-regression` devuelve distinto de cero con 0/6 y cero con 0/5.
13. El brazo sólo léxico ordena por BM25 sin consultar vectores, y su reporte se puede comparar con `--diff` contra el ancla.
14. Un caso con dos esperadas y una sola recuperada en el top 5 cuenta 0 como acierto estricto (todas) y 1 como acierto parcial (al menos una), y su R@5 fraccional sigue siendo 0,5.

`rag-eval`: no cambia ningún ranking. Se corre `--real` sobre `golden.json` y el resultado de las métricas **no debe cambiar** (diff vacío con el ancla `2026-10-15-real-project-indexing-tip-anchor.json`), y se verifica que los intervalos nuevos aparecen. El pre-gate aquí es la aritmética de arriba, ya medida con los archivos existentes.

## Riesgos

- **Falsa seguridad inversa.** Con 46 casos casi todo será «no significativo» y puede leerse como «no hay diferencia». El texto del diff tiene que decir «no distinguible con este número de casos» y nunca «sin diferencia».
- **Más comparaciones, más falsos positivos.** Cada métrica y cada subconjunto suma una prueba. La métrica primaria declarada antes es lo que lo evita; el resto se marca como informativo.
- **Un set versionado que se filtre.** Si alguien arma la versión pública copiando filas del store, se expone memoria privada. La regla de privacidad tiene que estar escrita donde está el set y verificarse en la revisión.
- **Cambios en el esquema del golden.** Un campo nuevo con `deny_unknown_fields` hace que un `rag-eval` viejo rechace un archivo nuevo. Dentro del repo el binario y el set se mueven juntos; el test 10 cubre lo opuesto.
- **Dos sets que se desvían.** El versionado y el privado pueden medir cosas distintas y dar conclusiones distintas; el reporte registra cuál se usó (el sha256 ya lo hace).
- **Estado del repo:** `CLAUDE.md` y `specs/eval` describen `--diff` y el modo agent-view; cualquier flag nuevo obliga a actualizarlos.
