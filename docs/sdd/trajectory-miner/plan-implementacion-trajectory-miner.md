# Plan de Implementación — Trajectory Miner (Aprendizaje entre Sesiones)

**Área:** Backend Rust — módulo `src-tauri/src/memory/`
**Base:** `docs/8-trajectory-miner.md` (spec de la feature)
**Prerequisito:** plan de memoria 3 capas COMPLETO (`docs/plan-implementacion-memoria-3-capas.md`) — en particular su Fase 2 (escritor episódico)
**Metodología:** SDD — cada fase es un *change* independiente con su ciclo completo

---

## 1. Contexto y veredicto

El doc 8 cierra el loop de aprendizaje entre sesiones: analizar trayectorias al cierre de cada sesión, detectar patrones exitosos recurrentes ("Golden Trajectories", `success_score > 0.85` + 2+ ocurrencias) y promoverlos a `procedural_skill` — la única tabla que, incluso después del plan de 3 capas, sigue sin escritor.

> **Veredicto: viable y es la continuación natural.** El lado de LECTURA de las skills está 100% construido (tabla, KNN, tier 1, workspace.md). El lado de INSUMO lo entrega el plan de 3 capas (trayectorias episódicas con `success_score`). Lo que falta es exactamente el MINERO: el proceso que transforma trayectorias en skills. Es la pieza con mayor densidad de decisiones de diseño de todo el sistema de memoria — acá no hay plumbing, hay algoritmo.

La analogía: el plan de 3 capas construyó la mina (trayectorias acumulándose) y la joyería (retrieval de skills con tier 1). El doc 8 es el refinador que convierte mineral en joyas. Sin él, `procedural_skill` queda vacía para siempre.

### ⚠️ Reconciliación obligatoria con el plan de 3 capas

El `mine_session` del doc 8 (paso 1) dice **"SIEMPRE persistir la trayectoria en `episodic_trajectory`"** — pero la Fase 2 del plan de 3 capas ya construye ese escritor en el mismo hook (cierre de sesión). Dos escritores para la misma fila = duplicación o corrupción.

**Resolución adoptada:** el miner **absorbe** el flujo del escritor episódico. El `episodic_writer` de la Fase 2 del plan anterior queda como *builder* (recolecta tool_calls/errores durante la sesión, computa `success_score`, arma la `EpisodicTrajectoryRow`) y `mine_session(trajectory)` se convierte en el único punto que inserta Y mina, en ese orden. Un solo hook, un solo INSERT, cero carreras.

**Constraint hacia atrás:** la Fase 2 del plan de 3 capas debe diseñar su escritor como función componible (`build_trajectory` separado de `insert_trajectory`), no como monolito del hook — así el miner lo envuelve sin refactor. Registrado también en el apéndice de aquel plan.

---

## 2. Lo que YA estará implementado al arrancar ✅

### Del RAG original (existe hoy)

| Componente | Archivo | Detalle |
|------------|---------|---------|
| **Tabla `procedural_skill` completa** | `migrations/V1–V3` | `skill_name`, `trigger`, `tool_sequence`, `success_count`, `promoted_at`, `source_trajectory_ids`, `importance` (default 8.0), `tier` (default 1) + `procedural_vectors` (float[384]) |
| **Lado de lectura de skills** | `search.rs`, `workspace.rs` | `knn_procedural` (half-life 180d hardcoded), `SearchScope::procedural_only`, top-5 skills en la sección AUTO del workspace.md |
| **`compute_tier` con label `procedural_skill` → tier 1** | `tier.rs` | La función pura ya resuelve el label estructural a tier 1 (lesson Phase 5 del RAG) — `promote_to_procedural_skill` la invoca, NO hardcodea |
| **Tabla `episodic_trajectory`** | `migrations/V1–V3` | `session_id UNIQUE`, `task`, `tool_calls`, `errors_encountered`, `resolution`, `success_score` + `episodic_vectors` |
| **Patrón LLM fire-and-forget** | `resolver.rs` | Adjudicación LLM spawneada desde `commands/session.rs` — molde para el Root Cause Analysis |
| **Patrón atomic INSERT** | `store_insert.rs` | Una sola `.call()` con `tx.transaction()` para fila + vector — a copiar en `promote_to_procedural_skill` |

