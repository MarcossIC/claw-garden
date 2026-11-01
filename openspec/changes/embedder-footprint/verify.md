# embedder-footprint — Verificación

Registro de la puerta (tarea 2). Están medidos los pasos 2.1 y 2.2, en Windows y en Linux, 2.3 en Windows y 2.4, las arenas de malloc, en Linux. Falta 2.5.

## La máquina y el build

| Dato | Valor |
|---|---|
| Sistema | Windows 11 25H2, x86_64 |
| CPU | AMD Ryzen AI 7 350 (Zen 5) |
| Hilos | 8 físicos, 16 lógicos (con SMT) |
| Instrucciones | AVX2, AVX-512F, FMA |
| Energía | con corriente, plan `Balanced`, modo `Best Power Efficiency` |
| Build | debug con la feature `testing`, versión 0.73.0, sobre `62d0db6`; lo que entró después en `develop` no toca el embedder |

## Los umbrales, fijados antes de medir

- **C (2.1):** abre si, con el modelo suelto (`embedder_resident=false`), el daemon normal tiene hilos de más de forma sostenida frente a un control del mismo binario con `TOKENIZERS_PARALLELISM=false`.
- **A (2.2):** abre si la mediana de (CPU de proceso en los 60 s tras una tanda − CPU en 60 s de reposo sin inferencia) es ≥ 0,6 s, el 1 % de un núcleo, con 3 repeticiones.

### Los de 2.3, también antes de medir

Un refresco de docs ya bajó de unos 45 a unos 20 minutos, así que no se devuelve tiempo sólo para que use menos. Cada brazo se compara con la base de su ronda: tiempo con `index_seconds`, CPU con usuario + sistema del proceso durante el indexado, memoria con el pico de privados. El techo de degradación es el menor entre +5 % y +60 s: en un refresco de 20 minutos, los dos coinciden.

| Tiempo frente a la base | Ganancia que necesita |
|---|---|
| Hasta +2 %, que no se distingue del ruido | cualquiera que no sea marginal |
| Más de +2 %, hasta el techo | al menos 22 % de CPU o de memoria, y que no sea marginal |
| Por encima del techo | se descarta |

- **Marginal:** un porcentaje alto sobre un consumo bajo no cuenta. La ganancia tiene que dejar libre al menos 1 núcleo en promedio durante el refresco, o 100 MiB de pico de privados.
- **Ruido:** dentro de una misma rama, sin cambio de régimen, `embed` varió menos del 2 % en docs-index-speed.
- **C:** abrió por 2.1, pero lo medido de su ganancia es el recuento de hilos. Si su brazo no degrada el tiempo y tampoco gana CPU, entrar o no queda como decisión expresa.
- **E (2.5):** abre sólo si las dos codificaciones de más pesan al menos 5 % del tiempo de un guardado de 64 KiB, contando las tres codificaciones y la tanda. Se fijó antes de medir `split_save`, pero después de estimar con los datos del refresco que rondaría el 2 %.

### El de 2.4, una arena contra dos, también antes de medir

Se compara el mismo build, sin la llamada a `keep_one_malloc_arena`, con `MALLOC_ARENA_MAX=1` y con `=2`, mientras el daemon refresca docs y un cliente busca cada 100 ms. Cada corrida con 2 arenas se compara con la de 1 arena de su ronda; los porcentajes son la mediana de los 3 cocientes, y las cifras absolutas, la mediana de las 3 diferencias.

**D6 de embedder-idle-unload, la arena única, se reabre sólo si, con 2 arenas, se cumplen las tres:**

- el p99 de búsqueda baja al menos 22 % y al menos 20 ms;
- el refresco no sube más que el menor entre +5 % y +60 s;
- el pico de `RssAnon` sube menos de 100 MiB.

Si falta una condición, queda una arena. La lectura vale para los hilos de la máquina donde se mide: con más hilos hay más contención por la arena, y la respuesta puede ser otra.

## Cómo se midió

