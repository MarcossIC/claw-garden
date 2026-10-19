# secret-coverage — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (conviene antes de 43 y 44; ver «Interacciones») | exploración (borrador) |

**Tesis:** un secreto que llega a clawg se enmascara en todo lugar donde la regla puede verlo, y `clawg purge` saca un texto de todos los lugares donde clawg lo copió, y sólo ese texto. Hoy hay cuatro huecos verificados. El detector no conoce formatos frecuentes (claves de AWS, OpenRouter, Hugging Face, webhooks de Slack, claves PGP) porque sus patrones por contexto sólo se anclan en `password`. `purge` deja el texto en `workspace.md` y la cabecera de un save largo, y a la vez puede borrar saves aceptados de otros agentes, porque compara el outbox por subcadena. Y el `reason` de un veredicto se guarda sin enmascarar, en una columna que `clawg secrets` no mira. Y `purge` de la versión vigente de un tema deja las versiones anteriores escondidas para siempre, porque nadie las revive. Los cuatro rompen la promesa que el producto hace con los secretos, y una importación (44) los multiplica.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| Los formatos listados no se enmascaran | **Cierta para casi todos.** Se probó cada uno con `rg` (mismo motor que el crate `regex`) sobre los patrones de `crates/clawg-core/src/memory/redact.rs:80-113`. Sin match: `AWS_SECRET_ACCESS_KEY=<40>`, `OPENROUTER_API_KEY=<32>`, `"client_secret": "<34>"`, `HF_TOKEN=hf_<34>`, `xapp-1-…` (Slack sólo `xox[abposr]-`, `:86`), el webhook `hooks.slack.com/services/T…/B…/…`, `-----BEGIN PGP PRIVATE KEY BLOCK-----` (el patrón exige `PRIVATE KEY-----` pegado, `:95`), `whsec_…` (sólo `[sr]k_live_`, `:88`), `X-Api-Key: <32>` y `AccountKey=<base64>==`. Sí se enmascara `PGPASSWORD=` (`:103`). |
| El patrón `Password` sólo se ancla en `password`/`passwd` | **Cierta** (`redact.rs:101-104`, `\w*pass(?:word\|wd)`). Una clave como `client_secret` o `api_key` no entra, aunque el valor sea literal. |
| `purge` no toca `workspace.md` | **Cierta.** `purge` (`crates/clawg-core/src/memory/secrets.rs:82-137`) sólo recibe `base_dir` y abre una `Connection`; el workspace se regenera con un `MemoryStore` y un `WorkspaceManager` async (`crates/clawg-core/src/memory/workspace.rs:107-143`). La sección AUTO copia las `user_preference` con importancia ≥ 8 y tier 1 (`workspace.rs:255-263`) y los otros tipos que elige `fetch_auto_rows` (`:230`). |
| El texto de un `purge` se borra del outbox por subcadena y entre todos los agentes | **Cierta.** `DELETE FROM ingest_outbox WHERE instr(content, ?1) > 0` (`secrets.rs:114`), sin `agent_id`. El mismo criterio se usa para nombrar backups (`outbox_holds`, `secrets.rs:172`). |
| Una fila de `memory` no guarda el `operation_id` de su save | **Cierta.** `memory` sólo guarda `source_session_id` (`migrations/V1__init.sql:16`); `operation_id` está sólo en `ingest_outbox` (`V14__ingest_outbox.sql:13`, `UNIQUE (agent_id, operation_id)` en `:23`) y `worker.rs` no lo escribe en `memory`. |
| Una fila guardada puede ser un pedazo de un save largo, así que la subcadena es a propósito | **Cierta**, y hay una salida que la premisa no mencionaba: «una fila sin vínculo es un save de un solo pedazo» (`crates/clawg-core/src/memory/pieces.rs:1-7`). Para ésas, el outbox tiene el mismo texto entero. |
| El `reason` de un veredicto se guarda sin enmascarar | **Cierta.** `judge_pair` lo recorta (`dispute.rs:327`) y lo escribe en `memory_conflict.reason` (`:383`, `V15__memory_conflict.sql:19`); la ruta llega por `crates/clawg-cli/src/daemon/api.rs:1442`. |
| `redact(` se llama sólo en `runtime.rs` ~222/~447, `worker.rs` ~621, `project_docs/mod.rs` ~382, `admin.rs` ~616 | **Cierta**, con líneas exactas `runtime.rs:222`, `:447`, `worker.rs:635`, `project_docs/mod.rs:382`, `admin.rs:616`. Más `secrets.rs:46` (el escaneo) y los tests. No hay otra llamada en `crates/`. |
| `secrets::scan` lee sólo `memory.content` | **Cierta** (`secrets.rs:41`). |
| `purge` revive las versiones anteriores de un tema, como un `delete` | **Falsa.** `purge` (`secrets.rs:82-137`) hace `DELETE FROM memory` (`:105`) y `release_challengers` (`:117`, sólo si la fila estaba viva); no llama a `revive_after_delete_of_piece`, que sí llama `delete_memory_core` (`admin.rs:355`) |
| La pasada de mantenimiento recupera lo que `purge` deja escondido | **Falsa.** `revive_after_delete` no hace nada si la fila no existe con `deleted_at IS NOT NULL` (`revival.rs:86-93`) y `HIDDEN_SQL` une la fila borrada (`revival.rs:156-159`), así que nunca ve un id purgado. `drop_orphaned_pairs` (`maintenance.rs:248`) sólo limpia `memory_conflict`; nada limpia un `superseded_by` hacia un id que ya no existe |
| Cuatro formatos que se creían sin cubrir | **Dos matices.** `PGPASSWORD=` ya se cubría. `AWS_SECRET_ACCESS_KEY` no tiene prefijo fijo (40 caracteres base64), así que sólo se ve por su clave. |

