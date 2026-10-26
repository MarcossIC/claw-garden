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

Pendiente para el pull request final del cambio. `project-locators` se publica entero, sin una release por entrega, así que no se abre un pull request de develop a main al cerrar cada una.

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

Pendiente para el pull request final del cambio, como el de la entrega 1.

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

Pendiente para el pull request final del cambio, como el de las entregas 1 y 2a.

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
