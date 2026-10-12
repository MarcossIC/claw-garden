# storage-safety-net — Diseño (partes B + C)

Hay design porque B tiene decisiones reales con alternativas: el mecanismo de copia, cuándo se hace, cómo se restaura y qué se retiene. C es mecánico y sólo lleva notas.

## D1. Mecanismo del snapshot

| Opción | A favor | En contra |
|---|---|---|
| **a. Copiar el archivo** (`std::fs::copy`) | Trivial | Con el daemon andando no incluye el WAL, y copiar `memory.db` + `memory.db-wal` por separado no es consistente: entre una copia y otra puede haber un commit o un checkpoint |
| **b. API de backup online** (feature `backup` de rusqlite) | Copia página a página, consistente, puede ir por pasos | Activa una feature nueva de rusqlite. Necesita una segunda `rusqlite::Connection` de destino dentro del `call` de `tokio_rusqlite`. Copia también el espacio libre |
| **c. `VACUUM INTO '<ruta>'`** | Una sentencia SQL sobre la conexión que ya existe. Consistente (corre en una transacción de lectura), incluye el WAL, y el resultado es una base compacta y autónoma. Sin features ni dependencias nuevas | Falla si el destino existe (se resuelve con un nombre temporal único). Bloquea la conexión mientras copia: para un store personal son milisegundos |

**Decisión: c.** Las tablas vec0 son virtuales: `VACUUM` copia su `CREATE VIRTUAL TABLE` y sus tablas sombra, que son ordinarias. Lo mismo vale para FTS5 (V8). Un test lo comprueba consultando vec0 y FTS sobre el snapshot abierto solo (R3).

El destino es `<backups>/<nombre>.tmp`; si `VACUUM INTO` termina bien, se hace `rename` al nombre final. Si falla, se borra el temporal. Así un snapshot cortado por un kill nunca se ve como completo, y la retención ignora los `.tmp`.

## D2. Cuándo se hace un snapshot

| Opción | Decisión |
|---|---|
| Antes de migrar | **Sí (R2).** Es el único momento en que un binario nuevo cambia el archivo de una forma que el anterior no puede leer |
| Antes de restaurar | **Sí (R6).** Restaurar reemplaza la base: sin esta copia, un restore equivocado es una pérdida |
| Antes de re-embeber | Lo hace la parte D cuando exista `clawg reembed`, llamando a la misma función |
| Periódico (diario, desde el worker de mantenimiento) | **No, por ahora.** Es una decisión abierta de la exploración y nadie lo pidió. `clawg backup` cubre el caso manual, y agregarlo después es llamar a la misma función |
| A pedido | **Sí (R5)**, con `clawg backup` |

### Plan de esquema (R1)

`MemoryStore::open` hoy migra sin mirar (`db.rs:203`). Pasa a:

1. leer `refinery_schema_history` (si existe) como `Vec<(version, name, checksum)>`;
2. calcular `schema_plan(applied, known)`, pura;
3. según el plan: `Newer`/`Diverged` → error tipado sin escribir nada; `Pending` → snapshot y migrar; `Fresh`/`UpToDate` → migrar como hoy.

**Por qué no clasificar el error de refinery después:** refinery ya verifica lo mismo (`MissingVersion`, `DivergentVersion`), pero lo hace dentro de `run`, cuando ya decidió. Necesitamos el plan **antes**, para saber si hace falta snapshot. Con el plan propio, el error de refinery queda como red por si el plan se equivoca.

**Lectura del historial:** refinery guarda el checksum como texto con el `u64` en decimal. Si la tabla no existe, el historial está vacío (`Fresh`). Si existe pero una fila no se puede leer, es un error de SQLite como cualquier otro: no se adivina.

**Detalle de `Diverged`:** una versión aplicada que no existe en el binario pero es menor que la última del binario (un hueco) también es `Diverged`. Sólo una versión mayor que todas las del binario es `Newer`, porque ese es el caso de "volviste a un binario viejo" y tiene su propio mensaje.

## D3. Restore

| Opción | A favor | En contra |
|---|---|---|
| **a. Procedimiento manual documentado** (parar, copiar, borrar el WAL) | Nada de código | El paso que importa es fácil de hacer mal: un `memory.db-wal` viejo sobre una base nueva se aplica en el próximo arranque y la corrompe. Y sin copia previa, un restore equivocado pierde la base actual |
| **b. `clawg restore` con el daemon parado** | Valida antes, deja copia de lo actual, y el orden de los pasos es código con tests | Un comando más |
| **c. Restore en caliente por la API** | No hay que parar | El daemon tendría que cerrar y reabrir su conexión con workers vivos. Mucho riesgo para una operación rara |

**Decisión: b.**

Orden de los pasos y por qué:

