# import-undo — Especificación

## IU1. Cada import que escribe tiene un número y un diario

Un import que escribe algo MUST quedar en `memory_import` con un número (`MAX + 1`), su inicio, su fuente (`clawg` o `engram`) y su snapshot `pre-import`, y su reporte MUST traer ese número en `import_id`. En la misma transacción que cada escritura MUST quedar en el diario:

- cada fila insertada (`memory_import_row`), con `update_count`, `deleted_at` y `supersede_cause` tal como se insertó;
- cada fila que `REVISED_SQL` reemplazó (`memory_import_supersede`), con su `status`, `superseded_by` y `supersede_cause` previos;
- en `memory_import_extra`, sólo si la escritura agregó la fila (`changes() = 1`): cada vínculo rellenado en una fila que el import no insertó (`fill`: fila, columna y valor), cada par (`pair`, con el `state` y el `verdict` escritos), cada derivación de una conclusión que no insertó (`derivation`), cada pieza de un save de una fila que no insertó (`piece`), cada raíz (`project`) y cada remoto (`remote`).

`finished_at` MUST escribirse en la transacción de los vínculos. Un dry run, y un import que no escribe nada, MUST NOT tener número (`import_id: null`) ni cabecera.

#### Escenario: número y cabecera
- GIVEN un store con una fila
- WHEN se importa un export de clawg y después el mismo archivo otra vez
- THEN el primero da `import_id = 1` con su cabecera terminada y una fila de diario por fila insertada, y el segundo da `null` y no agrega cabecera

**Tests:** `an_import_is_numbered_and_journaled`, `a_dry_run_and_an_import_that_adds_nothing_have_no_number` (`import.rs`).

## IU2. Deshacer devuelve el store al estado anterior

`undo_import(N)` MUST borrar de verdad cada fila insertada por N que el criterio de IU3 retira (la fila, su vector, sus enlaces de `memory_piece`, sus derivaciones y sus pares), MUST revertir lo que N cambió en filas previas que sigue como N lo dejó y MUST sacar del diario lo que retiró o revirtió:

- una fila que `REVISED_SQL` reemplazó vuelve a su estado previo si sigue `superseded` por la misma fila y esa fila se retira o ya no está;
- un vínculo rellenado vuelve a `NULL` si conserva el valor escrito;
- un par, una derivación o un save agregados se quitan si ninguna fila que se queda los sostiene, y el par sólo si conserva su `state` y su `verdict`;
- una raíz agregada se quita si ninguna fila queda con su id, no tiene `index_docs` ni documentos y ningún remoto que se queda la nombra; un remoto agregado, si ninguna fila queda con su proyecto y ninguna raíz que se queda tiene ese id.

Antes de escribir, si hay algo que deshacer, MUST escribir un snapshot `pre-undo`. El snapshot `pre-import` MUST NOT tocarse.

#### Escenario: un archivo de clawg ida y vuelta
- GIVEN un store con una fila y el export de `seed`
- WHEN se importa el export y se deshace el import
- THEN `memory`, `memory_vectors`, `memory_fts`, `memory_piece`, `memory_derivation`, `memory_conflict`, `project` y `project_remote` quedan como antes de importar, y un nuevo import del mismo archivo inserta las mismas filas con otro número

#### Escenario: un export de otra herramienta ida y vuelta
- GIVEN un store con una fila y notas de otra herramienta con relaciones entre ellas y una nota larga
- WHEN se importan y se deshace el import
- THEN el store queda como antes, pares incluidos

#### Escenario: una reimportación más nueva
- GIVEN notas importadas y una reimportación de una nota reescrita en la fuente, que reemplaza a la guardada
- WHEN se deshace la reimportación
- THEN la versión guardada vuelve a `active` sin `superseded_by` ni `supersede_cause`, y deshacer después el primer import retira todo

#### Escenario: un vínculo rellenado
- GIVEN un import cuyos vínculos se perdieron y un segundo import del mismo archivo que los rellena
- WHEN se deshace el segundo
- THEN los vínculos vuelven a vacíos; con un valor cambiado a mano después, ese queda

