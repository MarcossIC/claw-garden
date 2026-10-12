# eval-golden-expansion — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (habilita 1 y 3; 2 y 4 quedan para visibility-policy) | M | — | propuesta |

## Problema

El golden set no puede ver el canal léxico, y lexical-fusion no tiene con qué demostrar nada (detalle en `exploration.md`).

- 48 filas para un pool KNN de 30 (`crates/clawg-eval/src/bin/rag-eval/main.rs:36`): casi nada es candidato sólo léxico. Las corridas `lex*` y `tok-trigram-*` dan métricas idénticas.
- Ninguna consulta por identificador; ningún caso con más de una referencia (verificado sobre `docs/eval/golden.json`). Las métricas ya soportan varias (`metrics.rs:20-31`), así que el nDCG degenerado es un problema de datos.
- Importancia entre 3 y 7. `importance >= 9` cae en tier 1 (`crates/clawg-core/src/memory/tier.rs:28-30`), una zona que el set no cubre.
- `report::tests::a_committed_results_file_without_the_block_still_diffs_against_one_with_it` (`report.rs:851-853`) lee `docs/eval/results/2026-08-15-fake.json`. Ese archivo está gitignored y **tampoco existe en este disco**, así que el test falla en todas las máquinas.
- Corrección a la exploración: `docs/eval/README.md` no existe en disco.

## Alcance

1. **Generador sintético determinista** en código versionado (`crates/clawg-eval`). Recibe `--seed` y el tamaño, y escribe en `docs/eval/`. Va en el binario de autoría (`golden-mine`) y no en `rag-eval`: el instrumento no construye su propia vara (`crates/clawg-eval/Cargo.toml:21-23`). Produce:
   - **Distractores** (500 por defecto), cercanos en tema a las filas reales, para que la esperada pueda caer fuera del top-30 denso.
   - **Subconjunto léxico** de 30 casos o más: consulta por identificador, función o código de error, con hermanos casi idénticos que sólo difieren en ese identificador.
   - **10 casos o más con varias referencias.**
   - **20 filas o más con importancia entre 8 y 10**, y al menos 10 con 9 o más (tier 1).
2. **Etiqueta de subconjunto por caso.** Es opcional y vale `natural` por defecto: el `golden.json` actual sigue parseando con `deny_unknown_fields` (`golden.rs:53-71`).
3. **Métricas por subconjunto en el reporte**, junto a las de `by_lang` (`report.rs:348`).
4. **Arreglar el test de `report.rs:851`** con un fixture inline. Entra porque la suite en verde es criterio de éxito de cada tarea.

## Fuera de alcance

- **Minar el store real** (`golden-mine` sobre `~/.clawg`). Decisión del usuario.
- **Versionar el golden set.** `docs/eval/**` sigue en `.gitignore`.
- **Visibilidad multi-proyecto y multi-agente** (ítem 5): pasa a visibility-policy. `EvalStore::insert` escribe siempre `scope "agent"` y `project_id: None` (`crates/clawg-core/src/eval_api.rs:266-268`), y `EvalScope` nunca lleva proyecto (`:85-93`). No hay política de visibilidad en el código: el predicado es `agent_id = ?3` (`crates/clawg-core/src/memory/search.rs:121-122`). Sin política no se puede escribir la "visibilidad esperada" de un caso.
- **Eje `--fusion` en `rag-eval`** (ítem 6): pasa a lexical-fusion. `search_all_memories_with_scoring` no tiene parámetro de fusión (`search.rs:1018-1026`), y un enum con una sola variante sería una abstracción sin uso. Lo agrega lexical-fusion junto con su segunda variante.

## Reproducibilidad sin versionar datos

- La fuente es el código del generador más la semilla.
- Misma semilla y mismo commit producen un archivo idéntico byte a byte. `rag-eval` ya registra el sha256 del golden que mide (`report.rs:149-153`), y con eso se comparan dos máquinas.
- La parte natural (`golden.json`) sigue siendo local por decisión del usuario. Por eso el criterio de lexical-fusion se mide sobre el subconjunto léxico, que es 100 % sintético.

## Éxito

- **Tests que hoy fallan:**
  - el generador con la misma semilla produce la misma salida, y con otra semilla, otra;
  - un caso sin etiqueta parsea como `natural`;
  - el reporte trae un bloque por subconjunto;
  - el test de `report.rs:851` pasa sin `docs/eval`.
- **Regresión:** con el `golden.json` actual sin sintéticos, `rag-eval --real` reproduce exactamente R@5 0,7826 y MRR 0,5979.
- **Criterio de salida (exploración):** sobre el corpus ampliado, el scorer actual da un R@5 léxico al menos 10 puntos por debajo del R@5 natural. Si no hay brecha, se registra como resultado válido y lexical-fusion se re-evalúa antes de proponerse.
- **Baseline nuevo:** las métricas por subconjunto se anotan en `verify.md`, que sí se versiona.

## Umbrales de lexical-fusion (congelados desde ahora)

Contra el baseline nuevo `B`, con el mismo golden y la misma semilla, y medidos con `rag-eval --diff` pareado:

| Subconjunto | Regla |
|---|---|
| Léxico | R@5 ≥ B + min(0,15; (1 − B)/2), y MRR ≥ B + 0,10 |
| Natural | Ningún caso pasa de acierto a fallo en R@5, y el MRR no baja más de 0,02 |
| Global | `importance_rank_dominance` no sube |

No se ajustan después de ver los resultados de lexical-fusion.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Los distractores de plantilla se agrupan entre sí y no compiten con las filas reales | Media | El criterio de salida lo detecta. El design define la cercanía temática |
| Medir con `--real` lee el modelo desde el perfil dev (`main.rs:67-68`) | Alta | Pedir autorización antes de medir; los tests usan `--fake` |
| El subconjunto natural cambia de número al crecer el corpus | Alta | Esperado: se documenta como baseline nuevo y la regresión sin sintéticos lo ancla |

## Rollback

Revertir los commits. No toca el esquema ni datos guardados. Los archivos generados en `docs/eval/` se borran.

## Nota de revisión (2026-10-12)

La medición de la tarea 10.3 cambió el alcance. Detalle en `design.md` (revisión, D7 a D10).

- **Subconjuntos saturados.** `lexical`, `multi_ref` y `high_importance` dan R@5 1,0. Además, el canal léxico no cambia ningún ranking: con el scorer actual, un candidato sólo léxico se re-puntúa por coseno.
- **Lo que se agrega:**
  - un quinto subconjunto, `low_importance`;
  - la métrica `top_intruders` en `--audit`.
- **Cambia el criterio de salida.** "Léxico 10 puntos bajo el natural" pasa a ser C1 a C5 (spec, R8), absolutos y falsables.
- **Los umbrales de lexical-fusion no cambian.** Se aplican sobre el baseline nuevo `B` que fije la tarea 18.4.
