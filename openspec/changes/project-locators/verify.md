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

Pendiente: el pull request de la entrega todavía no se abrió.

### Lo que no se corrió

- **Linux en local:** sólo se corrió en Windows. Lo cubre el CI del pull request.
- **El perfil dev:** no se tocó ningún perfil real.
