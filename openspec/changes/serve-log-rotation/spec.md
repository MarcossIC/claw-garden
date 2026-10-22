# serve-log-rotation — Especificación

Suma a [`specs/storage`](../../specs/storage/spec.md) la sección «El log del daemon».

## R1. `serve.log` se rota al abrirse

`spawn_serve_at` MUST, antes de abrir `<data>/logs/serve.log`, rotarlo si su tamaño pasa de `SERVE_LOG_LIMIT` (10 MiB): `serve.log.1`, si existe, MUST pasar a `serve.log.2`, reemplazando el que hubiera, y `serve.log` MUST pasar a `serve.log.1`. Con el tamaño igual o menor, o sin `serve.log`, MUST NOT renombrar nada. La decisión de qué renombrar MUST ser una función pura del tamaño, el tope y la cantidad de ficheros (`rotation`). El daemon que corre MUST NOT rotar su log.

Un fichero que otro proceso tiene abierto MUST poder rotarse, y lo que ese proceso escribe después MUST quedar en el fichero renombrado, en Unix y en Windows.

Si la rotación falla, `spawn_serve_at` MUST abrir `serve.log` igual, en append, y MUST escribir en él una línea que nombre la causa antes de lanzar el daemon.

#### Escenario: el autoarranque rota
- GIVEN un home de prueba con un `serve.log` de más de `SERVE_LOG_LIMIT`
- WHEN un puente con autoarranque recibe una llamada
- THEN `serve.log.1` tiene el contenido viejo y `serve.log` empieza con la línea de arranque del daemon

#### Escenario: bajo el tope
- GIVEN un `serve.log` de `SERVE_LOG_LIMIT` bytes
- WHEN se decide la rotación
- THEN no se renombra nada

#### Escenario: tres ficheros
- GIVEN `serve.log` sobre el tope, `serve.log.1` y `serve.log.2`
- WHEN se rota
- THEN el viejo `serve.log.2` desaparece, `serve.log.1` pasó a `.2` y `serve.log` a `.1`

#### Escenario: otro escritor
- GIVEN `serve.log` sobre el tope y abierto en append por otro manejador
- WHEN se rota y ese manejador escribe
- THEN lo escrito queda en `serve.log.1`

**Tests:** `nothing_rotates_up_to_the_limit`, `over_the_limit_each_file_moves_one_place`, `rotating_keeps_three_files`, `a_file_open_for_writing_still_rotates`, `a_failed_rotation_is_written_to_the_log` (`serve_log.rs`); `an_autostart_rotates_a_full_serve_log` (`tests/binary.rs`).

## R2. La primera línea del daemon nombra su versión

`clawg serve` MUST registrar como su primera línea, con nivel `info` y antes de resolver el directorio de datos o abrir el store, `clawg serve <versión> starting, profile <perfil>`, con ` (<dist_target>)` después de la versión cuando el binario lo lleva. La versión y el `dist_target` MUST ser los de `build_info`. Vale también cuando el daemon escribe en la terminal.

#### Escenario: un build de release
- GIVEN la versión `0.50.0` y el `dist_target` `x86_64-unknown-linux-gnu`
- WHEN se arma la línea para `dev`
- THEN es `clawg serve 0.50.0 (x86_64-unknown-linux-gnu) starting, profile dev`

#### Escenario: un daemon lanzado a mano
- GIVEN un home de prueba
- WHEN arranca `clawg serve` con su stderr en un fichero
- THEN la primera línea del fichero nombra la versión del binario

**Tests:** `the_starting_line_names_the_version_and_the_build` (`daemon/serve.rs`); `a_daemon_names_its_version_first` y `an_autostart_rotates_a_full_serve_log` (`tests/binary.rs`).
