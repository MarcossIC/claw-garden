# project-clone-identity — Especificación

Cambia y suma requisitos a [`specs/project`](../../specs/project/spec.md) (R2, R4, R6 y nuevos), [`specs/project-docs`](../../specs/project-docs/spec.md), [`specs/hooks`](../../specs/hooks/spec.md), [`specs/portability`](../../specs/portability/spec.md), [`specs/storage`](../../specs/storage/spec.md) y [`specs/doctor`](../../specs/doctor/spec.md). Lo que no se nombra aquí sigue como está.

## Definiciones

- **Raíz:** la de R1 de `specs/project`, sin cambios.
- **Clave de una raíz:** el `origin` normalizado de su directorio git (CI1), o ninguna.
- **Id de raíz:** el id con que `project` guarda una raíz. Lo usan el índice de docs y `index_docs`.
- **Id de proyecto:** el id bajo el que se guardan y buscan las memorias de una raíz: el que `project_remote` da a su clave, o su id de raíz si no tiene clave.
- **Candidatos de una raíz:** `derive_project_id(raíz)` (el candidato 0) y, para `n` de 1 a 63, `{basename}-{hash4}` con el hash de los bytes de la ruta seguidos de `#n` (`candidate_id`).
- **Unir:** mover las filas de `memory` de un id a otro con el motor de mover sin rechazar choques, y reconciliar (CI5).

## CI1. La clave de una raíz

`origin_key(raíz)` MUST leer el `config` del directorio git de la raíz: `<raíz>/.git/config` si `.git` es un directorio; si es un archivo `gitdir:`, el `config` del directorio común cuando ese `gitdir` tiene `commondir`, y si no el del propio `gitdir`. MUST tomar el `url` de la sección `[remote "origin"]` (nombre de sección y de clave sin distinguir mayúsculas, subsección exacta; comentarios `#` y `;`; valor entre comillas) y normalizarlo con una función pura:

- quita el esquema (`https`, `http`, `ssh`, `git`, `git+ssh`, `ssh+git`), el usuario y la contraseña, el puerto, la barra final y el `.git` final;
- pasa `usuario@host:ruta` a `host/ruta`;
- todo en minúsculas;
- MUST dar ninguna clave para `file://`, una ruta (absoluta, relativa, `~`, `C:\`), un valor sin host o sin ruta, y un archivo que no se puede leer.

La clave MUST NOT contener usuario, contraseña ni token.

#### Escenario: las formas de un mismo remoto
- GIVEN `https://github.com/Acme/App.git`, `git@github.com:acme/app.git`, `ssh://git@github.com:22/Acme/App` y `https://u:tok@github.com/Acme/App/`
- THEN las cuatro dan `github.com/acme/app`, y ninguna salida contiene `tok`

#### Escenario: lo que no es un servidor
- THEN `/srv/git/app.git`, `../app`, `file:///srv/app`, `C:\repos\app` y `` dan ninguna clave

#### Escenario: el directorio git de un worktree y de un submódulo
- GIVEN un worktree cuyo `gitdir` tiene `commondir`, y un submódulo cuyo `gitdir` no lo tiene
- THEN la clave del worktree sale del `config` del directorio común y la del submódulo del `config` de su `gitdir`

**Tests:** `a_remote_normalizes_to_host_owner_and_repo`, `a_local_remote_has_no_key`, `the_origin_url_is_read_from_its_section` (puros, `identity.rs`); `the_key_of_a_worktree_and_of_a_submodule` y `a_root_without_origin_has_no_key` (`crates/clawg-core/tests/project_root.rs`).

## CI2. El registro asigna el id de raíz

`register_project` con ids asignados MUST dar a una raíz que ya está en `project` el id con que está, y a una que no, el primer candidato que no es el id de otra raíz, y guardarla con él. Ninguna raíz MUST quedar con el id de otra, y ningún registro MUST fallar porque el hash de dos raíces choque. Si los 64 candidatos están tomados, el registro MUST fallar con un error que lo dice, sin escribir.

#### Escenario: la raíz de siempre conserva su id
- GIVEN una raíz con filas bajo `derive_project_id(raíz)` y sin fila en `project`
- WHEN se registra
- THEN su id de raíz y de proyecto es `derive_project_id(raíz)` y no se mueve ninguna fila

#### Escenario: dos raíces que chocan
- GIVEN `project` con el id `derive_project_id(/work/repo)` para `/elsewhere/repo`
- WHEN se registra `/work/repo`
- THEN recibe otro id del mismo formato, `project` tiene las dos raíces, y la marca `index_docs` de `/elsewhere/repo` no cambia

**Tests:** `the_root_of_always_keeps_its_id`, `a_root_whose_id_is_taken_gets_the_next_free_one`, `a_root_keeps_the_id_it_was_given` (`project.rs`); `registering_two_roots_that_collide_gives_each_its_own_id` (`crates/clawg-cli/tests/api.rs`).

## CI3. Dos clones de un remoto comparten el id de proyecto

Una raíz con clave MUST usar como id de proyecto el que `project_remote` guarda para la clave; si no hay, MUST guardarle su propio id de raíz. Cuando el id de proyecto no es el id de raíz, el registro MUST unir (CI5) las filas de `memory` del id de raíz y de cada id viejo al id de proyecto, y pasar las de `episodic_trajectory` y `procedural_skill` con `project_rekey` como hasta ahora. Un id viejo que es el id de raíz de otra raíz MUST quedar como está.

#### Escenario: el segundo clone
- GIVEN dos raíces con el mismo `origin`, cada una con una fila bajo su `derive_project_id`, y la primera ya registrada
- WHEN se registra la segunda
- THEN las dos filas están bajo el id de la primera, la segunda conserva su id de raíz en `project`, y `memory_move` lista la fila movida

#### Escenario: el remoto cambia
- GIVEN una raíz registrada con el remoto R1
- WHEN su `origin` pasa a R2, que nadie registró, y se registra de nuevo
- THEN su id de proyecto no cambia y no se mueve nada

#### Escenario: otro remoto, otro proyecto
- GIVEN dos raíces con `origin` distintos y el mismo nombre de carpeta
- THEN tienen ids de proyecto distintos

**Tests:** `a_second_clone_joins_the_project_of_the_first`, `a_new_remote_keeps_the_project_of_its_root`, `two_remotes_are_two_projects` (`project.rs`); `two_clones_of_one_remote_share_their_memories` (`crates/clawg-cli/src/mcp/server.rs`).

## CI4. Un cliente anterior conserva la regla de antes

`POST /v1/projects` MUST tomar `assigned` (falso si falta) y responder `{ "project_id", "root_id", "rekeyed" }`. Sin `assigned`, MUST comportarse como antes: el id de raíz y de proyecto es `derive_project_id(raíz)`, sin unir por remoto, y si ese id es de otra raíz, o la raíz está guardada con otro id, MUST responder 409 `project_root_collision` sin escribir. Con `assigned`, MUST NOT responder 409.

#### Escenario
- GIVEN una raíz guardada con un id asignado distinto de `derive_project_id(raíz)`
- WHEN la registra un pedido sin `assigned`
- THEN la respuesta es 409 y nada cambia

**Tests:** `a_caller_that_derives_its_id_keeps_the_conflict` (`project.rs`); `registering_without_assigned_ids_keeps_the_old_answer` (`tests/api.rs`).

## CI5. Unir reconcilia

Unir MUST mover, como un movimiento de R8 de `specs/project` (registrado en `memory_move`, sin vectores, sin índice de texto, sin detector), cada unidad de las filas del origen, borradas incluidas, sin rechazar por `live_twin` ni `topic_taken`. Una unidad con filas en un proyecto que no es el origen ni el destino MUST quedar donde está, con un warn. Después, en la misma transacción y sobre las filas vivas del destino:

1. Por cada `topic_key` con filas vivas de más de un save, alguna movida: MUST quedar vivo el save de la fila más nueva (`created_at`, luego id), y cada otra fila viva con esa clave, fuera de ese save, MUST pasar a `superseded` por la primera fila de ese save, con causa `revision`.
2. Después, por cada `content_hash` con más de una fila viva, alguna movida: MUST quedar viva la más vieja, y cada otra, fuera de su save, MUST pasar a `superseded` por ella con causa `revision`, sólo si es un save de una fila o si todas las filas vivas de su save son reemplazadas así.

Ninguna otra columna MUST cambiar. Cada reemplazo MUST quedar en `memory_move_supersede` con el número del movimiento, la fila, la que la reemplaza y el `status`, `superseded_by` y `supersede_cause` que tenía. El resultado MUST NOT depender del orden en que se registran los clones.

#### Escenario
- GIVEN dos ids con una fila viva cada uno bajo la misma `topic_key`, y dos filas vivas con el mismo texto
- WHEN se unen
- THEN hay una sola fila viva por clave (la más nueva) y por texto (la más vieja), las otras quedan `superseded` por ellas con causa `revision`, y al borrar la vigente de la clave la otra vuelve viva

**Tests:** `merging_keeps_the_newest_version_of_a_key`, `merging_keeps_the_oldest_row_of_a_text`, `merging_does_not_depend_on_which_clone_came_first`, `merging_never_cuts_a_save` (`relocate.rs`).

## CI6. Deshacer una unión

`undo_move_core(N)` MUST, antes de mover de vuelta, devolver cada fila de `memory_move_supersede` de N que sigue reemplazada por la fila que anotó a su `status`, `superseded_by` y `supersede_cause` anteriores. Después de deshacer, cada fila MUST tener el id y el estado que tenía antes de unir.

**Test:** `undoing_a_merge_restores_every_row_and_state` (`relocate.rs`).

## CI7. Snapshot antes de unir

Un registro que va a mover filas MUST tomar antes un snapshot `pre-rekey` (`Reason::PreRekey`, automático), y MUST decidir otra vez después del snapshot, en la transacción que escribe. Uno que no mueve nada MUST NOT tomarlo.

**Test:** `a_registration_that_moves_rows_snapshots_first` (`project.rs`).

## CI8. Los clientes usan el id del daemon

El puente MUST registrar cada raíz con `assigned` y usar, para memorias, el id de proyecto que responde, y para `search_project`, el id de raíz; si el registro falla por otra causa que un 409 o un rechazo de conexión, MUST seguir con `derive_project_id(raíz)` y un warn, como antes. Los hooks MUST registrar la raíz del `cwd` (sin `index_docs`) y leer con el id que responde; si el registro falla, MUST leer con `derive_project_id(raíz)` y, ante un 409, no imprimir el contexto. `clawg projects add`, `projects move --to PATH` y `--here` MUST registrar y usar ese id. `clawg export --project PATH` MUST leer el id del store sin el daemon: el de `project_remote` para la clave de la raíz, si no el de la raíz en `project`, si no `derive_project_id`. El import MUST asignar los ids de sus raíces dentro de su transacción con la misma regla, y una raíz con clave MUST dar el id de proyecto de esa clave.

**Tests:** `a_bridge_uses_the_ids_the_daemon_gives` y `search_project_reads_the_docs_of_its_own_root` (`server.rs`); `the_hook_reads_the_project_the_daemon_gives` (`hook.rs`); `export_reads_the_project_id_from_the_store` (`crates/clawg-core/src/memory/project.rs`); `an_import_gives_a_colliding_root_its_own_id` (`import.rs`).

## CI9. `doctor`

El chequeo `projects` MUST aceptar un id que es un candidato de su raíz y MUST seguir marcando `id_mismatch` uno que no lo es.

**Test:** `a_project_id_assigned_to_its_root_is_ok` (`crates/clawg-cli/src/doctor/checks.rs`).

## CI10. Datos de la versión anterior

Una base en V23 con filas en dos proyectos MUST abrir con V24 sin cambiar ninguna fila de `memory` ni de `project`, con `project_remote` y `memory_move_supersede` vacías.

**Test:** `a_database_from_the_previous_version_opens_with_no_remote_recorded` (`crates/clawg-core/src/memory/db.rs`).