## Problema (verificado contra el código)

### 1. El detector no ve claves que vienen por nombre o por prefijo

- Los patrones por contexto son dos: `Password` y `AuthHeader` (`redact.rs:101-108`). Cualquier otra clave de configuración (`*_SECRET`, `*_TOKEN`, `*_API_KEY`, `client_secret`, `AccountKey`, `X-Api-Key`) pasa entera, y con ella un archivo `.env` pegado en un save.
- Los formatos con prefijo fijo son 11 (`redact.rs:82-92`) más la clave privada, el JWT y la URL. Faltan los de Hugging Face (`hf_`, justo el servicio del que el proyecto baja su modelo: `crates/clawg-core/src/memory/model_download.rs:25`), Slack `xapp-` y webhooks, Stripe `whsec_` y la cabecera PGP.
- **Consecuencia sobre lo ya guardado.** `scan` clasifica con `redact` (`secrets.rs:46`), así que una regla nueva lista filas viejas sin código extra (`specs/secrets` R4 lo exige). Pero `restore_memory_core` rechaza restaurar una fila borrada que tenga un secreto (`RestoreError::HoldsSecret`, `admin.rs:397`, chequeo en `:616-619`). Una regla nueva puede entonces **bloquear la restauración de filas viejas** que antes volvían. No pierde datos (la fila sigue borrada, `purge` es a pedido), pero cambia lo que `clawg undelete` acepta.
- **Falso positivo = pérdida** (tesis de `redaction-precision`): el texto enmascarado es el único que llega a la base. Un patrón por clave que enmascare `token: abc123def` en prosa técnica destruye el texto sin vuelta atrás.

### 2. `purge` no saca «todo» y saca «de más»

