# import-undo — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (guardar sin perder lo guardado después) | M | import-linear y import-decision-pass (tocan el código donde se escribe el diario); se coordina con import-job | exploración (borrador) |

**Tesis:** el único camino de vuelta de una importación es el snapshot `pre-import`, y restaurarlo descarta todo lo guardado desde entonces; una semana después deja de ser una opción. El cambio agrega `clawg import --undo N`, que retira una importación concreta y conserva lo posterior, igual que `clawg projects move --undo N` con un movimiento. Para eso la importación escribe un diario en la misma transacción que cada inserción (hoy el informe no trae número de importación, sólo la ruta del snapshot), y el deshacer decide fila por fila con el criterio de `relocate`: toca sólo lo que sigue como la importación lo dejó. Lo que una importación cambia en filas que ya estaban es poco y está acotado: marcar como reemplazadas filas de una importación anterior del mismo origen, rellenar vínculos vacíos y agregar pares, derivaciones, piezas y filas de proyecto. Todo eso se puede registrar y revertir. La decisión de fondo es **borrar de verdad** las filas insertadas (no marcarlas borradas), porque una importación posterior de un archivo ya importado no repone lo que está borrado y el mismo archivo no podría volver a importarse. Es de tamaño M: una migración con tres o cuatro tablas, la escritura del diario en `place`/`write_links`, una función de deshacer con decisión pura y una ruta del daemon.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `REVISED_SQL` pone en `superseded` filas vivas del mismo origen | **Cierta, pero acotada.** Corre sólo cuando una nota ajena reescrita es más nueva que la última versión guardada (`Found::Revision { newer: true }`, `crates/clawg-core/src/memory/portability/import.rs:1183-1187`, `REVISED_SQL` en `:413-414`). Las filas que afecta salen de `ORIGIN_SQL` (`source = 'import' AND created_by_ref = ?1`, `:409-411`): son siempre filas de una importación anterior, nunca filas orgánicas. Un archivo de clawg no pasa por ahí: sin `notes`, `found` es `None` (`:1143-1151`). |
| `Placement::Merged` toca la fila que ya estaba | **Falsa.** `place` devuelve `Merged` antes de cualquier escritura (`:563-565` y `:578-582`), y `write_links` salta las filas no propias (`:727-729`). No suma accesos: el informe lo documenta (`:104`). |
| `write_links` rellena vínculos de filas ya presentes | **Cierta.** Una fila `Present` es propia (`owned`, `:342-344`) y recibe `superseded_by` donde era nulo y estaba `superseded` (`:734-738`), `revived_from` donde era nulo (`:749`), `disputed_against` donde era nulo y estaba `disputed` (`:770-773`), derivaciones con `INSERT OR IGNORE` (`:786-790`) y piezas (`:862-865`). Una `Present` puede ser una fila orgánica si el archivo es un export de este mismo store (`PRESENT_SQL`, `:494-497`, no mira `source`). |
| Los pares `memory_conflict` de relaciones ajenas pueden tocar una fila previa | **Cierta, sin llegar a volverla `disputed`.** Un par se escribe si al menos un extremo es propio (`:800`), y el otro puede ser una fila previa a la que una fila del archivo se fusionó (`id_of` devuelve el id de la `Merged`, `:803`). `INSERT OR IGNORE` sobre `PRIMARY KEY (new_id, old_id)` (`migrations/V15__memory_conflict.sql:20`). El estado `disputed` y `disputed_against` se aplican en `translate` sobre la fila del archivo (`translate.rs:744-750`) y en `write_links` sólo sobre una fila propia (`:770-773`): una fila previa no cambia de estado, pero un par `escalated` con ella sí aparece en los conflictos activos. |
| Una `supersedes` ajena reemplaza filas previas | **Falsa.** El veredicto edita las filas de la traducción (`translate.rs:752-762`), no las del store. |
| `record_projects` toca algo más que filas nuevas | **Falsa.** `INSERT OR IGNORE` en `project` y `project_remote` (`import.rs:1059-1069`), `index_docs = 0`. Puede dejar una raíz que no existía. |
| Las piezas, vectores e índice de texto de las filas insertadas son sólo de ellas | **Cierta, con un matiz.** El vector va en la misma transacción (`:639-641`); el índice de texto lo agrega el disparador de inserción (`migrations/V8__memory_fts.sql:63-66`) y una fila insertada ya borrada se quita con el disparador de borrado (`:73-78`, `:84-88`). Las piezas de un archivo clawg se enlazan con los ids de `placed`, **fusionadas incluidas** (`import.rs:1297`): una fila previa fusionada puede recibir un enlace `memory_piece` si ninguna de las colocadas tiene uno. |
| El informe trae un número de importación | **Falsa.** `ImportReport` (`:93-123`) sólo trae `snapshot: Option<String>` (`:114`, escrito en `:1037`). No hay tabla de diario: la última migración es V24 (`migrations.rs:68`). |
| El deshacer de un movimiento se puede copiar tal cual | **Parcial.** `memory_move` y `memory_move_supersede` (V21, V24) guardan el estado previo y deshacer exige que la fila siga donde el movimiento la dejó (`relocate.rs:644-654`). Un movimiento no crea filas ni vínculos, una importación sí: el diario necesita además la lista de lo insertado y de lo agregado. |
| Mantenimiento y refuerzo no alteran la fila | **Falsa para dos columnas.** `demote_sql` baja el `tier` y **estampa `last_modified_at`** (`maintenance.rs:43-50`); un guardado duplicado suma `access_count` y mueve `last_accessed_at` (`store_insert.rs:510-517`). Ninguna de las dos es una edición: `last_modified_at`, `tier` y los accesos no pueden entrar en el criterio. Una edición, en cambio, suma `update_count` (`admin.rs:261-267`) y restaurar también (`admin.rs:565-566`); un movimiento no (`relocate.rs:417`). |
| Borrar una fila insertada con la ruta normal basta | **Falsa.** `delete_memory_core` pone `deleted_at`, quita el vector (`admin.rs:440-441`) y **dispara la reviviscencia** (`:450`, `revival.rs:140`), que traería de vuelta filas que ese borrado no tiene por qué tocar. Y una fila borrada sigue contando como presente: `find` devuelve `Present` si la versión está borrada (`import.rs:943-946`) y `PRESENT_SQL` no mira `deleted_at`. |

