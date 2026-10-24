# import-undo — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | import-linear (81), import-decision-pass (82) | ver [`state.yaml`](state.yaml) |

## Problema

Verificado en la [exploración](exploration.md#problema-verificado-contra-el-código). La única vuelta atrás de un import es el snapshot `pre-import` (`crates/clawg-core/src/memory/portability/import.rs:1459`), y restaurarlo (`clawg restore`) descarta todo lo guardado después. El reporte no trae un número de import (`ImportReport`, `import.rs:96-127`) y ninguna tabla dice qué import insertó qué fila, así que tampoco hay un borrado a mano seguro. La poda de snapshots conserva cinco automáticos y el más nuevo de cada motivo (`backup.rs`), de modo que el de un import viejo puede no estar.

## Objetivo

1: guardar sin perder lo guardado. `clawg import --undo N` retira el import N días después, sin tocar lo que se guardó, editó o juzgó desde entonces, como `clawg projects move --undo N` con un movimiento.

## Decisiones

Cierran las [decisiones abiertas](exploration.md#decisiones-abiertas) con la opción que la exploración recomienda en cada una.

1. **Diario del import (V26), sin claves foráneas**, como `memory_move` (V21): `memory_import` (la cabecera: número, inicio, fin, fuente, snapshot), `memory_import_row` (cada fila insertada con `update_count`, `deleted_at` y `supersede_cause` tal como se insertó), `memory_import_supersede` (lo que `REVISED_SQL` reemplazó, con su estado previo) y `memory_import_extra` (lo agregado que cuelga de filas que no insertó: vínculos rellenados, pares, derivaciones, piezas, raíces y remotos). Se escribe en la misma transacción que cada escritura real (`SqlSeat` y `record_projects`) y sólo cuando la escritura cambió algo (`changes() = 1`). Lo que se escribe en filas que el import insertó no va en `memory_import_extra`: muere o se queda con ellas.
2. **El número** es `MAX + 1`, asignado en la primera transacción que escribe algo. Un dry run, o un import que no escribe nada, no tiene número (`import_id: null`). `finished_at` queda nulo si el proceso muere: ese import se puede deshacer igual. Un import que este proceso está escribiendo no (`409`).
3. **Borrado real**, no `deleted_at`: una fila borrada cuenta como presente (`PRESENT_SQL`, `import.rs:993-996`), así que sin borrarla el mismo archivo no podría volver a importarse, y un borrado lógico dispara la reviviscencia (`admin.rs`). Se quitan la fila, su vector, sus piezas, sus derivaciones y sus pares; el índice de texto lo resuelve el disparador de V8.
4. **El criterio**, puro (`decide`) y conservador. Una fila insertada **se queda** si:
   - `update_count` o `deleted_at` cambiaron (editada, borrada o restaurada);
   - pasó a `supersede_cause = 'verdict'`, o un par suyo tiene un `state` o un `verdict` distinto del que escribió el import (o un veredicto, si el import no lo escribió): alguien la juzgó;
   - la nombra una fila que no se retira (`superseded_by`, `revived_from`, `disputed_against` o como fuente de `memory_derivation`), salvo por un vínculo que escribió el mismo import en una fila previa: una conclusión sintetizada que se apoya en ella la conserva;
   - es pieza de un save con una fila que se queda.

   Se itera hasta que no cambia. No cuentan el proyecto (`projects move --origin-project` es el paso normal después de importar), el `tier`, los accesos, `last_modified_at` (mantenimiento y refuerzo los mueven, `maintenance.rs`, `store_insert.rs`) ni un reemplazo por clave posterior (`revision`): la fila nueva se queda y la importada se va.
5. **Lo que el import cambió en filas previas** vuelve si sigue como el import lo dejó: un reemplazo de `REVISED_SQL` se revierte cuando su reemplazante se retira (o ya no está); un vínculo rellenado vuelve a vacío si conserva el valor escrito; un par, una derivación o un save agregados se quitan si ninguna fila que se queda los sostiene y, en el caso del par, si nadie lo juzgó; una raíz o un remoto que el import agregó se quitan si ninguna fila los usa, no tienen `index_docs` ni documentos indexados.
6. **Snapshot `pre-undo`** antes de escribir, como `pre-move`, sólo cuando hay algo que deshacer. El `pre-import` se queda.
7. **En lotes**: la decisión carga lo que lee en llamadas de `LOAD_BATCH` búsquedas y decide fuera de la conexión; el borrado va en transacciones de `WRITE_BATCH` filas, y cada una vuelve a mirar las columnas del criterio de cada fila antes de borrarla, así una edición que llegó entre la decisión y el borrado la conserva. Lo retirado sale del diario; lo conservado se queda, y un segundo `--undo N` lo vuelve a evaluar.

## Alcance

1. V26 con su prueba de un store de V25 y su fila en la de checksums fijados.
2. El diario en `SqlSeat`, `record_projects` y la transacción de los vínculos, con `import_id` en el reporte, un registro de los imports en curso en `MemoryStore` y la línea del número en la salida de `clawg import`.
3. `portability::undo`: `undo_import(store, backups, import_id, dry_run)`, con `decide` pura, `UndoReport`, `UndoError` y `Reason::PreUndo`.
4. `POST /v1/import/undo` (`import_id`, `dry_run`; sin embedder; `404` sin el import, `409` si está en curso) y `clawg import --undo N [--dry-run]`, en inglés y en español.
5. `.claude/CLAUDE.md`, el delta de `specs/portability` y de `specs/storage` y la fila 84 de `openspec/README.md`.

### No-alcance

Lo de la [exploración](exploration.md#no-alcance): no sustituye ni recorta el snapshot `pre-import`; los imports anteriores a V26 no tienen diario; no hay trabajo asíncrono de import (87); no cambia qué decide el import (82) ni cómo se parten las notas (83); no limpia snapshots, `workspace.md` ni exports que ya contienen lo importado; no agrega una regla de `doctor`.

## Cómo se mide

Tests que fallan hoy (la lista completa está en la [spec](spec.md)):

1. Un import devuelve `import_id` y escribe su cabecera; un dry run y un segundo import sin nada nuevo dan `null`.
2. Importar un archivo clawg y deshacerlo deja `memory`, `memory_vectors`, `memory_fts`, `memory_piece`, `memory_derivation`, `memory_conflict`, `project` y `project_remote` como antes, y el mismo archivo vuelve a importarse entero. Lo mismo con un export de otra herramienta con relaciones.
3. Lo guardado después se queda, con sus pares con filas que no se importaron.
4. Deshacer una reimportación ajena más nueva devuelve la versión anterior a `active` con su estado previo.
5. Un vínculo rellenado en una fila presente vuelve a vacío, y no si su valor cambió.
6. El criterio, motivo por motivo, en `decide` y en el flujo: se queda una fila editada, borrada, restaurada, juzgada, citada por una conclusión o pieza de un save que se queda; se va una movida, accedida, bajada de `tier` o reemplazada por una clave posterior.
7. Una raíz que agregó el import se quita si nada la usa y se queda si tiene filas o `index_docs`.
8. Una fila purgada no traba el resto; `--undo N` dos veces no hace nada la segunda; `--undo 99` es `404`; un import en curso es `409`; `--undo` con un archivo es un error de uso.
9. El dry run reporta lo mismo sin escribir ni snapshot; el real escribe `pre-undo`.
10. Un store de V25 abre con sus filas y las tablas nuevas vacías; `pre-undo` va y vuelve como nombre de snapshot.

Medición: el banco de 81 (`twice_the_notes_import_in_at_most_three_times_the_time`) con el diario sigue por debajo de su tope, y el tiempo del `--undo` de 20.000 notas, anotados en `verify.md`. No hay cambio de ranking: no corre `rag-eval`.

## Migración y vuelta atrás

V26 sólo agrega cuatro tablas vacías y sus índices; ninguna fila cambia. Un binario anterior no abre un store migrado (refinery rechaza una migración desconocida, como con cualquier versión), y la vuelta es el snapshot `pre-migration` que escribe la apertura que migra. Un deshacer se revierte con el snapshot `pre-undo`.
