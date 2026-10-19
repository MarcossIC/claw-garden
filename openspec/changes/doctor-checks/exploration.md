# doctor-checks — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | 26 client-setup (sólo el chequeo del registro MCP), 29 project-root-guard (sólo el de colisión de `project_id`); el resto no depende de nada; los invariantes de estado suponen 27 y 49 (archivados) y se coordinan con 53 | exploración (borrador) |

**Tesis:** `clawg doctor` hoy responde una sola pregunta: qué `clawg.exe` ejecuta la terminal. Todo lo demás que puede dejar a un usuario con una memoria que "no anda" se descubre por accidente: leyendo el log, probando `memory_status` o viendo que una búsqueda vuelve vacía. Los datos casi nunca están en riesgo. Lo que falta es una sola salida que diga qué está mal, por qué, con qué evidencia y cuál es el siguiente paso seguro, sin tocar la base si no se pide.

## Qué es `doctor` hoy

- Implementación: `crates/clawg-cli/src/doctor.rs`. Es una función pura, `analyze` (`doctor.rs:46`), sobre una lista de directorios del PATH y un predicado `exists`. La lectura del entorno vive en `run` (`doctor.rs:112`).
- **No abre el store** ni nada bajo el directorio de datos. Está dicho en el comentario de `run` (`doctor.rs:111`) y en `.claude/CLAUDE.md` (comando `doctor`).
- La salida es el JSON de `Report`: `exe`, `version`, `profile` y, aplanado, `installs`, `winner`, `shadowed`, `duplicates` (`doctor.rs:94-101`).
- Salida del proceso: 0 si `is_healthy()` (primer hit = binario actual y sin copia duplicada), 1 si no (`main.rs:181-185`).
- El comando sólo acepta `--profile` (test `doctor_takes_only_a_profile`, `cli.rs:633`).
- **Premisa parcialmente falsa: "doctor es de Windows".** El nombre del ejecutable ya se elige con `cfg!(windows)` (`doctor.rs:117`) y `identity` distingue Windows de lo demás (`doctor.rs:84-91`). Lo que sí es sólo de Windows es `clawg path` (`user_path.rs` usa el registro de Windows). El chequeo de PATH es neutral hoy.

## Problema (verificado contra el código)

Cada fila dice si el problema es real, y qué lo detecta hoy.

