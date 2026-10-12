# lexical-fusion — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 3 | M | eval-golden-expansion | exploración (borrador) |

**Tesis:** el canal léxico sólo **propone** candidatos. Después los puntúa el coseno, y un match exacto por identificador bajo el piso queda en 0. Fusionar es la dirección razonable, pero hoy no hay cómo demostrar que mejora algo.

## Problema (verificado)

### Candidatos

- Por defecto: KNN de memory 30, episodic 5, procedural 5, y léxico FTS 20 (`crates/clawg-core/src/memory/search.rs:32-39`).
- FTS5 existe sólo para `memory` (`crates/clawg-core/migrations/V8__memory_fts.sql`).

### Consulta léxica

- Hasta 16 tokens entre comillas, unidos con OR, tomados del **principio** de la query (`search.rs:482, 502`). Si la query es larga, un identificador al final se pierde.
- El tokenizer es `unicode61 remove_diacritics 2`. `getEstadoLabel` queda como un solo token, sin partir camelCase.

### Scoring

- BM25 sólo decide **cuáles** 20 entran (`search.rs:550-556`, `ORDER BY bm25(memory_fts) LIMIT`).
- Después se descarta: el candidato léxico se puntúa con el mismo coseno que el denso (`search.rs:711-721, 822-834`).
- No hay fusión, ni bonus para lo que aparece en los dos canales.

### Piso

- Por debajo de 0,8793 el score es 0 (`crates/clawg-core/src/memory/score.rs:201-203`).
- En `search_memory`, una fila en 0 sólo llega al agente si queda en las posiciones 0–1 o si el mejor score también es 0. Ver retrieval-hygiene.

### No medible hoy

- Todas las corridas `lex*` y `tok-trigram-*` dieron métricas idénticas. Ver eval-golden-expansion.

## El RRF del ejemplo de sqlite-vec no alcanza

El ejemplo oficial de búsqueda híbrida de sqlite-vec (CTE con `row_number() OVER (ORDER BY distance)` sobre KNN, y `rrf_k = 60`) funciona, pero copiado tal cual le faltan cosas:

- **Filtros.** No aplica estado, `deleted_at`, agente ni proyecto. Y si los agrega en el JOIN, quedan después del corte k (el mismo crowding que describe visibility-policy).
- **Cobertura.** Sólo cubre `memory`. Las otras tablas no tienen FTS.
- **Composición con el resto del score.** No define cómo se combina con el decaimiento, la importancia y el tier. ¿El RRF reemplaza a `cos'` dentro del producto?
- **Pocos hits léxicos.** RRF usa rangos, no magnitudes. Con 0 a 3 hits FTS, cada uno se lleva el bonus completo de 1/61, y un match léxico irrelevante puede ganarle a un buen match denso. Hace falta un gate o un peso léxico menor.

## Enfoques

| Opción | A favor | En contra |
|---|---|---|
| a. RRF ponderado, multiplicado por decaimiento × importancia | Estándar, estable entre escalas | Pierde la magnitud. Hay que re-ajustar los pesos y quizás el piso |
| b. Bypass del piso para matches léxicos fuertes (por ejemplo, rango BM25 1 con todos los tokens tipo identificador presentes) | Cambio mínimo, apunta justo al caso del identificador | Heurístico. Hay que definir qué es "fuerte" |
| c. Combinación lineal de BM25 normalizado y `cos'` | Conserva la magnitud | La normalización de BM25 es frágil entre consultas |
| d. Tokenizer de identificadores (partir camelCase y snake_case) | Mejora la recuperación léxica en sí | No cambia el scoring. Hay que combinarlo con a, b o c |

## Decisiones abiertas

- [ ] Elegir entre a, b o c, una vez que el golden set ampliado muestre la brecha.
- [ ] ¿Se mueve el piso de coseno? Hoy 0,8793 está atado a los prefijos e5 simétricos (`"query: "` en ambos lados, `crates/clawg-core/src/memory/embedder.rs:109`).
- [ ] ¿Va a haber FTS para otras tablas? Depende de knowledge-synthesis.

## Cómo se mide

- **`rag-eval --diff` pareado contra el baseline**, con un umbral pre-comprometido:
  - el subconjunto léxico sube R@5 al menos lo comprometido;
  - el subconjunto natural no pierde ningún caso;
  - `importance_rank_dominance` no vuelve a subir.
- **Que no se haya sacrificado otra cosa:** `--agent-view` muestra el hit rate y los tokens.
