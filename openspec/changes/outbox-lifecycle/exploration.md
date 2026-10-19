# outbox-lifecycle — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (conviene antes de 44; ver «Interacciones») | exploración (borrador) |

**Tesis:** un save que clawg aceptó termina de guardarse, o termina en un estado que el agente y el usuario ven y pueden resolver; un save que falla a mitad de camino no deja media versión viva. Hoy el outbox tiene el principio y el final del ciclo bien hechos (se escribe antes de contestar, se reanuda tras un corte) pero no el medio ni la salida: un fallo pasajero no se reintenta hasta el próximo arranque, un fallo de inserción en el pedazo k deja los pedazos 1..k-1 vivos y sin vínculo mientras la versión anterior de la clave sigue viva, el apagado nunca corta el evento en curso aunque ya podría retomarlo, y una fila `failed` no tiene salida. Además la documentación del proyecto y el código no dicen lo mismo sobre cuándo una fila pasa a `failed`. Son cuatro huecos del mismo ciclo de vida y por eso un cambio. Va antes de 44 (history-import), que trae miles de notas largas por este camino.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `outbox_pending` se lee una sola vez, al arrancar el worker | **Cierta.** La única llamada de producción es `runtime.rs:418`; las demás son tests (`worker.rs:3151`, `:3269`). Nada relee el outbox durante la sesión. |
| Un fallo transitorio devuelve el intento y nada lo reintenta en la sesión | **Cierta.** `after_failure` (`outbox.rs:75-79`) resta el intento si `transient`; `outbox_fail` (`:300-318`) lo escribe; el worker lo invoca en `worker.rs:313-318` con `transient = (failure == EmbedderUnavailable)`. La fila queda `pending` hasta el próximo arranque. **Matiz: es diseño escrito**, no olvido: `specs/storage`, «Intentos y fallo a la vista», dice «se reintenta en el próximo arranque, no en la misma ejecución, porque el mismo texto falla igual». Vale para un fallo determinista (un vector NaN), no para uno pasajero. |
| `EmbedderUnavailable` también sale con un daemon `ready` | **Cierta.** La recarga fallida del modelo ocioso devuelve `EmbedError::NotReady` (`idle_embedder.rs:105-111`), `embed_batch` lo mapea a `EmbedderUnavailable` (`worker.rs:748-750`), y la celda del embedder ya está llena (`worker.rs:649` no se alcanza). El estado de `/health` sigue `ready`. |
| El `.claude/CLAUDE.md` dice que una fila que «failed after the caller was answered» queda `failed` | **Cierta, y el código no lo hace.** En `outbox_fail`, una falla no transitoria con `attempts < 3` deja `pending` (`outbox.rs:77`); una fila sólo pasa a `failed` por `claim_step` tras 3 arranques (`outbox.rs:269-280`), por `after_failure` cuando ya traía 3 intentos, o por un evento que no parsea (`outbox.rs:376-380`). `specs/storage` (líneas del escenario «fallo después de un `202`») sí coincide con el código. La frase de `.claude/CLAUDE.md` es la que está mal. |
| El texto que el puente le da al agente cubre un `202` que después falla | **Parcial.** `server.rs:606-614` dice «memory_status lists it if it fails». Cierto mientras la fila está `pending` o `failed`; pero con un `500` contestado la fila se borra (`api.rs:1126-1130`) y ya no hay nada que listar, lo cual es correcto porque el agente recibió el fallo en la respuesta. |
| Un fallo en el pedazo k deja los pedazos 1..k-1 vivos | **Cierta para el insert, falsa para el embed.** Los vectores de todos los pedazos que sobreviven al dedup se calculan en **una** llamada antes de insertar nada (`worker.rs:744-754`); si falla, no se guardó ningún pedazo nuevo en ese intento. El prefijo sólo nace del fallo de `insert_noting` (`worker.rs:800-806`) o de una reanudación que ya traía pedazos. Cada pedazo se confirma en su propia transacción (`worker.rs:799`) y nace con el `topic_key` (`store_insert.rs:219`, escrito en `:474`). |
| `outbox_finish` es el único que corre `link_in` y `revise_in` | **Cierta** salvo para un evento sin fila, que usa `finish_save` (`worker.rs:837`). En producción todo save pasa por `accept` con fila (`runtime.rs:231`); un evento sin fila sólo viene de `From<MemoryEvent>` (`worker.rs:69-72`), usado por tests. |
| El fallo no reemplaza nada | **Cierta**, y es el problema: `process_tracked` vuelve antes de `outbox_finish` (`worker.rs:585-587`), así que la versión anterior de la clave sigue viva y el prefijo nuevo también lleva la clave. Dos filas vivas con la misma clave y pedazos sueltos que se muestran como saves independientes, sin `piece=K/N`. |
| Con el que guardó esperando, el handler olvida la fila y con ella el progreso | **Cierta.** `api.rs:1126-1130` llama `outbox_forget` ante cualquier `report.failure`. Con un prefijo guardado, la fila era lo único que permitía terminar el save. El puente dice `not saved: …; partially saved as #ids` (`server.rs:633-640`), lo que es literal pero deja el prefijo vivo. |
| Nada repara el prefijo | **Cierta, con una salvedad que `specs/pieces` documenta:** guardar el mismo texto otra vez absorbe los pedazos ya guardados (dedup) y escribe el vínculo entonces (`specs/pieces`, «un save que se corta…»). El agente tiene que reenviar a mano, y la absorción suma un acceso a cada pedazo. Un save sin `topic_key` que nadie reenvía deja los pedazos sueltos para siempre. |
| El comentario de `worker.rs:553-556` dice que un fallo «replaces nothing» | **Cierta**, y es lo que el prefijo viola desde el otro lado: no reemplaza, pero ya insertó. |
| El apagado nunca corta el evento en curso | **Cierta.** `drain_step` se evalúa una vez por evento, antes de empezarlo (`worker.rs:204-211`, llamado en `:288`). Los bucles por pedazo (`worker.rs:694-719`, `:783-825`) no miran el plazo. |
| La razón del comentario (`worker.rs:229-231`) ya no es cierta | **Cierta para filas con seguimiento.** `insert_noting` anota el progreso en la misma transacción (`worker.rs:799-800`, el absorbido en `:701-702`) y `resume_positions` (`outbox.rs:97-107`, usada en `worker.rs:680`) salta lo anotado. En producción todo save tiene fila (arriba), así que el caso «sin seguimiento» sólo existe en tests. |
| `DRAIN_DEADLINE` es 10 s y `STOP_TIMEOUT` 15 s | **Cierta** (`runtime.rs:54`, `crates/clawg-cli/src/stop.rs:19`; el test que los relaciona está en `stop.rs:66`). Hay además un plazo de confirmación de 10 s para el que espera (`SAVE_CONFIRM_TIMEOUT`, `api.rs:98`), tras el cual la respuesta es «sin confirmar» y el fallo posterior no llega a nadie. |
| Una fila `failed` no tiene salida | **Cierta.** Del outbox sólo salen `outbox_forget` (`api.rs:1127`, `runtime.rs:234`), `outbox_finish` (`outbox.rs:347`) y el `purge`. Nada la reintenta ni la descarta; `/health` la cuenta (`outbox.rs:396-402`) y reenviar su `operation_id` responde `500 ingest_failed` (`api.rs:1109-1111`). |
| `last_error` puede citar contenido | **No verificado.** Las causas que escribe el código son `embed failed: {e}`, `store insert failed: {e}` (`worker.rs:752`, `:803`) y las fijas de `outbox.rs`. Que `{e}` cite texto del save depende de cada librería; hace falta un fallo provocado (ver «Cómo se mide»). Es de secret-coverage (51); aquí sólo se anota. |