| Problema | ¿Real? | Cómo se ve hoy |
|---|---|---|
| **Registro MCP a un binario que ya no existe** | **No verificable en este repo.** `rg` por `mcpServers`, `claude.json` y `.mcp.json` en `crates/` no encuentra nada: ningún código lee la configuración de un cliente. Que pase depende de cómo se registre (por nombre en el PATH o por ruta absoluta) y eso lo decide 26 | El cliente muestra el servidor como caído; clawg no se entera |
| **Memorias partidas entre dos nombres de agente del mismo cliente** | **Plausible, y la mitad es determinista.** El id es `--agent`, o `clientInfo.name`, o `"mcp"` (`FALLBACK_AGENT_ID`, `mcp/server.rs:55`; `.claude/CLAUDE.md`). Un cliente que cambia de nombre entre versiones deja filas bajo dos ids. Las búsquedas siguen alcanzando las del otro agente porque el sharing por defecto lee todo (`reach`, `daemon/api.rs:813`), pero quedan marcadas `by <autor>` y de sólo lectura, así que el agente no puede corregirlas ni borrarlas | `clawg memories stats`; nada avisa |
| **Colisión de `project_id`** (16 bits) | **Parcial.** El id es `{nombre}-{4 hex}` (`identity.rs:146-153`): 65.536 valores por nombre de carpeta. Al **registrar** un proyecto la colisión ya se detecta: `register_project` compara `root_path` y devuelve `RootCollision` (`project.rs:83-87`, `:112-113`), que la API traduce a 409 (`daemon/api.rs:1033`). Pero el segundo root nunca queda anotado: la tabla `project` tiene `id` como clave (`V9__project.sql`), así que el store no guarda rastro de la colisión. Sólo queda en la respuesta 409 | El 409 llega al puente; no hay nada persistente que `doctor` pueda leer. Lo que sí puede chequear es que `derive_project_id(root_path) == id` en cada fila de `project` |
| **Cabecera de un save largo que `purge` dejó** | **Real, ya está en el backlog del README.** `purge` borra la fila, su vector y `memory_derivation` (`secrets.rs:102-107`) y no toca `memory_piece`. La tabla guarda `context` (los primeros tokens del save) por cada pedazo 2..N (`V11__memory_piece.sql:11-17`), así que el texto purgado puede seguir en `memory.db`, contra lo que promete `purge` (`secrets.rs:78-80`) | Nadie lo ve |
| **Disputas huérfanas** | **Mayormente falso como problema persistente.** La limpieza diaria las devuelve a `active` (`release_orphaned_disputes`, `maintenance.rs:118`, `:211`), y el delete y el purge liberan las suyas (`secrets.rs:109-111`). Un chequeo sólo vería la ventana entre un borrado y la próxima limpieza. Valor bajo; si 24 introduce pares candidatos, ahí sí hay una tabla nueva que puede quedar sucia | Se arregla solo |
| **Contrato de embedding distinto del modelo corriendo** | **Real, y ya visible por otro lado.** `ensure_embedding_contract` compara modelo, dimensión y los dos prefijos byte a byte (`db.rs:309-358`). Un mismatch deja el embedder en `failed` no reintentable (`bootstrap.rs:385-396`) y `/health` y `clawg status` lo muestran (`daemon/api.rs:424`, `:610`). Lo que `doctor` aportaría: decirlo **con el daemon parado**, y sin cargar el modelo. Ahora se puede: la identidad ya es una constante, `E5_MODEL_ID` (`embedder.rs:115`) | `clawg status`, sólo con daemon vivo |
| **`ingest-pending.jsonl` sin reproducir** | **Parcial.** El spill se reproduce solo en cada arranque del runtime (`spill::load`, `runtime.rs:355`). Un archivo presente significa que el daemon no arrancó desde el último `stop`: informativo. Lo que sí es un problema real es `ingest-pending.rejected.jsonl`: son líneas que no se pudieron parsear, "nothing deletes it" (`spill.rs:16-18`), y son saves de alguien | Sólo el log del arranque (`spill.rs:78-82`) |
| **Último backup viejo** | **Real.** El único snapshot que se toma solo es el de migrar, restaurar o re-embeber (`Reason`, `backup.rs:119-128`). `Manual` sólo sale de `clawg backup` (`backup.rs:317`) y no hay ninguno periódico (`rg` de `snapshot_into` no encuentra otro llamador). Un usuario que nunca corre `backup` puede tener un único snapshot de hace meses | Nadie |
| **La poda puede sacar el snapshot que era la vuelta atrás** | **Real pero improbable, y no la arregla `doctor`.** `to_prune` ordena **por nombre** y conserva los 5 automáticos más nuevos sin mirar el motivo (`backup.rs:235-243`). Cinco snapshots automáticos posteriores (migraciones, restores, re-embeds) desplazan al `pre-migration`. Los tests fijan el conteo (`pruning_keeps_the_newest_automatic_snapshots_and_every_manual_one`, `backup.rs:703`; `an_automatic_snapshot_prunes_older_automatic_ones`, `:680`) y **no** el motivo: sus nombres alternan `PreMigration` y `PreRestore` (`backup.rs:693`) y la aserción acepta que se pode cualquiera de los dos. Además el comentario de `AUTO_KEEP` sigue diciendo "pre-migration, pre-restore" (`backup.rs:227`) aunque `is_automatic` ya cubre `pre-reembed` (`:143-145`) | Nadie |
| **`sharing.json` inválido** | **Parcial: no es silencioso del todo.** Comparte nada (`Reach::Own`) y escribe `log::warn!("…; nothing is shared")` en cada búsqueda (`daemon/api.rs:813-820`). `memory_status` lo dice: `shared: none (sharing.json is not valid…` (test en `mcp/server.rs:2419-2421`), y `clawg sharing` muestra el error y se niega a sobrescribir el archivo (`sharing.rs:21-27`). Lo que falta: que `doctor` y `clawg status` lo digan, que es donde el usuario va a mirar cuando algo no se comparte | `memory_status`, `clawg sharing`, el log |

