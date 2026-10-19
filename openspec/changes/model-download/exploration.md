# model-download — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | — (conviene antes de 53 outbox-lifecycle y junto a 61 embedder-recovery; ver «Interacciones») | exploración (borrador) |

**Tesis:** conseguir el modelo nunca cuelga al daemon, y cuando falla siempre dice por qué. Hoy la descarga no tiene ningún plazo (ni de conexión ni de lectura), así que un servidor o un proxy que contesta y deja de enviar mantiene el bootstrap en `loading` para siempre; y la recarga del modelo ocioso, si la caché ya no pasa su SHA-256, ejecuta esa misma descarga con el cerrojo del embedder tomado, de modo que toda llamada que necesita un vector espera detrás de ella mientras `/health` dice `ready`. Cuando la descarga sí falla, la causa (el código HTTP, el error de red) se descarta y el usuario lee sólo «retry budget exhausted … after 3 attempts». Además, el camino de verificación lee y hashea el ONNX de ~470 MB hasta tres veces y lo hace en un hilo del runtime asíncrono. Nada de lo de acá toca `memory.db`: el cambio vive en los archivos del modelo (`<data>/models/`), no en la base del usuario. No toca inferencia ni chunking.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| La descarga no tiene plazos | **Cierta.** `reqwest::get(&url).await` (`crates/clawg-core/src/memory/model_download.rs:671`) construye un `Client` por llamada (`reqwest-0.12.28/src/lib.rs:323-324`) con `connect_timeout`, `read_timeout` y `timeout` en `None` (`reqwest-0.12.28/src/async_impl/client.rs:299, 313-314`). Los features de reqwest en `Cargo.toml:19` no cambian eso. El bucle de lectura (`model_download.rs:693-720`) tampoco tiene tope. |
| `tcp_keepalive` ya protege de un par que se calla | **Falsa.** El valor por defecto es 15 s (`client.rs:303-305`), pero el keepalive TCP sólo detecta un par que desapareció; un servidor vivo que acepta, manda cabeceras y no envía más sigue contestando los sondeos. |
| `read_timeout` existe en la versión fijada | **Cierta.** `ClientBuilder::read_timeout` (`client.rs:1453`): «se aplica a cada lectura y se reinicia tras una lectura correcta»; un enlace lento pero vivo pasa, uno parado no. `connect_timeout` en `client.rs:1466`. |
| El bootstrap queda en `loading` si la descarga se calla | **Cierta.** El reintento sólo se dispara tras un `Err` (`bootstrap.rs:652`, bucle en `:626-660`); una descarga que no vuelve ni éxito ni error nunca lo alcanza. `status` pasó a `Loading` en `:501` y no sale. El caso más probable es la primera ejecución detrás de un proxy corporativo o con un flujo HTTP/2 parado. |
| La recarga del modelo ocioso puede descargar | **Matiz:** sólo si la caché falla su SHA. Con la caché intacta `try_load_cached` devuelve `Some` (`model_download.rs:434`) y no hay red. Pero `lease` hace `(self.reload)().await` con `self.model.lock().await` tomado (`idle_embedder.rs:100, 105`) y la recarga llama a `ensure(base_dir, None)` (`bootstrap.rs:394`), que es `ensure_model` (`:278`) completo, descarga incluida. Basta un archivo borrado o corrupto en `models/` (limpieza de disco, un antivirus). |
| Mientras eso pasa, el resto espera | **Cierta.** `embed_with_role` y `embed_batch` hacen `lease` (`idle_embedder.rs:207, 212`), el barrido `unload_if_idle` toma el mismo cerrojo (`:136`) y `wake` lanza otro `lease` (`:242-244`). `/health` sigue en `ready` a propósito durante las recargas (`openspec/specs/storage/spec.md:378`). No medí si cada ruta HTTP que embebe tiene su propio tope, así que no afirmo cuáles quedan colgadas; un guardado con espera ve «sin confirmar» a los 10 s (`SAVE_CONFIRM_TIMEOUT`, `crates/clawg-cli/src/daemon/api.rs:98`). |
| Los errores de descarga llegan al usuario | **Falsa.** `Err(_)` en el envío descarta el error (`model_download.rs:671-677`), un estado que no es éxito se descarta (`:678-681`), un error de flujo sólo marca `had_http_err` (`:715-718`). `RetryExhausted { file, attempts }` (`:334-335`, construido en `:790-793`) no lleva causa. `DownloadError::Http` (`:322-327`) existe pero nada lo construye. El mensaje acaba en `embedder_error` de `clawg status` (`crates/clawg-cli/src/daemon/api.rs:899-918`) y en el 503 de un guardado; una corrida real contra un servidor que respondía 503 mostró sólo «retry budget exhausted for onnx/model.onnx after 3 attempts» (`openspec/changes/archive/2026-10-12-ingest-durability/verify.md:53`). |
| Trabajo bloqueante dentro de código `async` | **Cierta.** `ensure_model_with_oracle_spec_inner` (`model_download.rs:416`, async) llama a `try_load_cached` (`:434`, `std::fs::read` + `sha256_hex`, `:510-511`), `file_matches_sha` (`:465`, `:587`), `load_cached_or_err` (`:491`) y escribe con `File::create`/`write_all`/`fs::read` síncronos (`:660, 696, 757`). Corre en un worker de tokio en el arranque (`bootstrap.rs:521`) y en cada recarga (`:394`); sólo la carga de la sesión va en `spawn_blocking` (`:396, 529`). Además `runtime.rs:404-408` llama a `resolve_tokenizer_and_chunker(..., load_verified_tokenizer)` dentro del bloque `async` del worker de ingest: lee, hashea y parsea `tokenizer.json` (`model_download.rs:154-193`). Como el CLI usa el runtime multihilo (`crates/clawg-cli/src/main.rs:59`) es latencia añadida a otras tareas, no una parada total. |
| Se hashea de más | **Cierta.** `ALL_FILES` pone el ONNX primero (`model_download.rs:91-93`). Con un archivo pequeño faltante el ONNX se lee y hashea en `try_load_cached` (`:510`, antes de hallar el faltante), otra vez en `file_matches_sha` (`:465`) y otra en `load_cached_or_err` (`:491`): tres veces. Tras una descarga completa del ONNX, dos (`:757-767` y `:491`). En una recarga con caché intacta se leen y hashean los cinco archivos y los bytes de `tokenizer.json` se descartan sin parsear (`bootstrap.rs:223-227`, `:242`). Hashear los mismos bytes que luego recibe la sesión es deliberado (`bootstrap.rs:376-380`) y se queda. |
| Hay capas de costura de sobra | **Cierta.** `ensure_model` (`:348`) → `ensure_model_with_oracle` (`:360`) → `ensure_model_with_oracle_spec` (`:382`) → `_inner` (`:416`), cada una con un parámetro más sólo para tests; y tres lectores de hash casi iguales: `try_load_cached` (`:497`), `load_cached_or_err` (`:537`, el mismo bucle pero con `Err` tipado) y `file_matches_sha` (`:586`). |
| `CLAWG_HF_BASE_URL` es sólo para tests | **Falsa.** Se lee con `std::env::var` sin `cfg(feature = "testing")` y sin comprobar el esquema (`model_download.rs:197-199`), en builds de release también; el comentario del módulo (`:3-11`) y el de la función (`:195-196`) dicen que existe para tests, pero ningún test la usa (pasan la URL a `ensure_model_with_oracle*`; `rg` no la halla en ningún `#[cfg(test)]`). Sí la usaron como palanca manual notas archivadas (`ingest-durability/verify.md:53`, `install-smoke/exploration.md:27, 46, 70`, `release-and-self-update/exploration.md:128`). `release.yml:110` busca `CLAWG_TEST_`, que no la cubre. No está en `.claude/CLAUDE.md`. La integridad no corre riesgo (cinco SHA-256 fijados en `ALL_FILES`); queda disponibilidad y privacidad, y quien fija el entorno del daemon ya controla el proceso. |

