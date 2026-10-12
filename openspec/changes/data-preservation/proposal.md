# data-preservation — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| Transversal: la base del usuario no se pierde | M (partes A + F de la exploración) | — | propuesta |

**Este cambio cubre las partes A y F.** La exploración pide partir el cambio al proponerlo. A + F va primero porque es la única pérdida real que ocurre hoy, en cada actualización. B + C, D y E siguen en `exploration.md` y pasan a ser cambios propios (ver "Fuera de alcance").

## Problema

### A. El apagado pierde saves aceptados

- El worker de ingesta se lanza sin `JoinHandle` (`crates/clawg-cli/src/daemon/serve.rs:94`). Cuando termina `axum::serve`, `run` limpia `daemon.json` y retorna (`serve.rs:135-144`). `main` corre en `#[tokio::main]` (`crates/clawg-cli/src/main.rs:30`), así que al salir el runtime cancela el worker: se pierde lo encolado y el evento a medio embeber.
- Si el embedder no cargó, el worker descarta cada evento retenido en cuanto se cierra el canal: "ingest event dropped at shutdown, the embedder never loaded" (`crates/clawg-core/src/memory/worker.rs:152-159`, `:191-206`). Es la ventana más grande: justo después de arrancar, mientras se descarga o carga el modelo.
- El `AccessTracker` sale del loop sin vaciar el buffer cuando se cierra el canal (`crates/clawg-core/src/memory/access.rs:68`). Pierde hasta 30 s de estadísticas de acceso, no contenido. Además se lanza sin handle (`access.rs:52`), así que nadie puede esperarlo.
- `crates/clawg-cli/src/stop.rs:12-14` dice que el apagado es "a drain of the ingest queue and a WAL checkpoint". **Ninguna de las dos cosas existe:** no hay `wal_checkpoint` en el repo.
- El instalador ejecuta `clawg stop` antes de reemplazar el binario (`scripts/install.ps1:169-173`). Por eso **cada actualización** puede perder saves que el agente vio como `queued`.

### F. Endurecimientos

- **Base vacía si no hay home.** `data_dir` usa `./.clawg` sin avisar si no resuelve el home (`crates/clawg-core/src/paths.rs:146-152`). El daemon arranca con una base vacía en el directorio actual, y el usuario ve su memoria "desaparecida".
- **Migración e historial no son atómicos.** `run_migrations` usa el runner tal cual (`crates/clawg-core/src/memory/db.rs:362-364`), con `grouped=false`. Un kill entre el cuerpo de una migración y su fila de historial la vuelve a correr en el próximo arranque. Con `ALTER TABLE ... ADD COLUMN` (`crates/clawg-core/migrations/V4__root_cause.sql:3`, `V6__content_hash.sql:5`) eso es "duplicate column" en cada arranque: el daemon no vuelve a abrir la base.
- **Sin checksums fijados.** Nada falla si alguien edita una migración ya publicada. Refinery aborta con checksums divergentes (`abort_divergent=true`, por defecto), así que una edición accidental deja al usuario sin daemon. `.gitattributes` ya fuerza LF en `*.sql`, así que el riesgo CRLF de la exploración queda cubierto para los checkouts actuales; falta el test que lo fije.
- **`workspace.md` se reescribe sin atomicidad** (`crates/clawg-core/src/memory/workspace.rs:89-91`, `tokio::fs::write`) en cada `recall_context`. Un kill a mitad de escritura trunca el archivo, incluida la sección USER que edita el usuario.

## Alcance

1. **Apagado ordenado del daemon.** Después de que `axum::serve` termina (ya no entran requests):
   1. se cierra el canal de ingesta y el worker drena lo encolado, con un plazo;
   2. lo que no se pudo ingerir dentro del plazo (incluido todo lo retenido porque el embedder no cargó) se **vuelca a disco** en `<data>/ingest-pending.jsonl`;
   3. el `AccessTracker` hace un flush final y termina;
   4. el worker de mantenimiento se cancela;
   5. se hace `PRAGMA wal_checkpoint(TRUNCATE)` (best-effort, con log);
   6. se limpia `daemon.json` y se suelta el lock.
2. **Reinyección al arrancar.** Si existe `ingest-pending.jsonl`, el worker procesa esos eventos antes que los nuevos y borra el archivo cuando terminó.
3. **Plazo de drenaje menor que `STOP_TIMEOUT`.** Así `clawg stop` ve terminar el proceso, y el instalador no reemplaza el binario con el daemon todavía escribiendo.
4. **Doc de `stop.rs` corregido** para describir lo que el apagado hace de verdad.
5. **`data_dir` falla** si no hay home, en vez de usar `./.clawg`.
6. **Migraciones agrupadas:** todas las pendientes y su historial en una sola transacción (`set_grouped(true)`).
7. **Test de checksums fijados** de V1–V8.
8. **Escritura atómica de `workspace.md`:** archivo temporal en el mismo directorio y `rename`.

