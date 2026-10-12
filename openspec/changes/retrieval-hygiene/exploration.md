# retrieval-hygiene — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — | exploración (borrador) |

**Tesis:** dos defectos chicos del ranking, que el análisis no menciona, ensucian lo que el agente recibe. Los dos se pueden medir con `rag-eval --agent-view`.

## Problema 1: inflación de accesos (verificado)

- El handler `search` del daemon registra en el access tracker **todos** los hits que devuelve (`crates/clawg-cli/src/daemon/api.rs:496-501`).
- Lo hace **antes** del corte relativo del puente MCP. Ese corte se queda con `score >= 0,6 × mejor` y muestra mínimo 2 (`crates/clawg-cli/src/mcp/tools.rs:65-76`).
- Resultado: filas que el agente nunca vio reciben `access_count + 1` y un `last_accessed_at` nuevo. Eso resetea el decaimiento, porque `t` se mide desde el último acceso (`crates/clawg-core/src/memory/score.rs:250-289`).
- Además, pueden promover de tier (`crates/clawg-core/src/memory/access.rs:160`). `architectural_decision` y `session_summary` suben a tier 1 con 3 accesos.
- Es un bucle "el rico se hace más rico".

**Contexto de una decisión previa:** el corte relativo se puso en el puente, no en el core, a propósito. Así el ranking, R@10 de `rag-eval` y el visor no cambian. Este arreglo tiene que respetar esa decisión o revisarla de forma explícita.

## Problema 2: empates en score 0 ordenados por antigüedad (verificado)

- Por debajo del piso de coseno 0,8793, el score queda en 0 exacto (`score.rs:201-203`).
- Los empates se rompen por `(kind, id ASC)` (`crates/clawg-core/src/memory/search.rs:78-84`). Entre ceros, sale primero la memoria más vieja: es ruido.
- En el puente, si `mejor == 0`, la condición `0 >= 0` deja pasar todo hasta `limit`.
- `recall_context` no tiene corte relativo: rellena hasta `K_FINAL = 10` con ceros si faltan candidatos positivos (`crates/clawg-core/src/memory/prompt_builder.rs:130-158`).

## Enfoques

**Accesos**

| Opción | A favor | En contra |
|---|---|---|
| a. El puente informa al daemon qué ids mostró (param o segunda llamada) | El daemon registra exactamente lo que el agente vio | Una llamada extra, o estado entre llamadas |
| b. El daemon acepta `min_relative_score` y aplica el corte él mismo | Un solo punto de verdad. El corte sigue siendo opcional, así que no afecta a rag-eval | Mueve al core una decisión que antes se dejó a propósito en el puente |
| c. Registrar sólo los primeros N | Trivial | Aproximado: N no es lo que realmente se mostró |

**Empates en 0**

| Opción | A favor | En contra |
|---|---|---|
| a. Desempatar por distancia cruda (el coseno sin reescalar) | El orden vuelve a tener sentido semántico sin cambiar ningún score | Sigue mostrando filas irrelevantes |
| b. No mostrar filas con score 0 y decir "sin memorias relevantes" | Honesto con el agente | Puede ocultar un match léxico. Interactúa con lexical-fusion |
| c. a + b: desempatar en el core y filtrar en el puente | Las dos mejoras | Dos lugares que tocar |

## Decisiones abiertas

- [ ] Accesos: ¿a, b o c?
- [ ] Ceros: ¿a, b o c? Ojo: si se elige b antes de lexical-fusion, los matches léxicos bajo el piso desaparecen del todo.

## Cómo se mide

- **Tests unitarios:** un hit cortado por relevancia no incrementa `access_count`, y el orden entre ceros sigue la distancia.
- **`rag-eval --real --agent-view` con `--diff` contra el baseline:**
  - hit rate natural 0,891;
  - tokens por respuesta;
  - sin casos perdidos.
