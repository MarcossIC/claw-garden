# project-locators — Verificación

## Entrega 1 (0.64.0)

### Windows

- Con la versión en 0.64.0 y después de `cargo clean --workspace --target-dir target-test`, `cargo test --workspace --locked --target-dir target-test` pasa, con `tests/binary.rs`. `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` salen con 0, los dos en `target-test`.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin volver a bendecir (PL10).
- Qué se vio fallar antes de cada cambio:
  - **Tarea 4:** seis de los siete tests de `project.rs` fallaban. `a_caller_that_derives_its_id_never_follows_a_move` ya pasaba: protege lo que el cliente anterior hacía. `a_folder_moved_without_origin_registers_into_its_project` (`tests/api.rs`) no se corrió antes del cambio.
  - **Tarea 5:** `an_import_records_its_folders_as_a_registration_would` fallaba, porque las carpetas que escribía el import no tenían proyecto ni huellas, y la raíz movida se leía con su propio id. `an_import_reads_the_first_commit_off_the_store_s_thread` se escribió después del cambio. Se comprobó que falla (los dos `ThreadId` son el mismo) volviendo el plan, por un momento, a leer el disco en la conexión.
  - **Tarea 6:** los tres tests de `tests/api.rs` fallaban (405 al listar, 404 sin cuerpo al renombrar). Los de `registry.rs` no compilaban y no se corrieron antes.
  - **Tarea 7:** los tests se escribieron antes que el código y no compilaban, pero no se corrieron antes.
  - **Tareas 1 a 3:** este registro no guarda qué falló en RED.
- Durante la tarea 5 apareció un problema de la tarea 3: el plan del import corría `git` en el hilo de la conexión, hasta 1 s por raíz, y todo el store esperaba detrás. Se arregló en `798eb7c` con `ReadAhead`.

### El binario real en un home temporal

Se usó el binario de `target-test` con `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`, `CLAWG_TEST_DEFAULT_PORT=off` y `clawg serve --profile dev --port 0`. Sirvió sólo por TCP, porque el camino del directorio temporal no entra en un socket. Los repositorios eran reales, hechos con `git`:

1. Un repositorio `app` sin `origin` y con un commit. `clawg projects add` le da `app-8b6d`, y se guarda una memoria bajo él.
2. `mv app app2` y `projects add app2` dan `app-8b6d` otra vez. El log nombra el camino viejo y el proyecto (PL5).
3. Un repositorio nuevo en `app`, con otro primer commit, recibe un proyecto propio, `app-b45f`. El log avisa que el camino tiene un repositorio que no es ninguno de los grabados (PL4).
4. `git clone app2 copia` (su `origin` es un camino, así que no tiene clave) recibe `copia-85a9` y una sugerencia `root_commit` con `app-8b6d`, que queda en el log (PL5, PL6). Una carpeta `gone` se registra y después se borra.
5. `clawg projects list` muestra:
   - `app-8b6d` con su memoria y dos carpetas: la de `app`, que figura como que ya no está porque ahora tiene otro repositorio, y la de `app2`;
   - la sugerencia en los dos proyectos del par;
   - dos proyectos llamados `app`, que se distinguen por el id (D8);
   - la carpeta de `gone`, que ya no está;
   - cada proyecto marcado sin revisar, con cómo nació, y la línea que nombra `rename`.
6. `projects rename app-8b6d "  App web "` lo deja como `App web` y revisado. `projects rename app-b45f "app WEB"` se rechaza con salida 1, nombrando el id y la causa (PL9).
7. `clawg memories list --here` desde `app2` lee la memoria bajo `app-8b6d`.

`clawg stop` terminó el daemon con salida 0.

### CI

Esta entrega no tuvo pull request propio. La cubre el CI de los pull requests #95, #96 y #97 (a `develop`), que pasó en Linux y Windows sobre lo unido hasta cada uno. `project-locators` se publica entero, sin una release por entrega, así que no se abre un pull request de develop a main al cerrar cada una.

### Lo que no se corrió

- **Linux en local:** sólo se corrió en Windows. Lo cubre el CI del pull request.
- **El perfil dev:** no se tocó ningún perfil real.

## Entrega 2a (0.64.0)

### Windows

- Después de `cargo clean --workspace --target-dir target-test` al empezar la entrega, `cargo test --workspace --locked --target-dir target-test` pasa, con `tests/binary.rs`. `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` salen con 0, los dos en `target-test`.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin volver a bendecir (PL20).
- `a_merge_waits_for_the_saves_of_its_projects` tarda unos 5 s, lo que espera `SAVES_WAIT` antes de responder `saves_pending`.

### El binario real en un home temporal

Se usó el binario de `target-test` con `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`, `CLAWG_TEST_DEFAULT_PORT=off` y `clawg serve --profile dev --port 0`, con repositorios hechos con `git` y las memorias guardadas por HTTP con el token del perfil:

1. `app`, sin `origin`, y `web`, con `origin` `github.com/acme/web`, se registraron con `projects add` como `app-3794` y `web-f452`, con una memoria cada uno y un `workspace.md` de `claude-code` en `app`.
2. `projects merge app-3794 --into web-f452 --dry-run` dijo lo que iba a pasar, sin escribir. La unión de verdad pasó 1 memoria y la carpeta de `app`, copió el `workspace.md`, escribió el snapshot `pre-merge` y dio el número 1 con su `--undo`.
3. `projects list` mostró sólo `web-f452`, con las dos carpetas, su remoto y la línea de `app-3794` unido, con el comando que lo deshace.
4. `memories list --here` desde la carpeta de `app` leyó las dos memorias bajo `web-f452`. Un save con el id `app-3794` quedó bajo `web-f452`.
5. `projects rename app-3794` y `projects merge web-f452 --into app-3794` se rechazaron con salida 1, nombrando `web-f452`. `projects move --from-project app-3794` también, pero sólo con el código (`daemon returned 409: project_merged`); se arregló después, en `move_refused` (ver abajo).
6. `projects merge --undo 1` devolvió la memoria a `app-3794` y sacó la copia de `workspace.md`, con el snapshot `pre-undo`. El save hecho con el id viejo después de la unión siguió en `web-f452`, y los dos proyectos volvieron a estar sin revisar, como antes de la unión. La carpeta que la unión había creado para la copia quedó vacía: se arregló en `3df68b1`, y el test la comprueba.
7. `git clone app copia` y `projects add copia` dejaron la sugerencia `root_commit` entre `copia` y `app`, y la lista terminó con los dos comandos que la responden. Después de `projects keep-apart copia-59b1 app-67e3`, y de registrar `copia` otra vez, la sugerencia ya no se listó. Este paso se repitió en un segundo home, porque en el primero el guion no encontró `rg`.
8. Con otro repositorio llamado `app` registrado, `doctor` dio `project_duplicates` `same_name` con el paso `clawg projects merge app-bdb6 --into app-3794 --dry-run --profile dev`, y `project_orphans` y `project_merges` en `ok`. El par `app` y `copia`, con el mismo primer commit, no apareció, porque estaba respondido `apart`. `doctor` salió con 1 por `path` `shadowed`: en el PATH de esa máquina hay otro `clawg.exe` antes que el de la prueba.

