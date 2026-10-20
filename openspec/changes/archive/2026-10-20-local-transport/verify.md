# local-transport — Verificación

## Linux (2026-10-20)

Sobre `e434686` (el último commit de código), en `target`, con el compilador de `rust-toolchain.toml`:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0, y `cargo test --locked -p clawg-core --lib build_info` (el lado `cfg(not(feature = "testing"))`, donde `default_port_override` es `None` y `tcp_only` es falso) pasa.
- Cada commit de código de las [tareas](tasks.md) (`bf64c07`, `0657daf`, `ca4a3e0`, `a5170fd`, `37f8278`, `e434686`) pasó la suite de `clawg-cli` y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` pasa sin bendecir: nada de lo que ve el modelo cambió.

## Dos cuentas reales (T1, T3)

Con el binario de `target/debug` (build `testing`, modelo falso, `CLAWG_TEST_HOME` en el home de cada cuenta) y dos cuentas sin privilegios creadas para la prueba, `a` y `b`, perfil `dev`:

| Paso | Resultado |
|---|---|
| `a` corre `clawg serve` sin `--port` | `daemon.json` con `port: 27381` y `unix_socket: true`; `<data>/clawg.sock` creado; `curl --unix-socket … http://localhost/health` de `a` contesta 200 |
| `b` conecta al socket de `a` (directorio 0700) | `curl: (7) Couldn't connect to server`: no atraviesa el directorio |
| `b` crea un socket en el directorio de `a` | `PermissionError: [Errno 13] Permission denied` |
| `b` corre su propio `clawg serve` sin `--port` mientras el de `a` tiene 27381 | arranca; `warn` `could not bind 127.0.0.1:27381: Address already in use …; serving on the data directory's socket alone`; su `daemon.json` no tiene `port`; `clawg status` de `b` da `running: true` por el socket |
| el directorio de `a` forzado a 0755 y su socket a 0777; `b` conecta | `curl: (56) Recv failure: Connection reset by peer`; el log de `a` registra `refused 1 connection(s) on …/clawg.sock from another user` |
| `clawg stop` de cada uno | los dos salen con 0; no queda `clawg.sock` ni `daemon.json` en ningún directorio |

Mientras TCP siga encendido, `b` sí lee `/health` de `a` por `127.0.0.1:27381`: es lo que la transición deja abierto para los clientes anteriores, y lo cierra el cambio que apague TCP sin `--port`. Las cuentas se borraron al terminar.

## CI

`ci.yml` corre en el pull request de la rama, sobre Linux y Windows. Windows no se compiló acá (no hay `x86_64-w64-mingw32-gcc` para `ring`): `daemon/socket.rs`, `bind_profile_socket` y los escenarios nuevos de `binary.rs` son `#[cfg(unix)]`; en Windows `endpoint` nunca da el socket (`cfg!(unix)`), `tcp_plan` recibe `socket = false` y deja el puerto del perfil fatal como antes, y `serve_port` conserva el puerto del daemon viejo.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| T6 | `a_default_port_override_is_off_or_a_port` y `a_stamp_can_restrict_the_binary_to_tcp` no compilaban sin `parse_default_port`, `DefaultPort` ni `Stamp::transport` |
| T1, T2 | los tests de `daemon/socket.rs` no compilaban sin el módulo |
| T3 | `the_tcp_plan_follows_the_port_and_the_socket` no compilaba sin `tcp_plan`; `a_record_without_the_socket_mark_reads_as_tcp_and_one_without_a_port_has_none` no compilaba con `port: u16` y sin `unix_socket`; `clear_state` dejaba `clawg.sock` |
| T4 | `the_socket_wins_when_published_and_tcp_is_used_otherwise` no compilaba sin `endpoint`; antes `from_data_dir` armaba siempre `http://127.0.0.1:{port}` |
| T5 | `the_new_daemon_gets_a_port_only_where_tcp_is_the_only_transport` no compilaba sin `serve_port`; antes `start_and_wait` recibía siempre `Some(port)` |

Los escenarios de `binary.rs` se escribieron después del código que prueban y no se corrieron en rojo; cada uno comprueba algo que antes no existía (el archivo `clawg.sock`, `unix_socket` en `daemon.json`, un daemon sin `port`), y el del actualizador anterior comprueba primero que su daemon no publica socket, o sea que el sello `transport: "tcp"` hace efecto.

## Lo que no se midió

- Ningún ranking ni recuperación cambia, así que no hay `rag-eval`.
- macOS: `peer_cred` (`getpeereid`) lo implementa tokio y queda sin ejecutar; el tope de `sun_path` de 104 bytes no se probó en una máquina.
- La latencia: la exploración midió decenas de microsegundos de diferencia a favor del socket; no se volvió a medir.
- El sandbox de Bash de Claude Code con el socket (`network.allowUnixSockets` en macOS) no se probó.
