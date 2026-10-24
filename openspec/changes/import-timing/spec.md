# import-timing — Especificación

Suma IT1 a IT4 a [`specs/portability`](../../../specs/portability/spec.md).

## IT1. Cada fase de una importación se registra al terminar

Una importación MUST registrar cada fase que corre al terminarla, una vez y en el orden en que corre, con una línea de `info` en el log del daemon. Las fases son:

| Fase | Qué mide | Detalle |
|---|---|---|
| `read` | el handler lee el archivo (`format::parse` o `engram::read`) | — |
| `plan` | los ids que daría el registro de cada raíz | llamadas |
| `prepare` | export de clawg: analizar, enmascarar y armar las filas | filas |
| `translate` | export de otra herramienta: traducir las notas y armar las filas | filas |
| `load` | lo que la pasada de decisión lee del store | filas leídas, llamadas |
| `decide` | la pasada de decisión | filas |
| `embed` | los vectores de las filas a insertar | filas, tandas, filas/s, la tanda más lenta |
| `snapshot` | el snapshot `pre-import` | — |
| `write` | los lotes de escritura | filas, llamadas |
| `links` | los vínculos y el cierre del diario | llamadas |

Un dry run MUST terminar en `decide`. Ninguna línea MUST llevar texto de una fila, un id, una clave ni una ruta.

#### Escenario: una importación real de otra herramienta
- GIVEN un store y las notas de muestra de otra herramienta
- WHEN se importan con `FakeEmbedder`
- THEN quedan registradas `plan`, `translate`, `load`, `decide`, `embed`, `snapshot`, `write` y `links`, una vez cada una y en ese orden
- AND las filas de `embed` son las insertadas no borradas del reporte
- AND las llamadas de `write` son los lotes de `note_batches`

#### Escenario: un export de clawg
- GIVEN el export del store de prueba
- WHEN se importa
- THEN las fases son `plan`, `prepare`, `load`, `decide`, `embed`, `snapshot`, `write` y `links`

#### Escenario: un dry run
- GIVEN las mismas notas
- WHEN corre `--dry-run`
- THEN las fases son `plan`, `translate`, `load` y `decide`

**Tests:** `an_import_times_each_step_once_in_order`, `a_clawg_file_is_prepared_rather_than_translated` y `a_dry_run_times_only_what_it_runs` (`crates/clawg-core/src/memory/portability/import.rs`).

## IT2. La conexión: SQLite y la cola por separado

En cada fase que usa la conexión del store (`plan`, `load`, `write`, `links`), la línea MUST contar las llamadas y separar el tiempo dentro de la clausura (`in sqlite`) de lo que falta hasta que vuelve la llamada (`queued`). También MUST nombrar la llamada que más tiempo tuvo la conexión. La suma de los dos tiempos MUST NOT superar la duración de la fase. `load` MUST hacer una llamada por cada `LOAD_BATCH` búsquedas de cada conjunto, más la de las raíces.

#### Escenario: las llamadas de `load`
- GIVEN un store vacío y un export de clawg de 600 filas de textos distintos, sin clave y sin origen
- WHEN se importa
- THEN `load` hace 4 llamadas (3 de textos y 1 de las raíces)
- AND en `load` y en `write`, `in sqlite` + `queued` ≤ la duración

**Tests:** `time_in_sqlite_never_exceeds_the_call` (`import.rs`).

## IT3. El resumen

Al terminar una importación con éxito, el daemon MUST registrar una línea con el total desde que el handler la empezó y la duración de cada fase en el orden de IT1, en segundos con un decimal. Un dry run MUST decirlo en esa línea. Una importación que falla deja las líneas de las fases que terminó y no tiene resumen.

#### Escenario: el texto
- GIVEN una estructura armada a mano con las fases de una importación real
- WHEN se formatean una fase y el resumen
- THEN dan, por ejemplo, `load 3.1 s (812 rows, 5 calls, 2.6 s in sqlite, 0.5 s queued, slowest 41 ms)` e `import: done in 1362.4 s: read 1.8, …, links 0.4`

**Tests:** `a_step_reads_as_its_numbers` y `the_summary_names_every_step` (`crates/clawg-core/src/memory/portability/timing.rs`).

## IT4. Medir no cambia la importación

Ni lo que una importación escribe ni su reporte MUST cambiar. La paridad entre el dry run y el real (R7) se mantiene.

**Tests:** las pruebas de import de `import.rs`, `undo.rs`, `crates/clawg-cli/tests/api.rs` y `crates/clawg-cli/tests/binary.rs`, sin cambiar ninguna aserción.