### Del plan de 3 capas (premisa: completado)

| Componente | Fase de origen | Qué aporta al miner |
|------------|----------------|---------------------|
| **Builder de trayectorias** (`build_trajectory`: tool_calls, errores, `resolution`, heurística de `success_score`) | Fase 2 | El INSUMO del miner — sin esto no hay nada que minar |
| **Hook de cierre de sesión** | Fases 1–2 | El punto de integración donde `mine_session` se cuelga |
| **Extractor LLM de memorias pivotales** | Fase 1 | Patrón de prompt estructurado + parseo, reutilizable para nombrar skills y para el RCA |
| **Job de mantenimiento (timer 24h)** | Fase 3 | Infraestructura de scheduling si algún paso del mining se difiere a background |
| **Comandos Tauri de inspección** | Fase 4 | Observabilidad para verificar skills promovidas y medir el criterio de éxito |

---

## 3. Lo que FALTA (gaps) ❌

| # | Gap | Detalle |
|---|-----|---------|
| 1 | **`TrajectoryMiner` no existe** | Ni struct, ni `mine_session`, ni hook. Cero referencias en el codebase |
| 2 | **`extract_tool_sequences` — el algoritmo no está definido** | El doc dice "extrae la secuencia exacta de tool calls" pero no define QUÉ es una skill dentro de una trayectoria: ¿la secuencia completa? ¿sub-secuencias (n-grams)? ¿segmentos entre error y resolución? Decisión de diseño central |
| 3 | **`filter_recurring` — matching de recurrencia sin definir** | ¿Cómo se decide que dos trayectorias contienen "la misma" skill? ¿Match exacto de secuencia normalizada? ¿Similaridad vectorial del task? Determina la calidad de todo el sistema |
| 4 | **`promote_to_procedural_skill` no existe** | Sería el PRIMER escritor de `procedural_skill`. Falta además la política de dedup: si la skill ya existe → `success_count++`, NO fila nueva |
| 5 | **Root Cause Analysis no existe** | Clasificar fallos (contexto faltante / limitación del modelo / error procedimental) requiere LLM. Además: ¿dónde se guarda el veredicto? No hay columna `root_cause` en `episodic_trajectory` — probable migración V4 |
| 6 | **Sin métricas del ciclo de aprendizaje** | El criterio de éxito ("tercera sesión resuelve en menos pasos", "errores recurrentes disminuyen") no es medible sin instrumentación |

---

## 4. Las fases

Orden recomendado: **1 → 2 → 3 → 4**. Las Fases 1–2 son el corazón (sin ellas no hay skills); la 3 agrega aprendizaje de fallos; la 4 hace medible el criterio de éxito.

### Fase 1 — Pipeline del miner + Golden Trajectory gating ⭐

**Objetivo:** el esqueleto completo del flujo con la reconciliación resuelta: un solo hook al cierre de sesión que construye, inserta y mina.

- **Nuevo:** `src-tauri/src/memory/miner.rs` — `TrajectoryMiner { db, recurring_threshold }` con `mine_session`
- Absorbe el flujo del escritor episódico (ver §1 Reconciliación): `build_trajectory` → `insert_episodic_trajectory` (SIEMPRE, exitosa o no) → gate `success_score > 0.85` → `extract_tool_sequences`
- **Decisión de diseño (en propose):** definición de skill dentro de una trayectoria. Recomendación v1: secuencia de tool calls normalizada (nombres, sin argumentos) entre el inicio de la tarea y la resolución, con colapso de reintentos consecutivos. Determinista y testeable; nada de LLM en este paso
- **Nombrado de la skill (`skill_name`, `trigger`):** paso LLM reutilizando el patrón del extractor de Fase 1 del plan anterior. Si el LLM no responde, la skill queda con nombre heurístico — el mining NUNCA bloquea el cierre de sesión
- **Tests primero:** trayectoria con score bajo → solo INSERT episódico, cero skills; score alto → secuencias extraídas correctas; reintentos colapsados

### Fase 2 — Recurrencia y promoción a `procedural_skill`

