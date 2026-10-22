# docs-refresh-background — Diseño

## D1. Tarea propia y espera acotada (R1)

**Elegido:** la búsqueda toma la marca con `DocIndex::begin_if_due` (comprueba y toma en el mismo lock, así no hay carrera entre `is_due` y `begin`) y pasa el `RefreshGuard` a una tarea de `tokio::spawn`, la misma que lanza el registro (`refresh_in_background`, extraída de `spawn_doc_refresh`). La tarea tiene su propia copia del `AppCtx` y del embedder, así que no se cae con el futuro del handler. La búsqueda espera con `wait_idle`, que ya existía.

Como la marca se toma antes del `spawn`, `is_running` es cierto desde que la búsqueda decide; no hay un instante en que una segunda búsqueda vea el proyecto vencido y lance otro.

**El plazo.** `REFRESH_GRACE` = 3 s. La única medición es un reindexado completo de este repositorio: 645 s para 226 docs en un binario debug, unos 2,9 s por doc. Con 3 s, una edición de un doc entra en la búsqueda que la descubre aun en esa compilación, y un checkout o un cambio de modelo cuestan 3 s a esa búsqueda, no minutos. En release no hay cifra; el plazo es un tope, no un costo: si el refresco termina antes, la búsqueda sigue enseguida.

| Alternativa | Por qué no |
|---|---|
| A: no esperar (`GRACE = 0`) | Un doc editado no aparece en la búsqueda que lo descubre sino en la siguiente, que es justo lo que el diseño del índice quiso evitar; el costo de esperar un poco es acotado |
| C: refrescar en línea si el plan es chico | Hay que partir `refresh` en planear y aplicar y pasar la marca entre tareas, más un umbral; da casi lo mismo que B con más piezas |
| Plazo en el cliente | Rompe la garantía de `only_health_sets_a_timeout` y deja el refresco cancelándose a mitad |
| Esperar `GRACE` también al encontrar un refresco ajeno en curso | Durante un reindexado largo cada búsqueda pagaría 3 s sin ganar nada: el refresco ajeno no terminará por esperarlo |

**`last_started` tras una cancelación.** Ya no hace falta corregirlo: la petición no cancela el refresco. Sólo el apagado lo corta, y entonces el proceso se va.

## D2. El último error, en `Progress` (R2)

**Elegido:** `Progress` guarda `last_error: Option<String>`, escrito por `RefreshGuard::run` al terminar (con el texto del error o vacío si terminó bien) y antes de soltar la marca, así quien espera con `wait_idle` lo lee ya escrito. `try_refresh` pasa por `run`, de modo que todos los caminos lo escriben. `Progress` deja de ser `Copy`; se clona bajo el lock.

La búsqueda lee `last_error` después de esperar. Persistir hasta un refresco exitoso (y no sólo hasta la búsqueda siguiente) hace que el modelo sepa, mientras sea cierto, que el índice puede estar viejo: un error que se dice una vez y después calla deja creer que lo indexado está al día.

| Alternativa | Por qué no |
|---|---|
| Mostrarlo una vez y borrarlo | La siguiente búsqueda leería el índice viejo como fresco |
| Un canal o un registro de eventos | No hay quien lo consuma fuera de la búsqueda |

## D3. Qué es un doc malo y dónde se cuenta (R3)

**Elegido:** `embed_passages` devuelve `DocIndexError::Vector` sólo por `to_unit`, que mira un vector a la vez; la cuenta distinta pasa a un error propio, `VectorCount`, que aborta: dice que el embedder no cumple su contrato (`Embedder::embed_batch` exige un vector por texto), y seguir repetiría el fallo en cada doc. `refresh` atrapa sólo `Vector`, lo anota en `RefreshReport::failed` (`FailedDoc { path, reason }`) y continúa.

`skipped` sigue significando «grande, pasado el tope o no UTF-8», cosas del disco que no se reintentan. Un doc fallido sí se reintenta, así que va aparte.

El modelo no lo ve en `warning`: el refresco terminó, el resto del índice está al día y el doc conserva su versión anterior. El daemon lo deja en el log como `warn`, con la causa. `rag-eval --docs` falla, porque una medición sin un doc mide otro corpus.

**Reintento.** Un doc que falla siempre se vuelve a embeber en cada refresco, como mucho uno por minuto y sólo cuando alguien busca. Recordar la huella fallida evitaría ese costo, pero agrega estado para un caso que no se observó; queda en «Límites conocidos».
