# project-locators — Especificación

Esta especificación cubre seis entregas de la [propuesta](proposal.md):

- la **entrega 1, el registro (V27)** (PL1 a PL10). Cambia R10, R16, R18 y R20 de [`specs/project`](../../specs/project/spec.md) y suma V27 a [`specs/storage`](../../specs/storage/spec.md);
- la **entrega 2a, unir proyectos (V28)** (PL11 a PL20, [más abajo](#entrega-2a-unir-proyectos)). Cambia PL9 y suma V28 a `specs/storage` y tres hallazgos a [`specs/doctor`](../../specs/doctor/spec.md);
- la **entrega 2b, borrar proyectos (V29)** (PL21 a PL31, [más abajo](#entrega-2b-borrar-proyectos));
- la **entrega 3a, proyectos sin carpeta y el agente** (PL32 a PL42, [más abajo](#entrega-3a-proyectos-sin-carpeta-y-el-agente)). Suma a `specs/project` y a [`specs/mcp`](../../specs/mcp/spec.md), sin migración;
- la **entrega 3b, los subproyectos de un monorepo (V30)** (PL43 a PL52, [más abajo](#entrega-3b-los-subproyectos-de-un-monorepo)). Suma V30 a `specs/storage`, un chequeo a `specs/doctor`, y el camino crudo a `specs/project`, [`specs/hooks`](../../specs/hooks/spec.md) y `specs/mcp`;
- la **entrega 4, exportar e importar (V31)** (PL53 a PL65, [más abajo](#entrega-4-exportar-e-importar)). Cambia R1, R4, R5, IL1, HI7, HI16, IU1 e IU2 de [`specs/portability`](../../specs/portability/spec.md) y PL6, PL7 y PL47, y suma V31 a `specs/storage`.

Lo que no se nombra aquí sigue como está. El [diseño](design.md) dice cómo se cumple cada requisito y qué alternativas se descartaron.

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

`recall_context` MUST terminar con una línea por sugerencia, en el idioma de la sesión, que nombra el otro proyecto, el motivo, y que hay que preguntarle a la persona y responder con `answer_suggestion`. `session-start` MUST terminar con una sola línea que nombra hasta dos de los otros proyectos (y cuántos más hay), que hay que preguntar y responder con `answer_suggestion`, y que `recall_context` dice por qué: lo que suma el hook pasa a tener un tope de 1.500 bytes, y con los 8.000 del contexto sigue entrando en los 10.000 caracteres que inyecta el host (R4 de [`specs/hooks`](../../specs/hooks/spec.md)). Se muestran hasta que se responden. Un daemon que no manda el campo no muestra nada.

**Tests:** `the_context_names_the_pending_suggestions_of_its_project` (`tests/api.rs`); `a_suggestion_is_shown_until_answered` (`crates/clawg-cli/src/mcp/tools.rs`, en los dos idiomas); `session_start_names_the_pending_suggestions` y `what_the_hook_adds_stays_small`, con nueve sugerencias (`crates/clawg-cli/src/hook.rs`).

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
- `clawg projects locate <PROYECTO> <CAMINO> [--profile P]` MUST hacer que la raíz de CAMINO lleve a PROYECTO (un id o un nombre, que no sigue la lápida). El daemon registra la raíz como un puente (`POST /v1/projects/locate`, con `{"project", "root_path"}`; CAMINO tiene que ser un directorio). Si su proyecto ya es PROYECTO, no hace nada. Si no, ese proyecto MUST tener esa carpeta sola, o la respuesta es 409 `folder_shared`, que nombra el proyecto y la unión que haría falta; y con una sola carpeta MUST unirlo en PROYECTO como `POST /v1/projects/merge` (PL13), con su `--undo`, que es la forma de volver atrás. No tiene prueba en seco: la raíz se registra antes de saber a qué proyecto lleva.

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

## Entrega 3b: los subproyectos de un monorepo

Cubre la segunda parte de la [entrega 3](proposal.md#3-proyectos-sin-carpeta-y-el-agente): declarar una subcarpeta de un repositorio como un proyecto propio, el camino crudo que manda el cliente y la resolución por la declaración más profunda. El diseño está en [D50 a D59](design.md#entrega-3b-los-subproyectos-de-un-monorepo). Trae V30, que sólo crea una tabla. La clave de una declaración es el proyecto del repositorio más el subcamino, no el remoto, y `project_remote` no cambia (D51).

### Definiciones

- **Camino crudo:** el directorio desde el que se trabaja (el `cwd` de un hook, el directorio de trabajo del puente, un `project_path`), absoluto y canónico, antes de subir a su raíz (R1).
- **Cima de un checkout:** el directorio que tiene el `.git`, sea un directorio o un archivo. En un checkout común es su raíz. En un worktree es el directorio del worktree, y su raíz es la del repositorio principal (R1).
- **Subcamino:** el camino crudo relativo a la cima de su checkout, con sus componentes unidos por `/`. Está vacío en la cima. No hay subcamino fuera de git, ni cuando la raíz de esa cima no es la que se registra.
- **Contener:** un subcamino A contiene a B cuando B es A, o empieza con A seguido de `/`. Se compara por componentes enteros: `apps/web` contiene `apps/web/src`, pero no `apps/webapp`.
- **Proyecto del repositorio:** el proyecto que recibe la raíz (PL4, PL5).
- **Declaración:** una fila de `project_subfolder`. Tiene el proyecto de un repositorio (`parent_id`), un subcamino no vacío (`subpath`) y el proyecto al que va lo que se trabaja dentro de él (`project_id`).
- **Subproyecto:** un proyecto al que lleva una declaración.

## PL43. V30 agrega las declaraciones

V30 MUST crear `project_subfolder` (`parent_id`, `subpath`, `project_id`, `created_at`), con la clave primaria `(parent_id, subpath)` y un índice por `project_id`. MUST NOT cambiar ninguna fila ni ninguna otra tabla: `project_remote` y `project_path` quedan como están.

#### Escenario: un store en V29 abre sin declaraciones
- GIVEN un store en V29 con:
  - proyectos vivos, una lápida y un proyecto borrado con su borrado abierto;
  - dos clones de un remoto en dos caminos, y un camino con dos carpetas (R20);
  - filas vivas y borradas en las tres tablas, y una sugerencia pendiente
- WHEN se abre con V30
- THEN ninguna fila de ninguna tabla cambió, y `project_subfolder` existe y está vacía
- AND hay un snapshot `pre-migration`

**Tests:** `a_database_from_the_previous_version_opens_with_no_subfolder_declared` (`crates/clawg-core/src/memory/db.rs`, sembrado con `seed_on_disk(dir, Some(29))`); la fila 30 en `published_migration_checksums_are_pinned` (`migrations.rs`); los conteos de migraciones aplicadas de `db.rs`, que pasan de 29 a 30.

## PL44. El registro recibe el camino crudo

`POST /v1/projects` MUST aceptar un campo opcional `path`, un camino absoluto. Con `assigned` y `path`, el daemon MUST leer, en el mismo hilo bloqueante en el que lee el checkout, la cima del checkout de `path` y la raíz de esa cima. Toma el subcamino de `path` sólo si esa raíz es `root_path`. Si no lo es, o si `path` no está en un checkout, MUST ignorarlo, dejarlo en el log y responder como sin él.

La respuesta MUST tener la forma de siempre (`project_id`, `root_id`, `rekeyed`):

- `project_id` es el de PL45;
- `root_id` MUST ser siempre el de la carpeta de la raíz.

Sin `path`, con un subcamino vacío o sin `assigned`, MUST responder lo mismo que antes de V30 (R4, R18).

El subcamino MUST salir de una función pura de la cima y el camino (`subpath_of`, `crates/clawg-core/src/memory/identity.rs`). No da nada cuando el camino sale de la cima o cuando uno de sus componentes no es Unicode válido. La lectura del disco va aparte, en `Ground`.

#### Escenario: un worktree
- GIVEN `/work/mono`, con un worktree en `/work/mono-wt`
- WHEN se registra `/work/mono` con `path` `/work/mono-wt/apps/web/src`
- THEN el subcamino es `apps/web/src`

#### Escenario: un camino que no lleva a su raíz
- GIVEN `/work/a` y `/work/b`, dos repositorios
- WHEN se registra `/work/a` con `path` `/work/b/src`
- THEN la respuesta es la de `/work/a` sin `path`, y el log lo dice

**Tests:** la pura `the_subpath_is_measured_from_the_top_of_the_checkout` (`identity.rs`); `a_path_in_a_worktree_has_its_subpath_from_the_worktree` y `a_path_that_does_not_lead_to_its_root_has_no_subpath` (`crates/clawg-core/tests/project_root.rs`, con repositorios reales: necesitan `git` en el PATH); `a_registration_without_a_path_or_with_one_elsewhere_answers_as_before` (`crates/clawg-cli/tests/api.rs`).

## PL45. La declaración más profunda decide

Con un subcamino S no vacío y la raíz asentada en el proyecto P, el registro MUST:

1. leer las declaraciones de P y de las lápidas que llevan a P;
2. quedarse con las que contienen S;
3. elegir la más profunda. A igual subcamino, la de P va antes que la de una lápida, y entre lápidas gana la más vieja;
4. responder como `project_id` el proyecto al que lleva la elegida por `canonical`, pasando las lápidas. Si lleva a P, responde P.

Sin ninguna declaración, responde P. Un subproyecto borrado se responde a sí mismo: registrar no lo revive, y un save sí (PL22). Las declaraciones del subproyecto no se miran: no se encadenan.

Resolver una declaración MUST NOT:

- unir nada al subproyecto. Los ids viejos y el id de la carpeta de la raíz se unen en P, como en R4 y R17;
- cambiar `root_id`, `index_docs`, las carpetas ni los remotos;
- escribir nada que no escribiría el registro sin `path`, ni crear sugerencias.

#### Escenario: un subproyecto de un monorepo
- GIVEN `/work/monorepo` con `origin` `github.com/acme/monorepo` y su proyecto M, y una declaración de `apps/web` hacia el proyecto W
- WHEN se guarda desde `/work/monorepo/apps/web/src` y desde `/work/monorepo`
- THEN la primera memoria queda en W y la segunda en M
- AND `/work/monorepo/apps/webapp` lleva a M
- AND un clone del mismo remoto en `/otra/monorepo` lleva a W desde `/otra/monorepo/apps/web/src` y a M desde `/otra/monorepo`. Es el caso de otra máquina: del clone, el store sólo tiene su camino y su remoto

#### Escenario: un monorepo sin `origin` que se mueve
- GIVEN el mismo monorepo sin `origin`, con su primer commit grabado y la misma declaración
- WHEN se mueve a `/work/mono2` y se registra desde `/work/mono2/apps/web`
- THEN lleva a W, porque la raíz conserva M (PL5)

#### Escenario: después de una unión
- GIVEN la declaración de `apps/web` hacia W
- WHEN se une M en Z
- THEN `apps/web` lleva a W, y la raíz a Z
- AND cuando después se une W en Z, `apps/web` lleva a Z, y el `--undo` de esa unión la devuelve a W

**Tests:** la pura `the_deepest_declared_subfolder_wins` (`crates/clawg-core/src/memory/subfolder.rs`); `a_declared_subfolder_takes_what_is_saved_inside_it`, `a_declaration_follows_a_moved_checkout`, `a_declaration_is_read_through_merges` y `resolving_a_subfolder_joins_nothing` (`project.rs`, con el `Ground` falso); `a_declared_subfolder_takes_what_is_saved_inside_it_in_every_clone` (`tests/api.rs`, con repositorios reales).

## PL46. Declarar, cambiar y quitar una declaración

`POST /v1/projects/locate` MUST aceptar `subfolder: true`. Con él, `root_path` es la carpeta misma, no la raíz que la contiene. Se rechaza sin escribir:

- con 400 si `root_path` no es un directorio, o si su subcamino (PL44, leído de su propio checkout) está vacío o no existe: es la cima de un checkout, o está fuera de un repositorio. La frase lo dice, y dice que `locate` sin `subfolder` ubica una raíz;
- con 404, o 409 `project_ambiguous`, por `project`, como en PL35;
- con 409 `project_merged` si `project` es una lápida, y 409 `project_deleted` si está borrado (PL27).

Después registra la raíz como un puente (con `assigned` y sin `index_docs`) y la asienta en P. Sea X el proyecto nombrado:

- **X es P:** MUST borrar la declaración de S de P y de sus lápidas, y responder `removed: true`. Si no había ninguna, responde `already: true`.
- **La declaración de S ya lleva a X:** responde `already: true` sin escribir.
- **La declaración de S lleva a Q, que no es ni P ni X:** si Q no tiene carpeta, ni remoto, ni otra declaración que lleve a él, MUST unir Q en X como `POST /v1/projects/merge` (PL13), con su snapshot, su diario y su número, y responder su informe. Si no, MUST responder 409 `folder_shared`, con un `detail` que nombra Q y `clawg projects merge Q --into X`.
- **No hay declaración de S, o la que hay lleva a P:** MUST escribir (P, S, X), en lugar de la que había.

MUST NOT mover filas: lo guardado desde la subcarpeta antes de declararla queda en el proyecto al que iba. MUST NOT cambiar tampoco carpetas, remotos ni `index_docs`.

La respuesta trae `project_id` (X), `root_id`, `repository_id` (P), `subpath` (S), `already`, `removed` y, cuando unió, `merge`.

#### Escenario: declarar y quitar
- GIVEN `/work/monorepo` con memorias en M, algunas guardadas desde `apps/web`
- WHEN se declara `apps/web` hacia W, y después hacia M
- THEN las memorias de M no se movieron en ninguno de los dos pasos
- AND después del primero, lo que se guarda desde `apps/web` va a W; después del segundo, va a M, y lo guardado en W sigue en W

#### Escenario: declarar otra vez hacia otro proyecto
- GIVEN `apps/web` declarada hacia Q, y nada más que lleve a Q
- WHEN se declara hacia X
- THEN Q se une en X, la unión tiene número y `--undo`, y `apps/web` lleva a X

**Tests:** `declaring_a_subfolder_writes_one_row_and_moves_nothing`, `naming_the_repository_project_removes_the_declaration`, `declaring_again_merges_a_project_only_that_subfolder_led_to`, `a_project_reached_otherwise_is_not_merged_by_a_declaration` y `a_declaration_into_a_merged_or_deleted_project_is_refused` (`subfolder.rs`); `a_subfolder_is_declared_through_the_daemon_and_refused_at_a_root` (`tests/api.rs`); el brazo de `subfolder` en `every_route_that_takes_a_project_follows_a_merge` y en `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule` (`tests/api.rs`).

## PL47. Los clientes mandan el camino crudo

Cada cliente que registra con `assigned` MUST mandar `path`:

- **`session-start` y `user-prompt-submit`:** el `cwd` de la entrada. Leen, y registran la sesión, bajo el proyecto que responde el daemon.
- **El puente:** su directorio de trabajo para su proyecto por defecto, y el `project_path` o el `to_project_path` de una herramienta, absolutos. MUST registrar una vez por raíz y camino en el mismo proceso: cambia la clave de R6, que era la raíz y el id viejo. Lo que muestra la clave de una fila donde un save la revisaría (`revised_from`) MUST comparar con el proyecto de esa raíz y ese camino.
- **`clawg projects move --to CAMINO` y `--here`:** CAMINO, y el directorio de trabajo.

`clawg memories --here` y `clawg export --project CAMINO` MUST elegir el proyecto con el camino crudo, como un registro, sin escribir (`project_id_of`, `crates/clawg-core/src/memory/portability/export.rs`). En un store anterior a V30 MUST elegir como si no hubiera declaraciones.

`clawg projects add`, `clawg projects locate` sin `--subfolder` y el import no mandan `path`. Los dos primeros tratan la raíz y su índice de docs, y el import es de la entrega 4.

Contra un daemon que no conoce `path`, cada cliente MUST seguir funcionando, con el proyecto de la raíz.

#### Escenario: un puente abierto en el subproyecto
- GIVEN la declaración de `apps/web` hacia W
- WHEN un puente arranca en `/work/monorepo/apps/web` y guarda sin `project_path`, y otro arranca en `/work/monorepo`
- THEN la memoria del primero queda en W, y `recall_context` lee W en el primero y M en el segundo
- AND `search_project` del primero busca en el índice de docs de `/work/monorepo`

**Tests:** `session_start_in_a_declared_subfolder_reads_its_project` (`crates/clawg-cli/src/hook.rs`); `a_bridge_in_a_declared_subfolder_saves_into_its_project`, `a_project_path_in_a_declared_subfolder_goes_to_its_project` y `a_bridge_registers_each_path_of_a_root_once` (`crates/clawg-cli/src/mcp/server.rs`); `moving_to_a_declared_subfolder_moves_into_its_project` (`crates/clawg-cli/src/projects.rs`, contra un daemon real); `the_project_of_a_path_reads_the_declarations_without_writing` y `a_store_from_before_subfolders_reads_the_project_of_the_root` (`export.rs`).

## PL48. `clawg projects locate --subfolder`

`clawg projects locate <PROYECTO> <CAMINO> --subfolder [--profile P]` MUST mandar CAMINO absoluto con `subfolder` (PL46), sin subir a su raíz. MUST hablar con el daemon que corre, sin arrancar uno, e imprimir en el idioma de la preferencia, con una sentencia por `match` sobre `Lang`:

- **Declarada:** que CAMINO lleva a X dentro de P; que lo guardado allí antes sigue en P, con `clawg projects move --from-project P --to-project X --text PALABRAS --dry-run`; y que una sesión abierta antes sigue con P hasta que se reinicia.
- **Quitada:** que CAMINO vuelve a llevar a P, y que lo guardado en X sigue allí, con `clawg projects merge X --into P --dry-run`.
- **Ya estaba**, y **una unión**, como en PL39.

Un rechazo imprime la causa del daemon tal como llegó, y sale con 1.

**Tests:** `projects_locate_takes_subfolder` (`cli.rs`); `a_declared_subfolder_is_printed_with_what_stayed_behind`, en los dos idiomas (`projects.rs`).

## PL49. Lo que se ve de una declaración

- **`GET /v1/projects`** MUST responder en cada proyecto `subfolders`: las declaraciones que llevan a él y las de su repositorio, leídas por `canonical`, cada una bajo cada lado que se lista. Cada una trae `repository_id`, `subpath`, `project_id` y `paths`: cada carpeta del repositorio en esta máquina más el subcamino, con `present` (si ese directorio existe). Un daemon que no manda el campo se lee como sin declaraciones.
- **`clawg projects list`** MUST imprimir, bajo un subproyecto, dónde está declarado (el repositorio y el subcamino) con sus caminos, marcando los que no están. Bajo un repositorio, imprime cada declaración con su proyecto. Un proyecto sin carpeta propia y con declaraciones MUST NOT marcarse «sin carpeta».
- **`POST /v1/projects/resolve`:** para un proyecto sin carpetas propias y con declaraciones, `folders` MUST ser las carpetas de sus repositorios, las más viejas primero. Así, `search_project` con `project` busca en los docs de su repositorio (PL34).

**Tests:** `a_declaration_is_listed_on_both_sides_with_its_paths` y `a_subproject_resolves_to_the_folders_of_its_repository` (`registry.rs`); `a_listing_names_the_subfolders`, en los dos idiomas (`projects.rs`).

## PL50. Unir, borrar y la lápida

- Una unión y su `--undo`, un borrado y su `--undo`, y una purga MUST NOT cambiar `project_subfolder`. Una declaración se lee siempre por `canonical` (PL45, PL49).
- El informe de un borrado (PL24) MUST nombrar en `subfolders` las declaraciones que llevan al proyecto, y `clawg projects delete --with-memories` las imprime.
- El código (D29) MUST sumar, después de los remotos, una línea por cada declaración que lleva al proyecto, sólo si las hay. El código de un proyecto sin declaraciones es el de antes.
- Un save desde una subcarpeta declarada hacia un proyecto borrado lo hace volver (PL22). Registrar desde ella no.

#### Escenario: borrar un subproyecto
- GIVEN W con memorias, y la declaración de `apps/web` hacia W
- WHEN se pide el informe de W, se declara `libs/ui` hacia W, y se confirma con el código del informe
- THEN no se borra nada, y la respuesta es `code_changed`
- AND con el código nuevo W se borra, las dos declaraciones siguen, y un save desde `apps/web` lo hace volver

**Tests:** `merging_and_undoing_leave_the_declarations_as_they_were` (`merge.rs`); `a_delete_report_names_its_subfolders_and_a_declaration_changes_the_code` y la pura `the_code_of_a_project_without_subfolders_does_not_change` (`project_delete.rs`); `a_delete_report_names_its_subfolders`, en los dos idiomas (`projects.rs`).

## PL51. `doctor` y las declaraciones

Un chequeo nuevo, `project_subfolders`, MUST nombrar con `warn` y `broken_subfolder` cada declaración cuyo repositorio o destino no está en `project`. Su evidencia es el repositorio, el subcamino y el destino, y no tiene paso. Sin hallazgos MUST dar `ok`, con cuántas declaraciones hay. Una declaración bajo una lápida, o hacia un proyecto borrado, no es un hallazgo. En un store anterior a V30 no corre.

**Tests:** `a_declaration_whose_projects_are_gone_is_named` (`crates/clawg-cli/src/doctor/checks.rs`); la lectura del hecho nuevo en un store en V29 y en V30 (`crates/clawg-core/src/memory/inspect.rs`); `the_findings_are_the_same_in_both_languages_but_their_message` sigue pasando.

## PL52. Lo que no cambia en la entrega 3b

- `project_remote`, su clave y quienes lo leen. `project_path` guarda sólo raíces, y el índice de docs sigue por carpeta de raíz.
- R1: la raíz de un camino sigue siendo la del primer `.git`. Un subproyecto no es una raíz, y un puente abierto en él busca docs en su repositorio.
- La búsqueda de memorias: desde un subproyecto no se leen las del repositorio (`memory_project_predicate!`, `crates/clawg-core/src/memory/search.rs:107-114`). `project` y `all_projects` llegan a ellas.
- Los ids viejos de un camino se unen en el proyecto del repositorio (R4).
- El contrato del modelo: ninguna herramienta ni argumento cambia. `project_path` sigue siendo «Absolute path of the project», y uno dentro de una subcarpeta declarada lleva a su subproyecto.
- Las sugerencias: declarar no crea ninguna.
- El import y el formato de export (entrega 4). `--project-map NOMBRE=CAMINO` toma la raíz de CAMINO, y una fila de un subproyecto viaja con su id.
- `clawg projects add` y `clawg projects locate` sin `--subfolder`.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, sin volver a bendecir, y los de R1, R4, R6 y PL1 a PL42.

## Entrega 4: exportar e importar

Cubre la [entrega 4](proposal.md#4-exportar-e-importar): la línea `project` del formato lleva el nombre, los localizadores, la lápida y las declaraciones de un proyecto; el import decide adónde va cada proyecto del archivo por su remoto, su id y su nombre antes que por su camino, deja las sugerencias que pide su evidencia y anota en su diario lo que agrega al registro; y `--project-map` acepta un proyecto sin carpeta. El diseño está en [D60 a D70](design.md#entrega-4-exportar-e-importar). Trae V31, que crea una tabla y un índice. El formato sigue en la versión 1.

Cambia lo que fijaron PL6 y PL7 («leer, planear e importar no crean sugerencias»), R4 de [`specs/portability`](../../specs/portability/spec.md) («el import no deja sugerencias»; la raíz antes que todo) y la última línea de PL47 y de PL52 (el import no manda el camino crudo).

### Definiciones

- **Proyecto del archivo:** las líneas `project` de un mismo `id` de un export. Su `id` es el del proyecto en el store de origen.
- **Huellas de un proyecto del archivo:** sus remotos (`remotes`) y el remoto y el primer commit de cada una de sus carpetas (`folders`).
- **Huellas de un proyecto de acá:** los remotos que `project_remote` le ata, y el remoto y el primer commit de cada carpeta de `project_path` que lleva a él.
- **Contradecir:** un proyecto del archivo contradice a uno de acá cuando los dos conocen algún remoto y no comparten ninguno; o, si alguno de los dos no conoce remotos, cuando los dos conocen algún primer commit y no comparten ninguno. Es la regla de `fit` (PL4) sobre conjuntos: el remoto decide antes que el primer commit.
- **Carpeta de otra máquina:** una carpeta de un proyecto del archivo cuyo camino (o el `NEW` que le da `--map-root`) no existe en esta máquina.
- **Línea sin filas:** la de un proyecto del archivo al que no va ninguna línea `memory` del archivo: una lápida, el repositorio de una declaración o un subproyecto declarado.
- **Lo que el import agregó al registro:** los proyectos que creó, los proyectos borrados que hizo volver, las sugerencias que dejó y las declaraciones que escribió. Las carpetas y los remotos ya se anotan en `memory_import_extra` (IU1 de `specs/portability`).

## PL53. V31 agrega el diario del registro y un índice por fecha

V31 MUST:

- crear `memory_import_registry` (`import_id`, `kind`, `a`, `b`, `value`), sin `CHECK` sobre `kind`, con un índice por `import_id`;
- crear `idx_memory_created_at` sobre `memory (created_at)`, sin predicado, porque la presencia de PL62 busca también las filas borradas.

V31 MUST NOT cambiar ninguna fila ni ninguna otra tabla. `memory_import_extra` queda con su `CHECK` y sus filas, y sus tipos siguen diciendo lo mismo.

#### Escenario: un store en V30 abre con sus imports como estaban
- GIVEN un store en V30 con:
  - un import anotado: filas insertadas, una reemplazada, un vínculo rellenado, una carpeta y un remoto en `memory_import_extra`;
  - un proyecto borrado con su borrado abierto, una lápida y una declaración;
  - filas vivas y borradas en las tres tablas
- WHEN se abre con V31
- THEN ninguna fila de ninguna tabla cambió, `memory_import_registry` existe vacía y `idx_memory_created_at` existe
- AND `clawg import --undo N` de ese import retira lo que su diario anotó, como antes de V31
- AND hay un snapshot `pre-migration`

**Tests:** `a_database_from_the_previous_version_opens_with_its_imports_as_they_were` (`crates/clawg-core/src/memory/db.rs`, sembrado con `seed_on_disk(dir, Some(30))`); la fila 31 en `published_migration_checksums_are_pinned` (`migrations.rs`); los conteos de migraciones aplicadas de `db.rs`, que pasan de 30 a 31.

## PL54. Una línea `project` por proyecto

`clawg export` MUST escribir una línea `project` por proyecto, con:

- `id`: el del proyecto, no el de una de sus carpetas;
- `root_path`: el camino de su carpeta más vieja, para un lector de antes. Un proyecto sin carpeta no lo lleva;
- `name`;
- `folders`: cada carpeta que lleva a él, la más vieja primero, con su `root_path` y, si los grabó, su `remote` y su `root_commit`;
- `remotes`: los que `project_remote` le ata, del más viejo al más nuevo;
- `merged_into`, sólo en la línea de una lápida;
- `subfolders`: las declaraciones de las que es el repositorio, cada una con `subpath` y el `project_id` al que lleva.

Un campo vacío MUST NOT escribirse: la línea de un store anterior a V27 sale byte a byte como antes.

MUST escribir la línea de:

1. cada proyecto de una fila exportada;
2. cada lápida que lleva a uno de ellos, con `merged_into` y sin carpetas ni remotos, que pasaron con la unión;
3. cada repositorio con una declaración que lleva a uno de ellos;
4. cada subproyecto al que lleva una declaración de uno de ellos.

MUST NOT viajar: `deleted_at` (D61), `born`, `reviewed`, `created_at`, la marca `git_stamp` ni `index_docs`.

Un store se lee sin migrar, con lo que tiene: antes de V27, una línea por carpeta cuyo id es el proyecto de una fila, sin `name` (como hoy, `crates/clawg-core/src/memory/portability/export.rs:136-142`); antes de V24, sin remotos; antes de V28, sin lápidas; antes de V30, sin declaraciones.

La versión del formato sigue en 1. Un lector de antes lee `id` y `root_path` e ignora lo demás (R1 de `specs/portability`). A una línea sin `root_path` la reporta entre las líneas que no importó y deja las filas de ese proyecto con su id, que es lo que hacía antes de esta entrega.

#### Escenario: un clone y una idea
- GIVEN un proyecto `app` con dos carpetas (la suya y la de un clone del mismo remoto) y un proyecto `ideas de producto` sin carpeta, cada uno con filas
- WHEN se exporta
- THEN hay una línea de `app`, con su id, el camino de la carpeta más vieja en `root_path`, sus dos carpetas con sus huellas y su remoto, y una línea de `ideas de producto` sin `root_path`

#### Escenario: un subproyecto
- GIVEN un monorepo M con la declaración de `apps/web` hacia W, y filas sólo en W
- WHEN se exporta con `--project` desde `apps/web`
- THEN hay una línea de W y una de M con su declaración, y ninguna fila de M

**Tests:** `an_export_writes_one_line_per_project_with_its_locators`, `an_export_of_a_subproject_carries_its_repository_and_declaration`, `a_deleted_project_exports_without_its_mark` y `a_store_from_before_the_registry_exports_its_folders_as_before`, este sembrado en V26 (`crates/clawg-core/src/memory/portability/export.rs`); `the_pinned_example_reads_and_writes_back_unchanged`, con una segunda línea `project` con los campos nuevos, y `a_project_line_carries_its_name_locators_and_subfolders` (`crates/clawg-core/src/memory/portability/format.rs`).

## PL55. El import lee las líneas de antes y las de ahora

- El import MUST leer una línea sin `root_path` y con los campos nuevos. Las líneas de un mismo `id` son un solo proyecto, con las carpetas de todas.
- Un archivo de antes (una línea por carpeta, sin nombre ni huellas) MUST importarse como antes, salvo que su `id` se prueba antes que su camino (PL56).
- Una fila cuyo `project_id` no tiene línea lo conserva, como antes (`crates/clawg-core/src/memory/portability/import.rs:370`). Si `project` no lo tiene, el import MUST crearlo él, con `born = 'import'` y el nombre que da el id, y anotarlo (PL63), en lugar de dejárselo al disparador de PL2.

**Tests:** `a_project_line_without_a_folder_is_read` (`format.rs`); `lines_of_one_id_are_one_project` (`crates/clawg-core/src/memory/portability/projects.rs`, pura); `an_export_from_before_names_and_locators_imports_as_before` y `a_row_without_a_line_creates_its_project_as_the_import` (`import.rs`).

## PL56. Adónde va cada proyecto del archivo

Cada proyecto del archivo MUST ir al primero de estos:

1. **El mapa.** `--map-root OLD=NEW` nombra una de sus carpetas: el proyecto que un registro le da a NEW, con su camino crudo (PL61).
2. **Un remoto.** El primero de sus remotos, en el orden del archivo, que `project_remote` ata acá: ese proyecto, por `canonical`.
3. **El id.** Su id es un proyecto de acá al que no contradice: ese, por `canonical`, así que una lápida lleva a su proyecto vivo. Uno borrado es él mismo (PL59).
4. **El nombre.** Un solo proyecto vivo de acá tiene su nombre, sin distinguir mayúsculas (como el paso 2 de PL32), y ni ese proyecto ni el del archivo tienen huellas: ese. Con huellas de un solo lado, el nombre no decide, y queda la sugerencia de PL58.
5. **El camino.** El camino de una de sus carpetas está grabado acá o existe acá, y no contradice el proyecto que un registro le daría (`plan_ids_in`): ese.
6. **Uno nuevo** (PL57).

Un proyecto del archivo sin nombre (de un archivo de antes) saltea el paso 4. Una línea sin filas MUST NOT crear un proyecto: va adonde la llevan los pasos 2 a 5, o a ninguno. La de una lápida sólo prueba el paso 3. La única excepción es el subproyecto al que lleva una declaración que el import escribe (PL60).

Lo que decide MUST ser una función pura de los proyectos del archivo, del registro leído una vez por pasada del plan y de los planes de sus raíces. La transacción que escribe cada fila MUST seguir `canonical` otra vez (D17).

#### Escenario: un export de antes de una unión
- GIVEN un store con los proyectos X y Z, cada uno con una carpeta, un remoto y filas, y un export de X hecho antes de unir X en Z, con el camino de su carpeta cambiado por el de otra máquina
- WHEN se importa después de la unión
- THEN las filas quedan en Z, `project` tiene los mismos proyectos que antes y X sigue siendo una lápida
- AND con el mismo export sin remotos, las filas también quedan en Z, por el id de X

#### Escenario: un export de otra máquina con el mismo remoto
- GIVEN un proyecto P con la carpeta `/work/app` y el remoto `github.com/acme/app`
- WHEN se importa un export de otra máquina cuyo proyecto `app` tiene otro id, la carpeta `/home/otra/app` y ese remoto
- THEN sus filas quedan en P, y no se crea ningún proyecto

#### Escenario: un proyecto sin carpeta va y vuelve por nombre
- GIVEN un store A con el proyecto `Ideas de producto`, sin carpeta y con filas, y un store B con el proyecto `ideas de producto`, de otro id
- WHEN se exporta de A y se importa en B, y después se exporta de B y se importa en A
- THEN en B las filas de A quedan en `ideas de producto`, y en A las de B quedan en `Ideas de producto`, sin ningún proyecto nuevo
- AND en un store vacío, el export de A crea un proyecto `Ideas de producto` con el id de A

#### Escenario: el mismo camino, otro repositorio
- GIVEN acá `/work/api` con el remoto `github.com/acme/api-b` y filas en su proyecto L
- WHEN se importa un export cuyo proyecto `api` tiene la carpeta `/work/api`, el mismo id que L y el remoto `github.com/acme/api-a`
- THEN sus filas quedan en un proyecto nuevo, y L no recibe ninguna

**Tests:** la pura `a_project_of_the_file_goes_by_map_remote_id_name_then_path` (`projects.rs`); `an_export_from_before_a_merge_lands_in_its_target`, `an_export_from_another_machine_lands_in_the_project_of_its_remote`, `a_project_without_a_folder_goes_and_comes_back_by_name` y `a_project_contradicted_here_is_another_project` (`import.rs`); `an_export_of_another_store_imports_by_remote_and_by_name` (`crates/clawg-cli/tests/api.rs`); `an_import_planned_before_a_merge_writes_into_its_target` y `an_import_into_a_clone_goes_to_the_project_of_its_remote` siguen pasando.

## PL57. Lo que el import registra de cada proyecto

- **Un proyecto nuevo** MUST llevar el id del archivo cuando ni `project` ni `project_path` lo tienen, y si no, `new_project_id` de su nombre (PL33). Su nombre es el del archivo, o el que da su id. Nace con `born = 'import'` y `reviewed = 0`, y queda anotado (PL63).
- **Una carpeta de otra máquina** MUST grabarse en `project_path` sin `index_docs`, con el id que le da el plan, el proyecto al que fue su proyecto del archivo, y el `remote` y el `root_commit` que trae el archivo, sin marca. Una carpeta que existe acá MUST grabarse como lo haría un registro (PL7), salvo que su checkout contradiga a su proyecto del archivo: entonces no es suya y no se graba. Una carpeta ya grabada no cambia.
- **Cada remoto** del proyecto del archivo que `project_remote` no ata MUST atarse al proyecto al que fue. Un remoto ya atado no cambia.
- El import MUST NOT renombrar, marcar revisado, unir ni separar ningún proyecto de acá.

#### Escenario: un checkout sin `origin`, después del import
- GIVEN un export de otra máquina cuyo proyecto `notas` tiene una carpeta sin remoto, con el primer commit `c1`, importado en un store vacío
- WHEN después se registra acá `/work/notas`, un checkout sin `origin` con ese primer commit
- THEN es una mudanza (PL5): lleva al proyecto que creó el import, sin sugerencia

#### Escenario: un id que ya está tomado
- GIVEN acá una carpeta con el id `ideas-1a2b`
- WHEN se importa un proyecto del archivo sin carpeta, con ese id y el nombre `ideas`
- THEN se crea el proyecto `ideas` con otro id, y sus filas quedan en él

**Tests:** `a_new_project_keeps_the_id_and_name_of_its_line`, `an_id_taken_here_gives_the_new_project_another`, `a_folder_of_another_machine_records_the_fingerprints_of_its_line` y `a_checkout_without_origin_registered_after_an_import_joins_its_project` (`import.rs`); `an_import_records_its_folders_as_a_registration_would` sigue pasando, con la sugerencia de PL58.

## PL58. Las sugerencias de un import

Cambia PL6 y PL7: un import con escritura MUST dejar, con las reglas de PL6 (una por par, sin mover nada, nunca otra vez para un par que ya tiene una):

- las que dejaría un registro con `assigned` de cada carpeta que graba y existe acá: `root_commit` (PL5), `remote_changed` (PL6) y `folder_name` (PL38);
- `similar_name`, entre el proyecto adonde fue un proyecto del archivo con nombre y cada proyecto vivo de acá cuya clave de nombre (`name_key`) es la de ese nombre, que no lo contradice y que no fue elegido. Entran también un nombre exacto que tienen dos proyectos de acá, y uno que el paso 4 de PL56 no eligió porque sólo un lado tiene huellas;
- `merged_at_source`: la fuente tiene unidos dos proyectos que acá están separados. Pasa entre el proyecto adonde fue una lápida del archivo (paso 3) y el proyecto adonde fue su `merged_into`, cuando son dos proyectos vivos distintos; y entre los proyectos vivos de acá a los que llevan los remotos de un mismo proyecto del archivo.

Cada una que el import inserta MUST anotarse (PL63). Una sugerencia que ya existía no se anota. Una prueba en seco MUST reportarlas sin escribirlas. Se muestran como las demás (PL36), y `recall_context`, `clawg projects list` y los candidatos de un borrado MUST decir sus motivos en los dos idiomas.

#### Escenario: un nombre parecido
- GIVEN acá el proyecto `mi-app`
- WHEN se importa un proyecto del archivo sin carpeta llamado `Mi App`
- THEN se crea `Mi App`, y queda una sugerencia `similar_name` entre los dos

#### Escenario: una unión en la fuente
- GIVEN acá X y Z separados, con filas, y un export de otro store en el que X se unió en Z
- WHEN se importa
- THEN queda una sugerencia `merged_at_source` entre X y Z, y ninguno de los dos cambia

**Tests:** `an_import_leaves_the_suggestions_a_registration_would`, `a_project_named_like_one_here_is_suggested`, `a_merge_at_the_source_is_suggested_here` y `a_dry_run_reports_the_suggestions_and_writes_none` (`import.rs`); `an_import_records_its_folders_as_a_registration_would`, que ahora espera la sugerencia `remote_changed` de su segunda raíz (`import.rs`); `the_reasons_of_an_import_are_worded` (`crates/clawg-cli/src/projects.rs` y `crates/clawg-cli/src/mcp/tools.rs`, en los dos idiomas).

## PL59. Un proyecto borrado

- `deleted_at` no viaja (PL54).
- Una fila que el import inserta viva bajo un proyecto borrado lo hace volver (PL22). El import MUST anotarlo como vuelto (PL63), con el `deleted_at` que tenía, y el reporte MUST nombrarlo.
- Una fila que el import inserta borrada MUST NOT hacerlo volver. Hoy lo hace: el import inserta viva una fila borrada y después la marca, por el índice de texto (`crates/clawg-core/src/memory/portability/import.rs:675-681`), y el disparador de inserción la ve viva (`V29__project_delete.sql:63-64`). En la misma transacción, el import MUST dejar el proyecto con el `deleted_at` y el `reviewed` que tenía.
- Por nombre, un proyecto borrado nunca se elige: su nombre quedó libre (PL27). Por remoto, por id y por camino, sí (D27).

#### Escenario: lo borrado llega borrado
- GIVEN acá el proyecto A borrado con sus memorias, y un export que trae una memoria borrada de A
- WHEN se importa
- THEN la memoria queda borrada bajo A, y A sigue borrado y fuera de la lista
- AND con una memoria viva de A en el export, A vuelve sin revisar, el reporte lo dice y el `--undo` del import lo deja borrado otra vez

**Tests:** `a_row_imported_deleted_leaves_its_deleted_project_deleted` y `a_live_row_imported_into_a_deleted_project_brings_it_back_and_says_so` (`import.rs`); `a_deleted_project_exports_without_its_mark` (`export.rs`).

## PL60. Las declaraciones viajan

Después de decidir adónde va cada proyecto del archivo, por cada declaración (`subpath`, W) de un proyecto del archivo P, con P en el proyecto P' de acá y W en W':

- si P' y W' son distintos y `project_subfolder` no tiene (P', `subpath`), el import MUST escribir (P', `subpath`, W') y anotarla (PL63). Si W no fue a ningún proyecto, se crea, como manda PL57;
- una declaración de acá con esa clave MUST quedar como está, aunque lleve a otro proyecto, y el reporte la nombra;
- si P no fue a ningún proyecto, la declaración no se escribe y el reporte la nombra.

Escribir una declaración MUST NOT unir proyectos ni mover filas (D56).

#### Escenario: un monorepo en otra máquina
- GIVEN un export de un monorepo M con el remoto `github.com/acme/mono` y la declaración de `apps/web` hacia W, y acá un clone de ese remoto registrado sin declaraciones
- WHEN se importa, y después se registra el clone con el camino crudo `apps/web/src`
- THEN W existe acá, la declaración lleva al proyecto del clone y a W, y el registro responde W

**Tests:** `the_subfolders_of_a_repository_travel_with_it` y `a_declaration_here_is_kept_over_the_file` (`import.rs`).

## PL61. `--project-map NOMBRE=project:REF` y los caminos crudos

- **El CLI.** `--project-map NOMBRE=project:REF` MUST separarse en el último `=project:`, y si no lo hay, `NOMBRE=CAMINO` en el último `=`, como hoy (`crates/clawg-cli/src/cli.rs:339-346`).
- **Una referencia.** REF es un id o un nombre, y el daemon MUST resolverla como el destino de un movimiento (PL35): primero como id, sin seguir la lápida, que se sigue en la transacción que escribe; después como el nombre de un solo proyecto vivo. No crea nada ni busca parecidos. Antes de leer el archivo, MUST responder 404 si no es ninguno, 409 `project_ambiguous` si es de dos o más, y 409 `project_deleted` si es un proyecto borrado (PL27), con el `detail` de siempre. La transacción que escribe MUST comprobar el borrado otra vez: las filas de un proyecto que se borró mientras tanto se reportan inválidas, sin hacerlo volver.
- **Un camino.** Para `--project-map NOMBRE=CAMINO` y `--map-root OLD=NEW`, el CLI MUST mandar la raíz y el camino crudo, absoluto y canónico. El daemon MUST resolver la declaración más profunda como un registro (PL45): un CAMINO dentro de una subcarpeta declarada lleva a su subproyecto.
- **El cuerpo.** Cada par de `project_map` y de `map_roots` MUST aceptar como destino un texto (una raíz, como hoy), `{"root", "path"}` o, sólo en `project_map`, `{"project"}`. El CLI MUST mandar siempre un objeto, así un daemon anterior rechaza el cuerpo, porque espera un texto, en lugar de leer un camino crudo como una raíz.
- La tabla de PL19 MUST nombrar el proyecto de `POST /import` en `body.project_map[].project`, y los recorridos le mandan una lápida (las notas quedan en el proyecto vivo) y un borrado (409 `project_deleted`).

#### Escenario: un proyecto sin carpeta como destino
- GIVEN un export de engram con notas del proyecto `notas`, y acá el proyecto `ideas`, sin carpeta
- WHEN se importa con `--project-map notas=project:ideas`
- THEN las notas quedan en `ideas`, y ninguna carpeta se registra
- AND con `ideas` borrado, la respuesta es 409 `project_deleted` y no se escribe nada

**Tests:** `a_project_map_to_a_project_places_its_notes_there` y `a_project_map_path_in_a_declared_subfolder_goes_to_its_project` (`import.rs`); `a_project_map_to_a_deleted_ambiguous_or_unknown_project_is_refused`, `a_map_given_as_a_root_is_read_as_before` y el brazo de `/import` en `every_route_says_which_project_it_takes`, `every_route_that_takes_a_project_follows_a_merge` y `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule` (`tests/api.rs`); `import_takes_a_project_map_to_a_project` (`cli.rs`); `the_maps_are_sent_with_their_raw_paths` (`crates/clawg-cli/src/import.rs`, contra un daemon de prueba).

## PL62. Importar de nuevo después de un movimiento no duplica

Una fila de un export de clawg que no está presente en su proyecto (R5 de `specs/portability`) MUST contarse como presente si acá hay, en cualquier proyecto y borrada o no, una fila con su agente, su hash enmascarado, su `created_at` y su `created_by_ref`. No se inserta, sus vínculos van a esa fila como los de una presente, y el reporte la cuenta en `present_elsewhere` por el proyecto donde está.

MUST NOT valer para las filas de otra herramienta: a esas las encuentra su origen (HI11 de `specs/portability`). La búsqueda MUST ir por `idx_memory_created_at`, con su fila en la tabla de IL1 de `specs/portability`, y el planificador MUST decidir lo mismo que la pasada SQL (ID2).

#### Escenario: exportar, mover e importar otra vez
- GIVEN un store con su export, y después una memoria movida a otro proyecto con `projects move`
- WHEN se importa el export
- THEN no hay filas nuevas, y el reporte cuenta esa memoria como presente en el proyecto adonde se movió

#### Escenario: dos notas iguales de otra herramienta
- GIVEN un export de clawg con dos filas importadas de engram, en dos proyectos, con el mismo texto, el mismo agente y la misma fecha, y distinto origen
- WHEN se importa en un store vacío
- THEN quedan las dos, cada una en su proyecto

**Tests:** `an_export_imported_again_after_a_move_adds_nothing` y `two_rows_alike_from_two_origins_stay_two` (`import.rs`); la fila de `PRESENT_ELSEWHERE_SQL` en `the_import_lookups_seek_an_index` y los casos nuevos de `the_planner_decides_as_the_sql_pass` (`import.rs`).

## PL63. El diario y el `--undo`

En la misma transacción que cada escritura, el import MUST anotar en `memory_import_registry`:

| `kind` | `a` | `b` | `value` |
|---|---|---|---|
| `project` | el proyecto que creó | — | su nombre |
| `revived` | el proyecto borrado que hizo volver | — | el `deleted_at` que tenía |
| `suggestion` | `project_id` de la sugerencia que insertó | `other_id` | su motivo |
| `subfolder` | el repositorio de la declaración que escribió | su subcamino | el proyecto al que la hizo llevar |

`clawg import --undo N` MUST, después de retirar las filas, las carpetas y los remotos como hoy (IU2), y en la misma transacción:

1. borrar cada declaración anotada que sigue llevando al proyecto que escribió N;
2. borrar cada sugerencia anotada que sigue sin respuesta;
3. dejar otra vez borrado, con su `deleted_at`, cada proyecto anotado como vuelto que sigue vivo, sin revisar y sin filas vivas en las tres tablas, cuyo borrado con ese `deleted_at` sigue abierto;
4. borrar cada proyecto anotado como creado al que ya nada nombra: ninguna fila de las tres tablas en ningún estado, ninguna carpeta, remoto ni declaración, ninguna lápida que lleve a él, ninguna sugerencia, ningún diario de unión ni de borrado y ninguna fila de `synthesis_review`; que no es una lápida y sigue sin revisar.

Lo que queda MUST nombrarse en el reporte con su motivo: `in_use` (lo nombra algo), `answered` (una sugerencia respondida), `reviewed` (un proyecto renombrado o revisado) o `changed` (una declaración que ya lleva a otro). Lo que se retiró sale del diario, y lo que quedó se vuelve a pesar en el próximo `--undo`, así un segundo `--undo` no cambia nada. Una prueba en seco MUST reportar lo mismo sin escribir. Un import anterior a V31 no tiene estas anotaciones: lo que agregó al registro queda, como antes.

#### Escenario: importar y deshacer
- GIVEN un store vacío y un export con un proyecto sin carpeta, un monorepo con su declaración, un nombre parecido a uno de acá y una lápida de la fuente
- WHEN se importa y se deshace
- THEN `project`, `project_path`, `project_remote`, `project_suggestion` y `project_subfolder` quedan como antes de importar

#### Escenario: lo que se usó después
- GIVEN el mismo import, y después un save en uno de los proyectos que creó, la respuesta `apart` a su sugerencia y el renombre de otro de sus proyectos
- WHEN se deshace el import
- THEN quedan el proyecto con el save (`in_use`), la sugerencia respondida (`answered`) y el renombrado (`reviewed`), y el reporte los nombra

**Tests:** `undoing_an_import_takes_back_what_it_added_to_the_registry`, `what_an_import_added_to_the_registry_stays_once_used` y `undoing_an_import_deletes_again_the_project_it_brought_back` (`crates/clawg-core/src/memory/portability/undo.rs`); `an_undo_twice_changes_nothing_the_second_time` y `a_dry_run_undo_reports_without_writing` siguen pasando.

## PL64. Lo que dicen `clawg export` y `clawg import`

- El reporte de un import (`ImportReport`) MUST sumar, con `#[serde(default)]`:
  - `projects`: cada proyecto del archivo, con su id, su nombre, el proyecto adonde fue, el paso de PL56 que lo decidió (`map`, `remote`, `id`, `name`, `path` o `new`), y si el import lo creó o lo hizo volver;
  - `suggestions`, que dejó o dejaría;
  - `subfolders`: las declaraciones escritas y las que no, con su motivo;
  - `present_elsewhere`, por proyecto de acá (PL62).
- El reporte de un `--undo` (`UndoReport`) MUST sumar `registry`: los proyectos borrados, los que volvieron a borrarse, las sugerencias y las declaraciones retiradas, y lo que quedó con su motivo (PL63).
- `clawg import` y `clawg import --undo` MUST imprimirlos en el idioma de la preferencia, con una sentencia por `match` sobre `Lang`. Con proyectos nuevos o sugerencias, la salida MUST terminar con `clawg projects list`, que los muestra.
- `clawg export` MUST contar en su salida los proyectos del archivo y decir cuántos van sin carpeta.

**Tests:** `an_import_report_names_where_each_project_went` y `render_undo_names_what_it_took_back_from_the_registry`, en los dos idiomas (`crates/clawg-cli/src/import.rs`); `an_export_counts_its_projects` (`crates/clawg-cli/src/export.rs`).

## PL65. Lo que no cambia en la entrega 4

- La versión del formato, el encabezado y las líneas `memory`, `derivation`, `piece` y `conflict`. `--format markdown`.
- Lo que hace el import con cada fila: el enmascarado, el texto vivo y la clave de su proyecto (R5 de `specs/portability`), sin detector, y las reglas de las notas de otra herramienta (HI11, HI12).
- `memory_import_extra`, su `CHECK` y lo que retira su `--undo` (IU2 a IU4).
- El import nunca une, separa ni renombra proyectos, ni mueve filas que ya estaban acá.
- El contrato del modelo: ninguna herramienta, argumento ni descripción cambia.
- `doctor`: ningún chequeo nuevo. Los proyectos que crea un import salen en `project_duplicates` por su nombre, como cualquier otro.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, sin volver a bendecir, y los de `specs/portability` y PL1 a PL52.
