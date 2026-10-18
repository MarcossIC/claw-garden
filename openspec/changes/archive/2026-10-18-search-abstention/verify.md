# search-abstention — Verificación

## Linux

Sobre `973d84b`, el último commit de código de la rama (los que siguen sólo tocan documentación), build debug en `target`:

- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- `cargo test --workspace --no-fail-fast`: pasa entero.
- `agent_contract.json` no cambia: `the_agent_contract_matches_its_file` pasa sin `CLAWG_BLESS`.
- Un puente anterior, cuya variante `searched` no tiene campos, lee la respuesta nueva `{"state":"searched","hits":0,"withheld":4}` como `searched`: comprobado con un test descartable, fuera de la rama.

Cada requisito tiene sus tests en la [spec](spec.md); todos fallaban antes de su commit, por la aserción o por no compilar. Siete tests existentes contaban con filas de score 0 para tener hits bajo el embedder falso, cuyos vectores sólo coinciden con el mismo texto; sus fixtures se corrigieron sin cambiar lo que miden ([tareas](tasks.md), 1 y 2).

## CI (2026-10-18)

El CI corre clippy y la suite en Linux y en Windows. Pasó en el PR #41, sobre `324e95c` (corrida 39155881616: `test (ubuntu-latest)` y `test (windows-latest)`), y en el push del merge a `main`, `2d9997f` (corrida 39156259413, los mismos dos trabajos).

## Modelo real (2026-10-18)

Con el modelo cacheado en el perfil dev (`rag-eval --real --check-only` sale con 0) y `docs/eval/golden.json` (sha256 `363d3e08…a1fb`, n = 46). Los resultados están en `docs/eval/results/`, fuera de git. Se midió en Windows la rama `feat/release-and-self-update` en `1307019`, que ya tiene el cambio mergeado (`2026-10-18-real-search-abstention.json` y `…-agent-view.json`), contra las corridas del mismo día y la misma máquina en `78b62ea`, que no lo tienen (`2026-10-18-real-topic-revisions-final.json` y `…-final-agent-view.json`).

En el código que corre `rag-eval` (la búsqueda, el prompt, el formato de las herramientas del puente, el daemon y el agent-view) las dos puntas difieren sólo en este cambio. Los arreglos de topic-revisions están en las dos, y los commits de release-and-self-update en esos archivos sólo cambian la versión que informan `/health` y `serverInfo` (`build_info::version()`).

| # | Métrica | Sin el cambio | Con el cambio | Umbral | Resultado |
|---|---|---|---|---|---|
| U1 | ancla: R@1, R@5, R@10, MRR, nDCG@10 | 0,4348, 0,6522, 0,8478, 0,5524, 0,6215 | iguales, ningún caso movido | ningún caso se mueve | pasa |
| U2 | `search.found` | 36/46 | 36/46 | ningún caso empeora | pasa |
| U3 | `context.found` | 39/46 | 37/46 | ningún caso empeora | **falla** (c09, c10) |
| U4 | tokens medios de `search_memory` y `recall_context` | 764,0 y 1.083,7 | 761,4 y 987,7 | no aumentan | pasa |

`rag-eval --diff` no mueve ningún caso del ancla ni de similarity (p = 1). En agent-view, `search.found` no mueve ninguno, `context.found` empeora en 2 (p = 0,5), `search.tokens` mejora en 1 (c09, que dejó de mostrar su línea de score 0 de 123 tokens) y `context.tokens` mejora en 7 (p = 0,0156; −95,9 de media, intervalo [−178,3, −29,0]). Las dos métricas primarias quedan «not distinguishable», con 0 casos movidos. El golden set no tiene casos `unanswerable` (los 46 son `active`), así que no hay `search.abstain` que reportar.

**U3.** Los dos casos que se pierden son c09 (`¿Qué se entregó en la etapa donde definimos los contratos de dominio y las políticas de reintento?`, es>en) y c10 (`What was delivered in the very first milestone, when the app still had no real behaviour?`, en>en). En los dos, `search_memory` no encontraba la fila esperada ni antes ni después. `recall_context` la mostraba porque el prompt se rellenaba hasta `K_FINAL` con filas de score 0, y entre ellas `hit_cmp` desempata por id ascendente. Las listas del ancla son `c48, c02, c04, c05, c06, c09, c12, c13, c14, c15` y `c27, c32, c39, c02, c05, c06, c09, c10, c12, c15`: pasados los primeros puestos, los ids van en orden. c09 y c10 entraban por tener ids bajos, no por parecerse a la consulta, y que dejaran de salir lo confirma: con el ranking sin cambios, el prompt sólo saca filas de score 0. Es el relleno que este cambio quita. El umbral se anota fallido, como pide la [propuesta](proposal.md#medición-con-el-modelo-real).

## Lo que encontró la verificación

El `recall@10` del ancla cuenta c09 y c10 como encontrados, en las posiciones 6 y 8, por la misma razón: las métricas de ranking miden la lista entera, y en ella las filas de score 0 llegan por su id. Ninguna herramienta muestra esas filas. Queda en «Fuera de esta spec» de [`specs/retrieval`](../../../specs/retrieval/spec.md), para la puerta calibrada o para una revisión del eval.

## Diferencias con la propuesta y la spec

- **La base de la medición.** La propuesta la ponía en el commit anterior al cambio, `9a0ec3f`; se usó `78b62ea`, que tiene el mismo código de recuperación más los arreglos de topic-revisions, que también están en la punta medida. Sobre `4991ce7`, con el código de `9a0ec3f`, el ancla y el agent-view no se habían movido respecto de `78b62ea` ([verify de topic-revisions](../2026-10-18-topic-revisions/verify.md#modelo-real-2026-10-18)).
- **R2.** El escenario hablaba de tres filas ortogonales; `search_withholds_hits_that_did_not_score` siembra cuatro. La spec consolidada dice cuatro.
- **R5.** No decía qué es `N` en `search_memory`: son los hits que devolvió el daemon más su `withheld`, porque el puente también corta (`format_search`). La spec consolidada lo dice.

## Al archivar

- La spec quedó en [`specs/retrieval`](../../../specs/retrieval/spec.md): las definiciones de fila de score 0 y de frase de abstención, el corte relativo, `withheld` en `POST /v1/search`, el cliente y `search_memory`, el parser de agent-view, la medición, los invariantes y lo que queda fuera. También en [`specs/mcp`](../../../specs/mcp/spec.md), en «`recall_context` dice si buscó memorias». El apartado «`recall_context` no cuenta los ceros» se llama ahora «`recall_context` deja afuera los ceros», porque los ceros ya no aparecen en el prompt.
- Las citas `archivo:línea` de todas las specs a los archivos que el cambio tocó se recalcularon contra el código de hoy. 137, en 13 specs, se movieron sin cambiar a qué apuntan (las corrieron este cambio, topic-revisions y release-and-self-update), y 139 seguían bien. 6 apuntaban a código reescrito y se corrigieron a mano: 5 en las secciones de este cambio, y la del autoarranque en [`specs/install`](../../../specs/install/spec.md), que ahora cita `spawn_serve_at` sin puerto.
- La versión sube a 0.20.0, que cubre también el bump que topic-revisions no hizo al archivarse.

## Veredicto

Verificado en Linux, en el CI de Linux y Windows, y con el modelo real. U1, U2 y U4 pasan. U3 falla por los dos casos que sólo llegaban al prompt como relleno de score 0, que es lo que el cambio quita. Se archivó el 2026-10-18.