- **a. `workspace.md`.** Si la fila purgada era una preferencia, decisión o procedimiento elegible, su texto está en la sección AUTO de `workspaces/<agente>/<proyecto>/workspace.md`, que se regenera sólo en `recall_context`. `purge` corre con el daemon parado y sin `MemoryStore`. La sección USER es del usuario y nunca se reescribe (`assemble_file`, `workspace.rs:494-584`); puede tener el texto, y sólo se la puede **nombrar**. La copia de un archivo legado a un id de proyecto nuevo nunca mueve el original (`workspace.rs:50-55`), así que el texto puede estar en dos archivos.
- **b. Subcadena entre agentes.** Un texto corto (`use pnpm`) borra todo save pendiente o `failed` de **cualquier** agente que lo contenga: saves aceptados con `201`/`202` que se pierden sin aviso. Es el mismo contrato que dice «no se pierde nada de lo aceptado» (`.claude/CLAUDE.md`, sección del outbox).
- **c. Cabecera de un save largo** (backlog de `openspec/README.md`). `memory_piece.context` guarda el principio del save (`V11__memory_piece.sql:4-8`, `:11-16`); `purge` no toca esa tabla (`secrets.rs:103-111`), así que purgar el pedazo 1 deja sus primeros tokens en los vínculos de los pedazos 2..N.
- **d. Fila `failed` del outbox sin salida** (backlog). Nada la reintenta ni la descarta (`outbox.rs:300-317` sólo la marca). No es un problema de secretos por sí mismo, pero el `last_error` (`IngestFailure::Failed(format!("embed failed: {e}"))`, `worker.rs:766`, `:803`) puede citar el error de una librería; **no se verificó que cite el contenido**, y habría que comprobarlo con un fallo provocado.

- **e. El historial de un tema queda escondido.** Si la fila purgada es la versión vigente de un `topic_key` (el caso típico: era la que tenía el secreto), las versiones anteriores quedan `superseded` por un id que ya no existe. No salen en búsqueda ni en contexto, y `restore` no aplica porque no están borradas. `purge` promete sacar «sólo ese texto» y en realidad esconde todo el historial del tema. Solo las filas con `supersede_cause = 'revision'` deberían volver; las reemplazadas por un veredicto (`verdict`, V17) se quedan escondidas a propósito.
- Los comentarios de `maintenance.rs:162-163` y `:289-291` ya nombran a `purge` como el único borrado duro (`46045aa`); si este cambio cambia lo que `purge` borra, hay que revisarlos.

### 3. Texto libre que no pasa por `redact`

Barrido de `crates/clawg-core/migrations/` y de los escritores:

| Columna / archivo | Quién escribe | ¿Enmascarado? | ¿`secrets` / `purge` llegan? |
|---|---|---|---|
| `memory.content` | worker, tras `redact` (`worker.rs:635`) | Sí | Sí (`secrets.rs:41`; `purge` lo borra) |
| `ingest_outbox.content` | `accept`, tras `redact` (`runtime.rs:222`) | Sí | `purge` por subcadena (`:114`); `scan` no lo mira (ya enmascarado) |
| `ingest_outbox.event` | el resto del evento en JSON, sin `content` (`V14:6-7`); lleva `topic_key`, `derived_from`, `session_id` | No aplica, no es texto libre largo | No |
| `ingest_outbox.last_error` | worker, texto de un error (`worker.rs:333`) | No | No; sale en `/health` por `operation_id`. **A verificar** si cita contenido |
| `memory_conflict.reason` | `judge_pair` (`dispute.rs:327`, `:383`) | **No** | **No.** `purge` borra sólo pares de la fila purgada (`secrets.rs:111`); un secreto en el `reason` de otro par queda y `scan` no lo ve |
| `memory_piece.context` | worker, cabecera de un save, ya enmascarado | Sí (viene de `content`) | **No** `purge` (backlog) |
| `workspace.md`, sección AUTO | `regenerate_auto_section`, copia de `memory.content` | Hereda: una fila vieja sin enmascarar vuelve a copiarse | **No** |
| `workspace.md`, sección USER | el usuario a mano | No (a propósito: no se reescribe) | No; sólo nombrar |
| `project_doc_chunk` | índice de docs, tras `redact` (`project_docs/mod.rs:382`) | Sí | Derivado del disco |
| `memory.topic_key`, `source_session_id`, `created_by_ref`, `agent_id` | validados por forma (`topic.rs`, `sessions.rs:17-26`) o dichos por el cliente | Forma restringida, no texto libre | No; riesgo bajo |
| `project.root_path`, `project_rekey.legacy_id` | el daemon, ruta del proyecto | No aplica (una ruta) | No |
| `episodic_trajectory`, `procedural_skill`, `root_cause` | sin escritor activo (`README.md`, «Sobre el estado actual») | — | — |

Resultado: **una** columna con texto libre del usuario o el agente que nunca se enmascara (`memory_conflict.reason`), una que puede citar contenido y falta verificar (`last_error`) y dos copias derivadas que `purge` ignora (`memory_piece.context`, `workspace.md`).