## Problema (verificado contra el código)

1. **Una descarga parada no vuelve.** Sin `connect_timeout` ni `read_timeout`, un `get` o un `next()` del flujo (`model_download.rs:671, 693`) esperan indefinidamente. El reintento de tres intentos (`:650-652`) y el del bootstrap (`bootstrap.rs:626-660`) sólo corren tras un error. Efecto: `loading` eterno, 202 `embedder_loading` para cada guardado, sin ningún mensaje.
2. **La recarga descarga bajo el cerrojo.** La ruta `lease` → `reload` → `ensure` (`idle_embedder.rs:100-105`, `bootstrap.rs:394`) no distingue «volver a cargar archivos buenos» de «volver a bajarlos». Una caché dañada convierte una recarga de milisegundos en una descarga de cientos de MB, con tres intentos y esperas entre ellos, mientras toda llamada de embedding y el barrido de inactividad hacen cola. La spec dice «una recarga que falla MUST responder “no listo”» (`specs/storage/spec.md:367`); una recarga que no termina no responde nada.
3. **La causa se pierde.** Ver la tabla: el usuario no puede distinguir 503, 404, proxy, DNS o TLS.
4. **Latencia en los workers.** Leer y hashear 470 MB hasta tres veces, y leer, hashear y parsear 17 MB de tokenizer, sobre hilos de tokio.
5. **Código con capas de más** que oculta lo anterior y multiplica por tres cada cambio al formato del hash.

