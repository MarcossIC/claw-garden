# Plan de Implementación — Arquitectura de Memoria en 3 Capas

**Área:** Backend Rust — módulo `src-tauri/src/memory/`
**Base:** `docs/7-arquitectura-memoria-3-capas.md` (spec de la feature)
**Estado del RAG:** implementado (commit `afb5287 feat: add rag impl`)
**Metodología:** SDD (Spec-Driven Development) — cada fase es un *change* independiente con su propio ciclo completo

---

## 1. Contexto y veredicto

El doc 7 describe la memoria agéntica en 3 capas (episódica / procedimental / semántica) sobre una jerarquía de almacenamiento de 3 tiers (HMO). El análisis del código actual arroja una conclusión clave:

> **La feature está ~70% implementada.** Toda la capa de STORAGE y RETRIEVAL del doc 7 ya existe y está testeada. Lo que falta son los **PRODUCTORES de datos**: nada escribe en las tablas episódica/procedimental, y nadie emite los tipos de memoria pivotales que alimentan el Tier 1.

La analogía: construimos una autopista de 3 carriles por la que hoy circula un solo carril (`chat_message`, `tool_error`, `session_summary`). Este plan no implementa el doc 7 desde cero — **lo termina**.

---

## 2. Lo que YA está implementado ✅

| Componente | Archivo | Detalle |
|------------|---------|---------|
| **3 tablas por granularidad** | `migrations/V1–V3` | `memory` (particle), `episodic_trajectory` (session), `procedural_skill` (pattern) + 3 tablas vec0 (`float[384]`) + view `all_memories` |
| **Columnas de tier completas** | `migrations/V1–V3` | `tier`, `importance`, `access_count`, `last_accessed_at`, `status`, `scope`, `dispute_count` en las 3 tablas, con índices `idx_*_tier_status` |
| **Promoción / democión de tier** | `src-tauri/src/memory/tier.rs` | `compute_tier` (promoción eager, registrada como UDF SQL) y `demote_inactive` (democión lazy — **implementada pero nunca invocada**) |
| **Score con tier floors** | `src-tauri/src/memory/score.rs` + `search.rs` | `(1-distancia) × MAX(tier_floor, exp_decay) × importance_factor × dispute_factor`. Floors: tier 1 = 0.5, tier 2 = 0.2, tier 3 = 0.05. Decay λ polimórfico por `memory_type` |
| **KNN cross-table** | `src-tauri/src/memory/search.rs` | `search_all_memories`: 3 queries paralelas con `tokio::try_join!`, k-pools independientes por tabla (`SearchScope`), merge por score, `K_FINAL=10` |
| **Tier 1 = workspace.md** | `src-tauri/src/memory/workspace.rs` | `WorkspaceManager` regenera la sección AUTO (cap 1500 tokens) con filas tier 1; sección USER intocable. El KNN filtra `WHERE tier > 1` para no duplicar |
| **AccessTracker** | `src-tauri/src/memory/access.rs` | Buffer fire-and-forget; flush cada 30s o 500 entradas: actualiza `access_count` y recomputa `tier` en un solo UPDATE con CTE |
| **Pipeline de ingesta** | `src-tauri/src/memory/worker.rs`, `events.rs`, `chunker/` | `memory_worker` async: chunking por tipo, embedding local (fastembed, `multilingual-e5-small`, 384 dims), detección de contradicciones inline |
| **Resolutor de disputas** | `src-tauri/src/memory/resolver.rs` | Adjudicación LLM de memorias contradictorias, spawneado fire-and-forget al abrir sesión — **patrón reutilizable para el extractor de la Fase 1** |
| **Prompt builder 4 bloques** | `src-tauri/src/memory/prompt_builder.rs` | WORKSPACE / RECENT / LONG-TERM / CONFLICTS |

---

## 3. Lo que FALTA (gaps) ❌