Las invariantes de estado se tratan aparte, en la sección siguiente. Resumen de la tabla de arriba: de diez problemas, **cinco son reales y sin salida visible** (backup viejo, rechazados del spill, cabecera de `purge`, agentes partidos, registro MCP si 26 lo hace legible), **tres ya tienen otra vía** y lo que cambia es dónde se lee (contrato, `sharing.json`, spill pendiente), **uno se arregla solo** (disputas) y **uno es una falla de diseño de la poda** que se corrige en `backup.rs`, no se diagnostica (snapshot desplazado).

## Invariantes del estado de `memory` (agregado tras 27 y 49)

Esta exploración es anterior a topic-revisions (27) y revision-aware-delete (49), que dejaron a `memory` con reglas que ningún esquema impone. Verificado en `crates/clawg-core/migrations/`: no hay clave foránea ni `CHECK` sobre `status` ni `superseded_by` (`V1__init.sql`; «No foreign keys» en `V15__memory_conflict.sql:8`, también `V10__synthesis.sql:5` y `V11__memory_piece.sql:8`); el índice de dedup `V6__content_hash.sql:9-16` no es único. Los únicos `CHECK` son los de `scope`, `memory_conflict.state`/`verdict`, `ingest_outbox.state`, `supersede_cause`, `project_rekey.tbl` y `embedding_contract`. SQLite se compila con `SQLITE_DEFAULT_FOREIGN_KEYS=1`, pero sin clave declarada eso no suma nada.

Las invariantes las mantiene el código, y `status`, `superseded_by` y `deleted_at` los escriben siete módulos (la mayoría por `SUPERSEDE_SQL` y `REVIVE_SQL`): `dispute.rs` (`settle_disputed`, `release_challengers`), `admin.rs` (delete, restore, supersede), `revival.rs` (revive y reapuntado), `topic.rs:131`, `store_insert.rs:508-533` (estado inicial), `secrets.rs:105` (borrado duro) y `maintenance.rs` (`settle_disputed`, `revive_hidden`). Cada chequeo es una consulta SQL de sólo lectura:

| # | Invariante | Nota |
|---|---|---|
| 1 | A lo sumo un save vivo por (agente, proyecto, `topic_key`) | Los pedazos de un save comparten la clave: se cuentan saves, no filas |
| 2 | `status = 'superseded'` implica `superseded_by` con una fila existente | Decidir cómo se trata una fila escondida por un veredicto cuyo ganador se borró |
| 3 | `revived_from` nombra una fila existente | |
| 4 | Dos filas vivas no repiten `content_hash` en el mismo (agente, proyecto) | |
| 5 | Todo vínculo de `memory_piece` apunta a filas de `memory` existentes | Es el chequeo de cabeceras huérfanas ya planeado: se fusiona |
| 6 | Una fila `disputed` nombra al menos una fila viva en `disputed_against` | |

Caminos que hoy rompen alguna, y que el chequeo debe encontrar en bases ya existentes: un save que falla a la mitad (53, outbox-lifecycle), el `purge` de la versión vigente de un tema (51, secret-coverage: deja `superseded_by` colgante), el re-key de proyectos que une dos scopes sin reconciliar (30, project-clone-identity), un `restore` que corre contra el worker y un veredicto sobre un solo pedazo (ambos en el backlog).

## Lo que ya existe y se reutiliza