## Lo que ya existe y se reutiliza

- Los reintentos y la política de espera (`BackoffPolicy`, `ZeroBackoff` en tests) y los mocks de `mockito` (`model_download.rs:894-901, 1158-1159, 1239-1250`).
- La seam `EnsureModelFn` / `ReloadFn` (`bootstrap.rs:190`, `idle_embedder.rs:23-24`): se inyecta un `ensure` que no vuelve (`TestEmbedder::Stalled` ya hace `futures::future::pending()` para el bootstrap, `bootstrap.rs:342`).
- `futures::StreamExt` y `sha2::Digest` ya importados: el hash incremental no añade dependencias.
- `Fs2Oracle` y el chequeo de disco previo (`model_download.rs:211-214, 440-449`): no cambian.
- `classify` / `retry_delay` del bootstrap (`bootstrap.rs:71-93`): cualquier error de descarga ya se reintenta, así que un plazo vencido cae en el camino que existe.

## Enfoques posibles

### Plazos de red

**A (recomendado). Un `reqwest::Client` compartido con `connect_timeout` (~30 s) y `read_timeout` (~60 s).** Un `Client` construido una vez para la descarga (con `LazyLock` o pasado como parte de la configuración) en lugar de `reqwest::get`. No se usa `timeout` total: 470 MB en un enlace lento legítimo lo cortaría.
- **A favor:** directo; el plazo se reinicia en cada lectura, de modo que un enlace lento pero vivo pasa. Convierte el cuelgue en un error que el bucle existente reintenta.
- **En contra:** hay que verificar con un test que `read_timeout` cubra también la espera de las cabeceras (sin ellas, un servidor que acepta y no contesta seguiría colgando); si no las cubre, se envuelve `send()` en `tokio::time::timeout`. Los valores son una decisión (ver abiertas). Peor caso de una descarga parada: minutos por archivo entre intentos y esperas, de ahí la necesidad de B.

### La recarga

**B (recomendado). La recarga sólo comprueba la caché y nunca descarga.** `unloading_when_idle` usa una variante de sólo lectura (la ruta rápida de `try_load_cached`); si la caché no pasa, devuelve el error enseguida (la llamada ve `NotReady`, como ya pasa con un fallo de recarga, `idle_embedder.rs:107-112`) y la descarga, si se quiere, la hace una tarea aparte, fuera del cerrojo y con un solo vuelo a la vez.
- **A favor:** el cerrojo queda ocupado sólo lo que tarda un hash; el peor caso deja de depender de la red; cierra a la vez el riesgo de 53.
- **En contra:** cambia lo documentado («los mismos archivos verificados», `specs/storage/spec.md:356` y `.claude/CLAUDE.md`) en el caso en que los archivos ya no verifican: hay que añadir una línea en ambos. Hay que decidir quién repara la caché y cuándo; eso se solapa con 61, que hace volver al hilo de inferencia muerto y toca el mismo `IdleEmbedder::lease`.

**C. Envolver `reload` en `tokio::time::timeout`.** Más simple, un solo sitio. Pero el cerrojo sigue tomado durante todo el plazo, y vencido el plazo se descarta la descarga (el futuro se suelta) mientras un hash síncrono en curso no se puede cortar. Sólo acota; no arregla la espera.

### Causa del fallo

