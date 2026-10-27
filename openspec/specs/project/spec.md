# project — Especificación

Fuente de verdad del comportamiento actual de la identidad de proyecto. Cubre cómo una ruta se resuelve a la raíz de su proyecto y cómo se deriva el `project_id`. También cómo las filas guardadas bajo un id viejo pasan al de la raíz sin perderse, qué marca el registro sobre si el daemon puede leer los archivos de la raíz (`index_docs`), y qué hace el puente cuando el id de una raíz ya pertenece a otra. Un proyecto es una fila de `project`, la referencia de cada tabla, y una carpeta (`project_path`) es una forma de llegar a él: cómo se reconoce un checkout por sus huellas, cómo una mudanza conserva su proyecto, qué sugerencias quedan y cómo se listan y se nombran los proyectos (PL1 a PL10). También cómo se une un proyecto en otro y se deshace la unión, la lápida que deja y que cada entrada sigue, cómo se responde una sugerencia y qué duplicados nombra `doctor` (PL11 a PL20). También cómo se borra un proyecto, vacío o con sus memorias bajo un código, cómo se deshace el borrado, cómo un proyecto borrado vuelve al usarse, qué no toma, cómo se purga y qué dice `doctor` (PL21 a PL31). También cómo se nombra un proyecto por su id o su nombre, cómo se crea uno sin carpeta, cómo el agente ve y responde las sugerencias, cómo se ubica una carpeta en un proyecto y qué sugiere una carpeta movida sin VCS (PL32 a PL42). También cómo una subcarpeta de un repositorio se declara como un proyecto propio, el camino crudo que mandan los clientes y cómo decide la declaración más profunda (PL43 a PL52). Qué raíces se indexan y cómo se buscan sus docs está en [`specs/project-docs`](../project-docs/spec.md).

**Origen:**
- [project-identity](../../changes/archive/2026-10-12-project-identity/proposal.md), archivado el 2026-10-12.
- [project-root-guard](../../changes/archive/2026-10-15-project-root-guard/proposal.md), archivado el 2026-10-15 (la marca `index_docs` en R4 y R6, y la colisión como error en R6).
- [untrusted-recall](../../changes/archive/2026-10-20-untrusted-recall/proposal.md), archivado el 2026-10-20 (R6: la raíz del puente pide `index_docs` sólo si es un proyecto).
- [project-reassign](../../changes/archive/2026-10-22-project-reassign/proposal.md), archivado el 2026-10-22 (R8 a R13: mover filas de `memory` a otro proyecto o a ninguno, con su unidad, su registro y su vuelta atrás).
- [project-clone-identity](../../changes/archive/2026-10-23-project-clone-identity/proposal.md), archivado el 2026-10-23 (R14 a R19: el daemon asigna los ids, los clones de un remoto comparten el proyecto, y unir reconcilia; cambia R4 y R6).
- [history-import](../../changes/archive/2026-10-24-history-import/proposal.md), archivado el 2026-10-24 (HI3: elegir y listar filas por su proyecto de origen).
- [project-registry](../../changes/archive/2026-10-25-project-registry/proposal.md), archivado el 2026-10-25 con su entrega 0, que sale en 0.63.0 (R20: una fila de un camino sirve a una raíz sólo si su remoto no la contradice; cambia R15, R16 y R18). Sus entregas 1 a 4 (el registro de proyectos, unir y borrar, los proyectos sin carpeta, el export por localizador) pasaron a [project-locators](../../changes/archive/2026-10-27-project-locators/proposal.md).
- [ram-footprint](../../changes/archive/2026-10-24-ram-footprint/proposal.md), archivado el 2026-10-24 (RF1: un registro lanza el refresco de docs sólo si trae `index_docs: true`; cambia la nota de R4).
- [project-locators](../../changes/archive/2026-10-27-project-locators/proposal.md), archivado el 2026-10-27, su entrega 1, que sale en 0.65.0 (PL1 a PL10: `project` es la referencia de cada proyecto y las carpetas pasan a `project_path`, el primer commit es la segunda huella de una carpeta, una mudanza conserva el proyecto, lo dudoso queda como sugerencia, y `clawg projects list` y `rename`; cambia R4, R10, R15, R16, R18 y R20), y su entrega 2a, en la misma versión (PL11 a PL20: `clawg projects merge` con su lápida y su `--undo`, `canonical` en cada entrada que recibe un proyecto, `clawg projects keep-apart` y los duplicados de `doctor`; cambia PL9), y su entrega 2b, en la misma versión (PL21 a PL31: `clawg projects delete` en sus dos etapas con su `--undo`, el proyecto borrado que vuelve al usarse, `clawg purge --project-delete` y el chequeo `project_deletes`; cambia PL9 y PL15), y su entrega 3a, en la misma versión (PL32 a PL42: el argumento `project` y `answer_suggestion` del contrato, `POST /v1/projects/resolve`, `suggestions/answer` y `locate`, los nombres en `--to-project` y `PATCH`, `clawg projects create` y `locate`, las sugerencias en el contexto y la sugerencia `folder_name`; cambia R10 y PL19), y su entrega 3b, en la misma versión (PL43 a PL52: las declaraciones de V30, el camino crudo en el registro, `clawg projects locate --subfolder` y el chequeo `project_subfolders`; cambia R1, R3, R4 y R6), y su entrega 4, en la misma versión (PL53 a PL65: el diario del registro de V31, una línea `project` por proyecto con sus localizadores, adónde va cada proyecto del archivo, las sugerencias, las declaraciones y lo borrado en un import, `--project-map NOMBRE=project:REF` y lo que retira el `--undo`; cambia PL6, PL7, PL47 y PL52).

## Definiciones

- **Ruta cruda:** la que llega tal cual: el cwd del puente, `project_path` de una tool, o el cwd de `clawg memories --here`.
- **Raíz del proyecto:** el resultado de `resolve_project_root(ruta cruda)` (R1).
- **Id viejo:** `derive_project_id(ruta cruda)`, el que se usaba antes de resolver rutas (`crates/clawg-core/src/memory/identity.rs:11-15`).
- **Id nuevo:** `derive_project_id(raíz del proyecto)`. Mismo formato, `{basename}-{hash4}`.
- **Tablas con proyecto:** `memory`, `episodic_trajectory` y `procedural_skill` (`crates/clawg-core/migrations/V1__init.sql`).
- **Clave de una raíz:** el `origin` normalizado de su directorio git (R14), o ninguna.
- **Proyecto:** una fila de `project`, la tabla de referencia. Tiene un id que no cambia, un nombre, cómo nació (`born`) y si el usuario ya lo revisó (`reviewed`). Las memorias, y cualquier tabla nueva, guardan este id. Un proyecto puede tener cero o más carpetas y cero o más remotos: sigue existiendo después de que se borró su carpeta.
- **Carpeta:** una fila de `project_path` (la tabla `project` de antes de V27, renombrada): un camino de esta máquina, su id (el id de raíz), `index_docs`, el proyecto al que lleva y las huellas de lo que se encontró ahí. Es una forma de llegar a un proyecto, no el proyecto: se puede mover o desaparecer.
- **Carpetas de un camino:** las de `project_path` con ese `root_path`, de la más vieja a la más nueva.
- **Id de raíz:** el id de la carpeta con que `project_path` guarda una raíz (R15). Lo usan el índice de docs y `index_docs`.
- **Id de proyecto:** el id del proyecto bajo el que se guardan y buscan las memorias de una raíz (R16).
- **Checkout:** lo que hay hoy en una raíz: su clave, su **directorio git** (el que tiene el `config` de R14, `git_config_dir`) y su primer commit. Una raíz sin `.git` no tiene directorio git ni primer commit.
- **Primer commit:** el commit sin padres en el que termina la cadena de primeros padres de `HEAD`.
- **Marca del directorio git (`git_stamp`):** la fecha de creación del directorio git y, en Unix, también su dispositivo y su inodo. Cambia cuando el directorio git es otro (un clone nuevo, un `git init` nuevo, una copia), no con un `mv` dentro del mismo sistema de archivos.
- **Huellas de una carpeta:** su remoto (`project_path.remote`; una carpeta de antes de V27 sin él usa los que `project_remote` ata a su id) y su primer commit.
- **Proyecto de una carpeta:** el de `project_path.project_id`. Si es NULL, la carpeta no se registró desde V27, y su proyecto es el que da R16: el de su clave en `project_remote`, o si no su id.
- **Checkout grabado de una carpeta:** su camino con la marca que guardó. **Ya no está** cuando el camino no existe o, si guardó una marca, la de lo que hay hoy en él es otra.
- **Lápida:** un proyecto con `merged_into`, el que dejó una unión (PL13). No recibe nada: cada id que lo nombra lleva al proyecto en el que se unió. Queda porque los puentes abiertos guardan sus ids toda la sesión y los exports viejos los llevan.
- **Proyecto vivo:** uno sin `merged_into`.
- **`canonical(id)`:** el proyecto vivo al que lleva una lápida; para un proyecto vivo, o un id que `project` no conoce, el mismo id (`crates/clawg-core/src/memory/merge.rs`).
- **Parte del usuario de un `workspace.md`:** lo que la regeneración de la sección AUTO conserva tal cual (`split_existing`, `crates/clawg-core/src/memory/workspace.rs`): lo de antes de `<!-- AUTO-START -->` y la sección USER con lo que la sigue.
- **Filas de un proyecto:** las de `memory`, `episodic_trajectory` y `procedural_skill` cuyo `project_id` es el proyecto.
- **Fila viva** (de un borrado): una con `deleted_at` NULL, cualquiera sea su `status`. Una fila `superseded` está viva.
- **Proyecto vacío:** uno sin filas vivas. Puede tener filas borradas.
- **Proyecto borrado:** uno con `project.deleted_at`. No se lista y su nombre queda libre, pero su id sigue siendo suyo.
- **Borrado N:** una fila de `project_delete`. El de la etapa 1 borra un proyecto vacío (`with_memories` falso). El de la etapa 2 borra un proyecto con sus filas.
- **Conjunto de un borrado:** el proyecto, los ids de raíz de sus carpetas (`project_path` con su `project_id`), sus remotos (`project_remote`) y cada fila del proyecto, viva o borrada, con su id, `status`, `deleted_at` y `update_count` (`Set`, `crates/clawg-core/src/memory/project_delete.rs`).
- **Código:** los primeros 8 caracteres hexadecimales, en minúscula, del SHA-256 de una forma canónica del conjunto (`code_of`): una línea por dato, ordenadas (`project <id>`, `folder <raíz>`, `remote <remoto>` y, por tabla y por id, `<tabla> <id> <status> <deleted_at o -> <update_count>`). No entran el texto de las filas ni `last_modified_at`.
- **Borrado abierto:** uno sin `undone_at` ni `purged_at`.
- **Volver al usarse:** que un proyecto borrado deje de estarlo porque recibió una fila viva (PL22).
- **Candidatos de una raíz:** `candidate_id(raíz, n)` para `n` de 0 a 63: el id nuevo para 0, y para el resto el mismo formato con el hash de los bytes de la ruta seguidos de `#n` (`crates/clawg-core/src/memory/identity.rs`).

## R1. Resolución de la raíz

`resolve_project_root` MUST devolver la misma raíz para todas las formas de nombrar un mismo directorio de un repo:

- canonicaliza la ruta (enlaces, `.`, `..`, relativa al cwd, caja real del disco en Windows) y le quita el prefijo `\\?\`;
- sube desde ahí hasta el primer directorio que contenga `.git`;
- si `.git` es un directorio, la raíz es ese directorio;
- si `.git` es un archivo `gitdir: <dir>` y `<dir>` tiene un archivo `commondir`, la raíz es el padre del directorio común (un worktree comparte la raíz del repo principal);
- si `.git` es un archivo sin `commondir` en su `gitdir` (submódulo), la raíz es el directorio que contiene ese `.git`;
- si ningún ancestro tiene `.git`, la raíz es la ruta canonicalizada;
- si la ruta no existe, la raíz es la ruta normalizada léxicamente (sin separador final, sin `.`, con `..` resueltos), sin buscar `.git`.

La decisión (qué marcador, qué raíz) MUST ser una función pura que no toca el filesystem. La lectura de `.git` y `commondir` va aparte.

Una subcarpeta declarada como proyecto (PL43 a PL46) no es una raíz: R1 da la raíz de su repositorio, y la declaración sólo cambia el proyecto que responde el registro.

#### Escenario: misma raíz por varios caminos
- GIVEN un repo en `<tmp>/repo` con `.git/` y un subdirectorio `src/lib`
- WHEN se resuelven `<tmp>/repo`, `<tmp>/repo/`, `<tmp>/repo/src/lib` y `<tmp>/repo/src/..`
- THEN las cuatro dan `<tmp>/repo`

#### Escenario: worktree
- GIVEN `<tmp>/wt/.git` con `gitdir: <tmp>/repo/.git/worktrees/wt`, y ese directorio con `commondir` = `../..`
- WHEN se resuelve `<tmp>/wt`
- THEN la raíz es `<tmp>/repo`

#### Escenario: submódulo
- GIVEN `<tmp>/repo/sub/.git` con `gitdir: ../.git/modules/sub`, sin `commondir`
- WHEN se resuelve `<tmp>/repo/sub`
- THEN la raíz es `<tmp>/repo/sub`

#### Escenario: sin git
- GIVEN `<tmp>/plain` sin `.git` en ningún ancestro
- WHEN se resuelve `<tmp>/plain/`
- THEN la raíz es `<tmp>/plain`

#### Escenario: Windows
- GIVEN `C:\Users\u\Repo` existente con `.git\`
- WHEN se resuelven `c:\users\u\repo` y `C:/Users/u/Repo/`
- THEN las dos dan `C:\Users\u\Repo`, sin `\\?\`

**Tests:** unit de la decisión pura en `identity.rs` (marcadores falsos, sin filesystem); integración con directorios temporales en `crates/clawg-core/tests/project_root.rs`. El de Windows corre sólo en Windows (`#[cfg(windows)]`).