**Objetivo:** cerrar el loop del doc: `filter_recurring` + el primer escritor de `procedural_skill`.

- **`filter_recurring`:** buscar la misma secuencia normalizada en trayectorias previas (`recurring_threshold: 2`). Recomendación v1: match exacto sobre secuencia normalizada (determinista); similaridad vectorial del `task` como refinamiento futuro — se anota como change posible, no se implementa
- **`promote_to_procedural_skill`:** INSERT atómico en `procedural_skill` + `procedural_vectors` (patrón `insert_memory_with_vector`), con:
  - `importance = 8.0` (heurística estructural del doc — sin LLM)
  - `tier = compute_tier(3, 8.0, 0, "procedural_skill")` → **invoca la UDF, no hardcodea 1**
  - `source_trajectory_ids` = JSON array con el trajectory actual + los previos que `filter_recurring` devolvió (audit trail, principio de explicabilidad del doc 13)
- **Dedup obligatorio:** si ya existe una skill con la misma secuencia normalizada → `success_count++` y append a `source_trajectory_ids`, NO fila nueva
- **Resultado visible:** las skills aparecen solas en la sección "Procedural Skills" del workspace.md — el lado de lectura ya existe y las va a levantar sin tocar una línea

### Fase 3 — Root Cause Analysis de trayectorias fallidas

**Objetivo:** que las sesiones fallidas también enseñen.

- Gate inverso: `success_score` bajo → clasificación LLM en las 3 categorías del doc (contexto faltante / limitación del modelo / error procedimental), patrón `resolver.rs` fire-and-forget
- **Decisión de diseño (en propose):** destino del veredicto. Recomendación: migración **V4** agregando `root_cause TEXT NULL` a `episodic_trajectory` (el análisis pertenece a la trayectoria, no es una memoria nueva). Alternativa descartable: fila en `memory` con `memory_type='semantic_fact'` — mezcla granularidades, exactamente lo que el split de 3 tablas vino a evitar
- La categoría "contexto faltante" es además señal de mejora para el retrieval (¿el KNN no trajo lo que hacía falta?) — se registra, el consumo automatizado queda para doc 9/10

### Fase 4 — Métricas del ciclo de aprendizaje

**Objetivo:** hacer medible el criterio de éxito del doc 8.

- Instrumentar: pasos por tarea (largo de `tool_calls`), categorías de fallo por período, hits de skills en retrieval (¿el agente USA las skills promovidas?)
- Extender los comandos Tauri de inspección (Fase 4 del plan anterior) con vista de skills: nombre, `success_count`, trayectorias de origen
- Dashboard mínimo o log estructurado — suficiente para validar "tercera sesión, menos pasos"

### Fuera de scope (intencional)

| Qué | Por qué NO va acá |
|-----|--------------------|
| Consolidación nocturna / RAPTOR-lite sobre trayectorias | Doc 9 (autoDream) — `summarize_inactive_sessions` LEE de `episodic_trajectory`, pero es otro ciclo |
| Eviction de skills que dejaron de funcionar | Doc 10 (Strategic Forgetting). Acá solo se registra `success_count`; la democión/eviction por obsolescencia es del ciclo de olvido |
| Clustering semántico de patrones (embeddings de secuencias) | El "Pattern Clustering" del doc se entrega en v1 como match exacto normalizado. Clustering vectorial = change futuro cuando haya volumen real de trayectorias que lo justifique. YAGNI |
| Re-ranking del retrieval usando root_cause | Consumo automatizado de RCA es doc 9/10 |

---

## 5. Cómo se trabaja: SDD por fase

Mismo protocolo que el plan de 3 capas: cada fase es un **change SDD independiente**, no se arranca una fase sin archivar la anterior.

```
proposal -> specs --> tasks -> apply -> verify -> archive
             ^
             |
           design
```

1. **`/sdd-new trajectory-miner-fase-N`** → exploración + propuesta
2. **Specs + design** → acá viven las decisiones grandes de esta feature: definición de skill (Fase 1), matching de recurrencia (Fase 2), destino del RCA (Fase 3). **Este plan tiene más peso en design que el anterior** — el plan de 3 capas era plumbing sobre arquitectura hecha; este define algoritmos
3. **`/sdd-continue`** → tasks
4. **`/sdd-apply`** → implementación, **TDD estricto** — los algoritmos de extracción y matching son puros y deterministas: perfectos para test-first
5. **`/sdd-verify`** → validación contra specs
6. **`/sdd-archive`** → cierre + lessons al apéndice