## Problema (verificado contra el código)

### 1. Sólo existe el snapshot

La importación escribe un snapshot `pre-import` antes de escribir (`import.rs:1459`, `Reason::PreImport`, `backup.rs:139`) y devuelve su ruta (`:1037`). `clawg restore` reemplaza todo el store (CLAUDE.md, `clawg restore`): lo guardado después se pierde. Una importación grande se nota de inmediato o semanas después, y en el segundo caso el snapshot cuesta más de lo que protege. Además la poda conserva cinco automáticos y el más nuevo de cada motivo (`backup.rs:286-305`), de modo que el snapshot de una importación vieja puede haberse ido.

### 2. No queda registro de qué filas son de qué importación

Las filas importadas tienen `source = 'import'` y `created_by_ref` (origen de la nota) sólo para notas ajenas; las de un archivo clawg conservan su `source` original. No hay forma de saber qué importación insertó una fila, así que ni siquiera un `DELETE` a mano es seguro.

### 3. Lo que una importación cambia en filas previas (completo)

Verificado arriba; en orden de frecuencia:

1. **Nada**, para toda fila `Merged` y para toda fila nueva (caso normal de una primera importación).
2. **Reemplazo de filas de una importación anterior** (`REVISED_SQL`): sólo una reimportación ajena con versión más nueva.
3. **Vínculos rellenados en filas `Present`**: sólo si el archivo ya estaba (en parte) en el store, p. ej. tras un corte o al importar un export del mismo store.
4. **Filas agregadas que cuelgan de filas previas**: pares (`memory_conflict`), derivaciones (`memory_derivation`), enlaces de pieza (`memory_piece`), y las filas `project` / `project_remote`.

### 4. Qué puede pasarle a una fila importada después

Edición (`update_memory`: `update_count`), borrado, restauración, `purge` (fuera de línea, `secrets.rs:181`), movimiento a otro proyecto (`relocate.rs:417`; es el paso normal después de importar con `--project-map` ausente: `projects move --origin-project`), veredicto de un par, reemplazo por un guardado con la misma clave (`topic.rs`), síntesis que cita la fila como fuente (`store_insert.rs:246-253`) y decaimiento por mantenimiento. Un deshacer que no distinga estos casos o borra trabajo del usuario o no sirve para nada.

