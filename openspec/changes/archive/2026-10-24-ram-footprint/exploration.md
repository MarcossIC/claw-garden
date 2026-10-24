# ram-footprint — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M (el paso 0 y A son S; B, C y D van con puerta) | — (se coordina con 62, 73, 85, 87 y 91; ver «Interacciones») | exploración (borrador) |

**Tesis:** el daemon no pierde memoria, pero en uso real retiene mucho más de lo que necesita: con una sola sesión de Claude Code, el modelo de embeddings pasa cargado casi todo el tiempo, y el proceso llega a picos que nadie sabe a qué atribuir. Dos causas están verificadas en el código: cada prompt **despierta el modelo** y **lanza un refresco completo del índice de docs**, haga o no el agente una búsqueda. El pico máximo no tiene dueño porque las métricas que el daemon ya guarda por minuto sólo se leen por hora, día o versión. El cambio es de **medición primero**: un paso 0 que sólo lee y registra, un arreglo chico que ya estaba escrito en el backlog, y tres brazos que entran sólo si la medición muestra que importan.

## Lo medido en prod

`clawg metrics report` sobre el perfil prod el 2026-10-24: release 0.62.0, Windows, una sola sesión de Claude Code y una sola ejecución del daemon de unos 97 minutos. `process.private_bytes` es `PrivateUsage` (`crates/clawg-cli/src/daemon/process.rs:104`), muestreado una vez por minuto. Los valores van en MiB (bytes / 2²⁰).

| Hora local | Muestras | Privada media | Mínima | Máxima | Minutos con el modelo cargado (`embedder.resident`) |
|---|---|---|---|---|---|
| 13 | 57 | 799,7 MiB | 283,9 MiB | **1.278,4 MiB** | 81 % (46 de 57) |
| 14 | 40 | 684,1 MiB | 341,0 MiB | 856,7 MiB | 73 % (30 de 41) |

Lectura:

