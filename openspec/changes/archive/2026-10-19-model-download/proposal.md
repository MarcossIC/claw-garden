# model-download — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | — (antes de 53 outbox-lifecycle y de 61 embedder-recovery) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`bf5c1e4`).

1. **Una descarga parada no vuelve.** `reqwest::get` (`crates/clawg-core/src/memory/model_download.rs:671`) arma un cliente sin `connect_timeout` ni `read_timeout`, y el reintento sólo corre tras un `Err` (`model_download.rs:650-652`, `bootstrap.rs:626-660`). Un servidor o un proxy que acepta y deja de mandar deja el embedder en `loading` para siempre.
2. **La recarga descarga bajo el cerrojo.** `lease` llama a `reload` con `self.model.lock()` tomado (`crates/clawg-core/src/memory/idle_embedder.rs:100-105`) y la recarga pasa por `ensure` (`bootstrap.rs:394`), descarga incluida. Con un archivo de `models/` borrado o dañado, toda llamada que necesita un vector espera detrás de cientos de MB mientras `/health` dice `ready`.
3. **La causa se pierde.** Un envío fallido, un estado no exitoso y un error del flujo se descartan (`model_download.rs:671-681`, `:715-718`); `RetryExhausted` (`:335`) sólo cuenta intentos.
4. **Trabajo bloqueante en hilos de tokio, y hashes de más.** Leer y hashear los ~470 MB del ONNX corre dentro de código `async` (`model_download.rs:434`, `:465`, `:491`), hasta tres veces con un archivo chico faltante; el worker de ingest lee, hashea y parsea `tokenizer.json` en su bloque `async` (`crates/clawg-core/src/runtime.rs:404-408`).
5. **`CLAWG_HF_BASE_URL` se lee en release sin validar** (`model_download.rs:197-199`), con un comentario que dice que es para tests.
6. **Cuatro capas de `ensure_model*` y tres lectores con hash** (`model_download.rs:348`, `:360`, `:382`, `:416`; `:497`, `:537`, `:586`) multiplican cada cambio.

## Objetivo que sirve

Transversal: sin modelo no hay memoria. Que conseguirlo nunca cuelgue al daemon y que un fallo diga por qué sostiene los cinco objetivos.

## Decisiones

Cerradas con el usuario; el porqué está en el [diseño](design.md).

- **Plazos fijos:** `connect_timeout` 30 s y `read_timeout` 60 s (se reinicia en cada lectura y cubre la espera de las cabeceras), sin plazo total y sin variable de entorno.
- **La recarga sólo verifica la caché y nunca descarga.** Si la caché no pasa, responde «no listo» enseguida y lanza una única descarga en segundo plano, fuera del cerrojo; la llamada siguiente a que termine carga lo reparado. Es de este cambio, no de 61.
- **`CLAWG_HF_BASE_URL` queda documentada y validada:** `https://` con cualquier host, `http://` sólo con `localhost`, `127.0.0.1` o `[::1]`; otro valor es un error que nombra la variable.
- **La causa va en el texto de `RetryExhausted`** (lo ven `embedder_error` de `/health` y `clawg status`, y el 503 de un save), sin campo nuevo en `/health`. El texto nombra el host, nunca la URL entera.
- **Las costuras se colapsan en este cambio**, primero y en un commit aparte: una configuración de origen y un solo lector con hash.
- **No se salta el hash por tamaño o fecha** (G de la exploración): se sigue hasheando todo archivo que se carga.

## Alcance

1. `model_download.rs`: `ModelSource` (URL, oráculo de disco, espera, plazos, manifiesto) en lugar de las cuatro funciones; un lector con hash; un `reqwest::Client` con plazos por corrida; la causa y el host en `RetryExhausted` y un `warn!` por intento; la fase de caché en `spawn_blocking`; el hash mientras se descarga; `load_cached_model`, que no toca la red; la validación de `CLAWG_HF_BASE_URL`.
2. `bootstrap.rs`: las dependencias del bootstrap como `EmbedderDeps`, con la lectura de caché aparte de la descarga; `unloading_when_idle` recarga desde la caché y repara en segundo plano.
3. `runtime.rs`: la resolución del tokenizer del worker en `spawn_blocking`.
4. `serve.rs`: el cableado nuevo.
5. Docs: `.claude/CLAUDE.md`, `openspec/specs/storage/spec.md`, `openspec/specs/ingest/spec.md` y el mapa de `openspec/README.md`.

## Fuera de alcance

- Inferencia, chunking, el contrato de embedding o `memory.db`.
- Reanudar descargas con `Range`, descargar en paralelo, cambiar el origen del modelo.
- Hacer volver al hilo de inferencia muerto (61) y medir CPU y memoria (62).
- Un campo nuevo en `/health`.

## Cómo se mide el éxito

Tests que fallan hoy, todos con archivos falsos y servidores locales, sin la red ni el modelo real (ver [spec](spec.md)):

- un servidor que manda cabeceras y unos bytes y se calla, y otro que acepta y no contesta: hoy `ensure` no vuelve; con el cambio vuelve con un error que dice que se venció el plazo;
- una recarga con la caché dañada y una descarga que no termina: hoy la llamada no vuelve; con el cambio responde «no listo» enseguida, la descarga corre una sola vez en segundo plano y la llamada siguiente a la reparación carga;
- un 503 de todos los intentos: el error nombra 503 y el host, y no la contraseña de la URL;
- la fase de caché deja libre el hilo del runtime, y el worker lee el tokenizer fuera de él;
- cada archivo se hashea una vez por corrida, también el ONNX con un archivo chico faltante;
- `CLAWG_HF_BASE_URL`: tests puros de la validación.

## Datos y vuelta atrás

No toca `memory.db`, el esquema, los identificadores, los estados ni el contrato de embedding: el cambio vive en los archivos de `<data>/models/` y en cómo se consiguen. Los archivos del modelo y sus SHA-256 fijados son los mismos, así que una caché bajada por la versión anterior se carga sin descargar nada, y la versión anterior carga la que baje esta. Volver atrás no pide nada. Quien hoy fija `CLAWG_HF_BASE_URL` a un `http://` remoto ve un error que nombra la variable en `embedder_error` y lo corrige con `https://` o un espejo local.