## Lo que ya existe y se reutiliza

- El patrón de V21/V24: tabla sin clave foránea («`purge` removes rows for good, and an old move must not stop it», `migrations/V21__memory_move.sql:10`), número `MAX(...)+1` (`relocate.rs:412`), deshacer en dos pasos (primero el estado previo de lo reemplazado, luego las filas que siguen como quedaron, `relocate.rs:644-666`), instantánea `pre-move` antes de escribir (`:581`) y `dry_run` por la misma ruta de decisión (`:568-580`).
- `release_challengers` (`dispute.rs:197`) libera a quien disputaba una fila que se va; `purge` ya sabe quitar vector, pares y derivaciones de una fila (`secrets.rs:225-232`).
- `ImportReport` ya es `Serialize/Deserialize` con `#[serde(default)]` para campos nuevos (`import.rs:120-126`).
- `write` ya tiene un único sitio donde se inserta (`INSERT_SQL`, `:602`), uno donde se reemplaza (`:709`), uno donde se registran proyectos (`:526`) y uno donde se escriben vínculos (`:720`).
- `Reason` con su prueba de ida y vuelta de nombres (`backup.rs:680-693`) y la prueba de checksums fijados (`migrations.rs:43-82`).
- El patrón de prueba «un store de la versión anterior conserva sus filas» (`db.rs:1625-1664`).

## Enfoques posibles

### A. Diario por importación y deshacer por criterio (recomendado)

Migración con cuatro tablas, sin claves foráneas:

- `memory_import (import_id, started_at, finished_at NULL, source, snapshot)`: la cabecera. `import_id = MAX + 1`, asignado en la primera transacción de escritura. `finished_at` queda nulo si el proceso muere: la importación sigue siendo deshacible.
- `memory_import_row (import_id, row_id, update_count, deleted_at)`: una fila por fila insertada (incluidas las insertadas ya reemplazadas por una fila previa, `TopicTaken`/`Older`), escrita en la misma transacción que el `INSERT`.
- `memory_import_supersede (import_id, row_id, superseded_by, from_status, from_superseded_by, from_cause)`: idéntica a `memory_move_supersede`, para `REVISED_SQL`.
- `memory_import_extra (import_id, kind, a, b, value)`: lo agregado que cuelga de filas previas y no muere con las insertadas: `fill` (fila, columna, valor escrito) para los vínculos rellenados, y `pair`, `derivation`, `piece`, `project`, `remote` con su clave, **sólo cuando `changes() = 1`** (el `INSERT OR IGNORE` agregó la fila de verdad).

**Deshacer N** (`clawg import --undo N`), todo dentro de lotes por transacción, en este orden:

1. Revertir `memory_import_supersede`: restaurar el estado previo de la fila si sigue `superseded` por la fila de la importación **y esa fila se retira en este mismo deshacer** (si no, quedarían dos versiones vivas de una clave).
2. Revertir los `fill`: `SET col = NULL WHERE id = ? AND col = value`.
3. Decidir, con una función pura, cuáles filas insertadas se retiran (criterio abajo).
4. Para las que se retiran: `DELETE` de la fila, de `memory_vectors`, de `memory_piece`, de `memory_derivation` y de `memory_conflict` donde figuren (como `purge`, `secrets.rs:225-232`); el índice de texto lo resuelve el disparador de borrado (`V8:84-88`, que no resta dos veces una fila ya borrada). `release_challengers` antes, para liberar a quien la disputaba.
5. Quitar `pair`, `derivation` y `piece` de `memory_import_extra` que sigan intactos y cuyo otro extremo no se retiró; quitar `project` y `project_remote` sólo si ningún `memory`, `project_docs` ni `index_docs = 1` los usa.
6. Sacar del diario lo retirado. Lo que se conservó queda en el diario con su motivo en el informe; un segundo `--undo N` vuelve a evaluarlo.

**Criterio** (el de `relocate`: la fila sigue como la importación la dejó). Se **conserva** una fila si:

- `update_count` o `deleted_at` difieren de lo anotado (editada, borrada, restaurada);
- su estado pasó a un veredicto (`supersede_cause = 'verdict'`, o un par suyo tiene `verdict` o `state` distinto al escrito): alguien la juzgó;
- otra fila fuera de la importación la nombra: `superseded_by`, `revived_from`, `disputed_against` o `memory_derivation.source_id` (una conclusión sintetizada se apoya en ella). El primer caso incluye un veredicto del usuario que la dejó vigente.

No cuentan, a propósito: `project_id`/`scope` (el movimiento posterior a importar es el flujo normal), `tier`, accesos, `last_modified_at` (mantenimiento y refuerzo los mueven, ver premisas) y `supersede_cause = 'revision'` (reemplazo automático por una clave posterior: la fila nueva se queda y la importada se va).

- **A favor:** devuelve el estado sin descartar lo posterior; una importación abortada o repetida se puede deshacer; la reimportación del mismo archivo vuelve a funcionar; el diario es proporcional al volumen (una fila de pocas columnas por fila insertada, del orden de las 52 000 de la importación real).
- **En contra:** una tabla grande y cuatro escrituras nuevas dentro de las transacciones de importación; el borrado de 50 000 filas con `vec0` y FTS no está medido (hay que medirlo, ver «Cómo se mide»); `memory_import_extra` agrega superficie para un caso raro.

### B. Borrado lógico en lugar de real

Marcar `deleted_at` en las filas insertadas (la ruta de `delete_memory_core`) y dejar que `undelete` las traiga. **Problema que lo descarta como defecto:** una fila borrada cuenta como presente (`import.rs:943-946`, `PRESENT_SQL` sin `deleted_at`), así que el mismo archivo no se puede reimportar tras deshacerlo, y además dispara reviviscencia y deja el texto en el store, que es lo que quien deshace quiere sacar. Su ventaja real es la reversibilidad; la da mejor el snapshot que se toma justo antes de deshacer (ver B′).

### B′. Snapshot antes de deshacer

En cualquier variante, antes de escribir se toma un snapshot (`Reason::PreUndo`, `pre-undo`, un valor más en `Reason::ALL`, `backup.rs:146-155`), como `pre-move`. El nombre se compara por igualdad exacta (`backup.rs:235`), así que no hay confusión de prefijos con `pre-import`. El snapshot de la importación **se queda**: el deshacer lo complementa.

### C. Sólo `source`/`created_by_ref` como diario

Etiquetar la importación en una columna de `memory` y borrar por etiqueta. No registra lo agregado a filas previas (reemplazos, rellenos, pares, proyectos), obliga a otra migración sobre la tabla más grande y deja sin respuesta los archivos clawg, que conservan su `source`. Descartada.

### Forma de la API y la CLI

- **Núcleo** (`clawg_core::memory::portability::undo`): `undo_import(store, backups, import_id, dry_run) -> Result<UndoReport, UndoError>` con `UndoError::UnknownImport(i64)` (como `MoveError::UnknownMove`). La decisión es una función pura sobre estructuras (`decide(journal, current) -> Plan`), separada de la E/S.
- **`ImportReport`** gana `import_id: Option<i64>` (`#[serde(default)]`): `None` en un dry run o si no se insertó nada.
- **Daemon:** `POST /v1/import/undo` `{ import_id, dry_run }` (ruta aparte: `ImportRequest.text` es obligatorio, `api.rs:1871`). Respuesta: `UndoReport { import_id, dry_run, removed, restored, kept: [{ id, reason }], unfilled, pairs, projects, snapshot }`. No necesita embedder.
- **CLI:** `clawg import --undo N [--dry-run] [--profile P]`. El archivo pasa a ser opcional y `--undo` entra en conflicto con `file`, `--from`, `--agent`, `--map-root` y `--project-map` (como `projects move --undo` no admite selección, `api.rs:938`). La salida del import agrega una línea «esta es la importación N; `clawg import --undo N` la deshace», en los dos idiomas (`projects.rs:167-168` es el modelo). Se actualiza el texto del comando en `cli.rs:106-110` y `.claude/CLAUDE.md`.

### Recomendación: A con B′