1. **Lock primero.** `StoreLock::acquire` con un titular nuevo, `StoreHolder::Maintenance` ("a clawg backup or restore"). Si lo tiene el daemon, el error es `StoreInUse` y el comando agrega "run `clawg stop` first".
2. **Validar el archivo antes de tocar nada.** Se abre en sólo lectura: `PRAGMA quick_check`, lectura del historial y `schema_plan`. `Fresh` también se rechaza: un archivo sin historial no es una base de clawg.
3. **Snapshot de la base actual, y checkpoint.** El snapshot ya incluye el WAL. El `wal_checkpoint(TRUNCATE)` deja `memory.db-wal` vacío, así que borrarlo en el paso 4 no pierde nada aunque el snapshot se borre después a mano.
4. **Reemplazo.** Copia a `memory.db.restore-tmp` en el mismo directorio, `sync_all`, borrar `memory.db-wal` y `memory.db-shm`, y `rename` sobre `memory.db`. El WAL se borra **antes** del `rename`: al revés, un corte entre los dos pasos dejaría un WAL de la base vieja que SQLite aplicaría sobre la nueva.

El volcado `ingest-pending.jsonl` no se toca: son saves aceptados que todavía no están en ninguna base, y se ingieren en la restaurada.

### Restaurar un snapshot más viejo

Un snapshot `pre-migration` tiene el esquema anterior (plan `Pending`). `restore` lo acepta, y el próximo `open` lo migra, con su propio snapshot `pre-migration`. Así el único camino para "volver atrás" después de una actualización es: reinstalar el binario anterior y `clawg restore` del `pre-migration`. Ese camino queda en el mensaje de `SchemaNewer`.

## D4. Retención

- **Automáticos (`pre-migration`, `pre-restore`): los 5 más nuevos (`AUTO_KEEP`).** Se crean una vez por actualización con migración o por restore, así que 5 cubren varias actualizaciones seguidas sin crecer sin límite. Es un valor inicial, no medido.
- **Manuales: nunca se borran solos.** Los pidió el usuario; borrar algo que el usuario guardó a propósito contradice la regla de `config.yaml`.
- La poda corre después de cada snapshot automático, y es best-effort: si no puede borrar un archivo, lo registra y sigue. Un backup de más nunca es un error.
- La función que elige qué borrar recibe nombres y devuelve nombres. Los nombres que no parsean no se tocan.

**Alternativa descartada:** retención por edad (por ejemplo, 30 días). Una base que no se actualiza en meses perdería su único snapshot previo a la última migración.

## D5. Nombre del snapshot

`memory-2026-10-12T17-51-16.123Z-pre-migration.db`:

- UTC, con milisegundos, para que dos snapshots del mismo segundo no choquen y para que el orden alfabético sea el cronológico.
- Guiones en lugar de `:` porque Windows no acepta `:` en un nombre de archivo.
- Fecha legible porque el usuario elige qué restaurar mirando el directorio.
- Sin `chrono`: la conversión de días desde la época a fecha civil es un algoritmo conocido de diez líneas (Howard Hinnant, `civil_from_days`), pura y con test. No vale una dependencia.

## D6. `clawg backup` con y sin daemon

- **Con daemon:** el daemon tiene el lock y la única conexión, así que el backup tiene que pasar por él. `POST /v1/backup` llama a `MemoryStore::snapshot(Reason::Manual)` y responde la ruta. Va detrás del token como el resto de `/v1`.
- **Sin daemon:** no se autoarranca uno (cargaría el modelo y tomaría el lock sólo para copiar). Se toma el lock con `Maintenance`, se abre una conexión `rusqlite` directa sin migrar (un backup copia lo que hay, no lo que habría) y se hace el mismo `VACUUM INTO`.
- **Cómo decide el comando:** `probe_running(base_dir, false)`, igual que `clawg memories` (`main.rs:123-130`).

## D7. Notas de C

- **`clawg status` (R7).** `status_report` gana un parámetro `store: Option<&StoreProbe>`; `main.rs` sólo lo calcula si no hay daemon. `StoreProbe` sale de `backup::probe(base_dir)`: abre `memory.db` con `SQLITE_OPEN_READ_ONLY`, lee el historial y aplica `schema_plan`. No toma el lock: leer en sólo lectura no compite con nadie, y un probe que tomara el lock podría hacer fallar a un daemon que arranca en ese instante. Si `memory.db` no existe, `missing`, sin crear nada; cualquier error de lectura, `unreadable` con el texto del error.
- **MCP (R8).** `explain` separa `StartTimeout` de `NotRunning`. Es el único cambio: la lógica de autoarranque no cambia.
- **Instalador (R9).** Después del `stop`, `& $target status --profile prod *> $null` y mirar `$LASTEXITCODE`. Se usa `status` y no el código de `stop` porque `stop` sale con 1 tanto si no había daemon (el caso normal de una primera instalación o de una máquina sin daemon) como si venció el plazo; `status` sale con 0 sólo si `/health` responde, que es exactamente "sigue vivo".

## Sin dependencias nuevas

`VACUUM INTO`, `PRAGMA quick_check` y `SQLITE_OPEN_READ_ONLY` son de SQLite y ya están en `rusqlite` sin features nuevas. La fecha se calcula a mano (D5).