- Cada corrida es un daemon solo, `serve --profile dev --port 0`, con el modelo real, un `CLAWG_TEST_HOME` temporal y una store vacía. Corren una detrás de otra. No se abrió ningún perfil real.
- El binario se copia con otro nombre (`clawg-measure.exe`), para que el contador de `typeperf` no se mezcle con otros procesos `clawg.exe`.
- `typeperf` lee cada segundo `Thread Count`, `% User Time`, `% Privileged Time`, `Private Bytes` e `ID Process`. Son los contadores del sistema, no `Get-Process` como pedía la tarea. El `ID Process` coincide con el `pid` de `/health` durante toda cada corrida.
- La CPU de una ventana es la integral de los porcentajes, recortada a la ventana. No es un acumulado exacto del proceso.
- Cada guardado es un `POST /v1/memories` de un texto corto y único, una pieza. Las 5 corridas dieron 20 de 20 con `201`, y `pending` volvió a 0.
- Los CSV crudos no se conservan en git.

## 2.1 Hilos vivos

`CLAWG_EMBEDDER_IDLE_MINUTES=1`. Cada lectura es la primera muestra después de la marca, 15 s después de la actividad anterior. El modelo se soltó unos 70 s después del último guardado.

| Momento | Normal (hilos) | Control `TOKENIZERS_PARALLELISM=false` (hilos) |
|---|---|---|
| Antes de cargar el modelo | 24 | 24 |
| Modelo cargado, antes de guardar (T0) | 37 | 38 |
| Después de 20 guardados (T1) | 55 | 39 |
| Modelo suelto (T2) | **36** | **20** |
| Modelo suelto, hasta el final de la corrida | 34-35 | 18-19 |

El recuento oscila en 1 o 2 hilos dentro de cada fase.

- **El grupo de `rayon` deja 16 hilos con el modelo suelto**, los mismos que lógicos tiene la máquina. Aparecen con la primera tanda: el control no sube.
- **Cargar el modelo suma 16 hilos**, de 24 a 40 en las dos variantes. Es compatible con 15 del grupo de ONNX Runtime más `embedder-worker`, sin atribuir hilo por hilo. Soltarlo los devuelve.
- **La memoria casi no se mueve.** Los privados con el modelo suelto son 284,5 MiB y 284,4 MiB. Después de los guardados, 736,2 MiB y 734,6 MiB.
- Los 20 guardados tardaron 1,50 s en la corrida normal y 1,44 s en el control. En las tres corridas de 2.2, con el tokenizer paralelo, 1,45, 1,45 y 1,48 s. Con una sola muestra de control y tandas de un texto, esto no dice nada sobre un refresco de docs.

**C abre** por el umbral fijado. Lo medido del costo es sólo el recuento de hilos: ni memoria ni CPU en reposo apreciables (ver 2.2).

## 2.2 CPU en reposo

`CLAWG_EMBEDDER_IDLE_MINUTES=0`, tokenizer paralelo. Cada repetición es un daemon nuevo. El reposo empieza 30 s después de `embedder: ready`, sin ninguna inferencia hecha todavía. La ventana «tras la tanda» empieza al volver el último guardado con `pending` en 0.

| Repetición | Reposo, 60 s | Tras la tanda, 60 s | Diferencia | Los 60 s siguientes | Primeros 10 s tras la tanda |
|---|---|---|---|---|---|
| 1 | 0,213 s | 0,140 s | −0,073 s | 0,000 s | 0,140 s |
| 2 | 0,316 s | 0,144 s | −0,172 s | 0,031 s | 0,128 s |
| 3 | 0,161 s | 0,150 s | −0,011 s | 0,015 s | 0,150 s |

**Mediana de la diferencia: −0,073 s**, contra un umbral de +0,6 s. **A no abre por 2.2.** Lo más alto tras una tanda es 0,150 s en 60 s, un 0,25 % de un núcleo, y casi todo cae en los primeros 10 s. Los 16 hilos de `rayon` estaban vivos durante estas ventanas y no se ven en la CPU.

Tres límites de esta lectura:

- Mide lo que queda después de una tanda, no la espera activa **durante** un refresco, entre una tanda y la siguiente. Eso sólo lo puede responder 2.3.
- Las tandas de acá son de un texto corto, no de 16 pasajes largos.
- Es un build debug. La inferencia es la misma biblioteca nativa, pero el código Rust del daemon gasta más que en release. Ese gasto cae en las dos ventanas y no afecta a un resultado que queda por debajo del umbral.

## 2.1 y 2.2 en Linux

### La máquina y el build

