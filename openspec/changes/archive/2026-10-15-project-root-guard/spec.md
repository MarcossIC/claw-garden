# project-root-guard — Especificación

Cierra qué raíces lee el daemon por pedido del modelo y vuelve ruidosa una colisión de `project_id`. Al archivar, R1–R4 van a [`specs/project-docs`](../../specs/project-docs/spec.md) y R5–R7 a [`specs/project`](../../specs/project/spec.md), donde R5 reemplaza el último párrafo de R6 y R4 de ese archivo gana la marca.

## Definiciones

- **Raíz del puente:** la raíz de `default_project`, que el puente resolvió de su cwd al arrancar (`crates/clawg-cli/src/mcp/server.rs:325-327`).
- **Raíz aceptada:** una fila de `project` con `index_docs = 1`.
- **Raíz sólo registrada:** una fila de `project` con `index_docs = 0`: sus memorias se re-keyean y se buscan como cualquier otra, pero el daemon no lee sus archivos.

## R1. Sólo se indexa una raíz aceptada

El refresco del índice de docs de un proyecto (`DocIndex::try_refresh`, `crates/clawg-core/src/memory/project_docs/mod.rs:215`) MUST NOT leer el disco de una raíz sólo registrada: MUST fallar con un error propio, distinto de `UnknownProject`, antes de recorrerla.

`POST /v1/projects` MUST lanzar el refresco en segundo plano sólo cuando, después del registro, la raíz está aceptada.

#### Escenario: registrada sin indexar
- GIVEN una raíz con un `README.md`
- WHEN se registra con `index_docs: false`
- THEN `project` tiene su fila con `index_docs = 0` y, una vez cargado el embedder, `project_doc_file` sigue vacía

#### Escenario: el refresco se niega
- GIVEN una raíz sólo registrada
- WHEN se llama a `try_refresh` sobre ella
- THEN falla con el error de raíz no aceptada y no hay filas en `project_doc_file`

**Tests:** `a_root_registered_without_its_docs_is_not_indexed` en `crates/clawg-cli/tests/api.rs`; `a_root_that_was_not_accepted_is_not_read` en `crates/clawg-core/src/memory/project_docs/mod.rs`.

## R2. La marca sólo sube

`register_project` MUST recibir si la raíz se indexa y MUST:

- guardar ese valor al crear la fila;
- pasar `index_docs` de 0 a 1 cuando la fila existe y el registro pide indexar;
- MUST NOT bajarlo de 1 a 0: un save en otra sesión no le quita a una raíz aceptada su índice;
- devolver en el resultado el valor que quedó guardado;
- en una colisión (otra `root_path` con el mismo id), MUST NOT cambiar la marca de la otra raíz.

`POST /v1/projects` MUST tomar `index_docs` del cuerpo, falso si falta: un puente que no lo manda (uno anterior a este cambio) registra sin indexar, y lo arregla `clawg projects add` o un puente nuevo en esa raíz. La respuesta MUST NOT cambiar de forma.

#### Escenario
- GIVEN una raíz registrada con `index_docs: false`
- WHEN se registra otra vez con `true`, y después con `false`
- THEN queda en 1 después de la segunda llamada y sigue en 1 después de la tercera

**Tests:** `registering_to_index_raises_the_mark_and_never_lowers_it` y `a_collision_leaves_the_other_roots_mark_alone` en `crates/clawg-core/src/memory/project.rs`.

## R3. Buscar en una raíz no aceptada se niega con el remedio

`POST /v1/projects/docs/search` sobre una raíz sólo registrada MUST responder 403, sin refrescar ni buscar, con un `error` en inglés que nombre la raíz y el comando que la acepta, con el `--profile` del daemon cuando no es `prod`: ``clawg projects add "<raíz>" --profile dev``. Un `project_id` sin fila sigue siendo 404.

`search_project` MUST devolver ese texto como error de la herramienta, sin el prefijo `daemon returned 403:`.

La descripción de `project_path` en `SearchProjectArgs` MUST decir que sólo se buscan los docs de la raíz del puente o de una que el usuario agregó con `clawg projects add`.

#### Escenario: ruta ajena
- GIVEN un puente en `<tmp>/repo` y otra carpeta `<tmp>/other` con un `README.md`
- WHEN `search_project` recibe `project_path: <tmp>/other`
- THEN es un error que contiene `clawg projects add` y `<tmp>/other`, y `project_doc_file` no tiene filas de `<tmp>/other`

#### Escenario: la raíz del puente
- GIVEN un puente en `<tmp>/repo` con un `README.md`
- WHEN `search_project` recibe la consulta sin `project_path`
- THEN responde con el pasaje del `README.md`, como hoy