- **El piso está donde lo dejó 46.** Con el modelo suelto, la mínima de las 13 (283,9 MiB) es la que embedder-idle-unload midió en debug después de D5: 284 a 290 en diez recargas (`changes/archive/2026-10-17-embedder-idle-unload/verify.md:106-119`). En release el piso es el mismo, porque lo pone el tokenizer (~255 MB, el tema de 91) y no el código.
- **El modelo cargado es lo normal en una sesión activa:** 73 a 81 % de los minutos. Por eso la media sigue al modelo y no al piso.
- **El máximo no es el estado con el modelo cargado.** Con el modelo cargado el daemon ocupa unos 740 MB (46, `verify.md:106-117`), y la máxima de las 14 (856,7 MiB) entra en eso más una tanda en curso. La máxima de las 13, 1.278,4 MiB, queda medio giga por encima y no hay con qué saber qué la causó: una carga del modelo, un refresco de docs o algo más. Como la muestra es una por minuto, el pico real pudo ser todavía mayor.
- **La mínima de las 14 subió 57 MiB.** Con dos horas agregadas no se distingue un piso que deriva de una muestra tomada a mitad de una carga o de una descarga. Hace falta una serie por minuto.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El daemon pierde memoria | **No demostrado.** El piso con el modelo suelto coincide con el que midió 46 y en sus diez recargas no creció (`verify.md:119-120`). Lo que se ve es el modelo cargado la mayor parte del tiempo, más picos transitorios. La deriva de la mínima de las 14 queda abierta hasta tener una serie por minuto. |
| El modelo se suelta cuando nadie lo usa | **Cierta a medias.** Se suelta a los `CLAWG_EMBEDDER_IDLE_MINUTES` sin uso (`crates/clawg-core/src/memory/idle_embedder.rs:61-63`), pero el siguiente prompt lo vuelve a cargar: los hooks registran la raíz en cada `session-start` y `user-prompt-submit` (`crates/clawg-cli/src/hook.rs:242-264`), y `POST /v1/projects` llama a `embedder.wake()` (`crates/clawg-cli/src/daemon/api.rs:2063-2067`), que con el modelo suelto lo pide y lo carga (`idle_embedder.rs:287-296`). El comentario de esas líneas dice que el `wake` era para el puente que arranca su sesión. |
| Un registro sin `index_docs` no refresca el índice de docs | **Falsa.** `register_project` devuelve la marca guardada o la pedida (`crates/clawg-core/src/memory/project.rs:279`), y el handler lanza `spawn_doc_refresh` siempre que esa marca está (`api.rs:2068-2069`). Ya estaba en el backlog sin el dato de memoria. |
| Ese refresco respeta `REFRESH_INTERVAL` | **Falsa.** `spawn_doc_refresh` toma el refresco con `DocIndex::begin` (`api.rs:2087`, `crates/clawg-core/src/memory/project_docs/mod.rs:209-218`), que sólo lo saltea si ya hay uno en curso. `begin_if_due` (`:223-232`), que respeta los 60 s de `REFRESH_INTERVAL` (`:32`), lo usa sólo `search_project`. Así que cada prompt recorre y hashea cada archivo indexado, y embebe los pasajes de los que cambiaron: en tandas de 16 de hasta 512 tokens, que es el pico de inferencia. Cada tanda además refresca el reloj de ocio del modelo. |
| El puente registra en cada llamada | **Falsa.** Registra cada raíz una vez por proceso (`ids_for`, `crates/clawg-cli/src/mcp/server.rs:874-896`). El costo por prompt viene sólo de los hooks. |
| La carga del modelo tiene los 470 MB del ONNX en memoria mientras arma la sesión | **Cierta.** `load_cached_model` lee el archivo entero a un `Vec<u8>` (`crates/clawg-core/src/memory/model_download.rs:301`, `:605-606`), y la carga lo tiene vivo durante `commit_from_memory` (`crates/clawg-core/src/memory/bootstrap.rs:469`, `crates/clawg-core/src/memory/embedder.rs:524`). Se libera cuando la carga vuelve. Cuánto suma ONNX Runtime encima mientras arma la sesión no está medido: es la pregunta (b) de 62, que pasa a este cambio. |
| `commit_from_memory_directly` evita la copia | **Falsa para este modelo.** `ort` sólo activa `session.use_ort_model_bytes_directly` para modelos en formato `.ort` (`ort-2.0.0-rc.12/src/session/builder/impl_commit.rs:76-82`). Para un `.onnx`, la alternativa es `commit_from_file` (`:45`). |
| `metrics report` alcanza para atribuir un pico | **Falsa.** Agrupa por día, por hora del día o por versión (`crates/clawg-core/src/metrics/report.rs:16-19`), aunque `metrics.db` guarda las filas por minuto durante 14 días. En Windows tampoco se registra el pico de memoria privada: `process.rss_peak_bytes` es `PeakWorkingSetSize` (`process.rs:103`), y el working set lo recorta el sistema. |
| El import pierde memoria | **No demostrado.** Retiene por diseño: embebe todas las filas a un `HashMap<i64, Vec<u8>>` (1.536 bytes de vector por fila, unos 80 MB para 52.080) antes de sacar el snapshot y escribir (`crates/clawg-core/src/memory/portability/import.rs:1629-1652`). `prepared`, con cada fila y su texto, vive toda la importación, y el cuerpo llega como un único campo `text` (`api.rs:1873`), así que el archivo está en memoria al menos dos veces mientras se lee. Con el modelo cargado, eso explica un pico de ~1 GB. |

## Problema (verificado contra el código)

### 1. En una sesión activa el modelo casi no se suelta

El `wake` en cada registro hace que después de cualquier pausa de más de diez minutos el prompt siguiente cargue el modelo (~460 MB, contando la sesión) aunque el agente no vaya a buscar nada. El refresco por registro empeora lo anterior: mientras se edita código, cada prompt encuentra archivos cambiados y los embebe, lo que mantiene el modelo cargado y suma picos de atención. Con 16 pasajes de 512 tokens, una matriz de puntajes de atención ocupa 16 × 12 cabezas × 512² × 4 bytes ≈ 192 MiB. Es un cálculo sobre la forma del modelo, no una medición. `search_project` ya refresca solo cuando el índice tiene más de un minuto (docs-refresh-background, 64), así que el refresco por prompt no le da a ninguna búsqueda una frescura que no tuviera.

