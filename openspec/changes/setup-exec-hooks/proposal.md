# setup-exec-hooks — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 | S–M | — (50 local-isolation cubre el modo del directorio de datos; ver «Interacciones» de la [exploración](exploration.md)) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`3383f3d`).

1. **El hook pasa por un shell que clawg no controla.** `hook_commands` arma una sola cadena (`crates/clawg-cli/src/setup.rs:287-293`) que `install_hooks` escribe como `command`, sin `args` (`setup.rs:449`). El cliente la entrega a `sh -c`, a Git Bash o, sin Git Bash, a PowerShell. `shell_word` entrecomilla una ruta con espacio (`setup.rs:299-313`), y PowerShell no ejecuta una primera palabra entre comillas: en Windows sin Git Bash, con el binario bajo un usuario `Ana María`, el hook no corre y el agente arranca sin memoria.
2. **La salida de un perfil de shell precede al JSON** con Git Bash o `BASH_ENV`, y el cliente ignora el JSON sin error.
3. **La falla es muda:** `clawg hook` sale siempre con 0 (`crates/clawg-cli/src/hook.rs:6-7`).
4. **`setup` degrada lo que escribe.** `write_file` (`setup.rs:664-690`) copia con `std::fs::write`, escribe un temporal que nace con el `umask` y lo pone con `rename` sin `sync_all`: un `settings.json` enlazado pasa a ser archivo común, un 0600 queda 0644, y las copias de `backups/setup` nacen con el `umask` y nunca se podan.

## Objetivo que sirve

4: varios agentes comparten la memoria y el usuario controla qué ve cada uno; un hook que no corre deja a un agente sin contexto y sin aviso.

## Decisiones

Cerradas con el usuario; el porqué está en el [diseño](design.md).

- **Forma exec siempre** (enfoque A): `command` = la ruta absoluta del binario sin transformar, `args` = `hook <evento> --agent claude-code --profile P`. Sin shell, sin comillas, sin `\` a `/`.
- **`clawg` sin subcomando sale con 1, no con 2** (enfoque D): un cliente anterior a 2.1.139, que ignora `args`, correría el binario solo; con 1 no bloquea la sesión. Ningún otro error de uso cambia.
- **Sólo la forma heredada idéntica se migra:** la cadena exacta que generaba `setup` para ese evento y perfil, con `--agent claude-code`, sea cual sea la ruta de clawg. Se reescribe en su lugar a la forma exec. Cualquier otra cadena de clawg del mismo evento y perfil sigue siendo conflicto.
- **`--remove` quita las dos formas.**
- **Enlaces:** se escribe el archivo real al que apunta el enlace, y el enlace queda.
- **Modo:** un archivo que existía conserva su modo (Unix); el temporal se sincroniza antes del `rename`.
- **Copias:** nacen 0600 en Unix y se conservan las 5 más recientes por archivo; el reporte nombra las que se borran.

## Alcance

1. `crates/clawg-cli/src/setup.rs`: el plan de hooks en forma exec, el reconocimiento de las dos formas, la migración, `write_file` y la poda.
2. `crates/clawg-cli/src/cli.rs` y `main.rs`: `clawg` sin subcomando sale con 1.
3. `crates/clawg-cli/tests/binary.rs`: los tests del binario real que leen el hook, y el de `clawg` a secas.
4. Docs: README, `.claude/CLAUDE.md`.

## Fuera de alcance

- Detectar la versión de `claude` desde `setup` (enfoque B).
- Hooks de plugin del cliente o `${CLAUDE_PLUGIN_ROOT}`.
- El modo del directorio de datos y de `backups/setup` (local-isolation, 50): este cambio lo crea con `create_dir_all` como hoy.
- Mover el reporte de `setup` a un módulo propio.
- Otros clientes.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)): el plan produce `command` + `args` también con una ruta con espacio y no ASCII; la cadena heredada se migra y una segunda corrida no escribe; una cadena editada sigue en conflicto; `--remove` quita las dos formas; `clawg` a secas sale con 1; en Unix, un `settings.json` enlazado sigue enlazado, un 0600 sigue 0600, las copias nacen 0600 y quedan a lo sumo 5.

`rag-eval`: no aplica, no hay cambio de ranking.

## Datos y vuelta atrás

No toca el store, el esquema, los identificadores ni el contrato de embedding. Escribe lo mismo que hoy (`settings.json` y `CLAUDE.md` del cliente, y copias en `<data>/backups/setup/`), y además borra copias de `backups/setup` más allá de las 5 más recientes por archivo. Volver atrás: `clawg setup claude-code --remove` quita las dos formas; con un binario anterior, `--remove` no reconoce la forma exec como suya (`is_clawg_hook` lee sólo `command`), así que se borra a mano o se restaura una copia de `backups/setup`.
