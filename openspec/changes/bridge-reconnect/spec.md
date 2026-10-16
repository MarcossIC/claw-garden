# bridge-reconnect — Especificación

Cómo el puente `clawg mcp` vuelve a encontrar el daemon cuando éste se reinicia o no terminó de arrancar. Agrega un apartado a [`specs/mcp`](../../../specs/mcp/spec.md). No cambia ninguna herramienta, ni sus argumentos, ni lo que contestan cuando el daemon responde.

## Definiciones

- **Releer:** sondear el daemon como lo hace `probe_running(base_dir, false)` (`crates/clawg-cli/src/client.rs:965`): leer `daemon.json` y `token` del directorio de datos y pedir `/health`. Releer MUST NOT lanzar un daemon.
- **Falla que se resuelve releyendo:** `ClientError::Unauthorized` (el daemon contestó 401), `ClientError::NotRunning` (la conexión no se abrió) o `ClientError::StartTimeout` (el arranque del puente no terminó a tiempo).
- **Falla que se informa:** cualquier otra, entre ellas una respuesta del daemon que no sea 401 (`ClientError::Daemon`) y un error de transporte después de conectar (`ClientError::Http`).

## R1. Qué falla se resuelve releyendo

Una función pura MUST decidir, sólo a partir del `ClientError`, si una falla se resuelve releyendo o se informa, según las definiciones de arriba. `ClientError::StoreHeld`, `Spawn`, `Daemon`, `Http` y el resto MUST informarse sin releer.

#### Escenario
- GIVEN cada variante de `ClientError`
- WHEN se decide qué hacer con ella
- THEN `Unauthorized`, `NotRunning` y `StartTimeout` se resuelven releyendo, y todas las demás se informan

**Test:** `only_a_refused_token_a_refused_connection_or_an_unfinished_start_is_reresolved` (`mcp/startup.rs`).

## R2. Releer y reintentar una vez

Cuando una herramienta falla con una falla que se resuelve releyendo, sea al pedir el cliente o al llamar al daemon, el puente MUST releer una vez:

- Si releer encuentra un daemon que contesta, el puente MUST guardar ese cliente para las llamadas siguientes, MUST repetir la herramienta **una** vez con él y MUST devolver lo que conteste esa repetición.
- Si releer no encuentra un daemon, la herramienta MUST informar la falla de R4 sin repetir nada.
- Si la repetición falla otra vez, MUST informarse esa segunda falla, sin un tercer intento.

Repetir MUST NOT llegar dos veces al handler del daemon: sólo se repite lo que el daemon no atendió (un 401 lo corta `require_bearer` antes del handler, `crates/clawg-cli/src/daemon/api.rs:607-625`; una conexión rechazada no llegó). La única llamada del cliente con timeout MUST seguir siendo `/health` (`client.rs:505`), de modo que en una herramienta que escribe `NotRunning` sólo signifique una conexión que no se abrió.

Al encontrar un daemon de otra versión que el puente, el puente SHOULD dejarlo en el log con el texto de `version_warning` (`crates/clawg-cli/src/memories.rs:332`). No MUST cambiar lo que contesta la herramienta.

#### Escenario: el daemon se reinició
- GIVEN un puente contra el daemon A, con su puerto y su token
- AND A se detiene y el daemon B arranca sobre el mismo directorio de datos, con otro puerto y otro token
- WHEN el agente llama a `save_memory` y después a `search_memory`
- THEN el save se guarda en B y la búsqueda lo encuentra
- AND ninguna de las dos respuestas es un error

#### Escenario: el token se relee una sola vez
- GIVEN un daemon que contesta 401 a toda ruta con token
- WHEN el agente llama a `save_memory`
- THEN el daemon recibe exactamente dos pedidos a `/v1/memories`
- AND la respuesta es un error con el texto de R4 para el token rechazado