`clawg stop` terminó el daemon con salida 0.

### Lo que la verificación hizo cambiar

- **`3df68b1`:** el `--undo` dejaba vacía la carpeta que la unión había creado para la copia de `workspace.md`. Ahora `drop_adopted` la borra cuando no tiene nada más. `undoing_a_merge_leaves_the_store_as_it_was` falló con esa aserción antes del arreglo.
- **`move_refused`:** un movimiento rechazado con una causa en `detail` (`project_merged`, `part_of_merge`) ahora la muestra. `projects_are_merged_and_kept_apart_through_the_daemon` no compilaba sin la función.

### CI

Esta entrega no tuvo pull request propio. La cubre el CI de los pull requests #95, #96 y #97 (a `develop`), que pasó en Linux y Windows sobre lo unido hasta cada uno.

### Lo que no se corrió

- **Linux en local:** sólo se corrió en Windows. Lo cubre el CI del pull request final.
- **El perfil dev:** no se tocó ningún perfil real.

### Qué se vio fallar antes de cada cambio

- **Tarea 11:** los tests de `merge.rs` y `a_database_from_the_previous_version_opens_with_every_project_live` no compilaban, porque faltaban `canonical` y `MAX_MERGE_HOPS`. `published_migration_checksums_are_pinned` falló con el checksum 0, y se fijó el que imprimió. Los `*.sql` van con `eol=lf` (`.gitattributes`), así que el checksum es el mismo en Windows y en Linux. Tres tests de `db.rs` que fijan cuántas migraciones hay (27) fallaron con V28, y pasaron a 28.
- **Tarea 12:** los tests se escribieron antes que el código, pero no se corrieron antes porque no compilaban sin `merge_core`. Después del cambio se comprobó que atrapan un error, rompiendo dos reglas a propósito:
  - con `MAX` en lugar de `MIN` en `synthesis_review`, falla `merging_moves_every_row_and_locator_into_the_target`;
  - con `Span::Keep` en lugar de `Span::Carry`, falla `a_merge_moves_the_part_of_a_split_unit_it_holds`.
- **Tarea 13:** los cuatro tests de `merge.rs` se corrieron antes del cambio y no compilaban: faltaban `undo_merge_core`, las variantes `UnknownMerge`, `Undone` y `MergedLater`, y los campos `kept` y `removed`.
- **Tarea 14:** los cinco tests fallaban antes del cambio, por la aserción:
  - el save quedaba bajo la lápida;
  - el movimiento y su `--undo` llevaban la fila a la lápida;
  - el registro, la lectura y el plan daban el id de la lápida;
  - el import escribía bajo el proyecto que se unió entre el plan y la escritura. Para eso, el test usa un embedder que marca la lápida en su primera llamada.

  La suite entera encontró un caso más: `export_reads_the_project_id_from_the_store` falló con `no such column: merged_into`. `project_id_of` (`clawg memories --here`, `clawg export --project`) lee sin migrar stores de versiones anteriores, donde `project` no tiene esa columna. Se arregló con una vista temporal `project` sin lápidas, como las que ese código ya les prestaba, y el test pasó a cubrir también un store en V27.
- **Tarea 15:** antes del cambio fallaban:
  - `a_move_from_a_merged_project_is_refused` y `the_move_of_a_merge_is_undone_only_with_its_merge`, con 200 en lugar de 409;
  - `a_bridge_opened_before_a_merge_reads_and_writes_its_target`, porque la búsqueda con el id de la lápida no encontraba nada;
  - `every_route_that_takes_a_project_follows_a_merge`, en `GET /memories`.

  `every_route_says_which_project_it_takes` falló primero por un error del propio test: el fuente de `api.rs` llega con CRLF en Windows. El chequeo de `POST /context` se reescribió después del cambio, sin consulta, porque con el embedder falso los hits de una consulta puntúan 0 y el contexto los deja afuera. Por eso se comprobó aparte que atrapa el error: sacando por un momento `live_project` del handler, fallan ese test y el del puente.
- **Tarea 16:** los tres tests no compilaban antes del cambio: faltaban `keep_apart`, `KeptApart`, `KeepApartError`, `ListedMerged` y el campo `merged`. Después, `a_merged_project_is_listed_under_its_target_and_frees_its_name` falló por su propio fixture, que armaba la unión sin la fila `tombstoned` del diario que una unión real escribe.
- **Tarea 17:** antes del cambio, los tres tests de las rutas y el recorrido del router fallaban con 405, y `every_route_says_which_project_it_takes` fallaba porque la tabla nombraba dos rutas que `router()` todavía no tenía.
- **Tarea 18:** los tests de `cli.rs` y `projects.rs` no compilaban antes del cambio: faltaban `ProjectsAction::Merge`, `KeepApart`, `MergeRequest` y `render_merge`. `projects_are_merged_and_kept_apart_through_the_daemon` se agregó después, para cubrir el cliente contra un daemon real, y no se corrió antes.
- **Tarea 19:** los tests de `checks.rs` y de `inspect.rs` no compilaban antes del cambio: faltaban los tres chequeos, `RegistryRow`, `FolderPrint`, `IdRows` y los campos nuevos de `StoreFacts`.

