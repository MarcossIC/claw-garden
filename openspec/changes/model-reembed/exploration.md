# model-reembed — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S (fijar el contrato) a L (cambiar la dimensión) | — (los snapshots de storage-safety-net ya existen) | exploración (borrador) |

**Tesis:** hoy cambiar el modelo, la dimensión o los prefijos de embedding deja la memoria **apagada** y el binario no trae ninguna forma de volver a encenderla. Los datos no se pierden: quedan intactos, pero inalcanzables. El código para re-embeber existe y está probado, pero ningún comando lo llama. **No hay ningún cambio de modelo planeado.** Por eso la pregunta real es cuánto camino construir antes de necesitarlo.

Esta es la parte D de [data-preservation](../archive/2026-10-12-data-preservation/exploration.md) (`:95`), que nunca tuvo cambio propio.

## Qué del texto de 2026-10-12 ya no es cierto

| Afirmación de entonces | Hoy |
|---|---|
| «Los saves siguen respondiendo `queued`, quedan retenidos y se descartan al parar» (riesgo 2) | **Falso.** Con el embedder `failed` el save responde `503 embedder_failed` sin encolar (`crates/clawg-cli/src/daemon/api.rs:702-707`). Un contrato distinto es un `failed` no reintentable (`crates/clawg-core/src/memory/bootstrap.rs:48-52`). El MCP dice `not saved` (`crates/clawg-cli/src/mcp/server.rs:391-402`). Spec: `specs/ingest`, «Embedder fallido». |
| «No hay snapshot, ni backup, ni restore» (riesgo 3) | **Falso.** Existen `clawg backup`, `clawg restore`, el snapshot previo a migrar y la retención (`specs/storage`). |
| El mensaje del contrato promete «search will work in a minute» (riesgo 6) | **Corregido.** `embedder_failed` tiene su propio texto (`server.rs:391-402`). Pero sigue sin decir **cómo** se arregla: no hay ningún comando que nombrar. |
| «`reembed_all` (`maintenance.rs:442`) no tiene llamador» | **Ya no es cierto.** Está en `crates/clawg-core/src/memory/maintenance.rs:467` y lo llama `verify_or_reembed` en el arranque (`crates/clawg-core/src/memory/bootstrap.rs:296`), pero sólo para el caso de un cambio de pooling (ver «Qué repara hoy el arranque»). Para cualquier otro cambio de contrato, incluido un prefijo, sigue sin llamador. Sus tests están en `:1577-1764`. |
| «V2 fija `float[384]`» | **Sigue siendo cierto** (`crates/clawg-core/migrations/V2__vectors.sql:7,10,13`). |
| Prefijos e5 en `embedder.rs:109` | **Sigue siendo cierto.** `E5_PASSAGE_PREFIX = "query: "` (`crates/clawg-core/src/memory/embedder.rs:109`), igual que el de query (`:89`). |
| «`reembed_all` era un comando de Tauri y se borró en `5678ded`» | No lo reverifiqué. No cambia nada de lo que sigue. |

Un punto que ese texto no cubría: la memoria apagada no es «vacía» para todo. `POST /v1/search` responde 503 (`api.rs:756-761`), pero `POST /v1/context` cae a la recuperación léxica sin embedder. Eso lo dice `specs/ingest` (línea 128); no lo reverifiqué en el código.

## Problema (verificado)

### Dónde vive el contrato y qué ve el usuario

