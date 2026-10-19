# revision-aware-delete — Verificación

## Linux

- `cargo test --workspace`: pasa entera.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- El contrato del agente se bendijo con `CLAWG_BLESS=1`; el diff cambia sólo las descripciones de `delete_memory` (borrar la vigente de una `topic_key` trae la versión anterior) y `restore_memory` (el rechazo por clave y los pares nuevos).
- Cada requisito de la [spec](spec.md) tiene su test, escrito antes que el código. Los de R1 a R8 fallaron primero por la aserción o porque no compilaban.

## Windows

`cargo test --workspace --target-dir target-test` pasa entera y el clippy de `.claude/CLAUDE.md` sale con 0, con `target-test` limpio de los artefactos del workspace antes de compilar.

## Binario real, home temporal

`clawg serve --profile dev --port 0` desde la build de prueba, con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake`:

1. Dos saves con `topic_key` `storage/engine`: el segundo responde `replaced: [#1]`.
2. `DELETE` de #2 responde `revived: [#1 "Storage engine is SQLite"]`, y #1 vuelve `active`.
3. `POST …/2/restore` responde `topic_key` y `replaced: [#1]`, y #1 vuelve a quedar `superseded` por #2.
4. `clawg undelete 2`, después de otro borrado, imprime `restored memory #2 (topic storage/engine)` y `superseded again: #1 …`.

## V17 a través de `clawg update`

V17 es la primera migración que `clawg update` aplica sobre una instalación publicada: la última release lleva V16. Los tests de `update` que ya existían congelan la base en V15 con dos filas `active`, así que el `UPDATE` de V17 no tocaba ninguna fila en ellos.

- **Test nuevo:** `an_update_over_a_store_without_supersede_causes_keeps_its_rows_and_tells_revisions_from_verdicts` (`crates/clawg-cli/tests/binary.rs`). Congela la base en V16 (`UNCAUSED_SCHEMA`) con una revisión, una revisión deshecha por un veredicto, un veredicto entre filas sin clave y una vigente borrada, sembradas por SQL, y corre `clawg update` desde la copia instalada. Comprueba que el snapshot `pre-migration` es la base en V16 con todas sus filas, que la base migrada tiene las mismas filas y columnas, las causas `revision` / `verdict` de cada una, ningún `revived_from`, y, a través del daemon nuevo, que borrar la revisión vigente revive a la anterior y que borrar el ganador de un veredicto (el de la revisión deshecha incluido) deja a la perdedora en `hidden`.
- **El test detecta el error que importa:** con V17 cambiada a mano para que ignore el par `supersedes` juzgado, falla porque la revisión deshecha queda `revision`. Con V17 restaurada (idéntica a la del commit, checksum fijado) vuelve a pasar.
- **Lo que ya cubría el resto:** las migraciones corren agrupadas, en una transacción con su fila de historial, después del snapshot `pre-migration`. Un binario nuevo que migró y falla devuelve la base al snapshot (`a_new_binary_that_migrated_and_failed_gets_its_pre_migration_restored`), y uno anterior rechaza V17 sin tocarla (`SchemaNewer`).
- **Contra el código de la release anterior:** `revise_in` deja la `topic_key` en las filas que reemplaza, y `judge_pair` guarda el par como `(MAX, MIN)` con `verdict = 'supersedes'` (`RECORD_PAIR_SQL`), que es lo que lee el `CASE` de V17. Ningún trigger salta con ese `UPDATE` (los de V8 son de `INSERT`, `DELETE` y `UPDATE OF deleted_at`). Los structs de respuesta del puente anterior no rechazan campos desconocidos, así que un puente que sigue abierto durante la actualización lee `revived`, `hidden` y `replaced` sin error.

## Una base real de la release anterior

Un snapshot de `clawg backup` de una instalación en la release anterior (V16), copiado a un home temporal y abierto con `clawg serve` de la build de prueba y el modelo falso (el contrato de embeddings no coincide y deja la memoria deshabilitada, pero la migración corre al abrir el store, antes del modelo):

- Refinery aplica V17 en una transacción después del snapshot `pre-migration`.
- Cada tabla normal (`memory`, `memory_conflict`, `memory_piece`, las tablas de FTS y de vectores, los docs del proyecto) da el mismo hash, sobre sus columnas anteriores, antes y después. `integrity_check` da `ok` y el snapshot es igual a la base original.
- Las 18 filas `superseded` quedan 10 `revision` (su reemplazante es del mismo agente, proyecto y clave) y 8 `verdict` (cada una con su par `supersedes` juzgado o sin clave compartida). Ninguna fila tiene `revived_from`, y el primer barrido no revive nada en esa base.

## Perfil dev, puente MCP real y modelo real

La base dev estaba en V16. `clawg serve --profile dev` de la build de prueba la migró después de su snapshot `pre-migration`, con todas las tablas iguales a él. Por stdio, `clawg mcp --profile dev --no-autostart --agent revision-check` en una carpeta vacía:

1. Dos `save_memory` con la clave `revision-check/storage-engine`: el segundo responde `saved #96; replaces #95`.
2. `search_memory` encuentra sólo #96, con `topic=revision-check/storage-engine`.
3. `delete_memory` de #96 dice `the version it had replaced is current again: #95`, y `search_memory` encuentra #95.
4. `restore_memory` de #96 dice `restored #96 topic=revision-check/storage-engine` y `replaces again the version its delete had brought back: #95`, y `search_memory` vuelve a encontrar sólo #96.

Las dos filas se borraron al terminar y el daemon se detuvo con `clawg stop`.

## Lo que encontró la verificación

- Borrar pieza por pieza un save largo con clave no revivía nunca a la versión anterior: las filas reemplazadas apuntan a la primera pieza, y borrarla mientras las otras siguen vivas no revive nada. Hasta entonces sólo lo arreglaba el barrido. Ahora el borrado de la última pieza aplica la regla también a la primera (`ee3bd01`).
- Una base de la release anterior en la que se borró la vigente de un tema guarda la versión anterior `superseded` por una fila borrada. Después de actualizar, el primer paso de mantenimiento (`STARTUP_DELAY`, cinco minutos después de arrancar) la trae de vuelta (R8): sólo queda en el log del daemon y `clawg update` no lo informa. Se acepta así: es lo que pide R8, y restaurar la borrada lo deshace. Queda escrito en `specs/storage` y `specs/restore` como efecto de abrir una base anterior.

## No aplica

- `rag-eval`: el ranking no cambia y el golden set no tiene claves ni borrados.