### 2. Un pico de 1,28 GiB sin dueño

Hay tres candidatos y ningún dato para elegir entre ellos: la carga (el `Vec` del ONNX más lo que ONNX Runtime arma), una tanda del refresco de docs y una tanda de búsqueda o de guardado.

### 3. No hay con qué atribuir un pico

Las métricas se escriben por minuto, pero el reporte no las lee por minuto. El log dice cuánto tardó cada carga (`bootstrap.rs:472-476`) y qué indexó cada refresco (`api.rs:2105-2108`), pero no quién pidió la carga ni cuánta memoria había antes y después.

### 4. El import retiene todo hasta escribir

Ver la premisa. Lo que más pesa no es el import sino el modelo cargado debajo; lo que sí es del import (vectores, filas preparadas, el cuerpo) no está medido por paso.

### 5. ¿Se mueve el piso?

La mínima subió de 283,9 a 341,0 MiB de una hora a la siguiente. Puede ser ruido de muestreo o una deriva. 46 midió diez recargas en debug; nadie midió un día de uso real en release.

## Lo que ya existe y se reutiliza

- `metrics.db` (71): `process.private_bytes`, `process.rss_bytes`, `process.rss_peak_bytes` y `embedder.resident` por minuto durante 14 días, y `GET /v1/metrics` para el minuto en curso.
- Las líneas del log de `serve.log`: la de la carga (`bootstrap.rs:472`), la de la descarga por ocio (`idle_embedder.rs:199`) y la del refresco (`api.rs:2105`).
- Las líneas de tiempo de cada paso del import (88, `crates/clawg-core/src/memory/portability/timing.rs`).
- El procedimiento de medición de 46 en Windows (`PrivateMemorySize64`) y en Linux (`RssAnon`) (`verify.md:38`, `:127`).
- `CLAWG_EMBEDDER_IDLE_MINUTES` y `CLAWG_TEST_EMBEDDER=fake` para los tests que no necesitan el modelo.

## Paso 0: medir (sin puerta)

Nada de esto cambia qué se guarda ni cómo se busca.

| Medición | Cómo | Qué decide |
|---|---|---|
| M1. La serie por minuto | `clawg metrics report --by minute`: la agrupación existe en `report.rs`, falta la clave por minuto local dentro de `--since` (sólo cubre los 14 días que se guardan por minuto). Prueba pura sobre el agrupado | Atribuir cada pico a lo que pasaba en ese minuto, y ver si el piso deriva |
| M2. El pico privado en Windows | `PeakPagefileUsage`, de la misma llamada a `GetProcessMemoryInfo` (`process.rs:103-104`), como gauge `process.private_peak_bytes`. Linux no da un pico de `RssAnon` y se deja afuera, según la regla de 71 | El máximo real, aunque dure menos de un minuto: decide C |
| M3. La causa de cada carga | Una línea de log por carga que diga si la pidió un `wake` o una llamada que embebe, con la memoria privada antes y después. Si 73 llega antes, se toma de su evento `embedder.load` | Cuántas cargas por hora son de un `wake` que nadie usó: decide B |
| M4. La memoria por paso del import | La memoria privada al terminar cada paso, sumada a la línea de tiempo que 88 ya escribe | Cuánto es del import y cuánto del modelo: decide D |

Con M1 a M4, un día de uso normal en un perfil real alcanza para responder las cinco preguntas del problema.

## Enfoques posibles

### A. Refrescar el índice de docs sólo cuando el pedido trae `index_docs: true` (recomendado, sin puerta)

Es el arreglo que el backlog ya proponía. `spawn_doc_refresh` corre sólo cuando la petición lo pide (el primer registro del puente), no cuando la marca ya estaba guardada. La frescura queda a cargo de `search_project`. **A favor:** saca todo el trabajo por prompt (recorrer, hashear, embeber, escribir en la única conexión) sin cambiar lo que encuentra una búsqueda. **En contra:** la primera `search_project` después de editar espera hasta `REFRESH_GRACE` (3 s) por el refresco que lanza, como ya pasa hoy con cualquier edición dentro de la sesión. Hay que revisarlo contra `specs/project-docs`, que pide indexar en segundo plano al registrar con `index_docs`. Puede salir antes como arreglo suelto, como el paso 0 de 62.

