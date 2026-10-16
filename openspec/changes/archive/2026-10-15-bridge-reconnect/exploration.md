# bridge-reconnect — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 (y transversal) | S | — (se coordina con 36 por el reintento de un save; ver «Interacciones») | exploración (borrador) |

**Tesis:** el puente MCP resuelve el daemon una sola vez, al arrancar, y se queda toda la sesión con ese puerto y ese token. Pero el daemon genera un token nuevo cada vez que arranca. Entonces, apenas el daemon se reinicia (por una actualización, un `clawg stop` o un crash), todas las herramientas de todas las sesiones abiertas fallan con un mensaje que manda a borrar el token y reiniciar `clawg serve`, que es justo lo que no arregla nada. Mientras tanto, `memory_status` sigue diciendo que todo está bien. La única salida es que el usuario reconecte el servidor a mano desde el cliente. Y como el daemon es compartido por varios agentes (objetivo 4), un solo reinicio rompe a todos a la vez.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| El puente resuelve el cliente una sola vez | **Cierta.** `run_with` llama a `probe_running` y, si hay daemon, arma `ClientHandle::ready(client)` (`crates/clawg-cli/src/mcp/mod.rs:90-97`). `ready` crea un canal `watch` con un único valor y descarta el emisor (`crates/clawg-cli/src/mcp/startup.rs:32-35`). El arranque en segundo plano publica una sola vez (`mod.rs:99-121`). Ningún camino vuelve a leer `daemon.json` ni `token` después. |
| El daemon genera un token nuevo en cada arranque | **Cierta.** `generate_token` y `write_token` corren después del bind (`crates/clawg-cli/src/daemon/serve.rs:115-118`). |
| Un 401 devuelve un consejo equivocado | **Cierta.** Un 401 se convierte en `ClientError::Unauthorized` (`crates/clawg-cli/src/client.rs:465-466`), con el texto «the daemon rejected the token; delete the token file and restart `clawg serve`» (`client.rs:51`). `explain` no lo trata aparte y cae en la rama genérica (`crates/clawg-cli/src/mcp/server.rs:401-433`, rama `other` en `:432`). Borrar el token y reiniciar el daemon genera otro token, que el puente tampoco tiene. |
| `memory_status` dice que todo está bien con el puente roto | **Cierta.** `memory_status` consulta `/health`, que no lleva token (`client.rs:490-498`; ruta abierta en `crates/clawg-cli/src/daemon/api.rs:559-561`). Se reprodujo en el perfil prod: con el daemon recién reiniciado, `memory_status` respondía `ok` y `recall_context` devolvía el mensaje del token. |
| Si el daemon muere durante la sesión, el puente no lo vuelve a arrancar | **Cierta.** Se responde «no clawg daemon is running; start one with `clawg serve`» (`server.rs:403-405`). El autoarranque sólo existe al lanzar el puente (`mod.rs:99-121`). |
| El defecto ya se conoce | **Cierta, y está documentado como procedimiento:** «El puente no se reconecta solo. En Claude Code, escribí `/mcp` y reconectalo» (`README.md:317-318`). También aparece en verificaciones archivadas (`openspec/changes/archive/2026-10-14-long-save-context/verify.md:125`, `openspec/changes/archive/2026-10-15-project-indexing/verify.md:88`). |
| Un timeout se trata igual que «no hay daemon» | **Cierta.** `map_transport` convierte tanto `is_connect()` como `is_timeout()` en `NotRunning` (`client.rs:478-484`). Para reintentar importa la diferencia: una conexión rechazada nunca llegó al daemon, pero un timeout sí pudo haber llegado. |

## Problema (verificado contra el código)

- **Cuándo pasa.** Cada vez que el daemon arranca de nuevo con otro proceso:
  - una actualización (el instalador lo para antes de reemplazar el binario);
  - un `clawg stop` hecho a mano;
  - un `clawg restore` o un `clawg purge`, que exigen el daemon parado;
  - un crash seguido del autoarranque de otro puente.

  Con dos clientes abiertos (dos sesiones de Claude Code, o Claude Code y otro agente), el segundo autoarranca y el primero queda con un token viejo.
- **Qué ve el agente.**
  - Ve un error que no puede resolver y que lo empuja a decirle al usuario que borre un archivo.
  - Si después consulta `memory_status`, ve `ok`.
  - Las memorias que intente guardar en ese tramo no se guardan, y lo único que queda es el texto del error.
