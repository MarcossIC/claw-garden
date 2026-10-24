# import-decision-pass — Especificación

## ID1. La pasada de decisión no ocupa la conexión

La pasada que decide dónde va cada fila de un import (la que corre una corrida en seco entera y la real antes de embeber) MUST NOT correr en una llamada a la conexión del store que dure con el archivo. Lo que lee del store MUST cargarse en llamadas de como mucho `LOAD_BATCH` búsquedas por índice, cada una su propio `conn.call`, sin una transacción abierta entre una y otra. La decisión MUST correr fuera de la conexión y fuera de los hilos del runtime async (`spawn_blocking`). La pasada MUST NOT escribir nada en el store.

#### Escenario: una sonda durante la corrida en seco
- GIVEN un store vacío y un export de otra herramienta de 4.000 notas
- WHEN corre `--dry-run` y, al mismo tiempo, una sonda hace `SELECT 1` en la conexión cada 5 ms
- THEN ninguna sonda espera más de 300 ms

#### Escenario: nada escrito
- GIVEN el store de `a_dry_run_reports_without_writing`
- WHEN corre la corrida en seco
- THEN `memory`, `project`, `project_remote`, `memory_fts` y `sqlite_sequence` quedan iguales y no hay snapshot

**Tests:** `a_dry_run_leaves_the_connection_to_other_calls` y `a_dry_run_reports_without_writing` (ampliado), en `crates/clawg-core/src/memory/portability/import.rs`.

## ID2. El planificador decide lo mismo que la pasada SQL

El `Ledger` que da el planificador MUST ser igual al que daría la pasada SQL de antes (`place_rows` y `write_links` en una transacción que se suelta) sobre el mismo store y el mismo archivo. Eso incluye las ubicaciones con sus ids, los agentes de las filas insertadas, los proyectos tomados, las líneas inválidas, los enlaces perdidos, lo que se encontró de cada nota y las filas ausentes. Las reglas MUST estar escritas una vez, sobre un asiento que implementan la transacción y el planificador. Los ids que asigna MUST seguir a `sqlite_sequence`, como `AUTOINCREMENT`, y no a `MAX(id)`. Entre dos filas vivas del mismo texto y la misma fecha, el gemelo MUST ser la de menor id, en los dos.

#### Escenario: la tabla
- GIVEN cada uno de estos stores y archivos:
  - un export de clawg en un store con una fila;
  - el mismo, importado por segunda vez;
  - con un gemelo vivo de otro agente;
  - con una clave tomada;
  - con `--agent`;
  - las notas de muestra de otra herramienta, primero y por segunda vez;
  - una nota reescrita en la fuente, más nueva y más vieja;
  - una nota borrada acá y otra borrada en la fuente;
  - dos notas de un mismo origen;
  - un texto repetido en el archivo;
  - una clave repetida en el archivo;
  - un proyecto que tomó otra raíz;
  - dos filas vivas del mismo texto y la misma fecha;
  - un store con la última fila purgada;
  - un export cuyos enlaces se perdieron entre transacciones.
- WHEN se decide con la pasada SQL y con el planificador
- THEN los dos `Ledger` son iguales

#### Escenario: después de una purga
- GIVEN un store cuya fila de mayor id se borró, así que `sqlite_sequence` queda por encima de `MAX(id)`
- WHEN corre la corrida en seco y después la real
- THEN los dos reportes coinciden salvo `dry_run` y el snapshot

**Tests:** `the_planner_decides_as_the_sql_pass` y `a_dry_run_after_a_purge_reports_the_ids_of_the_import` (`import.rs`); el oráculo de import de `import.rs`, `crates/clawg-cli/tests/api.rs` y `crates/clawg-cli/tests/binary.rs`, sin editar.

## ID3. Un import soltado deja de planificar

Cuando se suelta el futuro de un import mientras el planificador corre (un apagado que corta el pedido), el planificador MUST terminar antes del próximo lote de filas sin decidir el resto.

**Tests:** `a_stopped_planner_decides_nothing` (`import.rs`).

## ID4. Cada búsqueda va por un índice

Las consultas de la carga MUST buscar por un índice y no recorrer ninguna tabla, como ya pasa con las de la pasada SQL: `ORIGIN_ROWS_SQL` por `idx_memory_import_origin`, `TEXT_ROWS_SQL` por `idx_memory_dedup`, `KEY_ROWS_SQL` por `idx_memory_topic` y `PROJECT_ROOT_SQL` por la clave primaria de `project`. `TWIN_SQL`, con `id` en su orden, MUST seguir buscando por `idx_memory_dedup`.

**Tests:** `the_import_lookups_seek_an_index` (ampliado).