## R2. Id estable en el caso común

El id nuevo de una ruta que ya es la raíz canónica MUST ser igual a `derive_project_id` de esa ruta hoy. Así, el puente que arranca en la raíz del repo no re-keyea nada. Es también el id de raíz que el registro le da mientras ninguna otra raíz lo tenga (R15).

#### Escenario
- GIVEN `<tmp>/repo` canónica, con `.git/`
- THEN `ProjectIdentity::resolve(<tmp>/repo).id == derive_project_id(<tmp>/repo)`, y su `legacy_id` es el mismo

**Tests:** `the_id_of_a_canonical_root_is_the_id_it_always_had` y `a_subdirectory_keeps_its_old_id_as_the_legacy_one` en `crates/clawg-core/tests/project_root.rs`.

## R3. Todos los caminos usan la misma resolución

El puente MCP (cwd y `project_path`) y `clawg memories --here` MUST derivar el id con `ProjectIdentity::resolve` (`crates/clawg-core/src/memory/identity.rs`). Si falla `current_dir()`, el puente MUST seguir como hoy: sin proyecto por defecto (`crates/clawg-cli/src/mcp/server.rs:470-472`).

#### Escenario
- GIVEN un puente con cwd en `<tmp>/repo/src`
- WHEN guarda una memoria sin `project_path`
- THEN la fila queda con el id de `<tmp>/repo`

`ProjectIdentity` guarda también el camino canónico desde el que subió a la raíz (`path`), que los clientes mandan como camino crudo (PL47).

**Tests:** `resolve_project` en `crates/clawg-cli/src/mcp/tools.rs` (la resolución entra como función) y `Scope::project_id` en `crates/clawg-cli/src/cli.rs` (entra el id ya resuelto), para seguir siendo puros. `a_bridge_in_a_subdirectory_brings_what_was_saved_there_to_the_root` en `crates/clawg-cli/src/mcp/server.rs`.

## R4. Registro de proyecto y re-key

`POST /v1/projects`, con token, recibe `{ "root_path": "...", "legacy_ids": ["..."], "index_docs": <bool>, "assigned": <bool>, "path": "..." }` (`path`, el camino crudo, es opcional y sólo cuenta con `assigned`: PL44) y responde `{ "project_id": "...", "root_id": "...", "rekeyed": <n> }` (`register`, `crates/clawg-cli/src/daemon/api.rs`; `register_project`, `crates/clawg-core/src/memory/project.rs`).

- El daemon MUST decidir los ids él mismo, de `root_path` y de las huellas que lee de su checkout (PL3); no los recibe. Con `assigned` los asigna (R15, R16); sin él (un cliente anterior) el id de raíz y de proyecto es `derive_project_id(root_path)`, sin clave (R18).
- `index_docs` dice si el daemon puede leer los archivos de la raíz ([`specs/project-docs`](../project-docs/spec.md), «Qué raíces se leen»). Si falta MUST tomarse como `false` (`RegisterRequest::index_docs`, `api.rs:1920`): un puente de una versión anterior registra sin indexar, y lo arregla `clawg projects add` o un puente actual en esa raíz cuando es un proyecto.
- Con `index_docs: true` y una `root_path` que no es un directorio, MUST responder 400 antes de tocar `project_path` (`register`, `api.rs`). Sin `index_docs`, la misma ruta se sigue registrando como siempre.
- MUST guardar en `project_path` la carpeta de la raíz (su id de raíz, `root_path`, `index_docs`, su proyecto y sus huellas, PL4) cuando ninguna carpeta del camino le sirve, y crear en `project` su proyecto cuando no existe.
- Si la fila existe con la misma raíz y el registro pide indexar, MUST pasar `index_docs` de 0 a 1. MUST NOT bajarlo de 1 a 0: un save en otra sesión no le quita a una raíz aceptada su índice (`project.rs:98-100`). El resultado (`RegisterOutcome::index_docs`) lleva el valor que quedó guardado.
- Sin `assigned`, si `derive_project_id(root_path)` ya está registrado con **otra** `root_path`, o la raíz está guardada con otro id, MUST responder 409 sin mover ninguna fila y sin tocar la marca de la otra raíz. El cuerpo es `{ "error": "project_root_collision", "detail": "<raíz> gets the project id <id>, which already belongs to <otra raíz>" }`. Con `assigned` MUST NOT responder 409 por un choque.
- Por cada id viejo distinto del de proyecto, y por el id de raíz cuando difiere de él, MUST unir al id de proyecto todas las filas que lo tengan, incluidas las borradas lógicamente, en **una** transacción: las de `memory` con R17, registradas en `memory_move`; las de `episodic_trajectory` y `procedural_skill` con una fila por fila movida en `project_rekey` (la tabla, el id de la fila, el id viejo, el nuevo y la fecha). Un id viejo que es el id de raíz de otra raíz MUST quedar como está.
- Un id viejo con formato inválido (fuera de `{basename}-[0-9a-f]{4}`) MUST dar 400 sin mover nada.
- La operación MUST ser idempotente: repetirla no mueve filas ni agrega entradas a `project_rekey`.
- Un registro que tiene éxito (nuevo o ya conocido), trae `index_docs: true` y deja la raíz aceptada también lanza el refresco del índice de documentación del proyecto, en segundo plano y sin que la respuesta lo espere: [`specs/project-docs`](../project-docs/spec.md), «Un registro que lo pide indexa la raíz en segundo plano». No lo lanzan uno sin `index_docs`, aunque la raíz ya estuviera aceptada (los hooks registran así en cada evento), uno que deja la raíz sólo registrada ni uno que falla (400, 409).

#### Escenario: la marca sólo sube
- GIVEN una raíz registrada con `index_docs: false`
- WHEN se registra otra vez con `true`, y después con `false`
- THEN la marca queda en 1 después de la segunda llamada y sigue en 1 después de la tercera

#### Escenario: colisión con un cliente anterior
- GIVEN `project_path` con el id de `/work/repo` registrado con la raíz `/elsewhere/repo` y `index_docs = 0`
- WHEN un pedido sin `assigned` registra `/work/repo` pidiendo indexar
- THEN el registro falla con `RootCollision` (por HTTP, 409 `project_root_collision` con las dos raíces en `detail`), y la marca de `/elsewhere/repo` sigue en 0

#### Escenario: filas viejas visibles bajo el id nuevo
- GIVEN una base con 2 memorias, 1 trayectoria y 1 skill en el id viejo `src-1a2b`, y 1 memoria en otro proyecto
- WHEN se registra la raíz con `legacy_ids = ["src-1a2b"]`
- THEN las 4 filas tienen el id nuevo, la del otro proyecto no cambió, `memory_move` tiene las 2 memorias, `project_rekey` las otras 2 filas y la respuesta dice `rekeyed: 4`

#### Escenario: idempotencia
- WHEN se repite el mismo registro
- THEN `rekeyed: 0`, no hay snapshot nuevo y los dos registros siguen como estaban

#### Escenario: rollback
- GIVEN el estado del primer escenario
- WHEN se deshace el movimiento con `undo_move_core` y se aplica a las otras dos tablas el SQL de `V9__project.sql`
- THEN cada fila vuelve a su id viejo

#### Escenario: búsqueda léxica después del re-key
- GIVEN una memoria re-keyeada con un término único
- WHEN se busca ese término en el proyecto nuevo
- THEN aparece por el canal léxico (FTS lee `project_id` de `memory`)

**Tests:** `rows_of_a_legacy_id_move_to_the_resolved_root`, `registering_again_moves_nothing`, `the_logs_restore_every_row_a_registration_moved`, `a_moved_row_is_found_lexically_under_its_new_project`, `a_caller_that_derives_its_id_keeps_the_conflict`, `a_malformed_legacy_id_is_refused_before_anything_moves`, `registering_to_index_raises_the_mark_and_never_lowers_it` y `a_legacy_id_of_another_registered_project_keeps_its_rows_and_workspace` en `crates/clawg-core/src/memory/project.rs`; HTTP en `crates/clawg-cli/tests/api.rs` (`registering_a_root_moves_the_rows_of_its_legacy_id`, `registering_a_root_whose_id_is_taken_by_another_is_a_conflict`, `registering_without_assigned_ids_keeps_the_old_answer`, `registering_a_missing_root_to_index_it_is_a_bad_request`, y los de 400 y 401).

## R5. Datos de la versión anterior

Una base creada por la versión anterior, con filas en ids viejos, MUST abrir con la migración nueva sin cambiar ninguna fila, y las filas MUST seguir alcanzables por su id hasta que un registro las re-keyee.

#### Escenario
- GIVEN una base migrada hasta la última migración publicada, con filas en tres proyectos
- WHEN se abre con la versión nueva
- THEN las filas y sus `project_id` son idénticos, y existen `project` y `project_rekey` vacías

**Test:** `a_database_from_the_previous_version_opens_with_its_project_ids_untouched` en `crates/clawg-core/src/memory/db.rs` (siembra con el runner hasta V8).

La columna `index_docs` (V13) llega con 1 en cada fila de `project` que ya existía: [`specs/project-docs`](../project-docs/spec.md), «Datos de la versión anterior». Desde V27 esa tabla se llama `project_path`, y `project` es la de los proyectos (PL1).

## R6. El puente registra cada raíz una vez

Antes de la primera tool que use una raíz, el puente MUST llamar a `POST /v1/projects` con esa raíz, el id viejo de la ruta cruda (si difiere), `index_docs` y `assigned` (`ClawgMcp::ids_for`, `crates/clawg-cli/src/mcp/server.rs`), y usar los ids que responde: el de proyecto para las memorias, el de raíz para `search_project`. Un daemon anterior, que no responde `root_id`, da uno solo para las dos. Con la raíz manda el camino crudo en `path` (PL47), y la clave de lo que ya registró es la raíz y ese camino: una subcarpeta declarada de una raíz lleva a otro proyecto. MUST NOT volver a llamarlo para la misma raíz y camino en el mismo proceso, salvo en dos casos: una colisión (abajo) y una falla que se resuelve releyendo ([`specs/mcp`](../mcp/spec.md), «Registro del proyecto»).

`index_docs` MUST ser `true` sólo cuando la raíz de la llamada es la raíz del puente, la que resolvió de su cwd al arrancar (`server.rs:470-472`), y esa raíz es un proyecto que se indexa solo: tiene un `.git` o un manifiesto, no es el directorio del usuario ni una raíz del sistema de archivos ([`specs/project-docs`](../project-docs/spec.md), «El puente acepta su raíz sola sólo si es un proyecto»). En cualquier otro caso MUST ser `false`. Una raíz que el modelo nombra en `project_path`, o la del puente cuando no es un proyecto, queda así sólo registrada hasta que el usuario la agregue con `clawg projects add`. La decisión es una función pura de las dos raíces, sin filesystem (`indexes_docs`, `crates/clawg-cli/src/mcp/tools.rs:876`): el puente le pasa su raíz sólo cuando `auto_indexable` la aceptó al construirse (`own_root_to_index`, `server.rs:451`), y si no, ninguna. Compara `root` y no el id, porque dos raíces que chocan comparten id.

**Colisión.** Sólo un daemon anterior a los ids asignados responde 409 `project_root_collision`. Si lo hace, la tool MUST fallar sin guardar ni buscar. El texto MUST nombrar las dos raíces y decir que clawg no mezcla las memorias de dos proyectos bajo un id. También MUST decir que el proyecto no se puede usar hasta renombrar una de las dos carpetas, y que `all_projects` sigue buscando en todos (`ClawgMcp::explain`, `server.rs:633-650`). El puente MUST NOT recordar la raíz como registrada (`server.rs:874-876`): la llamada siguiente vuelve a preguntar y vuelve a fallar mientras el choque siga. Así, un `clawg restore` que lo resuelve se nota sin reiniciar el puente. Vale para las cinco tools con proyecto: `save_memory`, `search_memory`, `recall_context`, `search_project` y `synthesize`. Con `all_projects: true` (en `search_memory` y `synthesize`) no hay proyecto ni registro, así que no fallan.

**Otras fallas.** Si el registro falla por otra causa (un daemon que no conoce la ruta, un 400, un error de transporte), la tool MUST seguir con `derive_project_id(raíz)` para los dos ids y dejar un warn: un registro fallido no bloquea un save, y el próximo registro que funciona une lo que se guardó así.

#### Escenario: un save en otra raíz
- GIVEN un puente en `<tmp>/repo/src`
- WHEN `save_memory` recibe `project_path: <tmp>/other`
- THEN la memoria se guarda bajo el id de `<tmp>/other`, que queda con `index_docs = 0`, y nada de `<tmp>/other` se indexa

#### Escenario: colisión con un daemon anterior
- GIVEN un daemon que responde 409 `project_root_collision` al registro de la raíz del puente
- WHEN el puente guarda una memoria dos veces
- THEN las dos llamadas son error, nombran las dos raíces y `all_projects`, y no hay ninguna fila bajo ese id
- AND `search_memory` con `all_projects: true` responde sin error

#### Escenario: un daemon que no sabe registrar
- GIVEN un daemon que responde 404 a `/v1/projects`
- WHEN el puente guarda una memoria
- THEN el save tiene éxito bajo el id de la raíz