## Lo que ya existe y se reutiliza

- `redact` pura e idempotente y `is_literal_secret` (`redact.rs:162-174`): una regla nueva se prueba con una tabla de casos, sin base. `accepts` (`:138`) ya filtra después del match.
- `scan` ya aplica las reglas vigentes a todo lo guardado, y `specs/secrets` R4 ya dice que una regla nueva alcanza filas viejas.
- `purge` ya compacta el FTS, hace `VACUUM` y checkpoint (`secrets.rs:123-131`) y nombra backups (`:142`).
- `memory_piece` ya distingue un pedazo de un save de uno solo (`pieces.rs:1-7`).
- El historial de `redaction-precision` (tabla de casos en los dos sentidos, falsos positivos cerrados con `accepts`) es el método para ampliar sin destruir texto.

## Enfoques posibles

### Detector

**A. Patrones por clave con un tipo nuevo (recomendado).** Un patrón `Credential` anclado en claves que terminan en `secret`, `token`, `api[_-]?key`, `access[_-]?key`, `client[_-]?secret`, `accountkey` y `x-api-key`, con `is_literal_secret` sobre el valor y su propio nombre en `[REDACTED:credential]`. Más los prefijos fijos que falten (`hf_`, `xapp-`, `whsec_`, webhooks de Slack) y `PRIVATE KEY BLOCK`.
- **A favor:** misma filosofía que `Password` (clave delante, sin entropía); el tipo nuevo no cambia el significado de `password` en `redacted` ni en los tests existentes.
- **En contra:** `is_literal_secret` exige 6 caracteres y un dígito o símbolo, así que `token: abc123def` en prosa se enmascara y `token = next_token()` no. Es el costo de la regla; se mide (ver «Cómo se mide»).

**B. Sólo prefijos fijos.** Cero falsos positivos nuevos, pero deja fuera `AWS_SECRET_ACCESS_KEY`, `client_secret`, `AccountKey` y `X-Api-Key`, que no tienen prefijo. Sirve de piso: A lo incluye.

**C. Reusar `Password` ampliado.** Menos código, pero cambia lo que significa `password` en la respuesta del save y en `clawg secrets`. Descartado por eso.

### `purge`

**D. Nombrar, no reescribir (recomendado como mínimo).** Después de purgar, listar los `workspace.md` del agente y del proyecto de la fila que contienen el texto (sección AUTO y USER por separado) y pedir que se regeneren o editen. Es lectura de archivos, sin reescribir nada.
**E. Reescribir sólo el bloque AUTO.** `purge` quita las líneas que copian el texto entre `<!-- AUTO-START -->` y `<!-- AUTO-END -->`. Cierra el hueco, pero escribe un archivo desde una ruta sin `WorkspaceManager`; el USER no se toca. Conviene sólo si D resulta incómodo en uso real.
**F. Regenerar AUTO desde la base purgada.** Lo más coherente, pero necesita arrancar un `MemoryStore` y el runtime en `purge`, que hoy corre con el daemon parado y sólo `rusqlite`. Costo de arquitectura alto.

**G. Outbox: filtrar por agente y por pedazo (recomendado).** `DELETE … WHERE agent_id = ?2 AND` y, si la fila no tiene vínculo en `memory_piece`, `content = ?1` en lugar de `instr`; con vínculo, `instr`. Sin esquema nuevo. Cierra la pérdida entre agentes y, casi siempre, la de un mismo agente con textos cortos. Queda un caso: un pedazo de un save largo y un texto corto de ese mismo agente.
**H. Listar antes de borrar.** Cualquiera de las filas borradas se imprime por `operation_id`, para que el usuario vea qué se fue.
**I. Guardar el `operation_id` en `memory` (migración V18).** Es la solución exacta, pero necesita migración, vuelta atrás y un test que siembre una base en V17. Las filas viejas no tendrían el vínculo (se mantendría la subcadena para ellas), y el dedup refuerza una fila con otro save (`store_insert.rs`, refuerzo por texto idéntico), así que una fila puede venir de varios saves. Se reabre sólo si G y H no alcanzan.

