# ram-footprint-gates — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S (la medición); B, C, D y F, S–M cada uno y con puerta | 90, archivado (su paso 0 sale en 0.63.0); se coordina con 62, 73, 74, 85, 87 y 91 | exploración: primera pasada medida en prod sin hooks; falta la segunda, con hooks |

**Tesis:** ram-footprint (90) sacó el refresco del índice de docs que corría en cada prompt (A) y dejó con qué atribuir cada pico de memoria (su paso 0). Quedan los brazos que dependen de una medición: despertar el modelo sólo al empezar una sesión (B), cargar el ONNX sin el `Vec` de 470 MB (C), embeber el import por tanda (D) y bajar el tiempo de ocio por defecto (E). La primera pasada sumó uno: que el refresco de docs reutilice el vector de cada pasaje cuyo texto no cambió (F). Las alternativas de B a E, analizadas contra el código, están en la [exploración de 90](../archive/2026-10-24-ram-footprint/exploration.md#enfoques-posibles). Acá están la medición que las decide y sus puertas. Este cambio nació al partir 90 cuando se archivó.

## Paso 0: el día de uso real (sin código)

Se hace con autorización y en sólo lectura, en el perfil prod con el release 0.63.0, que es el primero con el paso 0 de 90. Después de un día de uso normal con Claude Code se lee:

- `clawg metrics report --by minute --since 1d` sobre `process.private_bytes`, `process.private_peak_bytes` (Windows) y `embedder.resident`. `--by version` separa las horas de 0.62.0, que todavía refrescaban los docs en cada prompt.
- En `~/.clawg/logs/serve.log`, cuatro tipos de línea. Las de carga: `embedding model loaded again in … for a wake`, `… for a call that embeds` o `… for a call that found its worker gone`, con la memoria privada antes y después. Las de primer uso: `the model a wake loaded was first used N s later`. Las de descarga: las que terminan en `; no call used it after the wake that loaded it`. Y las de refresco de docs.
- Si en el día hubo un import, sus líneas de fase con `private N MiB`.

Responde cinco preguntas:

1. **¿Cuánto tiempo pasa cargado el modelo con A?** Se compara contra el 73 a 81 % de los minutos medido en 0.62.0 (exploración de 90, «Lo medido en prod»).
2. **¿Cuántas cargas son de un `wake` que nadie usó?** Son las descargas marcadas sobre las cargas `for a wake`. Para las demás cargas de un `wake`, cuánto tardó el primer uso. Decide B.
3. **¿La carga es el máximo del proceso?** Se compara `process.private_peak_bytes` del minuto de cada carga con la memoria después de la carga y con los demás minutos. Decide C.
4. **¿Cuánto retiene el import por encima del modelo?** Se compara la memoria al terminar `embed` y `write` con la de `decide`. Decide D. Un día de uso normal puede no tener ningún import. En ese caso se mide con un import en el perfil dev, como import-timing (88), y se dice qué build se usó.
5. **¿Se mueve el piso?** Es la mínima de los minutos con `embedder.resident = 0` a lo largo del día. Si deriva, va a 91 o es un hallazgo nuevo. Si no, cierra la duda que dejó 90.

## Primera pasada: prod sin hooks (2026-10-24 a 2026-10-29)

**Qué se leyó.** El perfil prod en Windows con 16 CPUs, en sólo lectura: `metrics.db` y `serve.log`. Son 4,28 días con 0.63.0 y 0.65.0, en tres ejecuciones del daemon (una terminó sin apagado ordenado), y la ejecución de 0.62.0 como referencia. Desde 0.63.0 hubo 46 cargas del modelo (más la de cada arranque), 14 registros de proyecto, 31 búsquedas, 72 guardados, 196 veredictos y ningún import. El tiempo de ocio fue el de por defecto, 10 minutos.

**Sin hooks.** La instalación no tenía los hooks que instala `clawg setup claude-code`: los únicos `POST /v1/projects` fueron los del puente, uno por raíz y por proceso, con `index_docs`. Por eso esta pasada no mide lo que B quiere sacar (el `wake` del hook `user-prompt-submit` en cada prompt) ni lo que sacó A (el refresco que lanzaban esos registros). Sí mide lo que no depende de los hooks: lo que cuesta una carga, el refresco de docs al empezar una sesión, el import y el piso.

**Cómo se leyó.**

- `metrics report --by minute` no dice de qué versión es cada minuto. Por eso `metrics.db` se abrió directo, en sólo lectura (`mode=ro`): `gauge_minute` (minutos desde epoch, en UTC) unida a `run` por `run_id` da la versión. Una ejecución sin `stopped_at_ms` termina donde arranca la siguiente.
- `serve.log` está en UTC, y las claves de minuto de `metrics report`, en hora local (`run.utc_offset_s`). Las horas de abajo son locales.
- `process.private_peak_bytes` es el pico de toda la vida del proceso (`PeakPagefileUsage`). Se atribuye por el minuto en que sube, no por su valor.
- `hist_minute` de `http.duration` cuenta las llamadas por minuto y ruta. Con eso se reconstruye qué minutos embebieron (los `POST` de `/v1/search`, `/v1/context`, `/v1/memories`, `/v1/projects` e `/v1/import`, más el fin de cada refresco en el log) y se repite el día con otro tiempo de ocio. Con 10 minutos la reconstrucción da las mismas 46 cargas que el log, y 720 minutos con el modelo cargado contra los 665 medidos.

### 1. Cuánto tiempo pasa cargado el modelo

| Versión | Minutos | Con el modelo cargado |
|---|---|---|
| 0.62.0 (la ejecución entera, 12 h, con un import) | 715 | 23,8 % |
| 0.63.0 | 3.560 | 12,2 % |
| 0.65.0 | 2.603 | 8,7 % |

Por día, entre el 1,7 y el 19,2 %.

- El 73 a 81 % que midió 90 eran las dos horas de un import de 54.457 filas y del refresco que le siguió, no un día normal.
- La baja no es de A: sin hooks, A no cambió nada en esta instalación. Es otro uso.
- Cada carga deja el modelo unos 11 minutos (mediana de 45 cargas), y casi todo ese tiempo es la espera de ocio. Repitiendo el día con 2 minutos de ocio quedan 231 de los 720 minutos.

### 2. Cuántas cargas son de un `wake` que nadie usó

Fueron 10 de 46 (22 %), y ninguna quedó sin uso: las 10 dicen `first used 0 s later`. Las 10 son inicios de puente, y el primer uso es el refresco de docs que lanza ese mismo registro, que embebió entre 37 y 3.990 pasajes. **No decide B.** En esta instalación no hubo ningún registro sin `index_docs`. Para un puente, el `wake` no ahorra ni cuesta nada, porque su refresco cargaría el modelo igual.

### 3. Si la carga es el máximo del proceso

**Sí.**

| Estado | Memoria privada |
|---|---|
| Piso, con el modelo suelto | 288 a 331 MiB |
| Modelo cargado y quieto | 748 a 781 MiB (el modelo suma de 443 a 469 MiB, constante) |
| Refresco grande de docs (1.700 a 4.000 pasajes), de 2 a 6,5 min | 1.150 a 1.171 MiB, muestreado por minuto |
| Import de 0.62.0, durante `embed` | 944 a 1.011 MiB, con picos de tanda de 1.237 |
| Carga del modelo, de 0,5 a 2 s | el piso + 1.283 a 1.288 MiB: de 1.561 a 1.605 MiB |

- `process.private_peak_bytes` ya vale entre 1.561 y 1.565 MiB en el primer minuto de cada ejecución, antes de cualquier refresco: lo pone la carga del arranque. Después sólo sube en el minuto de una recarga, de 1 a 28 MiB, a medida que sube el piso.
- La carga suma unos 830 MiB por encima del modelo quieto. `model.onnx` mide 470.268.510 bytes (448,5 MiB), que es el `Vec` que C saca. El resto, unos 380 MiB, es de ONNX Runtime mientras arma la sesión, y C no lo toca.
- Una carga en release tarda de 0,5 a 2,0 s: mediana de 0,82 s en 0.63.0 (31 cargas) y de 1,17 s en 0.65.0 (15). En 0.65.0 son más lentas las dos fases: leer y verificar pasa de 512 a 709 ms de mediana, y armar la sesión, de 303 a 451 ms. No tiene una causa atribuida. En debug eran de 8,7 a 13,5 s.

### 4. Cuánto retiene el import por encima del modelo

No hubo ningún import con 0.63.0 ni con 0.65.0. Se lee por minuto el de 0.62.0 (54.457 filas de otra herramienta, el 2026-10-24): no tiene las líneas con `private`, pero sí la serie. Lo precedieron dos pasadas en seco del mismo archivo, que hacen el mismo trabajo que el import hasta `decide`.

| Momento | Memoria privada |
|---|---|
| Antes, con el modelo suelto | 284,5 MiB |
| `translate` de la primera pasada en seco | 305,6 MiB |
| Después de `decide`, en la segunda pasada en seco | 353,0 MiB |
| `embed` (18 min, con el modelo cargado) | de 944 a 1.011 MiB, con picos de 1.121 a 1.237 |
| Fin de `write` | 817 MiB |
| Después del import y del refresco que le siguió, con el modelo suelto | 341,0 MiB |

- Al terminar `embed`, el import retiene unos 250 MiB por encima del modelo cargado. Los vectores, que es lo que D deja de juntar, son 54.436 × 1.536 bytes ≈ 80 MiB, y se los ve crecer durante `embed` (de 944 a 1.011). El resto son las filas preparadas y el cuerpo, que D no toca (el cuerpo es de 87).
- El import real en dev de embed-throughput (85), en un build debug, termina `embed` en 981 MiB: lo mismo.
- Con D, un import de 54.000 filas bajaría unos 80 MiB. En seis días corrió un import.

### 5. Si se mueve el piso

**Sí, poco.**

| Ejecución | Duración | Cargas | Piso después de la primera descarga | Piso después de la última |
|---|---|---|---|---|
| 0.63.0, la primera | 21,4 h | 18 | 302,3 MiB | 326,5 MiB |
| 0.63.0, la segunda | 37,9 h | 13 | 302,2 MiB | 323,0 MiB |
| 0.65.0 | 43,4 h hasta la lectura | 15 | 287,6 MiB | 319,5 MiB |

- Sube a escalones con la actividad, de 1,3 a 2,1 MiB por carga, y no con el tiempo. El 2026-10-26 quedó entre 303,8 y 305,8 MiB durante 22 horas, con dos cargas. Embedder-idle-unload (46) midió diez recargas en debug sin trabajo entre una y otra, y no vio crecimiento. Acá hubo búsquedas, guardados y refrescos entre las cargas.
- No es la caché de SQLite: clawg no fija `cache_size`, así que cada conexión tiene la de 2 MiB por defecto. Tampoco es el tokenizer, que no cambia después de cargarse. En dos días no se ve que se aplane, y cada arranque lo vuelve a bajar.
- El import de 0.62.0 dejó el piso 57 MiB más arriba.
- Es un hallazgo nuevo, no de 91. Se mide aparte: ver «La medición del piso».
- En Windows, `process.rss_bytes` es el working set, y el sistema lo recorta. En cada una de las tres ejecuciones, su mediana fue de 31 a 67 MiB, contra unos 300 MiB de memoria privada comprometida.

### Lo demás que se vio

- **El máximo sin dueño de 90 era el refresco de docs.** Los 1.278,4 MiB de las 13 del 2026-10-24 caen en el minuto 13:49, durante el refresco de 2.258 pasajes que terminó a las 13:50. El import ya había terminado, a las 13:43, en 817 MiB.
- **El refresco de un inicio de sesión vuelve a embeber archivos enteros.** Un archivo que cambió embebe todos sus pasajes (`crates/clawg-core/src/memory/project_docs/mod.rs:441-456`), aunque haya cambiado uno solo. Los diez inicios de sesión volvieron a embeber entre 37 y 3.990 pasajes (mediana de unos 2.000). Los de más de 1.700 pasajes tuvieron el proceso en unos 1,16 GiB de 2 a 6,5 min. Después de la carga, es lo que más memoria pide y por más tiempo.
- **Una búsqueda tarda unos 1,4 s** (mediana de la más lenta de cada minuto, en 31 búsquedas sobre unas 55.900 filas), y la peor, 2,6 s, en un minuto con una carga. Es tiempo, no memoria, y no es de este cambio.

### Lo que decide la primera pasada

| Brazo | Puerta | Primera pasada |
|---|---|---|
| B | Más de la mitad de las cargas de un día son de un `wake` que nadie usó | **No se decide.** Sin hooks, ningún registro llegó sin `index_docs`. Va a la segunda pasada |
| C | La carga es el máximo del proceso | **Pasa.** Una carga llega a unos 1,6 GiB. Lo siguiente en tamaño es el refresco más grande, con 1,17 GiB, y el modelo quieto ocupa 0,76 GiB |
| D | Los vectores y las filas preparadas pesan frente al modelo | **No pasa.** Son unos 80 MiB de vectores en un import de 54.000 filas, y hubo un import en seis días |
| E | Después de B, si el modelo sigue cargado la mayor parte del día | **Con el número, sin la decisión.** Repetir el día con 5 minutos de ocio baja un 42 % los minutos con el modelo cargado (de 720 a 421) a cambio de 16 cargas más en 4,28 días (+35 %), de alrededor de 1 s cada una. Con 3 minutos, −59 % y 21 cargas más. Se decide con la segunda pasada: con hooks, cada prompt después de una descarga vuelve a cargar el modelo, y un ocio más corto multiplica esas cargas |
| F | En los archivos que el refresco vuelve a indexar, la mayoría de los pasajes tiene el mismo texto que el índice | **Por medir.** Ver «F» abajo |

**La ventana de C.** Hoy la verificación y la carga leen los mismos bytes: `read_verified` lee `model.onnx` entero a un `Vec`, le calcula el SHA-256 y le pasa ese mismo `Vec` a `commit_from_memory` (`crates/clawg-core/src/memory/model_download.rs:605-613`, `crates/clawg-core/src/memory/embedder.rs:524`). Lo que se verificó es exactamente lo que se cargó. Con C, el SHA-256 se calcula leyendo el archivo de a partes, y después ONNX Runtime lo vuelve a abrir por su ruta. Entre las dos lecturas, alguien que pueda escribir en `<data>/models/` podría cambiar el archivo, y se cargaría un modelo que nadie verificó. Esa es la ventana. Sólo la abre quien ya puede escribir en el directorio de datos, que también puede cambiar `token`, `memory.db` o el binario. En Windows se puede cerrar: el archivo se abre sin compartir escritura ni borrado (`share_mode` de `std::os::windows::fs::OpenOptionsExt`) y queda abierto desde la verificación hasta que la sesión está armada, así que nadie puede escribirlo, renombrarlo ni borrarlo mientras tanto. Falta comprobar, al implementarlo, que ONNX Runtime abre el archivo compartiendo la lectura. En Unix el directorio es `0700`. Que C de verdad baje el pico se confirma con la medición que C ya tiene: diez cargas con `commit_from_memory` y diez con `commit_from_file`. Si ONNX Runtime lee el archivo entero a un buffer propio, C no ahorra nada y se cierra sin cambio.

### F: el refresco de docs reutiliza los pasajes que no cambiaron

**Qué.** Al volver a indexar un archivo, cada pasaje cuyo `indexed_text` ya está en el índice de ese archivo, con el mismo `model_id`, conserva su vector. Sólo se embeben los pasajes nuevos o cambiados. Un pasaje es determinista dado su texto y el modelo, así que el vector guardado vale lo mismo que uno nuevo. **A favor:** el refresco de un inicio de sesión pasa de minutos a lo que realmente cambió, y con eso baja el tiempo que el proceso pasa en 1,16 GiB y la CPU. **En contra:** no baja el pico de una tanda (16 pasajes de hasta 512 tokens siguen siendo la misma matriz de atención), y suma una consulta por archivo para leer los pasajes guardados.

**Puerta.** Se mide sin el modelo y sin tocar un perfil: sobre el historial de git del repositorio, para cada par de versiones consecutivas de cada archivo indexable que cambió, se trocean las dos con el tokenizer del modelo y se cuentan los pasajes de la nueva con el mismo `indexed_text` que alguno de la vieja. F entra si, en los archivos con más pasajes, la mayoría de los pasajes se repite.

**Con qué se coordina.** El orden por largo de las tandas del índice de docs es de 62. Se complementa con F: F embebe menos pasajes, y el orden rellena menos los que quedan.

## La segunda pasada: con hooks

Va después de lo que entre de C y F, con autorización: `clawg setup claude-code --profile prod` instala los hooks, y después de dos o tres días de uso normal se repite la misma lectura. Responde lo que la primera no pudo:

- **Pregunta 2, ahora sí.** Las cargas `for a wake` que vienen de `user-prompt-submit` y las descargas `no call used it`. Los `POST /v1/projects` por minuto de `hist_minute`, menos los inicios de puente, cuentan los prompts.
- **Pregunta 1 con hooks,** que es la base para E.
- Si C entró, el pico de cada carga en uso real.

## La medición controlada: el crecimiento con el modelo cargado

La pregunta (a) de 62 no sale del día. La pregunta 5 lee el piso con el modelo suelto, y un día de uso no junta miles de formas de entrada con el modelo siempre cargado. Se mide aparte, con autorización, en el perfil dev con el modelo real, como la define la tabla de la puerta de 62: un daemon con `CLAWG_EMBEDDER_IDLE_MINUTES=0`, 5.000 consultas de 1 a 512 tokens y una lectura de la memoria privada cada 500, en Linux y en Windows, diciendo qué build se usó. Si crece sin aplanarse, se prueba `with_memory_pattern(false)` (hoy nadie lo llama) y se vuelve a medir.

## La medición del piso

La primera pasada vio que el piso sube de 1,3 a 2,1 MiB por carga en uso real. Se mide aparte, con autorización, en el perfil dev con el modelo real y un build release: un daemon con `CLAWG_EMBEDDER_IDLE_MINUTES=1`, cien ciclos de una búsqueda que carga el modelo, unos guardados y búsquedas, y la descarga, leyendo la memoria privada después de cada descarga. Si se aplana, se cierra con lo medido. Si sigue subiendo, se busca a quién pertenece (el entorno de ONNX Runtime, el allocator) con la misma serie y la carga sin trabajo de 46 como control.

## Los brazos y sus puertas

| Brazo | Qué | Puerta | Decisión previa |
|---|---|---|---|
| B | Despertar el modelo sólo al empezar una sesión | Más de la mitad de las cargas de un día son de un `wake` que nadie usó (pregunta 2) | B1 (`user-prompt-submit` manda `wake: false`), B2 (sólo despierta el primer registro de una raíz) o B3 (ningún registro despierta y la primera búsqueda paga la carga) |
| C | Verificar el SHA-256 leyendo de a partes y cargar con `commit_from_file` en lugar de `commit_from_memory` | La carga es el máximo del proceso (pregunta 3) | Leer el archivo dos veces por carga, con la ventana entre la verificación y la carga cerrada en Windows por el archivo abierto sin compartir escritura |
| D | Embeber y escribir el import por tanda, con import-job (87) | Los vectores y las filas preparadas pesan frente al modelo (pregunta 4) | Aceptar que un import cortado quede parcial y se pueda deshacer con `import --undo` |
| E | Bajar el `CLAWG_EMBEDDER_IDLE_MINUTES` por defecto | Después de B, si el modelo sigue cargado la mayor parte del día (pregunta 1) | Es una decisión de producto: cuánto espera la primera búsqueda después de una pausa |
| F | Reutilizar en el refresco de docs el vector de cada pasaje que no cambió | La mayoría de los pasajes de un archivo reindexado se repite (medido sobre el historial de git) | Entra en este cambio o va aparte |

Cada brazo que entre lleva un test que falla antes del cambio y repite la misma lectura después. Si la medición dice que ninguno vale la pena, el cambio cierra con lo medido, y ese es un resultado válido.

## Interacciones

- **ram-footprint (90, archivado).** Aportó A y el paso 0, especificados en `specs/project-docs`, `specs/metrics` (R6 y R8), `specs/storage` («Cada carga dice quién la pidió y cuánto costó») y `specs/portability` (IT1).
- **embedder-footprint (62).** Sus preguntas de memoria (a) y (b) pasaron acá. El pico de la carga (b) lo respondió la pregunta 3. El crecimiento con `IDLE_MINUTES=0` y entradas de largo variable (a) sigue en la medición controlada, porque el día no lo responde. A 62 le queda la CPU, y su orden de las tandas del índice de docs se complementa con F.
- **pipeline-metrics (73).** Lleva a `metrics.db`, de forma permanente, lo que las líneas de carga dejan en el log. Con la versión en cada minuto, `metrics report --by minute` alcanzaría sin leer `metrics.db` directo.
- **perf-bench (74).** Un escenario de memoria a ritmo fijo, leído con `--by minute`, es la forma repetible de este día.
- **embed-throughput (85, archivado).** No adoptó INT8, así que lo que ahorra C no cambia.
- **import-job (87).** D va con él o después. La primera pasada dice que D, solo, ahorra unos 80 MiB por cada 54.000 filas.
- **tokenizer-footprint (91).** El piso al arrancar sigue en 288 a 302 MiB, que es su tema. La deriva del piso no es del tokenizer.

## Decisiones abiertas

Son del usuario.

- [ ] **B.** Se decide con la segunda pasada. ¿B1, B2 o B3? ¿El umbral es más de la mitad de las cargas de un día?
- [ ] **C.** ¿Se acepta leer el archivo dos veces, con la ventana cerrada en Windows por el archivo abierto sin compartir escritura?
- [ ] **D.** No pasó su puerta. ¿Se cierra con lo medido y queda para 87?
- [ ] **E.** ¿Se decide con la segunda pasada, después de B?
- [ ] **F.** ¿Se mide su puerta? Si pasa, ¿entra en este cambio o va aparte?
- [ ] **El piso.** ¿Se mide aparte, en release y con autorización?

## Riesgos

- **La primera pasada no tuvo hooks.** Lo que dice de B y de E vale para una instalación sin hooks, que no es la que deja `setup`. Por eso los dos esperan la segunda pasada.
- **Unos días pueden no ser representativos.** La primera pasada tuvo 4,28 días, 14 inicios de sesión, 31 búsquedas y 72 guardados, sin un import. Si la segunda es más liviana o más pesada, cambian las proporciones de las preguntas 1 y 2.
- **B hace esperar a la primera búsqueda después de una pausa.** La primera pasada midió esa espera en release: de 0,5 a 2,0 s.
- **C abre una ventana entre la verificación y la carga, y D cambia una garantía del import.** Por eso los dos van con una decisión explícita.
