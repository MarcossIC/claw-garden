# local-isolation — Especificación

Modifica [`specs/daemon-access`](../../../specs/daemon-access/spec.md): agrega R6 (el directorio de datos es privado) y cambia el párrafo `Límite` y R5.

## Definiciones

- **Directorio de datos:** `<home>/.clawg` o `<home>/.clawg-dev` (`data_dir_in`, `crates/clawg-core/src/paths.rs:159`).
- **Modo privado:** en Unix, un directorio sin ningún bit de grupo ni de otros (`modo & 0o077 == 0`). Uno nuevo nace 0700.
- **Ayudante:** `ensure_private_dir(dir)` en `crates/clawg-core/src/private_dir.rs`.

## Límite (modifica el de `specs/daemon-access`)

El guardia cierra el paso a una página web abierta en el navegador del usuario, con DNS rebinding o sin él. No es una barrera contra un proceso local: uno que puede leer el token manda también el `Host` correcto. En Unix otro usuario local no lee nada del directorio de datos (R6), ni el token; si llega al puerto, sigue frenado por el token. Nada de esto frena a un proceso del mismo usuario ni a root.

## R6. El directorio de datos es privado (Unix)

`ensure_private_dir(dir)` MUST crear `dir` y cada antecesor que falte con modo 0700, sea cual sea el `umask`. Si `dir` ya existe y no tiene modo privado, MUST quitarle los bits de grupo y de otros, conservar los demás, y registrar un `warn` con la ruta, el modo anterior y el nuevo. MUST NOT cambiar el modo de un antecesor que ya existía ni el de ningún archivo. Si no puede cambiar el modo (el directorio es de otro usuario, o el sistema de archivos no lo admite), MUST registrar un `warn` con la causa y seguir: el directorio existe y clawg funciona como antes. Si no puede crearlo, MUST devolver el error. En Windows MUST sólo crear lo que falte.

Todo lugar de clawg que crea el directorio de datos o un subdirectorio suyo MUST hacerlo con el ayudante: el de datos en `clawg serve`, en el autoarranque del puente antes del daemon, al abrir el store, en `restore`, al guardar `sharing.json` y en el candado de `update`; `logs/`, `backups/`, `backups/setup/`, `workspaces/…` y `models/…` donde se crean. `clawg setup` MUST pasar por el ayudante también el directorio de datos antes de copiar un archivo. Un error al crear el directorio al abrir el store MUST NOT ignorarse.

#### Escenario: una instalación nueva
- GIVEN un home sin directorio de datos y `umask` 022
- WHEN un puente arranca el daemon
- THEN el directorio de datos y `logs/` son 0700, y no se registra que se apretó nada

#### Escenario: una instalación anterior
- GIVEN un directorio de datos 0755 con un `memory.db` poblado
- WHEN arranca el daemon
- THEN el directorio de datos es 0700, el registro dice `0755` y `0700`, y el store devuelve las mismas filas

#### Escenario: un directorio ya privado
- GIVEN un directorio de datos 0700
- WHEN arranca el daemon
- THEN su modo sigue 0700 y no se registra nada

#### Escenario: sólo el directorio pedido
- GIVEN un directorio 0755 que contiene un archivo 0644, dentro de un padre 0755
- WHEN se llama al ayudante sobre él
- THEN el directorio es 0700, y el archivo y el padre conservan su modo

**Tests:** `a_missing_directory_and_its_parents_are_created_private`, `an_open_directory_is_tightened_and_the_rest_left_alone`, `a_private_directory_is_left_as_it_is`, `only_group_and_other_bits_are_taken` (`private_dir.rs`, Unix); `save_creates_a_private_data_directory` (`sharing.rs`), `a_snapshot_directory_is_created_private` (`backup.rs`), `restore_creates_a_private_data_directory` (`backup.rs`), `opening_a_store_makes_its_directory_private`, `a_data_directory_that_cannot_be_created_is_an_error` (`db.rs`), `the_update_lock_makes_the_data_directory_private` (`update/lock.rs`), `setup_copies_go_in_a_private_directory` (`setup.rs`), `autostart_creates_a_private_data_directory`, `the_serve_log_goes_in_a_private_directory` (`client.rs`), `workspace_directories_are_created_private` (`workspace.rs`), `the_model_cache_is_created_private` (`model_download.rs`); `a_daemon_on_a_fresh_home_leaves_a_private_data_directory`, `an_open_data_directory_is_tightened_and_keeps_its_rows` (`tests/binary.rs`, Unix).

## R5. El token y `daemon.json` nacen restringidos (Unix) (modifica R5)

Sin cambios en lo que exige. Se agrega: con R6, el modo 0600 del token y de `daemon.json` es una segunda capa; la primera es el directorio.