### B. Despertar el modelo sólo cuando empieza una sesión (con puerta M3)

- **B1.** El hook `user-prompt-submit` manda `wake: false`, un campo opcional que un daemon viejo ignora.
- **B2.** El daemon despierta sólo con el primer registro de una raíz en su vida, sin cambiar la API.
- **B3.** Ningún registro despierta, y la primera búsqueda paga la carga.

**A favor:** después de una pausa no se cargan ~460 MB si el agente no los usa. **En contra:** la primera búsqueda después de esa pausa espera la carga (lectura con hash más sesión: en release está sin medir, en debug fueron 8,7 a 13,5 s, `verify.md:123`). Si M3 muestra que casi todo `wake` termina en una llamada que embebe pocos segundos después, el `wake` hace bien su trabajo y se cierra sin cambio.

### C. Cargar el ONNX sin el `Vec` de 470 MB (con puerta M2)

Se verifica el SHA-256 leyendo el archivo de a partes, y después `commit_from_file` en lugar de `commit_from_memory`. **A favor:** el pico de la carga baja cerca de 470 MB si ONNX Runtime no hace su propia copia entera; M2 lo confirma antes y después. **En contra:** el archivo se lee dos veces por carga, y entre la verificación y la carga otro proceso que pueda escribir en el directorio de datos podría cambiarlo. Es la misma confianza que ya se le da a `token` (directorio `0700` en Unix, la ACL del perfil en Windows). Si M2 muestra que la carga no es el máximo del proceso, no se hace.

### D. Embeber y escribir el import por tanda (con puerta M4; se coordina con 87)

Cada transacción de `WRITE_BATCH` embebe sus filas antes de escribirlas, en lugar de juntar todos los vectores primero. **A favor:** los vectores dejan de crecer con el archivo. **En contra:** hoy un embebido que falla no saca snapshot ni escribe nada. Con D, el snapshot va antes del primer embebido, y un fallo a mitad de camino deja una importación parcial: queda en el diario y se deshace con `import --undo` (84), pero cambia lo que se promete. 87, que pasa el archivo por ruta en vez de por cuerpo, ya saca las copias del cuerpo, que es la otra mitad.

### E. Bajar el `CLAWG_EMBEDDER_IDLE_MINUTES` por defecto

Es una decisión de producto que se toma recién después de A y B y de M1: sin ellos se estaría ajustando un reloj que los hooks reinician igual.

## Recomendación

1. A, como arreglo suelto o primero dentro del cambio.
2. El paso 0 (M1 a M4) y un día de uso real.
3. B, C y D, cada uno según su medición. E al final, si todavía hace falta.

## Descartado

| Opción | Razón técnica |
|---|---|
| Un proceso hijo para el embedder | 46 lo descartó: su paso 0 midió que la memoria vuelve sin salir del proceso (`changes/archive/2026-10-17-embedder-idle-unload/proposal.md:23`) |
| mimalloc o `HeapCompact` en Windows | 46 los midió: mimalloc queda plano pero entre 280 y 430 MB, y `HeapCompact` devuelve 1 a 2,5 MB (`verify.md:64`). D5 resolvió la causa |
| INT8, o recortar el vocabulario del modelo | La tabla de embeddings (unos 250.000 tokens × 384 × 4 bytes ≈ 384 MB, cálculo sobre la forma del modelo) es la mayor parte del ONNX, pero achicarla cambia los vectores: es 85 y 17 |
| Achicar el tokenizer | Es el piso con el modelo suelto y un trabajo de otro tamaño: 91 |
| Bajar `EMBED_BATCH` o agrupar por tokens | Cambia la composición de cada tanda y puede mover bits: es el no-alcance de 62 y el brazo D de 62 y de 85 |
| `commit_from_memory_directly` | Sólo para modelos `.ort`; pasar a ese formato es otro contrato de descarga |

## Interacciones

