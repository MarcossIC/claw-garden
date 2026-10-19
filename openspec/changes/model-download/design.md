# model-download — Diseño

## D1. Una entrada y un lector, primero

**Elegido:** `ModelSource` junta lo que hoy se pasa por cuatro funciones encadenadas (`ensure_model` → `ensure_model_with_oracle` → `ensure_model_with_oracle_spec` → `_inner`): URL base, oráculo de disco, espera entre intentos, plazos de red y manifiesto. `ModelSource::production` arma la del daemon; un test arma la suya. Quedan dos entradas: `ensure_model` (producción) y `ensure_model_from(base_dir, &source, progress)`. Los tres lectores con hash (`try_load_cached`, `load_cached_or_err`, `file_matches_sha`) pasan a uno, `read_verified`, que devuelve los bytes o por qué no sirven (falta, no se lee, otro hash).

Va en el primer commit, sin cambio de conducta y con los tests que ya existen como red: los cambios que siguen (plazos, causa, hash único, caché sola) tocan cada uno un solo sitio en vez de tres.

| Alternativa | Por qué no |
|---|---|
| Hacerlo al final | Cada cambio anterior tocaría las cuatro capas y los tres lectores |
| Un `Default` con campos sueltos | Los tests no podrían pasar un oráculo sin un `Box` o un genérico; con referencias la estructura es de vida corta y no copia nada |

## D2. Plazos: un cliente por corrida, 30 s y 60 s

**Elegido:** `ensure_model_from` arma un `reqwest::Client` con `connect_timeout` y `read_timeout` de `ModelSource::timeouts` y lo usa para todos los archivos de la corrida. En `reqwest` 0.12.28 el plazo de lectura corre desde que sale la petición hasta las cabeceras (`PendingRequest::poll`, `async_impl/client.rs:3053`) y después por cada trozo del cuerpo (`ReadTimeoutBody`, `async_impl/body.rs:328`), así que cubre los dos casos de R1 sin envolver `send()`.

- **Sin plazo total:** cortaría una descarga legítima y lenta de 470 MB; lo que importa es que lleguen bytes.
- **Fijos:** 30 s para conectar sobra para un proxy corporativo; 60 s sin un byte es una conexión muerta. No hay variable de entorno: nada pidió cambiarlos y una variable más es superficie.
- **Por corrida, no global:** construir el cliente puede fallar (TLS); dentro de `ensure` el fallo es un `DownloadError::Client` que el bootstrap reintenta como cualquier otro. Una `Lazy` global no tendría a quién devolverlo. Una corrida es una vez por arranque o reparación, así que el costo no se nota.
- **Plazos en `ModelSource`:** el test de R1 usa plazos de milisegundos con un servidor real en un hilo; con el reloj de tokio pausado, el avance automático podría vencer el plazo de conexión antes de que conecte y el test pasaría por la razón equivocada.

## D3. La causa: host y última causa, nunca la URL

**Elegido:** `RetryExhausted { file, host, attempts, cause }`, con el texto `retry budget exhausted for {file} from {host} after {attempts} attempts: {cause}`. `cause` es `HTTP <estado>` para una respuesta no exitosa, o el error de `reqwest` sin URL (`without_url`) seguido de su cadena de `source()` separada por `: `, que es donde están `connection refused`, el error de TLS o `operation timed out`. El host sale de la URL base parseada, sin usuario ni contraseña. Cada intento fallido deja un `warn!` con la misma causa. La variante `Http`, que nada construía, se va.

| Alternativa | Por qué no |
|---|---|
| Un campo aparte en `/health` | Decidido: el texto ya llega a `embedder_error` y al 503 de un save, que es donde se mira |
| La URL entera | Una URL con `user:pass@` es válida para `reqwest` y terminaría en el log y en `/health` |
| Guardar la causa de cada intento | El último basta para actuar; los anteriores quedan en el log |

## D4. La recarga sólo lee la caché; la reparación va aparte

**Elegido:** `load_cached_model(base_dir)` lee y verifica la caché sin red y devuelve un `DownloadError` si algo no pasa. Las dependencias del bootstrap pasan de una tupla de tres a `EmbedderDeps { ensure, cached, load, verify }`: `cached` es esa lectura en producción, y los embedders de prueba devuelven lo mismo que su `ensure`. `unloading_when_idle` recibe `cached` para la recarga y `ensure` para la reparación:

1. La recarga (bajo el cerrojo de `IdleEmbedder`, como hoy) llama a `cached`. Si pasa, `load` en `spawn_blocking`, como hoy.
2. Si no pasa, lanza la reparación y vuelve con el error; `lease` responde `NotReady` y el log dice que la caché se está bajando otra vez.
3. La reparación es `ensure(base_dir, None)` en una tarea del runtime, fuera del cerrojo. Un `AtomicBool` hace que haya a lo sumo una: una recarga que la encuentra en curso no lanza otra. Al terminar deja el resultado en el log y libera la marca.
4. La llamada siguiente vuelve a leer la caché; si la reparación terminó, carga.

El cerrojo queda ocupado lo que tarda leer y hashear la caché, nunca lo que tarda la red. `IdleEmbedder` no cambia: sigue recibiendo un `ReloadFn`, así que 61 (embedder-recovery) toca `lease` sin pisar esto.

