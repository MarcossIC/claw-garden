# project-clone-identity — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 | M | project-root-guard (29), project-reassign (77) | ver [`state.yaml`](state.yaml) |

## Problema

El detalle, con cada lugar citado, está en la [exploración](exploration.md#problema-verificado-contra-el-código). En resumen:

1. **Dos clones del mismo repo son dos proyectos.** El id es `derive_project_id(raíz)`, un hash de la ruta (`crates/clawg-core/src/memory/identity.rs:11-15`), y nada mira el remoto. Quien tiene una copia de trabajo y otra de revisión, o vuelve a clonar en otra carpeta, no ve lo que guardó en la otra.
2. **Un choque de hash bloquea el proyecto.** El hash tiene 16 bits; dos raíces con el mismo id hacen que `register_project` falle (`crates/clawg-core/src/memory/project.rs:97`) y el puente rechaza toda herramienta en esa raíz (29) hasta renombrar una carpeta.
3. **Unir dos ids no reconcilia.** El re-key es un `UPDATE` por tabla (`project.rs:125`): deja dos versiones vigentes de una clave y dos filas vivas con el mismo texto en el proyecto que resulta. El motor de mover (77, `crates/clawg-core/src/memory/relocate.rs`) las rechaza en vez de unirlas.
4. **El id lo calcula cada cliente.** El puente (`project_for`, `crates/clawg-cli/src/mcp/server.rs:854-886`), los hooks (`crates/clawg-cli/src/hook.rs:227`), `export --project`, `projects move --here` y el import (`crates/clawg-core/src/memory/portability/import.rs:160`) derivan el id de la ruta y descartan el que responde el daemon.

## Decisiones

Tomadas con el usuario ([«Decisiones tomadas»](exploration.md#decisiones-tomadas)):

1. **El proyecto sigue siendo la raíz de siempre; el remoto es una pista.** La clave de una raíz es el `origin` normalizado (`host/dueño/repo`, en minúsculas, sin esquema, usuario, contraseña, puerto ni `.git`) del `config` de su directorio git. Sin git, sin `origin`, o con un `origin` que no es un servidor (una ruta, `file://`), no hay clave y nada cambia. Un monorepo sigue siendo un proyecto.
2. **El registro asigna el id (W3).** Cada raíz tiene su **id de raíz** en `project`: el que ya tiene, o `derive_project_id` si está libre, o el primer candidato libre del mismo formato. Una raíz con clave usa el **id de proyecto** que `project_remote` (V24) le da a esa clave: el de la primera raíz que la registró. Sin clave, el id de proyecto es el de raíz. Nadie más deriva el id: el puente, los hooks y los comandos usan el que responde el daemon.
3. **Unir reconcilia y se registra (Q1).** Cuando el id de raíz no es el de proyecto, sus filas de `memory` (y las de los ids viejos que mande el cliente) pasan al de proyecto con el motor de mover, sin rechazar choques: gana la versión más nueva de cada clave y la fila más vieja de cada texto, y la otra queda `superseded` por ella con causa `revision`. Cada fila movida queda en `memory_move` y cada reemplazo en `memory_move_supersede` (V24); `clawg projects move --undo N` deshace las dos cosas. Antes, un snapshot `pre-rekey`.
4. **Los docs siguen la raíz (I1).** El índice, `index_docs` y su refresco usan el id de raíz; `POST /v1/projects` responde los dos ids, y `search_project` usa el de raíz.
5. **Un cliente anterior no se entera.** Sólo un pedido con `assigned: true` recibe ids asignados y une por remoto. Uno sin el campo (un puente abierto antes de un `update`) sigue con la regla de hoy: el id es `derive_project_id(raíz)`, y si ese id no es el de esa raíz responde el 409 de siempre, para que nunca escriba bajo el id de otro proyecto.

## Alcance

1. `origin_key` en `identity.rs`: lectura del `config` y una normalización pura con tabla de casos.
2. V24: `project_remote` y `memory_move_supersede`, con el test de la base de la versión anterior.
3. `register_project` con ids asignados, la unión por remoto y el modo anterior; `Reason::PreRekey`.
4. El modo «unir» del motor de mover y su reconciliación, y el deshacer que la revierte.
5. `POST /v1/projects` con `assigned` y `root_id`; el cliente y sus usuarios: el puente (memoria y docs), los hooks, `projects add|move`, `export --project` (lectura sin daemon) y el import (asigna dentro de su transacción).
6. `doctor`: el chequeo `projects` acepta un id asignado a su raíz.
7. Las specs (`project`, `project-docs`, `hooks`, `mcp`, `portability`, `storage`, `doctor`) y `.claude/CLAUDE.md`.

## Fuera de alcance

- Subproyectos de un monorepo, una clave explícita por clone y `clawg projects split`.
- Mirar otro remoto que `origin`, ramas o historial; la red.
- Llevar la clave en un export.
- Reconciliar duplicados que ya estaban en el proyecto antes de unir: sólo los grupos que tocan una fila movida.
- `episodic_trajectory` y `procedural_skill` siguen con `project_rekey`, sin reconciliar: no tienen clave ni dedup.
- El ranking: `rag-eval` no aplica.

## Cómo se mide

Tests que fallan hoy (ver [spec](spec.md)): dos clones del mismo `origin` comparten un id y las filas del segundo se ven desde el primero; dos raíces cuyo hash choca reciben ids distintos y ninguna falla; una clave y un texto repetidos al unir quedan con una sola fila viva, y el deshacer devuelve cada fila a su id y su estado; una base de la versión anterior (V23) abre sin cambiar ninguna fila.

## Migración y vuelta atrás

- V24 sólo agrega dos tablas vacías: abrir la versión anterior no cambia ninguna fila, y la primera raíz de cada remoto conserva su id, así que el caso común (un clone por repo) no mueve nada.
- Lo que mueve un registro se deshace con `clawg projects move --undo N` (el número sale en el log y en `memory_move`) o restaurando el snapshot `pre-rekey`. `episodic_trajectory` y `procedural_skill` con el SQL de `V9__project.sql`.
- Volver a la versión anterior exige restaurar el snapshot `pre-migration`: una base con V24 es más nueva para ella y la rechaza intacta.