| # | Gap | Consecuencia |
|---|-----|--------------|
| 1 | **Cero INSERTs en `episodic_trajectory` y `procedural_skill`** — solo lectura/búsqueda | Dos de las tres capas de memoria están vacías. Sin trayectorias tampoco hay insumo para el TrajectoryMiner (doc 8) |
| 2 | **Nadie emite memorias pivotales** (`user_preference`, `architectural_decision`, `semantic_fact`) | El workspace.md (Tier 1) se regenera cada turno pero está casi vacío. El criterio de éxito del doc — "Tier 1 suficiente para el 80% de las consultas" — es inalcanzable |
| 3 | **`demote_inactive` nunca se invoca** (comentario en código: "Phase 8+") | Las memorias suben de tier pero nunca bajan — el Tier 1 crecería sin límite con el uso real |
| 4 | **`SemanticMemory` del doc no está modelada** (`concept`, `confidence`, `related_concepts`) | Hoy solo existe como string `memory_type='semantic_fact'`. Sin grafo de conceptos |
| 5 | **Sin comandos Tauri de gestión de memoria** (solo `retry_embedder_download`) | No hay forma de inspeccionar memorias, hacer override de `importance` (el doc promete `importance >= 9` como override del usuario) ni medir el criterio del 80% |

---

## 4. Las fases

Orden recomendado: **1 → 2 → 3 → 4**. La Fase 1 es la de mayor valor visible; las demás son incrementales e independientes entre sí.

### Fase 1 — Extractor de memorias pivotales ⭐

**Objetivo:** que el Tier 1 cobre vida. Un paso LLM al cierre de sesión / compaction que analiza la conversación y extrae `user_preference`, `architectural_decision` y `semantic_fact` con su `importance`.

- **Nuevo:** `src-tauri/src/memory/extractor.rs` — prompt de extracción, parseo de respuesta estructurada, inserción vía `insert_memory_with_vector`
- **Patrón a reutilizar:** `resolver.rs` (llamada LLM fire-and-forget spawneada desde `commands/session.rs`)
- **Verificar con tests:** que `compute_tier` asigne tier 1 a `user_preference` y tier 2 a `architectural_decision` según la tabla del doc 7 (el código existe; el mapeo hay que testearlo)
- **Resultado visible:** el workspace.md empieza a llenarse solo

### Fase 2 — Escritor episódico

**Objetivo:** poblar `episodic_trajectory`. Al cierre de sesión se construye la fila: `session_id`, `task`, secuencia de `tool_calls`, `errors_encountered`, `resolution`, `success_score`.

- **Nuevo:** `src-tauri/src/memory/episodic_writer.rs` + hook en el cierre de sesión
- **Heurística inicial de `success_score`:** errores no resueltos penalizan; tarea completada suma (se refina en doc 8)
- **Tier inicial:** 3, como manda el doc 7
- **Nota estratégica:** es el prerequisito del doc 8 (TrajectoryMiner) — sin trayectorias no hay nada que minar
- **⚠️ Constraint de forward-compatibility (del plan del doc 8):** diseñar el escritor como funciones componibles — `build_trajectory` (recolección + success_score) SEPARADO de `insert_episodic_trajectory` — NO como monolito del hook. El TrajectoryMiner (`docs/plan-implementacion-trajectory-miner.md`, §1 Reconciliación) va a absorber este flujo: su `mine_session` se convierte en el único punto que inserta Y mina. Si el escritor nace componible, el miner lo envuelve sin refactor

### Fase 3 — Job de mantenimiento (democión lazy)

**Objetivo:** invocar `demote_inactive`, que ya existe pero nadie llama.

- Task de Tokio al startup + timer de 24h que ejecute la democión
- Es el **placeholder del autoDream** (doc 9): cuando llegue el doc 9, este scheduler se reemplaza por el ciclo de consolidación completo
- **Tests:** filas con `days_since_last_access > 90` bajan 1→2, `> 180` bajan 2→3, `importance >= 9` queda protegida (pivotal absoluto)

