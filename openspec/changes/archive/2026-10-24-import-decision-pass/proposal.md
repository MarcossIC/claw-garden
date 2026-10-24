# import-decision-pass — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | import-linear (81) | ver [`state.yaml`](state.yaml) |

## Problema

Verificado en la [exploración](exploration.md#problema-verificado-contra-el-código). La pasada que decide dónde va cada fila de un import es **un solo** `conn.call` sobre la única conexión del daemon. Adentro abre una transacción, inserta las filas en `memory` y la suelta sin confirmar (`crates/clawg-core/src/memory/portability/import.rs:972-983`). Con el índice de 81 la pasada es lineal, pero sigue durando lo que dura el archivo: ~1,1 ms por fila en debug, unos 57 s para 52.080 filas, y un archivo puede llegar a `MAX_IMPORT_BYTES` (`import.rs:51`). Mientras tanto, todo lo que usa la conexión espera detrás. Eso incluye un save, una búsqueda, el bloque `ingest` de `/health` y el hook `session-start`, que tiene 3 s (`crates/clawg-cli/src/hook.rs:26`).

De esa pasada sólo sale el `Ledger`: dónde cae cada fila, qué enlaces se pierden y qué proyectos ya tomó otra raíz. Lo que escribe se tira.

## Objetivo

1: guardar y clasificar sin frenar lo demás. Ninguna llamada de la pasada de decisión dura más que una fracción del plazo del hook, sea cual sea el tamaño del archivo, y el reporte no cambia en nada.

## Decisiones

Cierran las [decisiones abiertas](exploration.md#decisiones-abiertas) con la opción que la exploración recomienda en cada una.

1. **Un asiento, dos implementaciones.** `find`, `present_at`, `place`, `place_rows` y `write_links` se escriben sobre un trait (`Seat`), con sus lecturas (`origin_rows`, `present`, `twin`, `holders`, `linked`) y sus escrituras (`insert`, `revise`, `link`). La escritura real usa `SqlSeat` sobre su transacción, y la pasada de decisión usa `Planned`, en memoria. Las reglas quedan escritas una vez. Una prueba diferencial compara las dos implementaciones sobre una tabla de escenarios.
2. **La carga va en lotes por índice.** `Planned` se arma con búsquedas puntuales en llamadas de a `LOAD_BATCH` (256) búsquedas cada una:
   - las filas de cada origen del archivo, por `idx_memory_import_origin`;
   - las de cada par proyecto y texto, por `idx_memory_dedup`;
   - las vivas de cada par proyecto y clave, por `idx_memory_topic`;
   - las piezas de las filas cargadas, por `idx_memory_piece_memory`;
   - las raíces de `project`;
   - y el próximo id de `memory`.

   Ninguna llamada depende del tamaño del store. Los proyectos y las claves a buscar salen del archivo y de las filas de origen ya cargadas, que son los únicos que las reglas pueden pedir.
3. **El umbral.** En la prueba de intercalado, ninguna sonda espera más de 300 ms (la décima parte del plazo del hook) durante la corrida en seco de 10.000 notas, que hoy tiene la conexión varios segundos. En el banco de 15.047 filas, la pasada de decisión baja a menos de la décima parte de los 16,4 s de hoy con el índice; se mide al verificar.
4. **Parada.** El planificador corre en `spawn_blocking` y mira, entre lote y lote de escritura (`note_batches`), un `AtomicBool` que un guard pone en verdadero cuando se suelta el futuro del import. Así, un apagado que corta el pedido no espera el plan entero.
5. **Empate de `created_at` en `TWIN_SQL`.** Se agrega `id` al `ORDER BY`, así el SQL y el planificador eligen la misma fila por construcción. El índice sigue sirviendo el orden: lleva el `rowid` detrás.
6. **C (otra conexión) no se escribe.** Queda en la exploración como descartada. Si la carga resultara larga al medirla, el remedio es achicar el lote, no otra conexión.

Dos RED de la exploración no se escriben:
- **El hook por el binario (RED 2).** Pide armar en cada corrida un archivo de miles de notas por el binario de debug. El hook es otro cliente de la misma cola, y la sonda de la prueba de intercalado mide justo esa espera.
- **Sin WAL (RED 6).** Sin una llamada larga en la pasada de decisión, el modo del diario no cambia nada en ella. La escritura, que es la que confirma, no cambia en este cambio.

## Alcance

1. `Seat`, `SqlSeat` y `Planned` en `import.rs`, con `find`, `present_at`, `place`, `place_rows` y `write_links` genéricos sobre el asiento. `record_projects` se parte en la decisión pura (`taken_projects`) y sus `INSERT`.
2. La carga de `Planned` en llamadas cortas, y la decisión en `spawn_blocking` con el indicador de parada.
3. `id` en el `ORDER BY` de `TWIN_SQL`, y las consultas de carga en la prueba de planes (`the_import_lookups_seek_an_index`).
4. De paso, `write_links` toma la cabecera de cada pieza de su propia línea en vez de buscarla en todas las piezas del archivo, una búsqueda que crece con el cuadrado de las piezas.
5. El párrafo de cabecera de `import.rs`, el delta de `specs/portability` y `.claude/CLAUDE.md`, si nombra la pasada.

### No-alcance

Lo que la [exploración](exploration.md#no-alcance) deja fuera: la escritura, los embeddings, el esquema, una conexión de lectura propia y deshacer un import. El reporte no cambia ni un byte, y tampoco el ranking.

## Cómo se mide

Tests (la lista está en la [spec](spec.md)):

1. **Intercalado:** una sonda `SELECT 1` en la conexión durante la corrida en seco de 10.000 notas nunca espera más de 300 ms. Hoy falla, porque la sonda espera la pasada entera.
2. **Equivalencia:** el `Ledger` de `Planned` es igual al de la pasada SQL en cada escenario de la tabla. Hoy no compila, porque el planificador no existe.
3. **Ids después de una purga:** con `sqlite_sequence` por encima de `MAX(id)`, la corrida en seco reporta lo mismo que la real. Es una guarda: hoy pasa, y falla con un planificador que parta de `MAX(id)`.
4. **Sin escritura:** después de la corrida en seco, `memory`, `project`, `project_remote`, `memory_fts` y `sqlite_sequence` siguen iguales.
5. **Parada:** con el indicador puesto, el planificador termina sin decidir.
6. El oráculo de la exploración (las pruebas de import de `import.rs`, `tests/api.rs` y `tests/binary.rs`) pasa sin editar ninguna.

Medición: el banco de 81 (`twice_the_notes_import_in_at_most_three_times_the_time`) y la espera máxima de la sonda antes y después, anotados en `verify.md`.

## Migración y vuelta atrás

No hay esquema nuevo ni datos que cambien. La escritura queda igual, y el reporte también. Volver es revertir los commits.
