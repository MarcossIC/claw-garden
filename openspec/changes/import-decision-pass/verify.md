# import-decision-pass — Verificación

## Linux (2026-10-24)

La suite (`cargo test --workspace --locked`) y los dos clippy de `.claude/CLAUDE.md` pasan: el del workspace con `--all-targets` y el de `-p clawg-cli` sin `testing`. Se enlazó ONNX Runtime 1.29 en dinámico (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK`) porque la descarga de `ort-sys` no estaba disponible. Ningún test carga el modelo real.

- **ID1, la conexión:** con `a_dry_run_leaves_the_connection_to_other_calls` (10.000 notas de una fila, corrida en seco, sonda `SELECT 1` cada 5 ms):

  | | Espera más larga de la sonda |
  |---|---|
  | Antes (`5e6ee21`, la pasada en un `conn.call`) | 1,34 s y 1,44 s, en dos corridas |
  | Después | entre 0,67 ms y 0,94 ms, en tres corridas |

  El test falla con el código de antes y pasa con el de después, con el tope en 300 ms.
- **ID2, equivalencia:** `the_planner_decides_as_the_sql_pass` compara el `Ledger` en los 16 casos de la tabla de la spec y da igual en todos. `a_dry_run_after_a_purge_reports_the_ids_of_the_import` pasa antes y después. El oráculo de import de `import.rs`, `tests/api.rs` y `tests/binary.rs` pasa sin editar ninguna prueba.
- **ID3, la parada:** `a_stopped_planner_decides_nothing`.
- **ID4, los índices:** `the_import_lookups_seek_an_index`, con las cuatro consultas de carga y `TWIN_SQL` con `id` en su orden.

### La pasada de decisión, medida sola

Es `write` con `dry_run` sobre 15.047 notas de una fila en un store vacío, en debug, con un test temporal que no se commiteó:

| | Tiempo | Llamada más larga a la conexión |
|---|---|---|
| Antes | 2,12 s y 2,10 s | la pasada entera, en una llamada |
| Después | 0,32 s y 0,34 s, carga y decisión juntas | unos milisegundos (`LOAD_BATCH`) |

**El umbral de la propuesta no se cumple tal como quedó escrito.** Pedía bajar a menos de la décima parte de «los 16,4 s de hoy con el índice», pero ese número era el de un import entero del banco de 81, no el de la pasada de decisión sola. Contra la pasada medida acá, la reducción es de 6,4 veces. El criterio que el cambio persigue, que ninguna llamada de la pasada ocupe la conexión más que una fracción del plazo del hook, se cumple por tres órdenes de magnitud (ID1).

### El banco de 81

`twice_the_notes_import_in_at_most_three_times_the_time`, en debug y no en release, que pide autorización: 10.000 notas en 3,28 s y 20.000 en 6,32 s, una razón de 1,93, por debajo del tope de 3.

## Windows (2026-10-24)

En `076eeb7`, cuyo `crates/` es el de la cabeza del pull request (`f3d8e62`), pasan la suite (`cargo test --locked --workspace --target-dir target-test`, con `target-test` limpio de los artefactos del workspace) y los dos clippy de `.claude/CLAUDE.md`. Quedan ignorados los dos bancos (`the_socket_is_not_slower_than_tcp` y el de 81) y `the_vectors_are_the_ones_fastembed_gave`, que pide el modelo real.

## CI

El del pull request #88 (cabeza `f3d8e62`) pasó en `ubuntu-latest` y en `windows-latest`.

## Importación real en el perfil de desarrollo (2026-10-24)

Se hizo con autorización explícita y con el método de import-linear. Se usó el binario de depuración de `target-test` en `076eeb7`, con el feature `testing`, y el modelo real. El perfil `dev` corrió en un home temporal (`CLAWG_TEST_HOME`) sobre una **copia** del snapshot `pre-import` del 2026-10-23 y una copia de su modelo, así que de `~/.clawg-dev` sólo se leyó. Por la longitud de esa ruta, el daemon sirvió sólo por TCP. Un `engram export --all` nuevo dio las mismas 3.814 notas y 52.080 filas que el de origen. Se guardó fuera del repositorio y se borró al terminar, junto con el home temporal y la copia del baseline.

Una sonda hizo una ronda por segundo de `/health`, el listado de una fila y una búsqueda, cada una con tope de 15 s, durante las dos corridas en seco y la importación. En la segunda corrida en seco se agregó una sonda densa, sólo de listado: 866 muestras en 68,9 s, con un hueco máximo de 204 ms entre dos. Cualquier retención de la conexión más larga que ese hueco habría quedado atrapada.

| | import-linear | Con este cambio |
|---|---|---|
| `--dry-run` | 70 s | 73,5 s y 68,9 s |
| Espera más larga del listado durante el dry run | 8,8 s, detrás de la pasada de decisión | 19,8 ms (ronda por segundo) y 15,1 ms (sonda densa) |
| Importación | 1.218 s | 1.362 s |
| Espera más larga en la importación | listado 1,7 s, búsqueda 1,3 s | listado 298 ms, búsqueda 1,77 s |
| Filas importadas | 51.996 de 52.080 | 51.996 de 52.080 |

Ninguna petición venció ni respondió algo distinto de 200. Por fase de la importación:

- **Antes de embeber** (los primeros 150 s, con la traducción y la pasada de decisión): el listado esperó a lo sumo 8,8 ms, y `/health` trajo su bloque `ingest` en todas las rondas.
- **Embebiendo:** el listado esperó a lo sumo 23,7 ms. Las búsquedas de hasta 1,77 s esperan al embedder, no a la conexión: en las mismas rondas, el listado, que no pide un vector, respondió en milisegundos. Es de embed-throughput (85).
- **Escribiendo** (los últimos 23 s, desde el snapshot `pre-import`): el listado llegó a 298 ms, y una vez `/health` respondió sin su bloque `ingest`, porque el store no contestó dentro de 250 ms. Es la colocación por lote dentro de la transacción que escribe, que este cambio deja fuera.

La importación tardó 144 s más que la de import-linear. El dry run, que hace todo lo que este cambio toca y no embebe, tardó lo mismo, así que la diferencia está en los embeddings y la escritura.

El informe de la importación es idéntico al de los dos dry run, salvo «would import» y la línea del snapshot. Las 51.996 filas importadas son las de la medición de origen, comparadas con su snapshot `pre-move`. Las trece columnas que compara import-linear (agente, proyecto, hash, fecha, estado, tipo, clave, borrado, causa, origen, proyecto de origen, idioma e importancia) dan el mismo SHA-256. Esta vez también dan el mismo los ids, `superseded_by`, `revived_from` y los enlaces de `memory_piece`, y coinciden las piezas (51.068) y los pares (141). Con datos reales, el planificador asigna los mismos ids que la pasada SQL de 0.56.0 (ID2). Sólo difieren los accesos (52.320 contra 52.405), por las búsquedas que recibió cada store.

## Pendiente

Nada de la verificación. Queda el archivo (tareas, 5).