### Fase 4 — Comandos Tauri de inspección y override

**Objetivo:** observabilidad y control manual.

- `list_memories` (con filtros por tabla/tier/tipo), `set_importance` (habilita el override `importance >= 9 → tier 1` prometido por el doc), `delete_memory` (soft delete vía `deleted_at`)
- Loguear el tier de cada hit recuperado → permite **medir el criterio de éxito del 80%** del doc 7

### Fuera de scope (intencional)

| Qué | Por qué NO va acá |
|-----|--------------------|
| Promoción a `procedural_skill` | La barrera de promoción ES el TrajectoryMiner (`success_score > 0.85` + 2+ ocurrencias) — es el **doc 8** completo. Duplicarlo a medias acá sería deuda |
| Grafo semántico (`related_concepts`, `confidence`) | No existe ningún consumidor de ese grafo todavía. Se agregan las columnas en una migración futura cuando haya quien las lea. YAGNI |
| RAPTOR-lite / consolidación nocturna | Doc 9 (autoDream) |
| Eviction definitivo | Doc 10 (Strategic Forgetting) |

---

## 5. Cómo se trabaja: SDD por fase

Cada fase se ejecuta como un **change SDD independiente**, con su ciclo completo. No se arranca una fase nueva sin archivar la anterior.

```
proposal -> specs --> tasks -> apply -> verify -> archive
             ^
             |
           design
```

### Flujo por fase

1. **`/sdd-new memoria-3-capas-fase-N`** → exploración + propuesta (intent, scope, approach)
2. **Specs + design** → requirements con escenarios verificables; el design documenta decisiones (p. ej. formato del prompt de extracción, heurística de success_score)
3. **`/sdd-continue`** → tasks: breakdown mecánico en checklist implementable
4. **`/sdd-apply`** → implementación por lotes, **TDD estricto: test primero, siempre**
5. **`/sdd-verify`** → validación contra specs (CRITICAL / WARNING / SUGGESTION)
6. **`/sdd-archive`** → cierre del change y persistencia del estado final

### Convenciones

- **Artefactos:** se persisten en engram con topic keys `sdd/memoria-3-capas-fase-N/{explore|proposal|spec|design|tasks|apply-progress|verify-report}`
- **Un change por fase:** scope acotado = verify confiable. Si durante una fase aparece scope nuevo, se anota como change futuro, no se infla el actual
- **TDD estricto:** ningún productor se escribe sin su test de inserción + retrieval KNN que lo recupere

### Tracking de progreso

Al avanzar cada fase con SDD, se actualiza esta tabla:

| Fase | Change SDD | Estado | Archivado |
|------|-----------|--------|-----------|
| 1 — Extractor de memorias pivotales | `memoria-3-capas-fase-1` | ✅ Archivada | 2026-07-10 |
| 2 — Escritor episódico | `memoria-3-capas-fase-2` | ✅ Archivada | 2026-07-11 |
| 3 — Job de mantenimiento | `memoria-3-capas-fase-3` | ✅ Archivada | 2026-07-11 |
| 4 — Comandos Tauri | `memoria-3-capas-fase-4` | ✅ Archivada | 2026-07-11 |

Estados: ⬜ Pendiente → 🔵 En propuesta → 🟡 En implementación → 🟢 Verificada → ✅ Archivada

---

## 6. Criterio de éxito global (del doc 7)

- El agente diferencia entre "recuerdo haber hecho esto" (episódica), "sé cómo hacerlo" (procedimental) y "entiendo qué es esto" (semántica) al inyectar contexto
- El Tier 1 (workspace.md) es suficiente para el **80% de las consultas** — medible recién con la Fase 4 (logging de tiers en retrieval)
- `demote_inactive` corre periódicamente sin competir por latencia con el path crítico del retrieval

---

## Apéndice — Lessons learned