## Problema (verificado contra el código)

### 1. Un fallo pasajero espera al próximo arranque

El único camino de reintento es el arranque (`runtime.rs:418` → `Replay::rows`, `worker.rs:176-182`). Los casos sin nadie esperando son: la fila que llegó con el modelo cargando (`202 embedder_loading`, `api.rs:1092-1117`), la que superó los 10 s del que esperaba, y toda fila repetida de un arranque. Si en ellos el embedder falla de forma pasajera (la recarga del modelo ocioso, `idle_embedder.rs:105-111`, o el trabajador del embedder caído), el worker anota la causa y sigue; la fila queda `pending` con `attempts` intacto y no se vuelve a mirar hasta que el daemon se reinicie, que puede ser días. Es una pérdida aparente: el agente fue informado de «accepted … memory_status lists it if it fails» y `memory_status` la lista, pero nada la termina.

### 2. La regla de `failed` no es una sola

`claim_step` cuenta arranques; `after_failure` cuenta fallos, pero un fallo no transitorio en la primera pasada deja `pending` con 1; el que espera nunca ve ese estado porque su fila se borra. El resultado es que «failed» significa hoy «el proceso murió 3 veces con esta fila», y la frase de `.claude/CLAUDE.md` promete otra cosa. Sea cual sea la regla que se elija, el código, `specs/storage` y `.claude/CLAUDE.md` tienen que decir la misma.