| Dato | Valor |
|---|---|
| Sistema | Ubuntu 24.04.5 LTS, núcleo Linux 6.18, x86_64, máquina virtual bajo KVM |
| CPU | Intel Xeon @ 2.10 GHz (familia 6, modelo 207), 1 zócalo |
| Hilos | 4 físicos, 4 lógicos (sin SMT) |
| Instrucciones | AVX2, AVX-512F, FMA |
| Memoria | 15 GiB, sin swap |
| Build | debug con la feature `testing` (`cargo test -p clawg-cli --no-run`), versión 0.73.0, sobre `6cb64fc` |

Esta máquina no cumple los 8 hilos lógicos con SMT de la guía. Ese requisito lo cubre la de Windows.

### Cómo se midió, y en qué difiere de Windows

- El esquema es el mismo: cada corrida es un daemon solo, `serve --profile dev --port 0`, con el modelo real, un `CLAWG_TEST_HOME` temporal y una store vacía, una detrás de otra. Corren con `RUST_LOG=info` y sin ninguna otra variable de clawg, del tokenizer, de `rayon` ni de malloc, salvo las que pide cada paso. `status` dio el directorio de datos dentro del home temporal; no se abrió ningún perfil real.
- El modelo se bajó una vez, en una corrida aparte que no se mide, y se movió de un home al siguiente. Todas las corridas medidas arrancan con el modelo en disco.
- **Hilos:** las entradas de `/proc/<pid>/task`, leídas en cada marca. El `pid` es el de `/health` y coincide con el proceso de `serve`, así que el binario no se renombra.
- **CPU:** usuario + sistema del proceso, los campos 14 y 15 de `/proc/<pid>/stat` divididos por `CLK_TCK` (100). Es el acumulado exacto del proceso, con los hilos que ya terminaron, en pasos de 0,01 s. En Windows era la integral de porcentajes leídos cada segundo.
- **Memoria:** `RssAnon` de `/proc/<pid>/status`, las páginas anónimas residentes. No es la misma medida que los privados de Windows, que cuentan la memoria comprometida esté residente o no, así que las cifras no se comparan entre plataformas.
- Además de las marcas, un muestreo de `/proc` cada segundo da los rangos dentro de cada fase. Leer `/proc` no gasta CPU del daemon.
- Cada guardado es un `POST /v1/memories` de unas 30 palabras, de 205 a 253 caracteres: una cabecera única y 26 palabras al azar de un vocabulario de unas 90. Las 5 tandas dieron 20 de 20 con `201`, cada guardado quedó en una sola pieza y `pending` volvió a 0. Como los textos comparten vocabulario, el detector de contradicciones registró conflictos entre ellos.
- Los registros crudos no se conservan en git.

### 2.1 Hilos vivos

`CLAWG_EMBEDDER_IDLE_MINUTES=1`. La lectura 1 llega con `embedder` en `loading`, a 0 s de uptime, antes de que se lea el tokenizer. El modelo se soltó 75 s después de que `pending` volviera a 0 en las dos corridas (74,9 y 75,0 s, mirando `/health` cada segundo).

| Momento | Normal: hilos | Normal: `RssAnon` | Control `TOKENIZERS_PARALLELISM=false`: hilos | Control: `RssAnon` |
|---|---|---|---|---|
| 1. Al arrancar, modelo cargando | 9 | 22,0 MiB | 9 | 21,9 MiB |
| 2. 15 s después de `ready` | 10 | 804,6 MiB | 10 | 804,6 MiB |
| 3. 15 s después de 20 guardados | 14 | 805,4 MiB | 10 | 805,3 MiB |
| 4. 15 s después de soltar el modelo | **10** | 330,5 MiB | **6** | 330,5 MiB |
| 5. 60 s más tarde | 10 | 330,5 MiB | 6 | 330,6 MiB |

En el muestreo de cada segundo, el recuento oscila en 1 hilo dentro de cada fase: con el modelo suelto, 10-11 en la normal y 6-7 en el control; con el modelo cargado después de la tanda, 14-15 y 10-11.