- **Qué arreglaría reintentar.** Un 401 lo rechaza `require_bearer` antes de llegar al handler (`api.rs:588-604`), y una conexión rechazada ni siquiera llegó. En los dos casos la operación no se hizo, así que repetirla con el token nuevo es seguro, incluso para un `save_memory`.

## Lo que ya existe y se reutiliza

- `DaemonClient::from_data_dir` lee `daemon.json` y `token` y arma una URL siempre loopback (`client.rs:429-444`). `read_token` devuelve `None` si el archivo falta o está vacío (`crates/clawg-cli/src/daemon/lifecycle.rs:129-138`). Un archivo a medio escribir se lee como «todavía no hay daemon», y no como un token inválido.
- El punto único por donde cada herramienta obtiene el cliente es `ClawgMcp::daemon` (`server.rs:384-392`). La traducción de errores también está centralizada en `explain` (`server.rs:401-433`).
- El dedup absorbe un save repetido: refuerza la fila y no inserta otra (`crates/clawg-core/src/memory/store_insert.rs:236-243`). Sirve de red si un reintento llegara a duplicar algo.
- Los tests del puente ya arman un daemon en proceso y un puente contra él (`bridge`/`bridge_at` en el `mod tests` de `server.rs`; `crates/clawg-cli/tests/api.rs`). Un segundo daemon sobre el mismo directorio con otro token es un caso armable sin procesos aparte.

## Enfoques posibles

### A. Volver a leer `daemon.json` y `token` ante un 401 o una conexión rechazada, y reintentar una vez (recomendado)

- **Decisión pura:** `on_failure(error) -> Reresolve | Report`. Devuelve `Reresolve` sólo para `Unauthorized` y para una conexión rechazada. Todo lo demás se informa como hoy.
- **Al reintentar:**
  - Se arma otro `DaemonClient` desde los archivos y se repite la llamada **una vez**.
  - Si el token releído es el mismo y el daemon igual lo rechaza, se informa con un texto correcto.
  - Si no hay archivos, se informa que el daemon no está.
- **El cliente pasa a ser reemplazable dentro del handle.** Hoy es un valor fijo de un `watch` sin emisor. Alcanza con un `tokio::sync::RwLock<DaemonClient>` o con conservar el emisor del `watch`; `tokio` ya es dependencia, así que no se suma ninguna.
- **Hay que separar «conexión rechazada» de «timeout»** en `map_transport`. Un timeout no se reintenta: la llamada pudo haber llegado. En un save, el dedup lo absorbería, pero sumaría un `access_count` que nadie generó (`store_insert.rs:310-317`). El matiz: `probe_running` hoy usa `NotRunning` para los dos casos (`client.rs:937-945`), y eso no debe cambiar para el autoarranque.
- **A favor:** cierra el caso de cada actualización sin intervención del usuario. Es puro y testeable. No cambia ni el protocolo ni la API.
- **En contra:** toca cómo se guarda el cliente del puente, y cada herramienta pasa por el reintento.

### B. Además, volver a arrancar el daemon si no está

- **A favor:** cubre también el crash.
- **En contra, y por eso no se recomienda:**
  - El puente lanza `current_exe()` (`client.rs:837`). Después de una actualización en Windows, el puente que sigue abierto es el binario viejo, el mismo que el instalador movió a un lado (`long-save-context/verify.md:125`). Autoarrancar desde ahí levantaría la versión vieja contra un store que la nueva quizá ya migró, y la vieja lo rechazaría.
  - Además, pelearía contra un `clawg stop` hecho a propósito: `restore` y `purge` necesitan el daemon parado. El lock evita el daño, pero el usuario ve un daemon que «vuelve solo».
- Queda como decisión. Lo seguro es informar y no arrancar.

### C. Sólo corregir el texto

`Unauthorized` diría «el daemon se reinició; reconectá este servidor desde el cliente» en lugar de mandar a borrar el token.

- **A favor:** es una línea.
- **En contra:** no arregla nada, sólo deja de dar un mal consejo. Va incluido en A para el caso en que el reintento también falla.

## Descartado

| Opción | Razón técnica |
|---|---|
| Vigilar `daemon.json` y `token` con notificaciones del sistema de archivos | Suma una dependencia y comportamientos distintos por plataforma. Releer dos archivos chicos sólo cuando una llamada falla cuesta nada y no puede quedar desincronizado. |
| Un token fijo entre arranques | Cambia el modelo de seguridad que daemon-hardening (38) da por supuesto («el token vive con el proceso y se regenera en cada arranque»). Un secreto que sobrevive al proceso queda más tiempo expuesto en disco. |
| Reintentar cualquier error | Un 400 o un 500 repetido da el mismo resultado. Y un timeout pudo haber llegado al daemon. |
| Que `memory_status` lleve token para detectar el problema | Rompe `probe_running` y `clawg stop` entre versiones mezcladas (`openspec/changes/daemon-hardening/exploration.md:72`). El puente ya se entera por el 401 de la primera herramienta. |

