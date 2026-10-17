# memory-rank-fusion — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | eval-rigor (33), archivado | ver [`state.yaml`](state.yaml) |

## Problema

En `search_memory`, una fila que tiene el identificador exacto de la consulta lleva un bono fijo de 1,0 sobre su coseno reescalado (`crates/clawg-core/src/memory/score.rs:273`), y el corte relativo deja sólo las filas con `score >= 0,35 × mejor` (`relevant_len`, `crates/clawg-cli/src/relevance.rs:25-34`). La fila con el bono fija ese mejor score, así que una fila sin bono necesita un coseno de 0,8635 a 0,918 para sobrevivir ([exploración](exploration.md#cómo-se-combinan)), y en el ancla con mean el percentil 95 de los cosenos es 0,830. Medido en el set de la semilla 7 (piso 0,8793): las 30 consultas `lexical` muestran exactamente 2 filas, contra 9 a 10 en los demás subconjuntos; la segunda está sólo por `MIN_SHOWN_HITS`.

El corte corre en dos lugares con la misma regla: el daemon, si el pedido trae `min_relative_score` (`crates/clawg-cli/src/daemon/api.rs:911-914`), antes de registrar accesos (`:915`), y el puente siempre (`relevant_hits`, `crates/clawg-cli/src/mcp/tools.rs:60-64`). La marca de coincidencia exacta existe en el núcleo (`RawHit.lexical_match`, `score.rs:28-30`) pero no llega a `RankedHit` (`crates/clawg-core/src/memory/search.rs:76-92`), así que ninguno de los dos cortes puede distinguirla.

Ninguna medida actual lo ve: `found` de agent-view acepta una sola fila esperada (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:181`), y ningún caso del golden espera a la vez la fila con el identificador y otra del mismo tema sin él.

## Decisiones

Tomadas sobre las preguntas abiertas de la exploración:

- **Alcance: sólo el enfoque A.** El corte se fija contra la mejor fila **sin** bono; el orden y los scores no cambian. La fusión por rango (B) es un cambio aparte, que se vuelve a evaluar con A medido y con el filtro de palabras vacías de docs-search-quality (20).
- **Las filas con bono se muestran todas.** Ya las acotan el presupuesto de identificadores (20; si más filas lo tienen, ninguna lleva el bono, `search.rs:800-802`) y el `limit` del pedido.
- **`MIN_SHOWN_HITS` cuenta sólo las filas sin bono.** Una consulta con identificador muestra sus coincidencias exactas y además, como mínimo, las dos mejores filas sin bono. Conserva la razón del mínimo (con scores bajos, la segunda puede ser la respuesta, `relevance.rs:56-62`) para la parte semántica, que es la que el corte mide.
- **Campo `exact` en la API HTTP.** Booleano en cada hit de `POST /v1/search`, siempre presente en la respuesta de un daemon nuevo. El cliente lo lee con `#[serde(default)]`, como `agent_id` y `piece` (`crates/clawg-cli/src/client.rs:162-169`): sin el campo, un hit vale `exact = false` y el corte es el de hoy.
- **Los dos cortes siguen existiendo** y usan la misma función. Unificarlos cambiaría el contrato de `POST /v1/search` y el orden de `record_hits` (descartado en la exploración).
- **El texto que ve el modelo no cambia.** Las líneas de `search_memory` no marcan la coincidencia exacta; `agent_contract.json` queda igual.
- **Umbrales:** los de la exploración, fijados ahora y sin moverse después de medir (abajo).

## Alcance

1. **Núcleo:** `RankedHit.exact`, copiado de `RawHit.lexical_match` en los dos lugares que arman hits (`search.rs:525`, `:1060`). No toca scores, orden, `collapse_saves` ni el canal léxico.
2. **Corte:** `relevant_len` pasa a decidir sobre pares `(score, exact)`; el daemon y el puente lo usan igual. El corte deja de ser un prefijo de la lista: una fila con bono por debajo de una fila cortada se sigue mostrando, en su lugar.
3. **Cliente:** `Hit.exact` con `#[serde(default)]`.
4. **`rag-eval`:** la medida estricta «todas las esperadas se muestran» (`all_hit_rate`) en cada resumen de agent-view.
5. **`golden-mine synth`:** un subconjunto `lexical_related` de 30 casos, cada uno con la fila del identificador y tres filas del mismo tema sin él como esperadas, más cuatro hermanas con identificadores vecinos. `rag-eval` acepta el subconjunto. El generador lo agrega al final, así que las filas y los casos de los demás subconjuntos no cambian para una misma semilla. `synth` acepta `query_lang` en la base, que hoy rechaza (`crates/clawg-eval/src/bin/golden-mine/synth/golden.rs:28-45` no lo declara y niega claves desconocidas), porque si no no puede leer el `golden.json` etiquetado por english-recall.
6. **`.claude/CLAUDE.md`:** el campo `exact` en la línea de `POST /v1/search`.

## Fuera de alcance

- La fusión por rango (B), el presupuesto de tokens (C) y escalar el bono (D).
- Cambiar el bono, `RELEVANCE_RATIO`, `MIN_SHOWN_HITS`, el piso de coseno o el rango de importancia.
- Filtrar palabras vacías del canal léxico (20), el prefijo del embedder o el modelo (21, 17).
- `recall_context` (no tiene corte, `prompt_builder.rs:179`) y el detector de contradicciones (`lexical: None`).
- Mostrar la marca en el texto de `search_memory`.
- Un `--diff` sobre `all_hit_rate`: se compara a mano entre dos archivos.

No toca el esquema, identificadores guardados ni el contrato de embedding: no hay migración. Volver atrás es revertir los commits; un puente viejo ignora `exact`, y un daemon viejo no lo manda.

## Cómo se mide

### Tests que fallan hoy

1. `relevant_len` con marca, puro: una fila con bono en 1,6 y filas sin bono en 0,30 / 0,25 / 0,20 / 0,05. Hoy deja 2; con A deja la del bono y las tres que superan `0,35 × 0,30`.
2. Sin ninguna marca, el corte da lo mismo que hoy en todos los casos de `relevance.rs`.
3. Un `Hit` sin `exact` se deserializa (daemon viejo), y `POST /v1/search` sin `min_relative_score` responde igual que hoy salvo el campo nuevo.
4. **El fixture de la tesis** (`crates/clawg-cli/tests/api.rs`): una fila con el identificador y tres filas sin él que pasan el corte frente a la mejor sin bono pero no frente a la del bono. Hoy el daemon devuelve dos; con A, las cuatro. Se arma con un embedder de prueba que fija los vectores (los cosenos los decide el test, no el modelo).
5. Daemon y puente coinciden sobre la lista mostrada.
6. Una fila de otro agente con el identificador llega con `exact` y se sigue mostrando `by <autor>`.
7. `record_hits` registra exactamente las filas que ahora se muestran.

### Medición con el modelo real

Necesita el modelo en el caché del perfil y el golden set fuera de git (`docs/eval/golden.json` y `docs/eval/golden-expanded.json`, que se vuelve a generar con `golden-mine synth --seed 7` sobre el `golden.json` actual para sumar `lexical_related`). `rag-eval` siembra un store temporal y nunca abre un store real.

1. **Base**, en el commit anterior al cambio del corte (con el subconjunto ya generado): `rag-eval --real --agent-view --golden docs/eval/golden-expanded.json`, y `rag-eval --real` y `--agent-view` sobre el ancla.
2. **Después**, las mismas tres corridas.

| # | Métrica | Base | Umbral |
|---|---|---|---|
| U1 | `lexical_related`: `search.all_hit_rate` | a medir; se espera cerca de 0 | `>= 0,90` |
| U2 | `lexical`: la fila con el identificador sigue primera (`rank = 1`) | 30 de 30 | 30 de 30 |
| U3 | `lexical`: tokens medios de `search_memory` | a medir | no más que los de `natural` en la misma corrida |
| U4 | `natural`, `multi_ref`, `high_importance`, `low_importance`, `long`: salida de `search_memory` | — | idéntica caso por caso |
| U5 | Ancla, `--diff` | R@1 0,4348, R@5 0,6522, R@10 0,8478, MRR 0,5524, nDCG@10 0,6215 | ningún caso se mueve |
| U6 | Agent-view del ancla, `--diff` | `search_memory` 36/46 a 764,0 tokens; `recall_context` 39/46 a 1.086,4 | ningún caso se mueve |

U4 y U6 se esperan idénticos porque esos casos no traen identificadores; si alguno se mueve, se busca la causa antes de aprobar. Un umbral que falla se anota tal cual en `verify.md`.

## Riesgos

- **Más tokens en las consultas con identificador:** es el costo de no esconder; U3 lo acota.
- **Un puente viejo con un daemon nuevo** vuelve a cortar con la regla de hoy y muestra lo mismo que hoy: no empeora nada.
- **Fixtures sustitutos:** `lexical_related` es sintético; que sus filas relacionadas lleguen a un coseno cercano a 0,88 se comprueba en la base, y si no llegan, U1 no mide la tesis y se anota.