#### Escenario: lo guardado después
- GIVEN un import y un save posterior que el detector empareja con una fila previa al import
- WHEN se deshace el import
- THEN el save y su par siguen

**Tests:** `undoing_a_clawg_import_gives_the_store_back`, `undoing_a_foreign_import_gives_the_store_back`, `undoing_a_rewrite_brings_the_stored_version_back`, `undoing_fills_empties_the_links_it_filled`, `what_was_saved_after_stays` (`undo.rs`).

## IU3. Sólo se retira lo que sigue como el import lo dejó

Una fila insertada por N MUST quedarse, nombrada con su motivo en `kept`, si:

- `edited`: su `update_count` difiere del anotado;
- `deleted` o `restored`: su `deleted_at` difiere del anotado;
- `judged`: su `supersede_cause` pasó a `verdict`, o un par suyo difiere del que escribió N, o tiene un veredicto y N no lo escribió;
- `referenced`: la nombra una fila que no se retira, por `superseded_by`, `revived_from`, `disputed_against` o como fuente en `memory_derivation`, sin contar los vínculos que N escribió en filas previas y siguen como los dejó;
- `tied`: es pieza de un save que tiene una fila de N que se queda.

La decisión MUST ser una función pura sobre el diario y el estado leído, iterada hasta que no cambie. MUST NOT contar el proyecto, el `scope`, el `tier`, los accesos, `last_modified_at` ni un `supersede_cause = 'revision'`. Una fila que ya no está (purgada) MUST salir del diario sin fallar. Cada transacción de borrado MUST volver a leer `update_count`, `deleted_at` y `supersede_cause` de sus filas y conservar las que cambiaron desde la decisión.

#### Escenario: el criterio
- GIVEN un import y, después, una fila editada, una borrada, una restaurada, una con un par juzgado, una que es fuente de una conclusión, una movida de proyecto, una accedida, una bajada de `tier` y una reemplazada por un save con su clave
- WHEN se deshace el import
- THEN las cinco primeras se quedan con su motivo, y las otras se retiran

**Tests:** `decide_keeps_what_changed_and_what_is_named` (unidad, cada motivo y la iteración), `the_rows_someone_changed_stay` y `a_purged_row_does_not_stop_the_undo` (`undo.rs`).

## IU4. Deshacer es explícito, repetible y se puede ensayar

Un `import_id` sin cabecera MUST ser `UnknownImport` (`404`); un import que este proceso está escribiendo MUST ser `Running` (`409`). Deshacer dos veces MUST NOT cambiar nada la segunda vez. Con `dry_run` MUST reportar lo mismo que el real sin escribir nada, ni snapshot.

**Tests:** `an_undo_twice_changes_nothing_the_second_time`, `an_unknown_or_running_import_is_refused`, `a_dry_run_undo_reports_without_writing` (`undo.rs`).

## IU5. La puerta del daemon y la del CLI

`POST /v1/import/undo` (`import_id`, `dry_run`) MUST responder el `UndoReport` sin pedir el embedder. `clawg import --undo N [--dry-run]` MUST mandarlo y decir lo que hizo en el idioma de la preferencia; `--undo` MUST NOT aceptar un archivo, `--from`, `--agent`, `--map-root` ni `--project-map`, y `clawg import` sin archivo ni `--undo` MUST ser un error de uso. La salida de un import con número MUST nombrar el comando que lo deshace.

**Tests:** `import_takes_a_file_or_an_undo` (`cli.rs`), el render de `undo` en los dos idiomas (`crates/clawg-cli/src/import.rs`), y el flujo por HTTP (`crates/clawg-cli/tests/api.rs`).

## IU6. El esquema

V26 MUST agregar sólo las cuatro tablas del diario, sin claves foráneas (`purge` borra filas sin pasar por el diario), y sus índices por número. Un store de V25 MUST abrir con sus filas y las tablas nuevas vacías. `pre-undo` MUST ir y volver como nombre de snapshot.

**Tests:** `a_store_from_before_the_import_journal_keeps_its_rows` (`db.rs`), `published_migration_checksums_are_pinned` (`migrations.rs`), `snapshot_names_parse_back_to_their_reason` (`backup.rs`).