- La tabla `embedding_contract` guarda una fila con `model_id`, `dim`, `query_prefix` y `passage_prefix` (`crates/clawg-core/migrations/V5__embedding_contract.sql`). También `metric` y `normalized`, que no se comparan.
- `ensure_embedding_contract` la escribe si no existe y la compara byte a byte si existe (`crates/clawg-core/src/memory/db.rs:309-358`). Una diferencia da `Mismatch`.
- La identidad la informa el embedder. Modelo y dimensión salen de literales en `FastEmbedLocal::from_bytes` (`embedder.rs:268-269`). Los prefijos salen de dos constantes (`embedder.rs:214-220`).
- El bootstrap compara **después** de bajar y cargar el modelo (`bootstrap.rs:320-346`). Ante un mismatch deja la celda del embedder vacía y devuelve `BootstrapError::Mismatch`. No reintenta (`specs/ingest`, líneas 184-185).
- **Lo que ve el usuario:**
  - `/health` y `clawg status`: `embedder: "failed"`, `embedder_retryable: false` y el texto con los campos que difieren (`api.rs:610`; `differing_fields`, `db.rs:87`).
  - MCP: `search_memory` y `save_memory` fallan con «could not load its embedding model (…); search and saving are unavailable until the cause is fixed».
  - **Nada dice qué hacer.** No hay `clawg reembed`, ni una vuelta atrás documentada. La salida real es restaurar un snapshot y volver al binario anterior.

### Qué repara hoy el arranque

- Al arrancar, `verify_or_reembed` compara el contrato y repara automáticamente **un solo** mismatch: el cambio de pooling, es decir el mismo modelo con vectores hechos por CLS y ahora por promedio, con misma dimensión y mismos prefijos (`is_pooling_upgrade`, `bootstrap.rs:255-261`; `verify_or_reembed`, `bootstrap.rs:269-307`). Antes toma un snapshot `pre-reembed` (`bootstrap.rs:287-294`), re-embebe con `reembed_all` y sólo entonces deja el contrato en `Match`; si no termina, devuelve el mismatch y el próximo arranque reintenta (`bootstrap.rs:296-306`).
- **Cualquier otro mismatch** (otro modelo, otra dimensión, **un cambio de prefijo**) se devuelve tal cual (`bootstrap.rs:282`) y apaga la memoria como describe el resto de esta exploración.
- Consecuencia: la parte «re-embeber en el arranque» ya existe para un caso. Si english-recall (21) confirma que un prefijo `passage: ` para los textos guardados rinde mejor con vectores promediados (hoy `E5_PASSAGE_PREFIX` es `"query: "`, `embedder.rs:110`), hay dos caminos: generalizar esa reparación de arranque a «mismo modelo y misma dimensión, distinto prefijo» (con el mismo snapshot previo), o la opción B de este cambio (`clawg reembed`). Eso es lo que subiría la urgencia de este cambio (ver «Urgencia»).

### `reembed_all` existe, está probado y tiene un solo dueño

- Dos modos: `Normalize` (renormaliza los vectores guardados, sin embedder) y `ReInfer(ReInferPlan)` (re-infiere con el embedder). Sólo `ReInfer` escribe el contrato (`maintenance.rs:467-497`).
- **Qué filas recorre:** las tres tablas, y sólo las que tienen fila en la vec0 (`JOIN … ON v.rowid = p.id`) **y** `deleted_at IS NULL` (`select_reembed_batch`, `maintenance.rs:604`).
  - Las memorias borradas no tienen vector (el delete lo quita, `admin.rs:281-283`), así que quedan fuera.
  - Una fila viva sin vector también queda fuera.
  - Las reemplazadas por un conflicto (`superseded_by`) no tienen `deleted_at`, así que entran.
- **Lotes:** 200 por lote, con paginación por clave (`id > last_id`; `REEMBED_BATCH`, `maintenance.rs:322`).
- **Transacciones:** la inferencia corre fuera de la conexión. El `UPDATE` de cada lote es una transacción propia (`write_reembed_batch`, `maintenance.rs:670`). No hay una transacción global.
- **Si se corta a la mitad:** los lotes ya confirmados quedan con el vector nuevo y el resto con el viejo. El contrato **no** se toca hasta que las tres tablas terminan sin fallas (`maintenance.rs:467-497`). Volver a correr converge: es idempotente en resultado, no en costo (test en `:1753`).
  - Con el binario nuevo: mismatch y memoria apagada. Es la parada segura que el código busca.
  - **Con el binario viejo:** el contrato coincide, y la base sirve una mezcla de dos espacios de embedding **sin ningún aviso**. Es el riesgo real de un re-embed sin snapshot.
