# project-reassign — Diseño

Los enfoques de fondo (A a D) están en la [exploración](exploration.md#enfoques-posibles); se eligió A, un motor con dos puertas. Aquí van las decisiones de implementación.

## D1. Un módulo, tres pasos

`crates/clawg-core/src/memory/relocate.rs`:

1. **Leer** (`read_units_in`), dentro de la transacción: para cada fila sembrada, en orden de id, su unidad (un recorrido en anchura por `memory_piece`, `superseded_by` con causa `revision` en los dos sentidos y `revived_from` en los dos sentidos), cada fila con su proyecto, agente, `scope`, si está viva o borrada, su tipo, su hash y su clave, y los titulares que el destino tiene al alcance del lector para los hashes y las claves vivas de la unidad. Una semilla que ya cayó en una unidad anterior no abre otra.
2. **Decidir** (`decide`), pura: con las unidades leídas y el destino, devuelve qué filas se mueven (con su `scope` de destino), cuáles quedan igual y qué unidades se omiten y por qué. Lleva un libro de los hashes y las claves que las unidades aceptadas llevan al destino, para que dos del mismo lote choquen entre sí como chocarían con lo guardado (el `Ledger` del import, `crates/clawg-core/src/memory/portability/import.rs`).
3. **Aplicar** (`apply_in`), dentro de la misma transacción: el número del movimiento es `MAX(move_id) + 1`; por fila, una inserción en `memory_move` y un `UPDATE memory SET project_id, scope`.

Leer y decidir son lo mismo para un ensayo y para un movimiento: el ensayo hace los dos pasos en una transacción que no se confirma (como el primer pase del import), así que PR7 se cumple por construcción.

| Alternativa | Por qué no |
|---|---|
| Un CTE recursivo para el cierre | Cuatro relaciones en dos sentidos dan un CTE difícil de leer y de probar; las unidades reales son de pocas filas, y el recorrido usa los índices de `memory_piece`, `superseded_by` y la clave primaria |
| Decidir en SQL | La regla de choques con el libro del lote y el orden «se mueve la primera» quedan en una función pura que se prueba sin base |

## D2. Choques con el alcance del lector

Los titulares del destino se buscan con `reader_predicate!` sobre el lector de la llamada: el usuario (sin lector) choca con toda fila, un agente sólo con lo que alcanza, como `RESTORE_TWIN` y `RESTORE_TOPIC_HOLDER` (`crates/clawg-core/src/memory/admin.rs`). Es la regla que dejó 76 para comparar un save con lo guardado. Una fila del destino que la política le oculta al agente no se nombra; si queda un duplicado, `clawg doctor` lo marca (`memory.topic`, `memory.content_hash`), igual que con un save.

Una fila de la unidad fuera del alcance rechaza la unidad entera (`out_of_reach`), sin nombrarla: moverla sin ella partiría el save, y nombrarla diría que existe.

## D3. `PATCH` compone el movimiento con el cambio de siempre

`MemoryPatch` suma un destino opcional (`Option<Option<String>>`: ausente, a un proyecto o a ninguno); `MemoryPatch::new` lo valida con el resto (vacío, destino en una tabla que no es `memory`). `update_memory_core` lee la fila como hoy y, si hay destino, lee y decide la unidad de esa fila en la misma transacción; cualquier rechazo (tipo protegido, unidad protegida sin `user_answered`, choque, destino no registrado) vuelve antes de escribir. `UpdatedRow` suma `project_id` y, cuando movió, `move_id` y `pulled_in`.

| Alternativa | Por qué no |
|---|---|
| Un endpoint propio para mover una fila | Dos llamadas para «es una decisión y va en B» que no serían atómicas; el `PATCH` ya es la puerta de las correcciones de metadatos |

## D4. El lote

`move_memories_core(store, backups, &MoveFilter, target, dry_run)` y `undo_move_core(store, backups, move_id, dry_run)`. Cada uno decide primero en una transacción que no confirma; si no hay nada que mover, responde ese reporte. Si hay, toma el snapshot `pre-move` (`MemoryStore::snapshot`, `Reason::PreMove`) y repite leer, decidir y aplicar en una transacción que confirma: lo que se guardó entre los dos pases se ve. La selección es SQL con parámetros ligados, como `push_filters`; `q` usa la expresión del visor (`viewer_match_expression`) sobre `memory_fts`, que sólo indexa filas vivas, por eso no va con `include_deleted`.

`undo` lee de `memory_move` las filas del movimiento que siguen en su destino (`project_id IS to_project`) y las agrupa por origen; cada grupo es un movimiento hacia su origen con las mismas reglas, sin pedir que esté registrado (puede ser un id anterior al registro de raíces o «sin proyecto»), y todos van en un mismo movimiento nuevo.

El reporte (`MoveReport`): `move_id`, `moved` (cada fila con su origen y su destino), `pulled_in`, `unchanged`, `skipped` (la primera fila seleccionada de la unidad y el motivo), `split_pairs`, `protected` y `snapshot`.

## D5. V21

```sql
CREATE TABLE IF NOT EXISTS memory_move (
    move_id      INTEGER NOT NULL,
    row_id       INTEGER NOT NULL,
    from_project TEXT,
    from_scope   TEXT    NOT NULL,
    to_project   TEXT,
    to_scope     TEXT    NOT NULL,
    moved_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_move ON memory_move (move_id);
```

Sin clave foránea a `memory`: `purge` borra filas y el registro de un movimiento viejo no debe impedirlo; deshacer ignora las filas que ya no están. `project_rekey` no sirve (`legacy_id` y `project_id` son `NOT NULL`, y no dice nada de `scope`).

## D6. El puente

`UpdateMemoryArgs` suma `to_project_path` y `clear_project`. El nombre no es `project_path` porque en las otras herramientas su ausencia significa «el proyecto del puente», y aquí tiene que significar «no cambiar». El puente resuelve la ruta con `ProjectIdentity::resolve`, rechaza una raíz que no es un directorio y la registra con `project_for` (que ya sabe de colisiones), y manda el `PATCH` con `project_id`. `format_update` nombra dónde quedó la fila y lo que se movió con ella.

## D7. El CLI

`ProjectsAction::Move(MoveOpts)` en `crates/clawg-cli/src/cli.rs`, con un grupo de clap que exige un destino o `--undo`, y `MoveOpts::request` puro (como `Scope::project_id`) que arma el pedido a partir de los argumentos y el proyecto del directorio actual o de `--to PATH` ya resueltos. `crates/clawg-cli/src/projects.rs` resuelve y registra `--to PATH` como `add` (sin `index_docs`) y manda el pedido con el cliente. `--no-project` va en `Filters`, que comparten `list` y `search`.