- **Un camino de lectura sin lock y compatible con el daemon.** `secrets::scan` abre con `open_for_probe`, sin lock y sin escribir: "works beside a running daemon: WAL lets a reader in" (`secrets.rs:30-52`; `open_for_probe`, `backup.rs:502-512`). Es el modelo para los chequeos que leen filas.
- **Un camino de escritura bajo lock exclusivo.** `purge` toma `StoreLock::acquire(base_dir, StoreHolder::Maintenance)`, abre sin migrar, verifica `schema_plan` y recién ahí escribe (`secrets.rs:81-94`). Es el modelo para `apply`.
- `read_history` y `schema_plan` (`backup.rs:20`, `:88-115`) dicen si el esquema está al día sin abrir con migraciones.
- `parse_snapshot_name` y el formato con fecha del nombre (`backup.rs:180-210`): la edad de un snapshot sale del nombre, sin mirar el reloj del sistema de archivos. Hoy `parse_snapshot_name` devuelve sólo el motivo; la fecha habría que parsearla (función pura nueva).
- `sharing::load` y `SharingError`, con mensajes ya redactados para el usuario (`sharing.rs:23-38`).
- `StoreLock::current_owner` y `memory.owner` (`store_lock.rs:179`) dicen quién tiene el store sin abrirlo; `probe_running` (`client.rs:937`) dice si hay daemon.
- `GET /health` (abierto) da el estado del embedder y de la versión (`daemon/api.rs:610`).
- `spill::SPILL_FILE` y `REJECTED_FILE` (`spill.rs:14-18`).
- `derive_project_id` (`identity.rs:11`) es pura: sirve para comprobar `project.id` contra `project.root_path`.
- `Report` ya es un `Serialize` con campos aplanados: se le puede **sumar** una clave sin romper al que lee las existentes.
- Visibilidad: `open_for_probe`, `open_existing` y `read_history` son `pub(crate)` en `clawg-core`. `doctor.rs` vive en `clawg-cli`, así que los chequeos del store necesitan una función **pública de `clawg-core`** (como lo es `secrets::scan`), no abrir esas funciones.

## Enfoques posibles

### A. Sobre sin tocar el store: sólo archivos y PATH

Chequeos que miran el directorio de datos (spill, `sharing.json`, edad y motivos de los snapshots por nombre de archivo, existencia del binario registrado) y nunca abren `memory.db`.

- A favor: conserva la promesa actual ("no abre el store"); trivial de coexistir con el daemon; cero riesgo.
- En contra: no ve el contrato, los agentes, `memory_piece` ni `project`. Deja fuera lo más valioso.

### B. Sobre + lectura del store con `open_for_probe` (recomendado como base)

A, más una función pública en `clawg-core` que abre en sólo lectura y sin lock, como `scan`, y devuelve hechos (contrato guardado, agentes con su cantidad de filas, cabeceras de `memory_piece` sin fila, `project` inconsistentes). Los chequeos son funciones puras sobre esos hechos.

- A favor: funciona con el daemon vivo o parado; reutiliza un patrón probado; todo es diagnóstico, nada escribe.
- En contra: `doctor` deja de "no abrir el store". Hay que decirlo en `.claude/CLAUDE.md`, en el comentario de `run` y en la spec. Una lectura WAL con el daemon escribiendo es consistente pero puede ir un instante atrás.
- Costo: la sonda abre vec0 (`ensure_vec0_registered`, `secrets.rs:39`). Es barato, pero un `doctor` que tarda por abrir la base es otro cambio de comportamiento.

### C. B + reparación (`plan` / `--dry-run` / `apply`)

B más un modo que corrige lo que es seguro corregir. Reglas:

1. `plan` (el modo por defecto) lista las reparaciones; no escribe.
2. `apply` exige que el daemon esté parado, toma el lock `Maintenance`, **vuelve a validar** cada hallazgo dentro del lock (el estado pudo cambiar entre el plan y el apply) y recién ahí escribe.
3. Antes de escribir, snapshot (`Reason::Manual` o uno nuevo). **Excepción verificada:** `purge` no hace snapshot a propósito, porque sería otra copia del texto que se quiere sacar (`secrets.rs:78-80`). La reparación de cabeceras de `memory_piece` borra texto residual de un purge: tampoco debe copiarlo a un snapshot nuevo. Cada reparación declara si snapshotea.
4. Nunca destructivo sin confirmación explícita (`--yes`, como `purge`).