**Topic keys engram:** `sdd/trajectory-miner-fase-N/{explore|proposal|spec|design|tasks|apply-progress|verify-report}`

### Tracking de progreso

| Fase | Change SDD | Estado | Archivado |
|------|-----------|--------|-----------|
| 1 — Pipeline + Golden gating | `trajectory-miner-fase-1` | ✅ Archivada | 2026-07-12 |
| 2 — Recurrencia y promoción | `trajectory-miner-fase-2` | ✅ Archivada | 2026-07-12 |
| 3 — Root Cause Analysis | `trajectory-miner-fase-3` | ✅ Archivada | 2026-07-12 |
| 4 — Métricas del ciclo | `trajectory-miner-fase-4` | ✅ Archivada | 2026-07-26 |

Estados: ⬜ Pendiente → 🔵 En propuesta → 🟡 En implementación → 🟢 Verificada → ✅ Archivada

---

## 6. Criterio de éxito global (del doc 8)

- Después de la tercera sesión resolviendo el mismo tipo de problema, el agente lo resuelve en menos pasos (medible con Fase 4)
- Los errores recurrentes (mismas categorías de fallo) disminuyen progresivamente (medible con Fases 3+4)
- `procedural_skill` deja de estar vacía: skills validadas aparecen en el workspace.md sin intervención manual

---

## Apéndice — Lessons learned

**Regla de mantenimiento:** al cerrar cada fase (`/sdd-archive`), las lessons del verify-report se integran acá. Mismo patrón que `docs/plan-implementacion-memoria-3-capas.md` y `docs/completed-impl/RAG-PHASE-PLAN.md`.

### Lessons heredadas (del RAG y del plan de 3 capas — aplican a esta feature)

Las 20 lessons del apéndice del plan de 3 capas aplican íntegras (constraints del código, patrones probados, disciplina de testing, proceso SDD). Las específicamente críticas para el miner:

1. **Atomic INSERT multi-tabla en una sola `.call()` con `tx.transaction()`** — `promote_to_procedural_skill` e `insert_episodic_trajectory` copian el patrón de `insert_memory_with_vector`. Es la única forma de garantizar 1-a-1 fila ↔ vector
2. **`compute_tier` se INVOCA, no se hardcodea** — el doc 8 lo dice explícito (línea 81): el label `"procedural_skill"` resuelve a tier 1 en `tier.rs`; si las reglas cambian, cambian en un solo lugar
3. **Fire-and-forget desde el hook, nunca bloquear el path crítico** — el mining corre spawneado al cierre de sesión (patrón `resolver.rs`); un LLM caído NO impide cerrar la sesión ni insertar la trayectoria
4. **`AppState.embedder` puede estar vacío (`OnceCell`)** — el miner tolera `embedder.get() == None`: inserta la trayectoria sin vector o encola, no crashea
5. **NO mockear la cosa testeada** — los algoritmos de extracción/matching se testean puros con trayectorias sintéticas; los mocks LLM son solo para el nombrado y el RCA
6. **Reconciliar spec vs design ANTES del apply** — este plan nace de una reconciliación (¿quién inserta la trayectoria?); la Fase 1 la valida en su propose contra el estado REAL del código post-plan-3-capas
7. **`log::warn!` para errores recuperables del worker** — mining fallido = warn + continue, nunca panic

### Lessons heredadas de las 4 fases del plan de 3 capas (integradas al arrancar Fase 1)

Fuente: apéndice de `docs/sdd/3-layers-memory/plan-implementacion-memoria-3-capas.md`. Las que aplican directo al miner:

**Del código y sus patrones:**

