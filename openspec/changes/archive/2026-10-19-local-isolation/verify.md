# local-isolation — Verificación

## Linux

Sobre el commit de docs (`865e34e`):

- `cargo test --workspace`: pasa entero.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` (lo que construye el release): sin advertencias.
- Cada test nuevo de la [spec](spec.md) falló antes de su código: por no compilar (`MemoryError::DataDir`) o por la aserción (los directorios nacían 0755 con `umask` 022). Los ocho de `clawg-core` se volvieron a correr con el ayudante reducido a `create_dir_all` y fallaron todos. Los dos de `tests/binary.rs` pasaban los modos ya después de la tarea 2, porque `MemoryStore::open` apretaba lo que `serve` acababa de crear 0755; lo que fallaba hasta la tarea 3 era el `warn` en una instalación nueva, que `a_daemon_on_a_fresh_home_leaves_a_private_data_directory` fija.

## Con dos cuentas en Linux

Dos cuentas del SO creadas para la prueba (dueña y otra), el home de la dueña en 0755 (como lo crean distribuciones con `HOME_MODE` abierto), `umask` 022, y el build de tests de `target/debug/clawg` copiado a ese home, con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake`. Las cuentas se borraron al terminar.

1. **Instalación nueva.** La dueña arrancó `clawg serve --profile dev --port 0` sin directorio de datos, guardó una memoria y lo paró. `.clawg-dev` quedó 0700; `memory.db` y `memory.lock` 0644 (como siempre, con el `umask`). El registro no dice que se apretara nada.
2. **Lo que deja una versión anterior.** Con `.clawg-dev` en 0755, la otra cuenta listó el directorio, leyó el texto de la memoria en `memory.db` y tomó `flock -s` sobre `memory.lock`.
3. **El derivado de `memory.lock`, medido.** Con el directorio en 0755 y la otra cuenta reteniendo `flock -s memory.lock`, `clawg serve` de la dueña apretó el directorio (`was 0755; made it 0700`) y salió con 1: `the memory store is in use by another process`. El riesgo de la exploración era real. Cuando ese proceso terminó, la otra cuenta ya no pudo abrir `memory.lock` (`Permission denied`) y el daemon arrancó. Un descriptor abierto antes de apretar sobrevive hasta que su proceso termina: la ventana es la de una instalación anterior hasta su primer arranque con esta versión.
4. **Después de arrancar esta versión.** El registro dice `/…/.clawg-dev was 0755; made it 0700 so other users of this machine cannot read it`. La otra cuenta: `ls` del directorio, `cat memory.db` y `flock` sobre `memory.lock` dieron `Permission denied`. La dueña lee su memoria por la API como antes.

## Lo que no se verificó

- **Windows**, fuera de la CI del pull request (suite y clippy en `windows-latest`). Ahí el ayudante sólo crea, como antes; no cambia ningún comportamiento.
- **macOS:** no hay CI. El código es el mismo de Linux (`DirBuilderExt`, `PermissionsExt`).
- Un directorio de datos en un sistema de archivos sin modos Unix, donde `set_permissions` falla y se registra: cubierto sólo por lectura del código (D3).
