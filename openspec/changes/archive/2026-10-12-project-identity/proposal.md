# project-identity — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 | M | data-preservation parte B (snapshot antes de migrar) | archivado |

## Problema

El mismo repositorio termina con varios `project_id` según cómo se lo nombre, y sus memorias quedan partidas entre ellos sin aviso. La exploración lo verificó contra el código; acá va lo que decide el alcance.

- `derive_project_id` hashea los **bytes crudos** de la ruta: `{basename}-{4 hex de SipHash-1-3}` (`crates/clawg-core/src/memory/identity.rs:11-15`).
- La ruta sale de cuatro lugares, y ninguno la normaliza:
  - el cwd del puente, leído una vez al arrancar (`crates/clawg-cli/src/mcp/server.rs:165-167`);
  - `project_path` de cada tool MCP (`crates/clawg-cli/src/mcp/tools.rs:145-147`);
  - `clawg memories --here` (`crates/clawg-cli/src/cli.rs:150-152`, cwd leído en `crates/clawg-cli/src/memories.rs:348`);
  - el extractor y el episodic writer, hoy inactivos (`crates/clawg-core/src/memory/extractor.rs:233-238`, `episodic_writer.rs:229-234`).
- Cada una de estas variantes da un id distinto: caja, separadores, separador final, ruta relativa, subdirectorio y worktree de git.
- El store guarda sólo el id, nunca la ruta (`crates/clawg-core/migrations/V1__init.sql:9`). Un id viejo no se puede volver a convertir en ruta.
- `workspace.md` vive en un directorio por `project_id` (`crates/clawg-core/src/memory/workspace.rs:42-46`). Su sección USER, escrita a mano, también queda partida.

## Decisiones (cierran las abiertas de la exploración)

**La raíz de un proyecto es la raíz git, resuelta desde la ruta canónica.**

