# local-transport — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. Los stand-ins de transporte (T6, D7)

- [ ] Tests primero: `a_default_port_override_is_off_or_a_port`, el campo `transport` del sello (`build_info.rs`).
- [ ] `default_port_override`, `tcp_only`, `Stamp::transport`.
- Commit: `test(core): stand in for the default port and an older transport in testing builds`

## 2. El listener del socket (T1, T2, D1, D5)

- [ ] Tests primero: `the_same_uid_is_admitted_and_another_is_not`, `the_socket_path_fits_under_the_limit_and_a_longer_one_is_named` (`daemon/socket.rs`); `the_router_answers_over_the_socket_behind_the_guard` (`tests/api.rs`).
- [ ] `socket_file` en `paths.rs`; `daemon/socket.rs` con `admits_peer`, `PeerChecked`, `check_socket_path`, `bind_socket`.
- Commit: `feat(daemon): a Unix socket listener that admits only its own user`

## 3. `serve` por el socket y TCP según la regla (T2, T3, D2, D4)

- [ ] Tests primero: `the_tcp_plan_follows_the_port_and_the_socket` (`serve.rs`); el estado sin `port` y con la marca (`lifecycle.rs`); `clear_state` borra el socket.
- [ ] `DaemonState` con `port` opcional y `unix_socket`; `serve::run(profile, Option<u16>)`; dos servidores con una señal.
- Commit: `feat(daemon): serve on the data directory's socket beside TCP`

## 4. El cliente por el socket (T4, D3)

- [ ] Tests primero: `the_socket_wins_when_published_and_tcp_is_used_otherwise`, `a_client_on_a_published_socket_with_nothing_behind_is_not_running` (`client.rs`).
- [ ] `endpoint`, `from_data_dir` con `unix_socket`.
- Commit: `feat(cli): reach the daemon over its socket when it publishes one`

## 5. `update` sin `--port` en Unix (T5, D6)

- [ ] Test primero: `the_new_daemon_gets_a_port_only_where_tcp_is_the_only_transport` (`update/effects.rs`).
- [ ] `Running.port` opcional, `serve_port`.
- Commit: `feat(update): start the new daemon without a port where it has a socket`

## 6. El binario (T2–T5)

- [ ] `binary.rs`: ayudantes por el socket; los escenarios de T2, T3, T4 y T5.
- Commit: `test(cli): drive the socket, autostart and older clients through the binary`

## 7. Docs y archivo

- [ ] `.claude/CLAUDE.md`, `README.md`.
- [ ] `openspec/specs/daemon-access`, `install`, `self-update`, `mcp`; mapa de `openspec/README.md`.
- [ ] `verify.md`, archivo y versión 0.37.0.