**Tests:** `a_root_is_registered_once_per_bridge`, `a_save_elsewhere_registers_the_root_without_reading_it`, `a_colliding_root_refuses_the_save_every_time` (contra un daemon que responde 409), `a_colliding_root_gets_a_project_of_its_own`, `a_colliding_root_still_searches_all_projects`, `a_registration_the_daemon_cannot_do_does_not_block_the_save`, `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` y `a_bridge_in_a_folder_that_is_not_a_project_indexes_nothing` en `crates/clawg-cli/src/mcp/server.rs`, contra un daemon en proceso; `only_the_bridges_own_root_is_indexed` (puro) en `crates/clawg-cli/src/mcp/tools.rs`; `auto_indexable_wants_a_project_that_is_not_home` (puro) en `crates/clawg-core/src/memory/project_docs/corpus.rs`.

## R7. `workspace.md` sigue al proyecto

En un re-key, por cada agente con `<workspaces>/<agente>/<id viejo>/workspace.md`:

- si no existe `<workspaces>/<agente>/<id nuevo>/workspace.md`, el daemon MUST copiarlo ahí (escritura atómica, como `write_atomically`);
- si existe, MUST dejar los dos como están y registrar un warn con las dos rutas;
- MUST NOT borrar ni modificar el archivo viejo.

#### Escenario
- GIVEN `agent-a/src-1a2b/workspace.md` con una sección USER escrita a mano
- WHEN se registra la raíz con ese id viejo
- THEN `agent-a/<id nuevo>/workspace.md` tiene la misma sección USER, y el viejo sigue igual

**Tests:** `a_rekey_copies_the_legacy_workspace_and_keeps_it`, `adopting_never_overwrites_the_projects_own_workspace` y `adopting_without_any_workspace_is_a_no_op` en `crates/clawg-core/src/memory/workspace.rs`; `registering_carries_the_legacy_workspace_over` en `project.rs`.

## R8. Mover cambia el proyecto de una unidad entera y nada más

Un **movimiento** cambia el `project_id` de filas de `memory` a un **destino**: un id de proyecto o «sin proyecto». La **unidad** de una fila es su cierre: las filas de todo save de `memory_piece` que la contiene, las que reemplazó con causa `revision` y la que la reemplazó con esa causa, las que su borrado revivió (`revived_from`) y la que revivió por el suyo, repetido hasta que no entra ninguna más, borradas y `superseded` incluidas (`read_units_in`, `crates/clawg-core/src/memory/relocate.rs`).

Un movimiento MUST cambiar, en una transacción, `project_id` y, en una fila sin agente, `scope` (`project` con destino, `global` sin él; `scope_after`) de cada fila de cada unidad aceptada, y MUST NOT tocar otra columna de `memory` (ni `last_modified_at`, ni `update_count`, ni `status`), `memory_vectors`, `memory_fts`, `memory_conflict`, `memory_derivation` ni `memory_piece`. No MUST necesitar el embedder ni correr el detector. Después, la búsqueda, RECENT, LAST SESSION, la dedup, la clave de tema, el revivir y la revisión de conclusiones MUST tratar las filas como guardadas en el destino. Una fila sin proyecto se sigue viendo desde todos los proyectos.

#### Escenario: sin proyecto a uno
- GIVEN una fila sin proyecto que se encuentra desde A y desde B
- WHEN se mueve a B
- THEN se encuentra desde B, por vector y por texto, y no desde A

#### Escenario: la cadena se mueve con su vigente
- GIVEN una clave con su vigente y la versión que reemplazó, en A
- WHEN se mueve la versión vieja a B y después se borra la vigente
- THEN las dos se movieron y la versión anterior vuelve viva en B

**Test:** `a_row_without_a_project_moves_into_one`, `moving_to_no_project_shows_the_row_from_every_project`, `a_row_without_an_agent_keeps_the_scope_check`, `the_pieces_of_a_save_move_together`, `two_saves_that_share_a_row_move_together`, `a_revision_chain_moves_with_its_head_and_deleting_the_head_still_revives`, `after_a_move_a_save_of_the_same_text_reinforces_the_moved_row`, `after_a_move_a_save_under_its_key_revises_it`, `a_session_summary_moved_into_a_project_is_its_last_session`, `moving_a_superseded_source_does_not_make_its_conclusions_reappear_for_review`, `a_move_leaves_vectors_and_the_text_index_alone` y `scope_after_a_move_follows_the_check` (`relocate.rs`).

## R9. Lo que rechaza una unidad

Una unidad MUST quedarse donde está, sin escribir ninguna de sus filas, cuando una fila viva del destino al alcance del lector tiene el `content_hash` (`live_twin`) o la `topic_key` (`topic_taken`) de una fila viva de la unidad, cuando una unidad aceptada antes en el mismo movimiento lleva ese hash o esa clave a ese destino (se mueve la primera, por id), cuando sus filas están en más de un proyecto (`spans_projects`) o cuando tiene una fila fuera del alcance del lector (`out_of_reach`, sin nombrarla). El motivo MUST nombrar la fila de la unidad y la que sostiene el texto o la clave. Una unidad que ya está en el destino queda `unchanged` y no se registra. Una fila borrada MUST moverse con su unidad sin chocar; un `restore` posterior valida en el proyecto nuevo. Los pares abiertos que quedan entre dos proyectos MUST nombrarse (`split_pairs`) y quedar como están. La decisión es pura (`decide`).

**Test:** `decide_*` (puros), `a_live_row_of_the_same_text_in_the_target_refuses_the_move`, `a_live_holder_of_the_topic_key_in_the_target_refuses_the_move`, `two_selected_saves_that_would_collide_in_the_target_move_only_the_first`, `a_unit_across_projects_is_refused`, `a_hidden_row_in_the_unit_refuses_it_without_naming_it`, `a_clash_is_judged_within_the_readers_reach`, `a_deleted_row_moves_with_its_save_and_is_restored_into_the_new_project` y `a_pair_left_across_projects_is_reported` (`relocate.rs`).

## R10. El destino está registrado

El daemon MUST rechazar con 404, sin escribir, un destino que no está en `project`; un proyecto sin carpeta es un destino (PL8), y «sin proyecto» siempre lo es. El puente (`to_project_path`) y `clawg projects move --to PATH` MUST registrar antes la raíz de la ruta, sin `index_docs`, y MUST rechazar una raíz que no es un directorio.

**Test:** `an_unregistered_target_is_refused` y `a_project_without_a_folder_is_a_target` (`relocate.rs`), `a_move_to_an_unregistered_project_is_404` (`tests/api.rs`), `update_memory_refuses_both_destinations_and_a_path_that_is_not_a_directory` (`server.rs`).

## R11. El registro y la vuelta atrás

Cada fila que un movimiento cambia, por cualquier puerta, MUST dejar en `memory_move` (V21) el número del movimiento (`MAX + 1`), la fila, el proyecto y el `scope` de origen y de destino y el momento, en la misma transacción. Deshacer el movimiento N (`undo_move_core`) MUST mover de vuelta a su origen, con R8 y R9 y sin exigir que el origen esté registrado, las filas de N que siguen en el destino que N anotó, como un movimiento nuevo; una fila que se movió otra vez después queda donde está. Un N que no existe es un 404.

**Test:** `every_moved_row_is_logged_and_the_log_undoes_it` y `undoing_leaves_a_row_that_moved_again` (`relocate.rs`).

## R12. El lote

`POST /v1/memories/move` MUST tomar `dry_run` y una selección (al menos uno de `ids`, `from_project`/`from_none`, `agent`, `memory_type`, `source`, `session_id` o `q`; sin `include_deleted`, filas no borradas; `q` con `include_deleted` es un 400) con un destino (`to_project` o `to_none`), o `undo`. Es del usuario: sin lector y sin `user_answered`; cuenta en `protected` las preferencias y decisiones vivas que movió. Con algo que mover y sin `dry_run`, MUST tomar antes el snapshot `pre-move`. Un ensayo MUST responder lo mismo que el movimiento, salvo `move_id` y `snapshot`, sin escribir nada. `clawg projects move` manda ese pedido al daemon que corre, sin arrancar uno, e imprime el reporte; su análisis de argumentos es puro (`MoveOpts::request`).

**Test:** `a_dry_run_reports_what_a_move_does_and_writes_nothing`, `a_selection_needs_a_filter_and_text_only_over_live_rows` y `the_filters_narrow_the_selection` (`relocate.rs`); `a_move_asks_for_one_target_or_an_undo` (`api.rs`); `the_batch_moves_reports_and_snapshots` y `a_batch_without_a_selection_or_a_target_is_400` (`tests/api.rs`); `move_takes_a_selection_and_one_target_or_an_undo` (`cli.rs`); `a_move_report_names_what_moved_what_stayed_and_the_way_back` y `projects_move_moves_a_memory_and_undoes_it` (`projects.rs`); `projects_move_moves_and_undoes_a_seeded_store` (`tests/binary.rs`).

## R13. Listar lo que no tiene proyecto

`GET /v1/memories?no_project=true` MUST listar sólo las filas sin proyecto; con `project` es un 400. `clawg memories list` y `search` toman `--no-project`.

**Test:** `a_listing_of_rows_without_a_project` (`admin.rs`), `no_project_and_project_together_are_refused` (`api.rs`), `the_listing_reaches_the_rows_without_a_project` (`tests/api.rs`), `list_and_search_take_no_project` (`cli.rs`).

## HI3. Elegir filas por su proyecto de origen

`clawg projects move` MUST aceptar `--origin-project NOMBRE`, que elige las filas cuyo `origin_project` es exactamente `NOMBRE`, y cuenta como filtro de selección (R12). `POST /v1/memories/move` MUST aceptar `origin_project` con el mismo efecto. `clawg memories list --origin-project NOMBRE` y `GET /v1/memories?origin_project=NOMBRE` MUST listar sólo esas filas, con los demás filtros.

#### Escenario: acomodar un proyecto de la fuente
- GIVEN notas importadas sin proyecto de dos proyectos de origen, `a` y `b`
- WHEN se corre `clawg projects move --origin-project a --to RUTA`
- THEN se mueven las de `a` con sus unidades y las de `b` quedan sin proyecto

**Tests:** `a_batch_move_selects_by_origin_project` (`crates/clawg-core/src/memory/relocate.rs`); `a_listing_filters_by_origin_project` (`crates/clawg-core/src/memory/admin.rs`); `move_and_list_take_an_origin_project` (`crates/clawg-cli/src/cli.rs`); `move_and_list_select_by_origin_project` (`crates/clawg-cli/tests/api.rs`).

## R14. La clave de una raíz