### 3. Media versión viva

Con un save largo y `topic_key` sobre una clave existente, si el insert del pedazo k falla con el que guardó esperando:

- los pedazos 1..k-1 están vivos, con la clave, sin fila en `memory_piece` (`link_in` no corrió);
- la versión anterior de la clave sigue viva (`revise_in` no corrió);
- la fila del outbox se borra (`api.rs:1126-1130`) y con ella el progreso;
- el agente recibe `not saved: …; partially saved as #ids`.

Para una búsqueda hay entonces dos versiones vivas de la clave y el prefijo aparece como saves independientes. La invariante «una versión viva por clave» (la que 31 querría comprobar) está rota hasta que alguien reenvíe. Para un save sin clave los pedazos quedan sueltos sin que nada los señale. Con nadie esperando el estado es mejor: la fila conserva el progreso y el próximo arranque termina el save (`resume_positions`), con el mismo costo de esperar del punto 1.

### 4. El apagado no retoma lo que podría

Un evento de, por ejemplo, 200 pedazos que empieza a los 9 s del plazo se termina entero o se mata el proceso a los 15 s. Si lo mata `STOP_TIMEOUT` sin terminar, el estado es el mismo de un corte duro: prefijo en `memory`, progreso anotado, la fila sigue. Pausar entre pedazos dejaría el mismo estado pero a propósito y sin riesgo de pasar el tope. Hay un detalle: `outbox_claim` ya gastó un intento (`outbox.rs:282-285`), así que tres pausas del apagado sobre un mismo save lo darían por `failed` con «the daemon stopped 3 times» (`outbox.rs:269-280`), que sería una mentira. Una pausa tiene que devolver el intento.

### 5. Una fila `failed` no sale

Está `failed` hasta que alguien edite la base. Cuenta en `/health` para siempre, el `operation_id` queda reservado (`api.rs:1109-1111`) y el que lo reenvía recibe el mismo `500`. Si la fila tiene progreso, también tiene pedazos vivos sueltos.

## Lo que ya existe y se reutiliza

- Progreso por pedazo en la misma transacción que la inserción y `resume_positions`: la reanudación sin duplicados ya existe y está probada (`worker.rs:2250-2290` y los de replay, `:3151-3280`).
- `after_failure` y `claim_step` son funciones puras (`outbox.rs:64-79`): la regla nueva se prueba sin base.
- `drain_step` pura (`worker.rs:204-211`); hace falta una variante por pedazo, no un mecanismo nuevo.
- Un único consumidor de los inserts en `memory`: `memory_worker` (nota en `worker.rs:246-251`). Todo reintento tiene que entrar por ahí.
- `delete_memory_core` (`admin.rs:316`) y su inverso `restore_memory_core` (`admin.rs:583`): borrar sin perder, con vuelta atrás.
- `OutboxSummary` y `OutboxProblem` (`outbox.rs:146-163`) ya dan a `/health`, y por él a `memory_status`, lo que hay que mostrar.
- `operation_id` único por agente (`outbox_record`): un reintento no duplica.

## Enfoques posibles

### Cuando un save falla a la mitad

**A. Deshacer.** En la transacción que registra el fallo contestado, los pedazos que **este** save insertó (`inserted: true` en el progreso, `outbox.rs:85-91`) pasan a borrados por el camino de `delete_memory_core`.
- **A favor:** el `not saved` es verdad; no hay media versión; la clave queda con una sola versión viva.
- **En contra:** cambia lo que `specs/ingest` («insert parcial», «corte en el primer fallo») y `specs/pieces` dicen hoy: que lo guardado es un prefijo y que el cuerpo del error lo lista en `memories`. Los pedazos reforzados por el dedup no se deshacen (les sumó un acceso, no es dato del usuario). Borrar la fila vigente de una clave dispara `revival.rs`: hay que probar que, como aquí la anterior nunca se reemplazó, no revive nada. Con nadie esperando no hay a quién decirle «no guardado», y deshacer trabajo que el reintento habría terminado es un desperdicio.