- **El grupo de `rayon` deja 4 hilos con el modelo suelto**, los mismos que lógicos tiene la máquina, como los 16 de Windows. Aparecen con la primera tanda: el control no sube.
- **Soltar el modelo quita 4 hilos en las dos variantes**, de 14 a 10 y de 10 a 6. Es compatible con `embedder-worker` más 3 del grupo de ONNX Runtime (la sesión pide 4 hilos de intra-op y uno es el que llama), sin atribuir hilo por hilo. Entre las lecturas 1 y 2 el recuento sólo sube de 9 a 10 porque durante la carga llega a 13, y los hilos de arranque se van en los 10 s siguientes.
- **El tokenizer paralelo no mueve la memoria.** Normal y control difieren en 0,1 MiB o menos en cada lectura. Soltar el modelo baja `RssAnon` unos 475 MiB en las dos.
- Los 20 guardados tardaron 0,749 s en la corrida normal y 0,724 s en el control. En las tres repeticiones de 2.2, 0,664, 0,759 y 0,757 s.

**C abre** también en Linux por el umbral fijado: con el modelo suelto, la normal tiene 4 hilos más que el control de forma sostenida, 10 contra 6 en las dos lecturas y 10-11 contra 6-7 en el muestreo. Coincide con Windows.

### 2.2 CPU en reposo

`CLAWG_EMBEDDER_IDLE_MINUTES=0`, tokenizer sin tocar. Cada repetición es un daemon nuevo en un home nuevo. El reposo empieza 30 s después de `embedder: ready`, sin ninguna inferencia hecha todavía. La ventana «tras la tanda» empieza al ver `pending` en 0 después del último guardado.

| Repetición | Reposo, 60 s | Tras la tanda, 60 s | Diferencia | Los 60 s siguientes | Primeros 10 s tras la tanda |
|---|---|---|---|---|---|
| 1 | 0,01 s | 0,02 s | +0,01 s | 0,01 s | 0,01 s |
| 2 | 0,01 s | 0,01 s | 0,00 s | 0,02 s | 0,00 s |
| 3 | 0,01 s | 0,01 s | 0,00 s | 0,00 s | 0,00 s |

**Mediana de la diferencia: 0,00 s**, contra un umbral de +0,6 s. **A no abre por 2.2** en Linux, y coincide con Windows. Cada ventana está entre 0 y 2 pasos de 0,01 s, así que la diferencia queda por debajo de lo que esta lectura resuelve. Durante las ventanas había 14 hilos vivos, los 4 de `rayon` entre ellos.

Límites de esta lectura:

- La ventana de reposo no tiene ninguna petición al daemon. La de tras la tanda y la siguiente tienen cada una una petición a `/health` en su primer instante, la de la lectura de la marca. Si pesa, sube la ventana tras la tanda.
- El reposo en Linux es 0,01 s en 60 s; en Windows fue de 0,161 a 0,316 s. Los métodos no son los mismos y la diferencia no se atribuye.
- Los tres límites de la lectura de Windows valen igual acá: no mide la espera activa durante un refresco, las tandas no son de pasajes largos y es un build debug.

### Lo raro

- **En Linux un hilo nuevo hereda el nombre del que lo crea, así que contar por nombre no separa los grupos.** Los 4 de `rayon` se llaman `embedder-worker`. Los 3 que se van con el modelo junto con `embedder-worker` se llaman `tokio-rt-worker`, porque la carga corre en un hilo de bloqueo de tokio. En la repetición 1 de 2.2, esos 3 tenían 0 s de CPU antes de la tanda y de 0,11 a 0,16 s cada uno después; `embedder-worker`, 0,29 s, y los 4 de `rayon`, 0 s cada uno.
- **Con el modelo suelto, `RssAnon` queda en 330,5 MiB** en las dos variantes. La lectura 1 (22 MiB) es anterior a leer el tokenizer, así que no sirve de base para saber cuánto queda del modelo.

## 2.3 CPU y pared de un refresco de docs

### Ronda 1 en Windows: brazos seguidos, sin pausa

Build release con la feature `testing`, versión 0.73.0, sobre `6cb64fc`, con dos parches de medición que no se commitean: hilos y espera activa por variable de entorno en `session_builder`, y el reloj, la CPU y el pico de privados alrededor del indexado en `rag-eval`. Cada corrida es `rag-eval --real --docs --no-cache` sobre el mismo árbol: 821 documentos, 19.696 pasajes. Los brazos corrieron en el orden de la tabla.