**Regla de mantenimiento:** al cerrar cada fase (`/sdd-archive`), las lessons del verify-report y del archive-report se integran acá en la subsección de la fase correspondiente. Este apéndice es el resumen para quien arranca la fase siguiente sin leer los archives completos (mismo patrón que `docs/completed-impl/RAG-PHASE-PLAN.md`).

### Lessons heredadas del plan RAG (ya aprendidas — aplican a esta feature)

Constraints y patrones validados en las Phases 1–7 del RAG que los apply agents de este plan DEBEN respetar:

**Constraints del código existente (no inventar, no romper):**

1. **`compute_tier` es PROMOTE-ONLY y NO tiene regla 3→2** — tiers iniciales por tipo + un solo anchor 2→1 (`access_count >= 3` para architectural_decision/session_summary). El doc 7 describe una promoción 3→2 (`access_count >= 3 OR importance >= 7`) que **NO está implementada** — si alguna fase la necesita, es un change explícito con su spec, no un "fix de paso". Reconciliar doc vs código en el propose de esa fase.
2. **`demote_inactive` genera warning `never used` — ES INTENCIONAL**: el compiler corrobora que no está en el call graph. La Fase 3 lo cablea; recién ahí desaparece el warning. No suprimir con `#[allow(dead_code)]` antes.
3. **`created_by_type='system'` para tool results** — el output de tools lo autorea el sistema, NO el agente. No existe valor `'tool'`. Crítico para el sycophancy guard del detector de contradicciones.
4. **`log::` no `tracing::`** — convención del codebase. Errores recuperables de workers → `log::warn!` + skip-and-continue, nunca panic.
5. **Inconsistencia preexistente `scope='global'` (doc 12) vs `scope='shared'` (doc 6)** — NO resuelta. Si una fase toca scopes, alinear primero.

**Patrones probados (copiar, no reinventar):**

6. **Atomic INSERT multi-tabla en una sola `.call()` con `tx.transaction()`** — la única forma correcta de garantizar 1-a-1 `fila ↔ fila_vectors`. El escritor episódico (Fase 2) y el extractor (Fase 1) deben copiar el patrón de `insert_memory_with_vector`.
7. **Producer-consumer con `UnboundedSender`** — fire-and-forget desde el hot path, single consumer (worker), FIFO garantizado. Nuevos productores pueden alimentar el mismo canal sin cambio de arquitectura.
8. **Fire-and-forget side-effects se integran en el call site, NO en el primitivo** — el AccessTracker se cableó en `prompt_builder.rs`, no en `search_all_memories`. El extractor de Fase 1 sigue la misma lógica: hook en el cierre de sesión, no dentro del worker de ingesta.
9. **Defense in depth con `Option<Self>` builders** — `MemoryEvent::from_message` filtra en el constructor Y `select_chunker` filtra de nuevo. Futuros productores adoptan las dos capas.
10. **`AppState.embedder` es `Arc<OnceCell>` vacío en boot** — todo productor nuevo DEBE tolerar `embedder.get() == None` y skipear el embedding (no crashear, no bloquear).
11. **Estado non-`Send` → dedicated `std::thread` + mpsc/oneshot** — patrón de `FastEmbedLocal`, aplicar a cualquier runtime nativo futuro.
12. **vec0 NO es uneable** — `UNION ALL` sobre `embedding MATCH` no parsea. Por eso el orquestador cross-table es Rust, no SQL. No proponer "una query unificada".
13. **`SearchScope` con k-pool por tabla preserva diversidad de tipos** — un KNN unificado devolvería 40 chats y 0 skills (los chats siempre están más cerca semánticamente).

**Disciplina de testing (TDD estricto):**