- **Costo:** infiere fila por fila con `embed_with_role`, siempre con rol `Passage` (`compute_new_embedding`). No lo medí; la inferencia ONNX en CPU es el costo dominante.
- **Fallos por fila:** la fila que no se puede embeber conserva su vector viejo, se cuenta como `failed` y el contrato no se actualiza (`maintenance.rs:507-570`).
- **Consecuencia en el arranque: todo o nada.** `reembed_all` escribe el contrato sólo si `failed == 0 && !aborted` en las tres tablas (`maintenance.rs:507-535`, la condición en `:516`); la fila que falla conserva su vector viejo (`:582`). `verify_or_reembed` toma el snapshot `pre-reembed` (`bootstrap.rs:440-455`) y, si el contrato no se escribió, devuelve `Mismatch` (`:467`); `bootstrap.rs:547-558` lo convierte en `BootstrapError::Mismatch`, que `bootstrap.rs:73` trata como `GiveUp`: el embedder queda `failed` sin reintento en ese proceso, los saves responden `embedder_failed` y la búsqueda está apagada. El próximo arranque repite el snapshot y el re-embed completo. Cada intento suma un `pre-reembed` al presupuesto automático de 5 y puede sacar al `pre-migration` (ver «Lo que se puede reusar para el snapshot previo»). Una sola fila rebelde apaga la memoria hasta un arranque donde todas pasen.
- **Qué hace fallar a una fila:** `EmbedError::{NotReady, WorkerGone, InferenceFailed, LoadFailed}` o un error de `to_unit` (dimensión, valor no finito, norma cero) (`maintenance.rs:617`, `embedder.rs:187-194, 216-235`, `vector.rs:41-57`). Un texto vacío, o vacío por el enmascarado, **no** falla: el tokenizador agrega los tokens especiales. El disparador realista es una falla transitoria o un texto patológico. Importa para las bases que aún tienen vectores por CLS (de antes de project-indexing, 12) y para cualquier cambio de modelo futuro por `clawg reembed`. Los tests cercanos a `maintenance.rs:1881` fijan el comportamiento de hoy (una fila fallida bloquea el contrato).
- **Dimensión:** `UPDATE memory_vectors SET embedding = ?` con un vector de otra longitud falla contra `float[384]`. `reembed_all` sirve para cambiar modelo o prefijos **a la misma dimensión**, no para cambiar la dimensión.

### Las tres tablas vec0 tienen la dimensión escrita

- `memory_vectors`, `episodic_vectors` y `procedural_vectors` son `vec0(embedding float[384])` (`V2__vectors.sql:7,10,13`). La dimensión no se puede alterar: hay que recrear la tabla.
- V2 está fijada por checksum (`crates/clawg-core/src/memory/migrations.rs:43-55`, `published_migration_checksums_are_pinned`). Se cambia con una migración **nueva**, nunca editando V2.
- Cambiar la dimensión necesita, como mínimo:
  1. una tabla nueva por cada una de las tres, con la dimensión nueva;
  2. llenarlas re-infiriendo todo el texto (un vector de 384 no se convierte a otra dimensión);
  3. reemplazar las viejas y escribir el contrato, de forma atómica.
- **Sin verificar:** si el DDL de vec0 (crear, renombrar, borrar) funciona dentro de una transacción. `data-preservation` ya lo dejaba pendiente (parte F). Hay que probarlo antes de diseñar sobre eso.
- El texto de origen sí está: `memory.content`, `episodic_trajectory.task` y `trigger + skill_name` para los skills (`skill_embedding_text`, `maintenance.rs`; ver `specs/retrieval`, línea 194). Ningún dato existe sólo como vector. Por eso re-embeber **no pierde** información.

### Hay más atado al modelo que el contrato

