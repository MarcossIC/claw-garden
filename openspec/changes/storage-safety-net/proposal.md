# storage-safety-net — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| Transversal: la base del usuario no se pierde | M (partes B + C de la [exploración de data-preservation](../archive/2026-10-12-data-preservation/exploration.md#alcance-candidato-partes)) | data-preservation A + F (archivado) | verify: falta lo de Windows |

**Este cambio cubre las partes B y C.** data-preservation se partió al proponerse: A + F ya están archivadas y consolidadas en [`specs/storage`](../../specs/storage/spec.md). Este cambio es el siguiente grupo que fijó esa exploración. D (cambio de modelo) y E (borrado recuperable) siguen siendo cambios propios (ver "Fuera de alcance").

## Problema

### B. No hay red de seguridad

- **No hay ningún backup.** El repo no tiene `backup`, `VACUUM INTO` ni la feature `backup` de rusqlite (`Cargo.toml:32` sólo activa `bundled`, `load_extension` y `functions`). Tampoco hay comando de restore.
- **Las migraciones corren sin copia previa.** `MemoryStore::open` llama a `run_migrations` sin mirar si hay pendientes (`crates/clawg-core/src/memory/db.rs:203`). Las de hoy son aditivas, pero la primera que reconstruya una tabla (cambiar un `CHECK`, o el `float[384]` de vec0 en `crates/clawg-core/migrations/V2__vectors.sql`) no va a tener vuelta atrás.
- **Una copia a mano no alcanza.** Con el daemon andando, copiar `memory.db` deja afuera lo que está en `memory.db-wal` (WAL activo, `db.rs:186-200`). Y un binario viejo no abre una base más nueva, así que volver al binario anterior (el instalador guarda uno, `scripts/install.ps1:186-195`) no sirve después de una migración.

### C. Actualización a ciegas y mensajes que esconden la causa

- **El instalador no comprueba que el daemon paró.** Ejecuta `& $target stop --profile prod *> $null` con los errores relajados y nunca mira el resultado (`scripts/install.ps1:169-173`). `clawg stop` sale con 1 tanto si no hay daemon como si vence `STOP_TIMEOUT` (`crates/clawg-cli/src/stop.rs:39-42`, `crates/clawg-cli/src/main.rs:94-110`). Si el daemon sigue vivo, el instalador mueve el binario a un lado y sigue: la versión vieja queda sirviendo con el lock tomado, sin que nadie lo diga.
- **Una base más nueva o con checksums distintos se rechaza sin decir por qué.** Refinery aborta con `MissingVersion` o `DivergentVersion` (refinery-core 0.9.1, `traits/mod.rs:11-12`), y el daemon sale con "could not open the memory store: Migration failed: migration V9__… is missing from the filesystem" (`crates/clawg-cli/src/daemon/serve.rs:35`). El texto no dice qué hacer, y la base queda intacta, pero el usuario no lo sabe.
- **El agente sólo ve "no hay daemon".** Cuando el autoarranque falla, `clawg mcp` responde "no clawg daemon is running; start one with `clawg serve` and try again" (`crates/clawg-cli/src/mcp/server.rs:232-234`). La causa real queda en `logs/serve.log`, y `clawg status` sólo dice `"running": false` (`crates/clawg-cli/src/status.rs:23-36`).

## Alcance

1. **Snapshot antes de migrar.** Si `memory.db` ya tiene migraciones aplicadas y este binario trae pendientes, `MemoryStore::open` hace un snapshot completo (`VACUUM INTO`, incluye lo que está en el WAL) en `<data>/backups/` **antes** de correr las migraciones. Si el snapshot falla, no migra y no abre.
2. **Verificación previa del esquema.** Antes de migrar, se compara el historial de `refinery_schema_history` con las migraciones del binario, con una función pura. Una base **más nueva** o **divergente** se rechaza con un error tipado que dice qué pasó, que nada se tocó y qué hacer. Una base nueva o al día no hace nada extra.
3. **`clawg backup [--profile P]`.** Con el daemon andando, se lo pide por `POST /v1/backup` (el daemon tiene la conexión y el lock). Sin daemon, toma el lock y copia directo, sin migrar. Imprime la ruta del archivo.
4. **`clawg restore <ARCHIVO> [--profile P]`.** Sólo con el daemon parado (toma el lock). Valida el archivo (que sea SQLite íntegro, de clawg, y no más nuevo que el binario), hace un snapshot de la base actual, y la reemplaza de forma atómica. Imprime de dónde restauró y dónde quedó la base anterior.
5. **Retención.** Los snapshots automáticos (antes de migrar y antes de restaurar) se podan a los 5 más nuevos. Los de `clawg backup` nunca se borran solos.
6. **El instalador verifica el stop.** Después de `clawg stop`, ejecuta `clawg status --profile prod`: si sale con 0, el daemon sigue vivo y el instalador aborta sin tocar nada.
7. **Mensajes con la causa.**
   - `clawg status` sin daemon agrega un bloque `store` con el estado del esquema de `memory.db` (al día, con pendientes, más nueva, divergente, inexistente o ilegible), leído en sólo lectura.
   - El mensaje de MCP cuando el daemon no arranca dice que corra `clawg status` para ver la causa.

