# secret-hygiene — Diseño

Hay design porque las cuatro decisiones abiertas de la exploración tienen alternativas reales, y la purga tiene detalles de SQLite que no son obvios (D5).

## D1. ¿Enmascarar o rechazar el save?

| Opción | A favor | En contra |
|---|---|---|
| **a. Rechazar** (`400 contains_secret`) | El secreto nunca entra al daemon | Se pierde todo el save por una línea. El agente tiene que reformular, y uno que reintenta igual vuelve a fallar. Un falso positivo bloquea una memoria legítima |
| **b. Enmascarar** | El resto del contenido se guarda. Un falso positivo pierde sólo el fragmento, no la memoria | El agente puede no enterarse |
| **c. Enmascarar y avisar** | Lo de b, y el agente sabe qué pasó | Un campo más en la respuesta |

**Decisión: c.** El aviso es posible porque el save ya espera el reporte del worker ([`specs/ingest`](../../../specs/ingest/spec.md), «Un save confirmado devuelve los ids»). En un `202` no hay aviso: el worker todavía no vio el evento. No hace falta: lo que se guarda sale enmascarado igual.

## D2. Dónde se enmascara

| Opción | A favor | En contra |
|---|---|---|
| **a. En el handler de `POST /v1/memories`** | El evento ya viaja limpio, también al volcado `ingest-pending.jsonl` | Cubre sólo una fuente. `eval_api` y cualquier productor futuro (los eventos de chat de `MemoryEvent::from_message`, `crates/clawg-core/src/memory/events.rs:73`) quedarían afuera |
| **b. En `process_event`, antes de trocear** | Es el único camino a la base para todas las fuentes, incluidos los eventos reinyectados de un volcado escrito por un binario anterior | El volcado de un apagado guarda el evento como llegó hasta que se reinyecta |
| **c. En los dos** | Lo mejor de a y b | Dos lugares que tienen que coincidir; el reporte saldría del handler y el worker repetiría el trabajo |

**Decisión: b.** El volcado existe sólo entre un apagado con eventos pendientes y el próximo arranque, y se borra cuando se reinyecta (`crates/clawg-core/src/memory/spill.rs:44-57`). Una sola llamada a `redact` en `process_event` (`crates/clawg-core/src/memory/worker.rs:422`) cubre el chunker, el hash de dedup (`worker.rs:134`), el embedding (`worker.rs:485`) y el insert.

`redact` corre **antes** de `select_chunker`, que mira el contenido: el chunker tiene que ver el mismo texto que se guarda.

## D3. `regex` como dependencia directa

| Opción | A favor | En contra |
|---|---|---|
| **a. Matchers a mano** | Ninguna línea nueva en `Cargo.toml` | Los prefijos fijos son fáciles, pero el PEM, el JWT y la URL con contraseña son pequeños autómatas a mano: más código para revisar que los patrones que reemplazan |
| **b. `regex`** | Los patrones de la spec se leen tal cual. Sin backtracking: tiempo lineal sobre cualquier entrada | Una dependencia directa más |

**Decisión: b.** `regex` 1.12.3 ya está compilado en el binario de producción: lo traen `tokenizers`, `refinery-core` y `tree-sitter` (`cargo tree -p clawg-core -i regex -e normal`). Agregarlo como directa no suma código al build. Se fija con `workspace = true`, a la versión del `Cargo.lock`. Los patrones se compilan una vez con `once_cell::sync::Lazy` (la crate ya la usa; `LazyLock` pide Rust 1.80 y el `rust-version` es 1.77.2).

**Por qué no un solo regex con alternativas:** cada tipo es un regex propio, recorrido en el orden de la tabla. Así el tipo sale de qué regex matcheó y no de grupos con nombre, y las reglas extra (letra y dígito en `openai_key`, las exclusiones de `url_password`) son funciones puras por tipo. Los matches de todos se juntan, se ordenan por inicio (y por largo, al revés) y se descartan los que se superponen con uno anterior.

## D4. Purga: por qué con el daemon parado