- **embedder-footprint (62).** Le saca sus preguntas de memoria (a) (crecimiento con `IDLE_MINUTES=0` y entradas de largo variable) y (b) (el pico de la carga). La (c) (una arena contra dos y la latencia durante un refresco) se queda allá. 62 queda con la CPU.
- **pipeline-metrics (73).** Su evento `embedder.load` y sus series del embedder hacen permanente lo que M3 registra en el log. Si 73 va antes, M3 sale de ahí.
- **perf-bench (74).** Un escenario de memoria (cargas, refrescos y búsquedas a ritmo fijo, con `--by minute`) es la forma repetible del día de uso de M1.
- **embed-throughput (85).** Si adopta INT8, cambia cuánto ahorran B y C. Ordenar por largo baja el relleno y, con él, los picos de atención.
- **import-job (87).** El archivo por ruta saca las copias del cuerpo. D va con 87 o después.
- **tokenizer-footprint (91).** El piso. Cuanto menos tiempo pasa el modelo cargado gracias a A y B, más pesa ese piso.
- **docs-refresh-background (64, archivado).** A se apoya en el refresco en segundo plano de `search_project`.
- **Specs:** `metrics` (M1 y M2), `project-docs` (A), `hooks` y `storage` («Ciclo de vida del embedder») para B, y `portability` para D.

## Alcance candidato

1. A, con un test de que un registro sin `index_docs` sobre una raíz marcada no lanza un refresco.
2. M1 (`--by minute`), M2 (`process.private_peak_bytes` en Windows), M3 (la causa de cada carga) y M4 (la memoria por paso del import).
3. La medición de un día de uso real y su registro en `verify.md`.
4. Según el resultado: B, C y/o D, cada uno con su test y su medición repetida.

### No-alcance

- Cambiar el modelo, el contrato de embedding o el tamaño de las tandas.
- El tokenizer (91).
- La CPU del embedder (62).

## Decisiones abiertas

Son del usuario.

- [ ] **A como arreglo suelto.** ¿Sale antes que el resto, con su propio patch? Candidato: sí, porque no necesita medición.
- [ ] **B.** ¿B1, B2 o B3? ¿Con qué proporción de `wake` sin uso posterior se justifica (por ejemplo, más de la mitad de las cargas de un día)?
- [ ] **C.** ¿Se acepta la ventana entre la verificación y la carga?
- [ ] **D.** ¿Se acepta que un import que falla a mitad de camino quede parcial y deshacible, en lugar de no escribir nada?
- [ ] **M1.** ¿`--by minute` en `metrics report`, o un subcomando que exporte la serie cruda?
- [ ] **El día de medición.** ¿Prod (autorizado, sólo lectura de `metrics.db` y `serve.log`) o dev con Claude Code?

## Cómo se mide

1. **A (RED).** Un test del handler: registrar sin `index_docs` una raíz marcada no reclama `DocIndex::begin`. Hoy lo reclama.
2. **M1 y M2.** Pruebas puras del agrupado por minuto y del muestreo en Windows (el campo existe y es mayor o igual que `PrivateUsage`).
3. **El día de uso.** Hace falta un perfil con uso real y el modelo real. `metrics report --by minute` sobre `process.private_bytes`, `process.private_peak_bytes` y `embedder.resident`, contra las líneas de carga y de refresco de `serve.log`. Las mismas lecturas antes y después de A y de B.
4. **C.** `process.private_peak_bytes` en diez cargas con `CLAWG_EMBEDDER_IDLE_MINUTES=1`, con `commit_from_memory` y con `commit_from_file`, y `e5_golden` para confirmar que los vectores no cambian.
5. **D.** M4 importando el mismo archivo antes y después, más los tests de `import --undo` sobre una importación cortada.

## Riesgos

- **Que la medición diga que no vale la pena.** Es un resultado válido: A sola ya saca el trabajo por prompt.
- **Los números de debug no son los de release.** Cada medición dice qué build usó. Lo de prod de arriba es release.
- **B empeora la primera búsqueda después de una pausa.** M3 cuantifica cuántas cargas evita, y el tiempo de carga en release se mide en el mismo paso.
- **C abre una ventana entre la verificación y la carga.** Queda acotada al mismo nivel de confianza que `token`.
- **D cambia una garantía del import.** Por eso va con decisión explícita y con 87.
