# project-locators — Especificación

Esta especificación cubre cuatro entregas de la [propuesta](proposal.md):

- la **entrega 1, el registro (V27)** (PL1 a PL10). Cambia R10, R16, R18 y R20 de [`specs/project`](../../specs/project/spec.md) y suma V27 a [`specs/storage`](../../specs/storage/spec.md);
- la **entrega 2a, unir proyectos (V28)** (PL11 a PL20, [más abajo](#entrega-2a-unir-proyectos)). Cambia PL9 y suma V28 a `specs/storage` y tres hallazgos a [`specs/doctor`](../../specs/doctor/spec.md);
- la **entrega 2b, borrar proyectos (V29)** (PL21 a PL31, [más abajo](#entrega-2b-borrar-proyectos));
- la **entrega 3a, proyectos sin carpeta y el agente** (PL32 a PL42, [más abajo](#entrega-3a-proyectos-sin-carpeta-y-el-agente)). Suma a `specs/project` y a [`specs/mcp`](../../specs/mcp/spec.md), sin migración.

Lo que no se nombra aquí sigue como está. Las entregas 3b y 4 agregan sus requisitos a este archivo cuando les toque. El [diseño](design.md) dice cómo se cumple cada requisito y qué alternativas se descartaron.

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
- **Parte del usuario de un `workspace.md`:** lo que la regeneración de la sección AUTO conserva tal cual (`split_existing`, `crates/clawg-core/src/memory/workspace.rs`): lo que hay antes de `<!-- AUTO-START -->`, entre `<!-- USER-START -->` y `<!-- USER-END -->`, y después.

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

**Tests:** `canonical_follows_a_merge_and_leaves_any_other_id_as_it_is` y `a_cycle_or_a_chain_too_long_answers_the_id_itself` (`crates/clawg-core/src/memory/merge.rs`); `a_save_retried_after_a_merge_lands_in_its_target` (`worker.rs`); `a_move_to_a_merged_project_lands_in_its_target` y `undoing_a_move_whose_origin_was_merged_returns_to_its_target` (`relocate.rs`); `a_root_whose_project_was_merged_registers_into_its_target`, también leyendo y planeando sin escribir (`project.rs`); `an_import_planned_before_a_merge_writes_into_its_target` (`import.rs`); `a_bridge_opened_before_a_merge_reads_and_writes_its_target`, `a_move_from_a_merged_project_is_refused` y `the_move_of_a_merge_is_undone_only_with_its_merge` (`crates/clawg-cli/tests/api.rs`).

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

**Tests:** `merging_moves_every_row_and_locator_into_the_target`, `a_merge_keeps_the_newest_save_of_a_key_and_the_oldest_of_a_text`, `a_merge_moves_the_part_of_a_split_unit_it_holds`, `merging_a_target_repoints_its_tombstones`, `a_dry_run_merge_changes_nothing`, `a_merge_copies_a_workspace_only_where_the_target_has_none`, `a_merge_is_refused_while_a_save_of_its_projects_is_pending`, `a_merge_of_a_tombstone_into_its_target_repairs_it` y `a_merge_is_refused_for_an_unknown_the_same_or_a_merged_project` (`merge.rs`); `decide_merge_carrying_takes_the_joined_part_of_a_unit_elsewhere` (`relocate.rs`); `a_merge_waits_for_the_saves_of_its_projects` y `merging_through_the_daemon_leaves_everything_under_the_target` (`tests/api.rs`).

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

Un rechazo nombra los ids y la causa del daemon tal como llegó. `saves_pending` dice que se vuelva a intentar. `clawg projects list` MUST nombrar debajo de cada proyecto las lápidas que apuntan a él, con el comando que deshace cada unión, y, cuando hay sugerencias pendientes, terminar con los dos comandos que las responden.

**Tests:** `projects_merge_and_keep_apart_parse` (`cli.rs`); `a_merge_report_names_what_moved_and_the_way_back`, `a_listing_names_what_was_merged_into_a_project` y `a_listing_marks_what_was_created_alone_and_the_suggestions` (`crates/clawg-cli/src/projects.rs`, en los dos idiomas); `projects_are_merged_and_kept_apart_through_the_daemon` (`projects.rs`, contra un daemon real).

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

## Entrega 2b: borrar proyectos

Cubre la parte de borrar de la [entrega 2](proposal.md#2-unir-revisar-y-borrar): `projects delete` en sus dos etapas, con su `--undo`, y `purge --project-delete`. El diseño está en [D26 a D37](design.md#entrega-2b-borrar-proyectos). Las tres elecciones que hizo la persona el 2026-10-25 están en D27, D33 y D34.

### Definiciones

- **Filas de un proyecto:** las de `memory`, `episodic_trajectory` y `procedural_skill` cuyo `project_id` es el proyecto.
- **Fila viva:** una con `deleted_at` NULL, cualquiera sea su `status`. Una fila `superseded` está viva.
- **Proyecto vacío:** uno sin filas vivas. Puede tener filas borradas.
- **Proyecto borrado:** uno con `deleted_at`. No se lista y su nombre queda libre, pero su id sigue siendo suyo.
- **Borrado N:** una fila de `project_delete`. El de la etapa 1 borra un proyecto vacío (`with_memories` falso). El de la etapa 2 borra un proyecto con sus filas.
- **Conjunto de un borrado:** el proyecto, los ids de raíz de sus carpetas (`project_path` con su `project_id`), sus remotos (`project_remote`) y cada fila del proyecto, viva o borrada, con su id, `status`, `deleted_at` y `update_count`.
- **Código:** los primeros 8 caracteres hexadecimales, en minúscula, del SHA-256 de una forma canónica del conjunto (D29).
- **Borrado abierto:** uno sin `undone_at` ni `purged_at`.
- **Volver al usarse:** que un proyecto borrado deje de estarlo porque recibió una fila viva (PL22).

## PL21. V29 agrega la marca de borrado, su diario y los disparadores que lo revierten

V29 MUST:

- agregar a `project` la columna `deleted_at`, NULL por defecto;
- crear `project_delete`, una fila por borrado: `delete_id` (clave primaria), `project_id`, `with_memories`, `code`, `snapshot`, `deleted_at`, `undone_at` y `purged_at`, con un índice por `project_id`;
- crear `project_delete_row`, una fila por cada fila del proyecto que estaba en el conjunto: `delete_id`, `tbl` (`memory`, `episodic_trajectory` o `procedural_skill`), `row_id` y `deleted_at`, que es el que tenía antes del borrado (NULL si la borró el borrado), con un índice por `delete_id`;
- crear `project_delete_folder`, una fila por carpeta a la que el borrado le bajó `index_docs`: `delete_id` y `root_id`;
- reemplazar `memory_project_ai` y `memory_project_au`, y crear `memory_project_restored` (PL22).

V29 MUST NOT cambiar ninguna fila.

#### Escenario: un store en V28 abre sin proyectos borrados
- GIVEN un store en V28 con proyectos vivos, una lápida, carpetas, remotos y filas vivas y borradas en las tres tablas
- WHEN se abre con V29
- THEN ninguna fila cambió y ningún proyecto tiene `deleted_at`
- AND un save bajo un id que `project` no conoce lo adopta como antes (PL2)
- AND hay un snapshot `pre-migration`

**Tests:** `a_database_from_the_previous_version_opens_with_no_project_deleted` (`crates/clawg-core/src/memory/db.rs`, sembrado con `seed_on_disk(dir, Some(28))`); la fila 29 en `published_migration_checksums_are_pinned` (`migrations.rs`).

## PL22. Un proyecto borrado vuelve al usarse

Un disparador de `memory` MUST dejar vivo, con `reviewed = 0`, al proyecto borrado que recibe una fila viva:

- `memory_project_ai`, después de insertar una fila viva con `project_id`. Sigue adoptando un id que `project` no conoce (PL2);
- `memory_project_au`, después de cambiar el `project_id` de una fila viva;
- `memory_project_restored`, después de que una fila con `project_id` pasa de borrada a viva.

Así vuelve con:

- un save de un puente abierto, que guarda los ids de su raíz toda la sesión (`ids_for`, `crates/clawg-cli/src/mcp/server.rs:860`);
- un save que estaba en el outbox, y `clawg outbox retry`;
- un import;
- `clawg undelete`;
- el `--undo` de un movimiento que le devuelve filas vivas;
- la unión de los ids viejos de un registro (R4).

MUST NOT revivirlo lo que no deja una fila viva bajo su id: registrar una de sus carpetas (que sigue llevando a él), buscar, pedir contexto, registrar una sesión o insertar una fila ya borrada.

Lo que lo revive MUST NOT tocar sus filas borradas, el `index_docs` de sus carpetas ni su borrado. El borrado sigue abierto, y su `--undo` (PL26) y su purga (PL28) siguen valiendo.

`episodic_trajectory` y `procedural_skill` no llevan disparador: en producción nadie las escribe (`insert_episodic_trajectory` y `promote_to_procedural_skill`, `crates/clawg-core/src/memory/store_insert.rs`, sólo tienen llamadas en tests). `doctor` nombra una fila viva bajo un proyecto borrado (PL30).

#### Escenario: un puente abierto después del borrado
- GIVEN un puente que registró `/work/app`, y el proyecto A de esa carpeta borrado con sus memorias
- WHEN el puente guarda con el id A
- THEN la memoria queda bajo A, y A vuelve a listarse, sin revisar, con esa memoria sola
- AND las memorias que borró el borrado siguen borradas, y el borrado sigue en `deletes` con `back`

#### Escenario: un hook en la carpeta de un proyecto borrado
- GIVEN el proyecto A de `/work/app`, borrado
- WHEN corre `session-start` en `/work/app`
- THEN el registro responde A, el contexto no trae memorias de A, y A sigue borrado

**Tests:** `a_live_row_brings_a_deleted_project_back_unreviewed`, por un insert, por un cambio de proyecto y por una restauración, y `a_deleted_row_or_a_registration_leaves_it_deleted` (`crates/clawg-core/src/memory/project_delete.rs`); `a_bridge_opened_before_a_delete_brings_its_project_back` (`crates/clawg-cli/tests/api.rs`).

## PL23. Borrar un proyecto vacío (etapa 1)

`POST /v1/projects/delete` con `{"project_id": X}`, detrás del token. Se rechaza sin escribir:

- 404 si X no está en `project`;
- 409 `project_merged` si X es una lápida, con `detail` que nombra el proyecto vivo;
- 409 `project_deleted` si X ya está borrado, con `detail` que nombra su borrado abierto;
- 409 `saves_pending`, con su número y `retryable`, si después de esperar hasta `SAVES_WAIT` (5 s) sigue habiendo en el outbox un save pendiente para X o para una lápida de X. La transacción que escribe MUST comprobarlo de nuevo.

Si X tiene filas vivas, MUST responder el informe de PL24 con `refused: "not_empty"` y sin escribir. El informe dice cuántas filas vivas tiene en cada tabla y nombra los candidatos para vaciarlo (D36):

- los proyectos de sus sugerencias pendientes, con los lados por `canonical` y con su motivo;
- los proyectos vivos con el mismo nombre, sin distinguir mayúsculas (`same_name`);
- los proyectos vivos que tienen una carpeta con el mismo primer commit que una de X y cuyos remotos no la contradicen (`same_first_commit`), con la regla de PL18.

Un par ya respondido (`apart` o `merged`) y un proyecto borrado MUST NOT ser candidatos.

Si X está vacío, MUST hacer lo siguiente en una transacción, sin snapshot:

1. anotar en `project_delete_row` cada fila del proyecto, todas borradas, con su `deleted_at`;
2. bajar `index_docs` de cada carpeta de X que lo tenía, anotarla en `project_delete_folder` y sacar del índice de docs lo que haya bajo su id de raíz;
3. dejar a X con `deleted_at`;
4. escribir la fila de `project_delete` con `with_memories` falso.

El borrado MUST NOT cambiar las carpetas (salvo `index_docs`), los remotos, las sugerencias, las lápidas que llevan a X, `synthesis_review` ni los `workspace.md`.

#### Escenario: un proyecto que ya se vació
- GIVEN un proyecto A cuyas memorias se movieron a B con `projects move`, con una memoria borrada y una carpeta con índice de docs
- WHEN se borra A
- THEN A no se lista, y `deletes` lo nombra con su número y una fila
- AND su carpeta tiene `index_docs` en 0, y el índice de docs de su raíz está vacío

#### Escenario: un proyecto con memorias
- GIVEN un proyecto A con tres memorias vivas, y un proyecto B con el mismo nombre
- WHEN se borra A
- THEN no se escribe nada, y el informe cuenta las tres memorias y nombra a B con `same_name`

**Tests:** `an_empty_project_is_deleted_and_its_docs_index_goes`, `a_project_with_live_rows_is_refused_with_its_candidates`, `a_delete_is_refused_for_an_unknown_a_merged_or_a_deleted_project` y `a_delete_waits_for_the_saves_of_its_project` (`project_delete.rs`).

## PL24. El informe y el código (etapa 2, primer paso)

`{"project_id": X, "with_memories": true}` MUST responder el informe del conjunto de X y su código, sin escribir nada y sin esperar al outbox. Se rechaza como en PL23, salvo por `saves_pending`: el informe cuenta los saves pendientes, y el segundo paso los espera.

El informe (`DeleteReport`) tiene:

- `project_id`, `name`, `with_memories`, `refused` (`not_empty`, `code_changed` o null), `code`, `delete_id` (cuando escribió) y `snapshot`;
- `live` y `deleted`: cuántas filas vivas y cuántas borradas tiene en cada tabla (`memories`, `trajectories`, `skills`);
- de sus filas vivas de `memory`: `by_type`, `by_author` (el `agent_id`, o `null` para las filas que no guardó ningún agente) y `by_status`;
- `protected`: los ids de sus `user_preference` y `architectural_decision` vivas;
- `split_pairs`: cuántos pares de `memory_conflict` unen una fila suya con una de otro proyecto o de ninguno. Quedan como están;
- `conclusions`: los ids de las conclusiones vivas de otro proyecto, o de ninguno, que salen de una fila suya. Con el borrado pasan a `to_review` ([`specs/synthesis`](../../specs/synthesis/spec.md), R6);
- `workspaces`: los `workspace.md` del proyecto, que no se tocan;
- `folders`: los ids de raíz de las carpetas cuyo índice de docs se quita, y `remotes`: sus remotos, que siguen vinculados;
- `pending`: cuántos saves del proyecto esperan en el outbox, y `failed`: cada save `failed` del proyecto, con su `operation_id` y su agente, porque reintentarlo lo haría volver;
- `candidates`, como en PL23;
- los campos del `--undo` (PL26): `restored` y `kept`.

El código MUST resumir el conjunto exacto (D29), y MUST NOT guardarse antes del segundo paso.

#### Escenario: el informe no escribe nada
- GIVEN un proyecto A con memorias de dos agentes, una preferencia, un par con una memoria de B y una conclusión de B que sale de una memoria de A
- WHEN se pide el informe de A
- THEN cuenta las memorias por tipo, autor y estado, nombra la preferencia, cuenta el par, nombra la conclusión y da un código
- AND el store, el índice de docs y la carpeta de backups quedan como estaban

#### Escenario: lo que cambia el código
- GIVEN el código de A
- WHEN llega un save a A, se edita la importancia de una memoria de A, cambia su estado, se restaura una borrada, o A gana una carpeta o un remoto
- THEN el código de A es otro

**Tests:** `the_report_counts_by_type_author_and_status_and_writes_nothing` y `the_code_changes_with_a_save_an_edit_a_restore_or_a_locator`, y la pura `the_code_of_a_set_does_not_depend_on_its_order` (`project_delete.rs`).

## PL25. Borrar con sus memorias (etapa 2, segundo paso)

`{"project_id": X, "with_memories": true, "confirm": CÓDIGO}` se rechaza como en PL23, `saves_pending` incluido. Si el código no es el del conjunto de ahora, MUST responder el informe nuevo, con su código y `refused: "code_changed"`, sin escribir. La transacción que escribe MUST recalcularlo.

Si el código coincide, MUST escribir un snapshot `pre-delete` y, en una transacción:

1. marcar borradas a la vez, con el mismo `deleted_at`, todas las filas vivas del proyecto en las tres tablas, y sacar sus vectores. El índice de texto lo resta su disparador (V8);
2. anotar en `project_delete_row` cada fila del conjunto: las que borró, con `deleted_at` NULL, y las que ya estaban borradas, con el suyo;
3. llevar a su estado de reposo a cada fila de afuera que disputaba una fila borrada (`release_challengers`, `crates/clawg-core/src/memory/dispute.rs:197`);
4. el índice de docs y `index_docs`, como en PL23;
5. dejar a X con `deleted_at` y escribir la fila de `project_delete` con `with_memories`, el código y el snapshot.

Como las filas se marcan juntas, MUST NOT revivir ninguna fila del proyecto (`REPLACED_SQL` sólo mira filas vivas, `crates/clawg-core/src/memory/revival.rs:67-74`). Una fila de afuera que una fila del proyecto reemplazó sigue reemplazada, como con cualquier borrado.

El borrado MUST NOT cambiar el texto, el `status`, el `update_count` ni el `last_modified_at` de ninguna fila. Tampoco los pares de `memory_conflict`, los enlaces de `memory_derivation`, las carpetas (salvo `index_docs`), los remotos, las sugerencias, `synthesis_review`, los `workspace.md` ni el outbox. No necesita el modelo.

#### Escenario: borrar A con sus memorias
- GIVEN un proyecto A con memorias vivas y borradas, una clave con dos versiones, una memoria de B que una de A reemplazó por veredicto, una memoria de B que disputaba una de A, y un `workspace.md`
- WHEN se borra A con el código de su informe
- THEN todas las memorias de A están borradas y ninguna revivió, la de B reemplazada sigue así y la que disputaba vuelve a `active`
- AND A no se lista, y `deletes` lo nombra con su número
- AND el `workspace.md` de A sigue como estaba, y hay un snapshot `pre-delete`

#### Escenario: un save entre los dos pasos
- GIVEN el informe de A con su código
- WHEN llega un save a A y después se confirma con ese código
- THEN no se escribe nada, y la respuesta trae el informe con la memoria nueva y otro código

**Tests:** `deleting_with_memories_marks_every_row_and_revives_none`, `a_delete_keeps_outside_rows_replaced_and_releases_their_disputes`, `a_stale_code_is_refused_with_the_new_report` y `a_delete_with_memories_waits_for_the_saves_of_its_project` (`project_delete.rs`); `deleting_through_the_daemon_takes_two_steps` (`tests/api.rs`).

## PL26. El `--undo` de un borrado

`{"undo": N, "dry_run": false}`. Se rechaza sin escribir:

- 404 si el borrado N no existe;
- 409 `delete_undone` si ya se deshizo, y 409 `delete_purged` si se purgó;
- 409 `deleted_later` si el mismo proyecto tiene, después de N, otro borrado abierto, nombrándolo;
- 409 `saves_pending`, como en PL23;
- 503 `embedder_not_ready` o `embedder_failed` si hay filas que restaurar y el modelo no está listo. Una prueba en seco no lo necesita.

Cuando hay algo que deshacer, MUST escribir un snapshot `pre-undo`, calcular antes los vectores de las filas que vuelven, y en una transacción:

1. dejar vivo a X si sigue con el `deleted_at` de N. Si volvió al usarse, ya lo está;
2. subir `index_docs` en las carpetas de `project_delete_folder` que siguen en 0. El próximo refresco reconstruye su índice;
3. restaurar cada fila que N borró y sigue como N la dejó, en X y con el `deleted_at` de N. Vuelve con `deleted_at` en NULL, su vector y su entrada en el índice de texto, y con el `status`, el `update_count` y el `last_modified_at` de antes. Después se lleva a su estado de reposo a la fila y a las que la disputaban (`settle_row`, `settle_challengers`);
4. dejar a N con `undone_at`.

Una fila que N borró MUST quedar borrada, y se nombra en `kept` con su motivo, cuando:

- `live_twin`: una fila viva de X, guardada después del borrado, tiene su texto (`content_hash`);
- `topic_taken`: una fila viva de X, de otro save, tiene su `topic_key`;
- `holds_secret`: `redact` encuentra en su texto un secreto con las reglas de hoy;
- `changed`: ya no está como N la dejó (se restauró, se volvió a borrar sola o se movió);
- `tied`: es una pieza de un save que tiene otra pieza que queda.

Son las negativas de `undelete` ([`specs/restore`](../../specs/restore/spec.md), R1). Una fila que queda borrada deja como están las que reemplazó, como lo haría su borrado con la clave tomada. Las filas que ya estaban borradas antes de N MUST NOT restaurarse. El `--undo` no corre el detector: los pares que tenía siguen ahí, y no se calculan pares con lo guardado después, como en un import.

#### Escenario: borrar y deshacer
- GIVEN el proyecto A borrado con sus memorias, como en el escenario de PL25
- WHEN se deshace el borrado
- THEN el store está como antes de borrar: filas, estados, vectores, índice de texto, disputas, `index_docs`, y A en la lista
- AND el próximo refresco reconstruye el índice de docs

#### Escenario: lo guardado mientras estaba borrado
- GIVEN A borrado con sus memorias. Después, un save en A con el mismo texto que una memoria borrada, y otro bajo una clave que tenía otra
- WHEN se deshace el borrado
- THEN los dos saves nuevos siguen vivos
- AND las dos memorias viejas quedan borradas y se nombran, con `live_twin` y `topic_taken`, y las demás vuelven

**Tests:** `undoing_a_delete_leaves_the_store_as_it_was`, `undoing_a_delete_keeps_what_was_saved_since`, `a_delete_followed_by_another_is_undone_after_it` y `undoing_an_empty_delete_needs_no_embedder` (`project_delete.rs`); `undoing_a_delete_needs_the_model` (`tests/api.rs`).

## PL27. Lo que administra proyectos no toma uno borrado

MUST responder 409 `project_deleted`, con `detail` que nombra el borrado abierto y su `--undo`:

- `PATCH /v1/projects/{id}`;
- cualquiera de los dos lados de `POST /v1/projects/keep-apart`;
- el origen o el destino de `POST /v1/projects/merge`, y el `--undo` de una unión cuyo destino está borrado;
- `to_project` de `POST /v1/memories/move`, y `project_id` de `PATCH /v1/memories/{kind}/{id}`.

`from_project` de `POST /v1/memories/move` sí acepta un proyecto borrado: sus filas borradas se pueden sacar antes de purgarlo. El `--undo` de un movimiento que le devuelve filas a un proyecto borrado lo deja vivo (PL22), porque esas filas eran suyas.

Lo que se ve, como con una lápida (PL15):

- `GET /v1/projects` MUST NOT listar un proyecto borrado, ni sus carpetas, ni las lápidas que llevan a él, ni sus sugerencias. Una sugerencia con un lado borrado no se lista hasta que ese lado vuelva.
- MUST responder también `deletes`: cada borrado abierto, con `delete_id`, `project_id`, `name`, `with_memories`, `deleted_at`, `rows` y `back`. `rows` son las filas de su conjunto que siguen borradas, lo que su purga sacaría; `back` dice si el proyecto volvió al usarse.
- El nombre de un proyecto borrado queda libre para `rename`. Su id no.

La tabla de PL19 suma `POST /projects/delete`, con el proyecto en `body.project_id`, que responde `project_merged` a una lápida. Un test MUST mandar a cada ruta de esa tabla que recibe un proyecto el id de un proyecto borrado, y comprobar que pasa una de tres cosas: lo lee vacío, lo hace volver con un save, o responde `project_deleted` donde este requisito lo pide.

**Tests:** `a_deleted_project_is_refused_by_what_manages_projects` (`registry.rs`, `merge.rs` y `relocate.rs`); `a_deleted_project_is_listed_only_in_deletes_and_frees_its_name` (`registry.rs`); `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule`, y la ruta nueva en `every_route_says_which_project_it_takes` y en `every_route_that_takes_a_project_follows_a_merge` (`tests/api.rs`).

## PL28. `clawg purge --project-delete N`

`clawg purge --project-delete N --yes [--profile P]` MUST seguir las reglas de `clawg purge <ID>` ([`specs/secrets`](../../specs/secrets/spec.md), R5): el daemon parado (toma el lock del store), sin migraciones pendientes, sin snapshot, y sin escribir nada antes de que pasen todos los chequeos. MUST:

- rechazar, sin escribir, un borrado que no existe, uno deshecho y uno ya purgado;
- en una transacción, sacar para siempre cada fila de N en `project_delete_row` que siga borrada, esté en el proyecto que esté, en las tres tablas: la fila y su vector y, en `memory`, lo que `purge` saca de una fila borrada (la cabeza en `memory_piece.context` si es la primera pieza, sus enlaces de `memory_derivation` en los que es conclusión, sus pares de `memory_conflict` y las filas del outbox con su texto y su agente);
- sacar otra vez del índice de docs lo que haya bajo las carpetas de `project_delete_folder` que siguen con `index_docs` en 0, porque un refresco en curso pudo escribir después del borrado;
- dejar a N con `purged_at`;
- al final, una sola vez, el `optimize` del índice de texto, el `VACUUM` y el WAL vacío (`leave_no_old_copy`, `crates/clawg-core/src/memory/secrets.rs:326`).

Una fila de N que está viva (se restauró después) MUST quedar, y la salida la nombra. El proyecto queda en `project` como está, borrado o vivo.

La salida MUST decir cuántas filas sacó de cada tabla, qué filas del outbox sacó y cuáles quedaron vivas. También MUST nombrar los backups que todavía guardan alguno de sus textos (el snapshot `pre-delete` entre ellos), los `workspace.md` del proyecto y los demás que guardan alguno de sus textos, sin cambiar ninguno. `<ID>` y `--project-delete` se excluyen, y los dos piden `--yes`: sin él MUST negarse antes de abrir nada.

#### Escenario: purgar un borrado
- GIVEN A borrado con sus memorias, con memorias que ya estaban borradas antes, y el daemon parado
- WHEN se corre `clawg purge --project-delete N --yes`
- THEN ninguna memoria del conjunto de N está en el store, ni en su índice de texto, ni en el archivo
- AND la salida nombra el snapshot `pre-delete` y el `workspace.md` de A

**Tests:** `purging_a_delete_leaves_no_trace_of_its_rows`, `purging_a_delete_keeps_a_row_restored_since`, `a_purge_of_a_delete_undone_or_purged_is_refused_untouched` y `purging_a_delete_names_the_backups_and_workspaces_that_hold_its_text` (`crates/clawg-core/src/memory/secrets.rs`); `purge_takes_an_id_or_a_delete` (`cli.rs`); `the_purge_report_of_a_delete_names_what_went`, en los dos idiomas (`crates/clawg-cli/src/secrets.rs`); `a_deleted_project_is_purged_once_the_daemon_stops` (`crates/clawg-cli/tests/binary.rs`).

## PL29. `clawg projects delete`

- `clawg projects delete <ID> [--profile P]`
- `clawg projects delete <ID> --with-memories [--confirm CÓDIGO] [--profile P]`
- `clawg projects delete --undo N [--dry-run] [--profile P]`

Los tres MUST hablar con el daemon que corre, sin arrancar uno, e imprimir en el idioma de la preferencia, con una sentencia por `match` sobre `Lang`:

- **Un proyecto vacío borrado:** que se borró, su número y `clawg projects delete --undo N`. Si tiene memorias borradas, también `clawg purge --project-delete N --yes`.
- **`not_empty`:** cuántas filas vivas tiene y cómo vaciarlo: `clawg projects merge X --into C --dry-run` por cada candidato, `clawg projects move --from-project X --to-project <ID>`, o borrarlo con `--with-memories`. Sale con 1.
- **El informe:** lo de PL24, con cada save `failed` y su `clawg outbox discard <OPERATION_ID> --agent <AGENTE>`, terminado con el comando exacto `clawg projects delete X --with-memories --confirm CÓDIGO`. Sale con 0.
- **`code_changed`:** lo dice, imprime el informe nuevo con su comando y sale con 1.
- **Un borrado confirmado:** cuántas memorias borró, el snapshot, su número, `--undo N` y `clawg purge --project-delete N --yes`.
- **Un `--undo`:** cuántas filas volvieron y cada una que quedó, con su motivo.

`clawg projects list` MUST terminar con los borrados abiertos, cada uno con su `--undo N` y, si tiene filas, su purga. Uno cuyo proyecto volvió al usarse lo dice. Un rechazo nombra los ids y la causa del daemon tal como llegó, y `saves_pending` dice que se vuelva a intentar.

**Tests:** `projects_delete_parses` (`cli.rs`); `a_delete_report_names_what_goes_and_the_exact_confirm`, `a_refused_delete_names_the_ways_to_empty_it` y `a_listing_ends_with_the_open_deletes`, en los dos idiomas (`crates/clawg-cli/src/projects.rs`); `a_project_is_deleted_and_undone_through_the_daemon` (`projects.rs`, contra un daemon real).

## PL30. `doctor` y los proyectos borrados

- `project_duplicates` MUST NOT tomar un proyecto borrado.
- Para `project_orphans` y `project_merges`, un proyecto borrado es un proyecto: sus filas no son huérfanas, y una cadena de lápidas que termina en él no está rota.
- Un chequeo nuevo, `project_deletes`, MUST nombrar con `warn` y `live_under_deleted` cada proyecto borrado que tiene filas vivas en alguna de las tres tablas. Sólo puede pasar en `episodic_trajectory` o `procedural_skill` (PL22), o en un store editado a mano. Su evidencia nombra el proyecto, su borrado abierto y los conteos, y no tiene paso. En un store anterior a V29 no corre.

**Tests:** `a_deleted_project_is_no_duplicate_nor_orphan` y `live_rows_under_a_deleted_project_are_named` (`crates/clawg-cli/src/doctor/checks.rs`); la lectura de los hechos nuevos en un store en V28 y en V29 (`crates/clawg-core/src/memory/inspect.rs`); `the_findings_are_the_same_in_both_languages_but_their_message` sigue pasando.

## PL31. Lo que no cambia en la entrega 2b

- El contrato del modelo (`agent_contract.json`, `agent_contract.es.json`): ninguna herramienta borra un proyecto, ninguna llama a la ruta de PL23, y `delete_memory` sigue igual.
- `clawg purge <ID>`. Y `clawg undelete <ID>`, que ahora además hace volver al proyecto de su fila (PL22).
- Las uniones de antes: las lápidas que llevan a un proyecto borrado siguen llevando a él.
- El índice de docs sigue por carpeta, y los ids de las carpetas no cambian.
- El formato de export (entrega 4).

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, sin volver a bendecir, y los de R4, R17 y PL1 a PL20.

## Entrega 3a: proyectos sin carpeta y el agente

Cubre la primera parte de la [entrega 3](proposal.md#3-proyectos-sin-carpeta-y-el-agente): proyectos sin carpeta, el argumento `project` del contrato, los nombres donde hoy van ids, `projects create|locate`, las sugerencias que ve y responde el agente, y la sugerencia por nombre de carpeta. Los subproyectos de un monorepo son de la entrega 3b. El diseño está en [D38 a D49](design.md#entrega-3a-proyectos-sin-carpeta-y-el-agente). Las cuatro elecciones que hizo la persona el 2026-10-26 están en D41, D42, D45 y D49. No trae migración: `born` y `project_suggestion.reason` no llevan `CHECK` (D6, D10).

### Definiciones

- **Referencia a un proyecto:** un texto que el usuario o el agente da donde va un proyecto. Es un id o un nombre.
- **Clave de un nombre (`name_key`):** el nombre en minúsculas de Unicode, sin nada que no sea letra o dígito. `Mi App`, `mi-app` y `mi_app` tienen la misma clave, `miapp`.
- **Proyecto parecido a un nombre:** un proyecto vivo o borrado, que no es una lápida, cuya clave de nombre es la del nombre.
- **Proyecto sin carpeta:** uno sin filas en `project_path` que lleven a él. Puede tener remotos.

## PL32. Una referencia se resuelve primero como id y después como nombre

`POST /v1/projects/resolve`, detrás del token, con `{"project": REF, "create": bool, "new_project": bool, "born": "agent"|"user"}`. MUST resolver así, y responder 200 en todos los casos que leen el registro:

1. Si REF es el id de un proyecto, ese proyecto, por `canonical` (una lápida lleva a su proyecto vivo). Un proyecto borrado se resuelve a sí mismo, con `deleted: true`: un save bajo él lo hace volver (PL22).
2. Si no, los proyectos vivos cuyo nombre es REF sin espacios a los lados, sin distinguir mayúsculas (en Rust, no con `lower()` de SQLite). Con uno, ese. Con dos o más, `refused: "ambiguous"` y los dos en `candidates`.
3. Si no, los proyectos parecidos a REF, vivos y borrados, en `candidates` con `refused: "similar"`. Un candidato borrado lleva `deleted: true` y su borrado abierto.
4. Si no hay ninguno y no se pidió `create`, `refused: "unknown"`.
5. Si no hay ninguno y se pidió `create`, MUST crear el proyecto (PL33) y responderlo con `created: true`.

Con `new_project: true` y `create`, el paso 3 MUST NOT impedir la creación: se crea aunque haya parecidos. El paso 2 sigue: un nombre que ya tiene un proyecto vivo responde ese proyecto, nunca uno nuevo con el mismo nombre.

La respuesta resuelta trae `project_id`, `name`, `created`, `deleted` y sus carpetas (`root_id`, `index_docs`), las más viejas primero. Cada candidato trae `id`, `name`, `deleted` y, si está borrado, `delete_id`. Nada se escribe fuera del paso 5.

Un REF vacío, de más de 80 caracteres o con caracteres de control es un 400 que no escribe nada.

#### Escenario: un parecido no crea nada
- GIVEN un proyecto `mi-app-1a2b` llamado `mi-app`
- WHEN se resuelve `Mi App` con `create`
- THEN no se crea nada, y la respuesta es `similar` con `mi-app-1a2b`
- AND con `new_project` se crea un proyecto `Mi App`, sin carpeta

#### Escenario: un nombre que nadie tiene
- GIVEN ningún proyecto parecido a `ideas de producto`
- WHEN se resuelve con `create`
- THEN se crea un proyecto `ideas de producto` con un id `ideas-de-producto-XXXX`, `born = 'agent'` y sin revisar

**Tests:** `a_reference_is_an_id_before_a_name`, `a_name_two_projects_hold_is_ambiguous`, `a_similar_name_creates_nothing_unless_asked`, `a_deleted_project_is_a_candidate_by_name_and_itself_by_id` y `a_reference_is_bounded` (`crates/clawg-core/src/memory/registry.rs`); `a_reference_resolves_through_the_daemon` (`crates/clawg-cli/tests/api.rs`).

## PL33. Un proyecto nuevo sin carpeta

Un proyecto que crea PL32 MUST tener:

- `name`: REF sin espacios a los lados;
- `id`: `slug-4hex`, con `is_project_id`. El slug es el nombre en minúsculas, con cada racha de lo que no es una letra ASCII o un dígito cambiada por un `-`, sin `-` a los lados y con 40 caracteres como mucho; `project` si queda vacío. Los cuatro hex salen de un hash del nombre, la hora y un contador, y el id MUST NOT ser el de otro proyecto ni el de una carpeta;
- `born = 'agent'` con `reviewed = 0`, o `born = 'user'` con `reviewed = 1` cuando lo pide la CLI;
- ninguna carpeta ni remoto.

La transacción que lo crea MUST volver a mirar el paso 2 de PL32: si mientras tanto un proyecto vivo tomó ese nombre, responde ese.

**Tests:** `a_new_project_gets_a_slug_and_four_hex_digits` (pura) y `a_project_is_created_once_by_name` (`registry.rs`).

## PL34. El argumento `project` de las herramientas

`save_memory`, `search_memory`, `recall_context`, `search_project` y `synthesize` MUST aceptar un argumento opcional `project`, un id o un nombre. `update_memory` MUST aceptar `to_project`. Con un `project_path` (o un `to_project_path`) a la vez, la herramienta MUST negarse sin llamar al daemon. Sin él, la herramienta MUST comportarse como hoy.

- `save_memory` resuelve con `create` antes de guardar, así que el proyecto existe antes de que el save entre al outbox. Acepta además `new_project: true` (PL32), y sin `project` MUST negarse a tomarlo.
- Las demás resuelven sin `create`: un nombre que no es de ningún proyecto MUST NOT crear nada.
- Un rechazo de PL32 MUST responder, sin guardar ni buscar, una frase que nombra cada candidato con su id y su nombre (y los borrados como tales) y dice qué hacer: preguntarle a la persona, y después usar el id, o `new_project` para un proyecto aparte. Un borrado se trae de vuelta guardando con su id.
- `search_project` MUST buscar en la carpeta con `index_docs` más vieja del proyecto, o en la más vieja. Un proyecto sin carpeta MUST responder su propio mensaje: no tiene archivos que buscar en esta máquina.
- `all_projects` y `project` juntos: `all_projects` gana, como gana hoy sobre `project_path`.

Cuando el directorio del puente no da ningún proyecto y la llamada no nombra uno, la búsqueda sigue sin alcance, como hoy.

#### Escenario: guardar en una idea
- GIVEN ningún proyecto parecido a `ideas de producto`
- WHEN el agente guarda con `project: "ideas de producto"`
- THEN se crea el proyecto y la memoria queda bajo él
- AND `search_memory` con `project: "ideas de producto"` la encuentra, y desde la carpeta del puente no

#### Escenario: sin el argumento
- THEN cada herramienta hace lo de hoy, y los tests de hoy pasan sin cambios

**Tests:** `saving_to_a_project_by_name_creates_it`, `a_similar_name_is_answered_with_its_candidates`, `project_and_project_path_are_refused_together`, `searching_the_docs_of_a_project_without_a_folder_says_so` (`crates/clawg-cli/src/mcp/server.rs`).

## PL35. Nombres donde hoy van ids

`to_project` de `POST /v1/memories/move` (`clawg projects move --to-project`) y `project_id` de `PATCH /v1/memories/{kind}/{id}` MUST aceptar un id o un nombre: primero como id, sin seguir la lápida (eso lo hace `target_in` en la transacción, PL12), y después como el nombre de un proyecto vivo, sin distinguir mayúsculas. No crean nada ni buscan parecidos. Un nombre de ninguno MUST seguir siendo el 404 de hoy, nombrándolo, y uno de dos o más MUST ser 409 `project_ambiguous` con `detail` que nombra sus ids.

**Tests:** `a_move_and_a_patch_take_a_project_by_name` y `a_name_two_projects_hold_is_refused_as_a_target` (`tests/api.rs`).

## PL36. Las sugerencias pendientes en el contexto

`POST /v1/context`, con consulta y sin ella, MUST responder `suggestions`: las sugerencias pendientes del proyecto, leídas como las lista `GET /v1/projects` (los lados por `canonical`, sin las que tienen un lado borrado, una por par), cada una con `other_id`, `other_name`, `reason` y `created_at`. Sin proyecto, o sin sugerencias, el campo se omite.

`recall_context` y `session-start` MUST terminar con una línea por sugerencia, en el idioma de la sesión, que nombra los dos proyectos, el motivo, y que hay que preguntarle a la persona y responder con `answer_suggestion`. Se muestran hasta que se responden. Un daemon que no manda el campo no muestra nada.

**Tests:** `the_context_names_the_pending_suggestions_of_its_project` (`tests/api.rs`); `a_suggestion_is_shown_until_answered` (`crates/clawg-cli/src/mcp/tools.rs`, en los dos idiomas); `session_start_names_the_pending_suggestions` (`crates/clawg-cli/src/hook.rs`).

## PL37. El agente responde una sugerencia

`POST /v1/projects/suggestions/answer`, detrás del token, con `{"project_id": P, "other_id": O, "answer": "merge"|"apart", "user_answered": true, "reason": "..."}`. MUST:

- responder 400 sin escribir cuando `user_answered` no es `true` o `reason` está vacío, como un veredicto que pide la respuesta de la persona;
- responder 404 cuando P u O no existen, y 404 `no_suggestion` cuando no hay una sugerencia pendiente entre los dos, leída por `canonical`;
- tratar a P y a O como lo que administra proyectos: 409 `project_merged` para una lápida y 409 `project_deleted` para un borrado (PL15, PL27);
- con `apart`, hacer lo de `keep_apart` (PL16);
- con `merge`, unir O en P como `POST /v1/projects/merge` (PL13), con su snapshot, su diario y su número, y responder su informe. `--undo` la deshace como a cualquier unión.

La herramienta `answer_suggestion` (`project` opcional, por defecto el del puente; `other`, `answer`, `reason`, `user_answered`) MUST llamar a esa ruta y decir qué hizo: con una unión, su número y `clawg projects merge --undo N`.

#### Escenario: responder «separados»
- GIVEN una copia de `/work/app` en `/work/copia`, con su sugerencia `root_commit`
- WHEN el agente responde `apart` con la respuesta de la persona
- THEN la sugerencia queda `apart`, deja de mostrarse en `recall_context` y `session-start`, y registrar la copia otra vez no la vuelve a crear

**Tests:** `answering_a_suggestion_needs_the_users_answer`, `answering_apart_keeps_the_pair_from_coming_back`, `answering_merge_merges_and_is_undone` y `answering_without_a_pending_suggestion_is_404` (`tests/api.rs`); `answer_suggestion_says_what_it_did` (`server.rs`).

## PL38. Una carpeta con el nombre de un proyecto cuyas carpetas ya no están

Un registro con `assigned` de un camino nuevo, que no se vincula por remoto ni por mudanza (PL5) y cuyo proyecto es uno nuevo, MUST buscar los proyectos vivos, distintos del suyo, cuya clave de nombre es la del nombre de la carpeta (el último componente de la raíz), que tienen al menos una carpeta y cuyos checkouts grabados ya no están, ninguno. Con cada uno MUST dejar una sugerencia `folder_name`, con las reglas de PL6: una por par, sin mover nada, y sólo un registro con `assigned`. El daemon mira en disco esos checkouts en la misma pasada que los de una mudanza posible.

Un proyecto con una carpeta presente, uno sin carpetas, una lápida y un borrado MUST NOT recibir esta sugerencia.

#### Escenario: una carpeta sin VCS que se mueve
- GIVEN `/work/notas`, sin `.git`, registrado y con memorias en su proyecto `notas`
- WHEN se mueve a `/archivo/notas` y se registra `/archivo/notas`
- THEN recibe un proyecto propio y una sugerencia `folder_name` con `notas`
- AND con `/work/notas` todavía ahí no hay sugerencia

**Tests:** `a_folder_named_as_a_project_whose_folders_are_gone_is_suggested`, `a_folder_named_as_a_project_still_there_is_not_suggested` (`crates/clawg-core/src/memory/project.rs`); `a_folder_without_vcs_moved_is_suggested_and_shown_until_answered` (`tests/api.rs`).

## PL39. `clawg projects create` y `clawg projects locate`

- `clawg projects create <NOMBRE> [--new] [--profile P]` MUST resolver NOMBRE con `create` y `born = 'user'` (PL32), con `new_project` si se pasa `--new`. Imprime el proyecto creado, o el que ya tenía ese nombre, o los candidatos y que `--new` crea uno aparte (sale con 1).
- `clawg projects locate <PROYECTO> <CAMINO> [--dry-run] [--profile P]` MUST hacer que la raíz de CAMINO lleve a PROYECTO (un id o un nombre, que no sigue la lápida). El daemon registra la raíz como un puente (`POST /v1/projects/locate`, con `{"project", "root_path", "dry_run"}`; CAMINO tiene que ser un directorio). Si su proyecto ya es PROYECTO, no hace nada. Si no, ese proyecto MUST tener esa carpeta sola, o la respuesta es 409 `folder_shared`, que nombra el proyecto y la unión que haría falta; y con una sola carpeta MUST unirlo en PROYECTO como `POST /v1/projects/merge` (PL13), con su `--undo`.

Los dos MUST hablar con el daemon que corre, sin arrancar uno, e imprimir en el idioma de la preferencia, con una sentencia por `match` sobre `Lang`.

**Tests:** `projects_create_and_locate_parse` (`cli.rs`); `a_created_project_or_its_candidates_are_printed` (`crates/clawg-cli/src/projects.rs`, en los dos idiomas); `locating_a_folder_merges_its_own_project_into_the_named_one` y `a_folder_shared_by_its_project_is_not_located` (`tests/api.rs`).

## PL40. Ninguna ruta nueva se olvida de la lápida

La tabla de PL19 suma:

- `POST /projects/resolve`, con el proyecto en `body.project`, que lleva una lápida a su proyecto vivo;
- `POST /projects/suggestions/answer`, con `body.project_id`, que responde `project_merged` a una lápida;
- `POST /projects/locate`, con `body.project`, que responde `project_merged` a una lápida.

**Tests:** las tres en `every_route_says_which_project_it_takes`, `every_route_that_takes_a_project_follows_a_merge` y `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule` (`tests/api.rs`).

## PL41. El contrato del modelo

`agent_contract.json` y `agent_contract.es.json` MUST cambiar sólo por los argumentos de PL34 y la herramienta de PL37, y se vuelven a bendecir. `protocol.md` y `protocol.es.md` MUST decir, fuera de su apertura, cuándo usar `project` (un proyecto sin carpeta, como una idea, o uno que no es el del directorio), que un nombre con parecidos se pregunta, y que una sugerencia se le pregunta a la persona antes de `answer_suggestion`. La apertura de cada variante sigue en 2048 caracteres o menos.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (vueltos a bendecir); los de `protocol.rs` y `tools_es.rs`.

## PL42. Lo que no cambia en la entrega 3a

- El esquema: ninguna migración.
- La resolución de una raíz (R1, PL4, PL5): un subproyecto de un monorepo sigue siendo parte del repositorio (entrega 3b).
- `POST /v1/projects` y los ids que da: el puente y los hooks siguen igual sin el argumento nuevo.
- `rename`, `merge`, `keep-apart` y `delete` siguen tomando ids.
- El import no crea sugerencias ni proyectos sin carpeta (entrega 4).
