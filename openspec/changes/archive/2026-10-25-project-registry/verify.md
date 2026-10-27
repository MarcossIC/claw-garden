# project-registry — Verificación

## Entrega 0 (0.62.1, publicada en 0.63.0)

### Linux

- `cargo test --workspace` pasa, con `tests/binary.rs`.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` salen con 0.
- ONNX Runtime 1.24.2 se enlazó de forma dinámica (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK`) porque el entorno no llegaba a `cdn.pyke.io`, de donde `ort-sys` lo baja al compilar. El código no cambia por eso; el CI compila como siempre.
- Los tests RED fallaban antes del arreglo: cinco de `project.rs` (`a_repository_in_the_path_of_another_gets_a_project_of_its_own`, `a_repository_in_the_path_of_another_takes_none_of_its_rows`, `each_repository_in_one_path_returns_to_its_own_project`, `a_different_remote_in_a_registered_path_is_another_project`, `a_caller_that_derives_its_id_is_refused_another_repository_s_project`) y el de `tests/api.rs`, que esperaba el 409 y recibía 200 con el proyecto ajeno.

### El binario real en un home temporal

Con `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake` y `clawg serve --profile dev --port 0`, sobre una carpeta con `.git/config`:

1. Con `origin` `github.com/acme/a`, `POST /v1/projects` con `assigned` responde `app-ca49` como id de raíz y de proyecto.
2. Con `origin` cambiado a `github.com/acme/b`, el mismo pedido sin `assigned` responde 409 `project_root_collision`. Su `detail` decía que el id ya era del mismo camino que se pedía; se corrigió para decir que el daemon le da otro id a esa raíz (`09feef4`).
3. Con `assigned`, responde `app-db3c`, una fila nueva del mismo camino, sin filas movidas.

Después, con `project_remote` sembrado a mano para atar un segundo remoto a `app-ca49`, como deja un store fundido por un clawg anterior, `clawg doctor --profile dev` da `project_remotes` `warn` `several_remotes` con los dos remotos y la raíz con su `origin` actual, y `projects` sigue `ok` con las dos filas del camino.

### CI

El CI del pull request #93 (cabeza `8022e49`) pasó en `ubuntu-latest` y en `windows-latest`, y `advisories` también. La entrega no tiene código propio de un sistema.

### Lo que no se corrió

- **El perfil dev**: la medición manual de la propuesta (la regla de `doctor` sobre una copia del store con un caso fundido) no se corrió, porque toca un perfil real. La cubre, sin un store real, el binario en un home temporal con `project_remote` sembrado (arriba). Queda como advertencia, no como requisito sin verificar: RG5 se verifica con `the_facts_name_the_remotes_of_each_project` y `project_remote_findings`.

## Al archivar (2026-10-25)

- RG1 a RG4 ya estaban en [`specs/project`](../../../specs/project/spec.md): R20, con los cambios de R15, R16 y R18. RG5 estaba en [`specs/doctor`](../../../specs/doctor/spec.md), en la fila `project_remotes` de R7. Se consolidaron con la entrega (`27f831c`). Al archivar sólo cambiaron el «Origen» de las dos specs, que ahora apuntan aquí, y la versión que cita la fila de `doctor` (anterior a 0.63.0, porque 0.62.1 no se publicó).
- La versión no sube. La entrega 0 ya subió la suya (0.62.1), que se absorbió en 0.63.0 al archivar ram-footprint, y 0.63.0 todavía no se publicó (el último tag es `v0.62.0`). Archivar no agrega código. Subir a 0.64.0 habría dejado falsas, sin ningún cambio de conducta, las notas que dicen que la entrega 0 y el paso 0 de ram-footprint salen en 0.63.0.
- El cambio se partió. Las entregas 1 a 4, con las decisiones de la propuesta, pasaron a [project-locators](../2026-10-27-project-locators/proposal.md) (93). En su propuesta, las citas de `project.rs` apuntan al código de después de la entrega 0.
- Las citas `ruta:línea` de la exploración, la propuesta, la spec y el diseño que quedan aquí no cambian: describen el código contra el que se escribieron. Sólo se ajustaron los enlaces relativos que rompió el traslado a `archive/`.
