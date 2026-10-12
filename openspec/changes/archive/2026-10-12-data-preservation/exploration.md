# data-preservation — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| Transversal: la base del usuario es lo más importante del producto | L (al proponer se parte en partes A–F) | — | exploración (cerrada para A + F) |

**Tesis:**

- Hoy una actualización **no borra** la base: nada en el instalador ni en el código de producción toca `memory.db`.
- Sí hay formas de **perder** lo que el usuario cree guardado, o de **esconder** toda la memoria detrás de una versión nueva.
- No existe ninguna red de seguridad: ni backups, ni restore, ni undelete.

**Regla que este cambio establece:** la app puede cambiar, la base no se pierde. Toda versión nueva tiene que poder abrir y servir la base de la anterior, o negarse a hacerlo sin tocarla y con un camino de vuelta.

## Lo que ya protege los datos (verificado)

- **Instalador.** `scripts/install.ps1` sólo toca el directorio de instalación y el PATH del usuario (`:175-206`). Nunca toca `~/.clawg`.
- **Migraciones V1–V8:**
  - son aditivas;
  - están embebidas en el binario (`crates/clawg-core/src/memory/migrations.rs:20`);
  - cada una corre en su propia transacción.
  - V7 sólo borra una vista.
- **Refinery con sus defaults.** `abort_divergent=true`, `abort_missing=true` (refinery-core 0.9.1 `runner.rs:238-241`), y `crates/clawg-core/src/memory/db.rs:363` no los cambia. Si se instala un binario viejo o con checksums distintos, el daemon **no arranca** y los datos quedan intactos.
- **Nada borra ni recrea `memory.db`.**
  - El delete es lógico: el texto queda con `deleted_at` (`crates/clawg-core/src/memory/admin.rs:205-251`).
  - No hay purga: `crates/clawg-core/src/memory/maintenance.rs:229-238`.
- **Lock exclusivo** `memory.lock` (`crates/clawg-core/src/memory/store_lock.rs:117-137`). Ningún segundo proceso abre ni migra la base.
- **WAL con `synchronous=NORMAL`** (`db.rs:182-201`).
  - Un kill del proceso no pierde nada commiteado.
  - Un corte de luz puede perder los últimos commits, pero no corrompe el archivo.
  - La memoria y su vector se insertan en una sola transacción (`crates/clawg-core/src/memory/store_insert.rs:329-372`).
- **Contrato de embedding.** Si no coincide, no borra nada: deja la memoria desactivada (`crates/clawg-core/src/memory/bootstrap.rs:281-302`).
- **Un binario debug se niega a abrir prod** (`crates/clawg-core/src/paths.rs:85-91`).

## Riesgos (verificados), por severidad

### 1. PÉRDIDA: los saves aceptados se pierden al parar el daemon

- El worker de ingesta se lanza sin `JoinHandle` (`crates/clawg-cli/src/daemon/serve.rs:94`).
- Cuando termina `axum::serve`, `run` retorna sin drenar (`serve.rs:135-144`). Se pierde lo encolado y lo que estaba a medio embeber.
- `crates/clawg-cli/src/stop.rs:12-14` documenta "a drain of the ingest queue and a WAL checkpoint". **Ninguna de las dos cosas existe.**
- El instalador ejecuta `clawg stop`, así que **cada actualización** puede perder saves que el agente vio como `queued`.
- La ventana más grande es justo después de arrancar, mientras carga el embedder. El worker lo deja en el log: "ingest event dropped at shutdown" (`crates/clawg-core/src/memory/worker.rs:152-159`).
- El `AccessTracker` no hace un flush final (`crates/clawg-core/src/memory/access.rs:68`). Pierde hasta 30 s de estadísticas, no contenido.

### 2. OCULTAMIENTO TOTAL: cambiar el modelo de embedding en una versión futura

- **Disparador:** cambiar el modelo, la dimensión o los prefijos e5 (`crates/clawg-core/src/memory/embedder.rs:109`).
- **Efecto:** el contrato no coincide y la memoria queda desactivada para todos. El usuario ve una memoria vacía.
- **No hay remedio en el binario.** `reembed_all` (`maintenance.rs:442`) no tiene llamador: era un comando de Tauri y se borró en `5678ded`.
- **Los saves siguen respondiendo `queued`**, quedan retenidos y se descartan al parar (ver riesgo 1).
- **Un cambio de dimensión además necesita una migración:** V2 fija `float[384]`.