Diario completo (A), borrado real y un snapshot `pre-undo` antes de escribir (B′). El criterio es conservador: ante la duda se conserva y se informa.

## Descartado

| Opción | Razón técnica |
|---|---|
| Reemplazar el snapshot por el deshacer | Ya decidido: el snapshot cubre lo que el diario no (un diario corrupto, una fila purgada, una migración). El deshacer lo complementa. |
| Borrado lógico (`deleted_at`) como mecanismo | Ver B: bloquea la reimportación (`import.rs:943-946`), dispara reviviscencia (`admin.rs:450`) y deja el texto. |
| Comparar `last_modified_at` o `tier` para saber si la fila cambió | Mantenimiento los mueve sin intervención (`maintenance.rs:43-50`); todas las filas viejas fallarían el criterio. |
| Exigir `project_id` igual al de la importación | `projects move --origin-project` es el paso previsto después de importar; el criterio bloquearía el caso de uso principal. |
| Etiquetar `memory` con `import_id` | Migración de la tabla más grande, no cubre lo agregado a filas previas ni los archivos clawg (enfoque C). |
| Reconstruir el diario a partir de `source = 'import'` | Falla para archivos clawg y no distingue importaciones entre sí. |
| Claves foráneas del diario a `memory` | `purge` borra filas sin avisar (V21:10); un diario viejo no debe frenarlo. |
| Un `UNIQUE` en `memory_import_row` más `ON CONFLICT DO NOTHING` | Ya rechazado en la hoja de ruta; además el diario no decide nada, sólo registra. |

## Interacciones

- **import-linear (81).** Cambia los índices y la forma de `PRESENT_SQL`/`TWIN_SQL`; no cambia qué se inserta. El diario no depende de ellos. Si 81 agrega una migración, los números de migración se asignan por orden de llegada; esta exploración los llama «la siguiente libre» y la prueba de checksums fijados (`migrations.rs:44-70`) exige una fila por migración.
- **import-decision-pass (82).** Reescribe `write`, `place` y `place_rows`. Condición para el diario: la escritura del diario va **junto a la inserción real** (`INSERT_SQL`, hoy `:602`) y a `REVISED_SQL` (hoy `:709`), no a la decisión. Hoy el pase de decisión corre en una transacción que se descarta (`:966-977`); si 82 la conserva, las escrituras del diario ahí se deshacen solas, pero cuestan, y si 82 la elimina deben ir detrás de la bandera de escritura. Si 82 pasa a inserciones por lote, el diario las sigue en la misma sentencia o transacción. El número de importación se asigna en la primera transacción de escritura, no en el pase de decisión.
- **import-job (87).** El número de importación es el del trabajo: `memory_import` es la tabla que 87 extiende (estado, progreso) en lugar de crear otra. Esta exploración sólo agrega `started_at` y `finished_at`; no diseña el trabajo. Deshacer una importación **en curso** debe rechazarse (el daemon sabe cuáles corre); una cortada tiene `finished_at` nulo y se puede deshacer.
- **import-pieces (83).** No cambia el esquema. Si cambia cómo se parten las notas, un deshacer de una importación hecha con el corte anterior sigue funcionando: el diario guarda ids, no textos.
- **`purge`.** Borra filas sin pasar por el diario; el diario queda con ids huérfanos y el deshacer los ignora (como `memory_move`). Un `purge` de una fila importada no vuelve a ser deshecho. `purge` ya quita pares y derivaciones de la fila, por lo que `memory_import_extra` los ve ausentes y los salta.
- **`delete` / `restore`.** Una fila importada borrada o restaurada tiene otro `deleted_at` o `update_count` (`admin.rs:565-566`): se conserva. Restaurar suma `update_count`, así que una fila restaurada tampoco se retira.
- **Reviviscencia (`revival.rs`).** El deshacer no pasa por ella: restaura por diario lo que la importación reemplazó (paso 1) y sólo cuando la fila que lo reemplazó se va. Una fila previa que una fila borrada por el usuario había traído de vuelta queda como está.
- **Cadenas de `topic_key`.** Las filas insertadas bajo una clave previa (`TopicTaken`, `Older`) se retiran sin tocar a quien las reemplazaba. La fila viva de una clave que un guardado posterior reemplazó (`cause = 'revision'`) se retira y la nueva se queda.
- **Reimportar el mismo archivo.** Tras un deshacer completo vuelve a insertar todo con otro número. Las filas conservadas siguen presentes y no se duplican. Una segunda importación sin inserciones no tiene número: no hay nada que deshacer.
- **`--dry-run`.** El de la importación no escribe diario ni número. El de `--undo N` corre la misma decisión y no escribe ni snapshot (`relocate.rs:578`).
- **`restore` de un snapshot.** El diario vive en `memory.db`: un store restaurado trae el diario que tenía. Un snapshot anterior a la importación no conoce ese número.
- **`export`/`import` y `doctor`.** El diario no viaja en un export (es del store, como el outbox). `doctor` no necesita una regla nueva: ids huérfanos son esperables. Si se quiere, un aviso de importaciones con `finished_at` nulo.
- **Especificaciones:** delta de `openspec/specs/` de portabilidad (diario, criterio, `--undo`) y del store (tablas).