8. **Split `AppHandle`-wrapper / testable-core es convención establecida (4 instancias: worker → resolver → extractor → episodic_writer)** — el miner es la 5ª: core testeable con deps explícitas, wrapper fino que solo resuelve `AppHandle`/`State`. Complemento: pure-helper extraction para internals de comandos Tauri (`snapshot_conversations`, `outgoing_session_id`)
9. **Las restricciones de persistencia viven en la frontera de serialización, no en el builder puro** — `build_trajectory` no conoce el cap de 500 chars (lo aplica `run_episodic_writer_core` al serializar). El miner puede envolver el builder con caps propios sin refactor — esta separación se diseñó PARA el miner (forward-compat de Fase 2 del plan anterior, cumplida)
10. **`last_accessed_at` se guarda en MILISEGUNDOS** — auditar unidades reales de columnas compartidas en sus writers antes de escribir SQL nuevo. Bug preexistente en `search.rs` (decay lo trata como segundos) anotado como change futuro `fix-decay-unit-mismatch`; código nuevo computa `now_ms` en Rust y lo bindea
11. **`compute_tier` es PROMOTE-ONLY y NO tiene regla 3→2** — si una fase la necesita, es change explícito con spec, no "fix de paso"

**De testing:**

12. **Worked-example tests lockean fórmulas, no texto del spec** — `success_score` tiene techo real **0.9** (no 1.0): `0.4 + 0.5·ended_clean − 0.5·(unresolved/tool_calls)`, clamp [0,1]. Si esta fase gatea con `> 0.85`, la banda satisfacible es (0.85, 0.9] — verificar contra la fórmula REAL, no contra el doc 8. Floats con epsilon, nunca `assert_eq!`
13. **Reloj pausado de tokio + `conn.call()` de tokio_rusqlite entre `advance()` = flaky** — colapsar advances sin I/O de BD entre medio; verificar BD solo antes del primero y después del último
14. **Gates de calidad se redactan relativos al diff** — "0 warnings clippy" = 0 warnings NUEVOS (hay ~114 preexistentes)

**De proceso SDD:**

15. **Design gana sobre spec en decisiones que leyeron el código; spec gana en contratos que los tests lockean** — reconciliar ANTES del apply; si el pseudo-código del design queda stale, se enmienda el artefacto en archive
16. **Las ambigüedades del doc de referencia se convierten en requirements explícitos testeados, no se resuelven en silencio** — ej.: democión scopeada a `memory` con test que asevera tablas hermanas intactas
17. **`mem_save` sin `project` falla silencioso (reincidió 2 veces)** — todo prompt delegado explicita `project: 'claw-garden'` + verificación post-save; el orquestador re-verifica con `mem_search` scoped antes de lanzar la fase dependiente
18. **Fault tolerance por construcción es veredicto de verify aceptable** — jobs autónomos retornan `()`, error local con `log::warn!`, nunca `?`/panic en el loop
19. **Métricas que cruzan subsistemas se loguean en el único call site que ve ambas señales** — no en los primitivos

### Lessons learned (Fase 1 — Pipeline + Golden gating)

20. **Design valida worked-examples contra la fórmula IMPLEMENTADA, no contra la fórmula abstracta del spec** — El design descubrió con aritmética f64 que dos de los worked-examples del spec eran estados IMPOSIBLES: la fórmula `build_trajectory` acopla `ended_clean` y `unresolved` (clean ⇒ unresolved=0 ⇒ score exacto 0.9; no-clean ⇒ score ≤ 0.4). La banda (0.85, 0.9) es inalcanzable en el path de producción real. Lección: enmienda de spec antes de tasks, nunca se arrastra al apply — se hace via upsert del artefacto spec con nota "Amendment (post-design)". Verificar siempre contra fórmula REAL, con epsilon 1e-9, nunca hardcodear scores de entrada.

21. **Cuando el borde exacto de un gate es estructuralmente inalcanzable por coupling en el path de producción, el lock se baja a nivel const-aritmético** — El gate `score > 0.85` es efectivamente binario (solo clean=0.9 o non-clean≤0.4 son posibles). Protección: prod code usa `<=` (equivalente a `!(>)`, sin warnings clippy); tests usan `std::hint::black_box` para evitar alertas por const. Ambas preservan el lock observable `>` 0.85. Documentar que es lock de const-aritmética, no de path real, para que revisiones futuras no intenten "optimizarlo".

