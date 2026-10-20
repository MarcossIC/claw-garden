# memory-portability — Propuesta

| Objetivo | Tamaño | Depende de |
|---|---|---|
| 1, 2 | M | — (los cambios de esquema que esperaba, 24, 27, 36, 18 y 49, ya están) |

## Problema

La única copia de la memoria es `memory.db`, y sólo se mueve entera: `snapshot_into` usa `VACUUM INTO` (`crates/clawg-core/src/memory/backup.rs:305`) y `restore` reemplaza el archivo (`backup.rs:362`). Un snapshot sólo lo abre un binario que entienda su esquema (`schema_plan`, `backup.rs:93`). Con eso no se puede llevar un proyecto o un agente de un perfil a otro, leer la memoria sin `clawg`, ni traer memorias de otra fuente (la puerta que necesita history-import, 44).

El save común no sirve para importar: el insert fija `access_count = 1`, `last_accessed_at = created_at`, calcula el tier y sólo deja `active` o `disputed` (`PreparedRow::new` e `insert`, `crates/clawg-core/src/memory/store_insert.rs:507-586`). Las relaciones usan ids locales: `superseded_by`, `disputed_against` y `revived_from` en `memory` (`migrations/V1__init.sql:24-28`, `V17__revival.sql:14`), `memory_derivation` (`V10__synthesis.sql:8`), `memory_piece` con un `save_id` propio (`V11__memory_piece.sql:11`) y `memory_conflict` (`V15__memory_conflict.sql:11`). El `project_id` es un hash de la ruta raíz (`crates/clawg-core/src/memory/identity.rs:11`).

## Decisiones

Tomadas sobre las [decisiones abiertas](exploration.md#decisiones-abiertas):

- **El import escribe a través del daemon**, el único escritor, que además tiene el modelo para re-embeber. Necesita un daemon en marcha, como `undelete`.
- **Borradas:** `export --include-deleted` las incluye y el import las repone borradas, recuperables con `undelete`.
- **Agente:** se conserva el `agent_id` de origen; `import --agent A` lo reasigna en cada fila que tenía agente (una fila `project` o `global` sigue sin agente).
- **Accesos, tier e importancia se conservan**, como fechas, estados, cadenas, piezas, derivaciones y pares.
- **El detector no corre** sobre lo importado: los pares del origen viajan en el archivo.
- **Sin `redact` al exportar:** el archivo lleva el texto como está guardado. El import sí enmascara, como todo camino de escritura.
- **Sin vectores en el archivo:** el import siempre re-embebe.
- **Markdown entra** como formato de salida de `export`, sólo para leer.

## Alcance

1. Formato `clawg-memories` versión 1 en JSON Lines, con su parser y su escritor puros y un test que fija un ejemplo.
2. `clawg export --out FILE [--project PATH] [--agent A] [--include-deleted] [--format jsonl|markdown]`: lee `memory.db` sin el lock (funciona con el daemon en marcha, como `secrets`), escribe el archivo sólo para el dueño y avisa de las saves pendientes en el outbox.
3. `clawg import FILE [--dry-run] [--agent A] [--map-root OLD=NEW]...` por `POST /v1/import`: snapshot `pre-import`, reasignación de ids, `project_id` recalculado desde `root_path`, re-embebido por lotes, sin duplicar en un segundo import.
4. El reporte, también en `--dry-run`: filas por tipo, estado y proyecto, cuántas ya estaban, cuántas se fundieron con una igual viva, cuántas quedaron reemplazadas por una clave de tema ya tomada, las inválidas y los vínculos que no se pudieron resolver.
5. Docs: `.claude/CLAUDE.md` (comandos y API) y la spec `specs/portability`.

### No-alcance

- Sincronización.
- Importar desde otras herramientas (44).
- Exportar vectores, el índice de docs, `sharing.json`, `synthesis_review` o el outbox.
- `episodic_trajectory` y `procedural_skill`: nada los escribe desde que no está la app de escritorio (`store_insert.rs:589-592`); el export los cuenta y dice que no viajan.

## Cómo se mide

Tests que fallan hoy (no existe el comando ni la ruta):

1. Ida y vuelta de una base sembrada (los siete tipos, una save en piezas, una conclusión, una cadena `superseded` con su causa, una fila revivida, un par `escalated` y uno juzgado, una borrada, un proyecto con `root_path`) a una base vacía: se conservan conteos por estado, tipo y proyecto, cadenas, vínculos, accesos y tier, y cada fila viva se alcanza por búsqueda.
2. Un segundo import del mismo archivo no agrega filas ni suma accesos.
3. Una fila inválida no impide el resto y se reporta con su línea.
4. Una versión de formato desconocida se rechaza sin escribir ni tomar snapshot.
5. `--map-root` da el `project_id` de la raíz de destino.
6. El import deja un snapshot `pre-import`; `--dry-run` no escribe nada.
7. Una base sembrada en V16 (antes de `supersede_cause` y `revived_from`) se exporta, y lo importado conserva lo mismo.

`rag-eval` no cambia: el ranking no se toca.

## Migración y vuelta atrás

Sin cambio de esquema. El import escribe filas nuevas y sólo completa vínculos vacíos de las que ya estaban; antes de escribir deja el snapshot `pre-import`, y `clawg restore` vuelve a él.