**Enfoque:** las decisiones (qué plan de esquema aplica, si hace falta snapshot, cómo se llama un snapshot, cuáles se podan, si un archivo se puede restaurar) son funciones puras. El I/O (`VACUUM INTO`, copia, `rename`) vive en un módulo nuevo de `clawg-core`, `memory::backup`. Detalle y alternativas en `design.md`.

## Fuera de alcance

- **D. Cambio de modelo con camino** (`clawg reembed`, test del contrato de embedding). Cambio propio. Este cambio deja la función de snapshot lista para que D la llame antes de re-embeber.
- **E. Borrado recuperable** (`restore_memory`). Cambio propio, coordinado con visibility-policy y secret-hygiene.
- **Backups periódicos** (por ejemplo, uno diario desde el worker de mantenimiento). Decisión abierta en la exploración; no se agrega sin un pedido. `clawg backup` cubre el caso manual.
- **Ubicación configurable de los backups.** Una carpeta fuera del directorio de datos (por ejemplo, sincronizada a la nube) saca de la máquina una copia en claro de toda la base. Se decide junto con secret-hygiene.
- **Parar también el perfil dev en el instalador.** El instalador instala el binario de release para prod. Un daemon dev con el binario viejo sigue corriendo desde el archivo movido a un lado (`install.ps1:186-195`), sin riesgo para los datos.
- **Un daemon viejo que arranca durante la instalación**, entre la verificación y el reemplazo del binario (un puente MCP que autoarranca). La ventana es de milisegundos; cerrarla necesita un lock de instalación que no existe.

## Éxito

`cargo test --workspace` en verde. Cada uno de estos tests falla hoy:

| # | Test | Falla hoy porque |
|---|---|---|
| 1 | Abrir una base sembrada en V7 con un binario en V8 deja un snapshot en `backups/`, y el snapshot abre con las filas de V7 | no hay snapshot |
| 2 | Un snapshot con escrituras recientes en el WAL las contiene | no existe |
| 3 | Una base con una V9 aplicada que el binario no conoce se rechaza con `SchemaNewer`, y el archivo queda byte a byte igual | refinery da `MissingVersion` |
| 4 | Una base con el checksum de V3 cambiado se rechaza con `SchemaDiverged` | refinery da `DivergentVersion` |
| 5 | `clawg backup` con el daemon andando y sin él produce un archivo que abre con las mismas filas | no existe |
| 6 | `clawg restore` reemplaza la base, deja la anterior en `backups/`, y se niega con el daemon andando o con un archivo más nuevo, corrupto o ajeno | no existe |
| 7 | La retención deja los 5 automáticos más nuevos y ningún manual borrado | no existe |
| 8 | `clawg status` sin daemon muestra el estado del esquema | sólo dice `running: false` |
| 9 | El mensaje de MCP por un daemon que no arrancó nombra `clawg status` | dice "start one with `clawg serve`" |

No toca ranking ni recuperación: no aplica `rag-eval`.

**Prueba manual en Windows**, sobre el perfil dev y **con autorización**: `clawg backup` con el daemon andando, `clawg stop`, `clawg restore` del archivo, arrancar y comprobar el conteo de filas. Y el instalador con un daemon que no para (o simulado con `status` devolviendo 0): tiene que abortar sin mover el binario.

## Datos guardados

- **Esquema:** no cambia. No hay migración nueva.
- **Directorio nuevo:** `<data>/backups/`, con archivos `memory-<fecha UTC>-<motivo>.db`. Cada uno es una base SQLite completa y autónoma (sin WAL).
- **`clawg restore`** es el único camino nuevo que reemplaza `memory.db`. Antes de reemplazarla siempre deja un snapshot de la base actual; si ese snapshot falla, no reemplaza nada.
- **Rollback:** revertir los commits. `backups/` queda en disco y un binario anterior lo ignora. Un snapshot restaurado a mano (copiándolo sobre `memory.db` con el daemon parado y sin `memory.db-wal`) abre con cualquier binario de su versión de esquema o posterior.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| `VACUUM INTO` con vec0 registrado en la conexión | Baja | Las tablas vec0 son virtuales, y sus datos viven en tablas sombra ordinarias que `VACUUM` copia. Lo prueba el test 1 con las ocho migraciones reales y vectores insertados |
| Un snapshot por arranque llena el disco | Baja | Sólo hay snapshot si hay migraciones pendientes, que es una vez por actualización con migración. Retención de 5 |
| `restore` deja la base a medias si se corta la luz | Baja | Se escribe a un temporal en el mismo directorio, con `fsync`, y se hace `rename`. Antes se vacía el WAL de la base actual con un checkpoint, así que borrar `memory.db-wal` no pierde nada y un WAL viejo nunca se aplica sobre la base nueva |
| `clawg status` lee `memory.db` mientras otro proceso la tiene | Baja | Sólo lee sin daemon, en modo sólo lectura y sin migrar. Si no puede leer, dice `unreadable` en vez de fallar |
| El snapshot previo a migrar falla (disco lleno) y el daemon no arranca | Media en disco lleno | Es lo correcto: migrar sin red es lo que este cambio evita. El error dice el motivo y la ruta |