## Entrega 2b (0.64.0)

### Windows

- `cargo test --workspace --locked --no-fail-fast --target-dir target-test` pasa en `d080e8a`, con `tests/binary.rs`: 19 binarios, ninguno con fallos. `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` salen con 0, los dos en `target-test`. No consta que se haya corrido `cargo clean --workspace --target-dir target-test` al empezar la entrega: la versión no cambió desde la 2a.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin volver a bendecir (PL31).
- `a_merge_waits_for_the_saves_of_its_projects` falló una vez con la máquina cargada (mide una espera de 300 ms), y pasó al correrlo solo, sobre el árbol sin los cambios de esta entrega y en la corrida completa. Es un test inestable de la 2a, no de este cambio.

### El binario real en un home temporal

Se usó el binario de `target-test`, copiado fuera del directorio de build, con `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`, `CLAWG_TEST_DEFAULT_PORT=off` y `clawg serve --profile dev --port 0`. Sirvió sólo por TCP, porque el camino del directorio temporal no entra en un socket. Los repositorios `vacio`, `app`, `otro` y `app2` se hicieron con `git` y se registraron con `projects add`. Los tres primeros salieron con el mismo primer commit, porque se crearon en el mismo segundo con commits vacíos iguales: eso dejó sugerencias `root_commit` entre ellos. Las memorias se guardaron por HTTP con el token del perfil.

1. `projects delete vacio-e2d6` borró el proyecto vacío como borrado 1, con su `--undo`. `projects list` dejó de listarlo y de listar sus sugerencias, y terminó con el borrado abierto. Borrarlo otra vez salió con 1 y nombró el borrado y su `--undo`. `projects delete --undo 1 --dry-run` dijo lo que iba a hacer, y el `--undo` lo dejó vivo, con el índice de docs encendido y el snapshot `pre-undo`.
2. Con tres memorias en `app-f619`, `projects delete app-f619` salió con 1, contó las filas vivas y nombró los candidatos y las otras formas de vaciarlo.
3. Con una memoria borrada antes, `--with-memories` dio el informe y el código. Un save en el medio hizo que el código viejo respondiera `code_changed` con el informe nuevo, sin borrar nada (salida 1). El código nuevo borró 3 memorias como borrado 2, con el snapshot `pre-delete`.
4. `hook session-start` y `hook user-prompt-submit` en la carpeta de `otro`, borrado, registraron la raíz y la sesión (2xx en las métricas) sin revivirlo. Un save con el id de `app-f619`, como el de un puente abierto antes del borrado, lo hizo volver sin revisar, con esa memoria sola. Sus memorias borradas siguieron borradas, y el borrado 2 siguió en la lista, con la línea de que un save lo trajo de vuelta.
5. `projects delete --undo 2` devolvió las tres memorias que borró, dejó borrada la que ya estaba borrada antes y escribió el snapshot `pre-undo`. La búsqueda volvió a encontrarlas.
6. `app-f619` se borró otra vez con sus 4 memorias (borrado 4). `purge --project-delete 4` sin `--yes` salió con 1, y con el daemon corriendo pidió `clawg stop`. Después de `clawg stop`, la purga sacó las 5 memorias del conjunto, contando la que estaba borrada antes del borrado (D34), y nombró los dos snapshots `pre-delete` y el `pre-undo`, que todavía guardan los textos. Una palabra de una memoria ya no aparece en `memory.db`, que quedó sin `-wal`. Purgar otra vez el borrado 4 o purgar el 1, deshecho, salió con 1 y la causa.
7. `doctor` dio `project_deletes`, `project_duplicates`, `project_orphans` y `project_merges` en `ok`. Salió con 1 por dos hallazgos del entorno, como en la 2a: `path` `shadowed`, porque en el PATH de esa máquina hay otro `clawg.exe` antes, y `socket` `path_unusable`, porque el camino del home temporal no entra en un socket.

`clawg stop` terminó el daemon con salida 0.

### Lo que la verificación hizo cambiar

- **`d78af0e`:** la salida de `not_empty` y la del informe repetían cada candidato. Un proyecto con el mismo primer commit es a la vez un par `same_first_commit` de `duplicates` y una sugerencia pendiente `root_commit`, y la CLI traduce los dos motivos a la misma frase. Ahora nombra cada proyecto una vez, con sus motivos distintos. `a_candidate_offered_for_two_reasons_is_named_once` falló con las líneas repetidas antes del arreglo. Con el binario nuevo, un clon de `vacio` (`copia`) salió una sola vez como candidato.

### CI

Esta entrega no tuvo pull request propio. La cubre el CI de los pull requests #95, #96 y #97 (a `develop`), que pasó en Linux y Windows sobre lo unido hasta cada uno.

### Lo que no se corrió

- **Linux en local:** sólo se corrió en Windows. Lo cubre el CI del pull request final.
- **El perfil dev:** no se tocó ningún perfil real.

### Qué se vio fallar antes de cada cambio

