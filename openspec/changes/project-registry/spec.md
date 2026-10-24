# project-registry — Especificación

Esta especificación crece con cada entrega de la [propuesta](proposal.md#entregas). Por ahora cubre la **entrega 0, el fundido**: cambia R15, R16 y R18 de [`specs/project`](../../specs/project/spec.md) y suma una fila a R7 de [`specs/doctor`](../../specs/doctor/spec.md). Lo que no se nombra aquí sigue como está. Las entregas 1 a 4 suman sus requisitos cuando les toque.

## Definiciones

- **Raíz**, **clave de una raíz**, **id de raíz**, **id de proyecto** y **candidatos de una raíz:** los de `specs/project`, sin cambios.
- **Filas de un camino:** las filas de `project` cuyo `root_path` es el de la raíz, de la más vieja a la más nueva (`created_at`, después `id`). Antes de este cambio había a lo sumo una; ahora puede haber varias, una por cada repositorio que ocupó el camino.
- **Remotos atados a una fila:** las claves de `project_remote` cuyo `project_id` es el id de esa fila.
- **Contradicción:** la clave actual de la raíz y un remoto atado a una fila son los dos conocidos y distintos.

## RG1. Una fila de un camino sirve a una raíz sólo si su remoto no la contradice

Con `assigned`, el registro MUST elegir entre las filas del camino con una regla pura (`settle`, `crates/clawg-core/src/memory/project.rs`):

1. sin filas, el camino es nuevo y la raíz MUST recibir el primero de sus candidatos libre (R15);
2. sin clave actual, MUST quedarse con la más vieja: una clave que no se puede leer no contradice nada;
3. con clave, MUST quedarse con la primera fila que tiene esa clave atada; si ninguna la tiene, con la más vieja que no tiene ningún remoto atado (un primer remoto, que entonces se ata a ella);
4. si todas las filas tienen remotos atados y ninguno es la clave, el camino está **contradicho**: la raíz MUST recibir el primero de sus candidatos que ninguna fila tiene, en una fila nueva del mismo camino, y su clave MUST atarse a esa fila cuando `project_remote` no la tenga.

El id de proyecto se sigue tomando como en R16: el que `project_remote` da a la clave, o el id de raíz elegido.

#### Escenario: otro repositorio en el camino de uno registrado
- GIVEN `/work/app` registrado con la clave `github.com/acme/a`, con una fila en su proyecto
- WHEN se registra `/work/app` con la clave `github.com/acme/b`, que nunca se registró
- THEN recibe `candidate_id(/work/app, 1)` como id de raíz y de proyecto, no se mueve ninguna fila, no se toma snapshot, y `project_remote` ata `github.com/acme/b` a ese id
- AND un clone de `github.com/acme/b` en `/review/b` se une a ese proyecto, no al de `github.com/acme/a`

#### Escenario: cada repositorio vuelve a su proyecto
- GIVEN los dos registros del escenario anterior
- WHEN se registra otra vez `/work/app` con `github.com/acme/a` y después con `github.com/acme/b`
- THEN cada uno recibe los ids de su primer registro, y `project` sigue con dos filas

#### Escenario: un primer remoto
- GIVEN `/work/app` registrado sin clave
- WHEN se registra con `github.com/acme/app`, y otra vez sin clave
- THEN las dos veces conserva su proyecto, y `project` tiene una fila

#### Escenario: un remoto distinto en un camino registrado
- GIVEN `/work/app` registrado con `github.com/acme/app`
- WHEN se registra con `gitlab.com/acme/app`
- THEN recibe otro proyecto. Sin el primer commit no se distingue un remoto renombrado de otro repositorio en el mismo camino, y ante la duda se separa: unir se deshace con `projects move`, separar lo fundido no

**Tests:** `a_path_settles_on_the_row_its_remote_does_not_contradict` (pura), `a_repository_in_the_path_of_another_gets_a_project_of_its_own`, `each_repository_in_one_path_returns_to_its_own_project`, `a_first_remote_keeps_the_project_of_its_root`, `a_different_remote_in_a_registered_path_is_another_project` (`project.rs`); `a_repository_cloned_where_another_was_registers_into_a_project_of_its_own` (`crates/clawg-cli/tests/api.rs`).

## RG2. Los ids viejos de un camino son de su primer repositorio

Los ids viejos que manda el cliente salen del camino crudo (R1), así que son del repositorio que ocupaba el camino cuando se guardaron. El registro MUST unir sus filas (R4) sólo cuando la raíz se queda con la fila más vieja de su camino o el camino es nuevo; con una fila nueva por contradicción, o con otra que no es la más vieja, MUST NOT moverlas. Un id viejo que es el id de cualquier otra fila de `project`, también una del mismo camino, MUST quedar como está.

#### Escenario: el remoto del recién llegado ya es de otro proyecto
- GIVEN `/work/app` registrado con `github.com/acme/a`, con una fila en su proyecto y otra bajo el id de `/work/app/sub`; y `github.com/acme/w` registrado desde `/else/w`
- WHEN se registra `/work/app` con `github.com/acme/w` y el id viejo de `/work/app/sub`
- THEN recibe una fila propia, su proyecto es el de `github.com/acme/w`, y ninguna de las dos filas se mueve ni se toma snapshot

**Test:** `a_repository_in_the_path_of_another_takes_none_of_its_rows` (`project.rs`).

## RG3. Leer y planear dan lo mismo que registrar

`project_id_in` (`clawg memories --here`, `clawg export --project`) y `plan_ids_in` (el import) MUST elegir la fila con la misma regla, sin escribir. En un lote, una raíz MUST elegir como si las anteriores ya estuvieran escritas: el mismo camino y la misma clave dos veces son un solo proyecto.

**Test:** `the_project_of_a_repository_in_another_s_path_is_read_alike_without_writing` (`project.rs`).

## RG4. Un cliente anterior recibe el 409 en vez del proyecto ajeno

El daemon MUST leer la clave de la raíz también para un pedido sin `assigned`, sólo para elegir la fila: MUST NOT atarla. Ese pedido MUST recibir 409 `project_root_collision` (R18) cuando la fila que la regla de RG1 elige no tiene `derive_project_id(raíz)` o el camino está contradicho, sin escribir nada.

#### Escenario: un puente anterior en el camino de otro repositorio
- GIVEN `/work/app` registrado con `github.com/acme/a`
- WHEN un pedido sin `assigned` registra `/work/app`, que ahora tiene `github.com/acme/b`
- THEN responde 409 `project_root_collision`, ninguna fila se mueve y `project` tiene una fila
- AND con `github.com/acme/a` de nuevo, responde el proyecto de siempre

**Tests:** `a_caller_that_derives_its_id_is_refused_another_repository_s_project` (`project.rs`); `a_repository_cloned_where_another_was_registers_into_a_project_of_its_own` (`tests/api.rs`).

## RG5. `doctor` nombra los proyectos con más de un remoto

`clawg doctor` MUST leer, sin escribir, `project_remote` del store (`inspect`) y la clave actual de cada `root_path` de `project` (`origin_key`, que lee el `config` de git y nada más), y MUST dar un hallazgo por cada proyecto con dos o más remotos atados:

| `check_id` | Hallazgo | Severidad | Siguiente paso |
|---|---|---|---|
| `project_remotes` | un proyecto con dos o más remotos atados: lo que deja un repositorio clonado en el camino de otro antes de este cambio, y también un remoto renombrado | `warn` `several_remotes`, con el id, sus remotos y cada raíz que lo usa (las filas con ese id y las raíces cuya clave actual es uno de sus remotos) con su clave actual | — |
| `project_remotes` | ninguno | `ok`, con cuántos remotos hay | — |

No repara nada: separar lo fundido pide elegir fila por fila, y para eso está `clawg projects move`.

**Tests:** `the_facts_name_the_remotes_of_each_project` (`inspect.rs`); `project_remote_findings` (`doctor`).

## Lo que no cambia

- El esquema: la entrega 0 no tiene migración. Lo único nuevo en los datos es una segunda fila de `project` con el mismo `root_path`, que `idx_project_root` (V24) no impide porque no es único.
- `merge_in`, `projects move`, el índice de docs (sigue por id de raíz) y la regla de `doctor` `projects` (la fila nueva tiene un candidato de su raíz).
