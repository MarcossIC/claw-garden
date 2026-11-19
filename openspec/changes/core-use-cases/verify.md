# core-use-cases — Verificación

**Resultado:** las partes 1 (el guardado completo), 2 (las escrituras de proyectos) y 3 (las otras escrituras) aplicadas y verificadas en Linux. En cada una, la suite entera y los dos clippy pasan sobre el último commit de código, y ningún test de respuestas ni ninguno de los dos contratos del agente cambió. Queda lo que se lista en [«Pendiente»](#pendiente), en el [«Pendiente» de la parte 2](#pendiente-de-la-parte-2) y en el [de la parte 3](#pendiente-de-la-parte-3). La parte 4 está en [`tasks.md`](tasks.md#partes-siguientes).

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

## Parte 3. Las otras escrituras

### Qué se corrió

Sobre el último commit de código (`refactor(daemon): answer a refused restore by its variant`), en Linux, con un único directorio de build (`target`):

| Comando | Resultado |
|---|---|
| `cargo test --workspace` | pasa, sin ningún fallo; incluye los doctests |
| `cargo clippy --workspace --all-targets -- -D warnings` | sin hallazgos |
| `cargo clippy -p clawg-cli -- -D warnings` | sin hallazgos |

Cada commit de código pasó antes los tests de los archivos que cambia, los de su tarea en [`tasks.md`](tasks.md#parte-3-las-otras-escrituras), y además `cargo test -p clawg-cli --test api` con los filtros `suggest`, `resolv` y `restor`, `--test conventions` y `cargo check --workspace --all-targets`, que compila `eval_api` con su `match` sobre `ResolveError`.

### Los tests en rojo

Todos en `crates/clawg-core/src/memory/projects/registry.rs`, sin `AppCtx`, router ni `StatusCode`. Se escribieron antes del código y no compilaban:

| Test | Qué se vio antes del código |
|---|---|
| `an_answer_is_merge_or_apart`, `an_answer_is_the_users_word` y `each_answer_refusal_keeps_its_text` (R12) | `SuggestionAnswer`, `UserAnswer` y `AnswerError` no existían en el núcleo |
| `an_answer_needs_a_pending_suggestion`, `an_answer_checks_both_projects`, `apart_keeps_the_two_from_being_suggested_again`, `merge_merges_the_other_into_the_project`, `a_refused_merge_refuses_the_answer` y `a_store_that_fails_refuses_the_answer` (R13) | `answer_suggestion_core` y `Answered` no existían; 30 errores del compilador entre los nueve |
| `new_project_without_create_is_refused_before_anything_is_read` (R14) | la variante `ResolveError::NewProjectWithoutCreate` no existía |

Los tres puros prueban `UserAnswer::new` sin store; los de R13 y R14, sobre `MemoryStore::new_for_test` con las filas sembradas con SQL. La unión rechazada es una fila `pending` del outbox con el proyecto que se une (`SavesPending(1)`), y el store que falla es uno cerrado.

### La caracterización

Los cinco tests de `mod tests` de `crates/clawg-cli/src/daemon/api.rs` se commitearon antes de tocar un handler (`test(daemon): pin the status and body of every update, delete, restore, dispute, answer and project resolve`) y pasan sobre el código de `develop`. El borrador llevaba un marcador en los cinco `500` de un store cerrado y en el informe de la unión, para leer el texto que da `develop`; dos literales más (el `404` de un movimiento a un proyecto no registrado y el `503` de un restore sin embedder, que pedía un runtime sin el `FakeEmbedder` de `new_for_test`, `daemon_without_embedder`) se corrigieron con lo que respondió el código de ese momento, antes del commit. Fijan, con el cuerpo JSON entero:

- **`each_memory_update_keeps_its_status_and_body`:** el `kind` retirado y el desconocido (antes que el cuerpo vacío), los dos destinos juntos, un nombre ambiguo que responde `409` antes que una importancia fuera de escala, el cambio vacío (también con sólo `user_answered`), la importancia fuera de 0 a 10, el tipo desconocido y que la importancia va antes, que el formato va antes que la fila, la fila que no existe, la borrada y la oculta por `sharing.json`, los dos `409` de una preferencia (el tipo y el proyecto), el `404` de un proyecto no registrado, cuatro cambios aceptados (la importancia, el tipo de otro agente, el tipo de una preferencia con `user_answered`, el proyecto ida y vuelta con su `move_id`) y el `500`;
- **`each_memory_delete_and_restore_keeps_its_status_and_body`:** el borrado con el `kind` desconocido, la fila que no existe, la oculta, el aceptado (`{"deleted": 1}`) y el segundo `404`; el restore con el `kind` desconocido, la fila que no existe, la viva, la oculta, los tres `409` (un secreto, un gemelo vivo, un tema ocupado), el aceptado y el segundo `404`; sin embedder, el `kind` antes que el `503 embedder_not_ready` y el `503 embedder_failed`; y los dos `500`;
- **`each_resolved_dispute_keeps_its_status_and_body`:** la misma fila (también con ids que no existen), las dos filas que no existen en su orden, la oculta, el par que no está en disputa, la fila reemplazada por otra, la preferencia sin la palabra de la persona, `keep_both`, el reemplazo y su vuelta, y el `500`;
- **`each_answered_suggestion_keeps_its_status_and_body`:** la respuesta desconocida antes que la palabra de la persona, las cinco formas sin esa palabra (también con proyectos que no existen), los dos proyectos que no existen, el mismo, el fusionado, el borrado, `no_suggestion`, el `apart` aceptado y que no se responde dos veces, el `merge` aceptado con el informe entero (salvo `merge_id`, un número, y `snapshot`, un camino `pre-merge`) y después `project_merged`, el `500`, y con el store cerrado la palabra de la persona antes de leerlo;
- **`each_resolved_project_keeps_its_status_and_body`:** `new_project` sin `create` antes que el nombre vacío, los tres `NameError`, los tres `refused` (`ambiguous` aun con `create`, `similar` aun con `create`, `unknown`), el id de un fusionado, el nombre con otra forma, el borrado, el creado y su segunda lectura, `new_project` con `create` al lado de uno parecido, y con el store cerrado `new_project` antes de leerlo y el `500`.

Después de los tres commits de código pasan sin haberse tocado.

### Las comprobaciones de D19

1. **Los tests de respuestas.** `git diff develop...HEAD -- crates/clawg-cli/tests/ crates/clawg-cli/src/mcp/` no muestra nada. Los dos tests de contrato pasan sin `CLAWG_BLESS`.
2. **El diff de `api.rs`.** Ninguna línea con `StatusCode` se va; las que llegan son de los tests nuevos. Los textos que se van del handler (`answer must be one of: …`, `answering a suggestion is the user's call: …`, `no suggestion is pending between …`, `new_project goes with create: …` y `no deleted memory row with id …`) llegan como el `Display` de `AnswerError`, `ResolveError::NewProjectWithoutCreate` y `RestoreError::NotFound`, y el código `no_suggestion` sigue en `answer_refused`.
3. **Los tests del núcleo** no construyen un `AppCtx`, un router ni un `StatusCode`.

### Los handlers

Contadas de la línea de la firma a la llave que cierra, y las de producción de `api.rs` hasta `mod tests`:

| | `develop` | Después |
|---|---:|---:|
| `answer_suggestion` | 37 líneas | 18 |
| `resolve_projects` | 14 líneas | 11 |
| `update_memory`, `delete_memory`, `restore_memory`, `resolve_dispute` | 28, 16, 18 y 11 | iguales |
| líneas de producción de `api.rs` | 3.306 | 3.288 |
| usos de `bad_request` en producción | 59 | 57 |

`answer_suggestion` ya no nombra `suggestion_between`, `keep_apart` ni `merge_core`, y `SuggestionAnswer` es del núcleo. Entra una traducción, `answer_refused`.

### Lo que se resolvió distinto de lo escrito

- **`daemon_on` y `daemon_without_embedder`.** El `AppCtx` de prueba que arma `daemon_embedding_with` pasó a `daemon_on`, para un daemon cuyo runtime no tiene embedder: `MemoryRuntime::new_for_test` ya siembra un `FakeEmbedder`, y sin eso el `503` de un restore no se alcanza. Entró en el commit de la caracterización.
- **`each_answer_refusal_keeps_its_text`** comprueba también la forma serializada de `SuggestionAnswer` (`merge`, `apart`), que es la que lleva `AnsweredBody`.

### Pendiente de la parte 3

- **Windows.** La suite y los dos clippy no se corrieron en Windows; lo hace el job `test (windows-latest)` del CI del PR. El cambio no agrega código `cfg(windows)` ni toca caminos de archivos: el informe de la unión se compara sin su `snapshot`.
- **El modelo real.** No hace falta: de las seis rutas sólo el restore embebe, con el `FakeEmbedder`, y su código no cambió salvo el texto del `404`, que es el mismo.
- **`rag-eval`.** No corre: el cambio no toca la recuperación.
- **Al archivar:** consolidar R12 a R14 en `specs/project` con ids nuevos (R15 es de conservación). PL37 y PL32 siguen siendo ciertas como conducta (el `400` sin la palabra de la persona, el `404 no_suggestion`, `new_project` con `create`); las reglas viven ahora en `UserAnswer::new`, `answer_suggestion_core` y `resolve_project`, y `resolve_project` rechaza `new_project` sin `create` en lugar de ignorarlo.

## Parte 4. Partir `api.rs`

### Qué se corrió

Sobre el último commit de código (`refactor(daemon): move the memories routes out of api.rs`), en **Windows**, con un único directorio de build (`target-test`, limpiado con `cargo clean --workspace --target-dir target-test` antes del primer build):

| Comando | Resultado |
|---|---|
| `cargo test --workspace --target-dir target-test` | pasa, sin ningún fallo; incluye los doctests |
| `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings` | sin hallazgos |
| `cargo clippy -p clawg-cli --target-dir target-test -- -D warnings` | sin hallazgos |

Es la primera corrida de la suite y de los dos clippy en Windows de este cambio: cubre el «Pendiente» de Windows de las partes 1 a 3 en lo que a la suite y a clippy se refiere. El CI de Windows del PR sigue siendo la comprobación de lo que no se corre aquí (el binario de release, los jobs de empaquetado).

Cada commit de código pasó antes `cargo test -p clawg-cli --lib --target-dir target-test daemon::api` (57 tests, los mismos en cada paso) y `cargo check -p clawg-cli --all-targets --target-dir target-test` sin avisos. El último, además, `cargo check --workspace --all-targets` y `--test conventions`.

### Los tests en rojo

No hay: es un movimiento y no agrega ninguna regla ([R16](spec.md#r16-partir-apirs-no-cambia-nada)). La prueba de que nada cambia son los tests que ya existían.

### Las comprobaciones de D23

1. **Los nombres de los tests.** `cargo test -p clawg-cli --lib -- --list`, ordenado, antes de empezar y al terminar: 1.106 tests en cada lista, y son los mismos salvo el camino del módulo de los que se mudaron (`daemon::api::tests::x` pasa a `daemon::api::<dominio>::tests::x`). `diff` de las dos listas, con el segmento del dominio quitado, no muestra nada.
2. **Los cuerpos no cambian.** Cada `fn` de `api.rs` (208, tests y ayudantes incluidos) se comparó con la del archivo donde queda, línea a línea sin la sangría: son iguales salvo tres, que cambian el camino de `super::process::` a `crate::daemon::process::` porque ahora están un módulo más abajo (`register_session`, `import_memories` y `accept_job`). Ninguna aserción ni nombre de test se editó.
3. **Los tests de respuestas y los contratos.** `git diff` de `crates/clawg-cli/tests/api.rs` y de `crates/clawg-cli/src/mcp/` no muestra nada, y el de `agent_contract.json` y `agent_contract.es.json` tampoco. Los dos tests de contrato pasan sin `CLAWG_BLESS`. El único archivo de `tests/` que cambia es `conventions.rs` (ver abajo).
4. **Los caminos públicos.** `cargo check --workspace --all-targets` compila `clawg-eval`, `tests/api.rs`, `client.rs`, `mcp/` y `stop.rs` sin tocarlos: `router`, `router_on`, `serve_ephemeral`, `AppCtx`, `SAVE_CONFIRM_TIMEOUT` y `MAX_CONTENT_BYTES` quedan en `api.rs`, y `DocsSearchBody` y `REFRESH_GRACE` se reexportan de `api/docs.rs` con un `pub use` explícito.

### Los archivos

Líneas, y tests (`#[test]` y `#[tokio::test]`) que viven en cada uno:

| Archivo | Líneas | Tests |
|---|---:|---:|
| `api.rs` | 589 | 3 |
| `api/memories.rs` | 2.059 | 27 |
| `api/portability.rs` | 902 | 15 |
| `api/projects.rs` | 978 | 5 |
| `api/docs.rs` | 323 | 6 |
| `api/disputes.rs` | 213 | 1 |
| `api/sessions.rs` | 118 | 0 |
| `api/outbox.rs` | 65 | 0 |
| `api/test_support.rs` | 104 | 0 (el arnés) |

`api.rs` tenía 5.159 líneas, 3.290 de código y 1.870 de tests. Quedan en la raíz `AppCtx`, la forma del error, el router, las ayudas compartidas (`reader`, `named_reader`, `reach`, `ready_embedder`, `embed_failure`, `live_project`, `ReaderQuery`) y `health`, `shutdown`, `metrics` y `stats`.

### Lo que se resolvió distinto de lo escrito

- **`tests/conventions.rs`.** `no_production_code_outside_core_reaches_the_connection` cuenta como producción todo lo que está antes del `mod tests` de un archivo, y `test_support.rs` es todo de tests (`seed_memory` nombra `rusqlite`). El archivo empieza con `#![cfg(test)]` y la convención lo reconoce (`a_file_that_is_all_test_code_has_no_production_lines`, que se escribió antes y falló). Es el único cambio de `tests/` ([D21](design.md#d21-los-tests)).
- **El orden de los commits.** El cambio de `conventions.rs` entró en el commit de `sessions`, que es donde nace `test_support.rs`, para que ningún commit de la serie rompa esa convención.
- **Los tipos privados en una firma.** Un tipo privado que aparece en la firma de un handler `pub(super)` pasó a `pub(super)` con él, como dice D20 (los de `projects.rs`, `portability.rs` y `memories.rs`); ningún item `pub` cambió de visibilidad.
- **Las citas.** Además de las de `crates/clawg-cli/src/daemon/api.rs`, se reescribieron las del nombre corto `api.rs` y los números de línea sueltos (`api.rs:1234`, `:1234`) de `docs/reference/` y `openspec/specs/`: las de `check_content`, `check_operation_id` y `replaced_list`, que ya eran del núcleo desde la parte 1, apuntan a `crates/clawg-core/src/memory/ingest/save.rs`. Los documentos de `docs/completed-impl/` y los cambios archivados son historia y no se tocaron.

### Pendiente de la parte 4

- **Al archivar:** nada de la parte 4 va a `specs/`: R16 es de conservación y no deja una regla nueva.
- **El CI del PR.** Corre `test (windows-latest)` y `test (ubuntu-latest)`; la parte no agrega código `cfg(windows)`.