**J. Cabecera de un save largo.** Que `purge` borre el vínculo de la fila y, si es la posición 1, vacíe `context` en los demás vínculos del save. Absorbido: es del mismo hueco («lo que `purge` promete») y de bajo costo.
**O. Revivir dentro de la transacción de `purge` (recomendado).** Marcar la fila como borrada primero, aplicar la misma revival que un `delete` (`revive_after_delete_of_piece`) y recién después borrarla de verdad; la salida de `purge` lista lo que volvió. Reusa la regla ya probada (sólo `revision`; `verdict` queda escondido) y deja un solo camino para «salir de la versión vigente». Toca las mismas líneas (`secrets.rs:103-119`) que J: se escriben juntas. Con J, `read_save` muestra un pedazo purgado como ausente; en cambio un veredicto `supersedes` sobre un único pedazo (ítem del backlog) deja el pedazo presente aunque `superseded`: son dos políticas distintas que hay que nombrar.
**P. Pasada de mantenimiento para `superseded_by` colgantes.** Para los que ya quedaron así en bases existentes: revivir (si `supersede_cause` es `revision`) o reapuntar. Cambia filas del usuario sin que nadie lo pida, así que es decisión y no se asume; O sola no arregla lo ya purgado.

**K. Fila `failed` sin salida.** Se deja fuera: es un camino de reintento/descarte, no de cobertura de secretos. Se anota para 31.

### Texto libre

**L. Enmascarar `reason` al escribirlo (recomendado)**, con `redact` en `judge_pair`, igual que `content`. **M. Además, que `scan` mire `memory_conflict.reason`** para encontrar los viejos, con una salida de otra forma (`pair <new> <old> <tipos>`). **N. Un comando para limpiar un `reason`** ya guardado: reescribe una columna del usuario, así que hace falta decisión y no se asume.

## Descartado

| Opción | Razón técnica |
|---|---|
| Entropía sin contexto | Enmascara hashes, UUID y sha256 que hoy se dejan a propósito. Pérdida segura en una base de código. |
| Reescribir filas viejas al subir la regla | Destruye texto sin que nadie lo pida. `scan` las lista y `purge` es a pedido. |
| Borrar de `workspace.md` la sección USER | Es del usuario y nunca se reescribe. |
| Snapshot antes de `purge` | Sería una copia más del texto (`specs/secrets` R5). |

## Interacciones

- **history-import (44) y memory-portability (43).** 44 trae miles de textos de engram por `redact`; cada formato sin cubrir queda guardado en claro y cada falso positivo se multiplica, sin vuelta atrás. El export de 43 llevaría además `memory_conflict.reason`. Por eso este cambio va antes de ambos.
- **redaction-precision (42, archivado).** Dejó la regla del valor literal y la marca privada; este cambio sólo suma formatos y claves, sin tocar esas reglas.
- **doctor-checks (31).** Puede señalar las filas `failed` y los workspaces con texto purgado; este cambio no las necesita. Su chequeo de `superseded_by` hacia un id inexistente es el que encontraría lo que `purge` dejó antes de O; O es el arreglo de origen y 31 sólo lo detecta.
- **topic-revisions (27) y revision-aware-delete (49), archivados.** Definen la cadena de `superseded`, `supersede_cause` y la revival que O reutiliza (`revival.rs`, `admin.rs:355`).
- **outbox-lifecycle (53).** Toca el outbox que `purge` también borra; coordinar G con lo que 53 decida sobre filas `failed`. Comparten la pregunta «qué hay en el store que no debería».
- **project-clone-identity (30).** Hace re-key de ids; el archivo legado de `workspace.md` es otro lugar que `purge` debe nombrar.
- **Ya arreglado (`92cee20`):** un `project_id` ya no coloca un `workspace.md` fuera de la carpeta de su agente (`project_segment`, `workspace.rs:639`). No hace falta para D.
- **Índice de docs.** Se re-lee por sha256 o modelo, no por cambio de regla (como en 42): un pasaje viejo queda con la regla vieja. Es derivado y no destruye nada del usuario; se acepta.

## Alcance candidato

