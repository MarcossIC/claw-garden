# shutdown-budget — Diseño

## D1. El presupuesto (R1, R2)

**Elegido:** `STOP_TIMEOUT` = 30 s y cuatro constantes, cada una junto al código que aplica su tope:

| Fase | Constante | Valor | Dónde se aplica |
|---|---|---|---|
| HTTP | `HTTP_GRACE` | 3 s | `serve.rs`, `within_grace` |
| Drenaje | `DRAIN_DEADLINE` | 10 s (sin cambio) | `runtime.rs`, la señal del worker |
| Después del drenaje | `AFTER_DRAIN` | 7 s | `runtime.rs`, `shutdown_by` |
| Desmontaje | `RUNTIME_TEARDOWN` | 2 s | `main.rs`, `shutdown_timeout` |

22 s de fases, más 5 s de holgura para la petición, la espera y una máquina cargada, caben en 30 s. `AFTER_DRAIN` cubre los 5 s que el `checkpoint` puede pasar en el busy handler de rusqlite (`sqlite3_busy_timeout(db, 5000)`) más el save en curso y el flush. Ningún valor está medido: son topes, y en un apagado normal todo termina en menos de un segundo.

| Alternativa | Por qué no |
|---|---|
| Mantener 15 s acortando fases (drenaje 5 s, busy timeout del `checkpoint` a 1 s) | Deja 1 s de holgura, ingiere menos en cada apagado y cambia el `checkpoint` para que entre; el costo de 30 s sólo se ve cuando algo ya se colgó |
| Un único tope global en `stop` | `stop` espera el fin del proceso y no puede acortar nada del daemon; los topes tienen que estar donde está la espera |

## D2. La fase HTTP (R1)

**Elegido:** la señal de apagado se observa una vez (`shutdown_signal`) y, al dispararse, avisa por un `oneshot` a `within_grace(server, stopping, grace)`: corre el servidor hasta que termina, o hasta `grace` después del aviso. Pura respecto del servidor (cualquier `Future`), así que se prueba con tiempo pausado y un futuro que no termina. Si vence, se registra un warn y `run` sigue con `runtime.shutdown`.

Las tareas de conexión que axum lanzó siguen vivas hasta el desmontaje (D4). No pueden perder un save: uno que ya tiene fila sigue en el outbox, y uno que llega a `accept` con el worker terminado se rechaza y su fila se borra (`accept`, `crates/clawg-core/src/runtime.rs:249-259`).

| Alternativa | Por qué no |
|---|---|
| `TimeoutLayer` por petición | Corta en operación normal también: un backup legítimo fallaría sin que nadie pida parar |
| Una segunda escucha de las señales para el plazo | Dos manejadores de Ctrl-C y SIGTERM registrarían dos veces la misma señal |

## D3. Después del drenaje (R1)

**Elegido:** `shutdown_by` envuelve todo su cuerpo en `tokio::time::timeout(drain_deadline + AFTER_DRAIN, …)`, relativo al momento en que manda el plazo. Si vence, aborta el mantenimiento y registra un warn; no hace el `checkpoint`. El save que estaba en curso queda en su fila, como en un kill.

| Alternativa | Por qué no |
|---|---|
| Un tope sólo para el `checkpoint` | El worker también puede quedar esperando un embed que no vuelve |
| Bajar el busy timeout del `checkpoint` | Cambia la conexión de escritura para un caso que el tope ya cubre |

## D4. El desmontaje (R1)

**Elegido:** `main` guarda si el comando es `serve` y, al volver `block_on`, llama `runtime.shutdown_timeout(RUNTIME_TEARDOWN)` en lugar de soltarlo. Los otros comandos quedan como hoy (el hook ya usa `shutdown_background`). Un hilo de una carga abandonada termina con el proceso.

`shutdown_background` descartaría también lo que tarda un instante (un snapshot que está por renombrarse). Cancelar de verdad una carga de ONNX Runtime no es posible desde fuera del hilo.

Para probarlo con el binario, `CLAWG_TEST_EMBEDDER` suma `blocked`: el modelo «se descarga» al instante y su carga duerme sin volver, en el `spawn_blocking` de siempre.

## D5. SIGTERM (R3)

**Elegido:** un tercer brazo en el `select!` de `shutdown_signal`, `#[cfg(unix)]`, con `tokio::signal::unix::signal(SignalKind::terminate())` (el feature `signal` ya está). Si el registro falla, el brazo registra un warn y queda pendiente para siempre, como el de la API sin emisor.

**Windows**: fuera. El daemon autoarrancado no tiene consola (`DETACHED_PROCESS`), y si recibe `CTRL_SHUTDOWN_EVENT` o `CTRL_LOGOFF_EVENT` sólo se sabe probándolo en una máquina Windows; un manejador sin probar no se declara.

**Código de salida 0**, como Ctrl-C: el apagado fue ordenado, y un supervisor que lo pidió no tiene que leerlo como un fallo.

## D6. `update` ante un `stop` que vence (R4)

**Elegido:** `Effects::run` devuelve `Result<(), StepError>`, con `StepError { cause, under_way }`. `under_way` dice que el paso dejó algo en marcha al fallar; `apply` suma el deshacer si el paso deja rastros siempre (`Stage`, `StartDaemon`) o si este fallo los dejó. `From<String>` da un `StepError` sin rastros, así que los demás pasos no cambian.

`Installation` guarda el pid de un `StopTimeout` (`stopping`). `undo StartOldDaemon` lo toma y, si existe, espera su fin con `Watched` en `spawn_blocking`, a lo sumo `STOP_TIMEOUT`, antes de `start_and_wait`. Sin esa espera, `wait_for` vería contestar al daemon que se está apagando, con la misma versión, y daría `Ready` en falso. Si el pid sigue vivo, el deshacer falla con un texto que lo dice y el informe (`daemon_state`) distingue ese caso: con `StopDaemon` como paso fallido y `StartOldDaemon` atascado, dice que se pidió el apagado y que se arranque con `clawg serve` o reconectando el MCP del editor cuando termine.

| Alternativa | Por qué no |
|---|---|
| G: sólo corregir el informe | La memoria sigue caída hasta que el usuario actúe |
| Esperar el pid siempre en `StartOldDaemon` | Tras un `stop` que terminó, ese pid puede ser ya de otro proceso, y se esperaría por nada |
| Un `Step` nuevo para «apagado pedido» | Cambia el orden del plan para un caso que es un modo de fallar del mismo paso |