**D (recomendado). Guardar la última causa en `RetryExhausted` y registrar cada intento.** Un campo `last_cause: String` (código de estado, o el error de reqwest con su cadena de `source`), y un `log::warn!` por intento con archivo, intento y causa. Se retira o se usa la variante `Http` hoy sin construir. El mensaje de `clawg status` pasa a decir, por ejemplo, «… after 3 attempts: HTTP 503».
- **En contra:** una URL de un mirror o un mensaje de proxy puede aparecer en la causa; hay que mostrar el host, no la URL con credenciales (`https://user:pass@…` es válida en reqwest).

### Trabajo bloqueante y rehashing

**E (recomendado). Fase de caché en `spawn_blocking` y lectura del tokenizer fuera del bloque `async`.** Lectura y hash de `try_load_cached` / `file_matches_sha` / `load_cached_or_err` en una tarea bloqueante; la escritura de la descarga con `tokio::fs`; la resolución del tokenizer al inicio del worker (`runtime.rs:404-408`) también en `spawn_blocking`.
- **En contra:** una tarea bloqueante no se cancela y el apagado la espera (55); un hash de ~470 MB es corto pero no nulo.

**F (recomendado). Hashear mientras se descarga** (`Sha256::update` por trozo) y quitar la relectura de `:757-767`, más un único lector que reemplace los tres.

**G. Saltar el hash en una recarga si tamaño y mtime no cambiaron desde el último hash bueno.** Quita 470 MB de lectura por recarga, pero deja de detectar una corrupción del mismo tamaño (la sesión de ONNX rechazaría un protobuf roto, no uno con un peso cambiado). No se recomienda sin medir (62 mide CPU y memoria del embedder).

### `CLAWG_HF_BASE_URL`

**H (recomendado). Dejarla como característica documentada.** Un espejo para redes cerradas y para el humo de instalación es un uso real (tres notas archivadas). Documentarla junto a `CLAWG_PROFILE` y `CLAWG_EMBEDDER_IDLE_MINUTES` en `.claude/CLAUDE.md`, exigir `https://` salvo `localhost`/`127.0.0.1`/`[::1]`, y rechazar un valor inválido con un error que nombre la variable (no caer en silencio a Hugging Face). Corregir el comentario viejo.

**I. Restringirla a `testing`.** Cierra la superficie, pero `install-smoke` pierde su palanca manual y se rompe un uso documentado en notas de verificación.

### Costuras

**J (recomendado, al final).** Una estructura de configuración (oráculo, URL, espera, manifiesto) y un solo lector de hash en lugar de cuatro funciones encadenadas y tres lectores. Es un refactor sin cambio de conducta, en commits aparte, con los tests existentes como red.

## Descartado

| Opción | Razón técnica |
|---|---|
| `timeout` total de la petición | Corta una descarga legítima y lenta de 470 MB; `read_timeout` mide lo que importa (que lleguen bytes). |
| Reanudar descargas con `Range` | `:658` empieza siempre de cero a propósito; el problema aquí es el cuelgue, no el ancho de banda. |
| Quitar el hash tras la descarga y confiar en TLS | El SHA-256 fijado es la garantía de integridad, y `CLAWG_HF_BASE_URL` puede apuntar a un espejo. |
| Dejar de hashear en la ruta de arranque | El hash de los bytes que reciben `commit_from_memory` es deliberado (`bootstrap.rs:376-380`). |

## Interacciones

- **61 embedder-recovery (nuevo).** Hermano: lo suyo es hacer volver al hilo de inferencia muerto. Los dos tocan `IdleEmbedder::lease`; hay que fijar el orden y quién es dueño de la reparación de la caché (B).
- **53 outbox-lifecycle.** Planea reintentar en la sesión los fallos pasajeros; cada reintento pasa por `lease` y por la recarga. Con la descarga bajo el cerrojo (hoy) esos reintentos se apilan detrás de ella; este cambio debe aterrizar antes.
- **55 shutdown-budget.** Una descarga parada es asíncrona y se suelta con el runtime (sólo las tareas bloqueantes se esperan, `openspec/changes/shutdown-budget/exploration.md`, fase 4); no entra en ese presupuesto. Pero E mueve hashes a `spawn_blocking`, que sí entran en la fase 4, y una petición HTTP detenida en un `lease` colgado alarga la fase 1: los plazos de A y B la acotan.
- **46 embedder-idle-unload (archivado).** Su spec («Ciclo de vida del embedder», `openspec/specs/storage/spec.md`) cambia en el caso de caché inválida (B).
- **40 install-smoke / 47 release-and-self-update (archivados).** Usan `CLAWG_HF_BASE_URL` como palanca; H la conserva.
- **17 model-reembed.** No cambia el contrato de embedding ni los vectores.
- **62 embedder-footprint.** Mide CPU y memoria; G no se decide sin sus números.