**B. Conservar y terminar.** Nunca olvidar una fila cuyo save insertó pedazos; terminarla después (reintento o próximo arranque).
- **A favor:** coherente con `specs/storage` (replay retoma) y con `specs/pieces`; no destruye ni oculta nada; es el único camino correcto con nadie esperando.
- **En contra:** al que esperaba ya se le dijo `not saved`; si el save termina después, la afirmación quedó falsa. Hay que cambiar el texto a «partially saved … clawg will finish it» o no contestar un 500. Hasta que termina, el estado «media versión» dura lo que dure el reintento.

**C. Un save en una sola transacción.** Insertar todos los pedazos, el vínculo y la revisión en una. Descartado abajo.

**D (recomendado). B por defecto, A como salida explícita.** Una fila con pedazos insertados no se olvida cuando el que esperaba recibe el fallo; el handler la deja `pending`/`failed` y la respuesta dice que quedó parcial y que se terminará. El reintento (punto siguiente) la termina; si llega a `failed`, el descarte (abajo) decide: dejar los pedazos o borrar suavemente los que insertó. Así el camino automático no destruye nada y el deshacer lo pide el usuario o el agente, con el id a la vista.

### Reintento en la sesión

**E (recomendado). Reintentar sólo lo pasajero.** Tras un `EmbedderUnavailable`, el worker vuelve a poner la fila en su propia cola local con un `not_before` creciente (1 s, 2 s, … tope 60 s) y la toma otra vez; es el mismo consumidor, sin tarea nueva. Un barrido de baja frecuencia (por ejemplo cada 5 min) relee `outbox_pending` para las `pending` cuya `updated_at` sea vieja y no estén ya en la cola local. Un fallo determinista (`Failed(_)`) conserva el comportamiento de hoy salvo por la regla de la sección siguiente.
- **A favor:** respeta la razón escrita en `specs/storage` (el mismo texto falla igual); cierra el hueco sin schema nuevo (el retraso vive en memoria); acotado por el tope y el barrido.
- **En contra:** el set `taken` (`worker.rs:262`) hay que vaciarlo para la fila al reencolar, o se la salta como «entrega doble» (`worker.rs:422`); un reintento tiene que respetar el drenaje (no reencolar en apagado).

**F. Reintentar también los fallos no transitorios en la sesión**, con retraso creciente hasta `MAX_ATTEMPTS`. Más simple de explicar («3 fallos y es `failed`»), pero contradice el diseño escrito y convierte un NaN determinista en tres ciclos de embed inútiles. Es la alternativa si se prefiere una sola regla (siguiente punto).

### Una sola regla para `failed`

**G (recomendado).** Un intento es una toma de la fila (`outbox_claim`). Una fila pasa a `failed` cuando gastó `MAX_ATTEMPTS` tomas que terminaron en fallo no transitorio o en muerte del proceso. Un fallo pasajero y una pausa de apagado devuelven su intento. Eso conserva `claim_step` y `after_failure` casi como están; lo que cambia es que el texto de `.claude/CLAUDE.md` pasa a decir lo que el código hace («3 tomas fallidas o interrumpidas») y `specs/storage` se ajusta a E. **H.** Cambiar el código para que un fallo no transitorio pase a `failed` en la primera vez: descartado, porque una muerte del proceso a mitad de un save quedaría igual a un NaN.

### Pausa en el apagado

**I (recomendado).** El bucle por pedazo (`worker.rs:783-825`) consulta el plazo antes de cada pedazo; vencido, devuelve un resultado `Paused` (no un `IngestFailure`) con el progreso ya anotado; el worker devuelve el intento (misma operación que un fallo transitorio, sin cambiar `last_error`), no llama a `outbox_fail` ni deja que el handler olvide la fila, y retorna con el resto como hoy. Sólo para filas con seguimiento: en producción son todas. Encaja con D: el estado en que queda es el de la reanudación normal. El que esperaba pierde su canal (la respuesta se descarta y ve «sin confirmar», el mismo caso del plazo vencido).
- **En contra:** hay que decidir qué ve el que espera; hoy un reporte con `failure: None` y menos pedazos que los pedidos no existe.