- **Tarea 22:** la escribió una sesión que se cortó antes de verificarla, y no consta qué falló en RED. Al correr la suite, tres tests de `db.rs` que fijan cuántas migraciones hay (28) fallaron con V29, y pasaron a 29. El checksum de V29 en `published_migration_checksums_are_pinned` ya era el real.
- **Tarea 23:** `duplicates_pair_by_name_and_by_first_commit_and_skip_answered_pairs` no compilaba: faltaban `duplicates` y `live_of`.
- **Tareas 24 a 27:** los tests no compilaban antes del cambio. Faltaban `delete_core` y sus tipos (24), su `confirm` y su `backups` (25), `undo_delete_core`, `KeepReason` y las variantes de error del `--undo` (26), y las variantes `Deleted` y `list_deletes` (27).
- **Tarea 28:** los tests nuevos y la fila de la tabla fallaban antes de que existiera la ruta.
- **Tarea 29:** los tests no compilaban. Después, `projects_delete_parses` falló una vez por una regla de clap: `requires = "undo"` en `--dry-run` no rechazaba `app-1a2b --dry-run`, y se cambió por `conflicts_with_all`.
- **Tarea 30:** los tests no compilaban: faltaban `purge_delete`, `LiveRow` y las tres variantes de `PurgeError`.
- **Tarea 31:** los tests se escribieron antes que el código, pero no se corrieron por separado. No podían pasar sin el cambio, porque usan `--project-delete`, `confirm_delete` y `purge_delete_report`, que no existían.
- **Tarea 32:** con `cargo test --no-run`, los tests no compilaban: faltaban `OpenDelete`, `project_deletes`, los conteos de filas vivas de `IdRows`, `RegistryRow::deleted_at` y `StoreFacts::open_deletes`. Después se corrigió el propio fixture: una fila viva insertada bajo un proyecto lo hace volver, así que el proyecto tiene que marcarse borrado después de las filas.

## Entrega 3a (0.64.0)

### Linux

- `cargo test --workspace --locked --no-fail-fast` pasa en `1fdde90`, con `tests/binary.rs`: 19 binarios, ninguno con fallos. `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` salen con 0. Es la primera vez que este cambio corre en Linux.
- El entorno no alcanza `cdn.pyke.io`, de donde `ort-sys` baja ONNX Runtime al compilar. La biblioteca se tomó de la rueda `onnxruntime==1.24.2` de PyPI (la versión que pide `ort-sys` 2.0.0-rc.12), con `ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1` y `LD_LIBRARY_PATH`: sólo cambia cómo se enlaza en esa máquina, no el repositorio. El build de release y el CI la siguen bajando como siempre.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` se volvieron a bendecir, y el diff sólo suma los argumentos de PL34 y la herramienta de PL37 (PL41).

### El binario real en un home temporal

Se usó el binario de `target/debug`, copiado fuera del directorio de build, con `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`, `CLAWG_TEST_DEFAULT_PORT=off` y `clawg serve --profile dev --port 0`. El puente (`clawg mcp --no-autostart --agent claude-code`) se manejó por stdio real, desde una carpeta `notas` sin VCS.

1. `save_memory` con `project: "ideas de producto"` creó `ideas-de-producto-2701` y guardó en él. Otro save con `Ideas-de-Producto` se negó sin guardar y nombró a ese proyecto como parecido, con `new_project`. `search_memory` con el nombre encontró la memoria, y `search_project` dijo que el proyecto no tiene carpeta.
2. `projects create Roadmap` creó `roadmap-d258`, revisado. `projects create road-map` salió con 1, nombró a `Roadmap` y el `--new`. `projects locate Roadmap work/plan` registró la carpeta y unió su proyecto `plan-3c02` en `Roadmap` (unión 1), con su `--undo`.
3. La carpeta `notas`, con una memoria, se movió a `archivo/notas`. `hook session-start` en la carpeta nueva terminó con la línea de la sugerencia. `recall_context` desde un puente en esa carpeta terminó con la línea `folder_name` que nombra a `notas-02f7`, y `projects list` mostró la sugerencia en los dos proyectos.
4. `answer_suggestion` con `apart` respondió que siguen siendo dos proyectos, y el `recall_context` siguiente ya no nombró `answer_suggestion`.
5. `doctor` dio 22 `ok`, 2 `info`, ningún `warn` ni `error`.

`clawg stop` terminó el daemon.

### Windows

- Después de `cargo clean --workspace --target-dir target-test`, `cargo test --workspace --locked --no-fail-fast --target-dir target-test` pasa en `6ba4c52`, con la 3a ya unida en `develop` y con `tests/binary.rs`: 19 binarios, ninguno con fallos. `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` salen con 0, los dos en `target-test`.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan con los archivos bendecidos en Linux, sin volver a bendecir (PL41).
- `a_merge_waits_for_the_saves_of_its_projects` pasó en la corrida completa.

### El binario real en un home temporal, en Windows

Se usó el binario de `target-test`, copiado fuera del directorio de build, con `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`, `CLAWG_TEST_DEFAULT_PORT=off` y `clawg serve --profile dev --port 0`. Sirvió sólo por TCP, porque el camino del home temporal (175 bytes) no entra en un socket. El puente (`clawg mcp --no-autostart --agent claude-code`) se manejó por stdio real: primero desde una carpeta `notas` sin VCS, y después desde esa misma carpeta movida.

1. `save_memory` con `project: "ideas de producto"` creó `ideas-de-producto-c510` y guardó en él. Otro save con `Ideas-de-Producto` se negó sin guardar, nombró a ese proyecto como parecido y ofreció `new_project`. `search_project` con el nombre dijo que el proyecto no tiene carpeta en esta máquina. Un save sin `project` quedó en el proyecto de la carpeta, `notas-e91b`.
2. `search_memory` con el nombre del proyecto respondió `no relevant memories for this query: 1 searched`. Buscó en el proyecto, que tenía una memoria, pero con el embedder falso un hit sin identificador puntúa 0. Con una memoria que nombra `weekly_digest_job`, la misma búsqueda, por nombre y por id, la encontró como `exact`. Desde la carpeta y sin `project`, respondió `no memories found`.
3. `projects create Roadmap` creó `roadmap-65d0`. `projects create road-map` salió con 1 y nombró a `Roadmap` y el `--new`. `projects locate Roadmap work/plan` (un repositorio hecho con `git`) registró la carpeta y unió su proyecto `plan-d754` en `Roadmap` (unión 1), con su `--undo`.
4. La carpeta `notas` se movió a `archivo/notas` con el puente ya cerrado, porque en Windows no se puede mover la carpeta de trabajo de un proceso vivo. `hook session-start` en la carpeta nueva dio la línea de la sugerencia, que nombra a `notas-e91b`. `projects list` mostró la sugerencia `folder_name` en los dos proyectos, y la carpeta vieja como ausente.
5. `recall_context` desde un puente en `archivo/notas` terminó con la línea que nombra a `notas-e91b` y a `answer_suggestion`. `answer_suggestion` con `apart` respondió que siguen siendo dos proyectos. El `recall_context` siguiente ya no la nombró, y `projects list` mostró los dos proyectos revisados y sin la sugerencia.
6. `doctor` dio 20 `ok`, 2 `info` y ningún `warn`. Salió con 1 por dos hallazgos del entorno, como en la 2a y la 2b:
   - `path` `shadowed`: en el PATH de esa máquina hay otro `clawg.exe` antes;
   - `socket` `path_unusable`: el camino del home temporal es demasiado largo.

`clawg stop` terminó el daemon con salida 0, las dos veces.

### Lo que la verificación hizo cambiar

- **El tope del hook.** La línea de las sugerencias en `session-start` no entraba en los 1.000 bytes que el hook podía sumar: el peor caso de antes ya ocupaba 978. `session-start` da una sola línea, que nombra hasta dos proyectos, y el tope pasó a 1.500, que con los 8.000 del contexto sigue entrando en los 10.000 caracteres del host (D45). `what_the_hook_adds_stays_small` falló con 1.389 bytes antes del cambio.
- **`locate` sin prueba en seco.** La raíz se registra antes de saber a qué proyecto lleva, y el `--undo` de la unión ya es la vuelta atrás (PL39).
- **Una respuesta inválida.** `answer: "maybe"` daba el 422 de axum. Ahora es un 400 que nombra los valores, como las demás rutas.

### CI

El CI del pull request #95 (a `develop`, en `a9a0cda`) pasó en Linux y Windows.

### Lo que no se corrió

- **El socket en el recorrido de Windows:** el home temporal sirvió sólo por TCP. El socket en Windows lo cubren los tests de `tests/binary.rs`.
- **El perfil dev y el de producción:** no se tocó ningún perfil real. El paso operativo de colocar las filas sin proyecto de producción sigue pendiente y pide autorización.

### Qué se vio fallar antes de cada cambio

- **Tarea 35:** los tests no compilaban: faltaban `name_key`, `new_project_id`, `resolve_project` y sus tipos, y `find_project`.
- **Tarea 36:** con `named_after_in` apagado, `a_folder_named_as_a_project_whose_folders_are_gone_is_suggested` falló sin la sugerencia, y `a_folder_named_as_a_project_still_there_is_not_suggested` pasó, como debe.
- **Tarea 37:** los tests no compilaban: faltaban `pending_suggestions` y `suggestion_between`.
- **Tarea 38:** con los tests escritos y sin las rutas, `a_name_two_projects_hold_is_refused_as_a_target` falló, y las demás de la tanda no tenían ruta que llamar.
- **Tareas 39 y 41:** los tests del puente se escribieron después del código, en el mismo paso; los de los contratos, la lista de herramientas de `.claude/CLAUDE.md`, la tabla en español y el protocolo fallaron hasta que se completaron.
- **Tarea 40:** `session_start_names_the_pending_suggestions` no compilaba sin la línea; después `what_the_hook_adds_stays_small` falló por el tope (arriba).
- **Tarea 42:** los tests no compilaban: faltaban `ProjectsAction::Create`, `ProjectsAction::Locate` y `render_created`.

## Entrega 3b (0.64.0)

### Linux

- `cargo test --workspace --locked` pasa en `663e83d`, con `tests/binary.rs`: ningún binario con fallos. `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings`, `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` y `cargo test --locked -p clawg-core --lib build_info` salen con 0.
- ONNX Runtime se enlazó con la rueda `onnxruntime==1.24.2` de PyPI, como en la 3a (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1`, `LD_LIBRARY_PATH`), porque el entorno no alcanza `cdn.pyke.io`. No cambia nada del repositorio.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin volver a bendecir (PL52).