## Interacciones

- **ingest-outbox (36).** Con el `operation_id` que emite el daemon, un save que venció por timeout se puede repetir sin efecto. Hasta entonces, este cambio sólo reintenta lo que no llegó al daemon.
- **install-smoke (40).** Reiniciar el daemon debajo de un puente abierto es un caso de proceso real. El test en proceso de este cambio lo cubre a nivel de lógica; 40 lo cubre con el binario.
- **agent-lifecycle-hooks (25).** Cada hook es un proceso nuevo que lee `daemon.json` y `token` en cada evento (`DaemonClient::from_data_dir`). No sufre este defecto.
- **daemon-hardening (38).** No cambia el ciclo de vida del token ni su archivo. Si 38 restringe los permisos del archivo desde su creación, la relectura sigue funcionando porque el puente corre como el mismo usuario.
- **Versiones mezcladas.** Después de una actualización, el puente viejo pasa a hablar con el daemon nuevo. `version_warning` (`crates/clawg-cli/src/memories.rs`) ya detecta la diferencia. Queda como decisión si el puente lo dice al reconectar.

## Alcance candidato

1. La función pura `on_failure` con su tabla de casos.
2. Un cliente reemplazable en `ClientHandle` y el reintento único en el camino de cada herramienta.
3. Separar «conexión rechazada» de «timeout» en `map_transport`, sin cambiar lo que `probe_running` decide.
4. Textos de `explain` para `Unauthorized` después de releer, y para un daemon que ya no está a mitad de sesión.
5. `README.md`, sección «Si algo falla»: el paso de `/mcp` deja de hacer falta después de un `clawg stop` o de una actualización.

### No-alcance

- Volver a arrancar el daemon desde un puente abierto (B), salvo que se decida lo contrario.
- Cambiar el ciclo de vida del token o el formato de `daemon.json`.
- Reintentos con espera o más de un reintento.

## Decisiones abiertas

Son del usuario.

- [ ] **Autoarranque a mitad de sesión.** ¿Nunca (recomendado), o sólo si el binario del puente es el mismo que resuelve el PATH?
- [ ] **Timeout.** ¿Nunca se reintenta (recomendado hasta 36), o se reintenta un save cuando exista `operation_id`?
- [ ] **Aviso de versión.** Al reconectar con un daemon de otra versión, ¿el puente lo dice en la respuesta de la herramienta o sólo en el log?
- [ ] **Texto final** de `Unauthorized` y de «daemon detenido» (en inglés, como el resto de las herramientas).

## Cómo se mide

Tests RED primero:

1. Un puente contra el daemon A. Se detiene A y se levanta B sobre el mismo directorio, con otro token. La siguiente herramienta responde con B. Hoy devuelve el texto de `Unauthorized`.
2. Un 401 con el archivo de token sin cambios: un solo reintento, y el texto final no dice «delete the token file».
3. Conexión rechazada con el daemon de vuelta: la herramienta responde. Sin daemon de vuelta: el texto de daemon detenido, sin autoarranque.
4. Timeout: no hay reintento, y un save no llega dos veces al daemon.
5. `on_failure` como tabla de casos, sin red.
6. `memory_status` sigue funcionando igual con y sin reconexión.

`rag-eval` no aplica: no cambia el ranking ni el contexto. No toca el esquema, los ids, los estados ni el contrato de embedding, así que no hace falta una base sembrada.

## Riesgos

- **Repetir algo que sí llegó.** Lo evita reintentar sólo ante 401 y conexión rechazada. Hay que cuidar que la separación de `map_transport` no cambie lo que decide el autoarranque.
- **Leer un token a medio escribir.** `write_token` crea el archivo y después escribe (`lifecycle.rs:122-127`). Un archivo vacío se lee como `None` (`lifecycle.rs:134-138`), así que el reintento informa «daemon no disponible» en lugar de mandar un token roto. Volver a llamar un instante después funciona.
- **Ocultar un problema real de token.** Si el daemon rechaza el token recién leído, el reintento no lo arregla, y el texto tiene que decir qué hacer sin esconderlo.
