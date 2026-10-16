# bridge-reconnect — Diseño

## D1. El reintento vive en el puente, no en `DaemonClient`

**Elegido:** `ClawgMcp` envuelve el cuerpo de cada herramienta en un `call` que pide el cliente, corre la operación, y ante una falla que se resuelve releyendo relee una vez y repite.

| Alternativa | A favor | En contra |
|---|---|---|
| **En el puente** (elegida) | El defecto es del único proceso de larga vida que guarda un cliente. Los comandos de la CLI no cambian. | Cada `*_impl` pasa su llamada como closure. |
| En `DaemonClient` (un `send` único con estado interior) | Un solo lugar para las 20 llamadas. | El cliente pasa a tener estado mutable compartido; cambia `clawg stop`, `backup`, `memories` y `undelete`, que ya releen los archivos en cada corrida y no lo necesitan. |
| Vigilar los archivos | — | Descartado en la exploración: dependencia nueva y comportamiento por plataforma. |

Cada herramienta repite **todo** su cuerpo (registro del proyecto incluido), no sólo la última llamada: así un 401 en el registro y otro en el save se resuelven con la misma relectura.

## D2. Releer es sondear sin lanzar

**Elegido:** releer reutiliza `probe_running(base_dir, false)` (`crates/clawg-cli/src/client.rs:965`): archivos más `/health`, con `autostart = false`, así nunca lanza un daemon y un store tomado por otro proceso sigue diciendo `StoreHeld`.

- **¿Por qué no `DaemonClient::from_data_dir` solo?** Un `daemon.json` de un crash apunta a un puerto muerto. Sondear cuesta un `/health` en loopback y deja elegir el texto: sin daemon, R4 de la spec; con daemon, se repite.
- **¿Por qué no autoarrancar?** Decisión de la propuesta (tarjeta del hilo): el puente viejo lanzaría el binario viejo (`client.rs:865`).

## D3. Dónde se guarda el cliente nuevo

**Elegido:** `ClientHandle` suma `base_dir` y un `Arc<std::sync::Mutex<Option<DaemonClient>>>` con el último cliente releído, compartido por todos los clones (rmcp clona el servidor por llamada). `get` devuelve ese cliente si existe; si no, espera el arranque como hoy (`crates/clawg-cli/src/mcp/startup.rs:44-60`). `reresolve` sondea según D2 y, si encuentra un daemon, lo guarda.

- **¿Por qué no conservar el emisor del `watch`?** El arranque en segundo plano ya es dueño del emisor (`mod.rs:99-121`); compartirlo mezcla «cómo terminó el arranque» con «cuál es el daemon de ahora». Un `Option` aparte deja intacto el estado del arranque.
- **`std::sync::Mutex`** y no el de tokio: nunca se sostiene a través de un `.await`.
- `ClientHandle::ready(client)` toma `base_dir` de `client.base_dir()`; `pending` lo recibe como parámetro.

Dos herramientas concurrentes que reciben un 401 releen las dos y la última escribe: las dos encuentran el mismo daemon, así que no hay carrera que importe.

## D4. La decisión es pura y no mira el transporte

**Elegido:** `on_failure(&ClientError) -> Next { Reresolve, Report }` en `startup.rs`, con su tabla de casos. No se separa «conexión rechazada» de «timeout» en `map_transport` (`client.rs:489-495`), como pedía la exploración: sólo `/health` tiene timeout (`client.rs:505`), así que en las llamadas que escriben `NotRunning` ya significa «no llegó». Un test (`only_health_sets_a_timeout`) fija esa invariante leyendo el fuente de `client.rs`: si alguien suma un `.timeout(` a otra llamada, falla y obliga a revisar este diseño.

- **Alternativa:** una variante `ClientError::TimedOut`. Cambiaría lo que deciden `clawg status`, `stop` y `backup` (`crates/clawg-cli/src/main.rs:117`, `:134`) por un caso que hoy no ocurre en el puente.

## D5. Qué error se informa

- Falla al pedir el cliente con `StartTimeout` y releer no encuentra daemon → el texto de `StartTimeout` (R3).
- Falla en la llamada y releer no encuentra daemon → el error de la relectura (`NotRunning` o `StoreHeld`), con los textos de R4.
- Repetición fallida → el error de la repetición, por `explain` o por el mapeo propio de la herramienta (`save_outcome_text`, `route_failure_text`, el 404 de `search_project`), igual que hoy.

`explain` suma una rama para `Unauthorized` con el texto de R4. Como sólo llega a `explain` después de releer, ese texto ya puede afirmar que se releyó.

## D6. Registro del proyecto

`project_for` (`crates/clawg-cli/src/mcp/server.rs:510-532`) saca la raíz del conjunto `registered` cuando el registro falla con una falla que `on_failure` resuelve releyendo. El resto de las fallas la deja marcada, como dice su comentario de hoy.