### El binario real en un home temporal

Se usó el binario de `target/debug`, copiado fuera del directorio de build, con `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`, `CLAWG_TEST_DEFAULT_PORT=off` y `clawg serve --profile dev --port 0`, que sirvió por el socket y por TCP. Los repositorios se hicieron con `git`: un monorepo con `origin` `https://github.com/acme/monorepo.git`, un clone suyo en otro camino con el mismo `origin`, y un worktree del monorepo. El puente (`clawg mcp --no-autostart --agent claude-code`) se manejó por stdio real.

1. `projects create web` creó `web-0680`. `projects locate web <monorepo>/apps/web --subfolder` dijo que la carpeta lleva a `web-0680` dentro del repositorio de `monorepo-58c9`, nombró el `projects move --from-project monorepo-58c9 --to-project web-0680 --text WORDS --dry-run` y que una sesión abierta antes sigue con `monorepo-58c9`.
2. Un puente abierto en `apps/web/src` guardó en `web-0680`, y uno abierto en la raíz guardó en `monorepo-58c9` (`memories list`).
3. `hook session-start` leyó la memoria de `web-0680` y no la del monorepo desde `apps/web/src`, desde el `apps/web/src` del clone y desde el `apps/web` del worktree; desde la raíz del monorepo y la del clone leyó la del monorepo y no la de `web`.
4. `projects list` mostró la declaración bajo el monorepo (`subfolder apps/web leads to web`) y bajo `web`, con sus caminos en las dos carpetas del repositorio.
5. `projects locate monorepo-58c9 <monorepo>/apps/web --subfolder` la quitó, nombró `clawg projects merge web-0680 --into monorepo-58c9 --dry-run`, y lo guardado en `web-0680` siguió allí. `session-start` desde `apps/web/src` volvió a leer el monorepo, y `projects list` mostró `web` sin carpeta.
6. `doctor` dio `project_subfolders` `ok`, 22 `ok`, 2 `info` y un `warn` del entorno (`backup.age`: el home temporal no tiene snapshot). Nada nuevo.

`clawg stop` respondió que el daemon seguía vivo después de 30 s. El log muestra `clawg serve dev stopped` en el mismo segundo del pedido, y no quedó ningún proceso: el script que manejaba la prueba esperaba al hijo `serve` recién después de `stop`, así que el pid seguía como zombi sin cosechar. No es del daemon.