22. **Requirements de verificación por conteo literal de símbolos (`rg`) son frágiles — documentar sobre invocaciones reales, no sobre comentarios/docs** — El apply tuvo que revertir doc-comments para que el gate "0 refs al símbolo viejo" no se rompiera por menciones en documentación. Redactar requirements contables como "run_episodic_writer\( appearances = 0 en todo el crate" (sobre invocaciones reales) no sobre "símbolo mencionado en ningún lado" (que incluye docstrings). Plus: verificar en el diff, no sobre el código base completo.

23. **Agente delegado truncado por API error mid-stream SIN mem_save = artefacto perdido en silencio; red de defensa: re-verificación del orquestador con mem_search scoped + resume del mismo agentId via SendMessage** — Ocurrió 2 veces: proposal y spec tuvieron interrupciones. No hay alerta visible; el orquestador debe, después de CADA delegación, hacer `mem_search(query: topic_key, project: 'claw-garden')` + si no encuentra, reintentar el agente vía `SendMessage(to: agentId, message: "resume from transcript")`. Implementar esto en la meta-regla del orchestrator SDD.

24. **El conteo absoluto de warnings de clippy difiere por unidad de compilación (lib vs lib-test, ~123 vs ~114 en summary-lines)** — Los gates "0 new warnings" deben filtrarse por archivos del diff, no comparar números absolutos. Recomendación: store baseline pre-change por target, comparar delta post-change por target identidad. Artefacto dual-line (lib + lib-test) si ambos targets se tocan — documentar para evitar ambigüedad futura.

25. **Naming fallback-persist es el patrón correcto para máquinas de LLM en path crítico** — El miner asigna heuristic-first (siempre válido), intenta LLM con timeout 30s + defensive parse, y en timeout/error/parse-fail/provider-absent → `log::warn!`, se queda con heuristic. Session close NUNCA espera LLM. Tests de timeout aislados a nivel unitario (tokio start_paused + advance sin I/O de BD). Este patrón reutilizable para RCA (Fase 3), extractor de memorias, y cualquier LLM en hook.

26. **Timeout test sin I/O de BD entre tokio::time::advance llamadas evita flakiness** — Fue gotcha del plan anterior (lesson 13): si hay BD I/O (`.call()`, `.await`) entre `advance()` calls, el test puede volverse flaky. Para `llm_name` timeout test, construir sin `&MemoryStore`, resolver providers inline, timeout sin DB. Plus: usar `tokio::test(start_paused=true)` para control absoluto de reloj.

### Lessons learned (Fase 2 — Recurrencia y promoción)

27. **Fases paralelas (spec ∥ design) exigen mensajería de reconciliación del orquestador** — El spec flaggeó la semántica del dedup-increment (R8) mientras el design ya corría; el orquestador reenvió el flag al agente de design in-flight, que lo resolvió contra el código real y produjo la enmienda. Sin ese relay, la ambigüedad llegaba a tasks. Patrón: todo flag abierto de un artefacto paralelo se reenvía al agente hermano ANTES de que persista el suyo.

28. **Semánticas de conteo se lockean como INVARIANTE sobre estado, no como regla de incremento por pasada** — `success_count += total_occurrences` double-counteaba porque las previas Golden persisten y re-matchean en CADA cierre posterior (sesión C daría 5 vs el 3 correcto). El lock correcto: `success_count == len(distinct source_trajectory_ids)`, protegido por un test de 3 sesiones que falla bajo la lectura double-count y un test de idempotencia (all ids present → no increment).

29. **El design que lee el código real caza bugs de move-semantics antes del apply** — `mine_session_core` MOVÍA el `embedder` a `run_episodic_writer_core` (miner.rs:221) y la promoción lo necesitaba después; el design lo detectó y mandó `embedder.clone()` como tarea explícita. Validar firmas contra el código, no contra pseudo-código de artefactos.

30. **Warnings de cobertura del verify se cierran con lock-tests puros, y el RED de un lock-test se demuestra con swap temporal en producción + revert** — Para assertar el string exacto de embedding (`"{trigger} {skill_name}"`) se usó un `CapturingEmbedder` que registra inputs; el RED se probó invirtiendo temporalmente el orden en producción (test falla con el diff esperado), revirtiendo y verificando con grep. Cero producción tocada al final.