- La descarga está escrita para un solo modelo. `HF_BASE_URL` apunta a `intfloat/multilingual-e5-small` y hay cinco SHA-256 fijos (`crates/clawg-core/src/memory/model_download.rs:20-21,56-64,85`). Un modelo nuevo es otro manifiesto, otra URL, otros hashes y otro embedder.
- El presupuesto de tokens del query depende de la ventana (`EMBED_MAX_TOKENS = 512`, `embedder.rs`) y del tokenizador del modelo.
- Los resultados de `rag-eval` guardan `model_id` y los prefijos (`crates/clawg-eval/src/bin/rag-eval/report.rs:266,281`). Un modelo nuevo invalida la comparación `--diff` contra la baseline actual.

### ¿Hay un test que fije el contrato de producción?

- **Prefijos: sí.** `e5_prefix_constants_are_symmetric_with_the_load_bearing_trailing_space` (`embedder.rs:730-738`). Cambiarlos rompe un test.
- **Modelo y dimensión de producción: no.** `FastEmbedLocal` sólo se construye con `from_bytes`, que necesita el modelo real. Los literales `384` y `"multilingual-e5-small"` (`embedder.rs:268-269`) no los verifica ningún test automático. Los tests de `db.rs:1816-1828` y `bootstrap.rs:889-1007` usan embedders simulados con valores escritos a mano, así que pasarían igual si el de producción cambiara. El único `384` que se afirma es el del `FakeEmbedder` (`embedder.rs:493-496`).
- **Nada compara la dimensión del embedder con la de V2.** Si alguien subiera `dimensions` a 768 sin migración, las bases existentes darían mismatch y, en una base nueva, los inserts fallarían contra `float[384]`.
- El comportamiento del mismatch sí está probado (`db.rs:1923,1921,1941`; `bootstrap.rs:889,960`).

### Opciones ante una fila que no se puede embeber

- **F1. Apartar la fila y escribir el contrato.** Quitar su vector del espacio nuevo (o dejarlo fuera de él), anotarla en una tabla o columna de «sin vector», mantenerla legible y listable, escribir el contrato e informar cuántas quedaron fuera. La memoria vuelve a encenderse; esas filas no aparecen en la búsqueda semántica (sí en la léxica, y por `list`/`read`). Hay que decidir cómo se reintentan y qué pasa si se guardan con un vector del espacio viejo: mezclar espacios es el riesgo que el contrato existe para evitar.
- **F2. Reintentos acotados dentro de la corrida.** Reintentar las filas fallidas un número fijo de veces antes de rendirse. Cubre la falla transitoria, no el texto patológico; sigue siendo todo o nada.
- **F3. Dejarlo como está.** Es la parada segura de hoy; el costo es la memoria apagada y un snapshot por arranque.

Un store con vectores CLS que un día no pueda terminar el re-embed tiene el mismo problema que un cambio de modelo.

## Lo que ya protege

- **Fail-closed.** Un contrato distinto no borra nada: apaga la memoria (`bootstrap.rs:338-346`). Los datos siguen en `memory.db`.
- **El contrato se escribe al final.** Un re-embed incompleto no puede declararse completo (`maintenance.rs:467-497`).
- **Snapshots.** `snapshot_into` usa `VACUUM INTO`, incluye el WAL y los vectores y no deja archivos a medias (`crates/clawg-core/src/memory/backup.rs:275`; tests `a_snapshot_keeps_vectors_and_full_text_searchable` y `a_failed_snapshot_leaves_no_file_behind`).
- **Restore con el daemon parado**, con snapshot de lo que reemplaza (`backup.rs:332`; `crates/clawg-cli/src/restore.rs`). Es **la** vuelta atrás de un re-embed: devuelve vectores y contrato viejos de una vez.
- **Lock exclusivo** `memory.lock`, con el titular `Maintenance` para `backup`, `restore` y `purge` (`crates/clawg-core/src/memory/store_lock.rs:58`).
- **Esquema.** Una base de otra versión se rechaza sin tocarla (`specs/storage`, «Verificación del esquema antes de migrar»).
- **Los saves ya no se pierden en silencio durante un mismatch** (ver la tabla de arriba).

### Lo que se puede reusar para el snapshot previo

