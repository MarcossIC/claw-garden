# daemon-hardening — Verificación

## Linux

Sobre el último commit del cambio:

- `cargo test --workspace`: pasa entero, `tests/binary.rs` incluido (`status`, `stop`, MCP por stdio y el reinicio con un puente abierto, todos contra daemons `--port 0`) sin tocar ninguna de sus peticiones.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sin advertencias.
- `the_agent_contract_matches_its_file` no se movió: el cambio no toca lo que ve el modelo.

## Un daemon real, perfil dev, home de prueba

`clawg serve --profile dev --port 0` del build de tests (`target/debug/clawg`), con `CLAWG_TEST_HOME` en un directorio temporal y `CLAWG_TEST_EMBEDDER=fake`. Nunca se tocó `~/.clawg*`.

| Petición | Respuesta |
|---|---|
| `ls -la` del directorio de datos | `token` y `daemon.json` en `-rw-------` |
| `GET /health` a `127.0.0.1:<puerto>` | 200 |
| `GET /health` a `localhost:<puerto>` | 200 |
| `GET /health` con `Host: evil.example:<puerto>` | 403 `{"error":"forbidden"}` |
| `POST /v1/search` con el token y `Origin: http://evil.example` | 403 `{"error":"forbidden"}` |
| `GET /health` con `curl --http2-prior-knowledge` | 200 sobre HTTP/2: el daemon sí sirve HTTP/2 en texto plano (`hyper-util` llega con `http2` por `reqwest`), y la petición pasa por la autoridad de la URI, sin `Host` |
| `clawg status --profile dev` | `running: true`, código 0 |
| `clawg stop --profile dev` | `stopped the dev daemon`, código 0 |

El log del daemon tiene una sola línea por los dos rechazos, que llegaron dentro del mismo minuto: `refused 1 request(s) not addressed to a loopback name or carrying an Origin header; …`, sin los valores de las cabeceras. El segundo rechazo se cuenta en la línea siguiente.

## `rag-eval`

No aplica: el cambio no toca el ranking. `rag-eval --agent-view` habla con `serve_ephemeral` por `DaemonClient` sobre `http://127.0.0.1:<puerto>`, el mismo camino que cubre `a_daemon_client_passes_the_guard`. No se corrió con el modelo real.

## Lo que no se verificó

- **Windows.** La suite corre en el CI del PR. Ahí el token y `daemon.json` se siguen truncando en el lugar y no llevan ACL explícita (decisión de la propuesta); los tests de modo son de Unix.
- **Un navegador real.** Los rechazos se probaron con las cabeceras que manda un navegador, no con una página.

## Veredicto

Sin CRITICAL. Windows queda como WARNING hasta el CI del PR.