14. **Timers en tests: `interval_at(now + period, period)` + `#[tokio::test(start_paused=true)]` + `tokio::time::advance`** — el tick t=0 de `interval` race-a con `select!`. NUNCA sleeps reales en tests. Aplica directo al job de la Fase 3.
15. **NO mockear la cosa que estás testeando** — `FakeEmbedder` es para callers del trait, no para auto-tests de la impl.
16. **`FakeEmbedder` está inline `#[cfg(test)]` en `embedder.rs`** — si una fase lo necesita cross-file, refactor mecánico a `#[cfg(any(test, feature = "test-helpers"))]` a scope de módulo.
17. **Verificar features de crates ANTES de lockear en proposal** (`cargo info` / `cargo add --dry-run`) — ya costó una corrección en CHECKPOINT 1 del RAG.
18. **Regresión en dirs untracked: `git stash` default NO captura untracked** — usar `git stash -u` o comparar outputs explícitamente.

**Proceso SDD:**

19. **Reconciliar spec vs design ANTES del apply** — cuando divergen, design suele ganar porque ya leyó el código. No arrastrar la divergencia a tasks.
20. **Decisiones inline del apply se documentan en el verify-report** — y las fases siguientes las heredan como constraints locked.

### Lessons learned (Fase 1 — Extractor de memorias pivotales)

1. **Cruzar requirements de retrieval contra filtros SQL pre-existentes ANTES de escribir el spec.** El filtro `AND m.tier > 1` de `search.rs:95` (pre-existente, no introducido por esta fase) hace que `user_preference` (tier 1) sea insatisfacible-por-diseño en el escenario KNN. No es bug: tier-1 es entregado exclusivamente por workspace.md, el KNN no lo duplica. Los productores tier 2/3 (`architectural_decision`/`semantic_fact`) SÍ son recuperables; tier 1 NO. Detectarlo al escribir el spec evita arrastrar un escenario PARTIAL hasta la verificación.

2. **El split `AppHandle`-wrapper / testable-core es ahora convención (3er caso: worker → resolver → extractor).** Tauri sin feature `test` hace que `AppHandle`/`State` sean inconstruibles en tests. Patrón: extraer un core testeable que toma sus deps explícitamente; el wrapper (untesteable) solo las resuelve. Futuros productores deben adoptar esto sin reinventar, no es un gap de cobertura.

3. **Regression-lockear productores existentes cuando un primitivo compartido empieza a computar para todos sus callers, en el mismo lote.** Mover `compute_tier` dentro de `insert_memory_with_vector` cambió `session_summary` de tier 3 (SQL DEFAULT) a tier 2 (correcto). Task 6.2 re-testeó los 3 productores viejos (chat_message/tool_error/session_summary) probando cero side-effects. Disciplina establecida: siempre lockear los existentes cuando amplías una primitiva.

### Lessons learned (Fase 2 — Escritor episódico)

1. **El orquestador debe verificar la persistencia de cada artefacto ANTES de lanzar la fase que depende de él.** El `mem_save` del spec omitió el parámetro `project` y quedó fuera del scope del proyecto — un fallo silencioso: el agente reportó éxito pero `mem_search` no lo encontraba. El design se escribió a ciegas (solo proposal + explore) y hubo que reconciliar spec vs design en la fase de tasks. La reconciliación funcionó (3 divergencias resueltas a favor del design, lesson 19 del RAG), pero el costo se evita con dos reglas: (a) todo prompt de delegación que pida `mem_save` debe explicitar `project` Y pedir verificación post-save con `mem_search` + obs ID confirmado; (b) el orquestador re-verifica con `mem_search` antes de lanzar la fase dependiente — el result contract del sub-agente puede reportar éxito falso.