- `snapshot_into(conn, dir, reason)` y `Reason` (`backup.rs:119`). `PreReembed` **ya existe** (`backup.rs:119-139`): el nombre del archivo lo lleva, `Reason::ALL` y `is_automatic` lo cubren, y el arranque ya lo usa para la reparación del pooling (`bootstrap.rs:290`). Un `clawg reembed` lo reutilizaría; lo que faltaría es, a lo sumo, ampliar los tests de nombres (`snapshot_names_parse_back_to_their_reason`) si cambia algo.
- La retención automática conserva las últimas 5 (`AUTO_KEEP`, `to_prune`, `backup.rs:227-243`), y `pre-reembed` comparte ese presupuesto con `pre-migration` y `pre-restore` (`is_automatic` es «todo menos `Manual`», `backup.rs:143-145`): un re-embed con varios reintentos puede sacar de la ventana un `pre-migration` sano, además del propio `pre-reembed`. **Cuidado:** varios reintentos de un re-embed podrían sacar de la ventana al snapshot sano. Va a decisiones abiertas.
- `StoreLock::acquire(…, StoreHolder::Maintenance)` y `open_existing` (`backup.rs:527`). El patrón de `purge` (`crates/clawg-core/src/memory/secrets.rs:81`) es el modelo: lock, abrir sin migrar, verificar `schema_plan` y operar.

## Enfoques posibles

| | A. Fijar el contrato y documentar el camino | B. `clawg reembed` (misma dimensión) | C. B + cambio de dimensión |
|---|---|---|---|
| Qué es | Tests que fijan modelo, dimensión y prefijos de producción, mensaje accionable y una guía del camino manual | Comando offline con snapshot previo sobre `reembed_all`, para cambio de modelo o de prefijos a la **misma** dimensión | B más una migración nueva que recrea las tres vec0 con otra dimensión |
| Tamaño | S | M | L |
| Resuelve | Cambiar el contrato deja de ser accidental | El mismatch tiene salida dentro del binario | Cualquier modelo |
| No resuelve | Ningún camino en el binario | Dimensión distinta; descargar un modelo nuevo | — |
| Riesgo | Casi ninguno | Se escribe sin un modelo real contra el cual probarlo (sólo embedders simulados) | Migración sobre datos de usuario, DDL de vec0 sin verificar, manifiesto de descarga por modelo |
| Costo de esperar | Ninguno | Bajo mientras no haya cambio de modelo | Alto escribirlo sin saber qué modelo |

### Dónde corre `clawg reembed`

- **Offline, con el daemon parado** (como `restore` y `purge`, `crates/clawg-cli/src/main.rs:141,157`):
  - un solo titular del store, sin saves concurrentes que inserten vectores del espacio nuevo a mitad del recorrido;
  - no depende del daemon, que ante un mismatch **no tiene embedder** (la celda queda vacía, `bootstrap.rs:338`);
  - el comando carga el modelo por su cuenta (`ensure` + `from_bytes`, como el bootstrap y `eval_api.rs:404`), lo que puede implicar descargar.
- **Por el daemon** (como `backup`, `main.rs:124`):
  - sirve el store sin parar;
  - pero hay que pausar la ingesta y rearmar el embedder dentro del daemon. Es mucha superficie nueva para algo que se hace una vez.

**Tradeoff:** offline es más simple y más seguro, y cuesta que el usuario pare el daemon. El comando toma el lock primero y falla con un mensaje claro si no lo consigue, como `restore`.

## Alcance candidato

El alcance **mínimo recomendable** (A):

1. **Constantes de identidad.** `MODEL_ID` y `DIMENSIONS` como constantes que usan `from_bytes` y los tests, para fijarlas sin cargar el modelo.
2. **Test que fija el contrato de producción:** modelo, dimensión y los dos prefijos.
3. **Test de coherencia con V2:** la dimensión declarada es la de las tres vec0 (leída del esquema real).
4. **Test de preservación ante un mismatch:** una base sembrada y abierta con otra identidad conserva filas, vectores y contrato. Restaurar el snapshot previo las devuelve alcanzables.
5. **Mensaje de mismatch accionable:** `clawg status` y el MCP nombran el camino (`clawg restore` con el snapshot previo, o `clawg reembed` si existe).
6. **Una guía** del camino manual en `docs/`.