| Alternativa | Por qué no |
|---|---|
| Reparar sólo al reiniciar | Decidido: el daemon quedaría sin vectores hasta un reinicio que nadie sabe que hace falta |
| Envolver la recarga en `tokio::time::timeout` | El cerrojo seguiría tomado durante todo el plazo y la descarga se soltaría a medias |
| Pasar un modo a `EnsureModelFn` (descargar o no) | Cambia la firma de todos los `ensure` de los tests y mezcla dos operaciones con efectos distintos en una |
| Que la reparación cargue el modelo al terminar | Duplicaría la carga de `lease`; la llamada siguiente ya lo hace con el mismo camino |

## D5. Fuera del runtime y un hash por archivo

**Elegido:**

- **La fase de caché** (`check_cache`) corre en `spawn_blocking`: lee cada archivo del manifiesto con `read_verified` una vez, guarda los bytes de los que pasan y anota los que faltan. Si no falta ninguno, arma `ModelFiles` con esos bytes; si falta alguno, se bajan sólo esos y los bytes ya verificados no se vuelven a leer.
- **El hash mientras se baja:** `download_one` actualiza un `Sha256` y acumula los bytes con cada trozo que escribe en el `.partial`; al terminar compara el hash y, si coincide, renombra y devuelve los bytes. No hay relectura del archivo bajado ni lectura final de la caché.
- **El cierre del archivo** (`sync_all` y el `rename`) va en `spawn_blocking`: sobre 470 MB, `sync_all` puede tardar segundos. Cada trozo se escribe con `std::io::Write` en el hilo del runtime: son unos KB al caché de páginas del sistema, y `tokio::fs` pediría el feature `fs` de tokio, que el workspace no tiene, para hacer lo mismo en un hilo bloqueante por trozo.
- **El tokenizer del worker:** `ingest_worker_future` resuelve el tokenizer y el chunker en `spawn_blocking`. El lector es un parámetro de esa función para que el test compruebe en qué hilo corre; producción pasa `load_verified_tokenizer`. Si la tarea entrara en pánico, el chunker se arma sobre la celda vacía y parte por bytes hasta que la carga del modelo la llene, como con un archivo que falta.
- **Mismos bytes:** lo que recibe la sesión es exactamente lo que se hasheó, en los dos caminos; se mantiene la decisión de `bootstrap.rs:376-380`.

El costo es memoria: con un archivo chico faltante, los ~470 MB del ONNX quedan en memoria mientras se baja el chico. Hoy se leían igual al final, así que el pico es el mismo. Lo que sí es nuevo es que el cuerpo de una respuesta se acumula en memoria y no sólo en disco, y que la reserva inicial sale de `Content-Length`: un espejo o un proxy que anunciara 1 TB tumbaría el proceso al reservarlo. Por eso un archivo que anuncia o trae más de 600 MB (`MAX_FILE_BYTES`, lo mismo que pide libre el chequeo de disco y más que cualquier archivo del modelo) termina con `TooLarge`, sin reintentar, como un SHA-256 distinto: el origen no va a mandar otra cosa en el intento siguiente.

Contar los hashes no tiene un punto de inyección natural: el test de R4 usa un registro de rutas leídas que existe sólo con `cfg(test)` dentro de `read_verified`, filtrado por el directorio temporal del test, porque la fase corre en otro hilo y un `thread_local` no la vería.

## D6. `CLAWG_HF_BASE_URL`: documentada y validada

**Elegido:** `base_url_from(raw: Option<&str>) -> Result<Url, String>`, pura: vacía o ausente es el origen de Hugging Face; si no parsea, o el esquema no es `https`, o es `http` con un host que no es `localhost`, `127.0.0.1` o `[::1]`, el error dice `CLAWG_HF_BASE_URL=<valor> …` y qué se acepta. `ModelSource::production` la llama y `ensure_model` devuelve `DownloadError::BaseUrl` antes de chequear el disco o pedir nada. La `/` final se quita al armar cada URL de archivo. Va documentada en `.claude/CLAUDE.md` junto a las otras variables del daemon.

La integridad no dependía de la variable (los cinco SHA-256 fijados se verifican con cualquier origen); lo que se cierra es mandar el pedido en claro a un host remoto y caer en silencio a Hugging Face con un valor mal escrito. Un error de configuración se reintenta como cualquier fallo de descarga (`classify`): el bootstrap no distingue configuración de red, y mientras tanto `embedder_error` dice qué corregir.

| Alternativa | Por qué no |
|---|---|
| Sólo con el feature `testing` | Decidido: el espejo para redes cerradas y la prueba de instalación son usos reales |
| Aceptar `http://` remoto con un aviso | El aviso se pierde en el log y el pedido sale igual en claro |

## Probar a mano

Sobre el perfil dev, con un build de la suite:

1. `CLAWG_HF_BASE_URL=http://example.com clawg serve --profile dev --port 0` y `clawg status --profile dev`: `embedder_error` nombra la variable.
2. Con un servidor local que acepta y no contesta (`nc -l 8099`), `CLAWG_HF_BASE_URL=http://127.0.0.1:8099` y un directorio de datos sin modelo: a los ~3 minutos (tres intentos de 60 s más las esperas) `embedder_error` dice `operation timed out` y el host.
3. Con el modelo cargado, `CLAWG_EMBEDDER_IDLE_MINUTES=1`, borrar `tokenizer_config.json` de `models/` y esperar que se suelte: una búsqueda responde `503 embedder_not_ready` enseguida, el log dice que la caché se baja otra vez, y una búsqueda posterior responde.
