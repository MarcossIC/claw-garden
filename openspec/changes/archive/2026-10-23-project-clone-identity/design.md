# project-clone-identity — Diseño

Los enfoques de fondo están en la [exploración](exploration.md#enfoques-posibles); se eligieron A (remoto como pista), W3, Q1 e I1. Aquí van las decisiones de implementación.

## D1. La clave, en `identity.rs`

Tres funciones puras y una que lee:

- `origin_url(config: &str) -> Option<String>`: un parser mínimo de las líneas de `config` de git: secciones `[remote "origin"]` (el nombre de sección sin mayúsculas, la subsección exacta), `url = valor`, comentarios y comillas. No sigue `include`, `includeIf` ni `insteadOf`: lo que da una URL distinta por configuración del usuario daría una clave distinta, nada peor que hoy.
- `normalize_remote(url: &str) -> Option<String>`: la tabla de CI1.
- `git_config_dir(dir, marker) -> PathBuf`: dónde está el `config` para lo que `read_marker` leyó (`GitMarker::Dir` → `<dir>/.git`; `Link` con `commondir` → el directorio común; sin él → el `gitdir`).
- `origin_key(root: &Path) -> Option<String>`: lee el marcador de la raíz y el `config`, y aplica las dos.

La lee el daemon al registrar (el cliente sólo manda la raíz) y `export`, que no habla con el daemon. Así un puente, un hook o un comando nuevo no repiten la lectura, y la clave nunca viaja por HTTP.

## D2. V24

```sql
CREATE TABLE IF NOT EXISTS project_remote (
    remote      TEXT    PRIMARY KEY,
    project_id  TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS memory_move_supersede (
    move_id         INTEGER NOT NULL,
    row_id          INTEGER NOT NULL,
    superseded_by   INTEGER NOT NULL,
    from_status     TEXT    NOT NULL,
    from_superseded_by INTEGER,
    from_cause      TEXT
);
CREATE INDEX IF NOT EXISTS idx_project_root ON project (root_path);
```

`project` no cambia de forma: cada fila sigue siendo una raíz con su id (el id de raíz). El id de proyecto de una raíz con clave es siempre el id de raíz de otra fila de `project` (la primera que trajo la clave), así que «el destino está registrado» (R10) se sigue cumpliendo sin tocar `check_target_in`.

| Alternativa | Por qué no |
|---|---|
| Una columna `remote` en `project` | Una raíz puede cambiar de remoto y un remoto tiene varias raíces; el mapa clave → proyecto es una relación propia |
| Un `PRIMARY KEY (root_path)` | Cambiar la clave primaria de `project` pide reconstruir la tabla; con la asignación en una transacción de la conexión única no hay dos filas para una raíz, y el índice hace barata la búsqueda |
| Ensanchar el hash (W2) | Lo descartó el usuario: no cubre dos raíces sin remoto que chocan |

## D3. `register_project`

Firma: `register_project(store, workspaces, root, legacy_ids, index_docs, assigned) -> RegisterOutcome { project_id, root_id, rekeyed, index_docs, move_id, snapshot }`. La clave la calcula el handler (`origin_key`) y entra como argumento (`remote: Option<&str>`), para que los tests de `project.rs` no necesiten un `.git`.

Pasos, en dos pases como `run_batch` (`relocate.rs`):

1. **Decidir** en una transacción que no se confirma: id de raíz (`root_id_in`: la fila de la raíz, o el primer candidato libre), id de proyecto (`project_remote` o el de raíz), los ids viejos (los del cliente, más el de raíz si difiere del de proyecto, menos el de proyecto y los que son id de raíz de otra raíz) y si hay filas que mover.
2. Si hay filas, el snapshot `pre-rekey`.
3. **Escribir** en una transacción que se confirma, decidiendo otra vez: la fila de `project` (y la marca `index_docs`, que sólo sube), la de `project_remote` si falta, la unión de `memory` (D4) y `project_rekey` para las otras dos tablas.

Con `assigned = false` el paso 1 es la regla de antes: el id es `derive_project_id(raíz)`; si la raíz tiene otra fila o el id es de otra raíz, `RootCollision`. No se lee `project_remote`. La unión de `memory` sí pasa por D4: un id viejo de un subdirectorio que repite un texto de la raíz también se reconcilia.

`RegisterError::RootCollision` sigue: sólo lo da el modo anterior. `TooManyCandidates` es el error nuevo de CI2.

Un puente abierto antes de un `update` sigue con `derive_project_id` hasta que se reinicia. Si en ese rato un cliente nuevo une su raíz a la de otro clone, esa sesión deja de ver las filas que se movieron (siguen en el store, bajo el id del proyecto) y guarda las nuevas bajo el id de raíz, que el próximo registro nuevo une. Es la ventana que ya tiene `version_warning`; no se pierde nada.

## D4. Unir en el motor de mover

`relocate.rs` suma `merge_in(c, from: &[String], target: &str, now) -> MoveReport`:

1. Siembra con todas las filas de `memory` de los ids de origen, borradas incluidas, y lee las unidades como siempre (`read_units_in`, sin lector).
2. `decide_merge`: como `decide`, pero sin `live_twin` ni `topic_taken`; una unidad con filas fuera del origen y del destino es `spans_projects`.
3. `apply_in` mueve y registra (un número de movimiento).
4. `reconcile_in(c, target, moved, move_id)`: lee las filas vivas del destino de los grupos de clave y de texto que tocan una fila movida, con su `created_at`, su save (`memory_piece`, la pieza 1 es la primera fila) y decide con una función pura, `reconcile(rows) -> Vec<Supersede>`, lo de CI5. Escribe cada reemplazo con un `UPDATE` de `status`, `superseded_by` y `supersede_cause` (no `SUPERSEDE_SQL`, que también toca `revived_from` y `last_modified_at`, y el deshacer no podría devolverlos) y su fila en `memory_move_supersede`.

El ganador de una clave es el más nuevo (como `revise_in`, `crates/clawg-core/src/memory/topic.rs:118`): unir dos clones es simétrico y no debe depender de cuál se abrió primero. El de un texto, el más viejo, como el refuerzo de la dedup (`insert_unless_twin`). La causa `revision` deja al revivir hacer lo de siempre: borrar la versión vigente de una clave trae la anterior, que ahora está en el mismo proyecto.

| Alternativa | Por qué no |
|---|---|
| `revise_in` por clave | Supersede por agente de alcance, cambia `tier`, `access_count`, `revived_from` y `last_modified_at`, y libera disputas: nada de eso se deshace con un registro |
| Rechazar la unidad (el motor tal cual) | Deja la unidad bajo un id que ninguna sesión lee: esconde memorias |
| Borrar el duplicado | Un borrado no vuelve con el revivir y `undelete` lo rechaza si el texto está vivo |

`undo_move_core` lee primero `memory_move_supersede` del movimiento y devuelve cada fila que sigue `superseded` por la que anotó; después mueve de vuelta como siempre.

## D5. HTTP y el cliente

`RegisterRequest` suma `assigned: bool` (`#[serde(default)]`); `RegisteredBody` suma `root_id`. El handler calcula `origin_key(root)` sólo con `assigned`. El refresco de docs se lanza con `root_id`.

`DaemonClient::register_project` pasa a devolver `Registered { project_id, root_id, rekeyed }`, con `root_id` igual a `project_id` cuando el daemon no lo manda (uno anterior), y siempre manda `assigned: true`.

## D6. El puente

`registered` pasa de `HashSet<(raíz, id viejo)>` a `HashMap<(raíz, id viejo), Ids>`. `project_for` devuelve `ProjectIds { project, root }` y las herramientas de memoria toman `project`, `search_project` toma `root`. Un registro que falla por otra causa deja `Ids` derivados de la raíz (lo de antes). `save_project` (la clave que muestra un hit) usa el mapa si la raíz ya se registró, si no `derive_project_id`.

## D7. Hooks, comandos, export e import

- `hook.rs`: registra la raíz del `cwd` con `index_docs: false` antes de leer, dentro del mismo tope de 3 s. Un 409 (un daemon anterior con un choque) no imprime nada; otro error lee con el id derivado.
- `projects.rs`: `add`, `move --to` y `--here` toman el id del registro.
- `export --project`: `project::lookup_project_id(conn, root, remote)` sobre la conexión de sólo lectura del export.
- import: `prepare` deja de calcular ids; `record_projects` asigna cada raíz con `assign_in` (D3, paso 1, en la transacción del import) y su clave por `origin_key`. Ya no hay `taken`: un id asignado nunca es de otra raíz.

## D8. `doctor`

`projects` compara el id con `candidate_id(raíz, n)` para `n` de 0 a 63 (`is_candidate_for`); sigue siendo puro.
