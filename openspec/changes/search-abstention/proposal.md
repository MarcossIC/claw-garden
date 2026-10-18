# search-abstention — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | eval-rigor (33) y memory-rank-fusion (34), archivados | ver [`state.yaml`](state.yaml) |

## Problema

La búsqueda de memorias no tiene una salida para «no hay nada relevante» ([exploración](exploration.md)):

- `search_memory` muestra siempre las dos mejores filas sin coincidencia exacta, sea cual sea su score: `MIN_SHOWN_HITS = 2` (`crates/clawg-cli/src/relevance.rs:22`) y `shown_hits` cuenta cualquier fila inexacta contra ese mínimo antes de mirar el score (`relevance.rs:31-43`). Una fila con score 0, es decir con coseno bajo el piso y sin bono, se muestra si es una de esas dos.
- `recall_context` llena hasta `K_FINAL = 10` con lo que haya: `build_memory_prompt` saca las filas del workspace y corta (`crates/clawg-core/src/memory/prompt_builder.rs:216-217`) sin mirar el score. Un test fija a propósito que una fila de score 0 llena el prompt (`recall_does_not_count_zero_score_hits_as_accesses`, `prompt_builder.rs:2290-2324`); lo único que se evita es contarla como acceso (`crates/clawg-core/src/memory/access.rs:147-155`).
- Los conflictos se piden sobre lo mostrado (`prompt_builder.rs:240-255`, `fetch_active_conflicts` en `:667`), así que una fila de relleno puede traer un par escalado ajeno a la pregunta.
- La única frase de vacío es `no memories found` (`crates/clawg-cli/src/mcp/tools.rs:82-86`), que hoy sólo aparece si el alcance no tiene filas. El agente no puede distinguir «no hay memorias» de «hay, pero ninguna se acerca».

memory-rank-fusion (34) ya fijó qué hits deja el corte sin cambiar los scores: el mínimo cuenta sólo filas sin bono, y una fila exacta se muestra siempre (`relevance.rs:24-43`). eval-rigor (33) dejó la clase `unanswerable` en el golden y la métrica de abstención de agent-view (`abstained` cuando la búsqueda no imprimió ninguna línea, `crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:199`).

## Decisiones

Tomadas sobre las preguntas abiertas de la exploración:

- **Alcance: sólo el enfoque C.** Dejar de rellenar con filas de score 0 y decirlo, sin ninguna constante nueva. La puerta calibrada (A) queda para un cambio aparte: necesita casos `unanswerable` en el golden fuera de git y scores crudos con el modelo real para calibrarse en la partición `calibration` y reportarse en `test`, y no hay ningún valor medido que justifique una constante hoy.
- **El cero lo decide el piso de coseno.** Una fila sin bono con score 0 tiene el coseno bajo 0,79 (`crates/clawg-core/src/memory/score.rs:215-217`, `:264-277`); no se agrega ningún umbral.
- **`MIN_SHOWN_HITS` cuenta sólo filas con score mayor que 0.** El mínimo conserva su razón (una respuesta con 0,04 bajo un mejor de 0,10, `relevance.rs:73-79`), que es de filas que puntuaron.
- **El daemon dice cuánto ocultó.** `POST /v1/search` responde `withheld`: los hits que el corte dejó afuera (0 sin `min_relative_score`). `POST /v1/context` responde, en `retrieval` con `state: "searched"`, `hits` (los hits con los que se armó el prompt, antes del presupuesto) y `withheld` (los de score 0 que dejó afuera). Con eso el puente distingue el alcance vacío de la falta de relevancia sin repetir la búsqueda.
- **Frase de abstención, en inglés como el resto de las herramientas:** `no relevant memories for this query: N searched, none close enough to show`. `search_memory` la dice cuando no queda ninguna línea y el daemon ocultó alguna; `recall_context` la pone delante del contexto cuando no quedó ningún hit y se ocultó alguno, seguida de que sólo se muestra el workspace o de que está vacío. Es distinta de `no memories found` (alcance vacío), de `no context yet` (prompt vacío sin búsqueda que ocultara algo) y de `Memories were not searched: …` (context-budget): con el embedder no listo o la búsqueda fallida no se dice que nada sea relevante.
- **Sin sugerencia en la frase.** El servidor informa; qué hacer lo dice el protocolo, que ya pide guardar lo aprendido.
- **Cliente viejo y daemon viejo.** Un daemon anterior no manda `withheld` ni `hits`: el puente los lee con `#[serde(default)]` y dice lo de hoy. Un puente anterior ignora los campos nuevos y, con un daemon nuevo, ve una lista sin las filas de score 0, o vacía con `no memories found`.
- **El workspace no se toca.** No depende de la consulta.

## Alcance