### Windows

- Después de `cargo clean --workspace --target-dir target-test`, `cargo test --workspace --locked --no-fail-fast --target-dir target-test` pasa en `7cfe81b` (0.65.0, con la 3b y la 4 ya unidas en `develop`), con `tests/binary.rs`: ningún binario con fallos. `cargo clippy --workspace --all-targets --locked --target-dir target-test -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings` salen con 0.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin volver a bendecir (PL52).
- `a_merge_waits_for_the_saves_of_its_projects` pasó en la corrida completa.
- `cargo clean --workspace` no saca los objetos de builds anteriores del workspace que quedan en `target-test/debug/deps` (`.rcgu.o`, `.exp` y `.lib` de `clawg*`, unos 674 MB), porque cargo no los registra. No cambian el resultado.

### El binario real en un home temporal, en Windows

Se usó el binario de `target-test`, copiado fuera del directorio de build, con `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`, `CLAWG_TEST_DEFAULT_PORT=off` y `clawg serve --profile dev --port 0`. Sirvió sólo por TCP, porque el camino del socket en el home temporal (172 bytes) no entra en `sun_path`. Los repositorios se hicieron con `git`: un monorepo con `origin` `https://github.com/acme/mono.git`, un clone suyo en otro camino con el mismo `origin`, y un `git worktree` del monorepo, cuyo `.git` git escribió como `gitdir: C:/…/mono/.git/worktrees/wt-mono`. El puente (`clawg mcp --no-autostart --agent claude-code`) se manejó por stdio real.

1. `projects create web` creó `web-0727`. `projects locate web <mono>\apps\web --subfolder` dijo que la carpeta lleva a `web-0727` dentro del repositorio de `mono-c05e`, nombró el `projects move … --dry-run` y dijo que una sesión abierta antes sigue con `mono-c05e`.
2. Un puente abierto en `apps\web\src` guardó en `web-0727`, y uno abierto en la raíz guardó en `mono-c05e` (`memories list`).
3. `hook session-start` leyó `web` desde el `apps\web\src` del monorepo y del clone, y desde el `apps\web` y el `apps\web\src` del worktree. Desde las tres raíces y desde `apps\api` leyó el monorepo.
4. `projects list` mostró `subfolder apps/web leads to web (web-0727)` bajo el monorepo, y la declaración bajo `web`, con sus caminos en el monorepo y en el clone. El worktree no es una carpeta propia, como en Linux.
5. `projects locate mono-c05e <mono>\apps\web --subfolder` la quitó y nombró `clawg projects merge web-0727 --into mono-c05e --dry-run`. Lo guardado en `web` siguió allí, los tres caminos de `web` volvieron a leer el monorepo, y `projects list` mostró `web` sin carpeta.
6. `doctor` dio `project_subfolders` `ok`, con 20 `ok`, 2 `info`, 1 `warn` y 2 `error`. Todos son del entorno: `path` `shadowed` (en el PATH de esa máquina hay otro `clawg.exe` antes), `socket` `path_unusable`, `store.owner` y `mcp` (`info`), y `backup.age` (`warn`: el home temporal no tiene snapshot, como en Linux). Nada nuevo.

En otro home se probaron las formas de escribir un camino en Windows, con la declaración `apps/web → web`:

- `projects locate … --subfolder` respondió que la carpeta ya lleva a `web`, sin escribir nada, con cada una de estas formas: `apps\web` y `apps/web` relativos, `..\web` desde `apps\api`, barras `/`, la unidad en minúscula, el camino en mayúsculas, un `\` final, un camino `\\?\` bien formado, el camino corto 8.3 y el camino a través del worktree. La raíz del checkout y la del worktree salieron con 1 («top of a checkout»); un archivo o una carpeta que no existe, con «is not a directory».
- `session-start` leyó `web` con cada una de esas formas como `cwd`, también relativa y con segmentos `..`. Los puentes abiertos con la unidad en minúscula, en mayúsculas o con el camino 8.3 guardaron en `web`.
- Un registro por HTTP con la raíz del monorepo y un `path` con la unidad en minúscula, barras `/`, `\\?\` o mayúsculas respondió `web`. Uno relativo, o bajo otra raíz, respondió el monorepo, porque se deja fuera (PL44).

`clawg stop` terminó el daemon con salida 0 cada vez.

### Lo que la verificación encontró

- **Un camino `\\?\` mal formado.** Con `{"cwd": "\\\\?\\C:\\…\\mono\\apps/web/src"}` (el prefijo verbatim con `/` después), `session-start` lee el monorepo en vez de `web`, y el monorepo queda con una segunda carpeta `\\?\C:\…\mono`.
  - La causa: en un camino `\\?\` la `/` no separa, y `fs::canonicalize` falla. El respaldo de `canonical_form` (`crates/clawg-core/src/memory/identity.rs:82-87`), `normalize_lexical(absolute(path))`, sí llega al directorio, pero no llama a `strip_verbatim`. La raíz conserva el prefijo, se graba como otra carpeta y el subcamino queda fuera de ella.
  - Lo mismo pasa con un cliente HTTP propio que manda una raíz con los separadores mezclados: `POST /v1/projects` toma `root_path` como llega, y cada forma se graba como otra carpeta. El puente, los hooks y la CLI mandan siempre la raíz canónica, y ni Claude Code ni una shell dan un `cwd` así.
  - El respaldo es anterior al cambio (`a556e90`). Queda como arreglo suelto en el backlog del [README](../../../README.md#backlog-sin-spec-arreglos-sueltos).

### CI

El CI del pull request #96 (a `develop`, en `420a36f`) pasó en Linux y Windows. El pull request de `develop` a `main` lo vuelve a correr, sobre el cambio entero, antes del release.

### Lo que no se corrió

- **El socket en el recorrido de Windows:** el home temporal sirvió sólo por TCP. El socket en Windows lo cubren los tests de `tests/binary.rs`.
- **El perfil dev y el de producción:** no se tocó ningún perfil real.

### Qué se vio fallar antes de cada cambio

- **Tarea 45:** sin V30, `a_database_from_the_previous_version_opens_with_no_subfolder_declared` y los tres conteos de migraciones fallaron; con el archivo y sin fijarlo, `published_migration_checksums_are_pinned`.
- **Tareas 46 a 53:** los tests no compilaban sin `subpath_of`, `Ground::subpath`, `ProjectIdentity::path`, el módulo `subfolder`, `Registration::subpath`, `ListedSubfolder`, `DeleteReport::subfolders`, `RegisterRequest::path`, `LocateRequest::subfolder`, `--subfolder` y `DeclaredSubfolder`. Los de `render_list` fallaron una vez por el orden de las líneas, que quedó después de los remotos.


## Entrega 4 (0.64.0)

### Linux

- `cargo test --workspace --locked` pasa en `9098632`, con `tests/binary.rs`: ningún binario con fallos. `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings`, `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` y `cargo test --locked -p clawg-core --lib build_info` salen con 0.
- ONNX Runtime se bajó de `cdn.pyke.io` al compilar, sin rueda de PyPI.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin volver a bendecir (PL65).

### El binario real en dos homes temporales

Se usó el binario de `target/debug`, copiado fuera del directorio de build, con `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`, `CLAWG_TEST_DEFAULT_PORT=off` y `clawg serve --profile dev --port 0`. El camino del home temporal no entra en `sun_path`, así que el daemon sirvió por TCP solo. Los repositorios se hicieron con `git`: `app` con `origin` `https://github.com/acme/app.git` y `mono` con `https://github.com/acme/mono.git`, copiados en otro camino para el home B, con el mismo primer commit. Los saves y los registros se mandaron por HTTP con el token.