### 3. SIN RED DE SEGURIDAD: no hay backups

- No hay snapshot antes de migrar, ni comando de backup o restore. El repo no tiene `backup`, `VACUUM INTO` ni `wal_checkpoint`.
- Las migraciones de hoy son seguras. Pero la primera que reconstruya una tabla (cambiar un CHECK, o cambiar vec0) no va a tener vuelta atrás.
- Una copia manual de `memory.db` con el daemon andando no incluye lo que está en `memory.db-wal`.
- El instalador conserva un solo binario anterior (`install.ps1:179-180`), y un binario viejo no abre una base más nueva.

### 4. ACTUALIZACIÓN A CIEGAS: el instalador no comprueba que el daemon paró

- El instalador ejecuta `& $target stop --profile prod *> $null` con los errores relajados, y nunca mira el resultado (`install.ps1:169-173`).
- `clawg stop` no mata el proceso: a los 15 s devuelve `StopTimeout` (`stop.rs:15, 37`).
- Un puente MCP que arranca durante la actualización puede levantar un daemon de la versión vieja (`crates/clawg-cli/src/mcp/mod.rs:86-117`).
- Sólo se para prod.
- **Resultado:** la versión vieja sigue sirviendo con el lock tomado. No hay pérdida de datos, pero sí confusión sobre qué versión está corriendo.

### 5. BORRADO POR ERROR: no se puede revertir

- El texto queda. El vector (`admin.rs:211-213`) y la entrada FTS (trigger de V8) se borran.
- No hay undelete por MCP, HTTP ni CLI. Poner `deleted_at = NULL` a mano no alcanza, porque falta el vector y `reembed_all` sólo recorre filas que tienen vector.
- **El delete no tiene scope:** cualquier agente con la herramienta puede borrar cualquier memoria, de a una (`crates/clawg-cli/src/mcp/server.rs:361-373`).
- **No hay borrado por proyecto.** Sólo se borra fila por fila, por MCP o por `DELETE /v1/memories/{kind}/{id}` (`crates/clawg-cli/src/daemon/api.rs:347-349`). Las `user_preference` guardan el `project_id` de la carpeta desde donde se guardaron, como cualquier otro save (`crates/clawg-core/src/memory/store_insert.rs:179`). Se ven desde todos los proyectos sólo por una cláusula de la consulta (`search.rs:119-122`). Por eso un borrado ingenuo por `project_id` también borraría preferencias globales.

### 6. Menores

- **Base vacía si no hay home.** Si no se puede resolver el home, se usa `./.clawg` sin avisar (`paths.rs:148-152`).
- **Migración e historial no son atómicos** (`grouped=false`). Un kill en una ventana mínima hace que la migración se vuelva a correr. Con las de `ALTER TABLE ADD COLUMN` (V4, V6), el daemon falla con "duplicate column" en cada arranque.
- **`workspace.md` se reescribe sin atomicidad** en cada `recall_context` (`crates/clawg-core/src/memory/workspace.rs:89`). Un kill puede truncar la sección USER, que edita el usuario.
- **Mensajes engañosos:**
  - downgrade o checksum: "no clawg daemon is running" (`server.rs:231`);
  - contrato: "search will work in a minute" (`server.rs:235-236`);
  - la causa real sólo aparece en `logs/serve.log`.
- **Checksums y CRLF.** Una base migrada por un binario compilado desde un checkout CRLF no abriría con uno LF. **No está verificado contra la base real:** hace falta autorización para revisar una copia.
- **Falta un test que fije los checksums de V1–V8.**

## Alcance candidato (partes)