| Opción | A favor | En contra |
|---|---|---|
| **a. Endpoint `POST /v1/purge` y comando que lo usa** | No hay que parar el daemon | Pone una operación destructiva detrás del mismo token que usa el puente MCP. El agente no tiene herramienta, pero un agente con shell puede leer `<data>/token`. Y la purga necesita un `VACUUM` y un checkpoint `TRUNCATE` sobre la conexión que el worker y la búsqueda comparten |
| **b. Comando con el daemon parado** (como `clawg restore`) | El lock lo garantiza: nada escribe mientras tanto. La conexión es propia: el `VACUUM` no frena al daemon | Hay que parar el daemon (`clawg stop`); el puente MCP lo vuelve a arrancar en la próxima herramienta |

**Decisión: b.** Purgar es raro y deliberado; parar el daemon es el mismo costo que ya paga `clawg restore`. Usa `StoreLock::acquire(.., StoreHolder::Maintenance)` y la conexión directa sin migrar de `backup_offline` (`crates/clawg-core/src/memory/backup.rs:306-315`). El texto de `Maintenance` pasa a nombrar también la purga.

**Sólo con la base al día.** Una base con migraciones pendientes se rechaza: purgar sobre un esquema viejo obligaría a pensar en cada versión (antes de V8 no hay FTS) para un caso que se arregla arrancando el daemon una vez.

**`--yes`.** La purga no tiene snapshot previo a propósito (sería otra copia del secreto), así que es lo único irreversible que hace clawg. Sin `--yes` se niega antes de abrir nada. Es una decisión pura del CLI.

## D5. Purga: cómo se saca el texto del archivo

Un `DELETE` no alcanza: el texto queda en tres lugares más.

- **El índice FTS.** FTS5 no borra términos en su lugar. El `'delete'` de un índice de contenido externo escribe una marca en un segmento nuevo, que lleva los términos, y los segmentos viejos siguen en `memory_fts_data` hasta un merge. `INSERT INTO memory_fts(memory_fts) VALUES('optimize')` los junta en uno y descarta lo borrado.
- **Las páginas libres.** Cada merge del índice (y cada `DELETE`) libera páginas, que conservan sus bytes en la lista libre de SQLite. Pasa en la operación normal del daemon, mucho antes de la purga.
- **El WAL.** Los frames viejos pueden tener la página como era.

Los pasos, después del `DELETE` de la fila y su vector: `optimize` del FTS, `VACUUM` y `wal_checkpoint(TRUNCATE)`. El `VACUUM` reescribe el archivo sólo con lo vivo, sin lista libre; el checkpoint lleva todo a `memory.db` y deja el WAL en 0 bytes.

**Lo que mostró el test.** El primer diseño usaba `PRAGMA secure_delete = ON` en lugar de `VACUUM`. Un prototipo en Python con 300 filas parecía darle la razón, pero el test sobre una base real (`MemoryStore::open`, WAL, 300 filas con vector) encontró `quokka` en una página de la lista libre: un segmento del FTS que un merge automático había liberado al insertar, sin `secure_delete`. `secure_delete` sólo pone en cero las páginas que se liberan desde que se activa. Sin el `optimize`, o sin el `VACUUM`, `purging_a_live_row_leaves_no_trace_of_its_text` y `purging_a_deleted_row_leaves_no_trace_of_its_text` fallan; con los dos, pasan.

**`VACUUM` y los rowids de vec0.** SQLite advierte que `VACUUM` puede renumerar los rowids de una tabla sin `INTEGER PRIMARY KEY`. Las tablas sombra de vec0 que guardan los vectores declaran `rowid PRIMARY KEY` sin el tipo (`sqlite-vec.c`, `SHADOW_TABLE_ROWID_QUIRK`), y vec0 las lee con `sqlite3_blob_open` por el rowid interno. `VACUUM` conserva los rowids de toda tabla con un índice, porque sus índices los referencian; renumera sólo las que no tienen ninguno (probado con SQLite 3.45: una tabla `(rowid PRIMARY KEY, v)` conserva `_rowid_` 50, 70, 1000 y 30000; una `(a, b)` sin índice pasa de 2, 9, 50 a 1, 2, 3). Todas las tablas sombra de vec0 tienen su clave primaria. El test lo fija: después de purgar, una búsqueda KNN sobre otra fila la sigue encontrando por su id. Es la misma copia que ya usan los backups (`VACUUM INTO`, `backup.rs:275`).