1. En A: `projects create ideas`, `projects create web` y `projects locate web <mono>/apps/web --subfolder`; un save en `ideas`, dos en `app`, uno en `web` (registrado con el camino crudo `apps/web/src`) y uno en `mono`. `clawg export` escribió 5 memorias y 4 proyectos, dijo «2 projects without a folder», y las líneas `project` llevaron nombre, carpetas con remoto y primer commit, remotos y la declaración de `mono`.
2. En B, con los clones registrados: `import --dry-run` y `import` dieron el mismo reporte; `app` y `mono` fueron a los proyectos de los clones `by remote`, `ideas` y `web` se crearon con su id y su nombre, y la declaración `apps/web → web` se escribió. `projects list` la mostró en los dos lados, y el registro del clone con el camino crudo `apps/web/src` respondió `web`.
3. `projects move --id 2 --to-project ideas` y otra vez `import`: «imported 0 of 5», 4 ya presentes y «already here, moved to another project: ideas 1»; `ideas` y `web` fueron `by id`, y la declaración quedó como estaba (`declared_here`).
4. Después de `projects move --undo 1`, `import --undo 1` retiró las 5 memorias, las carpetas grabadas de A, los dos proyectos creados y la declaración; `projects list` quedó como antes del import.
5. Un export de engram con `--project-map board=project:notas` dejó su nota en `notas`. Con `notas` borrado, el mismo import por su id respondió 409 `project_deleted`, y por su nombre 404, porque el nombre de un proyecto borrado queda libre (PL27).
6. `doctor` dio un `error` `socket` `path_unusable` (el camino largo del home temporal) y dos `info` del entorno (`store.owner`, `mcp`). Nada nuevo.

### Windows

- La misma corrida de la 3b, en `7cfe81b`: la suite y los dos clippy pasan en `target-test`, y los contratos del modelo pasan sin volver a bendecir (PL65).

### El binario real en dos homes temporales, en Windows

Como en Linux, con el binario de `target-test` copiado fuera del directorio de build y los mismos `CLAWG_TEST_*`, por TCP solo. Los homes A y B corrieron uno después del otro. Los repositorios se hicieron con `git`: `app` y `mono`, con `origin` `https://github.com/acme/app.git` y `https://github.com/acme/mono.git`, y un clone de cada uno en otro camino para el home B.

