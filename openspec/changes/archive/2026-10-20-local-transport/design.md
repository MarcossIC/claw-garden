# local-transport — Diseño

## D1. Un listener propio con chequeo de uid (A), no sólo el directorio (B)

`PeerChecked` envuelve `tokio::net::UnixListener` e implementa `axum::serve::Listener`. Su `accept` delega en el `accept` de axum para el socket (que ya reintenta los errores de `accept`), lee `UnixStream::peer_cred()` y devuelve la conexión sólo si `admits_peer(uid, geteuid())`; si no, la suelta y vuelve a aceptar.

- **B (sólo el directorio en 0700)** es más corto, pero deja abierto un directorio que no esté en el modo esperado (un `chmod` del usuario, un sistema de archivos que ignora el modo) y depende de un permiso sobre el socket que `unix(7)` llama no portable.
- **A** cuesta unas treinta líneas y una llamada por conexión. Root y el mismo uid pasan igual por las dos: ya leen `token`.

El `warn` del rechazo va con tope de uno por minuto, como el del guardia (`guard.rs`), para que una cuenta ajena que insiste no llene el log.

## D2. `daemon.json` lleva una marca, no la ruta

`DaemonState` gana `unix_socket: bool` (`serde(default)`, así un archivo anterior lo lee falso) y `port` pasa a `Option<u16>` (`serde(default, skip_serializing_if = "Option::is_none")`). Un clawg anterior no tiene `deny_unknown_fields`, así que ignora la marca, y lee `port` mientras esté; sin `port`, su `read_state` da `None` y cree que no hay daemon, que es lo que tiene que creer de un daemon al que no puede hablar.

La ruta la deriva el cliente (`socket_file(base_dir)`): un archivo alterado que nombrara otro socket mandaría el token a otro proceso, la misma razón por la que `from_data_dir` ya ignora el host.

## D3. El cliente no cae a TCP

`endpoint(state, base_dir) -> Option<Endpoint>` es pura: `Unix(socket_file)` si `unix_socket` (sólo en Unix), si no `Tcp(port)`, y `None` sin ninguno. Cuando el socket está publicado y no contesta, el cliente lee `NotRunning` y nunca prueba el puerto: un proceso de otra cuenta en ese puerto volvería a ser el daemon. Sobre el socket la base es `http://localhost`, que el guardia admite; reqwest no usa proxy ni DNS con `unix_socket`.

Un clawg anterior se emula con el sello: `endpoint` ignora la marca cuando `build_info::tcp_only()`.

## D4. TCP en la transición: una regla pura

```text
tcp_plan(explicit, profile_port, socket) =
  explicit = Some(p)            -> Required(p)
  explicit = None, !socket      -> Required(profile_port)     Windows, clawg anterior
  explicit = None, socket       -> Optional(profile_port)     o Off con el stand-in
```

`Required` falla el arranque como hoy; `Optional` registra el fallo y sigue. Si `Optional` fuera fatal, el choque entre dos usuarios del mismo perfil seguiría durante toda la transición; si nunca se intentara, los puentes y actualizadores anteriores quedarían sin daemon.

Los dos servidores comparten una señal: una tarea espera `shutdown_signal` y levanta un `watch`; cada `axum::serve` termina con él, y `within_grace` corre sobre el `try_join` de los dos, así un error de uno termina el daemon como hoy y `HTTP_GRACE` cuenta una sola vez.

## D5. Orden de arranque y el huérfano

Candado (`MemoryRuntime::open`) → borrar `clawg.sock` → `bind` del socket → TCP → `token` y `daemon.json`. Con el candado tomado, ningún otro daemon vivo sirve ese socket, así que borrarlo es seguro; un clawg de evaluación que tiene el candado no crea socket. El tope de `sun_path` se comprueba antes del `bind` con una función pura para que el error nombre la ruta y el límite (la de std sólo dice `path must be shorter than SUN_LEN`). `clear_state` borra el socket al salir, antes de que el candado se suelte.

## D6. `update` sin `--port` en Unix (i)

`Running.port` pasa a opcional. `StartDaemon` y `undo StartOldDaemon` llaman `spawn_serve_at(…, serve_port(old))`, con `serve_port` = `None` en Unix y el puerto viejo en Windows (único transporte hasta windows-pipe). Deshacer no necesita `--port`: arranca el binario instalado, que es el del propio actualizador y conoce el socket.

- **(ii) conservar un `--port` pedido a mano** no distingue a un usuario de un actualizador anterior, que siempre lo pasa: TCP se arrastraría de versión en versión.
- **(iii) conservarlo si difiere del perfil** pierde un `--port 27380` explícito y conserva cualquier puerto que un actualizador anterior copió.

Con el sello `transport: "tcp"` el actualizador pasa el puerto viejo, como un clawg anterior.

## D7. Los stand-ins

- `CLAWG_TEST_DEFAULT_PORT=off|<puerto>` (`build_info::default_port_override`): `off` apaga el TCP opcional, un número lo reemplaza. `binary.rs` pone `off` en Unix a cada hijo, así ningún test toca 27381; el del puerto ocupado pone el de su propio listener.
- `transport: "tcp"` en el sello (`build_info::tcp_only`): sin socket, el puerto del perfil fatal, `endpoint` por TCP y `update` con `--port`.

Los dos siguen la regla de `CLAWG_TEST_HOME`: sólo un build con `testing` los lee.
