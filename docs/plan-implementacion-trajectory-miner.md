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
| 1 — Pipeline + Golden gating | `trajectory-miner-fase-1` | ⬜ Pendiente | — |
| 2 — Recurrencia y promoción | `trajectory-miner-fase-2` | ⬜ Pendiente | — |
| 3 — Root Cause Analysis | `trajectory-miner-fase-3` | ⬜ Pendiente | — |
| 4 — Métricas del ciclo | `trajectory-miner-fase-4` | ⬜ Pendiente | — |

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

*Al arrancar la Fase 1, sumar acá las lessons que hayan dejado las 4 fases del plan de 3 capas (su apéndice es la fuente).*

### Lessons learned (Fase 1 — Pipeline + Golden gating)

*Pendiente — se completa al archivar `trajectory-miner-fase-1`.*

### Lessons learned (Fase 2 — Recurrencia y promoción)

*Pendiente — se completa al archivar `trajectory-miner-fase-2`.*

### Lessons learned (Fase 3 — Root Cause Analysis)

*Pendiente — se completa al archivar `trajectory-miner-fase-3`.*

### Lessons learned (Fase 4 — Métricas del ciclo)

*Pendiente — se completa al archivar `trajectory-miner-fase-4`.*
