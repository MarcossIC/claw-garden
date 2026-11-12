# history-reimport — Verificación del ensayo y del corte

**Resultado del ensayo, en `dev`:** la importación completa, en release, tardó 795,1 s contra los 1.146,9 s de la línea base, un 30,7 % menos. Las filas sin proyecto bajaron de 15.709 a 163. El perfil `dev` volvió a su contenido, pero perdió 4 snapshots automáticos viejos. Prod sólo recibió un `clawg backup`: terminó con las mismas 56.285 filas vivas y las mismas 15.709 sin proyecto.

**Resultado del corte, en prod:** se hizo el 2026-11-08 y tardó 813,1 s, un 29,1 % menos que la línea base. Las filas sin proyecto bajaron de 15.711 a 76. Está en [«El corte en prod»](#el-corte-en-prod), al final; lo que sigue hasta ahí es el ensayo.

## Qué se corrió

- Requisitos: Windows, el binario release instalado, el modelo real, el store real y el export de engram.
- Versiones: clawg 0.73.0, con el código fuente en 0.74.0, y engram 3.5.2 sin actualizar.
- Datos: una copia de prod restaurada en `dev`, con 56.303 filas, y un export nuevo con 3.939 observaciones y 14 prompts.
- Daemon: arrancado a mano. Hasta el final de las mediciones, el log tenía 46 líneas `INFO`, ningún `WARN` ni `ERROR` y ningún refresco de docs.

**El orden del `--dry-run`.** Corrió con el mapa después de `clawg projects create`, no antes del backup. El mapa nombra el proyecto de espera, y el daemon resuelve esa referencia antes de leer el archivo, así que sin el proyecto contesta 404. No se corrió contra prod. La línea «`--project-map` names no note has» no apareció en ninguna de las tres corridas con `--dry-run`, y un cruce fuera del daemon dio lo mismo: cada nombre del mapa tiene notas.

## Tiempo

| | Línea base (0.62.0) | Ensayo (0.73.0) | Diferencia |
|---|---:|---:|---:|
| Total | 1.146,9 s | 795,1 s | −351,8 s (−30,7 %) |
| Filas embebidas | 54.436 | 54.887 | +451 |
| Filas por segundo, total | 47,46 | 69,03 | +45,4 % |
| `embed` | ≈1.114,8 s (97,2 %) | 762,9 s (96,0 %) | −31,6 % |
| Filas por segundo, `embed` | ≈48,8 | 71,9 | +47,3 % |

Los valores con ≈ de la línea base salen de su porcentaje: en esta medición sólo se conservaron el total y el 97,2 %.

El reparto de `serve.log`, en segundos, fue: `read` 0,0, `plan` 0,5, `translate` 11,0, `load` 0,2, `decide` 0,2, `embed` 762,9, `snapshot` 1,2, `write` 18,5, `links` 0,4.

- **Memoria:** el máximo por minuto de `process.private_bytes` durante `embed` fue 1.008,0 MiB, y `process.private_peak_bytes` llegó a 1.268,8 MiB. La memoria privada arrancó en 811,9 MiB y cerró en 1.000,5 MiB.
- **CPU del daemon:** `process.cpu_ratio` promedió 11,09 entre los minutos de `embed`, unos 11 núcleos lógicos ocupados.
- **Otros tiempos:** el deshacer tardó 25,19 s, y 1,59 s con `--dry-run`. Cada `--dry-run` de la importación tardó entre 13 y 16 s.

**Lo que este número no prueba.** Es una sola corrida, sin repetición. La línea base corrió con otra versión y sobre el store de prod, y esta sobre una copia en `dev`. No se midió la carga total de la máquina: el muestreo por proceso quedó bloqueado por el sistema de permisos. Sólo se comprobó que no había compilaciones corriendo, con el daemon de prod y un bridge vivos.

## Ubicación

De 3.953 notas, 1.597 no tienen proyecto: 1.594 observaciones y 3 prompts.

| Destino | Notas |
|---|---:|
| Ubicadas en un proyecto | 834 |
| Proyecto de espera, sin señal | 732 |
| Proyecto de espera, con señales que discrepan | 3 |
| Sin proyecto: preferencias y alcance personal | 28 |

Las 9 notas de proyectos de origen sin carpeta también fueron al proyecto de espera, con `--project-map NOMBRE=project:REF`, y conservan su `origin_project`. La exploración no fijaba qué hacer con ellas: se aplicó su regla de que sólo las preferencias y las notas personales se quedan sin proyecto.

| Filas vivas | Antes | Después |
|---|---:|---:|
| Total del store | 56.285 | 57.007 |
| Sin proyecto | 15.709 | 163 |
| En el proyecto de espera | — | 6.311 |

De las 163, 90 son filas que el deshacer retuvo y que el mapa ubicaría, y 73 son de preferencias y notas personales.

**Contra el mapa:** 1.424 notas están en su destino, 28 exentas siguen sin proyecto, 18 quedaron retenidas por el deshacer y 127 no tienen fila viva propia. Ninguna quedó en otro proyecto. De esas 127, casi todas repiten el texto de otra nota de su proyecto de origen y 2 no tienen texto; antes del deshacer también eran 127.

## Señales

Control: las 2.345 observaciones con proyecto conocido. La regla pide 95 % de precisión o más sobre 30 casos decididos o más. Además, cada señal se midió en dos mitades del control (por hash de `sync_id`) y sobre las notas sin proyecto, contra las señales que no comparten su fuente.

| Señal | Aciertos | Contradice | Precisión | Alcance | Segundo control | Resultado |
|---|---:|---:|---:|---:|---:|---|
| Título del save en transcripciones | 628 | 0 | 100 % | 33 | 20 a 0 | aceptada |
| Primer segmento de la clave de tema, dejando afuera la nota | 172 | 0 | 100 % | 123 | 65 a 3 | aceptada |
| Una ruta o más, únicas de un repositorio | 1.086 | 7 | 99,4 % | 767 | 80 a 3 | aceptada |
| Dos rutas o más | 651 | 0 | 100 % | 537 | — | variante, alcanza menos |
| Cercanía en el tiempo, 180 min y 2 vecinas | 942 | 29 | 97,0 % | 546 | 142 a 133 | **rechazada** |
| Cercanía en el tiempo, 60 min y 2 vecinas | 1.150 | 49 | 95,9 % | 393 | 107 a 117 | **rechazada** |
| Cercanía en el tiempo, 30 min y 3 vecinas | 797 | 24 | 97,1 % | 150 | 45 a 53 | **rechazada** |
| Cercanía en el tiempo, 15 min y 3 vecinas | 566 | 18 | 96,9 % | 70 | 21 a 26 | **rechazada** |
| Nombre de archivo suelto, 2 o más | 649 | 0 | 100 % | 365 | 301 a 1 | medida, no aceptada |

**La cercanía en el tiempo pasa la regla y se rechazó igual.** En el control supera el 95 %, pero sobre las notas sin proyecto coincide con las otras señales cerca de la mitad de las veces, en todas las ventanas. La explicación probable, no medida: la vecina de una nota con proyecto suele ser de su misma ráfaga de trabajo, algo que una nota sin proyecto no tiene, así que el control premia una continuidad que no se transfiere. Como dos señales que discrepan dejan la nota sin ubicar, aceptarla habría sacado notas bien ubicadas. La regla dice que sólo se aceptan las que pasan: pasarla es necesario, no suficiente.

**Dos cifras de la exploración cambian.**

- Las 196 sin contradicciones de la clave de tema se reproducen, pero son circulares: la regla se deriva de esas mismas notas. Dejando afuera la nota evaluada dan 172 sin contradicciones.
- Las transcripciones dan 628 y no 659 porque aquí cuenta sólo la llamada a `mem_save` cuyo título aparece en una sola carpeta. Con eso desaparecen las 4 contradicciones y el alcance baja de 51 a 33.

**El nombre de archivo suelto** no se aceptó porque no es una ruta: agregaría 63 notas más. Su segundo control tampoco es independiente, porque se compara sobre todo con las rutas, que salen del mismo texto.

**Riesgo de lo ubicado.** 684 de las 834 notas dependen sólo de las rutas, cuyo segundo control son 80 de 83 (96,4 %, con un límite inferior de Wilson del 89,9 %). Si esa tasa vale para todas, son unas 25 notas mal ubicadas, y hasta unas 69 en el límite inferior. Es una estimación: nadie revisó esas notas una por una.

## Integridad

- **`clawg doctor`:** 0 errores. El único `warn` es `memory.topic` / `several_live_saves`, con una clave de tema que tiene 2 saves vivos. Ya estaba en el store antes del deshacer.
- **Segunda corrida con `--dry-run`:** `would import 0 of 55182`, con 55.158 ya presentes. Las otras 24 son piezas cuyo texto ya estaba vivo en su destino.

## Las filas que dejó el deshacer

`--undo 1` quitó 54.165 filas y dejó 271 de 42 notas: 44 con veredicto y 227 piezas de un save con una fila que se queda. Coincide con lo anticipado.

- **Ninguna nota quedó partida entre dos proyectos.** La importación encuentra cada nota por su origen y no la toca, así que una nota con filas retenidas queda entera donde estaba.
- **El costo es otro:** 18 de esas notas, con 90 filas, siguen sin proyecto aunque el mapa las ubique.
- **Tiene arreglo:** `clawg projects move --id … --to-project … --dry-run` movería las 90, hacia 5 proyectos, sin rechazar ninguna. No se ejecutó sin `--dry-run`.

## Lo que cambia en el store

Entre las filas importadas, antes del deshacer y después de importar:

| | Antes | Después |
|---|---:|---:|
| `superseded` | 1 | 546 |
| `disputed` | 1 | 5 |
| Tier 1 | 108 | 100 |
| Tier 2 | 38.990 | 42.993 |
| Tier 3 | 15.338 | 12.065 |
| Accesos por encima de uno por fila | 433 | 206 |
| Pares `pending` / `escalated` del store | 390 / 23 | 393 / 27 |

De las 546 filas `superseded`, 543 son revisiones y 3 vienen de un veredicto. Las revisiones salen del mapa: una nota ubicada y una de proyecto conocido comparten clave de tema en un mismo proyecto, y una pasa a versión vieja. El reporte lista 23 de esos pares, y cada uno toca una nota del mapa. **27 notas quedaron sin ninguna fila activa:** 11 ubicadas y 16 de proyecto conocido. Es lo correcto si la nota está bien ubicada. Si no lo está, esconde una nota válida, aunque la fila sigue en el store. Los accesos y el tier se pierden como anticipaba la exploración.

## Lo que quedó mal

- **4 snapshots automáticos viejos de `dev` se perdieron y no se pueden recuperar.** Eran un `pre-restore` del 16 de octubre, dos `pre-migration` del 19 y del 24 de octubre, y un `pre-import` del 24 de octubre. Se conservan 5 automáticos más el último de cada motivo (`AUTO_KEEP`, `crates/clawg-core/src/memory/backup.rs`), y los que escribió el ensayo los empujaron afuera. El procedimiento no lo contemplaba, y se vio recién al comparar el manifiesto de `dev` al final, cuando ya no había copia.
- **`dev` no quedó idéntico byte a byte.** `memory.db` pasó de 549 a 219 MB porque el snapshot está compactado: las 55 tablas tienen las mismas filas, `memory` da el mismo resumen y `quick_check` da `ok`. `metrics.db` guarda los minutos de la corrida, y `backups` conserva los 2 snapshots nuevos con datos de `dev`: el manual y un `pre-restore`.

## Para el corte

- **Prod tiene 6 snapshots automáticos, y el corte va a podar algunos.** Simulando la poda, que reproduce exactamente lo que perdió `dev`: `pre-undo` y `pre-import` empujan afuera los `pre-migration` del 18 y del 19 de octubre. El segundo `projects move` real empuja también el `pre-migration` y el `pre-import` del 24 de octubre, el de la primera importación, y las 90 filas retenidas piden 5 movimientos. Hay que decidir antes si alguno se copia aparte; los `manual` no se podan.
- **Un export nuevo trae notas que este mapa no conoce.** Hay que volver a clasificar con los scripts que quedaron junto al mapa, o esas notas sin proyecto quedarán sin proyecto.
- **El `--dry-run` con el mapa va después de `clawg projects create`.**
- **Después de importar:** colocar las 90 filas retenidas con `clawg projects move --id`.

El mapa, los informes con nombres de proyecto y los scripts quedan en `docs/completed-impl/history-reimport/`, que no se commitea. El export, su copia y los snapshots con datos de prod que escribió el ensayo se borraron, salvo el `clawg backup` de prod.

## El corte en prod

Se hizo el 2026-11-08, con clawg 0.79.1 instalado y engram 3.5.2 sin actualizar. La exploración lo pedía con el piloto aprobado. Se adelantó, y el piloto arrancó sobre el store ya reimportado.

- Requisitos: los del ensayo, y los clientes cerrados durante el deshacer y la importación, para que nada más embeba ni guarde en el medio.
- Máquina: AMD Ryzen AI 7 350, 8 núcleos y 16 lógicos, 31 GiB, enchufada, con el plan de energía equilibrado.

### Qué se hizo, en orden

1. **La foto de antes:** 56.466 filas vivas, 15.711 sin proyecto, 18 proyectos con carpeta y `clawg doctor` sin errores.
2. **Las redes:** los 6 snapshots automáticos de prod se copiaron aparte, 1.113 MB, y se hizo un `clawg backup`.
3. **Un export nuevo de engram:** 4.024 observaciones y 14 prompts.
4. **La clasificación** de las notas sin proyecto, y su mapa aplicado a una copia del export. Se importa la copia, no el export.
5. **`clawg projects create`** del proyecto de espera, sin carpeta.
6. **Dos ensayos con `--dry-run`** sobre el store de ese momento: la importación con el mapa y `clawg import --undo 1`. Ahí se frenó a revisar, y la clasificación se rehízo ([«Ubicación del corte»](#ubicación-del-corte)).
7. **Un muestreo** de la CPU y la RAM de la máquina, que corrió hasta el final de la importación.
8. **`clawg import --undo 1`:** quitó 54.151 filas y dejó 285, en 33,2 s.
9. **La importación con el mapa,** primero con `--dry-run`. Es la importación 2: entraron 55.201 de las 55.510 filas del archivo, con código 0. De las otras 309, 285 son las que dejó el deshacer y 24 son piezas cuyo texto ya estaba vivo en su destino.
10. **5 `clawg projects move --id`,** que colocaron 104 de las filas que dejó el deshacer.
11. **La verificación,** y el borrado de un proyecto sin memorias que estaba de antes del corte.
12. **La limpieza:** se borraron el export, su copia con el mapa y el caché de las transcripciones, y después de la revisión, las capturas y la copia aparte de los snapshots.

### Tiempo del corte

| | Línea base (0.62.0, prod) | Ensayo (0.73.0, `dev`) | Corte (0.79.1, prod) |
|---|---:|---:|---:|
| Total | 1.146,9 s | 795,1 s | 813,1 s |
| Filas embebidas | 54.436 | 54.887 | 55.201 |
| Filas por segundo, total | 47,46 | 69,03 | 67,89 |
| `embed` | ≈1.114,8 s (97,2 %) | 762,9 s (96,0 %) | 765,3 s (94,1 %) |
| Filas por segundo, `embed` | ≈48,8 | 71,9 | 72,1 |

Contra la línea base, el corte tardó 333,8 s menos (−29,1 %) y embebió un 47,7 % más de filas por segundo.

El reparto de `serve.log`, en segundos, fue: `read` 0,0, `plan` 0,7, `translate` 18,0, `load` 0,3, `decide` 0,2, `embed` 765,3, `snapshot` 1,2, `write` 26,6, `links` 0,6. El reloj de la terminal marcó 814,1 s. La tanda más lenta de `embed` tardó 3.360 ms.

- **Memoria:** el máximo por minuto de `process.private_bytes` durante `embed` fue 1.021 MiB. `process.private_peak_bytes` llegó a 1.294 MiB, y lo marcó el minuto de `translate` y de la recarga del modelo, no `embed`.
- **CPU del daemon:** `process.cpu_ratio` estuvo entre 10,7 y 11 durante `embed`, de 16 núcleos lógicos.
- **Carga de la máquina:** promedió 71 % de CPU, sin otro proceso pesado. Es lo que el ensayo no pudo medir.

**Lo que este número agrega.** Es la segunda corrida completa en release, y esta vez sobre el store de prod, como la línea base. Las dos corridas dan lo mismo por fila, 71,9 y 72,1 en `embed`. Sigue siendo una corrida por versión, sin repetición.

### Ubicación del corte

De 4.038 notas, 1.597 no tienen proyecto: 28 son preferencias o de alcance personal, y 1.569 se clasifican.

**La clave de tema dejó de pasar su segundo control.** Con las tres señales aceptadas en el ensayo, sobre las notas sin proyecto la clave de tema coincidió con las otras en 72 de 81 casos y las rutas en 85 de 94, las dos debajo del 95 %. Quedaba aceptada sólo la señal de las transcripciones, que ubicaba 30 notas. Los 9 desacuerdos, mirados uno por uno, eran de la clave de tema:

- 6 son de un segmento que nombra una funcionalidad con código en dos repositorios del mismo producto: la clave lo cuenta para uno, y las notas nombran archivos que sólo existen en el otro.
- 3 son de un segmento genérico, los mismos 3 desacuerdos del ensayo.

Se clasificó sin la clave de tema:

| Señal | Aciertos | Contradice | Precisión | Alcance | Segundo control | Resultado |
|---|---:|---:|---:|---:|---:|---|
| Título del save en transcripciones | 631 | 0 | 100 % | 30 | 16 a 0 | aceptada |
| Una ruta o más, únicas de un repositorio | 1.134 | 9 | 99,2 % | 767 | 16 a 0 | aceptada |
| Primer segmento de la clave de tema, dejando afuera la nota | 182 | 2 | 98,9 % | 138 | 72 a 9 | sacada |

**La salvedad:** sin la clave de tema, el segundo control de cada señal aceptada tiene 16 casos, todos de acuerdo. La regla pide 30. Se aceptó así en la revisión: con la clave de tema se ubicaban 30 notas y 1.539 iban al proyecto de espera.

| Destino | Notas |
|---|---:|
| Ubicadas en un proyecto | 781 |
| Proyecto de espera, sin señal | 788 |
| Sin proyecto: preferencias y alcance personal | 28 |

- De las 781, 751 dependen sólo de las rutas, 16 de las rutas y las transcripciones, y 14 sólo de las transcripciones.
- Las 9 notas de los 3 proyectos de origen sin carpeta fueron también al proyecto de espera.
- El riesgo de las 751 no se volvió a estimar: 16 casos no alcanzan. Con la tasa que midió el ensayo, 80 de 83, serían unas 27 notas mal ubicadas.

| Filas vivas | Antes | Después |
|---|---:|---:|
| Total del store | 56.466 | 57.520 |
| Sin proyecto | 15.711 | 76 |
| En el proyecto de espera | — | 6.881 |

- De las 76 sin proyecto, 70 entraron así con la importación 2, de preferencias y notas personales, y 3 son filas que dejó el deshacer y que tampoco tienen proyecto en la copia con el mapa. Las otras 3 no se desglosaron.
- De las 6.881 del proyecto de espera, 6.826 se importaron ahí y 55 se movieron después.

### Integridad del corte

- **`clawg doctor`:** 0 errores antes y después, con los mismos 3 avisos.
- **Segunda corrida con `--dry-run`:** `se importarían 0 de 55510`, con 55.486 ya presentes, que son las 55.201 importadas y las 285 que dejó el deshacer.
- **Proyectos:** el deshacer quitó las 15 carpetas y los 15 remotos que había registrado la importación 1, y la importación 2 los devolvió con el mismo id de proyecto. Hay 18 proyectos con carpeta antes y después.
- **Secretos:** 2 notas con un secreto se enmascararon enteras.

### Las filas que dejó el deshacer en prod

`--undo 1` dejó 285 filas: 48 con veredicto y 237 piezas de un save con una fila que se queda. En el ensayo eran 271.

- 107 estaban vivas y sin proyecto.
- 3 de esas tampoco tienen proyecto en la copia con el mapa, y se quedan así.
- Las otras 104 se movieron en 5 movimientos, de 21, 53, 2, 16 y 12 filas: 49 a 3 proyectos con carpeta y 55 al proyecto de espera.

### Lo que cambió en el store de prod

Sobre todo el store, no sólo sobre lo importado como en la tabla del ensayo:

| | Antes | Después |
|---|---:|---:|
| `superseded` | 285 | 810 |
| `disputed` | 4 | 11 |
| Tier 1 | 443 | 429 |
| Tier 2 | 38.430 | 44.059 |
| Tier 3 | 17.593 | 13.032 |

- De las 55.201 filas importadas, 524 entraron `superseded`.
- No se contó cuántas notas quedaron sin ninguna fila activa, las 27 del ensayo.
- Los accesos y el tier de lo importado se perdieron, como en el ensayo.

**Lo importado está en dos idiomas.** La importación 2 reportó 19.246 filas en inglés, 27.203 en español y 8.752 sin idioma detectado. En la prueba desde un proyecto, una consulta en español no trajo filas importadas que la misma consulta en inglés sí trajo. Es el cruce de idioma que mide [embedder-bakeoff](../../embedder-bakeoff/exploration.md), no un defecto del corte.

### Lo que pedía «Para el corte»

- **Los snapshots:** se copiaron aparte los 6 automáticos. La copia se borró el mismo día, después de la revisión. Con ella se fueron los automáticos viejos que el corte empujó afuera, entre ellos el `pre-import` de la primera importación.
- **El export nuevo** se clasificó de nuevo, con el cambio de señales de [«Ubicación del corte»](#ubicación-del-corte).
- **El `--dry-run` con el mapa** corrió después de `clawg projects create`.
- **Las filas que dejó el deshacer** se colocaron: 104 y no 90, porque quedaron 285 y no 271.

### Para volver atrás

En los backups de prod quedaron tres snapshots del 2026-11-08, en hora UTC:

| Snapshot | Hora | Qué tiene |
|---|---|---|
| `manual` | 04:09 | El store antes del corte |
| `pre-undo` | 04:50 | El store antes de deshacer la importación 1 |
| `pre-import` | 05:07 | El store después del deshacer y antes de la importación 2 |

- **`clawg restore`** con cualquiera de los tres vuelve a ese punto, y antes guarda el store actual como snapshot. Lo guardado después del corte no está en ellos.
- **`clawg import --undo 2`** deshace la importación y conserva lo que cambió desde entonces.
- **`clawg projects move --undo 1`** a **`5`** deshacen cada movimiento, y **`clawg projects delete --undo 1`** trae de vuelta el proyecto borrado.
- **Cuánto duran:** el `manual` no se poda. El `pre-undo` y el `pre-import` se conservan mientras sean el último de su motivo (`to_prune`, `crates/clawg-core/src/memory/backup.rs`): otra importación u otro deshacer los deja a merced de la poda.
- **Lo que no vuelve:** el export y la copia aparte de los snapshots viejos, que se borraron. engram sigue instalado y con su base, así que un export nuevo se puede sacar.