1. Patrón `Credential` y los prefijos que faltan, con tabla de casos en los dos sentidos, y `specs/secrets` R1.
2. `purge` filtra el outbox por agente y, para filas sin vínculo, por igualdad; lista lo borrado.
3. `purge` nombra los `workspace.md` que contienen el texto, sin reescribirlos; y limpia la cabecera del pedazo 1 en `memory_piece.context`.
3b. `purge` revive, en su transacción, las versiones anteriores del tema (O) y las lista.
4. `redact` en `judge_pair` (el `reason`).
5. `clawg secrets` mira `memory_conflict.reason`.
6. Docs: `specs/secrets`, `.claude/CLAUDE.md`, y los dos ítems del backlog que se absorben (cabecera; el de la fila `failed` queda fuera).

### No-alcance

- Entropía, reescritura de filas viejas y de workspaces, y la fila `failed` del outbox.
- Cambiar el esquema salvo que se elija I.
- Los backups: `purge` los nombra y no los toca.

## Decisiones abiertas

Son del usuario.

- [ ] **Claves del patrón `Credential`.** ¿Cuáles entran, y se mantiene el piso de 6 caracteres de `is_literal_secret`?
- [ ] **Tipo.** ¿`credential` aparte (recomendado) o ampliar `password`?
- [ ] **Workspace.** ¿D (nombrar), E (reescribir AUTO) o F (regenerar)?
- [ ] **Outbox.** ¿G con H, o I con su migración?
- [ ] **`reason` viejos.** ¿Sólo `scan` los lista (M), o hay un comando de limpieza (N)?
- [ ] **`last_error`.** ¿Se verifica y, si cita contenido, se enmascara?
- [ ] **Historial ya escondido.** O arregla lo que se purgue desde ahora. ¿Se agrega una pasada de mantenimiento (P) para los `superseded_by` que ya cuelgan, revive o reapunta, o los lista `doctor` (31) y se deja al usuario?
- [ ] **Restauración de filas viejas.** ¿Se acepta que una regla nueva bloquee `undelete` de filas que antes volvían?

## Cómo se mide

Tests RED primero, todos puros salvo los de `purge`:

1. Cada formato de la lista, sintético y armado en tiempo de ejecución como en `redact.rs`, se enmascara. Hoy ninguno.
2. Prosa técnica y código con las mismas palabras (`token = next_token()`, `secret: string`, `api_key: Option<String>`) quedan iguales.
3. `masking_twice_finds_nothing_new` sigue valiendo con el tipo nuevo.
4. `purge` de una fila con otro agente con el mismo texto en su outbox deja la fila del otro. Hoy la borra.
5. `purge` de un pedazo 1 deja `memory_piece.context` sin su cabecera; con un `workspace.md` que contiene el texto, la salida lo nombra.
6. Un `reason` con un secreto queda enmascarado.
7. Dos versiones bajo un mismo `topic_key`, `purge` de la vigente: la anterior vuelve a estar viva y la salida la lista. Hoy queda `superseded` con un `superseded_by` colgante. Una versión reemplazada por veredicto (`verdict`) sigue escondida.

**Falsos positivos sin tocar un perfil real.** Decisivo: un corpus de prosa técnica del propio repositorio (`docs/`, `openspec/`, `crates/**/*.md`) y fixtures de configuración, pasado por `find_secrets` antes y después de la regla; el resultado esperado es cero cambios. Complemento: restaurar un `clawg backup` en un `CLAWG_TEST_HOME` temporal y correr `clawg secrets` ahí. Correrlo contra un perfil real requiere autorización explícita y no es necesario.

## Riesgos

- **Cambiar falsos negativos por falsos positivos.** Los patrones por clave enmascaran texto del usuario sin vuelta atrás. La tabla de casos y el corpus del repo son la red.
- **`purge` que sigue sin garantizar «todo».** Los backups y la sección USER quedan fuera a propósito; la salida debe decirlo, no sugerir lo contrario.
- **G no cierra el caso de un pedazo de un save largo con texto corto del mismo agente.** Se acepta o se pasa a I.
- **Restaurar.** Ver la última decisión abierta.
- **Revivir en `purge` cambia lo que ve el agente.** Una versión vieja vuelve a ser la vigente; si también tiene un secreto, `scan` la lista y se purga de nuevo, que es el comportamiento correcto.
