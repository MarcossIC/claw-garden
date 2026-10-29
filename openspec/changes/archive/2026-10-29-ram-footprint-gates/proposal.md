# ram-footprint-gates — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M (C y F); B y E esperan la segunda pasada | 90, archivado | ver [`state.yaml`](state.yaml) |

La revisión de la primera versión de esta propuesta cambió el mecanismo de C. Leyó el código de ONNX Runtime 1.24.2, el que baja `ort` 2.0.0-rc.12, y de esa lectura sale una predicción: cargar por la ruta ahorraría menos de lo que pide la puerta. También ajustó cuatro puntos de F. Lo que dijo sobre clawg se comprobó contra el código. Lo que no se sostuvo quedó afuera, y se explica en cada decisión.

## Problema

La [primera pasada](exploration.md#primera-pasada-prod-sin-hooks-2026-10-24-a-2026-10-29) se midió en prod, sin hooks. De ella salen dos costos que no dependen de los hooks:

1. **Cada carga del modelo lleva el proceso a ~1,6 GiB**, unos 830 MiB por encima del modelo cargado. Es el máximo del proceso. Pasa al arrancar y unas 11 veces por día.
   - Una parte es el ONNX entero en un `Vec` de 448,5 MiB. `read_verified` lo lee con `std::fs::read` para hashearlo (`crates/clawg-core/src/memory/model_download.rs:605`), `ModelFiles::onnx` lo lleva (`:301`) y `build_session` se lo pasa a `commit_from_memory` (`crates/clawg-core/src/memory/embedder.rs:524`). El `Vec` sigue vivo mientras ONNX Runtime arma la sesión.
   - La corrida que baja el modelo junta el cuerpo en otro `Vec` del mismo tamaño (`download_one`, `:730`) y carga desde ahí.
   - El resto, unos 380 MiB, es según el código de ONNX Runtime 1.24.2 (sin medir) la segunda copia de la tabla de embeddings (366 MiB). Aparece mientras ONNX Runtime pasa la tabla del protobuf a un tensor (`ConvertInitializersIntoOrtValues`). C no la toca. Sólo la sacan los pesos como datos externos o el formato `.ort` (ver «No-alcance»).
2. **El refresco de docs de cada inicio de sesión vuelve a embeber archivos enteros.** `refresh` trocea un doc que cambió y embebe todos sus pasajes, en tandas de 16 de ese doc solo (`embed_passages`, `crates/clawg-core/src/memory/project_docs/mod.rs:464`), aunque la mayoría no haya cambiado. Los refrescos de más de 1.700 pasajes tuvieron el proceso en ~1,16 GiB de 2 a 6,5 min. Sobre los 62 intervalos entre refrescos reales de prod, el 68,4 % de los pasajes reembebidos tenía el mismo `indexed_text` que el índice ([«F»](exploration.md#f-el-refresco-de-docs-reutiliza-los-pasajes-que-no-cambiaron)).

## Objetivo

Transversal: que una carga y un refresco no pidan más memoria ni más tiempo que lo que su trabajo necesita. No cambia qué se guarda, cómo se busca, los vectores ni el contrato de embedding.

## Decisiones

1. **C verifica `model.onnx` sobre una vista de sólo lectura del archivo y carga esa misma vista.**
   - **Cómo.** El archivo se abre una vez. En Windows se abre sin compartir escritura ni borrado (`share_mode(FILE_SHARE_READ)`). Se mapea con `memmap2::Mmap::map`, nunca con `map_copy`, porque una vista copy-on-write cobra commit por la vista entera. Se le calcula el SHA-256 a la vista y la vista va a `commit_from_memory`.
   - **Por qué alcanza.** Lo que se hashea son las mismas páginas que parsea ONNX Runtime. ort pasa sólo el puntero y el largo, y la `Session` no toma prestado el buffer. Cuando `commit_from_memory` vuelve, haya armado la sesión o no, se sueltan la vista y el archivo, y una reparación puede reemplazarlo.
   - **En Unix.** La guarda es el directorio `0700`: quien escriba ahí puede cambiar el archivo en el lugar durante la carga, o truncarlo (`SIGBUS`). clawg nunca escribe `model.onnx` en el lugar: baja a `.partial` y renombra.
   - **Lo que suma.** El mapeo es un `unsafe`, con un comentario que dice por qué vale, y `memmap2` es una dependencia nueva de `clawg-core`. Los archivos chicos del modelo y del tokenizer siguen como hoy.
   - **Por qué no `commit_from_file`, la primera versión de esta decisión.** Desde un archivo, ONNX Runtime parsea en bloques de 4 MiB. Protobuf hace crecer el string de la tabla de 366 MiB hasta 543 MiB de capacidad con el factor 1,5× de MSVC, y el commit cuenta la diferencia. Así, se predice que la carga por ruta ahorraría unos 272 MiB, debajo de los 300 de la puerta. Además leía el archivo dos veces y obligaba a aflojar `specs/storage`. La puerta mide igual las dos variantes.
2. **La corrida que baja el ONNX ya no lo junta en memoria.** Lo escribe a `.partial` hasheándolo mientras llega, hace `sync_all` y lo renombra, como hoy, pero sin el `Vec` del cuerpo (`model_download.rs:730`). Después lo carga como una recarga: lo mapea y lo verifica desde la caché. Sólo en esa corrida el ONNX se hashea dos veces. `MAX_FILE_BYTES` (`:836`) sigue igual: los archivos chicos se siguen entregando desde memoria.
3. **La carga y la reparación no se pisan.** La reparación vive en el mismo daemon (`repair_in_background`, `crates/clawg-core/src/memory/bootstrap.rs:489`), y ningún otro proceso escribe en `<data>/models/`.
   - **El candado.** La carga lo toma desde que abre el archivo hasta que vuelve `commit_from_memory`. La reparación lo toma antes de renombrar `.partial` sobre `model.onnx` (`commit_file`, `:875`).
   - **Si la apertura choca.** Si un antivirus lo tiene abierto, la apertura de la carga puede fallar por compartir: errores 32, 33 o 5 de Windows. Se reintenta con espera creciente hasta unos 2 s. Si sigue fallando, el pedido contesta que el modelo no está listo, con una línea en el log, y **no lanza una reparación**. Hoy cualquier error de lectura cuenta como archivo faltante (`check_cache`, `:574`), y bajaría de nuevo 448 MiB por un choque de un segundo. Tampoco vuelve a leer el ONNX a un `Vec`.
   - **Los tests.** Ni el código ni los tests comparan `ErrorKind`, porque Rust deja el 32 como `Uncategorized`.
4. **La línea de cada carga dice su pico privado.** `IdleEmbedder` lee `PeakPagefileUsage` antes y después de la carga, y lo suma a la línea `embedding model loaded again in …` (`load_line`, `crates/clawg-core/src/memory/idle_embedder.rs:85`). La lectura viene de una segunda sonda que pasa `clawg serve`, como `MemoryProbe` (`crates/clawg-core/src/metrics/mod.rs:15`), porque Windows ya lo lee en `crates/clawg-cli/src/daemon/process.rs:114`.
   - Si el pico subió, la línea dice el valor nuevo: el pico del proceso durante la carga, que otra tarea que corra a la vez pudo haber subido. Si no subió, dice que el pico es de antes de esa carga.
   - Va sólo al log, sin tocar `metrics.db`, y en Linux no se dice nada.
   - Hace falta porque, después de C, una carga queda cerca del refresco grande, y `process.private_peak_bytes` ya no dice cuál de los dos fue.
5. **F reutiliza el vector de cada pasaje de un doc cambiado** cuyo `indexed_text` entero (`breadcrumb` y `body`) ya está en el índice de ese doc. La condición es que el `model_id` guardado del doc sea el del refresco, que se lee una vez (`mod.rs:420`).
   - **Por valor.** Se leen pares de texto y bytes del vector, nunca ids de fila. Un texto repetido en el doc reutiliza en todos sus pasajes. Sólo se embebe lo demás.
   - **Todo o nada.** El doc se sigue reemplazando entero en una transacción (`replace_doc`, `mod.rs:544`).
   - **Qué no reutiliza.** Un doc de otro modelo no reutiliza nada, ni un archivo movido, porque la ruta está en el `indexed_text`.
   - **Por qué no hace falta más.** Los refrescos de una raíz ya corren de a uno (`DocIndex::begin`, `mod.rs:209`), así que nadie cambia lo leído antes de que se escriba. El review pedía además descartar lo reutilizado si la sesión cambiaba antes de escribir. No hace falta, porque el `model_id` es una constante, `same_model` (`idle_embedder.rs:216`) rechaza una recarga con otro id, y una recarga vuelve a cargar los mismos bytes verificados con las mismas opciones.
6. **Un test fija el contrato de embedding, para que el `model_id` lo siga.** `E5_MODEL_ID` (`embedder.rs:135`) es una constante. Un test fija juntos `E5_MODEL_ID`, los dos prefijos, `EMBED_MAX_TOKENS`, `EMBEDDING_DIM`, el pooling y los SHA-256 fijados del modelo y del tokenizer. Si cambia uno, el test falla y pide revisar si `E5_MODEL_ID` tiene que subir.
   - Hoy, un doc que no cambia ya conserva sus vectores si cambia el contrato sin que cambie el id (`plan_refresh`, `mod.rs:121`). Con F eso alcanza también a los pasajes sin cambios de un doc cambiado: un pasaje así conserva su vector hasta que cambie el id o se reindexe.
   - Guardar la huella del contrato en el índice obligaba a reindexar todas las raíces una vez, y se descartó.
7. **`RefreshReport` cuenta los pasajes reutilizados (`reused`)**: los pasajes, no los textos distintos. La línea `indexed the docs of …` los muestra (`crates/clawg-cli/src/daemon/api.rs:2675`). Con eso, la segunda pasada mide F en prod.
8. **C y F son commits propios** y cada uno puede salir solo.

## Alcance

1. C:
   - la vista hasheada y cargada;
   - el candado entre carga y reparación, y el reintento de la apertura sin lanzar una reparación;
   - la corrida que baja, sin el cuerpo en memoria;
   - el pico de cada carga en su línea;
   - la dependencia `memmap2`.
2. F: la reutilización por `indexed_text` y `model_id`, el test del contrato, y `reused` en el reporte y en el log.
3. Los cambios de spec:
   - [`specs/storage`](../../../specs/storage/spec.md):
     - «Un hash por archivo, fuera de los hilos del runtime». La exigencia de que la carga reciba los mismos bytes que se hashearon se queda. Se suma que esos bytes pueden ser una vista de sólo lectura, nunca copy-on-write, y cómo se guarda en cada sistema. Cambia la frase sobre el archivo bajado que queda en memoria.
     - «Cada carga dice quién la pidió y cuánto costó», con el pico.
     - Un requisito nuevo, para que carga y reparación no se pisen.
   - [`specs/project-docs`](../../../specs/project-docs/spec.md), «Refresco incremental»:
     - la regla de reutilización, por valor y de a doc entero;
     - `reused`;
     - que el resultado es equivalente a un reindexado;
     - que el `model_id` sigue al contrato por el test.
4. `.claude/CLAUDE.md` y la fila 92 de `openspec/README.md`.

### No-alcance

- **B y E.** Los decide la segunda pasada, con hooks.
- **D.** No pasó su puerta y queda para import-job (87).
- **Mediciones aparte, con autorización.** La deriva del piso y el crecimiento con el modelo siempre cargado.
- **El orden de las tandas del índice de docs (62) y el tokenizer (91).** Para 62 quedan dos datos de la revisión:
  - Topar la tanda por S² y no por tokens bajó, en un codificador sintético, de +452 a +112 MiB.
  - Su análisis de la arena no aplica tal como está. La arena ya está apagada (`cpu_provider`, `embedder.rs:158`), así que los 1,16 GiB de un refresco son heap o tensores vivos, no retención de la arena.
- **Seguimientos que cambian el artefacto o lo que se guarda:**
  - **Los pesos como datos externos o en `.ort`.** Es lo que más bajaría la carga y el modelo quieto: se predicen ~100–250 MiB de pico sobre el piso y ~85–150 MiB con el modelo quieto. En Linux, con un modelo sintético, la revisión midió 112 MiB. Conviene decidirlo antes de E, porque le quitaría a B y a E casi toda su razón de memoria.
  - El troceador por párrafos.
  - La caché global por texto.
  - La huella del contrato como columna.
  - Un registro de verificación por identidad del archivo, que evitaría el hash en las recargas.
- **La ruta que sale del handle y la comparación de `FILE_ID_INFO`.** Sólo hacen falta si la puerta deja `commit_from_file`.

## Cómo se mide

Tests que fallan sin el cambio:

1. **C.**
   - **La vista.** La comprobación de la caché devuelve el ONNX verificado como una vista, sin copiar sus bytes, y la carga recibe esa vista.
   - **El rechazo.** Un `model.onnx` con un byte cambiado se sigue rechazando. Se prueba con un ONNX mínimo armado en el test, con su SHA-256 inyectado como esperado. ONNX Runtime arma una sesión con `commit_from_memory` desde la vista de ese ONNX.
   - **La guarda en Windows.** Mientras la carga lo tiene, fallan abrirlo para escribir, renombrarlo, borrarlo y renombrar otro encima; se acepta el error 5 o el 32. Una lectura con `FILE_SHARE_READ` abre. Después de la carga, todo eso anda. El test corre también en un directorio temporal cuya ruta tiene «ñ».
   - **El candado.** Una reparación espera a que la carga suelte el archivo.
   - **El choque al abrir.** Una apertura que sigue chocando contesta que el modelo no está listo y no lanza ninguna reparación.
   - **La corrida que baja.** No guarda el cuerpo, carga desde la caché y hashea el ONNX dos veces sólo en esa corrida. `each_file_is_hashed_once_per_run` cambia para decirlo.
   - **El pico.** La línea de una carga dice su pico.
2. **F.**
   - **Con un embebedor falso.** Se usa uno determinista, que saca el vector de un hash del texto y anota qué textos le piden. Si se edita una sección de un doc indexado, el refresco embebe sólo los pasajes que cambiaron. El doc queda igual, byte a byte, al de un reindexado completo con el mismo embebedor. Cada vector reutilizado es el blob que estaba guardado, y `reused` cuenta los pasajes. Con otro `model_id` no reutiliza ninguno. Un texto repetido reutiliza en todos sus pasajes.
   - **El test del contrato** falla si cambia uno de sus valores.
   - **Con el modelo real**, fuera de CI, la comparación con un reindexado es con tolerancia: 1e-5 por componente o coseno ≥ 1 − 1e-6. No es bit a bit, porque un pasaje embebido solo y en una tanda difieren cerca de 1e-7.

**La puerta propia de C**, fijada antes de medir:

- **La corrida.**
  - **Brazos.** Tres, de diez cargas cada uno: `commit_from_memory` con el `Vec` (hoy), `commit_from_file` (C, sólo en el arnés) y `commit_from_memory` sobre la vista mapeada (C′).
  - **Cargas.** Cada carga corre en un proceso nuevo, dentro de su propio job object. Los brazos van intercalados, y se descarta una carga de calentamiento por brazo.
  - **Entorno.** El modelo real, bajado a un directorio temporal, en release, con la función de carga real y las opciones de prod (arena apagada, nivel 3, un hilo por CPU), sin tocar ningún perfil. Hace falta autorización para el build release y la descarga.
- **Lo que se lee de cada carga.** El `PeakProcessMemoryUsed` del job y, en el hijo, `PeakPagefileUsage`, `PeakWorkingSetSize`, `PrivateUsage` después de la carga, y los ms de verificar y de armar la sesión.
- **Cuándo pasa un brazo.**
  - La mediana del pico de commit baja al menos 300 MiB.
  - Su peor carga queda por debajo de la mejor de hoy.
  - La mediana del tiempo total con caché tibia no sube más de max(150 ms, 15 %).
- **Qué queda.** Si pasa C′, sola o con C, queda C′. Si pasa sólo C, el mecanismo cambia a `commit_from_file`, con la ruta que sale del handle y la comparación de `FILE_ID_INFO`. Si no pasa ninguno, C se revierte y se cierra con lo medido.
- **Linux.** La misma corrida (`VmHWM`) confirma el mecanismo, pero no decide.

Del código de ONNX Runtime 1.24.2 se espera, sin medir, que C baje unos 272 MiB en Windows y C′ unos 448. Si en el brazo de C la distancia entre el pico de commit y el de working set crece unos 177 MiB respecto de hoy, es la capacidad del string que nunca se escribe.

La segunda pasada, con hooks, lee en prod el pico de cada carga en su línea y `reused` en cada refresco. Como las tandas se arman por doc, una edición chica da tandas de uno a tres pasajes. Por eso también lee si F baja el pico típico de un refresco.

## Datos

No toca `memory.db`, el esquema, los ids, los estados ni el contrato. F lee lo que el índice de docs ya guarda (`project_doc_chunk`, `project_doc_vectors`), que se deriva del disco y se puede rehacer. Volver atrás es revertir los commits.

El índice que deja F es equivalente al de un reindexado: tiene los mismos pasajes, con vectores iguales dentro de la tolerancia de punto flotante, y un reindexado lo rehace. Con F, los pasajes que no cambian conservan su vector hasta que cambie el `model_id`.