**Tests:** `searching_the_docs_of_a_root_that_was_not_accepted_is_forbidden` en `crates/clawg-cli/tests/api.rs`; el puro `the_refusal_names_the_command_for_the_daemons_profile` en `crates/clawg-cli/src/daemon/api.rs`; `search_project_does_not_read_a_root_the_user_did_not_add` y `search_project_lists_each_hit_under_its_breadcrumb` (el que ya existe) en `crates/clawg-cli/src/mcp/server.rs`.

## R4. `clawg projects add`

`clawg projects add <RUTA> [--profile P]` MUST resolver la raíz de RUTA con `ProjectIdentity::resolve` y registrarla con `index_docs: true` y el id viejo de RUTA si difiere, a través del daemon que está corriendo. MUST NOT abrir la base ni arrancar un daemon: sin daemon, falla como `clawg undelete`. Si la raíz no es un directorio, MUST fallar sin llamar al daemon. Al terminar imprime la raíz y su `project_id`, y que sus docs se indexan en segundo plano.

El daemon MUST responder 400 a un registro con `index_docs: true` cuya raíz no es un directorio, sin crear la fila.

#### Escenario
- GIVEN un daemon corriendo y una raíz sólo registrada `<tmp>/other`
- WHEN el usuario corre `clawg projects add <tmp>/other/docs`
- THEN `<tmp>/other` queda aceptada, y un `search_project` posterior con esa ruta indexa y responde

**Tests:** parseo en `crates/clawg-cli/src/cli.rs` (`projects_add_takes_a_path_and_a_profile`); `adding_a_project_lets_its_docs_be_searched` y `adding_a_path_that_is_not_a_directory_is_refused` en `crates/clawg-cli/src/projects.rs`; `registering_a_missing_root_to_index_it_is_a_bad_request` en `crates/clawg-cli/tests/api.rs`.

## R5. El puente pide indexar sólo su raíz

`project_for` MUST mandar `index_docs: true` cuando la raíz de la llamada es la raíz del puente, y `false` en cualquier otro caso. La decisión MUST ser una función pura de la raíz y la raíz del puente, sin filesystem.

#### Escenario
- GIVEN un puente en `<tmp>/repo`
- WHEN `save_memory` recibe `project_path: <tmp>/other`
- THEN la memoria se guarda bajo el id de `<tmp>/other`, `<tmp>/other` queda sólo registrada y nada de ella se indexa

**Tests:** `only_the_bridges_own_root_is_indexed` (puro) en `crates/clawg-cli/src/mcp/tools.rs`; `a_save_elsewhere_registers_the_root_without_reading_it` en `crates/clawg-cli/src/mcp/server.rs`.

## R6. Una colisión es un error en cada llamada

Si `POST /v1/projects` responde 409 (`error: "project_root_collision"`, con las dos raíces en `detail`), la herramienta MUST fallar sin guardar ni buscar, con un texto que diga que la raíz no se puede usar porque su `project_id` ya pertenece a otra raíz, nombre las dos, y diga que `all_projects` sigue buscando en todo. MUST NOT recordar la raíz como registrada, así la llamada siguiente vuelve a preguntar y vuelve a fallar mientras el choque siga.

Vale para las cinco herramientas con proyecto: `save_memory`, `search_memory`, `recall_context`, `search_project` y `synthesize`. Con `all_projects: true` no hay proyecto y no hay registro, así que no fallan.

#### Escenario
- GIVEN `project` con el id de la raíz del puente y otra `root_path`
- WHEN el puente guarda una memoria dos veces
- THEN las dos llamadas son error y nombran la otra raíz, y no hay ninguna fila bajo ese id

**Tests:** `a_colliding_root_refuses_the_save_every_time` (reemplaza a `a_refused_registration_does_not_block_the_save`) y `a_colliding_root_still_searches_all_projects` en `crates/clawg-cli/src/mcp/server.rs`.

## R7. Los demás fallos de registro no bloquean

Un registro que falla por otra causa (un daemon que no conoce la ruta, un 400, un error de transporte) MUST dejar el warn de hoy y la herramienta MUST seguir con el id de la raíz.

**Tests:** `a_registration_the_daemon_cannot_do_does_not_block_the_save` (nuevo, contra un daemon que responde 404 a `/v1/projects`) y `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` (el que ya existe) en `crates/clawg-cli/src/mcp/server.rs`.

## R8. La base de la versión anterior

Una base migrada hasta `V12` MUST abrir con `V13` sin cambiar ninguna fila de memorias, proyectos ni índice de docs, y cada fila de `project` MUST quedar con `index_docs = 1`.

#### Escenario
- GIVEN una base en `V12` con dos memorias, la raíz `/work/repo` en `project` y un pasaje indexado
- WHEN se abre con la versión nueva
- THEN las filas son idénticas y `/work/repo` está aceptada

**Test:** `a_database_from_the_previous_version_keeps_its_projects_indexed` en `crates/clawg-core/src/memory/db.rs`.