| Brazo | Hilos del proceso | Tiempo | Frente a la base | `split` | `count` | CPU total | Núcleos en promedio | Pico de privados |
|---|---|---|---|---|---|---|---|---|
| Base, 16 hilos | 36 | 1024,2 s | – | 10,08 s | 2,04 s | 10.955 s | 10,7 | 1483,6 MiB |
| 8 hilos | 27 | 1436,3 s | +40,2 % | 10,88 s (+7,9 %) | 1,92 s (−5,9 %) | 9.515 s (−13,1 %) | 6,6 | 1482,7 MiB |
| 4 hilos | 23 | 1906,5 s | +86,1 % | 10,84 s (+7,5 %) | 2,08 s (+2,0 %) | 6.788 s (−38,0 %) | 3,6 | 1483,3 MiB |
| Sin espera activa | 35 | 1343,7 s | +31,2 % | 11,83 s (+17,4 %) | 2,68 s (+31,4 %) | 14.584 s (+33,1 %) | 10,9 | 1485,3 MiB |

- **`split` y `count` sirven de termómetro.** Son trabajo del tokenizer, que estos tres brazos no tocan: si suben, la máquina entera iba más lenta. `split` cae a lo largo de la corrida y `count` se mide al terminar de indexar; dura unos 2 s, así que es una lectura ruidosa.
- **La ronda no es pareja.** La base corrió primera y en frío, y no se registró la temperatura durante los brazos. Tras 1 h 40 min de carga, el sensor térmico marcaba 373 K, unos 100 °C. Con la máquina casi en reposo bajó a 340 K en unos 15 minutos y quedó estable en 339 K. `% Passive Limit` siguió en 100 todo el tiempo, así que ese contador no descarta un freno por calor.
- **El brazo de tokenizer en serie no tiene datos.** Se cortó a los 4 minutos por la temperatura.
- **La memoria no se mueve** con ningún brazo: 1483 a 1485 MiB.
- **La búsqueda no cambia.** Las 40 preguntas dieron el mismo rango y el mismo primer pasaje en los cuatro brazos. Es buena señal, pero no reemplaza la comprobación de bits de 2.6.

**B se descarta en Windows, sin repetir.** `split` no quedó parejo, como pedía la regla para no repetir un brazo, pero `count` no sube. Aun descontando `split`, 8 hilos quedan en +30 % y 4 hilos en +73 %, contra un techo de +5 %. Tampoco hay un punto intermedio: 8 hilos ahorran 13 % de CPU a cambio de +40 % de tiempo, así que llegar a 22 % pide bajar de 8 hilos, y eso cuesta todavía más tiempo. En la máquina Linux de 4 hilos, físicos y `min(lógicos, 4)` coinciden con la base, así que no hay brazo que medir.

En esta ronda, A quedó sin decidir: descontando `count` y `split`, el tiempo daba entre 0 y +12 % y la CPU entre +1 % y +13 %, en una sola corrida y en caliente. Se resolvió con la prueba corta.

### Prueba corta en Windows: en frío, con temperatura y frecuencia registradas

El mismo build y el mismo `rag-eval --real --docs --no-cache`, sobre una muestra del árbol: cada séptimo archivo de cada tipo, más `README.md`. Son 119 documentos y 3223 pasajes, con una pregunta de relleno, así que las métricas de búsqueda de estas corridas no significan nada. Tres rondas con el orden rotado, y cada brazo se compara con la base de su ronda. `typeperf` registra cada 5 s el sensor térmico y `% Processor Performance`; la frecuencia es ese porcentaje sobre los 2000 MHz nominales, una aproximación.

| Corrida | Orden en la ronda | Tiempo | Frente a la base | CPU total | Frente a la base | Pico de privados | Hilos | Frecuencia, primeros 30 s |
|---|---|---|---|---|---|---|---|---|
| Base 1 | 1.º | 171,81 s | – | 1642,8 s | – | 1640,0 MiB | 38 | 3949 MHz |
| Sin espera 1 | 2.º | 179,86 s | +4,7 % | 1673,5 s | +1,9 % | 1639,5 MiB | 38 | 3930 MHz |
| Serie 1, repetida | al final | 173,51 s | +1,0 % | 1637,8 s | −0,3 % | 1618,3 MiB | 22 | 4020 MHz |
| Serie 2 | 1.º | 176,03 s | +2,3 % | 1674,7 s | −2,2 % | 1621,8 MiB | 22 | 3933 MHz |
| Base 2 | 2.º | 172,13 s | – | 1711,6 s | – | 1641,6 MiB | 37 | 4002 MHz |
| Sin espera 2 | 3.º | 173,82 s | +1,0 % | 1676,0 s | −2,1 % | 1641,0 MiB | 37 | 4003 MHz |
| Sin espera 3 | 1.º | 176,08 s | +3,0 % | 1674,8 s | −0,2 % | 1639,2 MiB | 38 | 4002 MHz |
| Serie 3 | 2.º | 172,85 s | +1,1 % | 1661,2 s | −1,0 % | 1621,4 MiB | 22 | 4003 MHz |
| Base 3 | 3.º | 170,93 s | – | 1678,3 s | – | 1639,2 MiB | 38 | 4010 MHz |

