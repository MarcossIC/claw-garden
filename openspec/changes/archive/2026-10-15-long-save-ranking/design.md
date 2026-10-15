# long-save-ranking — Diseño

## D1. Bajar el tope, no cambiar la fórmula

El problema es la unidad que se embebe, no cómo se puntúa: el pedazo con la respuesta ya es el mejor de su save, y lo que lo hunde es que su vector promedia 424 tokens de relleno ([exploración](exploration.md#main-tal-cual)).

| Alternativa | Por qué no |
|---|---|
| Ventanas en una tabla aparte (V12) | Mismo ranking esperado, con migración, rollback, re-embed de ventanas y un brazo KNN más. Queda como camino si las filas de más resultan un problema |
| Bono por save (máximo o suma de hermanos) | Los hermanos no tienen la respuesta: sube el save, no el pedazo correcto. No hay de dónde sacar el peso |
| Reranker que embebe oraciones al buscar | Un embebido por oración candidata en cada búsqueda; en CPU multiplica la latencia por decenas |

**96 con 24 de solapamiento.** Del barrido: 192/64 halló 11 de 12, 128/32 también 11, 128/64 y 96/24 los 12; 96/24 dejó la respuesta visible en `search_memory` en 11 y en el contexto en 11. El solapamiento pasa de un 15 % del pedazo (64 de 424) a un 25 % (24 de 96): un hecho de una frase ocupa más de un pedazo chico, y 24 tokens alcanzan para una frase corta cortada al medio. 128/32 y 96/24 tienen la misma proporción; 128/64 halló los 12 pero guardó más filas que 96/24.

Con 96 tokens de pedazo y 48 de cabecera, el texto embebido es un tercio cabecera. Eso es lo que hace que la clave del save llegue al vector de cada pedazo; medido, no empeora ningún otro subconjunto.

`PIECE_OVERLAP_TOKENS` es una constante nueva porque `CHUNK_OVERLAP_TOKENS` sigue gobernando `ChatChunker::chunk`, que no se toca.

## D2. El pozo cuenta saves dentro de `knn_one`

Con pedazos de 96, un save de 5.000 tokens tiene unos 70 pedazos que comparten cabecera, así que una consulta por su tema puede traer 30 hermanos como los 30 vecinos. El colapso (`collapse_saves`, `search.rs:104`) corre después, sobre el pozo ya cortado, y no tiene con qué rellenar.

- **Dónde:** dentro de `knn_one`, en la misma llamada a la conexión que ya hace la sonda de elegibles y la segunda pasada. Los vínculos de las filas de la ventana se leen con la misma consulta que `save_links` (`pieces.rs:206-232`), extraída a una función síncrona sobre `&rusqlite::Connection` que usan las dos.
- **Cuándo se amplía:** una sola condición de segunda pasada, `faltan elegibles || faltan saves`. Faltan saves cuando la ventana volvió llena y `distinct_saves(hits, links) < k`. Se mantiene el máximo de dos pasadas por tabla.
- **Cómo se recorta:** `trim_to_saves(hits, links, k)`: en orden de puntaje, las filas hasta haber visto `k` saves distintos. Los hermanos que quedan dentro se los saca `collapse_saves` después, como hoy.
- **Sólo `memory`:** las otras dos tablas no tienen pedazos; su rama no lee vínculos.
- **Si leer vínculos falla:** se cuenta por filas, como hoy, y el colapso de después tampoco tendrá vínculos (falla igual): la búsqueda devuelve lo de antes.

**Alternativa descartada:** colapsar dentro del SQL de KNN. vec0 corta en `k` antes de cualquier JOIN o GROUP BY (`search.rs:336-390`), así que agrupar ahí no recupera nada que la ventana no trajo.

## D3. El generador del eval fija su propio tope

`golden-mine synth` alarga cada fila `long` hasta que `eval_api::fallback_pieces` (`crates/clawg-core/src/eval_api.rs:67`) da 4 pedazos, y `fallback_pieces` usa el tope de producción. Con 96, las filas saldrían de unos 1.000 bytes, entrarían en un pedazo de 480 y dejarían de ser largas.

`fallback_pieces` pasa a recibir el tope con el que contar y corta como `ChatChunker::chunk`, con el solapamiento de 64 de un chunk. El generador le pasa 424 como constante propia (`LONG_PIECE_TOKENS`), nombrada por lo que significa: el largo con el que se definió el set. Con 424/64 es exactamente el corte que hacía `split_save` antes de este cambio. Las filas `long` quedan idénticas a las de hoy (R4), y la punta y la base miden el mismo set.

## D4. Sin cambios de datos

No hay migración. Los saves partidos con 424 conservan filas, vínculos y vectores; `join_pieces` no depende del tope porque busca el solapamiento en el texto. `reembed_all` re-embebe cada fila con el contexto de su vínculo, sea cual sea su largo.
