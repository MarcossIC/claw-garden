# project-identity — Diseño

Sólo las decisiones con alternativas reales. Lo que fija la spec no se repite.

## D1. Quién resuelve la raíz y quién re-keyea

| Opción | A favor | En contra |
|---|---|---|
| **a. El puente resuelve; el daemon re-keyea vía `POST /v1/projects`** | El id por defecto se sigue calculando al arrancar, sin esperar al daemon (`crates/clawg-cli/src/mcp/server.rs:163-168`). Sólo el puente conoce la ruta cruda, que es de donde sale el id viejo | Un endpoint nuevo |
| b. El puente manda la ruta cruda en cada tool y el daemon resuelve | Un solo lugar resuelve | Cambia todos los requests (`project_id` en `client.rs:174`, `:243`, `:271`) y el daemon pasa a hacer I/O de filesystem por cada búsqueda |
| c. Sin re-key: el daemon guarda alias y las consultas filtran por `project_id IN (alias)` | No reescribe ninguna fila | El predicado de proyecto está copiado byte a byte entre el KNN y el canal léxico (`crates/clawg-core/src/memory/search.rs:122`, `:180`, `:554`, `:720`, y el comentario de `V8__memory_fts.sql:24-29`), más el dedup (`store_insert.rs:216`) y el workspace (`workspace.rs:231`, `:281`). Cada uno pasaría a una lista |

**Decisión: a.** El re-key toca sólo las filas del id viejo, una vez, y deja todas las consultas como están. La ruta canónica viaja en el registro para que el daemon derive el id él mismo y lo guarde en `project`.

## D2. Cómo se deshace un re-key

| Opción | A favor | En contra |
|---|---|---|
| **a. `project_rekey` fila por fila** | Rollback exacto con un `UPDATE`, aunque varios ids viejos hayan caído en el mismo nuevo | Una fila por memoria movida. En un store personal son cientos o pocos miles, una vez |
| b. Sólo el mapeo `legacy_id → project_id` | Tabla mínima | Si dos ids viejos van al mismo nuevo, ya no se sabe cuál fila venía de cuál |
| c. Sólo el snapshot de data-preservation B | Nada nuevo | Restaurar el snapshot pierde todo lo guardado después del re-key |

**Decisión: a,** y el snapshot de B queda como segunda red. La migración nueva corre después del snapshot automático que B toma antes de migrar.

```sql
CREATE TABLE IF NOT EXISTS project (
    id          TEXT    PRIMARY KEY,
    root_path   TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS project_rekey (
    tbl         TEXT    NOT NULL
                        CHECK (tbl IN ('memory', 'episodic_trajectory', 'procedural_skill')),
    row_id      INTEGER NOT NULL,
    legacy_id   TEXT    NOT NULL,
    project_id  TEXT    NOT NULL,
    rekeyed_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_project_rekey_row ON project_rekey (tbl, row_id);
```

Rollback de datos, por tabla (para `episodic_trajectory` y `procedural_skill`, lo mismo con su nombre):

```sql
UPDATE memory
   SET project_id = r.legacy_id
  FROM project_rekey AS r
 WHERE r.tbl = 'memory' AND r.row_id = memory.id AND r.project_id = memory.project_id;
```

La condición `r.project_id = memory.project_id` evita deshacer una fila que se movió de nuevo después.

## D3. Re-key dentro de una transacción, en el hilo del store

El re-key es una función de `clawg-core` sobre `MemoryStore::call`, la única conexión serializada. Dentro de una transacción:

1. `INSERT OR IGNORE INTO project` y comparar `root_path` con la existente (409 si difiere).
2. Por cada tabla y cada id viejo distinto del nuevo: `INSERT INTO project_rekey SELECT '<tabla>', id, project_id, :nuevo, :ahora FROM <tabla> WHERE project_id = :viejo`, y después `UPDATE <tabla> SET project_id = :nuevo WHERE project_id = :viejo`.
3. Devolver la suma de filas movidas.

Idempotencia sin estado extra: en la segunda corrida ya no hay filas con el id viejo. No hace falta un trigger para FTS (`V8__memory_fts.sql:20-22`) ni tocar vec0, que no guarda `project_id` (`V2__vectors.sql`).

La copia de `workspace.md` (R7) va después del commit y fuera de la transacción: es I/O de archivos y no tiene que poder deshacer el re-key si falla. Si falla, se registra un warn; el archivo viejo sigue ahí y el próximo registro no lo vuelve a intentar, porque ya no hay filas que mover. Para que no se pierda, la copia se decide por los ids viejos recibidos, no por las filas movidas: se intenta en cada registro mientras el destino no exista.

## D4. Separar decisión e I/O en la resolución

`resolve_project_root` hace I/O (canonicalizar, leer `.git` y `commondir`). La decisión se escribe pura:

- `normalize_lexical(&Path) -> PathBuf` y `strip_verbatim(&Path) -> PathBuf`;
- `parse_gitdir(contenido, dir_del_archivo) -> Option<PathBuf>` (rutas relativas al directorio del `.git`);
- `root_from_marker(dir, marker) -> PathBuf`, con `marker` = directorio, worktree (`commondir` leído) o submódulo.

La función con I/O sólo recorre ancestros, lee y llama a esas. No hay trait de filesystem: los tests de la parte pura pasan valores, y los de integración usan directorios temporales (`tempfile` ya es dev-dependency de `clawg-core`).

## D5. Número de la migración

La migración nueva toma el siguiente número libre al momento de aplicar. El cambio de storage (backups, partes B + C de data-preservation) se está escribiendo en paralelo y puede agregar una migración: la que llegue segunda a `main` se renumera y actualiza su fila en `PINNED` (`crates/clawg-core/src/memory/migrations.rs:44`).