#### Escenario: un error que el daemon contestó no se repite
- GIVEN un daemon que contesta 500 a `/v1/memories`
- WHEN el agente llama a `save_memory`
- THEN el daemon recibe exactamente un pedido a `/v1/memories`

**Tests:** `a_restarted_daemon_is_reached_with_its_new_token`, `a_refused_token_is_reread_once` y `an_error_the_daemon_answered_reaches_it_once` (`mcp/server.rs`); `only_health_sets_a_timeout` (`client.rs`).

## R3. Un arranque que no terminó se retoma

Cuando el arranque en segundo plano del puente terminó en `StartTimeout`, o todavía no terminó al vencer la espera de la herramienta, la siguiente herramienta MUST releer. Si para entonces el daemon contesta, MUST usarse y guardarse como en R2. Si no, MUST informarse el texto de `StartTimeout` de hoy (`crates/clawg-cli/src/mcp/server.rs:418-422`), no el de un daemon detenido: lo que falló fue el arranque.

#### Escenario
- GIVEN un puente cuyo arranque venció
- AND después un daemon termina de arrancar y publica `daemon.json` y `token`
- WHEN el agente llama a `memory_status`
- THEN la respuesta es el estado de ese daemon, no un error

**Test:** `a_daemon_that_comes_up_after_the_start_gave_up_is_used` (`mcp/startup.rs`).

## R4. Textos de falla

En inglés, como el resto de las herramientas:

- **Daemon que no está** (`NotRunning` después de releer): MUST decir que el daemon no está corriendo, que pudo haberse detenido o actualizado, y que se arranca con `clawg serve` o reconectando clawg desde el cliente MCP. Texto: `the clawg daemon is not running (it may have been stopped or updated); start it with `clawg serve`, or reconnect clawg in your MCP client, then call again`.
- **Token rechazado después de releer** (`Unauthorized` en la repetición): MUST NOT mandar a borrar el archivo del token, y MUST decir que el token del directorio de datos no coincide con el daemon que corre. Texto: `the clawg daemon rejected the token in its data directory even after re-reading it, so the token file no longer matches the running daemon; restart the daemon (`clawg status` shows its pid) and call again`.

El texto de `ClientError::Unauthorized` de la CLI (`client.rs:51`) no cambia: los comandos releen los archivos en cada corrida y para ellos el consejo sí sirve.

#### Escenario
- GIVEN un puente cuyo daemon se detuvo y no volvió
- WHEN el agente llama a cualquier herramienta
- THEN la respuesta es un error con el texto del daemon que no está
- AND ningún daemon se lanza

**Test:** `a_daemon_stopped_mid_session_is_reported_without_starting_one` (`mcp/server.rs`); `the_refused_token_text_does_not_say_to_delete_it` (`mcp/server.rs`).

## R5. El registro del proyecto que no llegó se vuelve a mandar

Un registro de proyecto (`POST /v1/projects`, `project_for` en `mcp/server.rs`) que falló con una falla que se resuelve releyendo MUST NOT contar como enviado: la llamada siguiente MUST volver a mandarlo. Cualquier otra falla MUST seguir contando como enviada, como hoy, para no costarle un pedido a cada herramienta frente a un daemon que no sabe registrar.

#### Escenario
- GIVEN un puente cuyo daemon se reinició antes de la primera herramienta
- WHEN el agente llama a `save_memory` en su proyecto
- THEN el daemon nuevo recibe el registro del proyecto antes del save

**Test:** `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` (`mcp/server.rs`).

## R6. `memory_status` no cambia

`memory_status` MUST seguir sin token (`/health`) y MUST pasar por R2 y R3 como las demás herramientas: después de un reinicio muestra el daemon nuevo.

#### Escenario
- GIVEN un puente contra el daemon A que se detuvo, y el daemon B arrancado
- WHEN el agente llama a `memory_status`
- THEN la respuesta muestra el pid de B

**Test:** `memory_status_after_a_restart_reports_the_new_daemon` (`mcp/server.rs`).
