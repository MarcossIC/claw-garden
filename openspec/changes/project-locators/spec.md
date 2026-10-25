# project-locators — Especificación

Esta especificación cubre la **entrega 1, el registro (V27)**, de la [propuesta](proposal.md#1-el-registro-v27). Cambia R10, R16, R18 y R20 de [`specs/project`](../../specs/project/spec.md) y suma V27 a [`specs/storage`](../../specs/storage/spec.md). Lo que no se nombra aquí sigue como está. Las entregas 2 a 4 agregan sus requisitos a este archivo cuando les toque. El [diseño](design.md) dice cómo se cumple cada requisito y qué alternativas se descartaron.

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

## Lo que no cambia

- Ninguna fila de `memory` cambia de `project_id` por la migración ni por un registro, salvo las uniones de R4 y R17, como hoy.
- El índice de docs sigue por id de carpeta. Una mudanza le da a la raíz nueva su propia carpeta y su propio índice. Un proyecto sin carpeta no tiene docs.
- `doctor` no tiene hallazgos nuevos en esta entrega: los duplicados y los ids sin proyecto son de la entrega 2. Su regla `projects` compara cada carpeta con sus candidatos, como antes, leyéndola de `project_path`.
- No se responde ni se muestra ninguna sugerencia al agente: eso es de las entregas 2 y 3.
- Crear un proyecto sin carpeta, declarar un subproyecto de un monorepo y reconocer una carpeta sin VCS que se movió son de la entrega 3. Esta entrega no los impide (diseño, D14).