| Parte | Qué | Cubre |
|---|---|---|
| A. Apagado sin pérdida | Drenaje acotado del worker de ingesta, flush final del `AccessTracker`, checkpoint y cierre explícito de la conexión. Corregir el doc de `stop.rs` | 1 |
| B. Red de seguridad | Snapshot automático antes de aplicar migraciones pendientes y antes de un re-embed, con la API de backup online de SQLite. Más `clawg backup` y `clawg restore` (restore con el daemon parado y el lock tomado). Retención de N copias en `<data>/backups/` | 3 |
| C. Actualización verificada | El instalador confirma que el daemon paró (código de salida y `clawg status`) y aborta si no. Mensajes claros de downgrade, checksum y contrato en `clawg status` y en MCP | 4, 6 |
| D. Cambio de modelo con camino | `clawg reembed`, con backup previo, conectado a `reembed_all`. Un test que fije el contrato actual, para que cambiarlo sea deliberado. Mientras la memoria esté desactivada, los saves se rechazan (coordinado con ingest-durability) | 2 |
| E. Borrado recuperable | `restore_memory` por MCP y CLI: limpia `deleted_at`, re-embebe y re-inserta el FTS. El scope del delete lo define visibility-policy | 5 |
| F. Endurecimientos | Error si no hay home. Migraciones atómicas con su historial (`set_grouped(true)`, a verificar con vec0 dentro de una transacción). Test de checksums fijados. Escritura atómica de `workspace.md` (tmp + rename). Una regla nueva en `config.yaml`: toda migración trae un test de preservación de datos | 6 |

Agrupación sugerida para la propuesta:

1. **A + F**: apagado y escritura seguros. Chico, va primero porque es la única pérdida real de hoy.
2. **B + C**: red de seguridad y actualización verificada.
3. **D**: cambio de modelo.
4. **E**: borrado recuperable. Puede ir junto con visibility-policy.

## Decisiones abiertas

- [ ] **Backups.** ¿Sólo antes de migrar o re-embeber, o también periódicos (por ejemplo, uno diario desde el worker de mantenimiento)? ¿Cuántos se retienen? ¿Dentro del directorio de datos o en otra ubicación configurable? Cada backup es otra copia en claro de toda la base, incluido cualquier secreto guardado por error: una ubicación fuera del directorio de datos (por ejemplo, una carpeta sincronizada a la nube) lo saca de la máquina. Ver secret-hygiene.
- [ ] **Mecanismo de backup:**
  - la feature `backup` de rusqlite: es una feature de una dependencia que ya existe, no una dependencia nueva;
  - `VACUUM INTO`: queda por verificar que funcione con vec0 registrado en la conexión.
- [ ] **Restore.** ¿Comando dedicado, o procedimiento manual documentado?
- [ ] **Borrados.** ¿Restaurables siempre (sin purga, como hoy), o papelera con purga explícita? La pregunta también tiene un lado de privacidad: sin purga, un secreto guardado por error queda en texto plano para siempre. secret-hygiene propone una purga de una fila, sólo por CLI y nunca por MCP; las dos decisiones se toman juntas.
- [ ] **Borrado por proyecto.** ¿Hace falta "olvidar un proyecto" sin perder lo global? Si se agrega:
  - no toca las `user_preference` ni lo que visibility-policy defina como global;
  - se deshace en bloque con E;
  - es sólo por CLI, porque borrar en masa lo decide el usuario y no el agente;
  - depende de project-identity: con el `project_id` actual, un mismo repo puede estar partido en varios ids, y el borrado se quedaría corto.
- [ ] **Perfil dev.** ¿El instalador también para el perfil dev?
- [x] **¿Se separa en los cuatro cambios sugeridos?** Sí: `proposal.md` cubre A + F. B + C, D y E siguen acá hasta tener su propio cambio.

## Cómo se mide

Tests RED primero:

1. Un stop con eventos encolados: los eventos quedan persistidos.
2. Abrir una base con migraciones pendientes: existe un snapshot, y restaurarlo devuelve los mismos datos.
3. Un snapshot con WAL activo incluye las escrituras recientes.
4. Una base sembrada en V8 más una migración nueva: las filas quedan intactas. Es el patrón para las migraciones futuras.
5. Delete y después restore: la memoria vuelve a aparecer en `search_memory`.
6. Sin home resuelto: error, y no `./.clawg`.
7. Checksums fijados de V1–V8.

**Prueba manual de actualización real** sobre el perfil dev, con autorización: instalar la versión N, guardar, actualizar a N+1, y verificar el conteo de filas y que una búsqueda devuelva lo guardado.