1. **Corte:** `shown_hits` no muestra una fila inexacta con score 0 ni la cuenta para el mínimo. Lo usan el daemon y el puente.
2. **Daemon:** `withheld` en `SearchBody`; `hits` y `withheld` en `RetrievalState::Searched`.
3. **Núcleo:** `build_memory_prompt` saca los hits de score 0 antes de cortar en `K_FINAL` y devuelve cuántos hits usó y cuántos sacó en `MemoryPrompt`. Una fila sacada no entra en `shown`, así que no trae conflictos.
4. **Puente:** `DaemonClient::search` devuelve los hits y `withheld`; `format_search` y `format_recall` dicen la frase.
5. **`rag-eval --agent-view`:** el parser lee la frase como cero líneas, igual que `no memories found`, así que un caso `unanswerable` cuenta como abstención cuando la búsqueda la dice.
6. **`.claude/CLAUDE.md`** (API HTTP y herramientas) y el mapa de `openspec/README.md`.

## Fuera de alcance

- La puerta calibrada sobre casos sin respuesta (A) y el criterio relativo por consulta (B).
- Cambiar el piso de coseno, `RELEVANCE_RATIO`, el bono o el ranking. El core sigue devolviendo la lista completa a `rag-eval`.
- `search_project` y el workspace.
- Cambiar el texto de las descripciones de las herramientas o el protocolo: `agent_contract.json` queda igual.

No toca el esquema, identificadores guardados, estados ni el contrato de embedding: no hay migración. Volver atrás es revertir los commits.

## Cómo se mide

### Tests que fallan hoy

1. `shown_hits` con `[0.10, 0.0, 0.0]` deja 1 (hoy 2), con `[0.0, 0.0]` deja 0, y una fila exacta sigue mostrándose delante de ceros.
2. `build_memory_prompt` con una fila igual a la consulta y otra ortogonal: la ortogonal no entra (hoy sí); con sólo filas ortogonales, el prompt no tiene hits y `MemoryPrompt` dice cuántas sacó.
3. Una fila de score 0 que es lado de un par escalado no trae el bloque ACTIVE CONFLICTS.
4. `POST /v1/search` con `min_relative_score` sobre filas todas ortogonales devuelve `hits: []` y `withheld` igual a las filas; sin el campo, devuelve todas y `withheld: 0`.
5. `search_memory` sin nada relevante dice la frase; con el alcance vacío, `no memories found`.
6. `recall_context` sin hits y con filas ocultas dice la frase delante del workspace; con el embedder no listo dice `Memories were not searched`, no la frase.
7. Un daemon anterior (sin `withheld` ni `hits`) da la salida de hoy en las dos herramientas.
8. El parser de agent-view lee la frase como cero líneas.

### Medición con el modelo real

Necesita el modelo en el caché del perfil y `docs/eval/golden.json` (fuera de git). `rag-eval` siembra un store temporal y nunca abre un store real.

| # | Métrica | Base | Umbral |
|---|---|---|---|
| U1 | `rag-eval --real`, `--diff` contra la base | R@1 0,4348, R@5 0,6522, R@10 0,8478, MRR 0,5524, nDCG@10 0,6215 | ningún caso se mueve (el core no cambia) |
| U2 | `rag-eval --real --agent-view`, `search.found` | 36/46 | ningún caso empeora |
| U3 | ídem, `context.found` | 39/46 | ningún caso empeora |
| U4 | ídem, tokens medios de `search_memory` y `recall_context` | 764,0 y 1.086,4 | no aumentan |

La base es el commit anterior al cambio, corrido en la misma máquina. Se espera que casi nada se mueva: en la base de agent-view hay una línea con score 0 entre 345 ([exploración](exploration.md#qué-costaría-una-puerta-absoluta-con-los-datos-que-hay)). Si el golden trae casos `unanswerable`, se reporta `search.abstain` del `--diff` sin umbral: este cambio no promete abstener, sólo decirlo cuando ocurre. Un umbral que falla se anota tal cual en `verify.md`.

## Riesgos

- **Falsa tranquilidad.** El piso es el percentil 5 de los cosenos consulta-pasaje: casi todo texto sin relación puntúa más que 0 y sigue mostrándose. Este cambio quita el relleno literal y da la frase; la abstención real es la puerta calibrada.
- **Un candidato léxico bajo el piso.** El canal FTS5 suma filas que el KNN no trajo y las puntúa con el mismo coseno (`crates/clawg-core/src/memory/search.rs:1007-1083`); si su coseno queda bajo el piso y no tiene el identificador exacto, su score es 0 y deja de mostrarse. Hoy sólo se mostraba si era una de las dos primeras inexactas. U2 y U3 lo miden.
- **Mezclar estados.** Cuatro textos distintos para cuatro situaciones; los tests de 5 y 6 fijan cuál sale en cada una.