- A favor: cierra el ciclo; el usuario no tiene que escribir SQL.
- En contra: es la parte que toca datos del usuario. Cada reparación es una superficie con su test de preservación (`config.yaml`, regla de datos guardados). Sin 24 y 26 hay pocas reparaciones seguras que valgan el costo: la de cabeceras de `purge` y poco más.

### D. Herramienta MCP `doctor` (o ampliar `memory_status`)

Para que el agente diagnostique sin pedirle al usuario que corra un comando.

| Opción | Costo | Notas |
|---|---|---|
| Herramienta nueva | Una descripción y un schema más en el contexto de **cada** sesión. El repo no mide ese costo | Rompe tests que fijan la lista y el conteo: `every_tool_is_registered_under_its_wire_name` (`server.rs:1497-1521`), `the_viewer_is_not_exposed_to_the_model` (`server.rs:1528-1549`) y `handshake.rs:385`, `:442`. Hay que cambiarlos a propósito; con 32 quality-guards el cambio queda como diff explícito |
| Ampliar `memory_status` | Cero herramientas | Ya cuenta el daemon, el perfil y el `shared:` (`server.rs:756-776`). Sumar una línea por hallazgo grave es barato. Mezcla "estado" con "diagnóstico", pero ya es la herramienta a la que se recurre cuando algo falla |
| Nada en MCP | Cero | `doctor` es del usuario. El agente sólo ve lo que `memory_status` ya dice |

Recomendación: **no** agregar herramienta. Si se quiere que el agente se entere, ampliar `memory_status` con los hallazgos de severidad `error`, que salen de los mismos chequeos puros.

## Sobre de resultado uniforme

Cada chequeo devuelve el mismo registro, y `doctor` imprime una lista:

```
{ check_id, severity, reason_code, evidence, safe_next_step, requires_confirmation }
```

| Campo | Contenido |
|---|---|
| `check_id` | Estable, en inglés: `path.shadowed`, `store.contract_mismatch`, `backup.stale`, `spill.rejected_lines`, `sharing.invalid`, `agents.split`, `pieces.dangling_header`, `project.id_mismatch`, `mcp.binary_missing` |
| `severity` | `ok`, `info`, `warn`, `error` |
| `reason_code` | Un código estable por causa; el texto para humanos lo arma una función aparte |
| `evidence` | Hechos medidos (ruta, conteo, ids, fecha), **nunca contenido de filas**: `clawg secrets` ya fija la regla de listar por id y tipo y no el secreto (`secrets.rs:21-24`) |
| `safe_next_step` | Un comando que se puede copiar, o vacío |
| `requires_confirmation` | `true` si el paso toca datos |

Compatibilidad: el JSON actual es un objeto con `exe`, `version`, `profile`, `installs`, `winner`, `shadowed`, `duplicates`. Se **suman** claves (`checks`, `summary`) y no se renombra ninguna. Lo que lea las viejas sigue andando. Los chequeos de PATH pasan a ser también dos entradas del nuevo arreglo, sin sacar los campos de antes.

Códigos de salida: hoy 0 sano, 1 no sano (`main.rs:181-185`). Se mantiene 1 para PATH. Para los chequeos nuevos hay una decisión abierta (si `warn` falla el proceso).

## Enfoques descartados

