# eval-golden-expansion — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (habilita 1, 2, 3 y 4) | M | — | exploración (borrador) |

**Tesis:** el golden set actual no puede ver ni el canal léxico ni la visibilidad. Sin ampliarlo, lexical-fusion y visibility-policy no tienen con qué probar que mejoran algo.

## Problema (verificado)

**Composición de `docs/eval/golden.json`** (gitignored, está en disco):

- 46 casos, cada uno con 1 sola referencia esperada.
- 48 filas de corpus.
- Importancia sólo entre 3 y 7; tier NULL.
- 26 en español y 22 en inglés.
- 28 pares de contradicción.
- Las 46 consultas son paráfrasis en lenguaje natural. **No hay ninguna** por identificador, símbolo o código de error.

**El harness es ciego al canal léxico.** Las corridas `docs/eval/results/2026-08-17-real-{lexoff,lex20,budget-lex1/5/512,tok-trigram-*}` dan métricas **idénticas**: R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979, nDCG 0,6693. Con 48 filas y un pool KNN de 30, casi nada es candidato sólo léxico.

**Variantes que ya existen**, sobre el mismo corpus:

| Variante | Consultas | Hit rate (agent-view) |
|---|---|---|
| `golden-keywords.json` | palabras clave | 0,826 |
| `golden-oracle.json` | términos tomados de la respuesta; 12/46 con identificadores (`countryId`, `getEstadoLabel`) | 1,0 |
| natural | lenguaje natural | 0,891 |

**Una sola referencia por caso** vuelve degenerado al nDCG.

**No hay casos** multi-proyecto, multi-agente ni de importancia 8 a 10. Las `user_preference` reales tienen importancia 9–10, una zona que el sweep nunca vio.

**No hay eje de fusión en el harness.** `search_all_memories_with_scoring` sólo recibe un `ScoringConfig`, sin opción para elegir el método de fusión (`crates/clawg-core/src/memory/search.rs`, `crates/clawg-core/src/eval_api.rs`).

**`docs/eval/README.md` está desactualizado:** dice 13 casos y menciona el filtro `tier>1`.

## Alcance candidato

1. **Corpus distractor** de cientos de filas, para que la respuesta esperada pueda caer fuera del top-30 denso.
2. **Subconjunto léxico:** consultas por identificador, nombre de función y código de error.
3. **Casos con más de una referencia relevante.**
4. **Filas de importancia 8 a 10 y de tier 1.**
5. **Filas de varios proyectos y agentes**, con la visibilidad esperada por caso.
6. **Eje de fusión en `rag-eval`** (`--fusion current|rrf|...`), para que lexical-fusion pueda hacer A/B.
7. **Reportar las métricas por subconjunto**: natural, léxico y visibilidad.

## Decisiones abiertas

- [ ] **De dónde sale el corpus distractor:**
  - sintético;
  - minado del store real con `golden-mine`, lo que requiere autorización para leer `~/.clawg` y revisar privacidad;
  - una mezcla de los dos.
- [ ] **Versionado:** ¿se versiona el golden set? Hoy `docs/eval/**` está en `.gitignore` por decisión explícita.
- [ ] **Umbrales pre-comprometidos** que tendrá que superar lexical-fusion (por ejemplo, R@5 del subconjunto léxico, sin regresión en el natural).

## Cómo se mide

- **El criterio de salida es que el baseline actual muestre una brecha** en el subconjunto léxico. Si el scorer actual ya acierta todo ahí, lexical-fusion no tiene nada que demostrar, y eso también es un resultado.
- **Las métricas del subconjunto natural** quedan igual que el baseline actual: el corpus nuevo se agrega, no reemplaza.
