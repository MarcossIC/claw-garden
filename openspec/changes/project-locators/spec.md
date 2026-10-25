# project-locators — Especificación

Esta especificación cubre dos entregas de la [propuesta](proposal.md):

- la **entrega 1, el registro (V27)** (PL1 a PL10). Cambia R10, R16, R18 y R20 de [`specs/project`](../../specs/project/spec.md) y suma V27 a [`specs/storage`](../../specs/storage/spec.md);
- la **entrega 2a, unir proyectos (V28)** (PL11 a PL20, [más abajo](#entrega-2a-unir-proyectos)). Cambia PL9 y suma V28 a `specs/storage` y tres hallazgos a [`specs/doctor`](../../specs/doctor/spec.md).

Lo que no se nombra aquí sigue como está. Las entregas 2b a 4 agregan sus requisitos a este archivo cuando les toque. El [diseño](design.md) dice cómo se cumple cada requisito y qué alternativas se descartaron.

## Definiciones

- **Proyecto:** una fila de `project`, la tabla de referencia. Tiene un id que no cambia, un nombre, cómo nació y si el usuario ya lo revisó. Las memorias, y cualquier tabla nueva, guardan este id. Un proyecto puede tener cero o más carpetas y cero o más remotos: puede ser una idea sin carpeta, o seguir existiendo después de que se borró la suya.
- **Carpeta:** una fila de `project_path`. Es la tabla `project` de antes de V27, renombrada. Tiene:
  - un camino de esta máquina;
  - su id, que es el `root_id` de la API y la clave del índice de docs;
  - `index_docs`;
  - el proyecto al que lleva;
  - las huellas de lo que se encontró ahí.

  Una carpeta es una forma de llegar a un proyecto, no el proyecto. Se puede mover, desaparecer o no existir.
- **Raíz**, **clave de una raíz** y **candidatos de una raíz:** los de `specs/project` (R1, R14, R15), sin cambios. Lo que R15 llama «id de raíz» es el id de la carpeta.
- **Carpetas de un camino:** las filas de `project_path` con ese `root_path`, de la más vieja a la más nueva (R20).
- **Checkout:** lo que hay hoy en una raíz. Tiene tres datos: su clave (R14), su **directorio git** (el que tiene el `config` de R14: `git_config_dir`, `crates/clawg-core/src/memory/identity.rs:183-189`) y su **primer commit**. Una raíz sin `.git` no tiene directorio git ni primer commit.
- **Primer commit:** el commit sin padres en el que termina la cadena de primeros padres de `HEAD`.
- **Marca del directorio git (`git_stamp`):** la fecha de creación del directorio git y, en Unix, también su dispositivo y su inodo. Cambia cuando el directorio git es otro: un clone nuevo, un `git init` nuevo o una copia. No cambia con un `mv` dentro del mismo sistema de archivos.
- **Huellas de una carpeta:** su remoto y su primer commit. El **remoto de una carpeta** es `project_path.remote`. Una carpeta de antes de V27 que no lo tiene usa los remotos que `project_remote` ata a su id, como en R20.
- **Proyecto de una carpeta:** el de `project_path.project_id`. Si es NULL, la carpeta no se registró desde V27, y su proyecto es el que da R16: el de su clave en `project_remote`, o si no su id.
- **Checkout grabado de una carpeta:** su camino con la marca que guardó. **Ya no está** cuando el camino no existe o, si guardó una marca, la de lo que hay hoy en él es otra.

## PL1. V27 hace de `project` la referencia y pasa las carpetas a `project_path`

V27 MUST:

- renombrar `project` a `project_path` sin cambiar ningún valor ni índice;
- agregar a `project_path` las columnas `project_id`, `remote`, `root_commit` y `git_stamp`, todas NULL por defecto, y un índice parcial por `root_commit`;
- crear `project` (`id` clave primaria, `name`, `born`, `reviewed`, `created_at`);
- crear `project_suggestion` (`project_id`, `other_id`, `reason`, `created_at`, `answer`, `answered_at`), con un índice único sobre el par sin orden;
- crear los disparadores de PL2.

El relleno MUST crear un proyecto por cada id distinto que tenga una fila de `memory`, `episodic_trajectory` o `procedural_skill` (en cualquier estado, borradas incluidas) o una fila de `project_remote`. Cada proyecto lleva:

- `name`: el id sin su `-` y sus cuatro dígitos hex finales cuando tiene la forma de `is_project_id`; si no, el id entero;
- `born = 'migrated'` y `reviewed = 1`: lo que ya estaba no se marca «sin revisar»;
- `created_at`: el menor `created_at` entre la carpeta con ese id, sus filas en `project_remote` y sus filas en las tres tablas.

El relleno de cada carpeta MUST:

- poner en `project_path.project_id` su propio id cuando ese id es un proyecto, y dejar NULL si no. Queda NULL la carpeta de un clone, cuyas memorias están bajo el proyecto de su remoto, y la carpeta donde nunca se guardó nada. Las dos se resuelven en su primer registro;
- poner en `project_path.remote` el remoto que `project_remote` ata a su id, sólo cuando le ata exactamente uno;
- dejar NULL `root_commit` y `git_stamp`.

Un id que sólo aparece en un registro de historia (`memory_move`, `project_rekey`, `memory_import_extra`, `synthesis_review`) MUST NOT recibir un proyecto: no tiene nada que mostrar, y si algún día vuelve a tener filas lo recibe por PL2. V27 MUST NOT cambiar ninguna fila de `memory`, de las otras dos tablas ni de `project_remote`.

#### Escenario: un store en V26 abre con un proyecto por cada id en uso
- GIVEN un store en V26 con:
  - dos raíces que comparten un remoto, cada una con filas;
  - dos raíces cuyo hash choca (`candidate_id` 0 y 1);
  - un camino con dos filas (R20);
  - un proyecto con dos remotos atados (fundido);
  - filas en las tres tablas y un `project_id` de `memory` sin carpeta;
  - entradas en `memory_move`, `project_rekey` y `synthesis_review` para un id que ya no tiene filas
- WHEN se abre con V27
- THEN `project_path` tiene las filas de la `project` de antes con los mismos valores, y las filas de las tres tablas y de `project_remote` no cambian
- AND cada id con filas o con remoto tiene un proyecto `migrated`, revisado y con el nombre de su id, y el id que sólo nombran los registros no tiene ninguno
- AND la carpeta del segundo clone queda con `project_id` NULL, y la de cada raíz con filas queda con su propio id
- AND `project_path.remote` está lleno sólo en las carpetas con un único remoto atado: la fundida queda en NULL
- AND `list_memories` devuelve lo mismo que antes para cada proyecto y para «sin proyecto»
- AND los hechos que `inspect` le da a `doctor` (`projects`, `remotes`) son los de antes
- AND hay un snapshot `pre-migration`

**Tests:** `a_database_from_the_previous_version_opens_with_a_project_for_every_id_in_use` (`crates/clawg-core/src/memory/db.rs`); `published_migration_checksums_are_pinned` (`migrations.rs`). Los tests de migraciones anteriores que, después de abrir, leen las carpetas pasan a leerlas de `project_path`, sin cambiar lo que comprueban.

## PL2. Cada id que se escribe tiene su proyecto

Cuando se inserta una fila de `memory` con un `project_id` que no está en `project`, o se le cambia el `project_id` a uno que no está, la misma escritura MUST crear el proyecto: con el nombre que da PL1, `born = 'adopted'`, `reviewed = 0` y el `created_at` de la fila. MUST NOT rechazar la fila ni cambiar ninguna de sus columnas. Pasa con cualquier escritura: un save, un import, un movimiento, su `--undo` o un `PATCH`.

El nombre que da Rust (`project_name`) MUST ser el mismo que da el disparador.

#### Escenario: un save bajo un id que no se registró
- GIVEN un puente cuyo registro falló, que guarda bajo `derive_project_id(raíz)` (R6)
- WHEN el worker guarda la fila
- THEN existe el proyecto de ese id, `adopted` y sin revisar, y la fila es la que habría sido sin él

#### Escenario: un movimiento de vuelta a un id de la historia
- GIVEN un movimiento que llevó todas las filas de un id viejo a otro proyecto, y ese id sin proyecto
- WHEN se deshace el movimiento
- THEN las filas vuelven al id viejo, y ese id es un proyecto

**Tests:** `a_row_saved_under_an_unknown_project_adopts_it`, `a_row_moved_to_an_unknown_project_adopts_it` y `the_name_of_an_adopted_id_is_the_one_rust_gives` (`crates/clawg-core/src/memory/registry.rs`).

## PL3. El primer commit se lee una vez por directorio git

Cada registro MUST leer de la raíz su clave (R14) y la marca de su directorio git. El primer commit sólo se lee cuando ninguna carpeta del camino guardó esa misma marca: un camino nuevo, una carpeta de antes de V27, o un directorio git que no es el que se grabó. Si una carpeta guardó esa marca, el primer commit es el que guardó ella, también cuando guardó que no se pudo leer. Así, un camino grabado que se registra con el mismo directorio git MUST NOT correr `git`.

La lectura (`root_commit`, `crates/clawg-core/src/memory/identity.rs`) MUST:

- correr `git rev-list --max-parents=0 --first-parent HEAD` en la raíz:
  - sin las variables `GIT_*` que cambian qué repositorio se lee;
  - con la entrada cerrada y, en Windows, sin abrir una ventana: el daemon corre sin consola;
- esperar como mucho `ROOT_COMMIT_WAIT` (1 s), y pasado eso cortar el proceso;
- aceptar como respuesta sólo una línea de 40 o 64 dígitos hex.

Lo que se graba en la carpeta elegida (PL4) depende del resultado:

| Resultado | Cuándo | Se graba |
|---|---|---|
| primer commit | `git` respondió a tiempo con un hash | el commit y la marca |
| no se puede saber | el repositorio es superficial (su directorio git tiene `shallow`), o `git` no respondió a tiempo | NULL y la marca: no se vuelve a intentar hasta que cambie el directorio git |
| más tarde | `git` no se pudo correr, o terminó con error (por ejemplo, un repositorio todavía sin commits) | nada: el próximo registro vuelve a intentar |

Leer y planear sin escribir (PL7) siguen la misma regla. Sólo se diferencian en que no graban.

#### Escenario: un checkout grabado
- GIVEN `/work/app` registrado, con su primer commit y su marca grabados
- WHEN se registra otra vez con el mismo directorio git
- THEN no se lee el primer commit

#### Escenario: el mismo camino, otro clone
- GIVEN `/work/app` registrado
- WHEN se borra y se clona otra vez en el mismo camino, y se registra
- THEN se lee el primer commit del clone nuevo

**Tests:** `a_recorded_checkout_does_not_read_its_first_commit_again` (`project.rs`, con un `Ground` falso que cuenta las lecturas); `the_first_commit_is_the_root_of_the_first_parent_chain`, `a_shallow_or_empty_repository_has_no_first_commit_to_record`, `the_stamp_of_a_checkout_changes_when_it_is_cloned_again` (`crates/clawg-core/tests/project_root.rs`, con repositorios reales: necesitan `git` en el PATH); `a_command_that_does_not_end_in_time_is_stopped` (`identity.rs`).

## PL4. Una carpeta sirve a una raíz sólo si sus huellas no la contradicen

Reemplaza la regla de R20. Cada carpeta del camino se compara con el checkout con una función pura (`fit`, `crates/clawg-core/src/memory/project.rs`):

1. si el remoto de la carpeta y la clave del checkout se conocen los dos, la carpeta es **la misma** cuando el checkout tiene uno de sus remotos, y **contradicha** cuando no;
2. si no, y los dos primeros commits se conocen, es la misma cuando son iguales y contradicha cuando no;
3. si no, la carpeta es **desconocida**.

El registro MUST elegir la primera carpeta que es la misma; si no hay, la más vieja desconocida; y si todas están contradichas, el camino está contradicho y la raíz recibe una carpeta nueva como en R20.3. Una carpeta nueva recibe el primero de sus candidatos que no es el id de otra carpeta (R15).

En la carpeta elegida o nueva, el registro MUST grabar:

- su remoto, cuando no tiene ninguno propio ni atado: es un primer remoto;
- el primer commit y la marca, según PL3;
- su proyecto en `project_path.project_id`.

El proyecto de la raíz MUST ser el primero que exista de estos:

1. el de su clave en `project_remote` (R16);
2. el de `project_path.project_id` de la carpeta;
3. un proyecto nuevo con el id de la carpeta, `born = 'root'` y `reviewed = 0`.

El remoto decide antes que el primer commit. Un checkout del mismo remoto con otro primer commit (una rama huérfana, una historia reescrita) es la misma carpeta.

#### Escenario: un repositorio sin `origin` en el camino de otro
- GIVEN `/work/app` registrado sin clave, con el primer commit `c1` grabado y una fila en su proyecto
- WHEN se registra `/work/app` con la clave `github.com/acme/b` y el primer commit `c2`
- THEN recibe la carpeta `candidate_id(/work/app, 1)` y un proyecto nuevo con ese id, nada se mueve, y `github.com/acme/b` se ata a ese proyecto
- AND con los dos sin clave (`c1` y después `c2`) pasa lo mismo, sin remoto atado

#### Escenario: el remoto decide antes que el primer commit
- GIVEN `/work/app` registrado con `github.com/acme/app` y `c1`
- WHEN se registra con `github.com/acme/app` y `c9`
- THEN conserva su proyecto

#### Escenario: lo de R20 no cambia
- THEN `a_repository_in_the_path_of_another_gets_a_project_of_its_own`, `each_repository_in_one_path_returns_to_its_own_project`, `a_first_remote_keeps_the_project_of_its_root` y `a_different_remote_in_a_registered_path_is_another_project` siguen pasando

**Tests:** `a_checkout_settles_on_the_folder_its_fingerprints_do_not_contradict` (pura), `a_repository_without_origin_in_the_path_of_another_gets_a_project_of_its_own`, `two_repositories_without_origin_in_one_path_are_two_projects`, `a_remote_decides_before_the_first_commit`, `a_clone_folder_records_its_own_remote` (`project.rs`); `a_repository_without_origin_replaced_in_its_path_registers_into_a_project_of_its_own` (`crates/clawg-cli/tests/api.rs`, con repositorios reales).

## PL5. Una mudanza conserva el proyecto

Con un camino nuevo, un registro con `assigned` cuya clave no está en `project_remote` (o que no tiene clave) y cuyo primer commit se conoce MUST buscar las carpetas de otros caminos con ese primer commit que el remoto no contradice (PL4.1). Para cada una, el daemon MUST mirar en disco si su checkout grabado ya no está.

- Si las carpetas encontradas son de **un solo proyecto** y el checkout grabado de todas ya no está, es una mudanza. La raíz MUST recibir una carpeta nueva, con su id (R15) y ese proyecto en `project_path.project_id`. Sus ids viejos y el id de la carpeta MUST unirse a él (R4), y el log MUST nombrar el proyecto y el camino viejo.
- Si no (el checkout grabado sigue en su lugar: una copia o un segundo checkout, o hay más de un proyecto), MUST abrir un proyecto propio, como hoy, y dejar una sugerencia `root_commit` con cada uno de esos proyectos (PL6).

La carpeta vieja queda como está: es una forma más de llegar al proyecto, y si el checkout vuelve a ese camino, la usa otra vez.

Ante la duda queda separado: unir se deshace, y lo fundido no se puede separar.

#### Escenario: una carpeta sin `origin` que se mueve
- GIVEN `/work/app`, sin clave, registrado con el primer commit `c1` y con filas en su proyecto
- WHEN se mueve a `/work/app2` y se registra `/work/app2`
- THEN su proyecto es el de `/work/app`, que ahora tiene dos carpetas, y lo que se guardó allí se encuentra desde `/work/app2`

#### Escenario: una copia
- GIVEN el mismo `/work/app`, que sigue en su lugar
- WHEN se copia a `/work/copia` y se registra `/work/copia`
- THEN recibe un proyecto propio y una sugerencia `root_commit` con el de `/work/app`
- AND registrarla otra vez no agrega otra sugerencia

**Tests:** `a_checkout_moved_without_origin_keeps_its_project`, `a_copy_of_a_checkout_is_a_project_of_its_own_with_a_suggestion`, `a_checkout_two_projects_could_be_is_a_project_of_its_own` (`project.rs`); `a_folder_moved_without_origin_registers_into_its_project` (`tests/api.rs`, con repositorios reales).

## PL6. Un remoto cambiado en el mismo camino deja una sugerencia

Cuando el camino queda contradicho por el remoto (PL4), una carpeta contradicha tiene el mismo primer commit que el checkout, y el proyecto que recibe la raíz es otro que el de esa carpeta, el registro MUST dejar una sugerencia `remote_changed` entre los dos proyectos. Sin el mismo primer commit no hay sugerencia, como en R20.

Para las sugerencias de PL5 y PL6:

- Una sugerencia MUST ser una por par de proyectos, sin importar el orden. Una que ya existe, con respuesta o sin ella, no se vuelve a crear.
- Sólo un registro con `assigned` las crea. Leer, planear, importar y un cliente sin `assigned` MUST NOT crearlas.
- Crear una MUST NOT mover ninguna fila ni cambiar ninguno de los dos proyectos. El daemon MUST dejarla en el log con el comando que la lista.
- Esta entrega no las responde. Quedan pendientes (`answer` NULL), y `projects list` las muestra (PL9).

#### Escenario: un remoto renombrado
- GIVEN `/work/app` registrado con `github.com/acme/app` y el primer commit `c1`
- WHEN se registra con `github.com/acme-org/app` y `c1`
- THEN recibe otro proyecto, como en R20, y queda una sugerencia `remote_changed` con el anterior

**Tests:** `a_renamed_remote_in_its_path_is_another_project_with_a_suggestion`, `a_suggestion_is_made_once_per_pair` (`project.rs`).

## PL7. Leer, planear, importar y el cliente anterior eligen igual

- `project_id_in` (`clawg memories --here`, `clawg export --project`) y `plan_ids_in` (el import) MUST elegir con PL3 a PL5 sin escribir. En un lote, una raíz MUST elegir como si las anteriores ya estuvieran escritas, con sus huellas (amplía RG3 de R20).
- Las carpetas que escribe un import MUST llevar lo que grabaría un registro (`project_id`, `remote`, `root_commit` y `git_stamp`), y su proyecto MUST existir en `project` (`born = 'import'` cuando lo crea el import).
- Un pedido sin `assigned` MUST elegir la carpeta con PL4 y MUST recibir 409 `project_root_collision` cuando la carpeta elegida no tiene `derive_project_id(raíz)` o el camino está contradicho, también por el primer commit. MUST NOT vincularse por una mudanza: un camino nuevo recibe el id derivado, como en R18. Graba las huellas sólo en su propia carpeta.

**Tests:** `a_moved_checkout_is_read_and_planned_alike_without_writing`, `a_caller_that_derives_its_id_is_refused_a_repository_without_origin_in_its_path`, `a_caller_that_derives_its_id_never_follows_a_move` (`project.rs`); `an_import_records_its_folders_as_a_registration_would` (`crates/clawg-core/src/memory/portability/import.rs`).

## PL8. Un proyecto existe tenga carpeta o no

Cambia R10. El destino de un movimiento (`POST /v1/memories/move`, `PATCH` con `project_id`, `clawg projects move --to-project`) MUST aceptarse cuando está en `project`, tenga carpeta o no, y MUST seguir siendo un 404 sin escribir cuando no está. `--to PATH` y `to_project_path` siguen registrando antes la carpeta.

#### Escenario: un proyecto sin carpeta como destino
- GIVEN un proyecto que sólo tiene filas, sin carpeta (por ejemplo, adoptado por PL2)
- WHEN se mueve una fila con `--to-project` a ese id
- THEN se mueve, y un id que no está en `project` sigue dando 404

**Tests:** `a_project_without_a_folder_is_a_target` (`crates/clawg-core/src/memory/relocate.rs`); `an_unregistered_target_is_refused` y `a_move_to_an_unregistered_project_is_404` siguen pasando.

## PL9. `clawg projects list` y `clawg projects rename`

**`GET /v1/projects`** está detrás del token y MUST listar cada proyecto, tenga carpeta o no, con:

- `id`, `name`, `born`, `reviewed` y `created_at`;
- sus carpetas, de cero a muchas: `root_id`, `root_path`, `remote`, `index_docs` y `present`, que dice si el checkout grabado sigue ahí;
- sus remotos, de `project_remote`;
- cuántas filas de `memory` no borradas tiene;
- sus sugerencias pendientes, cada una con el otro proyecto y su nombre, el motivo y la fecha.

Una carpeta con `project_id` NULL MUST listarse en el proyecto que le da R16, leyendo su clave en disco. Una carpeta cuyo proyecto no está en `project` (una de antes de V27 donde nunca se guardó nada) no se lista hasta que un registro la hace proyecto.

**`PATCH /v1/projects/{id}`** con `{"name": "..."}` MUST cambiar el nombre y marcar el proyecto revisado. MUST NOT cambiar el id ni ninguna fila guardada. Responde así:

- 404 si el proyecto no existe;
- 400 si el nombre, sin espacios a los lados, queda vacío, pasa de 80 caracteres o tiene caracteres de control;
- 409 `name_taken` si el nombre es, sin distinguir mayúsculas, el nombre o el id de otro proyecto.

Volver a poner el mismo nombre sólo lo marca revisado.

**`clawg projects list [--profile P]`** y **`clawg projects rename <ID> <NOMBRE> [--profile P]`** MUST hablar con el daemon que corre, sin arrancar uno, e imprimir en el idioma de la preferencia, con una sentencia por `match` sobre `Lang`. La lista MUST marcar lo que nació solo y no se revisó, los proyectos sin carpeta, las carpetas que ya no están y las sugerencias pendientes.

#### Escenario: renombrar
- GIVEN un proyecto `app-1a2b` con memorias, sin revisar
- WHEN se corre `clawg projects rename app-1a2b "App web"`
- THEN `projects list` lo muestra como `App web`, revisado y con el mismo id
- AND `search_memory` desde su raíz encuentra lo mismo que antes

**Tests:** `renaming_keeps_the_id_and_marks_the_project_reviewed`, `a_name_another_project_holds_is_refused`, `a_name_is_trimmed_and_bounded` (`registry.rs`); `the_projects_are_listed_with_their_folders_and_suggestions`, `renaming_a_project_keeps_what_was_saved_under_it`, `renaming_an_unknown_project_is_404` (`tests/api.rs`); `projects_list_and_rename_parse` (`cli.rs`); `a_listing_marks_what_was_created_alone_and_the_suggestions` (`crates/clawg-cli/src/projects.rs`, en los dos idiomas).

## PL10. Los clientes no notan nada

- `POST /v1/projects` MUST responder con la misma forma de antes (`project_id`, `root_id`, `rekeyed`).
- El puente, los hooks, `projects add` y `projects move --to` no cambian.
- El contrato del modelo (`agent_contract.json`, `agent_contract.es.json`) no cambia.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (sin volver a bendecir), y los tests de R4, R6 y R14 a R20 de `specs/project`.

## Lo que no cambia en la entrega 1

- Ninguna fila de `memory` cambia de `project_id` por la migración ni por un registro, salvo las uniones de R4 y R17, como hoy.
- El índice de docs sigue por id de carpeta. Una mudanza le da a la raíz nueva su propia carpeta y su propio índice. Un proyecto sin carpeta no tiene docs.
- `doctor` no tiene hallazgos nuevos en esta entrega: los duplicados y los ids sin proyecto son de la entrega 2. Su regla `projects` compara cada carpeta con sus candidatos, como antes, leyéndola de `project_path`.
- No se responde ni se muestra ninguna sugerencia al agente: eso es de las entregas 2 y 3.
- Crear un proyecto sin carpeta, declarar un subproyecto de un monorepo y reconocer una carpeta sin VCS que se movió son de la entrega 3. Esta entrega no los impide (diseño, D14).

## Entrega 2a: unir proyectos

Cubre la parte de unir de la [entrega 2](proposal.md#2-unir-revisar-y-borrar): `projects merge` con su lápida y su `--undo`, `canonical(id)`, las respuestas a las sugerencias por CLI y los hallazgos de `doctor`. Borrar un proyecto es de la entrega 2b. El diseño está en [D15 a D25](design.md#entrega-2a-unir-proyectos).

### Definiciones

- **Unir X en Z:** pasar a Z todo lo que está bajo X, y dejar a X como lápida. Para el usuario, X desaparece.
- **Lápida:** un proyecto con `merged_into`. No recibe nada: cada id que lo nombra lleva al proyecto en el que se unió. Queda porque los puentes abiertos guardan sus ids durante toda la sesión (`ids_for`, `crates/clawg-cli/src/mcp/server.rs:860-897`) y los exports viejos los llevan.
- **Proyecto vivo:** uno sin `merged_into`.
- **`canonical(id)`:** el proyecto vivo al que lleva una lápida. Para un proyecto vivo, o un id que `project` no conoce, el mismo id.
- **Parte del usuario de un `workspace.md`:** lo que hay entre `<!-- USER-START -->` y `<!-- USER-END -->` (`crates/clawg-core/src/memory/workspace.rs:15-16`).

## PL11. V28 agrega la lápida y el diario de las uniones

V28 MUST:

- agregar a `project` las columnas `merged_into` y `merged_at`, NULL por defecto, y un índice parcial por `merged_into`;
- crear `project_merge`, una fila por unión: `merge_id` (clave primaria), `source`, `target`, `move_id`, `snapshot`, `merged_at`, `undone_at` y `undo_move_id`;
- crear `project_merge_change`, una fila por cosa que cambió una unión, con el valor que tenía antes: `merge_id`, `kind`, `a` y `b`, con un índice por `merge_id` y sin `CHECK` sobre `kind`.

V28 MUST NOT cambiar ninguna fila.

#### Escenario: un store en V27 abre con todos sus proyectos vivos
- GIVEN un store en V27 con proyectos, carpetas, remotos, sugerencias y filas en las tres tablas
- WHEN se abre con V28
- THEN ninguna fila cambió, ningún proyecto tiene `merged_into`, y `canonical` de cada id es el mismo id
- AND `GET /v1/projects` devuelve lo mismo que antes
- AND hay un snapshot `pre-migration`

**Tests:** `a_database_from_the_previous_version_opens_with_every_project_live` (`crates/clawg-core/src/memory/db.rs`, sembrado con `seed_on_disk(dir, Some(27))`); la fila 28 en `published_migration_checksums_are_pinned` (`migrations.rs`).

## PL12. Cada entrada del daemon que recibe un proyecto sigue la lápida

Una unión deja cada lápida apuntando a un proyecto vivo (PL13), así que `canonical` da un solo salto. Aun así, MUST seguir `merged_into` hasta `MAX_MERGE_HOPS` (8) saltos. Un ciclo, el tope alcanzado o un `merged_into` que `project` no conoce MUST devolver el id del que partió, sin error, con un `warn` en el log. `doctor` lo nombra (PL18).

`canonical` MUST correr en estos lugares:

| Dónde | Qué id | Cuándo |
|---|---|---|
| `POST /v1/memories` | `project_id` | al entrar, antes de buscar la sesión registrada y de escribir el outbox |
| el worker | el proyecto del evento | antes de leer nada del proyecto (la dedup, la clave, el detector y el insert); cubre un save que estaba en el outbox, una reanudación al arrancar y `clawg outbox retry` |
| `POST /v1/search`, `POST /v1/context`, `POST /v1/sessions`, `POST /v1/synthesis` | `project_id` | al entrar; en `context` también para el camino del `workspace.md` y los conteos de `synthesize` |
| `GET /v1/disputes`, `GET /v1/memories`, `GET /v1/stats` | `project_id`, `project` | al entrar |
| `PATCH /v1/memories/{kind}/{id}`, `POST /v1/memories/move` | `project_id`, `to_project` | en la transacción que mueve, donde se comprueba el destino |
| el `--undo` de un movimiento | el proyecto de origen de cada fila | en la transacción que mueve |
| `POST /v1/projects`, `project_id_in`, `plan_ids_in` | el proyecto que recibe la raíz | al decidir, así que la respuesta, `memories --here`, `export --project` y el plan del import nombran el proyecto vivo |
| el import | el proyecto de cada fila | en la transacción que la escribe |

Lo que administra proyectos MUST NOT seguir la lápida: responde 409 `project_merged`, con `detail` que nombra el proyecto vivo. Son:

- `PATCH /v1/projects/{id}`;
- cualquiera de los dos lados de `POST /v1/projects/keep-apart` (PL16);
- el destino de `POST /v1/projects/merge`, y su origen cuando es la lápida de otro proyecto que el destino (PL13);
- `from_project` de `POST /v1/memories/move`: «las filas de X» ya no quiere decir nada después de unirlo.

`POST /v1/memories/move` con un `undo` que es el movimiento de una unión, o el de su `--undo`, MUST responder 409 `part_of_merge` nombrando la unión: deshacer sólo las filas dejaría las demás cosas en Z y las filas bajo una lápida.

#### Escenario: un puente abierto antes de la unión
- GIVEN un puente que registró `/work/app` y guarda su proyecto X
- WHEN X se une en Z, y el puente guarda, busca y pide contexto con X
- THEN el save queda bajo Z, con la sesión que el hook registró, y la búsqueda y el contexto leen Z

#### Escenario: un save que falló antes de la unión
- GIVEN un save `failed` bajo X
- WHEN X se une en Z y después se corre `clawg outbox retry` sobre ese save
- THEN el save queda bajo Z

**Tests:** `canonical_follows_a_merge_and_leaves_any_other_id_as_it_is` y `a_cycle_or_a_chain_too_long_answers_the_id_itself` (`crates/clawg-core/src/memory/merge.rs`); `a_save_retried_after_a_merge_lands_in_its_target` (`worker.rs`); `a_bridge_opened_before_a_merge_reads_and_writes_its_target`, `a_move_from_a_merged_project_is_refused` y `the_move_of_a_merge_is_undone_only_with_its_merge` (`crates/clawg-cli/tests/api.rs`).

## PL13. `POST /v1/projects/merge` une X en Z

Con `{"source": X, "target": Z, "dry_run": false}`, detrás del token. Se rechaza sin escribir:

- 404 si X o Z no están en `project`;
- 400 si X y Z son el mismo id;
- 409 `project_merged` si Z es una lápida, o si X es la lápida de otro proyecto que Z. Si X ya es una lápida de Z, la unión es una **reparación**: pasa a Z lo que todavía esté bajo X (PL18), y X sigue siendo la lápida que era;
- 409 `saves_pending`, con su número y `retryable`, si después de esperar hasta `SAVES_WAIT` (5 s) el outbox sigue teniendo un save pendiente para X o para Z. La transacción que escribe MUST comprobarlo de nuevo.

Cuando hay algo que cambiar, la unión MUST escribir un snapshot `pre-merge` y, en una transacción que vuelve a decidir lo que vio la pasada en seco:

1. **`memory`:** cada fila bajo X, borradas incluidas, pasa a Z con la parte de su unidad que está en X. Las filas de la unidad que están en otro proyecto quedan donde están: una unidad que ya estaba partida sigue partida, ahora entre Z y ese proyecto, y el informe la nombra en `split`. Después, Z conserva el save más nuevo de cada `topic_key` y la fila más vieja de cada `content_hash` que toca una fila movida, y las demás quedan `superseded` con causa `revision`, como en R17. Todo queda en `memory_move` y `memory_move_supersede` bajo un número de movimiento, el `move_id` de la unión.
2. **`episodic_trajectory` y `procedural_skill`:** sus filas bajo X pasan a Z.
3. **`synthesis_review`:** cada fila con el alcance X pasa al alcance Z con el mismo agente. Si Z ya tiene una, queda el `reviewed_at` más viejo de las dos.
4. **Carpetas y remotos:** las filas de `project_path` con `project_id` X, y las de `project_remote` de X, pasan a Z.
5. **Lápidas:** las que apuntaban a X pasan a apuntar a Z.
6. **Sugerencias:** las pendientes cuyos dos lados pasan a ser el mismo proyecto quedan respondidas `merged`.
7. **Los proyectos:** X queda con `merged_into = Z` y `merged_at`, y Z queda revisado.
8. **El diario:** una fila de `project_merge` y, por cada cosa de los pasos 2 a 7, una de `project_merge_change` con el valor que tenía antes.

Después de la transacción, el `workspace.md` de X de cada agente MUST copiarse a Z cuando Z no tiene uno. Uno que Z ya tiene se deja como está, y el informe nombra los dos. El archivo de X nunca se cambia ni se borra. Las copias hechas se anotan en el diario. Una copia que falla queda en el log y en el informe, y la unión vale igual.

La unión MUST NOT cambiar el texto, el vector, el índice de texto ni `last_modified_at` de ninguna fila, los pares de `memory_conflict`, el nombre de X, el id ni `index_docs` de ninguna carpeta, ni el índice de docs, que sigue por carpeta. No necesita el modelo.

Con `dry_run`, MUST responder el mismo informe sin escribir, sin snapshot y sin tocar archivos.

El informe tiene `dry_run`, `merge_id`, `source`, `target`, `move_id`, `memories` (cuántas filas pasaron), `superseded`, `split`, `protected`, `trajectories`, `skills`, `reviews`, `folders`, `remotes`, `tombstones`, `suggestions`, `workspaces` (`copied`, `kept_beside`, `failed`) y `snapshot`.

#### Escenario: unir A en B
- GIVEN dos proyectos A y B, cada uno con filas en las tres tablas, una fila de `synthesis_review`, un `workspace.md`, una carpeta y un remoto
- WHEN se une A en B
- THEN todo está bajo B: filas, carpetas y remotos, y A es una lápida de B
- AND `search_memory` desde la carpeta de A encuentra lo que se guardó en A
- AND un save con el id A queda bajo B
- AND el `workspace.md` de A sigue como estaba, y el de B también

#### Escenario: la misma clave en los dos proyectos
- GIVEN un save bajo la clave `storage/engine` en A y otro más nuevo, con la misma clave, en B
- WHEN se une A en B
- THEN el de B queda vivo y el de A queda `superseded` por él, con causa `revision`

#### Escenario: unir el destino de otra unión
- GIVEN A unido en B
- WHEN se une B en C
- THEN la lápida de A apunta a C, y `canonical(A)` es C en un salto

#### Escenario: una prueba en seco
- WHEN se une A en B con `dry_run`
- THEN el informe dice lo que pasaría, y el store, los archivos y la carpeta de backups quedan como estaban

**Tests:** `merging_moves_every_row_and_locator_into_the_target`, `a_merge_keeps_the_newest_save_of_a_key_and_the_oldest_of_a_text`, `a_merge_moves_the_part_of_a_split_unit_it_holds`, `merging_a_target_repoints_its_tombstones`, `a_dry_run_merge_changes_nothing` y `a_merge_copies_a_workspace_only_where_the_target_has_none` (`merge.rs`); `a_merge_waits_for_the_saves_of_its_projects` y `merging_through_the_daemon_leaves_everything_under_the_target` (`tests/api.rs`).

## PL14. El `--undo` de una unión

Con `{"undo": N, "dry_run": false}`. Se rechaza sin escribir:

- 404 si la unión N no existe;
- 409 `merge_undone` si ya se deshizo;
- 409 `merged_later` si su destino es ahora una lápida, nombrando la unión que hay que deshacer antes;
- 409 `saves_pending`, como en PL13.

Cuando hay algo que deshacer, MUST escribir un snapshot `pre-undo` y, en una transacción:

1. X vuelve a estar vivo: `merged_into` y `merged_at` como estaban antes de la unión.
2. **`memory`:** vuelven las filas que la unión dejó `superseded`, si siguen así, y las que siguen en Z vuelven a X, como lo haría el `--undo` del movimiento de la unión y como un movimiento propio (`undo_move_id`).
3. Las filas de `episodic_trajectory` y `procedural_skill`, las carpetas, los remotos y las lápidas que la unión cambió y siguen apuntando a Z vuelven a X.
4. Vuelven las filas de `synthesis_review` del alcance X. La de Z vuelve a su valor de antes, si sigue con el que escribió la unión.
5. Las sugerencias que la unión respondió `merged`, si siguen así, vuelven a estar pendientes, y Z vuelve a su marca de revisado de antes.
6. La unión queda con `undone_at`.

Después de la transacción, cada copia de `workspace.md` que hizo la unión MUST borrarse si su parte del usuario sigue siendo la del archivo de X. Si no, queda, y el informe la nombra.

Lo que se guardó en Z después de la unión queda en Z. Una fila que se movió a otro lado después queda donde está, y el informe la nombra en `kept`. El informe tiene la misma forma que el de PL13.

#### Escenario: unir y deshacer
- GIVEN A unido en B, como en el escenario de PL13
- WHEN se deshace la unión
- THEN el store está como antes de unir: filas, estados, carpetas, remotos y `synthesis_review`, y la copia del `workspace.md` ya no está

#### Escenario: lo guardado después
- GIVEN A unido en B, y después un save nuevo en B y la copia del `workspace.md` editada en su parte del usuario
- WHEN se deshace la unión
- THEN el save nuevo sigue en B, y la copia editada queda y se nombra

#### Escenario: una cadena
- GIVEN A unido en B, y B unido en C
- WHEN se pide deshacer la primera unión
- THEN se rechaza con `merged_later`, nombrando la segunda
- AND después de deshacer la segunda, la primera se deshace

**Tests:** `undoing_a_merge_leaves_the_store_as_it_was`, `undoing_a_merge_keeps_what_was_saved_after_it`, `a_merge_whose_target_was_merged_since_is_undone_after_that_one` y `a_workspace_edited_since_the_merge_is_kept` (`merge.rs`).

## PL15. Lo que se ve de una lápida

Cambia PL9:

- `GET /v1/projects` MUST NOT listar una lápida. Cada proyecto vivo lista en `merged` las lápidas que apuntan a él, cada una con `id`, `name`, `merge_id` y `merged_at`.
- Una carpeta con `project_id` NULL cuyo proyecto por R16 es una lápida MUST listarse en el proyecto vivo.
- Una sugerencia MUST listarse con sus lados por `canonical`. Una cuyos lados son el mismo proyecto no se lista, y dos con el mismo par se listan una vez.
- `PATCH /v1/projects/{id}` sobre una lápida responde 409 `project_merged` (PL12). El nombre de una lápida queda libre y no cuenta para `name_taken`. Su id sigue contando.

**Tests:** `a_merged_project_is_listed_under_its_target_and_frees_its_name` (`crates/clawg-core/src/memory/registry.rs`); `the_listing_after_a_merge_shows_the_target_alone` (`tests/api.rs`).

## PL16. Mantener dos proyectos separados

`POST /v1/projects/keep-apart` con `{"project_id": A, "other_id": B}`, detrás del token. Se rechaza sin escribir con 404 si A o B no están en `project`, 400 si son el mismo proyecto y 409 `project_merged` si alguno es una lápida.

MUST responder `apart` en cada sugerencia pendiente cuyos lados son A y B por `canonical`. Si no hay ninguna, MUST grabar una con motivo `user` y esa respuesta, para que ningún registro la sugiera después. A y B quedan revisados. Responde `{"project_id", "other_id", "answered"}`, con cuántas sugerencias se respondieron.

#### Escenario: una copia que es otro proyecto
- GIVEN la sugerencia `root_commit` entre `app` y `copia` (PL5)
- WHEN se corre `clawg projects keep-apart app-8b6d copia-85a9`
- THEN la sugerencia ya no se lista, y registrar `copia` otra vez no crea otra

**Tests:** `keeping_two_projects_apart_answers_their_suggestion_for_good` (`registry.rs`); `a_pair_kept_apart_is_not_suggested_again` (`project.rs`).

## PL17. `clawg projects merge` y `clawg projects keep-apart`

- `clawg projects merge <SOURCE> --into <TARGET> [--dry-run] [--profile P]`
- `clawg projects merge --undo N [--dry-run] [--profile P]`
- `clawg projects keep-apart <A> <B> [--profile P]`

Los tres MUST hablar con el daemon que corre, sin arrancar uno, e imprimir en el idioma de la preferencia, con una sentencia por `match` sobre `Lang`. El informe de una unión dice:

- cuántas memorias pasaron y cuántas quedaron reemplazadas;
- las unidades partidas y las preferencias y decisiones que pasaron;
- las carpetas, los remotos y los alcances de `synthesize`;
- las copias de `workspace.md`, y cada par que quedó lado a lado;
- el snapshot;
- el número de la unión, con el comando que la deshace.

Un rechazo nombra los ids y la causa del daemon tal como llegó. `saves_pending` dice que se vuelva a intentar. `clawg projects list` MUST nombrar debajo de cada proyecto las lápidas que apuntan a él, con el comando que deshace cada unión.

**Tests:** `projects_merge_and_keep_apart_parse` (`cli.rs`); `a_merge_report_names_what_moved_and_the_way_back` y `a_listing_names_what_was_merged_into_a_project` (`crates/clawg-cli/src/projects.rs`, en los dos idiomas).

## PL18. `doctor` nombra los duplicados, los ids sin proyecto y las lápidas rotas

Tres chequeos nuevos, que leen el store sin su lock, como los demás. En un store anterior a V28 no hay lápidas que mirar, y en uno anterior a V27 no corren.

| `check_id` | `reason_code` | Cuándo | `safe_next_step` |
|---|---|---|---|
| `project_duplicates` (`warn`) | `same_name` | dos proyectos vivos con el mismo nombre, sin distinguir mayúsculas | `clawg projects merge <el más nuevo> --into <el más viejo> --dry-run` |
| | `same_first_commit` | carpetas de dos proyectos vivos con el mismo primer commit que sus remotos no contradicen: alguna de las dos no tiene remoto, o tienen el mismo | el mismo |
| `project_orphans` (`warn`) | `no_project` | un id con filas en `memory`, `episodic_trajectory` o `procedural_skill` que `project` no tiene | ninguno |
| `project_merges` (`warn`) | `rows_under_merged` | filas bajo una lápida | `clawg projects merge X --into <su proyecto vivo> --dry-run` (la reparación de PL13) |
| | `broken_merge` | una lápida cuya cadena no termina en un proyecto vivo: un ciclo, el tope de PL12 o un id que `project` no tiene | ninguno |

`project_duplicates` MUST saltear los pares con una sugerencia respondida (`apart` o `merged`). La evidencia nombra ids, nombres, motivos y conteos, nunca el texto de una fila. Ningún paso pide confirmación: una unión en seco no escribe nada.

**Tests:** `the_duplicates_are_named_by_name_and_by_first_commit`, `a_pair_kept_apart_is_no_duplicate`, `an_id_without_a_project_is_named` y `rows_under_a_merged_project_and_a_broken_chain_are_named` (`crates/clawg-cli/src/doctor/checks.rs`); la lectura de los hechos en `crates/clawg-core/src/memory/inspect.rs`; `the_findings_are_the_same_in_both_languages_but_their_message` sigue pasando con los hallazgos nuevos.

## PL19. Ninguna ruta nueva se olvida de la lápida

- Un test MUST leer `router()` (`crates/clawg-cli/src/daemon/api.rs`) y fallar cuando una ruta no está en su tabla. La tabla dice, de cada ruta, si recibe un proyecto y en qué campo, o si no recibe ninguno.
- Otro test MUST mandar a cada ruta de la tabla que recibe un proyecto el id de una lápida, y comprobar que actúa sobre el proyecto vivo, o que responde `project_merged` donde PL12 lo pide.

**Tests:** `every_route_says_which_project_it_takes` y `every_route_that_takes_a_project_follows_a_merge` (`tests/api.rs`).

## PL20. Lo que no cambia en la entrega 2a

- El contrato del modelo (`agent_contract.json`, `agent_contract.es.json`): ninguna herramienta, argumento ni descripción cambia, y el agente no responde sugerencias (entrega 3).
- La respuesta de `POST /v1/projects` tiene la misma forma, con el proyecto vivo en `project_id`.
- Las uniones de un registro (R4, R17) siguen como están: su `merge_in` deja en su lugar la unidad que toca otro proyecto, porque el id viejo no queda como lápida.
- El índice de docs, los ids de las carpetas y `index_docs`.
- Borrar un proyecto, y un save bajo el id de uno borrado, son de la entrega 2b. Los nombres donde hoy van ids son de la entrega 3.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (sin volver a bendecir), y los de R4, R17 y PL1 a PL10.