**Enfoque:** las decisiones (qué se drena, qué se vuelca, cuándo vence el plazo, cómo se lee y escribe el volcado) son funciones puras. La secuencia de apagado vive en `clawg-core` (`MemoryRuntime`), testeable con el store en memoria; `serve.rs` sólo la llama. Detalle y alternativas en `design.md`.

## Fuera de alcance

- **B + C. Red de seguridad y actualización verificada** (backups, `clawg backup`/`restore`, instalador que confirma el stop, mensajes de downgrade y contrato). Cambio propio.
- **D. Cambio de modelo con camino** (`clawg reembed`, test del contrato de embedding). Cambio propio.
- **E. Borrado recuperable** (`restore_memory`). Cambio propio, coordinado con visibility-policy y secret-hygiene.
- **Durabilidad ante un kill o un corte de luz.** El volcado cubre el apagado ordenado. Un `kill -9` sigue perdiendo lo encolado: eso es la opción C (outbox) de ingest-durability.
- **El contrato del save** (`queued`, rechazo de saves con la memoria desactivada, reintento del bootstrap, el mensaje "search will work in a minute" de `crates/clawg-cli/src/mcp/server.rs:235-238`): ingest-durability. La parte D sólo aporta el remedio (`clawg reembed`), y la parte C conserva los mensajes de downgrade y checksum. Los dos cambios tocan `serve.rs` en bloques distintos (el bootstrap allá, el apagado acá): a lo sumo un conflicto de merge.
- **Regla de preservación en `config.yaml`.** La exploración la proponía en F, pero ya existe: "Any change to the schema … ships with a test that seeds data at the previous version" (`openspec/config.yaml`, `rules.general`). No hay migraciones nuevas en este cambio, así que tampoco hay test de siembra.

## Éxito

`cargo test --workspace` en verde contra la base de 1304. Cada uno de estos tests falla hoy:

| # | Test | Falla hoy porque |
|---|---|---|
| 1 | Un apagado con eventos encolados y el embedder listo deja todas las filas en la base | el worker se cancela sin drenar |
| 2 | Un apagado con el embedder sin cargar vuelca los eventos a `ingest-pending.jsonl`, y el próximo arranque los ingiere | se descartan con un warn |
| 3 | Un apagado con accesos en el buffer los escribe en `access_count` | el tracker sale sin flush |
| 4 | Después del apagado, `memory.db-wal` está vacío o no existe | no hay checkpoint |
| 5 | El plazo de drenaje es menor que `STOP_TIMEOUT` | no existe el plazo |
| 6 | Sin home, `data_dir` devuelve un error | devuelve `./.clawg` |
| 7 | Una migración que falla no deja ni el esquema ni el historial de las anteriores del mismo arranque | `grouped=false` |
| 8 | Los checksums de V1–V8 coinciden con los fijados | no existe |
| 9 | Una escritura de `workspace.md` interrumpida deja el archivo anterior intacto | `tokio::fs::write` trunca |

No toca ranking ni recuperación: no aplica `rag-eval`.

**Prueba manual**, sobre el perfil dev y **con autorización**: arrancar el daemon, guardar mientras el modelo carga, `clawg stop`, volver a arrancar y comprobar que las filas aparecen.

## Datos guardados

- **Esquema:** no cambia. No hay migración nueva.
- **`set_grouped(true)`** sólo cambia cómo se aplican las migraciones pendientes. Una base ya migrada a V8 no tiene pendientes, así que no pasa nada al abrirla. Una base en una versión anterior se migra igual, pero de forma atómica.
- **Archivo nuevo:** `<data>/ingest-pending.jsonl`. Sólo existe entre un apagado con eventos sin ingerir y el arranque siguiente. Usa el `Serialize`/`Deserialize` que `MemoryEvent` ya tiene (`crates/clawg-core/src/memory/events.rs:12`).
- **Rollback:** revertir los commits. Si quedó un `ingest-pending.jsonl` y se vuelve a un binario anterior, el binario viejo lo ignora: esos eventos no se ingieren, pero la base no se toca. Al volver a la versión nueva se ingieren.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| El drenaje tarda más que `STOP_TIMEOUT` y `clawg stop` reporta timeout | Media con colas largas | El plazo de drenaje se fija por debajo y lo que no entra se vuelca, que es rápido. Un test fija la relación entre los dos números |
| Reinyectar duplica un save si el proceso muere a mitad de la reinyección | Baja | El volcado se borra recién al terminar. Un evento repetido pasa por el gate de dedup (`worker.rs:225`) y refuerza la fila existente: sube `access_count`, no crea otra fila |
| vec0 dentro de una transacción agrupada | Baja | `CREATE VIRTUAL TABLE` es transaccional en SQLite. Lo prueba el test 7 con las ocho migraciones reales sobre una base vacía |
| Mientras drena, `daemon.json` existe pero `/health` no responde, y un puente MCP intenta autoarrancar otro daemon | Media | El segundo daemon pierde el lock y no arranca (`StoreInUse`). La verificación del instalador es parte C |
| Cambiar la firma de `data_dir` rompe llamadores | Baja | Son dos (`serve.rs:66`, `crates/clawg-cli/src/client.rs:624`) más `data_dir_from_env` (`paths.rs:156`) |
