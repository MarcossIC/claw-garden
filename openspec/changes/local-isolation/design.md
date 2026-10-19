# local-isolation — Diseño

## D1. El directorio, no cada archivo

**Elegido:** un ayudante que crea y aprieta directorios (A1 de la exploración). Con el directorio de datos sin bits de grupo ni de otros, ningún otro usuario lo atraviesa y los modos de adentro no importan para él.

**Alternativas:**
- **A2, `umask(0o077)` en `serve`.** Sólo cubre lo que crea el daemon: el puente crea el directorio y `logs/` antes (`client.rs:1135`, `:1260`), y `backup`, `restore`, `sharing`, `setup` y `update` corren en otro proceso. No aprieta lo existente y pide `unsafe` global al proceso. No se hace.
- **A4, 0600 en cada archivo.** No llega a `-wal` y `-shm`, que crea SQLite, ni a lo que escribe `VACUUM INTO`. Descartada.

## D2. Qué se aprieta y qué no

- Sólo el directorio que se pide. Los antecesores que ya existían no se tocan: el de datos cuelga del directorio personal, y apretar `~` sería cambiar algo que no es de clawg. Los antecesores que faltaban nacen 0700 por `DirBuilder::recursive(true).mode(0o700)`.
- Se quitan sólo los bits de grupo y de otros (`modo & !0o077`); los del dueño y los especiales quedan. La decisión es pura (`tightened(modo) -> Option<u32>`), probada sin disco; el ayudante lee el modo, la consulta y llama a `set_permissions`.
- Los subdirectorios se crean privados, y uno existente que se pide también se aprieta, como defensa en profundidad para una copia que alguien saque del directorio con `cp -a`.
- Un enlace simbólico en lugar del directorio se sigue: se aprieta el destino, que es donde están los datos.
- Lo registra con `warn`, una vez: después del primer arranque el directorio ya es privado y no hay nada más que registrar.

## D3. No poder apretar no detiene a clawg

**Elegido:** si `set_permissions` falla, `warn` con la causa y seguir.

**Alternativa:** devolver el error. Un directorio de datos en un sistema de archivos sin modos Unix (una partición FAT montada, o `/mnt/c` en WSL sin `metadata`) o uno que quedó de root tras un `sudo clawg` dejaría de arrancar, y la memoria del usuario quedaría inaccesible por un cambio de seguridad que no puede aplicar. La regla del proyecto es que una versión nueva abre los datos de la anterior o se niega con un camino de vuelta; acá no hay nada que negar, porque el directorio no queda peor que hoy.

No poder **crear** el directorio sí es un error, como hoy. `MemoryStore::open` lo ignoraba (`db.rs:187`) y dejaba que fallara después el `flock` o SQLite con un mensaje sobre otro archivo; ahora devuelve `MemoryError::DataDir` con la ruta del directorio.

## D4. Dónde vive y quién lo llama

En `crates/clawg-core/src/paths.rs`, junto a `data_dir_in`: lo usan los dos crates y ya es el módulo de las rutas. Sin dependencia nueva (`std::os::unix::fs::{DirBuilderExt, PermissionsExt}`). Es síncrono; `workspace.rs` lo llama desde código `async` como ya lo hace `db.rs` con `create_dir_all`: crear un directorio no bloquea lo que importe.

`setup` es el único lugar que lo llama dos veces (el de datos y `backups/setup/`): puede ser lo primero que corre en una instalación anterior sin que arranque un daemon. En el resto, el directorio de datos ya pasó por el ayudante en el mismo proceso antes de crear el subdirectorio (el daemon al abrir el store; el puente en `start_daemon`; `update` en su candado).

## D5. Qué se automatiza y qué no

La suite corre con un solo usuario del SO: lo que comprueba son los **modos**, que deciden el acceso. Que otro usuario no pueda leer `memory.db` ni abrir `memory.lock` se comprueba a mano con dos cuentas, en `verify.md`.