- **Chequear la dimensión o el modelo cargando el embedder.** Cargar ONNX para decir "coincide" tarda segundos, puede descargar el modelo y no aporta: la identidad es una constante (`embedder.rs:115`) y la guardada está en una fila de `embedding_contract`.
- **Reparar el contrato (re-embeber) desde `doctor`.** El re-embed de un cambio de pooling ya corre solo en el arranque con snapshot previo (`bootstrap.rs:279-305`, `specs/storage`). Un cambio de modelo no está planeado (ver model-reembed). `doctor` informa, no re-embebe.
- **Detectar disputas huérfanas.** Las libera la limpieza diaria (`maintenance.rs:118`) y el delete y el purge en la misma transacción. Un chequeo vería ruido.
- **Reparar la poda desde `doctor`.** `doctor` no puede ver un snapshot que ya no existe. Se arregla en `to_prune` (conservar el más nuevo de cada motivo), que es un arreglo chico de `backup.rs` con su test.
- **Un daemon "modo diagnóstico" con endpoint propio.** Un endpoint nuevo en la API por algo que se puede leer en sólo lectura (B) es superficie sin uso real. La API ya da `/health`.
- **Fusionar agentes por heurística de nombre.** Decidir que `claude-code` y `Claude Code` son el mismo cliente es una política, no un hecho. `doctor` lista los agentes con sus conteos y marca el caso determinista (existe `mcp`, el id de respaldo); fusionar es de 26 (alias) y del usuario.

## Interacciones

- **history-import (44).** Este cambio debería correr antes: revisar el store antes y después de una importación es el uso más claro de los chequeos de invariantes.
- **topic-revisions (27) y revision-aware-delete (49), archivados.** Son quienes introducen las invariantes 1 a 3; el chequeo es su red de seguridad en bases ya existentes.
- **outbox-lifecycle (53).** Los saves que fallan a la mitad y las filas `failed` son fuente de violaciones; los hallazgos se coordinan con lo que 53 decida sobre ellas.
- **secret-coverage (51) y project-clone-identity (30).** Cada uno arregla de origen un camino que rompe una invariante (el `purge` de la versión vigente, el re-key que une scopes); `doctor` sólo encuentra lo que ya quedó guardado.
- **local-isolation (50), local-transport (58) y windows-pipe (59).** `doctor` puede informar, sin abrir el store, el modo del directorio de datos (el A3 de 50), el largo de la ruta del socket frente al tope de `sun_path` (108 bytes en Linux, 104 en macOS) y, en Windows, el descriptor de seguridad de la pipe del daemon que corre.

## Alcance candidato

1. Una función pública en `clawg-core` que abre el store en sólo lectura y sin lock (como `secrets::scan`) y devuelve **hechos**: fila de `embedding_contract`, agentes y conteos, cabeceras de `memory_piece` sin fila de `memory`, filas de `project`, versión de esquema.
2. Chequeos puros `(hechos) -> Vec<Finding>`, uno por `check_id`, cada uno con tests con fixtures y sin sistema de archivos:
   - PATH (lo de hoy, pasado al sobre);
   - contrato guardado contra `E5_MODEL_ID` y prefijos;
   - edad del último snapshot y motivos presentes;
   - `ingest-pending.rejected.jsonl` presente, y `ingest-pending.jsonl` presente con el daemon parado (info);
   - `sharing.json` ilegible;
   - agentes: existencia de `mcp` y lista con conteos;
   - cabeceras huérfanas de `memory_piece`;
   - `project.id` contra `derive_project_id(root_path)`;
   - las seis invariantes de estado de la sección «Invariantes del estado de `memory`», con las cabeceras huérfanas fusionadas en la 5;
   - filas `failed` del `ingest_outbox` (cuenta y `operation_id`, sin contenido), coordinado con 53;
   - el modo del directorio de datos y el largo de la ruta del socket (con 50 y 58).