### Salida para una fila `failed`

**J (recomendado).** Dos acciones sobre una fila `failed`, por `operation_id`: **reintentar** (`pending`, `attempts = 0`, encola en el worker) y **descartar** (borra la fila; opcionalmente borra suavemente los pedazos que insertó). Un comando `clawg outbox retry|discard <OPERATION_ID>` por el daemon, como `undelete`, y la misma información en `memory_status` (que ya lee `/health`) para que el agente la vea. **K.** Una herramienta MCP para ello: añade superficie al contrato de `agent_contract.json` por un caso raro; se prefiere el CLI para el usuario y que el agente sólo vea el estado. **L.** Descartar solo por expiración: borra un save aceptado sin que nadie lo decida; descartado.

## Descartado

| Opción | Razón técnica |
|---|---|
| C. Un save en una sola transacción | La sonda de contradicción del pedazo N busca por KNN sobre filas ya guardadas, incluso las del pedazo N-1 (`worker.rs:767-782`); meter todo en una transacción mantendría una escritura abierta durante cada embed y cambia el modelo de hermanos. Es un rediseño de tamaño L y 54 (save-bounds) toca el mismo bucle. |
| Reintentar desde otra tarea | Rompe el único consumidor de los inserts en `memory` (`worker.rs:246-251`). Todo entra por el canal o la cola local del worker. |
| Expiración automática de filas `failed` | Borraría un save aceptado sin una decisión. |
| Marcar `failed` la fila de un evento pausado por el apagado | Es una mentira sobre la causa y bloquea el replay del próximo arranque. |
| Reescribir `last_error` para enmascarar | Es de secret-coverage (51); aquí sólo se verifica. |

## Interacciones

- **54 save-bounds.** Comparte el bucle por pedazo y el formato de `progress` (`NotedPiece`, `outbox.rs:85-91`). Si 54 lo cambia (por ejemplo, un tope de pedazos), la pausa de I y el descarte de J se escriben sobre el formato nuevo; conviene acordar el orden antes de empezar a aplicar.
- **55 shutdown-budget.** El drenaje es una fase del presupuesto de `stop`; I lo hace acotado de verdad (hoy depende de que el último evento sea corto). No debe tocar `STOP_TIMEOUT` ni `AFTER_DRAIN`, sólo el modo de cumplir `DRAIN_DEADLINE`.
- **51 secret-coverage.** `purge` borra filas del outbox por subcadena (`secrets.rs:114`); las acciones de J se declaran sobre `operation_id` exacto y por agente. `last_error` y el `reason` de un veredicto son su tema.
- **31 doctor-checks.** Puede señalar filas `failed` y comprobar «una versión viva por clave»; este cambio la repara en el camino normal pero no la comprueba.
- **44 history-import.** Trae miles de notas largas por este camino, así que cada hueco de arriba se multiplica; por eso va antes.
- **36 ingest-outbox (archivado).** Se corrigen `openspec/specs/ingest/spec.md` («Un fallo del worker llega al que guardó», «insert parcial») y `openspec/specs/storage/spec.md` («Intentos y fallo a la vista»); `specs/pieces` se ajusta en su párrafo del save cortado.

## Alcance candidato

1. Un save con pedazos insertados no se olvida cuando el que esperaba recibe el fallo; la respuesta lo dice (D).
2. Reintento en la sesión de lo transitorio, con retraso creciente y barrido lento, por el mismo consumidor (E).
3. Regla única de `failed`, y `.claude/CLAUDE.md`, `specs/storage` y los comentarios de `outbox.rs` y `worker.rs` alineados (G).
4. Pausa del evento en curso al vencer el plazo de drenaje, devolviendo el intento (I).
5. Reintentar y descartar una fila `failed` (CLI por el daemon) y su estado en `memory_status` (J).
6. Textos del puente (`server.rs:600-640`) y de `protocol.md` si cambian; si cambia la descripción de una herramienta, `agent_contract.json` se bendice (`CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file`).

### No-alcance