Si el usuario elige B, además:

6b. Decidir el tratamiento de la fila que no se puede embeber (F1, F2 o F3) tanto para el arranque como para `clawg reembed`, e informar cuántas quedaron fuera.
7. `clawg reembed [--profile P]`, offline, con snapshot `pre-reembed`, lock `Maintenance`, informe por tabla (intentadas, actualizadas, fallidas) y salida distinta de cero si algo falló.
8. Rechazo si el modelo cargado ya coincide con el contrato (nada que hacer) o si la dimensión difiere (dice que hace falta C).
9. Retención del snapshot `pre-reembed` fuera de la poda automática, o con un conteo aparte.

**No-alcance:**
- Elegir o integrar un modelo nuevo (manifiesto de descarga, clase de embedder, ventana de tokens).
- Cambio de dimensión (C), salvo que se decida explícitamente.
- Generalizar el re-embed automático de arranque más allá del cambio de pooling (hoy el único que repara, `bootstrap.rs:255-261`), salvo que english-recall (21) pida un cambio de prefijo y se decida por esa vía en lugar de B. Para el resto, no re-embeber automáticamente al arrancar. El propio `reembed_all` lo descarta por diseño (doc de `reembed_all`, `maintenance.rs`): una migración de CPU que nadie pidió es peor que una parada ruidosa.
- Reconstruir vectores de filas borradas. Su texto queda pero sin vector; eso es la parte E (borrado recuperable).
- Cambiar el ranking. Si algo toca modelo o prefijos, se valida con `rag-eval --diff` (ver abajo).

## Urgencia (evaluación honesta)

- **No hay ningún cambio de modelo planeado.** Busqué en `docs/` y `openspec/`: e5-small es el modelo actual y `project-indexing` menciona otros retrievers sólo como referencia. El texto de `data-preservation` lo trataba como un riesgo de «versión futura».
- **El riesgo ya no es de pérdida.** Con los snapshots y los 503 limpios, un cambio de modelo hoy es una **parada**, no una pérdida: los datos quedan, el usuario ve que falló y puede volver con `clawg restore` y el binario anterior.
- **Qué sí es barato y vale ahora:** A. Fija el contrato de producción, que hoy no está fijado (modelo y dimensión), y cuesta poco.
- **Qué conviene esperar:** B y C. Escribir `clawg reembed` sin un modelo real para probarlo es construir a ciegas. C además toca una migración sobre las tres vec0 sin saber la dimensión de destino.
- **Qué cambiaría el juicio:** que el usuario quiera probar un modelo distinto, o un prefijo `passage: `, sobre su base real. En particular, si english-recall (21) confirma que `passage: ` para los textos guardados es mejor con vectores promediados, hace falta una forma de rehacer todos los vectores con otro prefijo: generalizar la reparación de arranque (`bootstrap.rs:255-261`, hoy sólo para pooling) o la opción B. Eso sube la urgencia de este cambio de «esperar» a «necesario antes de aplicar 21». Ahí B pasa a ser el camino, porque `rag-eval` mide sobre una base aparte y no sobre la del usuario.

## Decisiones abiertas

Son del usuario. Esta exploración no las decide.