2. **Los worked-example tests lockean la fórmula, no el texto del spec.** El spec decía literalmente "máximo 1.0" para `success_score`, pero la fórmula del design (`0.4 + 0.5·ended_clean − 0.5·(unresolved/tool_calls)`, clamp [0,1]) tiene techo real **0.9**. La reconciliación enmendó el spec y los tests aseveran `== 0.9` con ejemplos trabajados (sesión limpia / errores recuperados / no resueltos). Convención: los coeficientes se congelan vía worked-examples; cambiarlos (p. ej. si el doc 8 mueve el gate de 0.85) exige re-derivar todos los ejemplos, no tocar un número suelto. Bonus: aserciones de floats con epsilon (`(x − 0.15).abs() < 1e-9`), nunca `assert_eq!` — `0.4 − 0.25` no da `0.15` exacto en binario.

3. **Pure-helper extraction es ahora la convención para testear internals de comandos Tauri (2ª instancia).** `snapshot_conversations` (Fase 1) → `outgoing_session_id` (Fase 2): lógica pura extraída del comando intesteable (sin feature `test` de tauri no se construyen `AppHandle`/`State`), unit-testeada directo, el comando solo la invoca. Complementa el split wrapper/core de los workers (4ª instancia: worker → resolver → extractor → episodic_writer). El próximo productor adopta ambos patrones sin re-derivarlos.

4. **Las restricciones de persistencia viven en la frontera de serialización, no en el builder puro.** El cap de 500 chars por item de `errors_encountered` se aplica en `run_episodic_writer_core` al serializar a JSON, NO en `build_trajectory` — el builder puro queda sin conocimiento del contrato de BD y el TrajectoryMiner (doc 8) puede envolverlo con caps distintos si los necesita. Regla general: builder puro = semántica del dominio; capa de persistencia = contratos de la BD.

### Lessons learned (Fase 3 — Job de mantenimiento)

1. **Gotcha de tokio: reloj pausado + `conn.call()` de tokio_rusqlite entre múltiples `advance()` = flakiness no determinística.** `#[tokio::test(start_paused = true)]` pausa el reloj de tokio, pero `tokio_rusqlite` ejecuta en un OS thread real: intercalar `conn.call()` entre varios `advance()` hace que el reloj pausado avance de más de forma no determinística (~90% flaky con 3 advances). Patrón correcto (ya presente en `access.rs`): colapsar al mínimo de `advance()` posibles SIN ningún `conn.call()` entre medio; verificar estado en BD solo antes del primer advance y después del último. Constraint general para todo timer test futuro de este codebase.

2. **Auditar las unidades reales de una columna compartida en sus WRITERS antes de escribir SQL nuevo que la consuma.** El explore descubrió que `last_accessed_at` se guarda en **milisegundos** (store_insert.rs, access.rs) pero `search.rs:83` lo trata como segundos en el decay — bug preexistente, FUERA de scope, anotado como change futuro `fix-decay-unit-mismatch` con comentario inline. El job nuevo no hereda el bug: computa `now_ms` en Rust y lo bindea (cero `strftime` en el SQL de democión). Sin esa auditoría, la democión habría degradado casi todo inmediatamente.

3. **Un timer de 24h puro está funcionalmente muerto en desktop — todo job periódico necesita decisión explícita de primera corrida.** Las sesiones típicas viven menos de 24h: solo-tick significa que el job nunca corre. Resolución: `interval_at(now + STARTUP_DELAY, period)` con delay de 5 min (no compite con el boot: descarga del embedder + ingesta inicial comparten el single-writer) y `startup_delay`/`period` inyectables — eso es lo que habilita el timer test. Cuando el doc 9 (autoDream) reemplace este scheduler, hereda la misma decisión.

4. **Fault tolerance por construcción es un veredicto de verify aceptable cuando la firma lo garantiza.** `run_maintenance_once` retorna `()` y maneja el error localmente con `if let Err { log::warn! }` — no puede propagar ni paniquear, el loop no puede morir. Verify lo clasificó WARNING (sin test de runtime) y el orquestador lo cerró como "verificado por construcción"; el test de conexión envenenada queda como mejora futura no bloqueante. Convención para jobs autónomos: retorno `()`, error local, nunca `?`/`unwrap` en el loop.