1. En A: `projects create ideas` (`ideas-7033`), `projects create web` (`web-136d`) y `projects locate web <monoA>\apps\web --subfolder`. Se guardaron cinco memorias: una en `ideas` por su nombre (`project: "ideas"`, desde el puente), dos en `app` desde un puente en su repositorio, una por HTTP después de registrar el camino crudo `<monoA>\apps\web\src` (que respondió `web-136d`) y una en `mono`. `clawg export` escribió 5 memorias y 4 proyectos y dijo «2 projects without a folder». Las líneas `project` llevaron el nombre, las carpetas con su `root_path` con `\`, su remoto y su primer commit, los remotos y la declaración de `mono` (`subfolders: [{apps/web, web-136d}]`).
2. En B, con los clones registrados (`app-3eed`, `mono-3629`): `import --dry-run` e `import` dieron el mismo reporte, salvo la línea del snapshot y el número del import. Importaron 5 de 5; `app` y `mono` fueron `by remote`, `ideas` y `web` se crearon con su id y su nombre, y la declaración `apps/web → web-136d` se escribió en `mono-3629`. `projects list` la mostró en los dos lados, y el registro del clone con el camino crudo `…\apps\web\src` respondió `web-136d`.
3. `projects move --id 3 --to-project ideas` (el movimiento 1) y otra vez `import`: «imported 0 of 5», 4 ya presentes y «already here, moved to another project: ideas-7033 1». `ideas` y `web` fueron `by id`, y la declaración quedó como estaba (`declared_here`).
4. Después de `projects move --undo 1`, `import --undo 1` (su prueba en seco dio el mismo reporte) retiró las 5 memorias, las carpetas grabadas de A, los dos proyectos creados y la declaración. `projects list` quedó, byte a byte, como antes del import.
5. Se usó un export de engram sintético (0.2.0): el proyecto `board`, con una observación `decision` y una sesión con su resumen. Con `--project-map board=project:notas`, sus dos filas quedaron en `notas-9039`, bajo `migrated`. Con `notas` borrado (`projects delete notas-9039 --with-memories` y su `--confirm`), el mismo import por su id respondió 409 `project_deleted`, cuyo `detail` nombra el borrado 1 y su `--undo`. Por su nombre respondió 404, desde la CLI y por HTTP (PL27).
6. Las formas de un camino en Windows, con `web2` declarado en el clone de B: `--project-map board=<camino en apps\web\src>` llevó a `web2` con `\`, con `/`, con la unidad en minúscula y relativo desde el clone. `--map-root` llevó a NEW (`by map`) con `\`, con `/` o en mayúsculas.
7. `doctor` dio 20 `ok`, 2 `info`, 1 `warn` y 2 `error` en A, y 21 `ok`, 2 `info` y 2 `error` en B, todos del entorno, como en la 3b. Nada nuevo.

`clawg stop` terminó cada daemon con salida 0.

### Lo que la verificación encontró

- **`--map-root` compara OLD byte a byte.**
  - `roots_here` (`crates/clawg-core/src/memory/portability/import.rs:415`) busca un OLD igual al `root_path` exportado. En Windows, `C:/…/mono`, `c:\…` o un `\` final no son `C:\…\mono`, y el mapa se ignora sin aviso: el proyecto cae `by remote`.
  - Un OLD que no está en el archivo también se ignora sin aviso. `--project-map`, en cambio, nombra los nombres que ninguna nota tiene.
  - Es anterior a la entrega 4 (`6424bca`). Queda como arreglo suelto en el backlog del [README](../../../README.md#backlog-sin-spec-arreglos-sueltos).
- **Un `--map-root` hacia una subcarpeta declarada** hace lo que dice la spec: `--map-root <monoA>=<monoB>\apps\web\src` pone `mono` en `web2`. Después escribe la declaración del archivo como `web2 apps/web → web-136d`, cuyo lado del repositorio no tiene carpeta y nadie alcanza, y deja la sugerencia `similar_name` entre `web2` y `mono-3629`.

### CI

El CI del pull request #97 (a `develop`) pasó en Linux y Windows en `9504600`. Antes hubo dos corridas que fallaron en Windows, y lo que cambiaron fueron dos tests, no el código:

- **En `a14776b`:** `a_project_map_path_in_a_declared_subfolder_goes_to_its_project` armaba el camino del mapa con `/` sobre la raíz canónica, que en Windows lleva el prefijo `\\?\`. `6686947` le saca el prefijo con `strip_verbatim` y une los componentes con `join`. Es el camino mal formado del hallazgo de la 3b: se arregló el test, y el respaldo de `canonical_form` sigue como estaba.
- **En `6686947`:** `a_merge_waits_for_the_saves_of_its_projects` falló con la máquina cargada, porque la tarea que termina el save podía empezar su espera antes de que el test tomara la hora. `9504600` toma la hora antes del `spawn`.

El pull request de `develop` a `main` lo vuelve a correr, sobre el cambio entero, antes del release.

### Lo que no se corrió

- **El socket en el recorrido de Windows:** los homes temporales sirvieron sólo por TCP, como en la 3b.
- **El perfil dev y el de producción:** no se tocó ningún perfil real.

### Qué se vio fallar antes de cada cambio

- **Tareas 56 a 65:** cada tanda de tests se escribió antes que su código, y no compilaba sin lo que nombra: V31 y su fila de checksum, los campos nuevos de `ProjectLine`, el módulo `projects.rs`, los campos nuevos de `ImportReport` y `UndoReport`, `Seat::present_elsewhere`, `MapTarget`, `MapTo` y `Written::without_folder`. `a_row_imported_deleted_leaves_its_deleted_project_deleted` falló antes de su arreglo: el proyecto borrado volvía.

## Al archivar (2026-10-27)

- Los requisitos ya estaban en las specs, cada uno consolidado con su entrega: PL1 a PL65 en [`specs/project`](../../../specs/project/spec.md), V27 a V31 en [`specs/storage`](../../../specs/storage/spec.md), y lo de cada entrega en `specs/portability`, `specs/doctor`, `specs/mcp`, `specs/hooks` y `specs/secrets`. Al archivar sólo cambió el «Origen» de esas siete specs, que ahora apunta aquí y dice 0.65.0.
- **La versión no sube.** La entrega 1 subió la suya (0.64.0), que se absorbió en 0.65.0 al archivar embed-throughput, y 0.65.0 todavía no se publicó (el último tag es `v0.63.0`). Archivar no agrega código. Subir a 0.66.0 habría dejado falsas, sin ningún cambio de conducta, las notas que dicen que este cambio y embed-throughput salen en 0.65.0. Es la misma regla que se siguió al [archivar 89](../2026-10-25-project-registry/verify.md#al-archivar-2026-10-25). Los títulos de este documento conservan la versión con la que se verificó cada entrega.
- **El CI.** Las entregas 1 a 2b no tuvieron pull request propio. La 3a, la 3b y la 4 sí (#95, #96 y #97, a `develop`), y su CI pasó en Linux y Windows. El del #97 corrió sobre todo el cambio unido en `develop`, así que cubre también las entregas 1 a 2b. El pull request de `develop` a `main` lo vuelve a correr antes del release.
- **Lo que queda fuera del cambio:**
  - el paso operativo de ubicar las 15.709 filas sin proyecto del import en prod: no es código, pide autorización explícita y corre con 0.65.0 instalado, porque un build de debug no abre prod. Las que queden sin proyecto se cuentan antes de la línea base de 86;
  - los dos hallazgos de Windows (el respaldo de `canonical_form` sin `strip_verbatim`, y `--map-root` comparado byte a byte), anteriores al cambio, que quedan como arreglos sueltos en el backlog del [README](../../../README.md#backlog-sin-spec-arreglos-sueltos).
- Las citas `ruta:línea` de la propuesta, la spec y el diseño que quedan aquí no cambian, porque describen el código contra el que se escribieron. Sólo se ajustaron los enlaces relativos que rompió el traslado a `archive/`.