3. El sobre uniforme y el JSON compatible. Salida legible opcional además del JSON (decisión abierta).
4. Con 26: chequeo del registro MCP (binario existe, agente fijo) sobre un parser puro del texto de configuración que 26 define.
5. Con 29: chequeo de colisión de `project_id` si 29 hace que quede registrada.
6. Si se elige C: `apply` sólo para cabeceras huérfanas de `memory_piece`, con lock `Maintenance`, revalidación y sin snapshot (por la razón de arriba). Es la única reparación que se justifica con lo verificado.
6b. Exposición de los hallazgos de invariantes: por `clawg doctor` con el store (enfoque B, `open_for_probe`) y/o una línea en `memory_status`; también podría correr en la pasada de mantenimiento y dejar un log.
7. Docs: `.claude/CLAUDE.md` (el `doctor` ya no es "no abre el store" y qué más imprime) y el comentario de `run`.

### No-alcance

- Arreglar `clawg purge` para que borre `memory_piece` (queda en el backlog del README; es el arreglo de origen y `doctor` sólo cubre lo que ya quedó guardado).
- Arreglar la poda de snapshots (`to_prune`).
- Re-embeber, migrar o restaurar desde `doctor`.
- Fusionar o renombrar agentes (26).
- Registrar el servidor MCP en un cliente (26).
- Una herramienta MCP nueva.
- Un endpoint nuevo en la API del daemon.
- Backups periódicos automáticos (otra decisión de producto: crea copias de lo que el usuario borró).
- Reparar el contrato de embedding.

## Decisiones abiertas

Son del usuario.

- [ ] **¿`doctor` pasa a abrir el store en sólo lectura?** Es un cambio de promesa (B). La alternativa es A (sólo archivos), que pierde contrato, agentes y piezas. Recomendado: B.
- [ ] **¿Un subcomando o un flag para lo que lee el store?** `clawg doctor` rápido (PATH y archivos) y `clawg doctor --store` (o `--deep`) para el resto, o todo junto. Lo primero preserva el costo y el contrato de hoy.
- [ ] **Código de salida.** ¿1 sólo con `error` y 0 con `warn`? ¿O 2 para `warn`, para que un script distinga? Hoy hay 0 y 1 (`main.rs:33`).
- [ ] **¿Se incluye reparación (C) en este cambio?** Recomendado: sólo la de cabeceras de `memory_piece`, o nada y dejar C para cuando 24 y 26 den más qué reparar.
- [ ] **Umbral de "backup viejo".** ¿7, 14, 30 días? ¿Y vale la pena avisar si nunca hubo uno `manual`? Es una política, no un hecho.
- [ ] **¿Los hallazgos `error` llegan al agente por `memory_status`?** Recomendado: sí, una línea por hallazgo. Alternativa: no, `doctor` es del usuario.
- [ ] **Salida legible.** ¿Sólo JSON (como hoy) o también un resumen de texto con `--format text`?
- [ ] **El registro MCP.** ¿Hasta dónde llega el chequeo? Depende de qué escribe y qué puede leer 26 (¿lee la config del cliente o sólo el archivo que 26 mismo escribió?).
- [ ] **¿Orden de entrega?** El chequeo de PATH + archivos + contrato + backups no depende de 26 ni de 29. ¿Se entrega en dos tandas, o todo junto al final?
- [ ] **Invariantes de estado.** ¿Corren sólo en `clawg doctor --store`, también en `memory_status`, o también en la pasada de mantenimiento con un log? Y qué severidad tiene cada una (¿una fila `superseded` con ganador borrado por veredicto es `warn` o `info`?).
- [ ] **Orden respecto de 44.** Conviene que este cambio vaya antes de history-import (44): sirve para revisar el store antes y después de una importación.
- [ ] **Snapshots y su motivo.** ¿Se corrige `to_prune` en este cambio (conservar el más nuevo por motivo) o en el backlog como arreglo suelto?

## Cómo se mide

Tests RED primero. Todo lo puro sin sistema de archivos ni base.