| Alternativa | Por qué no |
|---|---|
| `secure_delete` en vez de `VACUUM` | No limpia las páginas que se liberaron antes (ver arriba) |
| `rebuild` del FTS en vez de `optimize` | Reindexa desde `memory`, incluidas las filas borradas, y rompe el invariante de V8 (sólo filas vivas, `V8__memory_fts.sql:99-104`) |
| Reemplazar `content` por un placeholder y dejar la fila | `content` es de escritura única: no hay trigger de FTS para un `UPDATE` de `content` (`V8__memory_fts.sql:90-93`), y el hash de dedup dejaría de corresponder |
| `VACUUM INTO` a un temporal y `rename`, como `clawg restore` | Lo mismo que `VACUUM` con más pasos que pueden fallar a mitad |

**Filas relacionadas:** lo mismo que el delete. Si la fila estaba viva, `release_challengers` (`crates/clawg-core/src/memory/dispute.rs:132`) libera las que la disputaban, en la misma transacción y después del `DELETE`: antes, la fila todavía cuenta como viva y nada se libera (lo encontró `purging_releases_the_rows_that_disputed_it`). Una fila con `superseded_by` apuntando a la purgada queda como con el delete; la resolución de disputas ya tolera ese puntero (`dispute.rs`, `a_stale_superseded_by_on_a_live_row_does_not_link`).

## D6. Purga y backups

| Opción | A favor | En contra |
|---|---|---|
| **a. Sólo documentarlo** | Nada de código | El usuario no sabe cuáles de sus backups tienen el secreto |
| **b. Reescribir los backups retenidos** | El secreto desaparece de todos lados | Toca la red de seguridad del usuario: un bug acá corrompe justo las copias que sirven para volver. Contradice la regla de no tocar la base |
| **c. Listar los backups que tienen la fila** | El usuario decide con información exacta. No toca nada | Abrir cada snapshot (son pocos: 5 automáticos y los manuales) |

**Decisión: c.** Cada snapshot se abre en sólo lectura (como el probe de `clawg status`, `backup.rs:464`) y se busca la fila por id y `content`. Uno que no se puede leer se lista igual: puede tenerla. La salida dice que esos archivos todavía tienen el texto y que borrarlos es del usuario.

## D7. El escaneo

| Opción | A favor | En contra |
|---|---|---|
| **a. Dentro de `clawg doctor`** | No suma comandos | `doctor` no abre el store por diseño (`.claude/CLAUDE.md`) |
| **b. `clawg memories secrets`, por la API** | Junto a los otros comandos de lectura | La API lista sólo filas vivas, y el escaneo tiene que ver también las borradas, que son justo las que tienen el texto escondido. Habría que ampliar la API para esto |
| **c. `clawg secrets`, leyendo el archivo en sólo lectura** | Ve todas las filas. Sin lock: funciona con el daemon andando, porque WAL admite lectores concurrentes | Un comando más |

**Decisión: c.** Abre con la misma función del probe de `clawg status` (`open_for_probe`, `backup.rs:464`): `immutable` sin WAL, sólo lectura con WAL. Nunca crea `memory.db`, `-wal` ni `-shm`. Imprime tipos, ids y estado, nunca contenido. La detección es la misma `find_secrets` del enmascarado: un secreto que el escaneo reporta es uno que hoy no entraría.

## Módulos

- `crates/clawg-core/src/memory/redact.rs`: `SecretKind`, `find_secrets`, `redact`. Puro.
- `crates/clawg-core/src/memory/worker.rs`: `process_event` llama a `redact`; `IngestReport.redacted`.
- `crates/clawg-core/src/memory/secrets.rs`: `scan(base_dir)`, `purge(base_dir, id)` y `PurgeError` (`thiserror`), que llega como `MemoryError::Purge`.
- `crates/clawg-cli/src/daemon/api.rs` y `client.rs`: el campo `redacted`.
- `crates/clawg-cli/src/mcp/server.rs`: el texto de `save_memory`.
- `crates/clawg-cli/src/secrets.rs`: la salida de `clawg secrets` y `clawg purge`, y la confirmación, puras.
