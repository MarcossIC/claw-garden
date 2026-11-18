# core-use-cases — Verificación

**Resultado:** las partes 1 (el guardado completo) y 2 (las escrituras de proyectos) aplicadas y verificadas en Linux. En cada una, la suite entera y los dos clippy pasan sobre el último commit de código, y ningún test de respuestas ni ninguno de los dos contratos del agente cambió. Queda lo que se lista en [«Pendiente»](#pendiente) y en el [«Pendiente» de la parte 2](#pendiente-de-la-parte-2). Las partes 3 y 4 están en [`tasks.md`](tasks.md#partes-siguientes).

## Qué se corrió

Sobre el último commit de código (`refactor(core): parse and admit a save in the core, not in the daemon`), en Linux, con un único directorio de build (`target`):

| Comando | Resultado |
|---|---|
| `cargo test --workspace` | pasa, sin ningún fallo; incluye los doctests |
| `cargo clippy --workspace --all-targets -- -D warnings` | sin hallazgos |
| `cargo clippy -p clawg-cli -- -D warnings` | sin hallazgos; es el núcleo sin `testing` ni `eval`, como lo construye el release |

Además, `cargo check -p clawg-cli` y `cargo check --workspace --all-targets` no dejan ningún warning.

## Los tests en rojo

Todos en `clawg-core`, sin router ni HTTP:

| Test | Qué se vio antes del código |
|---|---|
| Los diez de `crates/clawg-core/src/memory/ingest/save.rs` (R1, R2) | no compilaban: `SaveInput`, `NewSave`, `SaveRejected`, `check_operation_id` y los topes no existían |
| Los siete de `mod tests::admit` en `crates/clawg-core/src/runtime.rs` (R2 a R6) | no compilaban: `MemoryRuntime::admit` no existía, ni las variantes `MissingSources`, `ReplacedSources`, `EmbedderFailed` y `Store` |

`replaced_sources_name_what_replaced_them` y `missing_sources_are_the_ones_that_were_not_found` se escribieron con el primer grupo y se sacaron hasta tener `admit`, para que el formato compilara solo; volvieron con el segundo grupo, todavía en rojo.

## La caracterización

`each_refused_save_keeps_its_status_and_body` (`mod tests` de `crates/clawg-cli/src/daemon/api.rs`) se commiteó antes de tocar el handler (`test(daemon): pin the status and body of every refused save`) y pasó sobre el código de `develop`. Fija, con el cuerpo JSON entero:

- las siete reglas de formato de R1, cada una sola, y seis pares que fijan el orden;
- las fuentes que faltan (inexistente, borrada y oculta por `sharing.json`) y las reemplazadas, y que una que falta va antes que una reemplazada;
- el `503 embedder_failed` con `retryable` `false` y `true`, y que las fuentes se comprueban antes que el embedder;
- los tres `500` de un store cerrado: con `derived_from` (`memory backend channel error: source lookup: ConnectionClosed`), con un proyecto (`Memory DB connection error: ConnectionClosed`) y sin ninguno de los dos, en el outbox (`memory backend channel error: outbox_record: ConnectionClosed`).

Los tres textos del `500` son los que dio `develop`; el borrador del test tenía otros dos y se corrigieron con lo que respondió el código de ese momento, antes del commit. Después del cambio pasa sin haberse tocado.

## Las comprobaciones de D8

1. **Los tests de respuestas.** `git diff origin/develop -- crates/clawg-cli/tests crates/clawg-cli/src/mcp` no muestra nada: ni `tests/api.rs`, ni los tests del puente, ni `agent_contract.json` ni `agent_contract.es.json`. Los dos tests de contrato pasan sin `CLAWG_BLESS`.
2. **Los tests puros que se mudaron.** `content_is_checked_in_bytes`, `sources_are_counted_once_and_sorted`, `too_many_sources_are_refused`, `replaced_sources_name_what_replaced_them`, `missing_sources_are_the_ones_that_were_not_found` y `operation_ids_are_checked` pasan de `crates/clawg-cli/src/daemon/api.rs` a `save.rs` con el mismo nombre y los mismos literales. Donde comparaban `ApiError::status` con `400` comparan la variante, y donde comparaban `.message`, el `Display`. `replaced_sources_name_what_replaced_them` compara ahora el mensaje entero de `ReplacedSources` en lugar de la lista sola.
3. **El diff de `api.rs`.** Ningún `StatusCode` cambia. Los `ApiError` que se van son los de las funciones que se mudaron y los del cuerpo de `save_memory`; los que llegan son los tres brazos de `save_refused` (`bad_request`, `embedder_failed`, `internal`), con los mismos estados, y el `bad_request` de `requested_operation`, que antes venía de `check_operation_id`.

## El handler

| | Antes | Después |
|---|---:|---:|
| `save_memory` | 133 líneas (`api.rs:1420-1552`) | 65 líneas |
| líneas de producción de `api.rs` | 3.412 | 3.305 |
| funciones de producción de `api.rs` | 126 | 123 |
| usos de `bad_request` en producción | 68 | 61 |

`save_memory` ya no nombra `EmbedderStatus`, `existing_memory_ids`, `replaced_memory_ids`, `live_project`, `validate_topic_key` ni `MemoryEvent`. De `MemoryType` sólo usa `as_str`, para la etiqueta de la métrica de la sesión. `existing_memory_ids` y `replaced_memory_ids` pasaron a `pub(crate)`, porque ya no los usa otro crate.

## Lo que se resolvió distinto de lo escrito

- **Las tareas 3 y 4 son un commit.** Con el formato solo en el núcleo, el handler seguía comprobando las fuentes y armando el evento, y `NewSave` habría tenido que mostrar getters y dejar cambiar su proyecto desde otro crate hasta el commit siguiente ([tarea 3](tasks.md#3-el-guardado-en-el-núcleo-r1-a-r7)).
- **`Admitted` está en `save.rs`**, como dice D1, y no en `runtime.rs`; `admit` lo construye con sus campos `pub(crate)`.
- **`EmbedderFailed` tiene un `Display` propio** (`the embedder failed: <causa>`) que ningún cliente ve: el handler responde con `ApiError::embedder_failed`, que usa los campos.

## Pendiente

Lo que esta verificación no cubre:

- **Windows.** La suite y los dos clippy no se corrieron en Windows; el job `test (windows-latest)` del CI del PR lo hace. El cambio no agrega código `cfg(windows)` ni toca rutas de archivos, salvo `sharing.json`, que `admit` lee con `sharing::reader_in` igual que antes lo leía el handler.
- **El modelo real.** Ningún test corrió con el modelo real: el guardado se probó con `FakeEmbedder`. Un save con el embedder `loading` mientras el modelo se descarga de verdad (el `202 embedder_loading`) sólo se comprueba con un daemon de perfil `dev` que todavía no tiene el modelo; la decisión de no esperar es la misma de antes, ahora en `Admitted::waits`.
- **`rag-eval`.** No corre: el cambio no toca la recuperación ni `eval_api` ([D6](design.md#d6-eval_api-no-cambia)).
- **Al archivar:** consolidar R1 a R6 en `specs/ingest`, `specs/synthesis` y `specs/provenance` con ids nuevos (R7 es de conservación y no se consolida), y corregir las citas que nombran las funciones que se mudaron: `check_content` (`specs/ingest`, «El contenido: vacío o sobre el tope») y los tests puros de `crates/clawg-cli/src/daemon/api.rs` que cita `specs/synthesis`, que ahora están en `save.rs`. Subir la versión y `openspec/ROADMAP.md`.

## Parte 2. Las escrituras de proyectos

### Qué se corrió

Sobre el último commit de código (`refactor(core): refuse a confirm without its rows in delete_core`), en Linux, con un único directorio de build (`target`):

| Comando | Resultado |
|---|---|
| `cargo test --workspace` | pasa, sin ningún fallo; incluye los doctests |
| `cargo clippy --workspace --all-targets -- -D warnings` | sin hallazgos |
| `cargo clippy -p clawg-cli -- -D warnings` | sin hallazgos |

Cada commit de código pasó antes los tests de los archivos que cambia, los de su tarea en [`tasks.md`](tasks.md#parte-2-las-escrituras-de-proyectos), y además `cargo test -p clawg-cli --test api` con los filtros `regist`, `locat`, `subfolder` y `delet`, y `--test conventions`.

### Los tests en rojo

Todos en `clawg-core`, sin router ni HTTP. Se escribieron antes del código y no compilaban:

| Test | Qué se vio antes del código |
|---|---|
| Los tres de `check_root` en `crates/clawg-core/src/memory/projects/project.rs` (R8) | `check_root` no existía, ni `RegisterError::EmptyRoot` y `NotADirectory` |
| Los seis de `locate_folder` en `crates/clawg-core/src/memory/projects/registry.rs` (R9) | `check_folder`, `locate_folder`, `Located` y `LocateError` no existían |
| `a_confirm_without_the_rows_is_refused_before_anything_is_read` en `project_delete.rs` (R10) | `DeleteError::ConfirmWithoutRows` no existía |

### La caracterización

`each_refused_registration_keeps_its_status_and_body`, `each_located_folder_keeps_its_status_and_body` y `each_refused_project_delete_keeps_its_status_and_body` (`mod tests` de `crates/clawg-cli/src/daemon/api.rs`) se commitearon antes de tocar un handler (`test(daemon): pin the status and body of every refused registration, locate and project delete`) y pasaron sobre el código de `develop` la primera vez. Fijan, con el cuerpo JSON entero:

- **el registro:** la raíz vacía (también con `index_docs` y un id viejo malo), la raíz a indexar que no existe o es un archivo, que una raíz que no es un directorio va antes que un id viejo malo, el id viejo malo solo, y los dos `detail` de `project_root_collision`: el de una raíz cuyo id tiene otra, y el de una raíz guardada con otro id;
- **la ubicación:** la carpeta que no existe, el archivo con `subfolder` (la regla del directorio va antes que la de la subcarpeta), lo alto de un checkout con `subfolder`, el nombrado que no existe (`404`) y el ambiguo (`409 project_ambiguous`), `folder_shared` con su `detail`, una unión rechazada porque el nombrado está borrado (`409 project_deleted`), y los dos cuerpos que se aceptan: `already` en `false` con la unión (su `source`, `target`, `dry_run` y `merge_id`) y después en `true` sin ella;
- **el borrado:** las cinco formas del cuerpo que no piden nada, el `confirm` sin `with_memories` (con y sin `with_memories: false`) antes del `404` de un proyecto que no existe, y los `404` de un proyecto y de un borrado que no existen.

Después de los tres commits de código pasan sin haberse tocado.

### Las comprobaciones de D14

1. **Los tests de respuestas.** `git diff origin/develop -- crates/clawg-cli/tests crates/clawg-cli/src/mcp` no muestra nada. Los dos tests de contrato pasan sin `CLAWG_BLESS`.
2. **El diff de `api.rs`.** Las líneas con `StatusCode` que cambian son todas de los tests nuevos. Los textos que se van del handler (`root_path must not be empty`, `… is not a directory, so it has no files to index`, `… is not a directory: name a directory of the project`, el `detail` de `folder_shared` y `confirm goes with with_memories: …`) llegan como el `Display` de las variantes, y la caracterización compara el cuerpo entero.
3. **Los tests del núcleo** no construyen un `AppCtx`, un router ni un `StatusCode`: `check_root` y `check_folder` con un `is_dir` de prueba, que en los casos sin `index_docs` entra en pánico si se lo llama; `locate_folder` y `delete_core` sobre `MemoryStore::new_for_test`, cerrado donde el test pide que el store no se lea o falle.

### Los handlers

Contadas de la línea de la firma a la llave que cierra, y las de producción de `api.rs` hasta `mod tests`:

| | `develop` | Después |
|---:|---:|---:|
| `register_root` | 71 líneas | 50 |
| `locate_project` | 49 líneas | 41 |
| `delete_projects` | 26 líneas | 23 |
| líneas de producción de `api.rs` | 3.306 | 3.306 |
| usos de `bad_request` en producción | 61 | 59 |

Las líneas de producción quedan iguales porque entran dos traducciones nuevas, `register_refused` (el `match` de `RegisterError` que antes estaba dentro de `register_root`) y `located_refused`. `locate_project` ya no nombra `own_folder_count` ni `merge_core`, y `register_root` ya no compara `root_path` ni llama a `is_dir` por su cuenta. `own_folder_count` pasó a privada de `registry.rs`.

### Lo que se resolvió distinto de lo escrito

- **Los textos de `project_root_collision`** nombran `root_path` como llegó, en lugar de `Path::new(root_path).display()`: es el mismo texto para un `String`, y `register_refused` no necesita la `Path`.
- **`Located` lleva `#[expect(clippy::large_enum_variant)]`.** `clippy` lo encontró en la verificación: `Merged` lleva un `MergeReport` de unos 400 bytes y `Already` nada. Se toma la excepción con el mismo motivo que `subfolder::Step` (uno por ubicación, se mira apenas vuelve y no se guarda) en lugar de un `Box`, y entró en el commit de la tarea 9; la suite entera se corrió otra vez sobre el último commit después.
- **`ConfirmWithoutRows` es un brazo del `match` de `delete_core`** (`(false, Some(_))`), el primero que se evalúa, y no una comprobación aparte: no hay lectura antes, y el `(false, _)` de antes, que ignoraba el código, deja de existir. El doc-comment de `delete_core` decía que el código se ignoraba; ahora dice que se rechaza.

### Pendiente de la parte 2

- **Windows.** La suite y los dos clippy no se corrieron en Windows; lo hace el job `test (windows-latest)` del CI del PR. `check_root` y `check_folder` reciben `Path::is_dir` y una `PathBuf` hecha del `root_path` que llegó, que se muestra igual en los dos sistemas; los tests de caracterización usan caminos de `tempfile` y `/no/such/dir`, que en Windows tampoco es un directorio.
- **El modelo real.** No hace falta: ninguna de las tres rutas lo usa salvo el `--undo` de un borrado, que no cambió.
- **`rag-eval`.** No corre: el cambio no toca la recuperación.
- **Al archivar:** consolidar R8 a R10 en `specs/project` con ids nuevos (R11 es de conservación) y corregir SB3, que dice que `locate_project` decide con `own_folder_count` (`api.rs:3097`): ahora decide `locate_folder`, y `own_folder_count` es privada de `registry.rs`. PL24 dice que el `confirm` sin `with_memories` es un `400`, lo que sigue siendo cierto; la regla vive ahora en `delete_core`.