31. **El conteo canónico de clippy es la línea `generated N`, no `grep -c "^warning:"`** — El grep naive cuenta summary-lines y duplicados acumulados (124/153 vs los reales 123/114). Los gates por baseline deben comparar el `generated N` por target idéntico. Complementa la lesson 24.

32. **Baselines de test declarados en prompts se re-verifican contra la corrida real antes de asumir regresión** — El prompt del apply decía 587 passed pero el baseline real era 586 + 1 flaky preexistente (`tier2_under_180_days_unchanged`, ajeno al diff, confirmado con `git diff --stat` vacío). El apply lo documentó como flaky en vez de "arreglarlo" — causa raíz pendiente como deuda explícita fuera de scope.

### Lessons learned (Fase 3 — Root Cause Analysis)

33. **Fault tolerance por construcción es veredicto aceptable de verificación para paths fire-and-forget con errores tragados** — El path de error DB de `update_trajectory_root_cause` no tiene harness de inyección de fallos (solo happy-path + LLM timeout tests). El verify flaggeó como warning; resolución: el handler es branchless (`log::warn!` + return `Ok(())` incondicionalmente), el callee está totalmente desacoplado del caller (session.rs nunca espera el resultado de la tarea spawneada), y la trayectoria persiste igual. Lección: cuando un path es garantizado fault-tolerant por contrato del call-site (spawn-and-forget) Y el error handler es branchless (sin recovery condicional), la barra de verificación para ese path NO es "testear cada rama de error" sino "confirmar el desacoplamiento en call site." Documentar el gap residual (test de inyección DB) como deuda conocida; no retrofittear un test para un path que es observably inalcanzable en producción.

34. **Cuando un feature legítimamente reutiliza un recurso compartido debajo de un gate, los locks preexistentes de "N calls" se re-expresan via una invariante ortogonal** — El test de Fase 1 `low_score_only_inserts_zero_candidates_zero_llm` lockeaba "el provider LLM nunca se invoca debajo del gate Golden" contando llamadas. El RCA ahora invoca legítimamente el provider debajo del gate (`calls == 1`), volviendo ambiguo el contador. Resolución del apply (validada por verify): renombrar a `_namer_never_invoked` y preservar el lock original transitivamente via la invariante co-aseverada `candidates == Vec::new()` (cero candidates prueba que el namer no corrió). Lección: los locks por conteo de llamadas son frágiles ante adiciones de features; re-expresarlos como propiedad ortogonal de estado que siga probando la garantía original.

