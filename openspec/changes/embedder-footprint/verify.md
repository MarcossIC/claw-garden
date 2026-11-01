# embedder-footprint — Verificación

Registro de la puerta (tarea 2). Sólo están medidos los pasos 2.1 y 2.2, en Windows. Falta Linux y faltan los pasos 2.3 a 2.6.

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
- **Sin fijar todavía:** el p99 de búsqueda de 2.4 y el tiempo de `split_save` de 2.5.

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

## Lo que esto corrige de la exploración

- **`window_encodings` ya no es el único llamador de `encode_batch`.** También lo llama `TokenSplitter::count_all` (`crates/clawg-core/src/memory/chunker/token_splitter.rs:57`), que está hecho para repartir la tanda entre los núcleos. `set_parallelism(false)` cambiaría los dos.

## Pendiente

- 2.1 y 2.2 en Linux.
- 2.3 a 2.7, con sus umbrales fijados antes de medir.