5. **Las ambigüedades de scope del doc de referencia se convierten en requirements explícitos testeados, no se resuelven en silencio.** El doc 7 dice que la democión aplica a "las 3 tablas", pero `demote_inactive`/el tiering activo hoy solo cubren `memory`. El spec lo convirtió en requirement explícito ("Bulk Demotion Scoped to `memory` Table") con un test que asevera que `episodic_trajectory`/`procedural_skill` quedan intactas, y la cobertura de las tablas hermanas quedó como change futuro. Misma disciplina que la lesson 1 de Fase 1: detectar el conflicto doc-vs-código al escribir el spec, no en verify.

### Lessons learned (Fase 4 — Comandos Tauri)

1. **Cuando el pseudo-código de "Interfaces" del design diverge del spec en firmas/strings, el SPEC es la fuente de verdad — y el design se enmienda en archive, no se ignora.** El design decía `format_tier1_coverage(bool, &[RankedHit])` y strings de retorno distintos; spec + tasks (2-de-3, con worked example) pinnearon `(bool, usize)` → el apply siguió al spec y verify lo ratificó (W1). El artefacto de design quedó enmendado en engram (sección "Amendments") para que el próximo lector no herede la firma stale. Complementa la lesson 19 del RAG: design gana sobre spec en *decisiones que leyeron el código*; spec gana sobre design en *contratos que los tests lockean*.

2. **Gates de "0 warnings" son insatisfacibles sobre un codebase con warnings preexistentes — el criterio se scopea a código nuevo/tocado desde el task breakdown.** Task 5.6 pedía `cargo clippy --all-targets → 0 warnings` con 114 warnings preexistentes fuera de scope; el criterio real (y satisfecho) es "0 warnings NUEVOS". Reformulado en archive (W2). Todo gate de calidad futuro se redacta relativo al diff, no al mundo.

3. **El fallo silencioso de `mem_save` sin `project` REINCIDIÓ pese a instrucción explícita en el prompt — la re-verificación del orquestador con `mem_search` scoped es la única red que funciona.** El design de esta fase se guardó fuera del scope del proyecto (mismo fallo que el spec de Fase 2); el orquestador lo detectó al re-verificar antes de lanzar tasks y lo re-persistió él mismo con el contenido recuperado por ID. Protocolo confirmado: (a) verificar cada artefacto con `mem_search(project:...)` ANTES de lanzar la fase dependiente, (b) si no aparece, `mem_get_observation` por ID directo — el contenido suele existir sin scope — y re-save desde el orquestador.

4. **La métrica de éxito que cruza dos subsistemas se loguea en el único call site que ve ambas señales, no en los primitivos.** "Tier 1 suficiente para el 80%" necesita workspace-presente (tier 1) + hits KNN (tier 2/3): `search.rs` no puede medirla solo (su filtro `tier > 1` le oculta el tier 1 por diseño). El log de cobertura vive en `prompt_builder.rs` (incondicional, `log::info!`), y el tier-por-hit de `search.rs` es solo diagnóstico (`log::debug!`). Generalización de la lesson 8 del RAG al dominio de métricas.

5. **APIs admin sobre tablas hermanas con PKs independientes exigen discriminador explícito (`kind`) — y un enum API-input nuevo antes que mutar el serde de un enum interno.** Los ids colisionan entre las 3 tablas físicas → `kind` obligatorio en `set_importance`/`delete_memory`, con test de colisión que asevera que la tabla hermana queda intacta. Existía `TableKind` (PascalCase, usado en `RankedHit`), pero cambiarle el serde rippearía a retrieval: se creó `MemoryKind` lowercase como concepto API-input separado. Unificación = change futuro, no "fix de paso".

**Changes futuros anotados:** test del combo `kind=None + memory_type` (S1), unificación `MemoryKind`/`TableKind`, filtro `agent_id` en `list_memories`, y los ya pendientes `fix-decay-unit-mismatch` y democión de tablas hermanas.