- Se canonicaliza la ruta con `std::fs::canonicalize` y se le quita el prefijo `\\?\` en Windows. En Windows, `canonicalize` devuelve la caja real del disco, así que `C:\X` y `c:\x` quedan iguales sin pasar todo a minúsculas.
- Desde ahí se sube hasta el primer directorio que tenga `.git`, sea directorio o archivo.
- Si la ruta no existe, se normaliza léxicamente (separador final, `.` y `..`) y no se busca raíz git.

**Worktrees: comparten las memorias del repo principal.** Un worktree es el mismo proyecto en otra rama, y Claude Code los usa para sus agentes. El `.git` de un worktree es un archivo `gitdir: <repo>/.git/worktrees/<n>`, y ese directorio tiene un `commondir` que apunta a `<repo>/.git`. La raíz es el padre de ese directorio común. Un submódulo (`gitdir: <repo>/.git/modules/<n>`, sin `commondir`) es su propio proyecto.

**Directorios sin git: la ruta canónica es la raíz.** Es lo que pasa hoy, ya normalizado. "Sin proyecto" escondería las memorias de quien trabaja fuera de git.

**El formato del id no cambia:** `basename-hash4` sobre los bytes de la raíz canónica. En el caso común, el puente arranca en la raíz del repo con la ruta tal como la devuelve el sistema, y el id nuevo es igual al viejo: nada que re-keyear. Ampliar el hash cambiaría todos los ids para resolver colisiones, que no son el problema.

**Re-key perezoso, registrado fila por fila.**

- Cuando un puente resuelve una ruta cuyo id viejo (el de la ruta cruda, como hoy) difiere del nuevo, le pide al daemon que pase las filas del id viejo al nuevo.
- El daemon lo hace en una transacción sobre `memory`, `episodic_trajectory` y `procedural_skill`, y anota cada fila movida en `project_rekey` (tabla, id de fila, id viejo, id nuevo, fecha). Revertir es un `UPDATE` desde esa tabla.
- `memory_fts` no necesita nada: es external-content y lee `project_id` de `memory` al consultar (`crates/clawg-core/migrations/V8__memory_fts.sql:20-22`). Las tablas vec0 no tienen `project_id` (`V2__vectors.sql`).
- El índice de dedup no es UNIQUE (`V6__content_hash.sql:9-16`), así que dos ids que se unen no rompen ninguna restricción.

**Memorias guardadas desde subdirectorios:** su id viejo sólo se recupera cuando el puente vuelve a arrancar en ese subdirectorio. No se agrega un comando `clawg projects merge` en este cambio. Queda en el backlog, y `project_rekey` ya deja el lugar para registrarlo.

**Rollback:** `project_rekey` guarda el mapeo fila por fila. Además, la migración que crea las tablas corre después del snapshot automático de la parte B de data-preservation.

## Alcance

1. Función `resolve_project_root(path)` en `clawg-core`, con la decisión pura separada del filesystem: normalización léxica, lectura del archivo `.git` y de `commondir`, y elección de la raíz.
2. `ProjectIdentity::resolve(path)`: la raíz, su id y el id viejo de la ruta cruda. La usan el puente (cwd y `project_path`) y `clawg memories --here`.
3. Migración nueva: tablas `project` (`id`, `root_path`, `created_at`) y `project_rekey`.
4. `POST /v1/projects` en el daemon: registra la raíz y re-keyea los ids viejos que el puente manda. Es idempotente.
5. El puente registra cada raíz una sola vez por proceso, antes de la primera tool que la usa.
6. `workspace.md`: si el directorio del id nuevo no tiene `workspace.md` y el del viejo sí, se **copia** el viejo. El viejo nunca se borra.

## Fuera de alcance

- Un segundo clon del mismo repo: otra ruta, otro proyecto. Unirlos necesitaría la URL del remoto como identidad.
- `clawg projects merge` para ids viejos de subdirectorios donde el puente no vuelve a arrancar.
- Qué pasa si falla `current_dir()`: sigue siendo `None`, saves con `project_id` NULL y visibles en todos lados. Es raro, y fallar el save sería peor. Queda fijado con un test.
- Las rutas crudas que el miner escribe en `procedural_skill.project_id` (`crates/clawg-core/src/memory/miner.rs:537`): es código inactivo, va con knowledge-synthesis.
- El extractor y el episodic writer: siguen derivando de la ruta cruda. Se pasan a la función nueva cuando se activen.

## Éxito

`cargo test --workspace` sin fallas nuevas contra la base. Tests que fallan hoy:

- `C:\X\repo` y `c:\x\repo\` dan el mismo id (Windows); `repo/` y `repo` dan el mismo id; `repo/src/lib` da el id de `repo`; un worktree da el id del repo principal; un directorio sin git da el id de su ruta canónica.
- La raíz canónica de una ruta ya canónica da el mismo id que `derive_project_id` hoy (ids comunes estables).
- Una base sembrada con filas en un id viejo, en las tres tablas, las muestra bajo el id nuevo después del registro; una segunda corrida no mueve nada; `project_rekey` alcanza para dejarlas como estaban.
- `workspace.md` del id viejo aparece bajo el id nuevo y el viejo sigue existiendo.

El ranking no cambia: no hace falta `rag-eval`. El golden set no tiene `project_id`.

## Datos guardados

- **Esquema:** dos tablas nuevas. Ninguna columna existente cambia.
- **Identificadores:** las filas de un id viejo pasan al nuevo sólo cuando un puente nombra esa ruta. Ninguna se borra ni se oculta: una fila re-keyeada sigue visible desde la raíz del proyecto, que es justo lo que antes no pasaba.
- **Migración:** la nueva entra en `PINNED` (`crates/clawg-core/src/memory/migrations.rs:44`) y corre agrupada con las demás (`specs/storage`, "Migraciones atómicas").
- **Rollback:**
  - de datos: `UPDATE <tabla> SET project_id = legacy_id FROM project_rekey ...` (el SQL exacto queda en `design.md`);
  - de binario: una versión anterior abre la base, porque no conoce las tablas nuevas y no las lee. Las filas re-keyeadas quedan visibles para ella cuando el puente arranca en la raíz canónica.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Windows: `canonicalize` no devuelve la caja que asumimos | Baja | Test de integración que lo fija en Windows; queda pendiente en `verify.md` hasta correrlo ahí |
| Un id viejo que no era de este proyecto choca con el de una ruta de este proyecto | Muy baja | Sólo se re-keyea el id de la ruta que el puente acaba de recibir. El choque necesita el mismo basename y los mismos 16 bits |
| Un cliente con token re-keyea ids ajenos | Baja | El mismo token ya permite borrar cualquier memoria. El daemon valida el formato del id, y `project_rekey` permite deshacerlo |
| El usuario edita a mano el `workspace.md` viejo después de la copia | Baja | El viejo no se borra. Se registra un warn con las dos rutas |