## Alcance candidato

1. Migración con las tablas del diario y prueba que siembra la versión anterior, comprueba que las filas siguen alcanzables y que las tablas nuevas están vacías; fila en la prueba de checksums fijados.
2. Escritura del diario en la transacción de cada lote de `write` y en el de los vínculos (inserciones, reemplazos, rellenos, pares, derivaciones, piezas, proyectos), con `import_id` en el informe y su línea en la CLI.
3. `undo_import` con decisión pura, lotes acotados y su informe; `Reason::PreUndo`.
4. `POST /v1/import/undo` y `clawg import --undo N [--dry-run]`, con textos en inglés y español.
5. Actualizar `.claude/CLAUDE.md` y los specs.

### No-alcance

- Sustituir o recortar el snapshot `pre-import`.
- Deshacer importaciones hechas antes de este cambio (no tienen diario; sólo les queda el snapshot).
- Un trabajo asíncrono de importación (87) y su progreso.
- Cambiar qué decide la importación (82) o cómo se parten las notas (83).
- Limpiar textos de snapshots, `workspace.md` y salidas de export que ya contienen lo importado (lo hace `purge`).
- Una regla de `doctor` sobre el diario.

## Decisiones abiertas

Son del usuario.

- [ ] **Borrado real o lógico.** Real (recomendado: permite reimportar y saca el texto; la vuelta atrás es el snapshot `pre-undo`) o lógico (reversible con `undelete`, pero bloquea la reimportación y no corrige `find`).
- [ ] **Rigor del criterio.** El propuesto (conservar si hay edición, borrado, restauración, veredicto o referencia externa) u otro más estricto que también conserve filas con `supersede_cause = 'revision'` o filas movidas (más seguro, deja más basura) o más laxo que ignore las referencias de síntesis (menos basura, rompe conclusiones).
- [ ] **Conclusiones apoyadas en filas importadas.** Conservar la fila (recomendado, se informa) o retirarla y dejar la fuente como `purged`, como hace `purge` (`secrets.rs:227-232`).
- [ ] **Proyectos creados por la importación.** Retirarlos si nada los usa (recomendado) o dejarlos siempre.
- [ ] **Relleno de vínculos en filas presentes.** Registrarlos en `memory_import_extra` (recomendado: completo, costo pequeño, sólo cuando `changes() = 1`) o documentar que una reimportación sobre filas ya presentes no se deshace.
- [ ] **Importación cortada.** Deshacible con `finished_at` nulo (recomendado) o sólo tras la marca de fin.
- [ ] **Nombre de la importación y del motivo del snapshot.** `import_id` y `pre-undo` (propuestos), o `pre-import-undo`.

## Cómo se mide

Tests RED (cada uno falla hoy; la mayoría en `crates/clawg-core/src/memory/portability/import.rs` y en un módulo `undo`, y el flujo completo en `crates/clawg-cli/tests/api.rs`/`binary.rs` con el embedder falso):