- Un save en una sola transacción (C) y cualquier cambio de esquema: la hipótesis es que no hace falta (el retraso vive en memoria y el progreso ya está). Si D exige distinguir «contestada» de «no contestada» en la fila, se reabre con migración, vuelta atrás y un test sembrado en la versión anterior.
- Enmascarar `last_error` (51), los chequeos de 31, topes de tamaño (54).
- Cambiar `MAX_ATTEMPTS` o `DRAIN_DEADLINE`.

## Decisiones abiertas

Son del usuario.

- [ ] **Fallo parcial con el que esperaba.** ¿D (conservar y terminar, deshacer a pedido), A (deshacer siempre) o B sola?
- [ ] **Qué le dice la respuesta.** ¿`500 ingest_failed` con `memories` y un texto nuevo, o un `202` con el estado parcial? El primero conserva el contrato; el segundo es más honesto.
- [ ] **Alcance del reintento.** ¿Sólo lo transitorio (E) o también lo determinista (F)? Si F, ¿cuántos intentos y qué retraso?
- [ ] **Qué significa `failed`.** ¿La regla G, que deja el `.claude/CLAUDE.md` corregido, u otra?
- [ ] **Descartar una fila `failed` con progreso.** ¿Deja los pedazos guardados o los borra suavemente (reversible con `undelete`)?
- [ ] **Superficie.** ¿CLI y `memory_status` (J), o también una herramienta MCP (K)?
- [ ] **Orden con 54.** ¿Este cambio primero (y 54 se escribe sobre el bucle ya pausable) o al revés?

## Cómo se mide

Tests RED primero, con `FakeEmbedder` y un embedder de prueba configurable (hoy el repo tiene `NanAtEmbedder`, `worker.rs:2250`; hace falta uno que falle una vez y luego funcione):

1. Embedder que falla una vez con `EmbedderUnavailable` y save sin nadie esperando (`reply: None`): la fila se reintenta y termina en la misma ejecución del worker, sin reiniciarlo. Hoy queda `pending`.
2. Save con `topic_key` sobre una clave existente, de varios pedazos, cuyo insert falla en el pedazo 2 con el que guardó esperando: después del fallo la clave tiene una versión viva y ningún pedazo suelto, o la fila sigue con su progreso y se termina después (según lo que se elija). Hoy hay dos versiones vivas y se borra la fila.
3. Evento de varios pedazos con progreso y un plazo de drenaje que vence a mitad: el worker retorna pronto, la fila conserva el progreso y los `attempts` previos, y una segunda ejecución la termina sin duplicados (mismas ids, `memory_piece` completo). Hoy el evento corre entero.
4. Tres pausas seguidas de un mismo save no lo dejan `failed` (`claim_step`).
5. Unitarios puros de la regla de `failed` (extienden `after_failure_fails_a_row_only_at_the_cap_and_hands_back_a_transient_attempt`).
6. Una fila `failed` se reintenta o se descarta por su `operation_id`; reenviar el `operation_id` de una descartada la acepta; `/health` deja de contarla.
7. `last_error` de un fallo provocado (vector NaN, embedder que devuelve un error con texto): se comprueba si cita el contenido del save. Resultado esperado para 51, no una corrección aquí.

Un test sembrado en la versión anterior (regla de `config.yaml`) sólo hace falta si algún enfoque cambia el esquema. No lo hace en la recomendación.

## Riesgos

- **Perder o duplicar un save al reencolar.** Reintentar por el mismo canal y con el progreso anotado evita duplicar; el riesgo está en `taken` y en una fila que llega por dos caminos. El test 3 y el de entrega doble existente lo cubren.
- **Un reintento que no termina nunca.** El tope del retraso y el de `MAX_ATTEMPTS` lo acotan, y el barrido no toca filas en la cola local.
- **Contradecir al agente.** Si un `500` se contesta y el save termina después, el agente cree que no se guardó. D lo evita cambiando lo que dice la respuesta; es la decisión que más importa.
- **Deshacer toca la base del usuario.** Si se elige A o el descarte con borrado, es un borrado suave (reversible) y se prueba contra `revival.rs`; nunca un borrado físico.
- **Pausar con un reporte que nadie espera.** El que esperaba ve «sin confirmar»; es el mismo estado de un plazo vencido y no un error nuevo.
- **Solapamiento con 54 y 55** sobre el mismo bucle y la misma fase del apagado; se mitiga fijando el orden antes de aplicar.
