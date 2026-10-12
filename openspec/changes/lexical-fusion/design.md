# lexical-fusion — Diseño

## D1. Bono aditivo por identificador exacto, no RRF

| Opción | Por qué no / por qué sí |
|---|---|
| a. RRF ponderado × decaimiento × importancia | Reemplaza `cos'` por rangos en **todas** las consultas. Pierde la magnitud del coseno, con la que se calibraron el piso 0,8793 y el rango de importancia 0,85–1,0 (`score.rs:139-172`), y cambia el ranking de las 46 consultas naturales sin que ninguna lo necesite. Con 0 a 3 hits FTS, cada uno se lleva el bono completo de `1/61`: un match de una palabra común le gana a un buen match denso. |
| c. BM25 normalizado + `cos'` | BM25 no tiene escala fija entre consultas; normalizarlo por consulta (min-max, división por el máximo) premia al mejor match aunque sea malo. Mismo problema que a con pocos hits. |
| b. Bypass del piso para matches fuertes | Apunta al caso real, pero un bypass sólo levanta la fila hasta el piso: no le gana a los hermanos que ya están arriba. |
| **b', bono aditivo `cos' + w` sólo para matches de identificador** | **Elegida.** Toca únicamente las consultas con un identificador que discrimina. Para el resto el código no suma nada, así que el ranking natural queda igual bit a bit, y eso se prueba con un test en lugar de medirse. |

`w = 1,0`: un identificador exacto vale lo que un coseno perfecto. Como `cos' ∈ [0, 1]`, con el mismo decaimiento e importancia un match siempre queda arriba de un no-match. Decaimiento e importancia siguen multiplicando: un match de tier 3 viejo (piso 0,05) no le gana a un hit denso fresco. Es la composición que la exploración pedía para cualquier fusión ("multiplicado por decaimiento × importancia").

## D2. Qué es un identificador, y por qué los números solos no

La señal que el coseno no ve es el **nombre exacto**: el embedder trata `resolveLedger4182` y `resolveLedger4812` casi igual. Las formas que se escriben como nombres son las de la definición de la spec: letras con dígitos, `_`, o una joroba camelCase.

- Un número solo (`2026`, `3`) aparece en fechas, versiones y cantidades. Con bono fijo, "qué decidimos en 2026" subiría todas las filas con ese año.
- Una palabra común nunca es identificador, así que una consulta en lenguaje natural no corre ninguna consulta extra (R4).

Falsos positivos posibles: `iPhone`, `McDonald`, `v2`. Son nombres propios o versiones; si discriminan, el bono es correcto; si no, los cubre D3.

## D3. Gate por presupuesto: el identificador tiene que discriminar

La consulta de matches pide `LIMIT presupuesto + 1`. Si vuelven más filas que el presupuesto, no hay bono para ninguna.

- Un bono uniforme sobre 200 filas que comparten `clawg_core` no ordena nada útil y desplaza a los mejores hits densos.
- Quedarse con las primeras 20 por BM25 sería arbitrario: entre filas que contienen el mismo identificador, BM25 sólo mide largo y repetición.
- Se reusa el presupuesto léxico (20) en lugar de un número nuevo: es lo que el canal ya está dispuesto a puntuar.

Alternativa descartada: ponderar el bono por IDF. Agrega una escala continua que habría que calibrar con mediciones que este entorno no puede hacer, y el gate binario ya separa "nombra una cosa" de "nombra muchas".

## D4. `AND` entre identificadores

Una fila es match si tiene **todos** los identificadores de la consulta. Con uno solo (el caso común) da igual. Con dos, una fila que tiene uno no recibe nada: queda como hoy. Un bono parcial (por fracción de identificadores) queda fuera de alcance hasta que el golden set tenga casos con varios identificadores.

## D5. La consulta de matches corre antes que las ramas densas

El bono tiene que estar en el score **antes** del recorte a k dentro de `knn_one` (`search.rs:432-433`), porque ahí se decide qué filas densas sobreviven una ventana ensanchada. Por eso:

- `identifier_matches` corre antes del `try_join!` y devuelve un `HashSet<i64>` de ids de `memory`.
- `knn_one` y `score_lexical_candidates` marcan `RawHit.lexical_match` al mapear cada fila, y `score()` sigue siendo una función pura de `RawHit`.
- No agrega latencia real: `tokio_rusqlite` serializa todo en una sola conexión (`MemoryStore.conn`), así que las ramas del `try_join!` ya corren una detrás de otra en el hilo de la base.
- Episodic y procedural no tienen FTS; sus filas nunca se marcan. Los ids son por tabla, así que el marcado se aplica sólo a filas `TableKind::Memory`.

## D6. Los matches también son candidatos

Los ids de `identifier_matches` se suman a los de `lexical_candidates` antes de restar los densos. Así un identificador que el KNN no trae (fuera del top 30, o pasado el token 16 del OR) igual se puntúa. El total queda acotado a dos veces el presupuesto, y el presupuesto ya está acotado a `MAX_LEXICAL_CANDIDATES` (512, `search.rs:582`), muy por debajo del límite de parámetros de SQLite.

## D7. Medición

El ambiente de este cambio no tiene el golden set (`docs/eval/` está fuera de git) ni el modelo en caché. La medición `--real` de la propuesta corre en la máquina del usuario, contra los resultados guardados del baseline de eval-golden-expansion. No se agrega un eje de sweep: si un umbral falla, el siguiente paso es ajustar `w` o el gate, y el eje se agrega entonces.