| Brazo | Mediana de tiempo | Mediana de CPU | Pico de privados, mediana |
|---|---|---|---|
| Sin espera activa | +3,0 % | −0,2 % | −0,5 MiB |
| Tokenizer en serie | +1,1 % | −1,0 % | −19,8 MiB |

- **Un sensor térmico solo no prueba un arranque en frío.** El sensor quedó fijo en 337 K durante 5 minutos, de 12:59 a 13:04, con la máquina a plena carga, y saltó de golpe a 361 K. El guion leyó 337 K al terminar «sin espera 1» y lanzó «serie 1» 2 segundos después. Esa corrida arrancó a 3483 MHz, dio 192,56 s y no cuenta. Se repitió al final tras 10 minutos fijos de pausa y se compara con la base de su ronda. Las otras 9 corridas arrancaron entre 3930 y 4020 MHz.
- **La frecuencia baja a la mitad de cada corrida.** Arranca cerca de 4,0 GHz y termina cerca de 3,45 GHz, con el sensor entre 361 K y 372 K. Una corrida de 3 minutos pasa por las dos zonas; un refresco completo pasa casi todo su tiempo en la segunda.
- **La base es estable.** 171,81, 172,13 y 170,93 s: 0,7 % de punta a punta.
- **El recuento de tokens es el mismo en todas las corridas,** también en serie: 1.012.475 reales, 1.501.796 con relleno.
- **`split` no cambia en serie** (1,47 a 1,77 s, contra 1,45 a 1,95 s de la base). **`count` sí:** de 0,28 a 0,30 s pasa a 0,90 a 1,64 s, porque es el trabajo que `encode_batch` reparte entre los núcleos.

**A se descarta.** Con +3,0 % de tiempo necesitaba al menos 22 % de CPU o de memoria y gana 0,2 % de CPU. El +31 % de la ronda 1 era casi todo calor: en frío queda en +3 %. Tampoco abrió por 2.2, ni en Windows ni en Linux. Esta prueba corta no se corrió en Linux.

**C no pasa por la regla general de 2.3 y queda en el caso previsto para él.** El tiempo sube 1,1 %, dentro de lo que no se distingue del ruido, aunque sube en las tres rondas. La ganancia es marginal: 1 % de CPU, unos 0,1 núcleos, y 20 MiB de pico, contra 1 núcleo o 100 MiB. Lo único que cambia de forma clara son los hilos, 16 menos. Entrar o no era decisión expresa.

**C no entra.** Una ganancia tan baja no justifica cambiar el tokenizer, y menos con el tiempo subiendo en las tres rondas. Sin ningún brazo en pie, no hacen falta el refresco completo de confirmación ni medir 2.3 en Linux.

## 2.4 y 2.5 en Linux

### La máquina y el build

| Dato | Valor |
|---|---|
| Sistema | Ubuntu 24.04.5 LTS, núcleo Linux 6.18, glibc 2.39, x86_64, máquina virtual bajo KVM |
| CPU | Intel Xeon @ 2.10 GHz (familia 6, modelo 207), 1 zócalo. La máquina virtual no expone sensor térmico ni control de frecuencia |
| Hilos | 4 físicos, 4 lógicos (sin SMT) |
| Instrucciones | AVX2, AVX-512F, FMA |
| Memoria | 15 GiB, sin swap |
| Build | release con la feature `testing`: `cargo build --release --locked -p clawg-cli --features clawg-core/testing`, rustc 1.99.1, versión 0.73.0, sobre `453fc0a`, cuyos fuentes de `crates/` son los de `6cb64fc` |

La huella es la de 2.1 y 2.2 en Linux. Cada paso tiene su build, con un parche de medición que no se commitea:

- **El de 2.4** quita de `crates/clawg-cli/src/main.rs` la llamada a `keep_one_malloc_arena` y su `use`. Nada más cambia, así que las arenas las fija sólo `MALLOC_ARENA_MAX`. Las dos variantes usan el mismo binario.

Los builds se compilaron antes de la primera corrida medida. Mientras se midió no se compiló ni se corrió ningún test, y las corridas fueron de a una.

### 2.4 Una arena contra dos

#### Cómo se midió

- **La muestra.** De la prueba corta de Windows quedaron la regla (cada séptimo archivo de cada tipo, más `README.md`) y el tamaño (119 documentos, 3223 pasajes), pero no la lista de archivos. El árbol es el de `6cb64fc` sin el documento que `rag-eval` deja afuera por citar una pregunta del golden (`openspec/changes/archive/2026-10-15-project-indexing/verify.md`): 821 documentos. Indexado entero en Linux, sin ese documento da 19.694 pasajes, contra 19.696 en Windows, así que la plataforma casi no mueve el recuento. Ninguna de las reglas probadas da 3223 justo: por orden de bytes de la ruta, la muestra da 2598 pasajes desde el índice 1 y 3743 desde el 2. Se usa la que más se acerca, con **119 documentos y 3231 pasajes**. Toma los archivos en el orden en que los lista `Get-ChildItem -Recurse` (primero los de cada carpeta, después sus subcarpetas, sin distinguir mayúsculas), los agrupa por tipo y se queda con uno de cada siete desde el índice 2, más `README.md`. Son 88 Markdown, `README.md` incluido, 30 de Rust y 1 manifiesto.
- **El store.** 10.000 filas de la siembra de `perf_bench` (`seed_export`, semilla 74), con el generador portado a un guion: 4 agentes, 3 proyectos y filas sin proyecto; 8.515 vivas, 1.032 reemplazadas y 453 borradas. Se importó una sola vez, con `POST /v1/import` y el modelo real, en un home aparte. Cada corrida arranca sobre una copia de ese directorio de datos, con el índice de docs vacío, así que cada refresco indexa la muestra entera.
- **Cada corrida** es un daemon solo, `serve --profile dev --port 0`, en un `CLAWG_TEST_HOME` temporal que también es su `HOME`. Corre con `RUST_LOG=info` y el `MALLOC_ARENA_MAX` de su variante, sin ninguna otra variable de clawg, del tokenizer, de `rayon` ni de malloc. El modelo real se bajó una vez con `curl`, los cinco archivos verificaron contra sus `sha256` y cada home lo recibe enlazado. El log de cada daemon nombra su directorio de datos dentro del home temporal; no se abrió ningún perfil real. La ruta del home pasa de los 108 bytes que admite un socket Unix, así que el daemon sólo escucha por TCP, que es por donde llega el cliente.
- **Una corrida, en orden:** el modelo listo, 30 s de reposo, 20 búsquedas seguidas de calentamiento que no cuentan, 5 s de pausa y `POST /v1/projects` con `index_docs: true` sobre la muestra, que arranca el refresco. Desde ese instante el cliente lanza un `POST /v1/search` cada 100 ms, cada uno en su propio hilo y sin esperar a los anteriores. La latencia cuenta desde el tic, no desde el envío (lazo abierto, como en `perf_bench`); el envío más tardío salió 23 ms después de su tic. Las consultas son las de `perf_bench` para la misma semilla: de 2 a 4 palabras frecuentes y, en un tercio, un identificador raro. El agente rota entre los cuatro. El cliente deja de lanzar cuando el log dice `docs refresh of …: done in N s` y espera a las búsquedas que siguen en vuelo.
- **El refresco** es el tiempo de esa línea. **`RssAnon`** sale de `/proc/<pid>/status` cada 100 ms, y el pico es el máximo dentro del refresco. **Los percentiles** se toman por rango más cercano, como en `perf-bench`.
- **3 rondas con el orden rotado** (1-2, 2-1, 1-2) y 30 s entre corridas. Antes de la primera hubo dos pilotos con 1 arena, sobre las dos muestras por orden de bytes, y el indexado del árbol entero; ninguno cuenta.
- Los registros crudos no se conservan en git.

#### Resultados