35. **Los conteos de escenarios citados en briefs/reportes se deslizan del artefacto persistido — siempre contar los bloques literales `#### Scenario:`** — El reporte del agente de spec declaró "7 requirements, 15 escenarios"; el artefacto spec persistido (#1263) contiene 12 bloques `#### Scenario:` literales. El brief del apply arrastró el 15, el apply contó 12, y verify reconcilió contando el artefacto él mismo: 12 es el número real, los 12 cubiertos. Lección: el spec persistido es el contrato; los gates de cobertura se verifican contando los bloques literales del artefacto, nunca los números citados en reportes o prompts de delegación. El conteo de tests (+24) es una tally separada — los tests son más finos que los escenarios.

36. **Los maintenance time-based tests (`tier1_low_importance_under_90_days_...`, `tier2_under_180_days_unchanged`) son flaky bajo paralelismo full-suite pero green en aislamiento — clock-injection pendiente como deuda explícita, nunca atribuir sus fallos a diffs del feature** — En el verify de Fase 3 la suite completa dio 629/2 con esos dos únicos fallos; en aislamiento (`cargo test tier -- --test-threads=1`) pasan 6/6. `maintenance.rs` no está en el diff de la fase (verificable con `git diff`), así que el efectivo es 631/631. Es el mismo flaky ya documentado en lessons 32; ahora se manifestó en ambos tests tier. Lección: ante fallos de suite completa, correr los tests sospechosos en aislamiento y confirmar con `git diff` que el archivo no está en el cambio antes de atribuir regresión. **RESUELTO post-archive (2026-07-12)**: la causa raíz NO era el reloj de tokio — era un margen de carrera de exactamente **1 ms** entre dos lecturas de wall-clock: los tests de borde inferior sembraban `last_accessed_at = now_ms() − (90d − 1ms)` y `run_maintenance_once` computaba su PROPIO `now_ms` después; cualquier gap de scheduling > 1ms cruzaba el borde y democionaba espurio. Fix: clock injection — `run_maintenance_once_at(store, now_ms)` como core inyectable (wrapper de producción sin cambio de comportamiento) y los tests siembran + ejecutan contra el MISMO `now`. Preserva la semántica de borde exacto de 1ms de forma determinista. Verificado: suite completa 631/631 en corridas repetidas, clippy delta-0.

### Lessons learned (Fase 4 — Métricas del ciclo)

37. **El guard `json_valid` en `step_count` es load-bearing, no defensivo-decorativo — `json_array_length` LEVANTA "malformed JSON", NO retorna NULL** — El design cazó contra SQLite real que `COALESCE(json_array_length(tool_calls), 0)` NO satisface el escenario "JSON inválido → 0": ante input malformado `json_array_length` levanta error y el COALESCE nunca actúa. La forma correcta es `CASE WHEN json_valid(tool_calls) THEN COALESCE(json_array_length(...),0) ELSE 0 END`. El test que lo prueba usa un `UPDATE ... tool_calls='{bad'` crudo para saltear el insert tipado (`Vec<String>` serde + `NOT NULL DEFAULT '[]'` lo hace inalcanzable en prod) — sin el guard se habría shippeado un panic latente. Lección: cuando un escenario del spec es "inalcanzable por el tipo pero seedable en test", el test crudo por SQL es la única forma de ejercerlo, y verificar el comportamiento REAL del motor (no el asumido) antes de lockear el SQL.

38. **El histograma EXCLUYE `root_cause NULL` porque NULL mezcla 3 poblaciones — incluirlo contaría éxitos como fallos** — El RCA solo clasifica bajo el gate golden (`miner.rs:379`, `success_score <= 0.85`); las trayectorias exitosas quedan `root_cause` NULL POR CONSTRUCCIÓN, junto con filas pre-V4 y timeouts/parse-fails del RCA. Un bucket "unclassified" en un histograma de FALLOS contaría la mayoría (los éxitos) como fallos, corrompiendo la señal doc 8 "errores recurrentes disminuyen". El design ganó sobre el spec acá (D5) con evidencia dura del código. El test aserta el NEGATIVO (`rows.all(|r| !r.root_cause.is_empty())`), no solo un count positivo — un test que solo cuenta pasaría vacío ante un bug que cuente golden. Lección: cuando un WHERE excluye una población, el test aserta la ausencia explícita, no la presencia de lo esperado.

39. **El guardrail de diff se verifica contra el gitStatus INICIAL de la sesión, no contra `git diff --stat` crudo** — El working-tree estaba contaminado por un change previo en progreso (`memory/*`, `V4`, `miner.rs` estaban en el gitStatus inicial). Leer `git diff --stat` como "el diff del change" habría reportado archivos ajenos. El método correcto: aislar los archivos PROPIOS del change (`memory_admin.rs`, `prompt_builder.rs`, `lib.rs`) restando lo que ya estaba sucio al arrancar. La invariante real del guardrail —`access.rs` ausente del diff— se sostuvo. Lección: en un repo con cambios pre-existentes, el guardrail "archivo X no se toca" se mide sobre el delta de la sesión, no sobre el estado absoluto del working-tree.

40. **`cargo clippy --lib --tests` imprime DOS líneas `generated N` (lib=123 Y lib-test=114); el gate mira la línea `(lib test)`** — Confirma la Lesson 31: el target del gate es la línea `(lib test) generated N`, no la línea `lib` co-impresa ni un `grep -c "^warning:"` (que doble-cuenta summary-lines). Leer la línea equivocada misreportaría el delta. Baseline re-verificado esta sesión por el verifier independiente: 645 passed / 0 failed, `generated 123` (lib) / `generated 114` (lib-test), delta-0 en ambos.
