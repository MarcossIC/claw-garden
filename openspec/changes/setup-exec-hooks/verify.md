# setup-exec-hooks — Verificación

## Linux

Sobre el commit de docs (`67487e2`):

- `cargo test --workspace`: pasa entero.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sin advertencias.
- Cada test nuevo de la [spec](spec.md) falló antes de su código: por no compilar (`is_bare_invocation`, `legacy_command`, `migrated`, `stale_copies`) o por la aserción (`a_bare_clawg_exits_1` salía con 2; el enlace pasaba a archivo común y el 0640 a 0644).

## `clawg setup` con el `claude` real (2.1.304), perfil dev, home temporal

Con `HOME`, `CLAUDE_CONFIG_DIR` y `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y el build de tests de `target/debug/clawg`. Nunca se tocó `~/.claude`, `~/.claude.json` ni `~/.clawg*`.

1. Un `settings.json` 0600 con los dos hooks como los escribía la versión anterior (la cadena `<ruta> hook <evento> --agent claude-code --profile dev`). `clawg setup claude-code --profile dev`: `migrated SessionStart, UserPromptSubmit to run without a shell`, sale con 0. Cada hook quedó con `command` = la ruta del binario y `args` = `hook <evento> --agent claude-code --profile dev`, en su grupo y con su matcher; `theme` siguió en su lugar. `settings.json` siguió en 0600 y su copia nació 0600.
2. `claude mcp get clawg-dev`: `Status: √ Connected`, la ruta absoluta y `--agent claude-code`.
3. Una segunda corrida: `already there` en los dos eventos, sin copias nuevas, sale con 0.
4. Con una memoria guardada y `BASH_ENV` apuntando a un archivo que imprime una línea: la forma heredada corrida por `bash -c` dio esa línea antes del JSON del hook; la forma exec, el binario con sus argumentos, dio sólo el JSON.
5. `clawg` sin argumentos salió con 1.
6. `--remove`: quitó el servidor, los dos hooks y el bloque; `settings.json` quedó con `theme` sólo. `clawg stop --profile dev` paró el daemon que arrancó el chequeo de `claude mcp get`.

## Lo que no se verificó

- **Windows.** La suite corre ahí en el CI del PR, pero los tests de enlace y de modo son de Unix. Falta `clawg setup claude-code` en Windows sin Git Bash y con un usuario cuya ruta tenga un espacio, con una sesión de Claude Code que reciba el contexto.
- **Una sesión de Claude Code ejecutando el hook exec.** El contenedor no tiene credenciales para abrir una sesión; el hook se corrió a mano como lo corre el cliente (binario y argumentos, sin shell).
- **Un cliente anterior a 2.1.139** corriendo `command` sin `args`: se verificó sólo que `clawg` a secas sale con 1.

## Veredicto

Sin CRITICAL. Los tres puntos de arriba quedan como WARNING para verificar en Windows y con una sesión real antes de archivar.