| Corrida | Orden en la ronda | Búsquedas | p50 | p99 | Refresco | Pico de `RssAnon` | CPU del refresco |
|---|---|---|---|---|---|---|---|
| 1 arena, ronda 1 | 1.º | 3749 | 1811 ms | 4215 ms | 374,9 s | 1313,9 MiB | 1194 s |
| 2 arenas, ronda 1 | 2.º | 3651 | 1773 ms | 4310 ms | 365,0 s | 1333,7 MiB | 1179 s |
| 2 arenas, ronda 2 | 1.º | 3231 | 1487 ms | 3594 ms | 323,0 s | 1301,2 MiB | 1056 s |
| 1 arena, ronda 2 | 2.º | 3519 | 1651 ms | 4424 ms | 351,8 s | 1288,2 MiB | 1104 s |
| 1 arena, ronda 3 | 1.º | 3331 | 1538 ms | 3720 ms | 333,0 s | 1293,3 MiB | 1078 s |
| 2 arenas, ronda 3 | 2.º | 3480 | 1627 ms | 3838 ms | 347,9 s | 1303,5 MiB | 1125 s |

Todas las búsquedas respondieron 200, y en cada corrida se indexaron los 119 documentos y los 3231 pasajes. Los p50 y p99 de cada corrida se calculan sobre todas sus búsquedas.

| 2 arenas frente a 1, en la misma ronda | p99 | Refresco | Pico de `RssAnon` |
|---|---|---|---|
| Ronda 1 | +2,3 % (+95 ms) | −2,6 % (−9,9 s) | +19,8 MiB |
| Ronda 2 | −18,8 % (−829 ms) | −8,2 % (−28,8 s) | +13,0 MiB |
| Ronda 3 | +3,2 % (+118 ms) | +4,5 % (+14,9 s) | +10,2 MiB |
| **Mediana** | **+2,3 % (+95 ms)** | **−2,6 % (−9,9 s)** | **+13,0 MiB** |

Contra el umbral:

- **p99:** tenía que bajar al menos 22 % y 20 ms. Sube 2,3 % y 95 ms. **No se cumple.**
- **Refresco:** en un refresco de unos 350 s el techo que manda es +5 %, menor que +60 s. Baja 2,6 % y 9,9 s. Se cumple.
- **Pico de `RssAnon`:** sube 13,0 MiB, por debajo de 100 MiB. Se cumple.

**Queda una arena: D6 de embedder-idle-unload no se reabre.** La lectura vale para los 4 hilos de esta máquina. En una con más hilos que pidan memoria a la vez, la arena única puede pesar más, y eso no está medido.

Para leer los números:

- **Durante un refresco, una búsqueda espera a la tanda de docs que está corriendo.** Fuera del refresco, las búsquedas de calentamiento tardan de 33 a 56 ms. Durante el refresco, la mediana va de 1,5 a 1,8 s. El embedder tiene un solo hilo que atiende una cola por orden de llegada, y el refresco manda cada tanda de 16 pasajes como un pedido aparte (`E5Embedder::embed_batch`). La consulta de una búsqueda entra, entonces, detrás de la tanda en curso y de las búsquedas que llegaron antes. El p99 lo pone la inferencia de una tanda de pasajes largos, que la arena de malloc no toca.
- **El ruido entre corridas es mayor que la diferencia entre variantes.** Con el mismo binario y la misma muestra, el refresco fue de 323 a 375 s, un 16 % de punta a punta. La ronda 2 es la única donde 2 arenas ganan claro; en las otras dos pierden por poco.
- Durante el refresco el daemon tiene como máximo 15 hilos en todas las corridas y usa en promedio de 3,1 a 3,3 de los 4 núcleos. Antes del refresco, `RssAnon` ronda los 800 MiB, y 837,8 MiB en la última corrida.

## Lo que esto corrige de la exploración

- **`window_encodings` ya no es el único llamador de `encode_batch`.** También lo llama `TokenSplitter::count_all` (`crates/clawg-core/src/memory/chunker/token_splitter.rs:57`), que está hecho para repartir la tanda entre los núcleos. `set_parallelism(false)` cambiaría los dos.

## Pendiente

- 2.5: `split_save`.
- 2.6 no se corre: no entra ningún cambio de hilos, de espera activa ni de tokenizer, así que no hay combinación que comprobar.
- 2.7: cerrar el registro.