- [ ] **¿Cuánto se construye ahora?** A (fijar y documentar), A + B (comando para la misma dimensión) o A + B + C (también la dimensión). Ver la tabla de enfoques.
- [ ] **¿Dónde corre `clawg reembed`?** Offline con el daemon parado (como `restore` y `purge`) o a través del daemon (como `backup`).
- [ ] **¿Qué se hace con `ReembedMode::Normalize`?** Renormaliza vectores sin embedder y hoy no tiene ningún caso de uso. ¿Se expone, se borra o se deja sin llamador? Las reglas del repo piden no dejar abstracciones sin uso.
- [ ] **Retención del snapshot previo.** ¿`pre-reembed` entra en la poda automática de 5, o se conserva aparte hasta que el usuario lo borre? Con reintentos, la poda podría eliminar el único snapshot sano.
- [ ] **Texto del mismatch.** Hoy los saves responden 503 (verificado). ¿Alcanza, o el mensaje debe nombrar el comando de reparación?
- [ ] **Fila que no se puede embeber.** ¿F1 (apartarla y escribir el contrato), F2 (reintentos acotados) o F3 (todo o nada, como hoy)? Si F1: dónde se anota, si la búsqueda léxica la sigue alcanzando y cómo se reintenta. Hoy cada arranque fallido suma un snapshot `pre-reembed`.
- [ ] **Re-embed incompleto.** Hoy queda mezclado con el contrato viejo, y el binario viejo lo serviría sin avisar. ¿Se acepta con el restore como única vuelta atrás, o el comando deja una marca (por ejemplo una fila `reembed_in_progress`) que hace que cualquier binario se niegue a abrir el store hasta terminar?
- [ ] **Dimensión (si se elige C).** ¿Tablas nuevas y renombrar en una transacción (a verificar con vec0) o `DROP` y recrear dentro de una migración? Lo segundo deja la memoria sin vectores hasta terminar.
- [ ] **¿Una opción para aceptar el mismatch?** Para quien cambia sólo un prefijo y acepta vectores viejos. Hoy no existe y no la recomiendo: la base queda inconsistente.
- [ ] **Prueba manual.** ¿Se hace sobre `~/.clawg-dev`? Requiere autorización.

## Cómo se mide

Tests RED primero:

1. **Contrato de producción fijado:** modelo, dimensión y los dos prefijos tienen un test que falla si cambian. Hoy sólo los prefijos lo tienen.
2. **Coherencia con V2:** la dimensión del embedder es la de las tres vec0 (leída de `sqlite_master`).
3. **Preservación ante un mismatch:** una base sembrada con filas de las tres tablas y abierta con otra identidad queda con las mismas filas, los mismos vectores y el mismo contrato. El estado es `failed` no reintentable.
4. **Snapshot previo (si B):** tras `clawg reembed` existe un snapshot `pre-reembed` que, restaurado, devuelve vectores y contrato anteriores, y las filas siguen alcanzables por `search`.
5. **Re-embed completo (si B):** con un embedder simulado de la misma dimensión, las tres tablas quedan con el vector nuevo, el contrato cambia y el siguiente arranque queda `ready`.
6. **Re-embed cortado (si B):** un fallo a mitad deja el contrato viejo, el informe lo dice, el comando sale distinto de cero y volver a correrlo termina el trabajo. Hoy esto está probado a nivel función (`maintenance.rs:1700,1753`).
7. **Rechazos (si B):** con el daemon vivo no hace nada (como `restore_refuses_while_the_daemon_holds_the_store`); con el contrato igual no hace nada; con otra dimensión lo dice.
8. **Fila rebelde (si F1 o F2):** un store con una fila que no se puede embeber (embedder simulado que falla para un id) y las demás con vectores CLS: tras el arranque el contrato queda escrito, la fila sigue legible y listable, el informe cuenta una fila fuera, y no se acumulan snapshots `pre-reembed` en cada arranque. Con F3, el mismo caso mantiene el comportamiento de hoy (los tests de `maintenance.rs:1881`).
9. **Dimensión (si C):** una base sembrada en V10 con filas en las tres tablas abre con la migración nueva y todas las filas siguen alcanzables tras el re-embed. Es la regla de `config.yaml` para cambios de esquema.

`rag-eval --diff` contra la baseline: **sólo si cambia el modelo o un prefijo.** Para A y B con el mismo contrato, las métricas deben ser idénticas. Un modelo nuevo exige una baseline nueva (el `model_id` va en el resultado) y se mide con `--real`, que necesita el modelo.

**Prueba manual**, con autorización y sobre el perfil dev: guardar con el modelo actual, cambiar el contrato, comprobar el mensaje de mismatch, correr el re-embed, y verificar el conteo de filas y que una búsqueda devuelva lo guardado.