1. **Sobre compatible:** el JSON de `Report` conserva `exe`, `version`, `profile`, `installs`, `winner`, `shadowed`, `duplicates` con los mismos tipos que hoy; las claves nuevas se suman.
2. **Un test por chequeo, estado sano y estado roto**, sobre hechos fabricados: contrato igual y contrato con modelo, dimensión y prefijo distintos; último snapshot de hace 2 y de hace 90 días; con y sin snapshot; `ingest-pending.rejected.jsonl` presente; `sharing.json` con JSON roto, versión 2 y nombre vacío (`SharingError`); agente `mcp` presente; `memory_piece` con y sin fila de `memory`; `project.id` que no corresponde a su `root_path`.
3. **Sin contenido:** un fixture con una fila que contiene un secreto: ni `evidence` ni `safe_next_step` lo contienen.
4. **Sondeo con el daemon vivo:** con un `StoreLock` de `Daemon` tomado (patrón de `client.rs:1473`), la lectura funciona y no escribe: el archivo `memory.db` y su WAL no cambian de contenido.
5. **Base sembrada en la versión anterior** (regla de `config.yaml`): una base con filas de las tres tablas, un `memory_piece` huérfano y un snapshot viejo; `doctor` la lee sin migrar, no modifica `memory.db` y los hallazgos son los esperados.
6. **Esquema pendiente o más nuevo:** un store con migraciones pendientes o de un `clawg` más nuevo se lee sin tocarse y el hallazgo lo dice (`schema_plan`, `backup.rs:88`).
7. **Si hay `apply`:** un `apply` con el daemon vivo se niega y no escribe; entre `plan` y `apply` cambia el hallazgo y el `apply` no repara lo que ya no corresponde (revalidación bajo lock); el texto de las cabeceras ya no está en `memory.db` ni en su WAL (el mismo criterio de `purge`).
8. **Salida del proceso:** 0 sin hallazgos `warn` o `error`, y el código acordado en los otros casos.
9. **Invariantes de estado:** para cada una de las seis, una base fabricada que la viola (dos saves vivos con el mismo `topic_key`; `superseded_by` hacia un id borrado de verdad; `revived_from` colgante; dos filas vivas con el mismo hash; un vínculo de `memory_piece` sin fila; una fila `disputed` sin pareja viva) da el hallazgo, y la base sana no da ninguno. Los pedazos de un mismo save no cuentan como violación de la 1.
10. **Si se corrige `to_prune`:** un test donde seis automáticos posteriores NO desplazan al `pre-migration` más reciente. Hoy falla.

No toca ranking ni retrieval: `rag-eval --diff` no aplica.

## Riesgos

- **Cambiar la promesa de `doctor`.** Quien hoy lo corre en un script lo espera instantáneo y sin efectos. Abrir la base puede tardar y, con la base en una versión más nueva, devolver un hallazgo en vez de un éxito. Mitigación: flag aparte o dejar el chequeo rápido por defecto.
- **Falsos positivos con el daemon escribiendo.** La lectura WAL es consistente pero puede ir atrás. Un hallazgo de "cabecera huérfana" durante un purge en curso sería ruido. Mitigación: los chequeos de integridad piden que no haya daemon, o marcan `info` cuando hay otro titular (`memory.owner`).
- **Hallazgos que filtran contenido.** `doctor` se pega en issues y chats. Sólo ids, tipos, conteos y rutas.
- **Reparar de más.** Cada `apply` es código nuevo sobre datos del usuario. Es la razón de empezar por ninguna o una sola.
- **Heurísticas de agentes.** Un falso "split" asusta al usuario sin motivo. Sólo se marca lo determinista.
- **Dependencias blandas.** Si 26 o 29 se retrasan, los chequeos que dependen de ellos quedan fuera sin bloquear el resto; el estado del cambio lo debe decir.
- **Falsos positivos de invariantes durante una escritura.** Con el daemon escribiendo, un save a la mitad puede verse como violación de la 1 o la 4. Mitigación: la lectura WAL es consistente por transacción, y los chequeos marcan `info` si hay otro titular (`memory.owner`).
- **Estado del repo.** El `.claude/CLAUDE.md` describe `doctor` como "no abre el store" (frase explícita); si cambia, hay que corregirlo en el mismo commit que el comportamiento.