1. **Número de importación.** Importar un archivo devuelve `import_id` y escribe una cabecera; un dry run, y una segunda importación sin inserciones, devuelven `None`. Hoy el campo no existe.
2. **Round-trip de un archivo clawg.** Importar y deshacer deja `memory`, `memory_vectors`, `memory_fts`, `memory_piece`, `memory_derivation`, `memory_conflict` y `project` como antes (comparando un volcado completo), y una nueva importación del mismo archivo inserta lo mismo.
3. **Round-trip de una exportación ajena** (engram), con relaciones: el store vuelve a su estado previo, pares incluidos.
4. **Lo posterior se conserva.** Se guardan filas nuevas después de importar; tras deshacer siguen, con sus pares con filas no importadas.
5. **Reemplazo por reimportación ajena.** Importar, reimportar con una nota reescrita (más nueva), deshacer la segunda: la fila vuelve a `active` con `superseded_by` y `supersede_cause` previos (`REVISED_SQL`). Deshacer la primera después retira todo. Con la fila de reemplazo conservada, no se restaura el reemplazado.
6. **Relleno en fila presente.** Un export del mismo store importado sobre filas orgánicas rellena un vínculo; deshacer lo deja vacío, y no si el valor cambió.
7. **Criterio.** Una fila editada (`update_count`), borrada, restaurada, con veredicto, o citada por una conclusión, se conserva y se nombra con su motivo; una movida de proyecto, accedida, demovida de `tier` por mantenimiento o reemplazada por una clave posterior (`revision`) se retira.
8. **Pares con filas previas.** Un par `escalated` entre una fila importada y una previa desaparece al deshacer; la fila previa queda `active` y no `disputed`.
9. **Proyectos.** Una raíz creada por la importación se retira si nada la usa y se conserva si tiene filas, índice de docs o `index_docs = 1`.
10. **`purge` previo.** Si una fila importada fue purgada, el deshacer no falla y el resto se retira.
11. **Idempotencia y errores.** `--undo N` dos veces: la segunda no hace nada; `--undo 99` es `UnknownImport`; deshacer una importación en curso se rechaza; `--undo` con `file` es un error de uso (`Cli::parse_args`).
12. **`--dry-run`** informa lo mismo y no escribe nada ni snapshot; el real escribe `pre-undo` antes.
13. **Migración.** Un store de la versión anterior (sembrado como en `db.rs:1625-1664`) conserva sus filas y gana las tablas vacías; la prueba de checksums fijados incluye la nueva fila; el nombre de `pre-undo` va y vuelve (`backup.rs:680-693`).
14. **Reimportación tras deshacer** inserta de nuevo y el informe lo cuenta.

**Medición** (sin tocar los perfiles reales): importar el archivo de banco con el embedder falso en un `CLAWG_TEST_HOME` temporal (el de 2 000 notas y 15 047 filas de la hoja de ruta), medir el tiempo de la importación con y sin diario (el diario no debe cambiar la complejidad) y el de `--undo` en lotes; el resultado esperado es lineal. La importación real de 52 080 filas fija el orden de magnitud del borrado, que no está medido. El criterio de aceptación es que el deshacer de ese banco deje el volcado idéntico al anterior a importar. No se corre `rag-eval`: no hay cambio de ranking.

## Riesgos

- **Un deshacer que borra de más.** La pérdida de datos del usuario es el riesgo mayor. Se acota con el criterio conservador, con el snapshot `pre-undo` y con `--dry-run`; la decisión es una función pura con casos por cada motivo de conservación.
- **Diario incompleto.** Si una escritura nueva de la importación (de 82 o de otro) olvida registrar, el deshacer deja residuos. La prueba 2/3 compara el volcado completo y fallará por tabla.
- **Costo del diario en la importación.** Una escritura por fila insertada en la misma transacción; debe medirse que no cambia la forma de la curva que 81 y 82 arreglan.
- **Un deshacer largo ocupa la única conexión.** Es el mismo problema que 82 resuelve en la importación; el deshacer va en lotes con una decisión basada en conjuntos, no un `conn.call` único.
- **Ids huérfanos y reutilización de ids.** `AUTOINCREMENT` no reutiliza ids (`secrets.rs:229-230`), por lo que un id del diario nunca apunta a otra fila.
- **Colisión de números de migración** con 81 si ambas agregan una; se resuelve al integrar.
- **Textos que sobreviven.** Los snapshots posteriores, `workspace.md` y los exports ya hechos conservan lo importado; el deshacer no los toca (como `purge`, que al menos los nombra; una decisión de no-alcance).
