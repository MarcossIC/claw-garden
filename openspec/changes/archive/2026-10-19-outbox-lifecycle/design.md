# outbox-lifecycle — Diseño

## D1. Un fallo pasajero retiene la cola (R1)

**Elegido:** tras un `EmbedderUnavailable` de un save con fila, el worker anota la causa y devuelve el intento como hoy, contesta al que espera (si lo hay) y se queda con ese mismo pedido: espera `retry_delay(n)` mirando la señal de drenaje, toma la fila otra vez (`take_row`, que lee el progreso fresco) y lo procesa. El pedido reintentado ya no lleva respuesta. Nada detrás de él se toma mientras tanto.

`retry_delay(n)` es pura: `min(1 s · 2ⁿ, 60 s)`. El tope coincide con `RECOVERY_INTERVAL` de `IdleEmbedder`, que recupera un hilo muerto a lo sumo una vez por minuto: un reintento más seguido sólo encontraría otro «no listo».

| Alternativa | Por qué no |
|---|---|
| E tal cual: una cola local de reintentos con `not_before` y el worker siguiendo con lo demás | Rompe el orden de dos saves con la misma clave: si el modelo vuelve entre el fallo del primero y el segundo, el segundo queda vivo y el reintento del primero lo reemplaza. Detectar la clave en común es lógica nueva para un caso en que, de todos modos, lo que sigue fallaría igual |
| Barrido periódico del outbox | Las filas que esperan en el canal también están `pending` y el worker no las ve hasta recibirlas: el barrido las tomaría fuera de orden. Sin él, lo que el reintento no cubre (un error de la base al tomar o terminar una fila) espera al próximo arranque, como hoy |
| Reintentar también lo determinista (F) | Un NaN falla igual cada vez; tres embeds inútiles por save |

**Al que espera** se le contesta en el primer fallo, no al terminar: el reintento puede tardar minutos y el plazo de confirmación es de 10 s. La respuesta `202 retrying` dice lo que pasa, y el handler conserva la fila porque el fallo es pasajero (D2).

**Una fila de la lectura de arranque** que se reintenta retiene también la copia que llegó por el canal con la respuesta de quien la guardó: ese llamante ve `202 unconfirmed` al vencer su espera, que es cierto (la fila sigue y se termina). Sólo ocurre con un save aceptado entre que el daemon escucha y que el worker lee el outbox.

**Drenaje:** la espera mira la misma señal que `next_event`; si empieza un drenaje, el pedido sale como uno sin empezar (su fila ya tiene el intento devuelto) y el worker devuelve el resto como hoy.

**Un evento sin fila** no se reintenta: no hay fila que tomar, y sólo existe en tests y evaluación.

## D2. El handler sólo olvida un save que no guardó nada (R2)

**Elegido:** `save_memory` borra la fila sólo con un fallo no pasajero y `memories` vacío. Con un fallo pasajero contesta `202 retrying` (D1). Con un fallo no pasajero y algún pedazo guardado contesta `202 partially_saved` con los pedazos y la causa; la fila sigue `pending` con su progreso y el próximo arranque la termina (`resume_positions`), o llega a `failed` y se descarta (D4).

| Alternativa | Por qué no |
|---|---|
| A: deshacer los pedazos en el fallo | Deshace trabajo que el replay habría terminado, y con nadie esperando no hay a quién decirle «no guardado» |
| Seguir con `500` y un texto nuevo | Un error lleva al agente a guardar otra vez con otro `operation_id`: el texto se absorbería y la fila vieja, al terminarse, volvería a vincular los mismos pedazos |

`SaveBody` suma `detail`; `SaveOutcome::Queued` suma `memories` y `detail`. Un puente anterior lee estos `202` como uno sin confirmar.

## D3. La pausa del apagado (R3)

**Elegido:** `ingest_pieces` recibe un `Pause` (el plazo ya visto y la señal de drenaje) y, en un save con fila, antes de cada insert pregunta `pause_due(now, plazo)` (pura). Vencido, vuelve con el reporte marcado `paused` y los pedazos guardados hasta ahí. El worker entonces llama `outbox_pause` (`attempts = MAX(attempts - 1, 0)`, sin tocar `last_error`), suelta la respuesta sin contestar y termina como con un evento sin empezar.

El plazo se lee de la señal en cada pedazo porque el drenaje puede empezar después de tomar el evento. La primera inserción de cada toma no se pausa: pausar ahí tiraría el embed entero que la toma pagó, un save de un solo pedazo nunca terminaría en un apagado, y uno que se pausara siempre en el mismo punto no avanzaría nunca. La pausa no es un `IngestFailure`: nada falló, y un fallo se anotaría en `last_error` y llegaría al que espera.

| Alternativa | Por qué no |
|---|---|
| Pausar también antes del embed | El embed del lote es una sola llamada; el plazo se mira al tomar el evento (`drain_step`) y otra vez antes de cada insert, que es lo que se puede retomar |
| Marcar la fila `failed` | Miente sobre la causa y bloquea el replay |

## D4. Reintentar y descartar por `operation_id` (R5)

**Elegido:** dos rutas del daemon y un comando `clawg outbox`, como `undelete`.

- `MemoryStore::outbox_find_failed(op, agent)` resuelve la fila; `OutboxActionError` dice por qué no (`404` sin fila, `409` pendiente, compartida por varios agentes o con un evento que ya no parsea).
- **Reintentar:** `outbox_requeue(row)` inserta la fila otra vez con otro `id`, `pending`, `attempts = 0`, sin `last_error` y con su progreso, y borra la vieja, en una transacción; después se encola. Un `id` nuevo hace falta porque el worker reconoce por `id` las filas de la lectura de arranque, y una fila que estaba `pending` al arrancar y quedó `failed` en esta ejecución se tomaría por la segunda entrega de sí misma (`StartupRows`).
- **Descartar:** se borran con `delete_memory_core` los pedazos con `inserted: true` del progreso, con el agente de la fila como dueño, y después la fila. Como el save nunca terminó, `revise_in` no corrió: la versión anterior de su clave sigue viva, así que borrar sus pedazos no revive nada. Un corte entre los dos pasos deja la fila `failed`; descartarla otra vez no encuentra pedazos vivos que borrar.

| Alternativa | Por qué no |
|---|---|
| K: una herramienta MCP | Suma superficie al contrato que el modelo ve en cada sesión por un caso raro; el agente ve el estado en `memory_status` |
| L: descartar por antigüedad | Borra un save aceptado sin que nadie lo decida |
| Descartar dejando los pedazos | Quedan sueltos, sin vínculo, y si el save tenía clave, como segunda versión viva |

`OutboxProblem` suma `pieces` (los ids insertados según el progreso), para que `/health` y `memory_status` muestren qué borraría un descarte.

## D5. La regla de `failed` (R4)

El código ya cuenta tomas: `claim_step` da por perdida una fila que gastó 3, y `after_failure` la marca `failed` cuando un fallo no pasajero llega con 3. Lo que cambia es el texto de `.claude/CLAUDE.md`, que prometía `failed` para todo fallo después de contestar, y la spec de storage, que decía que un fallo pasajero espera al próximo arranque.