## Alcance candidato

1. `Client` compartido con `connect_timeout` y `read_timeout` (A).
2. Recarga que sólo verifica la caché; reparación fuera del cerrojo (B), con spec y `.claude/CLAUDE.md` ajustados.
3. Causa del último intento en `RetryExhausted` y un `log::warn!` por intento (D).
4. Fase de caché y lectura del tokenizer fuera de los hilos asíncronos (E); hash incremental en la descarga (F).
5. `CLAWG_HF_BASE_URL` documentada y validada (H); comentario corregido.
6. Una configuración y un lector de hash (J).

### No-alcance

- Inferencia, chunking, cambios al contrato de embedding o a `memory.db`.
- Reanudar descargas, descargar en paralelo o cambiar el origen del modelo.
- Hacer volver al hilo de inferencia muerto (61) y medir CPU/memoria (62).
- Un salto del hash por mtime (G) sin números.

## Decisiones abiertas

Son del usuario.

- [ ] **Valores de los plazos.** ¿`connect_timeout` 30 s y `read_timeout` 60 s, u otros? ¿Configurables por entorno?
- [ ] **Reparar la caché.** Con B, ¿quién baja de nuevo los archivos que ya no verifican: una tarea disparada por la recarga fallida, o sólo el reinicio del daemon? ¿Es de este cambio o de 61?
- [ ] **`CLAWG_HF_BASE_URL`.** ¿H (documentada, https salvo loopback, valor inválido es error) o I (sólo `testing`)?
- [ ] **Salto del hash por mtime (G).** ¿Se acepta perder la detección de corrupción del mismo tamaño, o se mantiene el hash completo?
- [ ] **Forma del mensaje.** ¿La causa va en el texto de `RetryExhausted` o en un campo aparte de `/health`?
- [ ] **Colapsar las costuras (J).** ¿En este cambio o aparte?

## Cómo se mide

Tests RED primero (TDD estricto), todos con archivos falsos y servidores simulados; nada necesita el modelo real ni la red:

1. **Servidor parado.** Un `TcpListener` que contesta `200` con cabeceras y unos bytes y duerme sin cerrar: `ensure_model_with_oracle_spec` no vuelve hoy (se detecta con `tokio::time::timeout` en el test); con A vuelve con error. Variante con un listener que acepta y no contesta, para decidir si `send()` necesita su propio plazo.
2. **Recarga colgada.** Un `ReloadFn` que devuelve `futures::future::pending()`: un `embed_with_role` concurrente no vuelve hoy; con B responde `NotReady` dentro de un plazo.
3. **Causa.** `mockito` con 503: el error final nombra 503 (hoy sólo cuenta intentos, `model_download.rs:1172`).
4. **Hilo libre.** `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]`: un `sleep` de 10 ms concurrente no se retrasa mientras `ensure` verifica un archivo grande.
5. **Contador de hashes.** Un oráculo que cuenta lecturas: con un archivo pequeño faltante el ONNX se hashea una vez, no tres; tras una descarga, una vez.
6. **URL base.** Unitarios puros de la validación: `https://` pasa, `http://127.0.0.1` pasa, `http://ejemplo.com` es error que nombra la variable.

## Riesgos

- **Plazos demasiado cortos en redes lentas:** `read_timeout` mide la pausa entre lecturas, no la velocidad; el reintento y el bucle del bootstrap absorben un falso positivo, pero un valor excesivamente bajo convierte una red mala en un bucle de descargas desde cero (`:658`).
- **Reparación de caché sin dueño:** con B, si nadie repara, el modelo suelto queda `NotReady` hasta reiniciar; hay que decidirlo (ver abiertas) antes de aplicar.
- **Tareas bloqueantes no cancelables (E)** retrasan el apagado hasta que terminan (55).
- **Cadena de errores de reqwest con URL:** la causa puede traer un host de mirror o credenciales; mostrar sólo el host.
- **Validar la variable** puede dejar sin arrancar el modelo a quien hoy usa un `http://` remoto; el error nombra la variable y la salida.
- **Solapamiento con 61** sobre `IdleEmbedder::lease`; se mitiga fijando el orden antes de aplicar.