`origin_key(raíz)` (`crates/clawg-core/src/memory/identity.rs`) MUST leer el `config` del directorio git de la raíz: `<raíz>/.git/config` si `.git` es un directorio; si es un archivo `gitdir:`, el del directorio común cuando ese `gitdir` tiene `commondir`, y si no el del propio `gitdir` (`git_config_dir`). MUST tomar el primer `url` de `[remote "origin"]` (nombre de sección y de clave sin mayúsculas, subsección exacta, comentarios `#` y `;`, valor entre comillas; `origin_url`) y normalizarlo con una función pura (`normalize_remote`): sin esquema (`https`, `http`, `ssh`, `git`, `git+ssh`, `ssh+git`), usuario, contraseña, puerto, barra final ni `.git`; `usuario@host:ruta` como `host/ruta`; todo en minúsculas. MUST dar ninguna clave para `file://`, una ruta (absoluta, relativa, `~`, `C:\`), un valor sin host o sin ruta, otro esquema y un archivo que no se lee. La clave MUST NOT llevar usuario, contraseña ni token, y no se guarda la URL. No sigue `include` ni `insteadOf`, y no mira otro remoto que `origin`.

#### Escenario: las formas de un mismo remoto
- GIVEN `https://github.com/Acme/App.git`, `git@github.com:acme/app.git`, `ssh://git@github.com:22/Acme/App` y `https://u:tok@github.com/Acme/App/`
- THEN las cuatro dan `github.com/acme/app`, sin `tok`

#### Escenario: worktree y submódulo
- THEN la clave de un worktree sale del `config` de su directorio común y la de un submódulo del de su `gitdir`

**Tests:** `a_remote_normalizes_to_host_owner_and_repo`, `a_local_remote_has_no_key`, `the_origin_url_is_read_from_its_section`, `the_config_of_a_worktree_is_in_its_common_directory` (`identity.rs`); `two_clones_of_one_remote_have_one_key`, `the_key_of_a_worktree_and_of_a_submodule`, `a_root_without_origin_has_no_key` (`crates/clawg-core/tests/project_root.rs`).

## R15. El registro asigna el id de raíz

Con `assigned`, una raíz MUST conservar el id de la carpeta de su camino que sus huellas eligen (PL4), y una sin carpeta que le sirva MUST recibir el primero de sus candidatos que no es el id de otra carpeta (`settle_in`, `project.rs`). Ninguna raíz MUST quedar con el id de otra y ningún registro MUST fallar porque dos hashes choquen; con los 64 candidatos tomados, MUST fallar sin escribir (409 `no_free_project_id`).

**Tests:** `the_root_of_always_keeps_its_id`, `a_root_whose_id_is_taken_gets_the_next_free_one`, `a_root_keeps_the_id_it_was_given`, `every_id_taken_is_an_error_that_writes_nothing`, `the_candidates_of_a_root_keep_its_name_and_start_with_its_id` (`project.rs`, `identity.rs`); `registering_two_roots_that_collide_gives_each_its_own_id` (`tests/api.rs`); `a_colliding_root_gets_a_project_of_its_own` (`server.rs`).

## R16. Los clones de un remoto comparten el proyecto

Con `assigned`, el id de proyecto de una raíz MUST ser el primero que exista de estos: el que `project_remote` (V24) da a su clave; el del checkout que se mudó a ella (PL5); el que su carpeta guarda en `project_path.project_id`; o un proyecto nuevo con el id de su carpeta (PL4). Una clave que `project_remote` no tiene MUST atarse al proyecto que recibe la raíz. Así la primera raíz que trae un remoto conserva su id y sus filas, un primer remoto de una raíz ya registrada no le cambia el proyecto, un remoto distinto del que ya tenía atado le da otro (R20), y una raíz cuyo remoto ya es de otro proyecto une a él sus filas (R4, R17). Dos remotos son dos proyectos (un fork lo es, aunque comparta el primer commit). Los docs siguen siendo de cada carpeta (`specs/project-docs`).

Los clientes MUST usar el id que responde el daemon: el puente (R6), los hooks ([`specs/hooks`](../hooks/spec.md) R6), `clawg projects add`, `projects move --to` y `--here`. `clawg memories --here` y `clawg export --project` MUST leerlo del store sin escribir (`project_id_of`, `crates/clawg-core/src/memory/portability/export.rs`), y el import lo planea igual y graba sus carpetas como un registro (PL7; [`specs/portability`](../portability/spec.md) R4).

#### Escenario: el segundo clone
- GIVEN dos raíces con el mismo `origin`, cada una con una fila bajo su `derive_project_id`, la primera registrada
- WHEN se registra la segunda
- THEN las dos filas están bajo el id de la primera, la segunda conserva su id de raíz, y `memory_move` lista la fila movida

**Tests:** `a_second_clone_joins_the_project_of_the_first`, `a_clone_joins_with_one_live_row_per_text`, `a_first_remote_keeps_the_project_of_its_root`, `two_remotes_are_two_projects`, `a_caller_that_derives_its_id_joins_no_remote`, `a_clone_hands_its_workspace_to_the_shared_project`, `the_project_of_a_root_is_read_without_writing` (`project.rs`); `two_clones_of_one_remote_register_into_one_project` (`tests/api.rs`); `two_clones_of_one_remote_share_their_memories` y `search_project_reads_the_docs_of_its_own_root` (`server.rs`); `two_clones_share_their_memories_through_a_real_daemon` (`tests/binary.rs`: un hook en el segundo clone y `memories list --here` ven lo que se guardó en el primero).

## R17. Unir reconcilia, se registra y se deshace

Unir ids (R4) MUST mover las filas de `memory` como un movimiento de R8, registrado en `memory_move`, sin vectores, índice de texto ni detector, con cada unidad entera y sin rechazar por `live_twin` ni `topic_taken` (`merge_in`, `decide_merge`, `crates/clawg-core/src/memory/relocate.rs`). Una unidad con filas en un proyecto que no es el origen ni el destino MUST quedar donde está, con un warn. Después, en la misma transacción, sobre las filas vivas del destino de los grupos que toca una fila movida (`reconcile`, pura):

1. de cada `topic_key` con filas vivas de más de un save, MUST quedar vivo el save de la fila más nueva (`created_at`, luego id), y cada otra fila viva con esa clave fuera de él MUST pasar a `superseded` por la primera fila de ese save, con causa `revision`, salvo un pedazo de un save cuya primera fila está viva sin esa clave;
2. de cada `content_hash` con más de una fila viva, MUST quedar la más vieja, y cada otra fuera de su save MUST pasar a `superseded` por ella con causa `revision`, sólo si es un save de una fila o si todas las filas vivas de su save se reemplazan así.

Ninguna otra columna MUST cambiar. Cada reemplazo MUST quedar en `memory_move_supersede` con el movimiento, la fila, la que la reemplaza y su `status`, `superseded_by` y `supersede_cause` de antes. El resultado MUST NOT depender de qué clone se registró primero. Deshacer el movimiento (`undo_move_core`) MUST devolver primero cada fila reemplazada que sigue `superseded` por la que anotó a su estado de antes, y después mover de vuelta. Un registro que va a mover filas MUST tomar antes un snapshot `pre-rekey` y decidir otra vez en la transacción que escribe; uno que no mueve nada MUST NOT tomarlo. El daemon MUST dejar en el log cuántas filas unió, el número del movimiento y el snapshot.

**Tests:** `reconcile_keeps_the_newest_save_of_a_key_whichever_side_moved`, `reconcile_keeps_the_oldest_row_of_a_text`, `reconcile_replaces_a_piece_only_with_its_whole_save`, `reconcile_leaves_a_piece_another_save_holds_and_one_key_per_save`, `decide_merge_moves_what_is_not_in_the_target_and_stops_at_other_projects` (puros), `merging_keeps_the_newest_version_of_a_key`, `merging_keeps_the_oldest_row_of_a_text`, `merging_does_not_depend_on_which_clone_came_first`, `merging_a_long_save_twice_keeps_one_whole`, `undoing_a_merge_restores_every_row_and_state` (`relocate.rs`); `a_registration_that_moves_rows_snapshots_first` (`project.rs`).

## R18. Un cliente anterior conserva la regla de antes

Un pedido sin `assigned` MUST recibir `derive_project_id(raíz)` como los dos ids, MUST NOT unirse por remoto ni seguir una mudanza (PL7), y MUST recibir 409 `project_root_collision` cuando ese id es de otra raíz, cuando la carpeta que sus huellas eligen (PL4) tiene otro id o cuando su camino está contradicho, también por el primer commit, para que un puente abierto antes de una actualización nunca escriba bajo el id de otro proyecto. Graba las huellas sólo en su propia carpeta, y su proyecto sólo cuando la carpeta es nueva. Mientras ese puente siga abierto, lo que un cliente nuevo unió de su raíz a otro clone no lo ve; sigue en el store y el próximo registro nuevo une lo que ese puente guardó.

**Tests:** `a_caller_that_derives_its_id_keeps_the_conflict`, `a_caller_that_derives_its_id_joins_no_remote`, `a_caller_that_derives_its_id_is_refused_another_repository_s_project`, `a_caller_that_derives_its_id_is_refused_a_repository_without_origin_in_its_path` y `a_caller_that_derives_its_id_never_follows_a_move` (`project.rs`); `registering_without_assigned_ids_keeps_the_old_answer` y `a_repository_cloned_where_another_was_registers_into_a_project_of_its_own` (`tests/api.rs`).

## R19. Datos de la versión anterior a los remotos

Una base en V23 con filas en dos proyectos MUST abrir con V24 sin cambiar ninguna fila de `memory` ni de `project` (desde V27, `project_path`), con `project_remote` y `memory_move_supersede` vacías ([`specs/storage`](../storage/spec.md), «Los remotos y lo que reemplaza una unión»).

**Test:** `a_database_from_the_previous_version_opens_with_no_remote_recorded` (`crates/clawg-core/src/memory/db.rs`).

## R20. Una carpeta de un camino sirve a una raíz sólo si su remoto no la contradice

Un camino no es una identidad: otro repositorio puede clonarse donde estuvo uno. Un camino puede tener varias carpetas en `project_path`, una por repositorio; los remotos de una carpeta son el suyo o, para una de antes de V27, las claves que `project_remote` da a su id. El registro MUST elegir la carpeta con una regla pura, y `project_id_in` y `plan_ids_in` MUST usar la misma sin escribir (en un lote, como si las raíces anteriores ya estuvieran escritas). Esa regla es la de PL4, que compara también el primer commit. Por el remoto:

1. sin clave actual, el remoto no decide: una clave que no se puede leer no contradice nada, y decide el primer commit (PL4);
2. con clave, la primera carpeta con esa clave; si no hay, la más vieja sin remotos que el primer commit no contradice (un primer remoto, que se graba en ella);
3. si todas las carpetas están contradichas, el camino lo está: la raíz MUST recibir el primero de sus candidatos que ninguna carpeta tiene, en una carpeta nueva, y su clave MUST atarse a su proyecto cuando `project_remote` no la tenga.

Los ids viejos del cliente salen del camino, así que son de su primer repositorio: el registro MUST unir sus filas sólo con un camino nuevo o con la carpeta más vieja, y MUST NOT tocar un id viejo que es el de cualquier otra carpeta. El daemon MUST leer la clave también para un pedido sin `assigned`, sin atarla (R18).

Una carpeta sin remotos acepta un primer remoto: si el repositorio anterior del camino no tenía `origin`, el que llega con uno se queda con su carpeta, salvo que el primer commit lo contradiga. Un remoto renombrado en el mismo camino abre otro proyecto, y cuando el primer commit es el mismo queda una sugerencia `remote_changed` entre los dos (PL6); `clawg projects move --from-project VIEJO --to-project NUEVO` los une y se deshace. `clawg doctor` nombra los proyectos que ya tienen dos remotos ([`specs/doctor`](../doctor/spec.md) R7).

#### Escenario: otro repositorio en el camino de uno registrado
- GIVEN `/work/app` registrado con `github.com/acme/a`, con una fila en su proyecto
- WHEN se registra `/work/app` con `github.com/acme/b`
- THEN recibe `candidate_id(/work/app, 1)`, nada se mueve, `github.com/acme/b` queda atado a ese id, y un clone de `github.com/acme/b` en otro camino se une a él
- AND registrar otra vez cada uno da los ids de su primer registro, con dos carpetas en `project_path`

**Tests:** `a_path_settles_on_the_row_its_remote_does_not_contradict` (pura), `a_repository_in_the_path_of_another_gets_a_project_of_its_own`, `a_repository_in_the_path_of_another_takes_none_of_its_rows`, `each_repository_in_one_path_returns_to_its_own_project`, `the_project_of_a_repository_in_another_s_path_is_read_alike_without_writing`, `a_first_remote_keeps_the_project_of_its_root`, `a_different_remote_in_a_registered_path_is_another_project` (`project.rs`); `a_repository_cloned_where_another_was_registers_into_a_project_of_its_own` (`tests/api.rs`).

## PL1. V27 hace de `project` la referencia y pasa las carpetas a `project_path`

V27 MUST renombrar `project` a `project_path` sin cambiar ningún valor ni índice, agregarle `project_id`, `remote`, `root_commit` y `git_stamp` (NULL por defecto) con un índice parcial por `root_commit`, y crear `project` (`id` clave primaria, `name`, `born`, `reviewed`, `created_at`), `project_suggestion` (`project_id`, `other_id`, `reason`, `created_at`, `answer`, `answered_at`, con un índice único sobre el par sin orden) y los disparadores de PL2. El esquema, el relleno y la vuelta atrás están en [`specs/storage`](../storage/spec.md), «Los proyectos y sus carpetas».

El relleno MUST crear un proyecto por cada id distinto que tenga una fila de `memory`, `episodic_trajectory` o `procedural_skill` (borradas incluidas) o una fila de `project_remote`, con:

- `name`: el id sin su `-` y sus cuatro dígitos hex finales cuando tiene la forma de `is_project_id`; si no, el id entero;
- `born = 'migrated'` y `reviewed = 1`: lo que ya estaba no se marca «sin revisar»;
- `created_at`: el menor `created_at` de lo que lo nombra.

En cada carpeta MUST poner en `project_id` su propio id cuando es un proyecto (NULL si no: la carpeta de un clone, cuyas memorias están bajo el proyecto de su remoto, y la carpeta donde nunca se guardó nada, que se resuelven en su primer registro), y en `remote` el remoto que `project_remote` ata a su id sólo cuando le ata exactamente uno. Un id que sólo nombra un registro de historia (`memory_move`, `project_rekey`, `memory_import_extra`, `synthesis_review`) MUST NOT recibir un proyecto, y V27 MUST NOT cambiar ninguna fila de las tres tablas ni de `project_remote`.

**Tests:** `a_database_from_the_previous_version_opens_with_a_project_for_every_id_in_use` (`crates/clawg-core/src/memory/db.rs`); `published_migration_checksums_are_pinned` (`migrations.rs`).

## PL2. Cada id que se escribe tiene su proyecto

Cuando se inserta una fila de `memory` con un `project_id` que no está en `project`, o se le cambia el `project_id` a uno que no está, la misma escritura MUST crear el proyecto, con el nombre de PL1, `born = 'adopted'`, `reviewed = 0` y el `created_at` de la fila, sin rechazarla ni cambiar ninguna de sus columnas: un save, un import, un movimiento, su `--undo` o un `PATCH`. El nombre que da Rust (`project_name`, `crates/clawg-core/src/memory/registry.rs`) MUST ser el del disparador.

#### Escenario: un movimiento de vuelta a un id de la historia
- GIVEN un movimiento que llevó todas las filas de un id viejo a otro proyecto, y ese id sin proyecto
- WHEN se deshace el movimiento
- THEN las filas vuelven al id viejo, y ese id es un proyecto `adopted`

**Tests:** `a_row_saved_under_an_unknown_project_adopts_it`, `a_row_moved_to_an_unknown_project_adopts_it` y `the_name_of_an_adopted_id_is_the_one_rust_gives` (`registry.rs`).

## PL3. El primer commit se lee una vez por directorio git

Cada registro MUST leer de la raíz su clave (R14) y la marca de su directorio git. El primer commit sólo se lee cuando ninguna carpeta del camino guardó esa misma marca: un camino nuevo, una carpeta de antes de V27, o un directorio git que no es el que se grabó. Si una carpeta guardó esa marca, el primer commit es el que guardó ella, también cuando guardó que no se pudo leer. Un camino grabado que se registra con el mismo directorio git MUST NOT correr `git`, y el daemon MUST correrlo en un hilo bloqueante, nunca en el de la conexión (`Ground`, `crates/clawg-core/src/memory/identity.rs`).

La lectura (`root_commit`) MUST correr `git rev-list --max-parents=0 --first-parent HEAD` sobre el directorio git que nombra la raíz, sin las variables `GIT_*`, con la entrada cerrada y, en Windows, sin abrir una ventana; esperar como mucho `ROOT_COMMIT_WAIT` (1 s) y cortar el proceso después; y aceptar sólo una línea de 40 o 64 dígitos hex.

| Resultado | Cuándo | Se graba |
|---|---|---|
| primer commit | `git` respondió a tiempo con un hash | el commit y la marca |
| no se puede saber | el repositorio es superficial (su directorio git tiene `shallow`), o `git` no respondió a tiempo | NULL y la marca: no se vuelve a intentar hasta que cambie el directorio git |
| más tarde | no hay directorio git, `git` no se pudo correr, o terminó con error (un repositorio sin commits) | nada: el próximo registro vuelve a intentar |

**Tests:** `a_recorded_checkout_does_not_read_its_first_commit_again` (`project.rs`, con un `Ground` falso que cuenta las lecturas); `the_first_commit_is_the_root_of_the_first_parent_chain`, `a_shallow_or_empty_repository_has_no_first_commit_to_record`, `the_stamp_of_a_checkout_changes_when_it_is_cloned_again` (`crates/clawg-core/tests/project_root.rs`, con repositorios reales: necesitan `git` en el PATH); `a_command_that_does_not_end_in_time_is_stopped` (`identity.rs`).

## PL4. Una carpeta sirve a una raíz sólo si sus huellas no la contradicen

Cada carpeta del camino se compara con el checkout con una función pura (`fit`, `crates/clawg-core/src/memory/project.rs`):

1. si el remoto de la carpeta y la clave del checkout se conocen los dos, la carpeta es **la misma** cuando el checkout tiene uno de sus remotos, y **contradicha** cuando no;
2. si no, y los dos primeros commits se conocen, es la misma cuando son iguales y contradicha cuando no;
3. si no, es **desconocida**.

El registro MUST elegir la primera carpeta que es la misma; si no hay, la más vieja desconocida; si todas están contradichas, el camino lo está y la raíz recibe una carpeta nueva (R20.3, R15). En la carpeta elegida o nueva MUST grabar su remoto cuando no tiene ninguno (un primer remoto), el primer commit y la marca según PL3, y su proyecto en `project_path.project_id`, sin escribir nada cuando nada cambió: los hooks registran en cada evento. El proyecto de la raíz es el de R16; uno nuevo nace con el id de la carpeta, `born = 'root'` y `reviewed = 0`. El remoto decide antes que el primer commit: un checkout del mismo remoto con otro primer commit (una rama huérfana, una historia reescrita) es la misma carpeta.

#### Escenario: un repositorio sin `origin` en el camino de otro
- GIVEN `/work/app` registrado sin clave, con el primer commit `c1` grabado y una fila en su proyecto
- WHEN se registra `/work/app` con la clave `github.com/acme/b` y el primer commit `c2`
- THEN recibe la carpeta `candidate_id(/work/app, 1)` y un proyecto nuevo con ese id, nada se mueve, y `github.com/acme/b` se ata a ese proyecto
- AND con los dos sin clave (`c1` y después `c2`) pasa lo mismo, sin remoto atado

**Tests:** `a_checkout_settles_on_the_folder_its_fingerprints_do_not_contradict` (pura), `a_repository_without_origin_in_the_path_of_another_gets_a_project_of_its_own`, `two_repositories_without_origin_in_one_path_are_two_projects`, `a_remote_decides_before_the_first_commit`, `a_clone_folder_records_its_own_remote` (`project.rs`); `a_repository_without_origin_replaced_in_its_path_registers_into_a_project_of_its_own` (`tests/api.rs`, con repositorios reales).

## PL5. Una mudanza conserva el proyecto

Con un camino nuevo, un registro con `assigned` cuya clave no está en `project_remote` (o que no tiene clave) y cuyo primer commit se conoce MUST buscar las carpetas de otros caminos con ese primer commit que el remoto no contradice (PL4.1), y mirar en disco, en un hilo bloqueante, si el checkout grabado de cada una ya no está.

- Si las carpetas encontradas son de **un solo proyecto** y el checkout grabado de todas ya no está, es una mudanza: la raíz MUST recibir una carpeta nueva, con su id (R15) y ese proyecto en `project_path.project_id`; sus ids viejos y el id de la carpeta MUST unirse a él (R4), y el log MUST nombrar el proyecto y el camino viejo.
- Si no (el checkout grabado sigue en su lugar: una copia o un segundo checkout, o hay más de un proyecto), MUST abrir un proyecto propio y dejar una sugerencia `root_commit` con cada uno de esos proyectos (PL6).

La carpeta vieja queda como está: es una forma más de llegar al proyecto, y si el checkout vuelve a ese camino, la usa otra vez. Ante la duda queda separado: unir se deshace, y lo fundido no se puede separar. Un camino donde antes hubo otro repositorio no es un camino nuevo: queda contradicho y recibe un proyecto propio. Una mudanza le da a la raíz su propia carpeta y su propio índice de docs ([`specs/project-docs`](../project-docs/spec.md)).

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

Cuando el camino queda contradicho por el remoto (PL4), una carpeta contradicha tiene el mismo primer commit que el checkout, y el proyecto que recibe la raíz es otro que el de esa carpeta, el registro MUST dejar una sugerencia `remote_changed` entre los dos proyectos. Sin el mismo primer commit no hay sugerencia (R20).

Para las sugerencias de PL5 y PL6:

- MUST haber una por par de proyectos, sin importar el orden: una que ya existe, con respuesta o sin ella, no se vuelve a crear (`INSERT OR IGNORE` sobre el índice único del par).
- Sólo un registro con `assigned` y un import con escritura las crean (el import, las de PL58). Leer, planear, una prueba en seco y un cliente sin `assigned` MUST NOT crearlas.
- Crear una MUST NOT mover ninguna fila ni cambiar ninguno de los dos proyectos. El daemon MUST dejarla en el log con el comando que la lista.
- Todavía no se responden: quedan pendientes (`answer` NULL), `projects list` las muestra (PL9) y el agente no las ve.

#### Escenario: un remoto renombrado
- GIVEN `/work/app` registrado con `github.com/acme/app` y el primer commit `c1`
- WHEN se registra con `github.com/acme-org/app` y `c1`
- THEN recibe otro proyecto, como en R20, y queda una sugerencia `remote_changed` con el anterior

**Tests:** `a_renamed_remote_in_its_path_is_another_project_with_a_suggestion`, `a_suggestion_is_made_once_per_pair` (`project.rs`).

## PL7. Leer, planear, importar y el cliente anterior eligen igual

- `project_id_in` (`clawg memories --here`, `clawg export --project`) y `plan_ids_in` (el import) MUST elegir con PL3 a PL5 sin escribir. En un lote, una raíz MUST elegir como si las anteriores ya estuvieran escritas, con sus huellas.
- El import MUST leer lo que su plan necesita del disco en un hilo bloqueante, entre pasadas del plan sobre la conexión (`ReadAhead`, `crates/clawg-core/src/memory/identity.rs`; `plan`, `crates/clawg-core/src/memory/portability/import.rs`). Las carpetas nuevas que escribe MUST llevar lo que grabaría un registro (`project_id`, `remote`, `root_commit` y `git_stamp`), y su proyecto MUST existir en `project`, con `born = 'import'` cuando lo crea el import. Una carpeta que ya estaba no se toca. Adónde va cada proyecto del archivo, y qué graba de una carpeta de otra máquina, está en PL56 y PL57.
- Un pedido sin `assigned` sigue R18: elige la carpeta con PL4 y nunca sigue una mudanza.

**Tests:** `a_moved_checkout_is_read_and_planned_alike_without_writing`, `a_caller_that_derives_its_id_is_refused_a_repository_without_origin_in_its_path`, `a_caller_that_derives_its_id_never_follows_a_move` (`project.rs`); `an_import_records_its_folders_as_a_registration_would` y `an_import_reads_the_first_commit_off_the_store_s_thread` (`crates/clawg-core/src/memory/portability/import.rs`).

## PL8. Un proyecto existe tenga carpeta o no

El destino de un movimiento (`POST /v1/memories/move`, `PATCH` con `project_id`, `clawg projects move --to-project`) MUST aceptarse cuando está en `project`, tenga carpeta o no, y MUST seguir siendo un 404 sin escribir cuando no está (R10).

**Tests:** `a_project_without_a_folder_is_a_target` (`crates/clawg-core/src/memory/relocate.rs`); `an_unregistered_target_is_refused` y `a_move_to_an_unregistered_project_is_404`.

## PL9. `clawg projects list` y `clawg projects rename`

**`GET /v1/projects`** está detrás del token y MUST responder `{"projects": [...]}` con cada proyecto vivo (ni las lápidas, como dice PL15, ni los borrados, como dice PL27), tenga carpeta o no, por nombre sin distinguir mayúsculas y después por id (`list_projects`, `crates/clawg-core/src/memory/registry.rs`), con:

- `id`, `name`, `born`, `reviewed` y `created_at`;
- `folders`, de cero a muchas: `root_id`, `root_path`, `remote`, `index_docs` y `present`, que dice si el checkout grabado sigue ahí (sin marca grabada, si el camino existe);
- `remotes`, de `project_remote`;
- `memories`: cuántas filas de `memory` no borradas tiene;
- `suggestions`: las pendientes en las que aparece, de cualquiera de los dos lados, cada una con `other_id`, `other_name`, `reason` y `created_at`.

Una carpeta con `project_id` NULL MUST listarse en el proyecto que le da R16, leyendo su clave en disco; la presencia y esa clave se leen en un hilo bloqueante. Una carpeta cuyo proyecto no está en `project` (una de antes de V27 donde nunca se guardó nada) no se lista hasta que un registro la hace proyecto.

**`PATCH /v1/projects/{id}`** con `{"name": "..."}` MUST cambiar el nombre, sin los espacios a los lados, y marcar el proyecto revisado, y responder el proyecto. MUST NOT cambiar el id ni ninguna otra fila. Responde 404 si el proyecto no existe, con un error que nombra el id; 400 si el nombre queda vacío, pasa de 80 caracteres (`MAX_NAME_CHARS`) o tiene caracteres de control; y 409 `name_taken`, con `detail`, si el nombre es, sin distinguir mayúsculas (Unicode, no sólo ASCII), el nombre o el id de otro proyecto. Volver a poner el mismo nombre sólo lo marca revisado.

**`clawg projects list [--profile P]`** y **`clawg projects rename <ID> <NOMBRE> [--profile P]`** MUST hablar con el daemon que corre, sin arrancar uno, e imprimir en el idioma de la preferencia, con una sentencia por `match` sobre `Lang` (`crates/clawg-cli/src/projects.rs`). La lista MUST marcar lo que nació solo y no se revisó (con cómo nació), los proyectos sin carpeta, las carpetas que ya no están y las sugerencias pendientes, y nombrar `rename` cuando hay algo para revisar. Un rechazo de `rename` MUST decir qué id no se renombró, con la causa del daemon como vino. El id viaja como un solo segmento del camino.

#### Escenario: renombrar
- GIVEN un proyecto `app-1a2b` con memorias, sin revisar
- WHEN se corre `clawg projects rename app-1a2b "App web"`
- THEN `projects list` lo muestra como `App web`, revisado y con el mismo id
- AND lo que se guardó bajo él se encuentra como antes

**Tests:** `renaming_keeps_the_id_and_marks_the_project_reviewed`, `a_name_another_project_holds_is_refused`, `a_name_is_trimmed_and_bounded`, `a_folder_no_registration_recorded_is_listed_in_the_project_of_its_remote` (`registry.rs`); `the_projects_are_listed_with_their_folders_and_suggestions`, `renaming_a_project_keeps_what_was_saved_under_it`, `renaming_an_unknown_project_is_404` (`tests/api.rs`); `projects_list_and_rename_parse` (`cli.rs`); `a_listing_marks_what_was_created_alone_and_the_suggestions` (en los dos idiomas) y `projects_are_renamed_and_listed_through_the_daemon` (`crates/clawg-cli/src/projects.rs`).

## PL10. Los clientes no notan nada

- `POST /v1/projects` MUST responder con la misma forma de antes (`project_id`, `root_id`, `rekeyed`).
- El puente, los hooks, `projects add` y `projects move --to` no cambian.
- El contrato del modelo (`agent_contract.json`, `agent_contract.es.json`) no cambia, y ninguna sugerencia llega al agente.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (sin volver a bendecir), y los de R4, R6 y R14 a R20.

## PL11. V28: la lápida y el diario de las uniones

V28 MUST agregar a `project` `merged_into` y `merged_at` (NULL por defecto, con un índice parcial por `merged_into`) y crear `project_merge` (`merge_id`, `source`, `target`, `move_id`, `snapshot`, `merged_at`, `undone_at`, `undo_move_id`) y `project_merge_change` (`merge_id`, `kind`, `a`, `b`, sin `CHECK` sobre `kind`), sin cambiar ninguna fila. Un store en V27 abre con todos sus proyectos vivos, y `canonical` de cada id es el mismo id. El detalle de V28 está en [`specs/storage`](../storage/spec.md).

**Tests:** `a_database_from_the_previous_version_opens_with_every_project_live` (`crates/clawg-core/src/memory/db.rs`); la fila 28 de `published_migration_checksums_are_pinned`.

## PL12. Cada entrada que recibe un proyecto sigue la lápida

Una unión deja cada lápida apuntando a un proyecto vivo (PL13), así que `canonical` da un salto. Aun así MUST seguir `merged_into` hasta `MAX_MERGE_HOPS` (8), y un ciclo, el tope o un id que `project` no tiene MUST devolver el id del que partió, con un `warn`; `doctor` lo nombra (PL18).

`canonical` MUST correr:

- al entrar, en `POST /v1/memories` (antes de buscar la sesión y de escribir el outbox), `POST /v1/search`, `POST /v1/context` (también para el camino del `workspace.md` y los conteos de `synthesize`), `POST /v1/sessions`, `POST /v1/synthesis`, `GET /v1/disputes`, `GET /v1/memories` y `GET /v1/stats` (`live_project`, `crates/clawg-cli/src/daemon/api.rs`);
- en el worker, al empezar cada save (`follow_merge`, `crates/clawg-core/src/memory/worker.rs`), antes de leer nada del proyecto: cubre un save que estaba en el outbox, una reanudación y `clawg outbox retry`;
- en la transacción que escribe: el destino de un movimiento o de un `PATCH` (`target_in`, `relocate.rs`), el origen de cada fila en el `--undo` de un movimiento, el proyecto que recibe una raíz al registrarla, leerla o planearla (`plan_in`, `plan_ids_in`) y el de cada fila y carpeta de un import (`SqlSeat`, `record_projects`).

Lo que administra proyectos MUST NOT seguir la lápida y responde 409 `project_merged`, con `detail` que nombra el proyecto vivo: `PATCH /v1/projects/{id}`, cualquiera de los lados de `POST /v1/projects/keep-apart`, el destino de `POST /v1/projects/merge` (y su origen cuando es la lápida de otro proyecto) y `from_project` de `POST /v1/memories/move`. El `undo` de `POST /v1/memories/move` que es el movimiento de una unión, o el de su `--undo`, MUST responder 409 `part_of_merge` nombrando la unión.

**Tests:** `canonical_follows_a_merge_and_leaves_any_other_id_as_it_is` y `a_cycle_or_a_chain_too_long_answers_the_id_itself` (`merge.rs`); `a_save_retried_after_a_merge_lands_in_its_target` (`worker.rs`); `a_move_to_a_merged_project_lands_in_its_target` y `undoing_a_move_whose_origin_was_merged_returns_to_its_target` (`relocate.rs`); `a_root_whose_project_was_merged_registers_into_its_target` (`project.rs`); `an_import_planned_before_a_merge_writes_into_its_target` (`import.rs`); `a_bridge_opened_before_a_merge_reads_and_writes_its_target`, `a_move_from_a_merged_project_is_refused` y `the_move_of_a_merge_is_undone_only_with_its_merge` (`tests/api.rs`).

## PL13. Unir X en Z

`POST /v1/projects/merge` con `{"source": X, "target": Z, "dry_run"}`, detrás del token (`merge_core`, `merge.rs`). Se rechaza sin escribir con 404 si X o Z no están en `project`, 400 si son el mismo id, 409 `project_merged` si Z es una lápida o X es la lápida de otro proyecto, y 409 `saves_pending`, `retryable`, si después de esperar hasta `SAVES_WAIT` (5 s) el outbox sigue con un save pendiente de X o de Z; la transacción que escribe lo comprueba de nuevo. Si X ya es una lápida de Z, la unión es una **reparación**: pasa a Z lo que siga bajo X, y X queda como estaba.

Cuando hay algo que cambiar, MUST escribir un snapshot `pre-merge` y, en una transacción que vuelve a decidir lo que vio una pasada en seco:

1. pasar a Z cada fila de `memory` bajo X, borradas incluidas, con la parte de su unidad que está en X (`merge_in` con `Span::Carry`): la parte de una unidad en otro proyecto queda ahí y se nombra en `split`. Después Z conserva el save más nuevo de cada `topic_key` y la fila más vieja de cada `content_hash` que toca una fila movida, y las demás quedan `superseded` con causa `revision`, como en R17, bajo un número de `memory_move`;
2. pasar a Z las filas de `episodic_trajectory` y `procedural_skill` de X;
3. pasar a Z cada fila de `synthesis_review` del alcance X, con el `reviewed_at` más viejo cuando Z ya tiene una;
4. pasar a Z las carpetas, los remotos y las lápidas que llevaban a X;
5. responder `merged` las sugerencias pendientes cuyos dos lados pasan a ser un proyecto;
6. dejar X con `merged_into = Z` y `merged_at`, y Z revisado;
7. anotar cada cambio con lo que había antes en `project_merge_change`.

Después de la transacción, MUST copiar el `workspace.md` de X de cada agente donde Z no tiene uno, como `adopt_legacy`, sin cambiar nunca el de X; uno que Z ya tiene queda al lado y se nombra. Las copias se anotan en el diario; una que falla queda en el log y en el informe. La unión MUST NOT cambiar el texto, el vector, el índice de texto ni `last_modified_at` de ninguna fila, los pares de `memory_conflict`, los ids ni `index_docs` de las carpetas, ni el índice de docs, y no necesita el modelo. Con `dry_run` responde el mismo informe sin escribir, sin snapshot y sin tocar archivos. El informe tiene `dry_run`, `merge_id`, `source`, `target`, `move_id`, `memories`, `superseded`, `split`, `protected`, `trajectories`, `skills`, `reviews`, `folders`, `remotes`, `tombstones`, `suggestions`, `workspaces` (`copied`, `kept_beside`, `failed`) y `snapshot`.

**Tests:** `merging_moves_every_row_and_locator_into_the_target`, `a_merge_keeps_the_newest_save_of_a_key_and_the_oldest_of_a_text`, `a_merge_moves_the_part_of_a_split_unit_it_holds`, `merging_a_target_repoints_its_tombstones`, `a_dry_run_merge_changes_nothing`, `a_merge_copies_a_workspace_only_where_the_target_has_none`, `a_merge_is_refused_while_a_save_of_its_projects_is_pending`, `a_merge_of_a_tombstone_into_its_target_repairs_it` y `a_merge_is_refused_for_an_unknown_the_same_or_a_merged_project` (`merge.rs`); `decide_merge_carrying_takes_the_joined_part_of_a_unit_elsewhere` (`relocate.rs`); `merging_through_the_daemon_leaves_everything_under_the_target` y `a_merge_waits_for_the_saves_of_its_projects` (`tests/api.rs`).

## PL14. El `--undo` de una unión

`POST /v1/projects/merge` con `{"undo": N, "dry_run"}` (`undo_merge_core`). Se rechaza sin escribir con 404 si la unión no existe, 409 `merge_undone` si ya se deshizo, 409 `merged_later` si su destino es ahora una lápida, nombrando la unión que hay que deshacer antes, y 409 `saves_pending` como en PL13.

MUST escribir un snapshot `pre-undo` y, en una transacción, dejar X vivo otra vez, devolver las filas que la unión dejó `superseded` si siguen así, llevar de vuelta a X las de `memory` que siguen en Z como un movimiento propio (`undo_move_in`, el cuerpo de `undo_move_core`), y lo demás que la unión cambió y sigue apuntando a Z: trayectorias, habilidades, carpetas, remotos y lápidas. Las filas de `synthesis_review` de X vuelven, y la de Z vuelve a su valor de antes si sigue con el que escribió la unión. Las sugerencias que respondió `merged` vuelven a estar pendientes, y Z, a su marca de revisado de antes. Después, cada copia de `workspace.md` que hizo la unión MUST borrarse si su parte del usuario sigue siendo la del archivo de X; si no, queda y se nombra (`drop_adopted`). Lo guardado en Z después de la unión queda en Z, y una fila que se movió a otro lado queda donde está y se nombra en `kept`.

**Tests:** `undoing_a_merge_leaves_the_store_as_it_was`, `undoing_a_merge_keeps_what_was_saved_after_it`, `a_merge_whose_target_was_merged_since_is_undone_after_that_one` y `a_workspace_edited_since_the_merge_is_kept` (`merge.rs`).

## PL15. Lo que se ve de una lápida

Cambia PL9. `GET /v1/projects` MUST NOT listar una lápida: cada proyecto vivo lista en `merged` las que apuntan a él, cada una con `id`, `name`, `merge_id` (la unión que la hizo) y `merged_at`. Una carpeta o un remoto que llevan a una lápida se listan en su proyecto vivo, y una sugerencia se lista con sus lados por `canonical`: una cuyos lados son un solo proyecto no se lista, y dos con el mismo par se listan una vez. El nombre de una lápida queda libre para `rename`; su id sigue tomado. Renombrar una lápida es 409 `project_merged` (PL12). Un proyecto borrado se ve igual: PL27.

**Tests:** `a_merged_project_is_listed_under_its_target_and_frees_its_name` (`registry.rs`); `the_listing_after_a_merge_shows_the_target_alone` (`tests/api.rs`).

## PL16. Mantener dos proyectos separados

`POST /v1/projects/keep-apart` con `{"project_id": A, "other_id": B}`, detrás del token (`keep_apart`, `registry.rs`), responde 404 si alguno no está en `project`, 400 si son el mismo y 409 `project_merged` si alguno es una lápida. MUST responder `apart` cada sugerencia pendiente cuyos lados son A y B por `canonical` y, si no había ninguna, grabar el par con motivo `user` y esa respuesta, para que ningún registro lo sugiera después. A y B quedan revisados. Responde `{"project_id", "other_id", "answered"}`.

**Tests:** `keeping_two_projects_apart_answers_their_suggestion_for_good` (`registry.rs`); `a_pair_kept_apart_is_not_suggested_again` (`project.rs`).

## PL17. `clawg projects merge` y `clawg projects keep-apart`

`clawg projects merge <SOURCE> --into <TARGET> [--dry-run] [--profile P]`, `clawg projects merge --undo N [--dry-run] [--profile P]` y `clawg projects keep-apart <A> <B> [--profile P]` MUST hablar con el daemon que corre, sin arrancar uno, e imprimir en el idioma de la preferencia, con una sentencia por `match` sobre `Lang` (`render_merge`, `crates/clawg-cli/src/projects.rs`). El informe dice cuántas memorias pasaron y cuáles se reemplazaron, las unidades partidas, las preferencias y decisiones, lo que fue con ellas, las carpetas, los remotos y las lápidas, las copias de `workspace.md`, el snapshot y el número de la unión con el comando que la deshace. Un rechazo dice qué no se hizo con la causa del daemon como vino, y `saves_pending` dice que se puede volver a intentar. `clawg projects move` muestra igual la causa de un 409 que la trae en `detail` (`project_merged`, `part_of_merge`; `move_refused`). `clawg projects list` MUST nombrar debajo de cada proyecto los que se unieron en él, con el comando que deshace cada unión, y, cuando hay sugerencias, terminar con los dos comandos que las responden.

**Tests:** `projects_merge_and_keep_apart_parse` (`cli.rs`); `a_merge_report_names_what_moved_and_the_way_back`, `a_listing_names_what_was_merged_into_a_project` y `a_listing_marks_what_was_created_alone_and_the_suggestions` (en los dos idiomas) y `projects_are_merged_and_kept_apart_through_the_daemon` (`projects.rs`).

## PL18. `doctor` nombra los duplicados, los ids sin proyecto y las lápidas rotas

Los chequeos `project_duplicates`, `project_orphans` y `project_merges`, desde V27, están en [`specs/doctor`](../doctor/spec.md) (R7).

## PL19. Ninguna ruta nueva se olvida de la lápida

`PROJECT_INPUTS` (`crates/clawg-cli/tests/api.rs`) dice, de cada ruta de `router()`, en qué campo recibe un proyecto o que no recibe ninguno. Un test MUST leer `router()` de su fuente y fallar cuando una ruta no está en la tabla, y otro MUST mandar el id de una lápida a cada ruta de la tabla que recibe un proyecto y comprobar que actúa sobre el proyecto vivo o responde `project_merged` donde PL12 lo pide.

**Tests:** `every_route_says_which_project_it_takes` y `every_route_that_takes_a_project_follows_a_merge` (`tests/api.rs`).

## PL20. Lo que no cambia con la unión

- El contrato del modelo no cambia: el agente no une ni responde sugerencias.
- `POST /v1/projects` responde con la misma forma, con el proyecto vivo en `project_id`.
- Las uniones de un registro (R4, R17) siguen como estaban: su `merge_in` (`Span::Keep`) deja en su lugar la unidad que toca otro proyecto, porque el id viejo no queda como lápida.
- El índice de docs, los ids de las carpetas e `index_docs`.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (sin volver a bendecir), y los de R4, R17 y PL1 a PL10.

## PL21. V29: la marca de borrado, su diario y los disparadores que lo revierten

V29 (`crates/clawg-core/migrations/V29__project_delete.sql`) MUST:

- agregar a `project` la columna `deleted_at`, NULL por defecto;
- crear `project_delete`, una fila por borrado: `delete_id` (clave primaria), `project_id`, `with_memories`, `code`, `snapshot`, `deleted_at`, `undone_at` y `purged_at`, con un índice por `project_id`;
- crear `project_delete_row`, una fila por cada fila del proyecto que estaba en el conjunto: `delete_id`, `tbl` (`memory`, `episodic_trajectory` o `procedural_skill`), `row_id` y `deleted_at`, que es el que tenía antes del borrado (NULL si lo borró el borrado), con un índice por `delete_id`;
- crear `project_delete_folder`, una fila por carpeta a la que el borrado le bajó `index_docs`: `delete_id` y `root_id`;
- reemplazar `memory_project_ai` y `memory_project_au`, y crear `memory_project_restored` (PL22).

V29 MUST NOT cambiar ninguna fila. Un store en V28 abre con todos sus proyectos sin `deleted_at`, y un save bajo un id que `project` no conoce lo adopta como antes (PL2). El detalle de V29 y su vuelta atrás está en [`specs/storage`](../storage/spec.md).

**Tests:** `a_database_from_the_previous_version_opens_with_no_project_deleted` (`crates/clawg-core/src/memory/db.rs`); la fila 29 de `published_migration_checksums_are_pinned` (`migrations.rs`).

## PL22. Un proyecto borrado vuelve al usarse

Un disparador de `memory` MUST dejar vivo, con `reviewed = 0`, al proyecto borrado que recibe una fila viva:

- `memory_project_ai`, después de insertar una fila viva con `project_id`. Sigue adoptando un id que `project` no conoce (PL2);
- `memory_project_au`, después de cambiar el `project_id` de una fila viva;
- `memory_project_restored`, después de que una fila con `project_id` pasa de borrada a viva.

Así vuelve con un save de un puente abierto, que guarda los ids de su raíz toda la sesión, con un save que estaba en el outbox y `clawg outbox retry`, con un import, con `clawg undelete`, con el `--undo` de un movimiento que le devuelve filas vivas y con la unión de los ids viejos de un registro (R4).

MUST NOT revivirlo lo que no deja una fila viva bajo su id: registrar una de sus carpetas (que sigue llevando a él: `write_in` hace `INSERT OR IGNORE INTO project` y la respuesta nombra el proyecto borrado), buscar, pedir contexto, registrar una sesión o insertar una fila ya borrada. Lo que lo revive MUST NOT tocar sus filas borradas, el `index_docs` de sus carpetas ni su borrado: el borrado sigue abierto, con `back` en `deletes` (PL27), y su `--undo` (PL26) y su purga (PL28) siguen valiendo.

`episodic_trajectory` y `procedural_skill` no llevan disparador: en producción nadie las escribe. `doctor` nombra una fila viva bajo un proyecto borrado (PL30).

#### Escenario: un puente abierto después del borrado
- GIVEN un puente que registró `/work/app`, y el proyecto A de esa carpeta borrado con sus memorias
- WHEN el puente guarda con el id A
- THEN la memoria queda bajo A, y A vuelve a listarse, sin revisar, con esa memoria sola
- AND las memorias que borró el borrado siguen borradas, y el borrado sigue en `deletes` con `back`

#### Escenario: un hook en la carpeta de un proyecto borrado
- GIVEN el proyecto A de `/work/app`, borrado
- WHEN corre `session-start` en `/work/app`
- THEN el registro responde A, el contexto no trae memorias de A, y A sigue borrado

**Tests:** `a_live_row_brings_a_deleted_project_back_unreviewed` (por un insert, por un cambio de proyecto y por una restauración) y `a_deleted_row_or_a_registration_leaves_it_deleted` (`crates/clawg-core/src/memory/project_delete.rs`); `a_bridge_opened_before_a_delete_brings_its_project_back` (`crates/clawg-cli/tests/api.rs`).

## PL23. Borrar un proyecto vacío (etapa 1)

`POST /v1/projects/delete` con `{"project_id": X}`, detrás del token (`delete_core`, `project_delete.rs`; `delete_projects`, `crates/clawg-cli/src/daemon/api.rs`). Se rechaza sin escribir, y en este orden:

- 404 si X no está en `project`;
- 409 `project_merged` si X es una lápida, con `detail` que nombra el proyecto vivo;
- 409 `project_deleted` si X ya está borrado, con `detail` que nombra su borrado abierto y su `--undo` (`deleted_message`: «project X is deleted, in delete N: undo it with `clawg projects delete --undo N`», y sólo «project X is deleted» sin un borrado abierto);
- 409 `saves_pending`, `retryable`, si después de esperar hasta `SAVES_WAIT` (5 s) sigue habiendo en el outbox un save pendiente para X o para una lápida de X. La transacción que escribe lo comprueba de nuevo.

Si X tiene filas vivas, MUST responder 200 con el informe de PL24 con `refused: "not_empty"` y sin escribir: la respuesta es un informe y no un 409 porque la CLI arma con él sus comandos en el idioma de la persona. El informe dice cuántas filas vivas tiene en cada tabla y nombra los candidatos para vaciarlo:

- los proyectos de sus sugerencias pendientes, con los lados por `canonical` y con su motivo;
- los proyectos vivos con el mismo nombre, sin distinguir mayúsculas (`same_name`);
- los proyectos vivos que tienen una carpeta con el mismo primer commit que una de X y cuyos remotos no la contradicen (`same_first_commit`), con la regla de PL18.

Salen de la regla de `doctor` (`duplicates`, `crates/clawg-core/src/memory/inspect.rs`) sobre los hechos que lee la misma transacción. Un par ya respondido (`apart` o `merged`) y un proyecto borrado MUST NOT ser candidatos.

Si X está vacío, MUST hacer lo siguiente en una transacción, sin snapshot:

1. anotar en `project_delete_row` cada fila del proyecto, todas borradas, con su `deleted_at`;
2. bajar `index_docs` de cada carpeta de X que lo tenía, anotarla en `project_delete_folder` y sacar del índice de docs lo que haya bajo su id de raíz (`drop_root_index_in`, `crates/clawg-core/src/memory/project_docs/mod.rs`);
3. dejar a X con `deleted_at`;
4. escribir la fila de `project_delete` con `with_memories` falso.

El borrado MUST NOT cambiar las carpetas (salvo `index_docs`), los remotos, las sugerencias, las lápidas que llevan a X, `synthesis_review` ni los `workspace.md`. El cuerpo con `confirm` sin `with_memories`, o con `dry_run`, es 400 (PL24).

#### Escenario: un proyecto que ya se vació
- GIVEN un proyecto A cuyas memorias se movieron a B con `projects move`, con una memoria borrada y una carpeta con índice de docs
- WHEN se borra A
- THEN A no se lista, y `deletes` lo nombra con su número y una fila
- AND su carpeta tiene `index_docs` en 0, y el índice de docs de su raíz está vacío

#### Escenario: un proyecto con memorias
- GIVEN un proyecto A con tres memorias vivas, y un proyecto B con el mismo nombre
- WHEN se borra A
- THEN no se escribe nada, y el informe cuenta las tres memorias y nombra a B con `same_name`

**Tests:** `an_empty_project_is_deleted_and_its_docs_index_goes`, `a_project_with_live_rows_is_refused_with_its_candidates`, `a_delete_is_refused_for_an_unknown_a_merged_or_a_deleted_project` y `a_delete_waits_for_the_saves_of_its_project` (`project_delete.rs`); `deleting_through_the_daemon_takes_two_steps` (`tests/api.rs`).

## PL24. El informe y el código (etapa 2, primer paso)

`{"project_id": X, "with_memories": true}` MUST responder 200 con el informe del conjunto de X y su código, sin escribir nada y sin esperar al outbox. Se rechaza como en PL23, salvo por `saves_pending`: el informe cuenta los saves pendientes, y el segundo paso los espera.

El informe (`DeleteReport`, que un lector acepta con campos de menos) tiene:

- `project_id`, `name`, `with_memories`, `refused` (`not_empty`, `code_changed` o null), `code` (sólo en esta etapa; null en un `not_empty`), `delete_id` (cuando escribió) y `snapshot`;
- `live` y `deleted`: cuántas filas vivas y cuántas borradas tiene en cada tabla (`memories`, `trajectories`, `skills`). En un borrado confirmado, `live` es lo que ese borrado borró;
- de sus filas vivas de `memory`: `by_type` y `by_status` (mapas) y `by_author`, una lista de `{agent_id, rows}` con `agent_id` null para las filas que no guardó ningún agente;
- `protected`: los ids de sus `user_preference` y `architectural_decision` vivas;
- `split_pairs`: cuántos pares de `memory_conflict` unen una fila suya con una de otro proyecto o de ninguno. Quedan como están;
- `conclusions`: los ids de las conclusiones vivas de otro proyecto, o de ninguno, que salen de una fila viva suya. Con el borrado pasan a `to_review` ([`specs/synthesis`](../synthesis/spec.md), R6);
- `workspaces`: las rutas de los `workspace.md` del proyecto, que no se tocan;
- `folders`: los ids de raíz de las carpetas cuyo índice de docs se quita, y `remotes`: sus remotos, que siguen vinculados;
- `pending`: cuántos saves de X o de una lápida de X esperan en el outbox, y `failed`: cada save `failed`, con su `operation_id` y su `agent_id`, porque reintentarlo haría volver al proyecto;
- `candidates` (`project_id`, `name`, `reason`), como en PL23;
- `dry_run`, `restored` y `kept`, que son del `--undo` (PL26).

El código MUST resumir el conjunto exacto y MUST NOT guardarse antes del segundo paso. Cambian el código un save, un import o un movimiento que trae o se lleva una fila, un `PATCH` de importancia o tipo (`update_count`), un veredicto o un save bajo una clave (`status`), borrar o restaurar una fila (`deleted_at`) y un registro que vincula una carpeta o un remoto.

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

`{"project_id": X, "with_memories": true, "confirm": CÓDIGO}` se rechaza como en PL23, `saves_pending` incluido (espera hasta `SAVES_WAIT`). El código se compara sin los espacios de los bordes y sin distinguir mayúsculas, en dos pasadas como las de una unión: una en una transacción que no se confirma, y la segunda, después del snapshot, que lo recalcula y escribe. Si no es el del conjunto de ahora, en cualquiera de las dos, MUST responder 200 con el informe nuevo, con su código y `refused: "code_changed"`, sin escribir; si es la primera, sin snapshot, y si es la segunda el snapshot queda para la poda.

Si el código coincide, MUST escribir un snapshot `pre-delete` y, en una transacción:

1. marcar borradas a la vez, con el mismo `deleted_at`, todas las filas vivas del proyecto en las tres tablas, y sacar sus vectores (`memory_vectors`, `episodic_vectors`, `procedural_vectors`). El índice de texto lo resta su disparador (V8);
2. anotar en `project_delete_row` cada fila del conjunto: las que borró, con `deleted_at` NULL, y las que ya estaban borradas, con el suyo;
3. llevar a su estado de reposo a cada fila de afuera que disputaba una fila borrada (`release_challengers`, [`specs/disputes`](../disputes/spec.md)), que se anota la fecha como en cualquier borrado;
4. el índice de docs y `index_docs`, como en PL23;
5. dejar a X con `deleted_at` y escribir la fila de `project_delete` con `with_memories`, el código y el snapshot.

Como las filas se marcan juntas, MUST NOT revivir ninguna fila del proyecto (`REPLACED_SQL` sólo mira filas vivas). No llama a `delete_memory_core`. Una fila de afuera que una fila del proyecto reemplazó sigue reemplazada. El borrado MUST NOT cambiar el texto, el `status`, el `update_count` ni el `last_modified_at` de ninguna fila del proyecto, ni los pares de `memory_conflict`, los enlaces de `memory_derivation`, las carpetas (salvo `index_docs`), los remotos, las sugerencias, `synthesis_review`, los `workspace.md` ni el outbox. No necesita el modelo. El informe trae `delete_id`, `snapshot` y `code`. Un cuerpo con `confirm` sin `with_memories` es 400 en el daemon, y uno con `dry_run` y un proyecto también.

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

`{"undo": N, "dry_run": false}` (`undo_delete_core`). Se rechaza sin escribir:

- 404 si el borrado N no existe;
- 409 `delete_undone` si ya se deshizo, y 409 `delete_purged` si se purgó;
- 409 `deleted_later` si el mismo proyecto tiene, después de N, otro borrado abierto, nombrándolo;
- 409 `saves_pending`, como en PL23;
- 503 `embedder_not_ready` o `embedder_failed` si hay filas que restaurar y el modelo no está listo. Una prueba en seco y el `--undo` de un borrado sin filas que restaurar no lo necesitan: el daemon le pasa el modelo si lo tiene y no fuerza una carga.

Una primera pasada que no escribe decide qué filas vuelven y cuáles quedan; después se calculan fuera de la transacción los vectores de las que vuelven, en tandas de 32 con `embed_batch` (`embedding_text` para `memory`, `task` para una trayectoria y `skill_embedding_text` para una habilidad), se escribe un snapshot `pre-undo` (también para el de un proyecto vacío) y una transacción vuelve a decidir y escribe:

1. dejar vivo a X si sigue con el `deleted_at` de N. Si volvió al usarse, ya lo está. Es lo primero, así que el disparador de PL22 no le toca `reviewed`;
2. subir `index_docs` en las carpetas de `project_delete_folder` que siguen en 0. El próximo refresco reconstruye su índice;
3. restaurar cada fila que N borró y sigue como N la dejó, en X y con el `deleted_at` de N: `deleted_at` en NULL, su vector y su entrada en el índice de texto, conservando su `status`, su `update_count` y su `last_modified_at`. Después se lleva a su estado de reposo a la fila y a las que la disputaban (`settle_row`, `settle_challengers`), que se anotan la fecha;
4. dejar a N con `undone_at`.

Una fila que N borró MUST quedar borrada, y se nombra en `kept` (`table`, `id`, `reason`) con su motivo, cuando:

- `holds_secret`: `redact` encuentra en su texto un secreto con las reglas de hoy;
- `live_twin`: una fila viva (`active` o `disputed`) de X tiene su texto (`content_hash`);
- `topic_taken`: una fila viva de X, de otro save, tiene su `topic_key`;
- `changed`: ya no está como N la dejó (se restauró, se volvió a borrar sola, se movió o ya no existe), o no llegó a tener su vector entre las dos pasadas;
- `tied`: es una pieza de un save que tiene otra pieza que queda borrada; se repite hasta que no quede ninguna (`tied`, función pura).

Son las negativas de `undelete` ([`specs/restore`](../restore/spec.md), R1), en ese orden. Una fila que queda borrada deja como están las que reemplazó. Las filas que ya estaban borradas antes de N MUST NOT restaurarse. El `--undo` no corre el detector. El informe trae `delete_id`, `restored` (cuántas filas por tabla), `kept`, `folders` (las carpetas a las que subió `index_docs`), `snapshot` y `dry_run`.

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

MUST responder 409 `project_deleted`, con `detail` que nombra el borrado abierto y su `--undo` (`deleted_message`, una sola frase para todos):

- `PATCH /v1/projects/{id}`;
- cualquiera de los dos lados de `POST /v1/projects/keep-apart`;
- el origen o el destino de `POST /v1/projects/merge`, y el `--undo` de una unión cuyo destino está borrado;
- `to_project` de `POST /v1/memories/move`, y `project_id` de `PATCH /v1/memories/{kind}/{id}` (`target_in`, `relocate.rs`);
- el propio `POST /v1/projects/delete` sobre un proyecto ya borrado.

`from_project` de `POST /v1/memories/move` sí acepta un proyecto borrado: sus filas borradas se pueden sacar antes de purgarlo. El `--undo` de un movimiento que le devuelve filas vivas a un proyecto borrado lo deja vivo (PL22), porque esas filas eran suyas.

Lo que se ve, como con una lápida (PL15):

- `GET /v1/projects` MUST NOT listar un proyecto borrado, ni sus carpetas, ni sus remotos, ni las lápidas que llevan a él, ni sus sugerencias. Una sugerencia con un lado borrado no se lista hasta que ese lado vuelva.
- MUST responder también `deletes` (`list_deletes`, `crates/clawg-core/src/memory/registry.rs`): cada borrado abierto, por número, con `delete_id`, `project_id`, `name`, `with_memories`, `deleted_at`, `rows` y `back`. `rows` son las filas de su conjunto que siguen borradas, estén donde estén ahora: lo que su purga sacaría; `back` dice si el proyecto volvió al usarse. Un daemon anterior no lo manda, y la CLI lo lee como vacío.
- El nombre de un proyecto borrado queda libre para `rename`. Su id no.

La tabla de PL19 suma `POST /projects/delete`, con el proyecto en `body.project_id`, que responde `project_merged` a una lápida. Un test MUST mandar a cada ruta de esa tabla que recibe un proyecto el id de un proyecto borrado, y comprobar que pasa una de tres cosas: lo lee vacío, lo hace volver con un save, o responde `project_deleted` donde este requisito lo pide.

**Tests:** `a_deleted_project_is_refused_by_what_manages_projects` (`registry.rs`, `merge.rs` y `relocate.rs`); `a_deleted_project_is_listed_only_in_deletes_and_frees_its_name` (`registry.rs`); `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule`, y la ruta nueva en `every_route_says_which_project_it_takes` y en `every_route_that_takes_a_project_follows_a_merge` (`tests/api.rs`).

## PL28. `clawg purge --project-delete N`

`clawg purge --project-delete N --yes [--profile P]` (`purge_delete`, `crates/clawg-core/src/memory/secrets.rs`) MUST seguir las reglas de `clawg purge <ID>` ([`specs/secrets`](../secrets/spec.md), R5 y R5b de esa spec). El detalle de lo que saca, de lo que nombra y de sus negativas está ahí.

**Tests:** `purging_a_delete_leaves_no_trace_of_its_rows`, `purging_a_delete_keeps_a_row_restored_since`, `a_purge_of_a_delete_undone_or_purged_is_refused_untouched` y `purging_a_delete_names_the_backups_and_workspaces_that_hold_its_text` (`secrets.rs`); `purge_takes_an_id_or_a_delete` (`cli.rs`); `the_purge_report_of_a_delete_names_what_went` (`crates/clawg-cli/src/secrets.rs`); `a_deleted_project_is_purged_once_the_daemon_stops` (`tests/binary.rs`).

## PL29. `clawg projects delete`

- `clawg projects delete <ID> [--profile P]`
- `clawg projects delete <ID> --with-memories [--confirm CÓDIGO] [--profile P]`
- `clawg projects delete --undo N [--dry-run] [--profile P]`

`<ID>` y `--undo` se excluyen y uno es obligatorio; `--confirm` pide `--with-memories`; `--undo` no va con `<ID>`, `--with-memories` ni `--confirm`, y `--dry-run` sólo va con `--undo`. Los tres MUST hablar con el daemon que corre, sin arrancar uno, e imprimir en el idioma de la preferencia, con una sentencia por `match` sobre `Lang` (`render_delete`, `crates/clawg-cli/src/projects.rs`):

- **Un proyecto vacío borrado:** que se borró, su número y `clawg projects delete --undo N`. Si tiene memorias borradas, también `clawg purge --project-delete N --yes`. Sale con 0.
- **`not_empty`:** cuántas filas vivas tiene y cómo vaciarlo: `clawg projects merge X --into C --dry-run` por cada candidato, con su nombre y su motivo, `clawg projects move --from-project X --to-project <ID>`, o borrarlo con `--with-memories`. Sale con 1.
- **El informe:** lo de PL24 (conteos, tipos, autores y estados, preferencias y decisiones, pares, conclusiones, workspaces, carpetas, remotos, saves pendientes y cada save `failed` con `clawg outbox discard <OPERATION_ID> --agent <AGENTE>`), con los candidatos y terminado con el comando exacto `clawg projects delete X --with-memories --confirm CÓDIGO`. Sale con 0.
- **`code_changed`:** lo dice, imprime el informe nuevo con su comando y sale con 1.
- **Un borrado confirmado:** cuántas memorias borró, el snapshot, su número, `--undo N` y `clawg purge --project-delete N --yes`. Sale con 0.
- **Un `--undo`:** cuántas filas volvieron y cada una que quedó, con su motivo; con `--dry-run`, lo que haría. Sale con 0.

`clawg projects list` MUST terminar con los borrados abiertos (`render_listing`), cada uno con su `--undo N` y, si tiene filas, su purga. Uno cuyo proyecto volvió al usarse lo dice. Un rechazo (400, 404 o 409) nombra la causa del daemon tal como llegó (`delete_refused`) y sale con 1, y `saves_pending` dice que se puede volver a intentar. Los comandos que imprime no llevan `--profile`.

**Tests:** `projects_delete_parses` (`cli.rs`); `a_delete_report_names_what_goes_and_the_exact_confirm`, `a_refused_delete_names_the_ways_to_empty_it` y `a_listing_ends_with_the_open_deletes`, en los dos idiomas (`projects.rs`); `a_project_is_deleted_and_undone_through_the_daemon` (`projects.rs`, contra un daemon real).

## PL30. `doctor` y los proyectos borrados

- `project_duplicates` MUST NOT tomar un proyecto borrado, ni un par cuyo primer commit llega por una lápida a uno borrado (`duplicates`).
- Para `project_orphans` y `project_merges`, un proyecto borrado es un proyecto: sus filas no son huérfanas, y una cadena de lápidas que termina en él no está rota.
- Un chequeo nuevo, `project_deletes`, MUST nombrar con `warn` y `live_under_deleted` cada proyecto borrado que tiene filas vivas en alguna de las tres tablas. Sólo puede pasar en `episodic_trajectory` o `procedural_skill` (PL22), o en un store editado a mano. Su evidencia nombra el proyecto, su borrado abierto más reciente y los conteos de filas vivas por tabla, y no tiene paso. En un store anterior a V29 no corre: sin la tabla `project_delete` no hay hallazgo, ni siquiera un `ok`.

**Tests:** `a_deleted_project_is_no_duplicate_nor_orphan` y `live_rows_under_a_deleted_project_are_named` (`crates/clawg-cli/src/doctor/checks.rs`); `the_facts_read_a_deleted_project_where_the_schema_has_one` y `a_deleted_project_is_no_duplicate_and_leads_nowhere_for_a_tombstone` (`inspect.rs`); `the_findings_are_the_same_in_both_languages_but_their_message` sigue pasando.

## PL31. Lo que no cambia con el borrado

- El contrato del modelo (`agent_contract.json`, `agent_contract.es.json`): ninguna herramienta borra un proyecto, ninguna llama a la ruta de PL23, y `delete_memory` sigue igual.
- `clawg purge <ID>`. Y `clawg undelete <ID>`, que ahora además hace volver al proyecto de su fila (PL22).
- Las uniones de antes: las lápidas que llevan a un proyecto borrado siguen llevando a él.
- El índice de docs sigue por carpeta, y los ids de las carpetas no cambian.
- El formato de export.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, sin volver a bendecir, y los de R4, R17 y PL1 a PL20.

## Definiciones de los proyectos sin carpeta

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

`recall_context` MUST terminar con una línea por sugerencia, en el idioma de la sesión, que nombra el otro proyecto, el motivo, y que hay que preguntarle a la persona y responder con `answer_suggestion`. `session-start` MUST terminar con una sola línea que nombra hasta dos de los otros proyectos (y cuántos más hay), que hay que preguntar y responder con `answer_suggestion`, y que `recall_context` dice por qué: lo que suma el hook pasa a tener un tope de 1.500 bytes, y con los 8.000 del contexto sigue entrando en los 10.000 caracteres que inyecta el host (R4 de [`specs/hooks`](../hooks/spec.md)). Se muestran hasta que se responden. Un daemon que no manda el campo no muestra nada.

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

## PL42. Lo que no cambia con los proyectos sin carpeta

- El esquema: ninguna migración.
- La resolución de una raíz (R1, PL4, PL5): un subproyecto de un monorepo sigue siendo parte del repositorio (un cambio posterior).
- `POST /v1/projects` y los ids que da: el puente y los hooks siguen igual sin el argumento nuevo.
- `rename`, `merge`, `keep-apart` y `delete` siguen tomando ids.
- El import no crea sugerencias ni proyectos sin carpeta (lo cambió la entrega 4: PL57 y PL58).

## Definiciones de los subproyectos de un monorepo

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
3. elegir la más profunda. A igual subcamino, la de P va antes que la de una lápida, y entre lápidas gana la declaración más vieja;
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

La respuesta trae `project_id` (X), `root_id`, `repository_id` (P), `subpath` (S), `already`, `removed`, cuando quitó una declaración `previous` (el proyecto al que llevaba, que conserva lo guardado allí) y, cuando unió, `merge`.

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

`clawg projects add` y `clawg projects locate` sin `--subfolder` no mandan `path`: tratan la raíz y su índice de docs. El import manda el camino crudo de cada raíz de sus mapas (PL61).

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
- El import y el formato de export, que cambió la entrega 4 (PL54 a PL64).
- `clawg projects add` y `clawg projects locate` sin `--subfolder`.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, sin volver a bendecir, y los de R1, R4, R6 y PL1 a PL42.

## Definiciones de exportar e importar

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

Un store se lee sin migrar, con lo que tiene: antes de V27, una línea por carpeta cuyo id es el proyecto de una fila, sin `name`, como antes de esta entrega; antes de V24, sin remotos; antes de V28, sin lápidas; antes de V30, sin declaraciones.

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
- Una fila cuyo `project_id` no tiene línea lo conserva, como antes. Si `project` no lo tiene, el import MUST crearlo él, con `born = 'import'` y el nombre que da el id, y anotarlo (PL63), en lugar de dejárselo al disparador de PL2.

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
- Una fila que el import inserta borrada MUST NOT hacerlo volver. El import inserta viva una fila borrada y después la marca, por el índice de texto, y el disparador de inserción la ve viva (`V29__project_delete.sql`): en la misma transacción, el import MUST dejar el proyecto con el `deleted_at` y el `reviewed` que tenía (`SqlSeat::insert`, `crates/clawg-core/src/memory/portability/import.rs`).
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

- **El CLI.** `--project-map NOMBRE=project:REF` MUST separarse en el último `=project:`, y si no lo hay, `NOMBRE=CAMINO` en el último `=`, como antes (`ProjectMap`, `crates/clawg-cli/src/cli.rs`).
- **Una referencia.** REF es un id o un nombre, y el daemon MUST resolverla como el destino de un movimiento (PL35): primero como id, sin seguir la lápida, que se sigue en la transacción que escribe; después como el nombre de un solo proyecto vivo. No crea nada ni busca parecidos. Antes de leer el archivo, MUST responder 404 si no es ninguno, 409 `project_ambiguous` si es de dos o más, y 409 `project_deleted` si es un proyecto borrado (PL27), con el `detail` de siempre. La transacción que escribe MUST comprobar el borrado otra vez: las filas de un proyecto que se borró mientras tanto se reportan inválidas, sin hacerlo volver.
- **Un camino.** Para `--project-map NOMBRE=CAMINO` y `--map-root OLD=NEW`, el CLI MUST mandar la raíz y el camino crudo, absoluto y canónico. El daemon MUST resolver la declaración más profunda como un registro (PL45): un CAMINO dentro de una subcarpeta declarada lleva a su subproyecto.
- **El cuerpo.** Cada par de `project_map` y de `map_roots` MUST aceptar como destino un texto (una raíz, como antes), `{"root", "path"}` o, sólo en `project_map`, `{"project"}`. El CLI MUST mandar siempre un objeto, así un daemon anterior rechaza el cuerpo, porque espera un texto, en lugar de leer un camino crudo como una raíz.
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

`clawg import --undo N` MUST, después de retirar las filas, las carpetas y los remotos (IU2), y en la misma transacción:

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
